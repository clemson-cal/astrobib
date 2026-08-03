"""E edits a saved query in place, everything the prompt owns at once.

S always composes a *new* query, so a saved one's text used to be
write-once: changing it meant replacing the tab and losing its name, its
display sort and its cached results with it. E reopens the same prompt
over the tab you are looking at, pre-filled with its text, its result
limit and what it returns.

The network is not available here, so this covers reaching the editor
and what it is pre-filled with. The replacement itself only lands when
ADS answers — deliberately, since a failed search should not overwrite a
working tab — and that path needs RUN_ADS_TESTS.
"""

import json
import os

from driver import require

DESCRIPTION = "E edits a query in place, pre-filled from the tab"

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
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)
    # limit 50, so the editor is seen to pre-fill it rather than default
    tab = {"id": "tt1", "query": "kilonova", "label": "kilonova", "limit": 50, "created": 0}
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {"global": [tab]}}, f)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"tt1": _ARTICLES}}, f)


PRE_LAUNCH = _pre_launch


def run(t):
    footer = len(t.lines()) - 1

    # the affordance is a property of a query page, so the library has none
    require(
        "edit query (E)" not in t.lines()[footer],
        "the library scope should offer no query editor",
        t,
    )
    t.send("E")
    t.wait_for("only a saved query can be edited", what="the refusal off a query")

    t.send("]")
    t.wait_for("A cached result", what="the restored query scope")
    t.wait_for(
        lambda: "edit query (E)" in t.lines()[footer],
        what="the editor affordance on a query page",
    )

    # it looks clickable, and answers a click
    ex = t.lines()[footer].index("edit query") + 2
    t.hover(ex, footer)
    t.wait_for(
        lambda: t.screen.buffer[footer][ex].underscore,
        what="the affordance underlining under the pointer",
    )
    t.click(ex, footer)
    t.wait_for("edit query:", what="the editor opened by clicking")

    # pre-filled with every part of the query the prompt owns, and the
    # limit proves it is the tab's rather than the default for a new one
    require("kilonova" in t.lines()[footer], "the text should be pre-filled", t)
    require(
        "ADS returns 50 (↑↓)" in t.lines()[footer],
        f"the tab's own limit should be pre-filled: {t.lines()[footer]!r}",
        t,
    )
    require(
        "newest posting (⌃r)" in t.lines()[footer],
        "what the tab returns should be pre-filled",
        t,
    )

    # and it is the editor, not a new query — S says the other thing
    require(
        "ADS query:" not in t.lines()[footer],
        "the editor should not present itself as a new query",
        t,
    )
    t.key("esc")
    t.wait_gone("edit query:")

    # E reaches the same editor
    t.send("E")
    t.wait_for("edit query:", what="the editor opened by key")
    # wait for it to close before the next key: an ESC written back to
    # back with a letter arrives in one pty read and parses as alt+letter
    t.key("esc")
    t.wait_gone("edit query:")

    # S still composes a new one, from blank
    t.send("S")
    t.wait_for("ADS query:", what="S composing a new query")
    require(
        "kilonova" not in t.lines()[footer],
        "S should not pre-fill from the query being viewed",
        t,
    )
    require(
        "ADS returns 20 (↑↓)" in t.lines()[footer],
        f"a new query should start at the default limit: {t.lines()[footer]!r}",
        t,
    )
    t.key("esc")
