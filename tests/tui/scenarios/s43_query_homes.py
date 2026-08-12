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
        # two global queries, so a move out and back has somewhere wrong
        # to land: without a stable order the first would come to rest
        # behind the second
        "global": [
            _tab("g1", "kilonova ejecta", "kilonovae"),
            _tab("g2", "pulsar timing", "pulsars"),
        ],
        OTHER: [_tab("x1", "someone else's query", "theirs")],
        ms: [local],
        os.path.realpath(ms): [local],
    }
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": contexts}, f)
    cache = os.path.join(scratch, "home", ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump(
            {"version": 1, "tabs": {"g1": [_ARTICLE], "g2": [_ARTICLE], "l1": [_ARTICLE]}}, f
        )


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


def _strip(t):
    """The capsule strip as one string, however many rows it takes.

    It wraps when the capsules outgrow the width, and each query capsule
    now spends two more cells on its ✕ — so an assertion about the order
    of the capsules has to read the whole strip rather than its first
    row. "+ new" is the last capsule, which is what ends it.
    """
    rows = []
    for line in t.lines()[t.row_of("kilonovae") :]:
        rows.append(line.rstrip())
        if "+ new" in line:
            break
    return "  ".join(rows)


def run(t):
    t.wait_for("kilonovae", what="the global query's capsule")
    require("magnetars" in t.text(), f"the manuscript's own query should show too:\n{t.text()}", t)

    # both groups on the strip, global first, with the mark between
    strip = _strip(t)
    require(
        strip.index("kilonovae") < strip.index("magnetars"),
        f"global queries should come before the manuscript's own:\n{strip}",
        t,
    )
    between = strip[strip.index("kilonovae") : strip.index("magnetars")]
    require("│" in between, f"the two groups should be marked apart:\n{strip}", t)

    # the footer names where the query you are on is visible — one label
    # stating the home it is in, not two sides leaving you to guess which
    # of them is the state and which is the button
    t.send("]")
    t.send("]")  # 0 Library, 1 Manuscript, 2 the global query
    t.wait_for(lambda: "everywhere" in t.lines()[-1], what="the query-home indicator")
    require(
        "this paper" not in t.lines()[-1],
        f"only the home it is in should be named:\n{t.lines()[-1]}",
        t,
    )

    # H twice is a no-op. Both halves matter: the query has to come back
    # to where it was in the strip rather than to the end of its group,
    # and the gesture must never leave you on a different tab — which is
    # what the move reporting the same query's name twice proves, since
    # it always acts on the one you are on.
    before = _strip(t)
    t.send("H")
    t.wait_for(lambda: "'kilonovae' moved to this manuscript" in t.text(), what="out")
    # the indicator's words change, so the move is visible without
    # reading a colour
    require(
        "this paper" in t.lines()[-1] and "everywhere" not in t.lines()[-1],
        f"the indicator should now name the manuscript:\n{t.lines()[-1]}",
        t,
    )
    t.send("H")
    t.wait_for(lambda: "'kilonovae' moved to the global queries" in t.text(), what="and back")
    require(
        "everywhere" in t.lines()[-1],
        f"and read as global again:\n{t.lines()[-1]}",
        t,
    )
    require(
        _strip(t) == before,
        f"H twice should put the strip back as it was:\n{before}\n{_strip(t)}",
        t,
    )

    # H moves the manuscript's own query to the global set
    t.send("]")
    t.send("]")  # past the second global query, onto the manuscript's own
    t.wait_for(lambda: "magnetar bursts" in t.text() or "magnetars" in t.text(), what="local query")
    t.send("H")
    t.wait_for(lambda: "moved to the global queries" in t.text(), what="the move to global")
    require(
        _ids(_contexts(t), "global") == ["g1", "g2", "l1"],
        f"the query should have joined the global set, at its end:\n{_contexts(t)}",
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
        "│" not in _strip(t),
        "one group needs no separator",
        t,
    )

    # clicking the indicator moves the query back
    x, y = t.find("everywhere")
    t.click(x, y)
    t.wait_for(lambda: "moved to this manuscript" in t.text(), what="the move back")
    require(
        _ids(_contexts(t), "global") == ["g1", "g2"],
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
    strip = _strip(t)
    require(
        strip.index("│") < strip.index("cites→"),
        f"a citations query belongs with the manuscript:\n{strip}",
        t,
    )
