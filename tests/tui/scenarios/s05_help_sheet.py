"""? opens the keys panel above the log (non-modal: other keys still
work); ? or Esc closes it."""

from driver import require

DESCRIPTION = "? keys panel is non-modal; ?/Esc close"


def run(t):
    t.send("?")
    t.wait_for(lambda: "this cheat-sheet" in t.text(), what="keys panel")
    require(" keys " in t.text(), "panel title missing", t)
    # non-modal: navigation keys act while the panel is up
    t.send("j")
    t.settle()
    require("this cheat-sheet" in t.text(), "panel wrongly dismissed by j", t)
    t.send("?")
    t.wait_gone("this cheat-sheet")
    t.send("?")
    t.wait_for(lambda: "this cheat-sheet" in t.text(), what="panel reopens")
    t.key("esc")
    t.wait_gone("this cheat-sheet")
