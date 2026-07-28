"""? opens the keyboard cheat-sheet; any key dismisses it without acting."""

from driver import require

DESCRIPTION = "? cheat-sheet opens, any key dismisses"


def run(t):
    t.send("?")
    t.wait_for("this cheat-sheet")
    require("select / toggle row" in t.text(), "cheat-sheet body missing", t)
    require("◼ keys" in t.text(), "keys badge should light up while open", t)
    # any key dismisses — and is swallowed, not executed ('y' would
    # otherwise open the copy modal)
    t.send("y")
    t.wait_gone("this cheat-sheet")
    require("copy → clipboard" not in t.text(), "dismissal key must be swallowed", t)
    require("◻ keys" in t.text(), "keys badge should turn back off", t)
