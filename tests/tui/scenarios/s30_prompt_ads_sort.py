"""The query prompt shows, and sets, what ADS will return.

Two properties of a query are not query *syntax* — the result limit and
what ADS returns ride alongside `q` as the `rows` and `sort` API
parameters — and both belong where the query is composed. What ADS
returns had no home at all before: it could only be reached after the
tab existed, so a first run could never be anything but the default.

It shows as one glyph, because the query text deserves the line. Rolling
it names the mode and the chord in place of the standing hint, and the
chord is ⌃r so it cannot be confused with typing.
"""

import json
import os

from driver import require

DESCRIPTION = "query prompt: the ADS-returns glyph, its chord and its click"

# (glyph, name) in the order ⌃r steps them, wrapping
MODES = [
    ("⇓", "newest posting"),
    ("↓", "newest published"),
    ("≫", "most cited"),
    ("≈", "most relevant"),
]
CTRL_R = "\x12"


def _pre_launch(state_dir):
    # a token, so S composes a query instead of opening first-run setup
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)


PRE_LAUNCH = _pre_launch


def run(t):
    footer = len(t.lines()) - 1
    t.send("S")
    t.wait_for("ADS query:", what="the ADS query prompt")

    # typing goes to the query; the parameters sit past it
    t.send("little red dots")
    t.wait_for(
        lambda: "little red dots" in t.lines()[footer],
        what="the typed query",
    )
    require("n=20" in t.lines()[footer], "the prompt should show the result limit", t)
    require(
        MODES[0][0] in t.lines()[footer],
        f"the prompt should open on {MODES[0][0]!r}: {t.lines()[footer]!r}",
        t,
    )
    # at rest it is a glyph only — the name costs a line the query wants
    require(
        MODES[0][1] not in t.lines()[footer],
        "the mode name should not be spelled out until it is rolled over",
        t,
    )

    # rolling it names the mode and the chord, in place of the hint
    gx = t.lines()[footer].index(MODES[0][0])
    t.hover(gx, footer)
    t.wait_for(
        f"ADS returns {MODES[0][1]}",
        what="the rollover naming the mode",
    )
    require("⌃r" in t.lines()[footer], "the rollover should name the chord too", t)

    # ⌃r steps it, and wraps
    for glyph, name in MODES[1:] + [MODES[0]]:
        t.send(CTRL_R)
        t.wait_for(
            lambda n=name: f"ADS returns {n}" in t.lines()[footer],
            what=f"⌃r stepping to {name!r}",
        )
        require(glyph in t.lines()[footer], f"expected the {name!r} glyph {glyph!r}", t)

    # and it is clickable, being a thing that looks clickable when rolled
    t.click(gx, footer)
    t.wait_for(
        f"ADS returns {MODES[1][1]}",
        what="clicking the glyph stepping the mode",
    )

    # ↑ still steps the limit, so the two parameters do not fight
    t.key("up")
    t.wait_for(
        lambda: "n=50" in t.lines()[footer] and MODES[1][0] in t.lines()[footer],
        what="the limit stepping while the mode holds",
    )

    # Esc leaves without querying — this scenario has no network
    t.key("esc")
    t.wait_gone("ADS query:")
