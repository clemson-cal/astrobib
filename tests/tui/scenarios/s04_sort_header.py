"""Clicking a column header sorts by it; the same header flips the ▲/▼ marker."""

from driver import require

DESCRIPTION = "sort-by-header click flips ▲/▼"


def run(t):
    require("Year ▼" in t.text(), "expected default Year ▼ marker", t)
    x, y = t.find("Year")
    header_y = y

    # same header flips direction: Year ▼ -> Year ▲, oldest entry on top
    t.click(x, y)
    t.wait_for("Year ▲")
    require("Year ▼" not in t.text(), "old marker should be gone", t)
    first = t.lines()[header_y + 2]  # header, rule, then data rows
    require(
        "lacroix, +1" in first,  # Délacroix — the 2018 entry
        "Year ▲ should put the 2018 entry first",
        t,
    )

    # a different header moves the marker (text columns start ascending)
    tx, ty = t.find("Title")
    t.click(tx, ty)
    t.wait_for("Title ▲")
    require("Year ▲" not in t.text(), "marker should leave the Year column", t)
    first = t.lines()[header_y + 2]
    require(
        "A census of runaway white dwarfs" in first,
        "Title ▲ should put 'A census …' first",
        t,
    )

    # clicking Title again flips it
    t.click(tx, ty)
    t.wait_for("Title ▼")
