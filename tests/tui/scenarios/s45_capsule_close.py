"""A query capsule says how it is closed, and closes when you say so.

Closing a scope was ⌃w and nothing else: written down in the README, and
nowhere the app itself would tell you. Now every query capsule carries a
✕ — the permanent scopes carry none, which is how the strip says which
capsules go away — the ✕ closes the query it is drawn on rather than the
one you are standing in, and ⌃w is a row on the keys panel like any
other key.

Seeded and offline: two saved queries with cached results, so the strip
has capsules without a round trip to ADS.
"""

import json
import os

from driver import require

DESCRIPTION = "a query capsule's ✕ closes it; ⌃w is on the keys panel"

CTRL_W = b"\x17"

_ARTICLE = {
    "bibcode": "2021ApJ...912...77A",
    "title": ["Relativistic jet braking in dense circumstellar environments"],
    "author": ["Andersson, Freya"],
    "year": "2021",
    "abstract": "Seeded, not fetched.",
    "doi": ["10.3847/1538-4357/abf123"],
    "identifier": ["arXiv:2103.04156"],
    "citation_count": 11,
    "pub": "ApJ",
    "volume": "912",
    "issue": "",
    "page": ["77"],
}


def _tab(tid, query, label):
    return {"id": tid, "query": query, "label": label, "limit": 20, "created": 0}


def _pre_launch(state_dir):
    scratch = os.path.dirname(state_dir)
    contexts = {
        "global": [
            _tab("g1", "kilonova ejecta", "kilonovae"),
            _tab("g2", "pulsar timing", "pulsars"),
        ]
    }
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": contexts}, f)
    cache = os.path.join(scratch, "home", ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"g1": [_ARTICLE], "g2": [_ARTICLE]}}, f)


PRE_LAUNCH = _pre_launch


def _saved(t):
    with open(os.path.join(t.state_dir, "tabs.json")) as f:
        return [x["id"] for x in json.load(f)["contexts"].get("global", [])]


def _mark_of(t, label):
    """The cell of the ✕ on a named capsule: one space past its label."""
    x, y = t.find(label)
    return x + len(label) + 1, y


def run(t):
    t.wait_for("kilonovae", what="the saved queries' capsules")
    strip = t.lines()[t.row_of("kilonovae")]

    # the library is permanent, so it carries no close mark; a capsule
    # with a ✕ is one that goes away
    require(
        "✕" not in strip[strip.index("Library") : strip.index("kilonovae")],
        f"the Library capsule should carry no ✕:\n{strip}",
        t,
    )
    for label in ("kilonovae", "pulsars"):
        x, y = _mark_of(t, label)
        require(
            t.lines()[y][x] == "✕",
            f"'{label}' should carry a ✕ one cell past its label:\n{strip}",
            t,
        )

    # the mark lights under the pointer and the footer says what it does
    x, y = _mark_of(t, "pulsars")
    t.hover(x, y)
    t.wait_for(
        lambda: t.screen.buffer[y][x].underscore,
        what="the ✕ to light under the pointer",
    )
    t.wait_for("close this query", what="the footer naming what the ✕ does")

    # clicking it closes the query it is drawn on — which is not the one
    # we are standing in, since nothing has moved us off the library
    t.click(x, y)
    t.wait_gone("pulsars")
    require("kilonovae" in t.text(), f"the other query should be untouched:\n{t.text()}", t)
    t.wait_for(lambda: _saved(t) == ["g1"], what="the closed query to leave tabs.json")

    # ⌃w on a permanent scope has nothing to close, and says so rather
    # than doing nothing at all
    t.send(CTRL_W)
    t.wait_for("only a query capsule closes", what="⌃w explaining itself on the library")
    require("kilonovae" in t.text(), "⌃w must not have closed anything", t)

    # and it closes the query you are on, from the keyboard
    t.send("]")
    # the footer's ambient call-to-action is per scope, so it is proof
    # the move landed: only a query scope offers to edit its query
    t.wait_for(lambda: "edit query" in t.lines()[-1], what="the query scope")
    t.send(CTRL_W)
    t.wait_gone("kilonovae")
    t.wait_for(lambda: _saved(t) == [], what="the last query to leave tabs.json")

    # the keys panel names the chord, so none of this has to be known in
    # advance from the README
    t.send("?")
    t.wait_for(lambda: "this cheat-sheet" in t.text(), what="keys panel")
    require(
        "⌃w" in t.text() and "close this query" in t.text(),
        f"⌃w should be a row on the keys panel:\n{t.text()}",
        t,
    )
