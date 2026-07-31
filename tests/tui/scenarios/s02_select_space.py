"""Space enters selection mode; Esc and toggling the last row off both leave it."""

from driver import require

DESCRIPTION = "selection mode via Space"


def run(t):
    t.key("space")
    # in selection mode every visible row carries a gutter marker. The
    # footer is drawn after the table within a frame, so "1 selected"
    # appearing means the gutters of that same frame are already on
    # screen — but wait on the gutters themselves anyway, so a failure
    # names the thing that is actually wrong.
    t.wait_for("1 selected", what="selection-mode footer count")
    t.wait_for(
        lambda: t.text().count("◯") >= 4 and "◉" in t.text(),
        what="4+ unselected ◯ gutters and the toggled row's ◉",
    )

    # toggling the last selected row off exits the mode
    t.key("space")
    t.wait_gone("1 selected")
    # negative assertion: the footer count is drawn after the table, so
    # its disappearance already implies the repaint that clears the gutters
    require("◯" not in t.text(), "leaving selection mode should clear ◯ gutters", t)

    # re-enter, grow the selection, then Esc clears everything at once
    t.key("space")
    t.wait_for("1 selected", what="selection mode re-entered")
    t.send("j")
    t.key("space")
    t.wait_for("2 selected", what="second row added to the selection")
    t.key("esc")
    t.wait_gone("selected")
    require("◯" not in t.text(), "Esc should exit selection mode", t)
