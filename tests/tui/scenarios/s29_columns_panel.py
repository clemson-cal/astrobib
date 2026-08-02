"""The table panel: show, hide, resize, and sort — including on a
column that is not on screen.

The panel is a toggle view beside the table, not a modal, so the
interesting part is focus: it needs ↑↓ to walk its list and ←→ to size a
column, and those are the table's own navigation keys. Opening it hands
the arrows over, Esc hands them back without closing it.

Sorting a hidden column is the point of listing the hidden ones: a query
can be ordered by entry date without spending ten columns of screen on
the dates.
"""

import json
import os

from driver import require

DESCRIPTION = "table panel: show/hide, width, sort, focus"

COLS = 150


def _panel_line(t, label):
    """The panel row carrying `label`, or "".

    The panel occupies the leftmost columns and the table starts after
    it, so a label found in the first 24 columns is the panel's — the
    table has its own "Year" and "Title" headers further right.
    """
    for ln in t.lines():
        if label in ln[:24]:
            return ln
    return ""


def _first_data_row(t):
    # the panel draws a rule of its own between the columns and the ADS
    # section, so require a run only the table is wide enough to make
    # (the table area is never narrower than 40)
    for i, ln in enumerate(t.lines()):
        if "─" * 30 in ln:
            return t.lines()[i + 1]
    raise AssertionError(f"no table header rule on screen\n{t.dump()}")


def run(t):
    # the author cell only ever appears in a table row, so it is the
    # honest witness for "is the Author column drawn" — the word
    # "Author" itself is in the panel too
    require("Cabrera, +1" in t.text(), "expected the author column drawn at startup", t)

    t.send("|")
    t.wait_for("Table configuration", what="the table panel")
    require(_panel_line(t, "Year"), "no Year row in the panel", t)
    require(_panel_line(t, "Key"), "no Key row in the panel", t)

    # the panel has the arrows now: three downs land on Author (the rows
    # are Metric, ↓, Year, Author, Title, Key — the ● column is left out
    # with no manuscript, having neither a label nor anything to sort)
    t.key("down")
    t.key("down")
    t.key("down")
    t.send(" ")
    t.wait_for(
        lambda: "Cabrera, +1" not in t.text(),
        what="the Author column gone from the table",
    )
    require("· Author" in t.text(), "the panel should mark Author hidden", t)

    # …and that is on disk, so it would survive a restart
    with open(os.path.join(t.state_dir, "state.json")) as f:
        cfg = json.load(f).get("columns", {})
    require(
        cfg.get("library", {}).get("visible") == {"author": False},
        f"expected library.visible == {{'author': False}}, got {cfg!r}",
        t,
    )

    # ←/→ resize the selected column, and the width shown is the real one
    t.key("up")  # back to Year
    require("6" in _panel_line(t, "Year"), f"expected Year at width 6: {_panel_line(t, 'Year')!r}", t)
    t.key("right")
    t.key("right")
    t.wait_for(
        lambda: "8" in _panel_line(t, "Year"),
        what="Year widened to 8",
    )

    # sorting a column that is not drawn: hide Key, then sort by it
    for _ in range(3):
        t.key("down")  # Year -> Author -> Title -> Key
    t.send(" ")
    # the cite key is on the pub card too, so ask the table row itself
    t.wait_for(
        lambda: "Cabrera2024" not in _first_data_row(t),
        what="the Key column gone from the table row",
    )
    t.send("s")
    # cite keys ascending puts Andersson2021 first; the default Year ▼
    # had Cabrera (2024) on top, so this cannot pass by accident. With
    # Author and Key both hidden the title is what names the row.
    t.wait_for(
        lambda: "Relativistic jet braking" in _first_data_row(t),
        what="the library re-sorted by a hidden column",
    )

    # Esc gives the arrows back to the table without closing the panel
    t.send("\x1b")
    t.wait_quiet()
    require("Table configuration" in t.text(), "Esc should not close the panel", t)
    # by cite key ascending the second paper is Baxter2019; the card's
    # author line is card-only, so it witnesses the table cursor moving
    t.key("down")
    t.wait_for("Baxter · 2019", what="the table cursor moving once focus is back")

    # Tab is the reversible form of the same thing: it toggles which of
    # the two arrow-navigable areas the arrows drive
    t.send("\t")
    t.wait_quiet()
    t.key("down")
    # back in the panel, so the arrows move its cursor and leave the
    # table's alone — the card still shows the same paper
    t.wait_for(
        lambda: "Baxter · 2019" in t.text(),
        what="the table cursor staying put while the panel has focus",
    )
    t.send("\t")
    t.wait_quiet()
    t.key("down")
    t.wait_for(
        lambda: "Baxter · 2019" not in t.text(),
        what="the table cursor moving again once Tab hands focus back",
    )

    # a column whose width is not the user's to set says which kind of
    # not-yours it is, rather than one generic refusal for both
    t.send("|")
    t.wait_gone("Table configuration")
    t.send("|")
    t.wait_for("Table configuration", what="the panel reopened, focused")
    for _ in range(8):
        t.key("up")  # to the top of the list, wherever the cursor was left
    t.send(" ")
    t.wait_for(lambda: "✓ Metric" in t.text(), what="the metric column switched on")
    t.key("right")
    t.wait_for(
        "the metric swatch is one cell wide",
        what="the metric column explaining its fixed size",
    )
    # Title still holds the leftover width — Author and Key were hidden
    # above, but it was not
    for _ in range(4):
        t.key("down")  # Metric -> ↓ -> Year -> Author -> Title
    t.key("right")
    t.wait_for(
        "taking the leftover width",
        what="the flex column explaining that its size is derived",
    )

    # the list scrolls when it outgrows the pane. Without this the cursor
    # walked off the bottom into rows that were never drawn — it looked
    # like the selection descending past the final row into nothing.
    t.resize(150, 10)
    for _ in range(8):
        t.key("down")
    t.wait_for(
        lambda: _panel_line(t, "Key") != "",
        what="the last row still drawn once the list scrolls",
    )
    require(
        "Table configuration" not in t.text(),
        "the heading should have scrolled off to make room",
        t,
    )
    # and back: with room again the whole list is shown from the top
    t.resize(150, 40)
    t.wait_for("Table configuration", what="the heading back once the list fits")

    # and | closes it
    t.send("|")
    t.wait_gone("Table configuration")
