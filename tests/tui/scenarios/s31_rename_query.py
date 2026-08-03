"""N names a saved query, and the confirmation is seen.

A saved query's capsule is labelled from the query text, which says what
you typed rather than what you were doing. N replaces that with a name,
which persists — including across edits to the query itself, since a
name you typed is a decision and re-deriving would quietly discard it.

The confirmation is the other half. The prompt takes the footer, so a
note raised while it is up would be drawn over by the prompt and never
seen; the prompt closes *first* and the note lands on the footer it just
gave back. That ordering is what this pins.
"""

import json
import os

from driver import require

DESCRIPTION = "N names a query; the confirmation lands on the footer"

_ARTICLES = [
    {
        "bibcode": "2020TestA...1..1Z",
        "title": ["A cached result about kilonovae"],
        "author": ["Cachette, Q."],
        "year": "2020",
        "abstract": "From the cache.",
        "doi": [],
        "identifier": [],
        "citation_count": 7,
        "entry_date": "2024-03-02T00:00:00Z",
        "pub": "TestJ",
        "volume": "1",
        "issue": "",
        "page": ["1"],
    }
]


def _pre_launch(state_dir):
    cache = os.path.join(os.path.dirname(state_dir), "home", ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    tab = {"id": "tt1", "query": "kilonova", "label": "kilonova", "limit": 20, "created": 0}
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {"global": [tab]}}, f)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"tt1": _ARTICLES}}, f)


PRE_LAUNCH = _pre_launch


def _saved_labels(t):
    with open(os.path.join(t.state_dir, "tabs.json")) as f:
        tabs = json.load(f)
    return [x["label"] for ctx in tabs["contexts"].values() for x in ctx]


def run(t):
    footer = len(t.lines()) - 1

    # the library and manuscript scopes are not queries and say so
    t.send("N")
    t.wait_for("only a saved query can be renamed", what="the refusal on a fixed scope")

    t.send("]")
    t.wait_for("A cached result", what="the restored query scope")
    require("kilonova" in t.lines()[1], "expected the derived capsule label", t)

    # the prompt opens pre-filled with the current name
    t.send("N")
    t.wait_for("name this query:", what="the rename prompt")
    require("kilonova" in t.lines()[footer], "the prompt should pre-fill the current name", t)

    for _ in range(len("kilonova")):
        t.key("backspace")
    t.send("LRD reading")
    t.key("enter")

    # the capsule takes the name…
    t.wait_for(lambda: "LRD reading" in t.lines()[1], what="the renamed capsule")
    # …and the confirmation is on the footer, the prompt having closed
    # before it was raised
    t.wait_for(
        lambda: "query named 'LRD reading'" in t.lines()[footer],
        what="the confirmation on the footer the prompt gave back",
    )
    require(
        "name this query:" not in t.lines()[footer],
        "the prompt should be gone by the time the confirmation shows",
        t,
    )
    require(
        _saved_labels(t) == ["LRD reading"],
        f"the name should be persisted, got {_saved_labels(t)}",
        t,
    )

    # an empty name restores the one derived from the query text
    t.send("N")
    t.wait_for("name this query:", what="the rename prompt again")
    for _ in range(len("LRD reading")):
        t.key("backspace")
    t.key("enter")
    t.wait_for(
        lambda: "name cleared" in t.lines()[footer],
        what="the note explaining the derived name is back",
    )
    t.wait_for(lambda: "kilonova" in t.lines()[1], what="the derived capsule label back")

    # Esc says so rather than leaving you wondering whether it took
    t.send("N")
    t.wait_for("name this query:", what="the rename prompt once more")
    t.send("something else")
    t.key("esc")
    t.wait_for("rename cancelled", what="the cancellation note")
    require("kilonova" in t.lines()[1], "the name should be untouched after Esc", t)
