"""A state dir that has gone unwritable is reported — once. Curated
priorities that stop persisting must not be discovered at next launch,
and must not log a line per idle tick either."""

import os

from driver import require

DESCRIPTION = "unwritable state dir reported once, not per tick"

ERR = "could not save metrics.json"


def run(t):
    os.chmod(t.state_dir, 0o500)  # readable, not writable
    try:
        # . sets priority 1.0; the idle tick then fails to flush it
        t.send(".")
        t.wait_for(lambda: "priority 1.00" in t.text(), what="priority feedback")
        t.wait_for(lambda: ERR in t.text(), what="write failure reported")

        # further failing flushes stay quiet: one report, not one per tick
        for _ in range(2):
            t.send("j")
            t.send(".")
            t.wait_for(lambda: "priority 1.00" in t.text(), what="priority feedback")
        t.settle(1.0)

        t.send("L")  # the event log holds the history the footer shows one of
        t.wait_for(lambda: " Log " in t.text(), what="log pane")
        n = sum(1 for line in t.lines() if ERR in line)
        require(n == 1, f"expected exactly one {ERR!r} line in the log, saw {n}", t)
    finally:
        os.chmod(t.state_dir, 0o700)
