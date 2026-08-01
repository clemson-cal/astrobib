"""A query tab has two clocks, and they disagree.

`year` is when a paper was published; `entry_date` is when ADS indexed
the record. A 2015 paper indexed last month is new by the second measure
and old by the first, which is the whole point of a date-sorted query
tab: it reads as a feed of postings, not of publications.

The fixtures are built so the two orders are exact opposites, so sorting
by one and then the other cannot accidentally agree. This also proves
entry_date survives the query cache round-trip, since these results are
restored from cache and never touch the network.
"""

import json
import os

from driver import require

DESCRIPTION = "Entered column: postings clock, distinct from Year"

# published 2020, indexed 2024 — and published 2015, indexed 2026. Newest
# by year is the first; newest by entry date is the second.
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
    },
    {
        "bibcode": "2015TestB...9..44Q",
        "title": ["Older cached companion paper"],
        "author": ["Quist, M."],
        "year": "2015",
        "abstract": "Also cached.",
        "doi": [],
        "identifier": [],
        "citation_count": 112,
        "entry_date": "2026-01-19T00:00:00Z",
        "pub": "TestJ",
        "volume": "9",
        "issue": "",
        "page": ["44"],
    },
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


def _first_data_row(t):
    for i, ln in enumerate(t.lines()):
        if ln[:20] == "─" * 20:
            return t.lines()[i + 1]
    raise AssertionError(f"no table header rule on screen\n{t.dump()}")


def run(t):
    t.send("]")  # no manuscript here, so the query is scope 1
    t.wait_for("Entered", what="the Entered column header")

    # both dates render, which means entry_date round-tripped through
    # the on-disk query cache rather than being dropped on the way
    t.wait_for("2024-03-02", what="the 2020 paper's entry date")
    require("2026-01-19" in t.text(), "the 2015 paper's entry date is missing", t)

    # the tab opens on its default Year ▼: newest *published* first
    require("Year ▼" in t.text(), "query should start at Year ▼", t)
    require(
        "2020" in _first_data_row(t),
        f"expected the 2020 paper on top under Year ▼, got: {_first_data_row(t)!r}",
        t,
    )

    # switching to the posting clock inverts that, because the older
    # paper was indexed more recently
    x, y = t.find("Entered")
    t.click(x, y)
    t.wait_for("Entered ▼", what="Entered marker, descending — a date column leads newest-first")
    t.wait_for(
        lambda: "2026-01-19" in _first_data_row(t),
        what="the most recently indexed record on the first data row",
    )
    require("Year ▼" not in t.text(), "the marker should have left the Year column", t)

    # the tab's ADS sort — which records come back at all — is the
    # posting clock too, and is stored on the tab
    with open(os.path.join(t.state_dir, "tabs.json")) as f:
        tabs = json.load(f)
    saved = [tab for ctx in tabs["contexts"].values() for tab in ctx]
    require(saved, "no saved tabs written back", t)
    require(
        all(s.get("ads_sort") == "entry_date desc" for s in saved),
        f"tab ads_sort should select by entry date, got {[s.get('ads_sort') for s in saved]}",
        t,
    )
    require(
        any(s.get("sort_col") == "entered" and s.get("sort_asc") is False for s in saved),
        f"display sort should be entered/descending, got "
        f"{[(s.get('sort_col'), s.get('sort_asc')) for s in saved]}",
        t,
    )
