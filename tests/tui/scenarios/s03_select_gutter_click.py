"""A click in the leftmost gutter (SGR mouse injection) enters selection mode."""

from driver import require

DESCRIPTION = "selection mode via gutter click"


def run(t):
    row = t.row_of("census of runaway white dwarfs")  # Baxter2019, not the cursor row
    t.click(1, row)
    t.wait_for("1 selected")
    line = t.lines()[row]
    require(line.lstrip().startswith("◉"), "clicked row should be selected (◉)", t)
    # the click also moved the cursor: the card now shows Baxter's entry
    require("Baxter2019" in t.text(), "card should follow the clicked row", t)
    # a second gutter click on the same row deselects and exits
    t.click(1, row)
    t.wait_gone("1 selected")
