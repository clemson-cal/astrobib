"""pty driver for astrobib TUI integration tests.

Spawns target/release/astrobib in a pseudo-terminal against a per-test
scratch library built from the committed fixtures, and reconstructs the
screen with pyte. All assertions go through the reconstructed screen —
never grep the raw byte stream.

Sandboxing: HOME, ASTROBIB_LIBRARY, and ASTROBIB_STATE_DIR all point into
the scratch directory, so the app can never see (or write) the user's real
library, state, caches, or Downloads. ASTROBIB_ASCII=1 keeps the pill
rendering out of the private-use glyph range.
"""

from __future__ import annotations

import fcntl
import os
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

import pyte

FIXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")

KEYS = {
    "esc": b"\x1b",
    "enter": b"\r",
    "space": b" ",
    "tab": b"\t",
    "backspace": b"\x7f",
    "delete": b"\x1b[3~",
    "up": b"\x1b[A",
    "down": b"\x1b[B",
    "right": b"\x1b[C",
    "left": b"\x1b[D",
}


class Skip(Exception):
    """Raised by a scenario to mark itself skipped."""


class Timeout(AssertionError):
    """wait_for ran out of time; message carries a screen dump."""


class Session:
    """One TUI process in one pty against one scratch library."""

    def __init__(self, binary, cols=140, rows=40, allow_network=False, manuscript=None,
                 pre_launch=None):
        self.binary = os.path.abspath(binary)
        self.cols = cols
        self.rows = rows
        self.scratch = tempfile.mkdtemp(prefix="astrobib-tui-")
        self.home = os.path.join(self.scratch, "home")
        self.state_dir = os.path.join(self.scratch, "state")
        self.library = os.path.join(self.scratch, "library")
        self.bib_dir = os.path.join(self.library, "bib")
        os.makedirs(self.home)
        os.makedirs(self.state_dir)
        os.makedirs(self.bib_dir)
        for name in sorted(os.listdir(FIXTURES)):
            if name.endswith(".bib"):
                shutil.copy(os.path.join(FIXTURES, name), self.bib_dir)
        # a manuscript (tier-2) root under $HOME: {filename: content},
        # with an empty bib/ so the walk-up finds it; the TUI starts there
        cwd = self.home
        if manuscript:
            cwd = os.path.join(self.home, "ms")
            os.makedirs(os.path.join(cwd, "bib"))
            for name, content in manuscript.items():
                path = os.path.join(cwd, name)
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "w") as f:
                    f.write(content)
        self.cwd = cwd
        # a scenario hook that seeds state files (tabs, caches) before
        # the binary starts
        if pre_launch:
            pre_launch(self.state_dir)

        env = {
            "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8",
            "LC_ALL": "en_US.UTF-8",
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": self.home,
            "ASTROBIB_LIBRARY": self.library,
            "ASTROBIB_STATE_DIR": self.state_dir,
            "ASTROBIB_ASCII": "1",
        }
        if allow_network and os.environ.get("ADS_API_TOKEN"):
            env["ADS_API_TOKEN"] = os.environ["ADS_API_TOKEN"]

        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        self.eof = False

        self.master, slave = os.openpty()
        winsz = struct.pack("HHHH", rows, cols, 0, 0)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, winsz)
        self.proc = subprocess.Popen(
            [self.binary],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            cwd=self.cwd,
            start_new_session=True,
        )
        os.close(slave)

    # -- lifecycle ---------------------------------------------------------

    def wait_ready(self, timeout=10.0):
        """Block until a COMPLETE startup frame is up.

        The footer badges are drawn last, so requiring them means the
        frame finished flushing — waiting only on the table header can
        return mid-frame, before the card and footer exist.
        """
        self.wait_for(
            lambda: "Library" in self.text()
            and "Year" in self.text()
            and "keys" in self.lines()[-1],
            timeout=timeout,
            what="complete startup frame",
        )
        return self

    def quit(self, timeout=5.0):
        """q should exit promptly; escalate only if it does not."""
        if self.proc.poll() is None:
            try:
                self.send("q")
            except OSError:
                pass
            deadline = time.monotonic() + timeout
            while self.proc.poll() is None and time.monotonic() < deadline:
                self._pump(0.05)
        self.close()

    def resize(self, cols, rows=None):
        """Resize the pty, and the pyte screen with it.

        Setting the window size on the master delivers SIGWINCH to the
        slave's foreground process group, so the app re-lays-out exactly
        as it would in a real terminal being dragged — this is the only
        way to exercise the responsive column rules without spawning a
        fresh session per width. The caller should wait_quiet() after:
        the repaint is asynchronous.
        """
        rows = self.rows if rows is None else rows
        fcntl.ioctl(
            self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0)
        )
        self.cols, self.rows = cols, rows
        self.screen.resize(rows, cols)

    def close(self):
        if self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                self.proc.kill()
            self.proc.wait()
        try:
            os.close(self.master)
        except OSError:
            pass
        shutil.rmtree(self.scratch, ignore_errors=True)

    # -- input -------------------------------------------------------------

    def send(self, text):
        """Send literal characters (str) or raw bytes."""
        data = text if isinstance(text, bytes) else text.encode()
        os.write(self.master, data)

    def key(self, name):
        self.send(KEYS[name])

    def paste(self, text):
        """Paste text as a real terminal does, in bracketed-paste markers.

        The app enables bracketed paste, so the whole string arrives as
        one `Event::Paste` rather than as a burst of key events — which
        is what lets a pasted URL be recognised as one thing instead of
        a hundred characters that happen to spell one.
        """
        self.send(b"\x1b[200~" + text.encode() + b"\x1b[201~")

    def click(self, x, y):
        """Left click at 0-based screen cell (x, y) via SGR mouse injection."""
        self.send(b"\x1b[<0;%d;%dM" % (x + 1, y + 1))
        self.send(b"\x1b[<0;%d;%dm" % (x + 1, y + 1))

    def hover(self, x, y):
        """Move the mouse to 0-based cell (x, y) via SGR motion injection."""
        self.send(b"\x1b[<35;%d;%dM" % (x + 1, y + 1))

    # -- screen ------------------------------------------------------------

    def _pump(self, budget=0.05):
        """Drain pending pty output into the pyte screen."""
        wait = budget
        while not self.eof:
            r, _, _ = select.select([self.master], [], [], wait)
            if not r:
                return
            try:
                data = os.read(self.master, 65536)
            except OSError:
                self.eof = True
                return
            if not data:
                self.eof = True
                return
            self.stream.feed(data)
            wait = 0  # keep draining whatever is already buffered

    def lines(self):
        self._pump(0)
        return list(self.screen.display)

    def text(self):
        return "\n".join(self.lines())

    def dump(self):
        """Screen with a border, for failure reports."""
        rule = "+" + "-" * self.cols + "+"
        body = "\n".join("|" + ln + "|" for ln in self.lines())
        return f"{rule}\n{body}\n{rule}"

    def find(self, needle):
        """(x, y) of the first cell of the first occurrence, or None."""
        for y, ln in enumerate(self.lines()):
            x = ln.find(needle)
            if x >= 0:
                return (x, y)
        return None

    def row_of(self, needle):
        pos = self.find(needle)
        if pos is None:
            raise AssertionError(f"{needle!r} not on screen\n{self.dump()}")
        return pos[1]

    # -- waiting -----------------------------------------------------------

    def wait_for(self, cond, timeout=8.0, what=None):
        """Wait until cond (a substring or a nullary predicate) holds."""
        if isinstance(cond, str):
            needle, pred = cond, lambda: cond in self.text()
        else:
            needle, pred = what or "predicate", cond
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self._pump(0.05)
            if pred():
                return
            if self.proc.poll() is not None and self.eof:
                # one final look at whatever the dying process flushed
                if pred():
                    return
                raise Timeout(
                    f"process exited (rc={self.proc.returncode}) before "
                    f"{what or needle!r} appeared\n{self.dump()}"
                )
        raise Timeout(f"timed out waiting for {what or needle!r}\n{self.dump()}")

    def wait_gone(self, needle, timeout=8.0):
        self.wait_for(
            lambda: needle not in self.text(),
            timeout=timeout,
            what=f"disappearance of {needle!r}",
        )

    def wait_quiet(self, idle=0.15, timeout=5.0):
        """Block until the pty has been silent for `idle` seconds.

        ratatui diff-renders, so a redraw is a burst of bytes followed by
        silence: once the stream goes quiet again, whatever the last input
        triggered has finished landing. This is the condition-based form of
        "let the frame settle" — prefer it to `settle()` whenever there is
        no positive needle to wait for (proving a key did *not* change
        something, sweeping many hover positions), because `settle()` can
        return in the middle of a burst and hand the scenario half a frame.

        Returns True if the stream went quiet, False on timeout.
        """
        deadline = time.monotonic() + timeout
        while not self.eof and time.monotonic() < deadline:
            r, _, _ = select.select([self.master], [], [], idle)
            if not r:
                return True
            self._pump(0)
        return self.eof

    def settle(self, seconds=0.3):
        """Sleep for a fixed span, pumping as we go.

        A blunt instrument: it cannot tell "nothing happened yet" from
        "nothing will happen". Reach for `wait_for` first and `wait_quiet`
        second; the one remaining use is the pty keyboard-fusion window in
        s15, where the point is to leave a gap between two keystrokes, not
        to wait for the screen. Any new call site needs a comment saying
        why neither of the other two works.
        """
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self._pump(0.05)


def require(condition, message, session=None):
    if not condition:
        suffix = f"\n{session.dump()}" if session is not None else ""
        raise AssertionError(message + suffix)
