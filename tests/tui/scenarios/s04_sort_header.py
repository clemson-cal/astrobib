"""Clicking a column header sorts by it; the same header flips the ▲/▼ marker."""

from driver import require

DESCRIPTION = "sort-by-header click flips ▲/▼"


def run(t):
    require("Year ▼" in t.text(), "expected default Year ▼ marker", t)
    x, y = t.find("Year")
    header_y = y
    first_row = header_y + 2  # header, rule, then data rows

    # same header flips direction: Year ▼ -> Year ▲, oldest entry on top.
    # Marker and row order come from the same render, but wait on the row
    # too: if the sort ever lands a frame behind the marker, this says so
    # instead of failing on a stale screen.
    t.click(x, y)
    t.wait_for("Year ▲", what="Year marker flipped to ascending")
    t.wait_for(
        lambda: "lacroix, +1" in t.lines()[first_row],  # Délacroix — the 2018 entry
        what=f"the 2018 entry (Délacroix) on the first data row, {first_row}",
    )
    require("Year ▼" not in t.text(), "old descending marker should be gone", t)

    # a different header moves the marker (text columns start ascending)
    tx, ty = t.find("Title")
    t.click(tx, ty)
    t.wait_for("Title ▲", what="Title marker, ascending")
    t.wait_for(
        lambda: "A census of runaway white dwarfs" in t.lines()[first_row],
        what=f"'A census …' on the first data row, {first_row}",
    )
    require("Year ▲" not in t.text(), "marker should leave the Year column", t)

    # clicking Title again flips it
    t.click(tx, ty)
    t.wait_for("Title ▼", what="Title marker flipped to descending")
