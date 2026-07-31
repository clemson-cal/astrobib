"""e prompts for a destination path in the footer and writes the
selection (or cursor entry) as one .bib file; bad paths fail with an
error, creating no intermediate directories."""

import os

from driver import require

DESCRIPTION = "e exports selection to a .bib path"


def run(t):
    # select two rows, export to an absolute scratch path. Step through
    # the selection so a broken Space/j reports itself here rather than as
    # a puzzling "export 1 →" further down.
    t.send(" ")
    t.wait_for("1 selected", what="first row selected")
    t.send("j")
    t.send(" ")
    t.wait_for("2 selected", what="second row selected")
    t.send("e")
    t.wait_for(lambda: "export 2 →" in t.text(), what="export prompt for 2 entries")
    dest = os.path.join(t.scratch, "out.bib")
    # the prompt is prefilled with refs.bib — replace it wholesale
    t.send(b"\x15")  # ctrl+u clears the line
    t.send(dest)
    t.key("enter")
    t.wait_for(lambda: "Exported 2 entries" in t.text(), what="export confirmation")
    content = open(dest).read()
    require(
        content.count("@article{") == 2,
        f"expected 2 entries in the exported .bib, found "
        f"{content.count('@article{')}:\n{content[:400]}",
        t,
    )
    t.key("esc")  # leave selection mode
    t.wait_gone("selected")
    # settle, not wait_quiet: the screen is already correct and quiet. What
    # this gap is for is the *keyboard*: a lone ESC written back-to-back
    # with the next byte arrives as one pty read, and crossterm parses
    # ESC+'e' as alt+e, which is not a binding. The pause splits them.
    t.settle(0.2)
    # a path through a nonexistent directory fails, and creates nothing
    t.send("e")
    t.wait_for(lambda: "export 1 →" in t.text(), what="single-entry prompt")
    t.send(b"\x15")
    bad = os.path.join(t.scratch, "no-such-dir", "x.bib")
    t.send(bad)
    t.key("enter")
    t.wait_for(lambda: "could not write" in t.text(), what="write error")
    require(not os.path.exists(os.path.dirname(bad)), "intermediate dir was created", t)
