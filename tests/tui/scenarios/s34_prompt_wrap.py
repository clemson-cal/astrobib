"""A long query wraps instead of scrolling off the end.

ADS queries grow: a topical feed with spelling variants and exclusions
runs past 200 characters, and a one-row prompt that scrolls horizontally
shows you a window onto your own query with no way to see the rest.

The text stays *one logical line* throughout. There is no newline in it
and none is sent to ADS, which has no use for one — the wrapping is
display only. It breaks at the column rather than at word boundaries, so
the cursor maps to a row and a column by division: exact, and with no
reflow surprise while typing in the middle.

A short query still renders on one row, exactly as before. That is the
common case and it should not pay for the long one.
"""

import json
import os

from driver import require

DESCRIPTION = "a long query wraps across rows; a short one does not"

COLS = 100
ROWS = 24


def _pre_launch(state_dir):
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)


PRE_LAUNCH = _pre_launch


def _prompt_rows(t):
    """Screen rows carrying the prompt, from its label down."""
    start = next(i for i, l in enumerate(t.lines()) if "ADS query:" in l)
    return t.lines()[start:]


def run(t):
    footer = len(t.lines()) - 1

    t.send("S")
    t.wait_for("ADS query:", what="the ADS query prompt")

    # a short query is one row, with its parameters beside it — the
    # layout this has always had
    t.send("kilonova")
    t.wait_for(
        lambda: "kilonova" in t.lines()[footer] and "ADS returns" in t.lines()[footer],
        what="a short query sharing its row with the parameters",
    )

    # a long one wraps, and the parameters move to a row of their own
    t.send("A" * 200)
    t.wait_for(
        lambda: len(_prompt_rows(t)) > 2,
        what="the prompt growing past one row",
    )
    rows = _prompt_rows(t)
    require(
        "ADS returns" not in rows[0],
        f"the parameters should leave the text's row: {rows[0]!r}",
        t,
    )
    require(
        any("ADS returns" in r for r in rows),
        "the parameters should still be shown, on their own row",
        t,
    )
    # continuation rows are indented under the label, not relabelled
    require(
        "ADS query:" not in rows[1],
        f"only the first row wears the label: {rows[1]!r}",
        t,
    )

    # the cursor is on the last text row, not stranded on the first
    text_rows = [i for i, r in enumerate(rows) if "ADS returns" not in r]
    top = next(i for i, l in enumerate(t.lines()) if "ADS query:" in l)
    require(
        t.screen.cursor.y == top + text_rows[-1],
        f"cursor should be on the last text row: got y={t.screen.cursor.y}, "
        f"expected {top + text_rows[-1]}",
        t,
    )

    # walking left moves it back up a row, so the mapping is not
    # one-directional bookkeeping that only happens to work at the end
    was = t.screen.cursor.y
    for _ in range(120):
        t.key("left")
    t.wait_for(
        lambda: t.screen.cursor.y < was,
        what="the cursor moving up a row as it walks back through the text",
    )

    # and it all retracts, giving the table its rows back
    t.key("esc")
    t.wait_gone("ADS query:")
    require(
        "ADS returns" not in t.lines()[footer],
        "the prompt should be gone entirely",
        t,
    )
