"""A click in the leftmost gutter (SGR mouse injection) enters selection mode."""

DESCRIPTION = "selection mode via gutter click"


def run(t):
    row = t.row_of("census of runaway white dwarfs")  # Baxter2019, not the cursor row
    t.click(1, row)
    t.wait_for("1 selected", what="selection-mode footer count")
    t.wait_for(
        lambda: t.lines()[row].lstrip().startswith("◉"),
        what=f"◉ gutter on the clicked row (screen row {row})",
    )
    # the click also moved the cursor: the card now shows Baxter's entry.
    # Needle on the *full* key — the table's Key column shows the short
    # "Baxter2019" on every frame, so it proves nothing about the card.
    # The card repaints from the new cursor and can lag the table by a
    # frame, so wait rather than assert.
    t.wait_for("Baxter2019equxm", what="pub card following the clicked row")
    # a second gutter click on the same row deselects and exits
    t.click(1, row)
    t.wait_gone("1 selected")
