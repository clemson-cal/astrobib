"""Space enters selection mode; Esc and toggling the last row off both leave it."""

from driver import require

DESCRIPTION = "selection mode via Space"


def run(t):
    t.key("space")
    t.wait_for("1 selected")
    # in selection mode every visible row carries a gutter marker
    require(t.text().count("◯") >= 4, "unselected rows should show ◯ gutters", t)
    require("◉" in t.text(), "the toggled row should show a ◉ gutter", t)
    # toggling the last selected row off exits the mode
    t.key("space")
    t.wait_gone("1 selected")
    require("◯" not in t.text(), "leaving selection mode should clear ◯ gutters", t)

    # re-enter, grow the selection, then Esc clears everything at once
    t.key("space")
    t.wait_for("1 selected")
    t.send("j")
    t.key("space")
    t.wait_for("2 selected")
    t.key("esc")
    t.wait_gone("selected")
    require("◯" not in t.text(), "Esc should exit selection mode", t)
