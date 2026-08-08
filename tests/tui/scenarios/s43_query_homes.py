"""A saved query has one of two homes, and can be moved between them.

Queries used to be filed under the manuscript you happened to be
standing in, so walking away from a `bib/` took them off screen — and an
empty `bib/`, which git cannot even record, was enough to switch the
set. Now a session reads both: the global queries, visible from every
directory, then the active manuscript's own. The strip marks the
boundary, the footer names the home of the query you are on, and H (or
clicking the other side of that control) moves it.

Everything here is seeded and offline. The citations() query at the end
never reaches ADS — the point is only where its tab is filed, and a tab
appears the moment the query is sent rather than when results land.
"""

import json
import os

from driver import require

DESCRIPTION = "saved queries have a global or manuscript home, and move"

MANUSCRIPT = {
    "main.tex": "\\documentclass{article}\n\\begin{document}\nNothing cited yet.\n\\end{document}\n",
}

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

# a manuscript that is not this one: its queries are nobody else's to
# rewrite, and every save here must carry them across untouched
OTHER = "/somewhere/else/another-paper"


def _tab(tid, query, label):
    return {"id": tid, "query": query, "label": label, "limit": 20, "created": 0}


def _pre_launch(state_dir):
    scratch = os.path.dirname(state_dir)
    ms = os.path.join(scratch, "home", "ms")
    local = _tab("l1", "magnetar bursts", "magnetars")
    # the app resolves symlinks in cwd and Python does not, so the
    # manuscript's own key is written under both spellings
    contexts = {
        "global": [_tab("g1", "kilonova ejecta", "kilonovae")],
        OTHER: [_tab("x1", "someone else's query", "theirs")],
        ms: [local],
        os.path.realpath(ms): [local],
    }
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": contexts}, f)
    cache = os.path.join(scratch, "home", ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"g1": [_ARTICLE], "l1": [_ARTICLE]}}, f)


PRE_LAUNCH = _pre_launch


def _contexts(t):
    with open(os.path.join(t.state_dir, "tabs.json")) as f:
        return json.load(f)["contexts"]


def _ids(contexts, key):
    return [x["id"] for x in contexts.get(key, [])]


def _ms_keys(t):
    """Both spellings of the manuscript's own key, since only the one the
    app resolved to is rewritten; the other keeps whatever it was seeded
    with and must not be read as the live set."""
    return {t.cwd, os.path.realpath(t.cwd)}


def _local_ids(t):
    c = _contexts(t)
    return {k: _ids(c, k) for k in _ms_keys(t) if k in c}


def run(t):
    t.wait_for("kilonovae", what="the global query's capsule")
    require("magnetars" in t.text(), f"the manuscript's own query should show too:\n{t.text()}", t)

    # both groups on one strip row, global first, with the mark between
    row = t.row_of("kilonovae")
    strip = t.lines()[row]
    require(
        strip.index("kilonovae") < strip.index("magnetars"),
        f"global queries should come before the manuscript's own:\n{strip}",
        t,
    )
    between = strip[strip.index("kilonovae") : strip.index("magnetars")]
    require("│" in between, f"the two groups should be marked apart:\n{strip}", t)

    # the footer names the home of the query you are on
    t.send("]")
    t.send("]")  # 0 Library, 1 Manuscript, 2 the global query
    t.wait_for(lambda: "query" in t.lines()[-1], what="the query-home control")
    require(
        "local" in t.lines()[-1] and "global" in t.lines()[-1],
        f"the control should offer both homes:\n{t.lines()[-1]}",
        t,
    )

    # H moves the manuscript's own query to the global set
    t.send("]")
    t.wait_for(lambda: "magnetar bursts" in t.text() or "magnetars" in t.text(), what="local query")
    t.send("H")
    t.wait_for(lambda: "moved to the global queries" in t.text(), what="the move to global")
    require(
        sorted(_ids(_contexts(t), "global")) == ["g1", "l1"],
        f"the query should have joined the global set:\n{_contexts(t)}",
        t,
    )
    # only the spelling the app resolved to is rewritten; the other is a
    # key this session never read and must not have invented a write for
    require(
        any(v == [] for v in _local_ids(t).values()),
        f"and left the manuscript's own:\n{_local_ids(t)}",
        t,
    )
    # a save writes this session's two sets and nothing else
    require(
        _ids(_contexts(t), OTHER) == ["x1"],
        f"another manuscript's queries must survive untouched:\n{_contexts(t)}",
        t,
    )
    # with both now global there is nothing to mark apart
    require(
        "│" not in t.lines()[t.row_of("kilonovae")],
        "one group needs no separator",
        t,
    )

    # clicking the other side of the footer control moves it back
    x, y = t.find("local")
    t.click(x, y)
    t.wait_for(lambda: "moved to this manuscript" in t.text(), what="the move back")
    require(
        _ids(_contexts(t), "global") == ["g1"],
        f"the query should have left the global set:\n{_contexts(t)}",
        t,
    )
    require(
        any(v == ["l1"] for v in _local_ids(t).values()),
        f"and joined the manuscript's own:\n{_local_ids(t)}",
        t,
    )

    # a citations(...) query is about one paper, so it is filed with the
    # paper — it never reaches ADS here, but its tab appears at once
    t.send("[")
    t.wait_for(lambda: "Relativistic jet braking" in t.text(), what="a query result to ask about")
    t.send("C")
    t.wait_for(lambda: "cites→" in t.text(), what="the citations tab")
    strip = t.lines()[t.row_of("kilonovae")]
    require(
        strip.index("│") < strip.index("cites→"),
        f"a citations query belongs with the manuscript:\n{strip}",
        t,
    )
