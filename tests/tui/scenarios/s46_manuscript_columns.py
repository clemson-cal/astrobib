"""The manuscript page draws — and configures — the same columns as the
library.

A manuscript row is a cite, but every cite that resolves names a paper,
so the paper columns (↓, Year, Author, Key, the metric swatch) are on
offer here exactly as they are in the library and on a query page, with
Cited and State interleaved among them. This walks the ones that are new
to this scope: the headers, a cell that only a resolved row can produce,
sorting in both directions by one of them, and the panel listing the
rest.
"""

from driver import require

DESCRIPTION = "the manuscript scope offers the library's columns"

# wide enough that Year and Author both clear the title's comfort width
# and default on; the panel opening later narrows the table and drops
# Author again, which is the same responsive rule the library follows
COLS = 170

MANUSCRIPT = {
    "main.tex": (
        "\\documentclass{article}\n\\begin{document}\n"
        "Citing \\citep{Andersson2021} and \\citet{Baxter2019} "
        "and \\citep{Nowhere2020}.\n\\end{document}\n"
    ),
}


def _header(t):
    """The table's header line — the one above the rule."""
    for i, ln in enumerate(t.lines()):
        if "─" * 30 in ln:
            return t.lines()[i - 1]
    raise AssertionError(f"no table header rule on screen\n{t.dump()}")


def _first_data_row(t):
    for i, ln in enumerate(t.lines()):
        if "─" * 30 in ln:
            return t.lines()[i + 1]
    raise AssertionError(f"no table header rule on screen\n{t.dump()}")


def _panel_line(t, label):
    # the panel occupies the leftmost columns; the table's own headers
    # sit further right
    for ln in t.lines():
        if label in ln[:24]:
            return ln
    return ""


def run(t):
    t.send("]")
    t.wait_for(lambda: "missing" in t.text(), what="manuscript rows")

    head = _header(t)
    for label in ("↓", "Cited", "State", "Year", "Author", "Title"):
        require(label in head, f"no {label} column on the manuscript header: {head!r}", t)

    # the author cell is the honest witness that the column carries the
    # resolved paper's data and not just a header
    t.wait_for("Andersson, +1", what="the author cell of a resolved cite")

    # …and it sorts, like every other header — starting descending, the
    # same "newest first" the library's Year column starts at. Flipping
    # it puts the cite that resolves to nothing (so has no year at all)
    # on top, which is an order the scan cannot produce.
    x, y = t.find("Year")
    t.click(x, y)
    t.wait_for("Year ▼", what="the manuscript sorted by Year, newest first")
    t.wait_for(
        lambda: "Andersson2021" in _first_data_row(t),
        what="the newest paper first, descending",
    )
    t.click(x, y)
    t.wait_for("Year ▲", what="the sort flipped")
    t.wait_for(
        lambda: "Nowhere2020" in _first_data_row(t),
        what="the yearless cite first, ascending",
    )

    # the panel offers the rest of the library's set here too
    t.send("|")
    t.wait_for("Table configuration", what="the table panel")
    for label in ("metric", "pdf", "Cited", "State", "Year", "Author", "Title", "Key"):
        require(_panel_line(t, label), f"no {label} row in the manuscript panel", t)

    # Key is the one paper column off by default here — Cited already
    # names the paper — so switching it on is what proves the panel is
    # driving this scope's columns and not the library's
    require("· Key" in t.text(), "Key should start hidden in the manuscript", t)
    # Key is the last row the panel lists, and the cursor clamps there
    for _ in range(12):
        t.key("down")
    t.send(" ")
    t.wait_for(lambda: "✓ Key" in t.text(), what="the Key column switched on")
    t.wait_for(
        lambda: "Key" in _header(t),
        what="the Key column drawn in the manuscript table",
    )
