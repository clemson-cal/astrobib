"""m on a query page toggles the imported twin's manuscript membership
(and, adding, sets its priority to 1.0) instead of doing nothing."""

import json
import os

from driver import require

DESCRIPTION = "query page: m toggles the twin's manuscript membership"

MANUSCRIPT = {
    "main.tex": "\\documentclass{article}\n\\begin{document}\nNothing cited yet.\n\\end{document}\n",
}

_IMPORTED = {
    "bibcode": "2021ApJ...912...77A",
    "title": ["Relativistic jet braking in dense circumstellar environments"],
    "author": ["Andersson, Freya", "Blomqvist, Karin"],
    "year": "2021",
    "abstract": "Already in the library.",
    "doi": ["10.3847/1538-4357/abf123"],
    "identifier": ["arXiv:2103.04156"],
    "citation_count": 11,
    "pub": "ApJ",
    "volume": "912",
    "issue": "",
    "page": ["77"],
}

KEY = "Andersson2021pombz"


def _pre_launch(state_dir):
    # tabs are keyed by manuscript root; the app sees the resolved cwd,
    # so store both spellings of the scratch path
    scratch = os.path.dirname(state_dir)
    ms = os.path.join(scratch, "home", "ms")
    tab = {"id": "q1", "query": "jets", "label": "jets", "limit": 20, "created": 0}
    contexts = {ms: [tab], os.path.realpath(ms): [tab]}
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": contexts}, f)
    with open(os.path.join(state_dir, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"q1": [_IMPORTED]}}, f)


PRE_LAUNCH = _pre_launch


def _priority(t):
    """The stored priority of KEY, or None (written on the idle tick)."""
    path = os.path.join(t.state_dir, "metrics.json")
    if not os.path.exists(path):
        return None
    with open(path) as f:
        try:
            doc = json.load(f)
        except ValueError:
            return None
    return doc.get("papers", {}).get(KEY, {}).get("priority")


def run(t):
    ms_copy = os.path.join(t.cwd, "bib", KEY + ".bib")
    t.wait_for("Manuscript", what="manuscript capsule")
    t.send("]")
    t.send("]")  # 0 Library, 1 Manuscript, 2 the restored query
    t.wait_for(lambda: "Relativistic jet braking" in t.text(), what="imported query row")
    require("add to manuscript" in t.text(), "card should offer the ms toggle", t)
    require(not os.path.exists(ms_copy), "manuscript copy exists before the test", t)

    # m acts on the imported twin
    t.send("m")
    t.wait_for(lambda: "Added 1 paper(s) to manuscript db" in t.text(), what="add feedback")
    require(os.path.exists(ms_copy), "m did not write the manuscript copy", t)
    require("in manuscript" in t.text(), "card should show membership after m", t)
    # entering the manuscript means top priority, flushed on the idle tick
    t.wait_for(lambda: _priority(t) == 1.0, what="priority 1.0 in metrics.json")

    # and m again takes it back out
    t.send("m")
    t.wait_for(lambda: "Removed 1 paper(s) from manuscript db" in t.text(), what="remove feedback")
    require(not os.path.exists(ms_copy), "second m did not remove the manuscript copy", t)
