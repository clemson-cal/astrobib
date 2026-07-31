"""Delete means three different things by context, so the confirm modal
states which one, in plain words, before it happens."""

import json
import os

from driver import require

DESCRIPTION = "confirm modal states the removal's consequence"

MANUSCRIPT = {
    "main.tex": (
        "\\documentclass{article}\n\\begin{document}\n"
        "As shown \\cite{Andersson2021pombz}.\n\\end{document}\n"
    ),
}

_IMPORTED = {
    "bibcode": "2021ApJ...912...77A",
    "title": ["Relativistic jet braking in dense circumstellar environments"],
    "author": ["Andersson, Freya", "Blomqvist, Karin"],
    "year": "2021",
    "abstract": "Already in the library, and cited by the manuscript.",
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
    # tabs are keyed by manuscript root; the app sees the resolved cwd
    ms = os.path.join(os.path.dirname(state_dir), "home", "ms")
    tab = {"id": "q1", "query": "jets", "label": "jets", "limit": 20, "created": 0}
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {ms: [tab], os.path.realpath(ms): [tab]}}, f)
    cache = os.path.join(os.path.dirname(state_dir), "home", ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"q1": [_IMPORTED]}}, f)


PRE_LAUNCH = _pre_launch


def run(t):
    ms_copy = os.path.join(t.cwd, "bib", KEY + ".bib")
    t.wait_for("Manuscript", what="manuscript capsule")

    # 1. the ordinary library case: both tiers go
    t.wait_for(lambda: "Cabrera" in t.text(), what="library rows")
    t.key("delete")
    t.wait_for("removes from both tiers")
    require("Cabrera2024txuze" in t.text(), "modal should list its target", t)
    t.key("esc")
    t.wait_gone("removes from both tiers")

    # put the cited paper in the manuscript db, from the query page
    t.send("]")
    t.send("]")
    t.wait_for(lambda: "Relativistic jet braking" in t.text(), what="query row")
    t.send("m")
    t.wait_for(lambda: "Added 1 paper(s) to manuscript db" in t.text(), what="add feedback")
    require(os.path.exists(ms_copy), "manuscript copy missing", t)

    # 2. on a query page, a paper the manuscript cites keeps that copy
    t.key("delete")
    t.wait_for("removes from the global library")
    # the sentence wraps inside the 52-wide modal, so match its parts
    require(
        "kept in the" in t.text() and "manuscript: 1 cited" in t.text(),
        "modal should name what survives",
        t,
    )
    t.key("esc")
    t.wait_for("removal cancelled")

    # 3. with the global tier hidden, only the local tier's copy goes
    t.send("[")
    t.send("[")  # back to the library scope
    t.send("t")
    t.wait_for(lambda: "global tier hidden" in t.text(), what="tier toggle")
    t.key("delete")
    t.wait_for("removes from this manuscript")
    require(
        "rescued to the global library" in t.text(),
        "modal should say sole copies are rescued",
        t,
    )
    t.key("esc")
    t.wait_for("removal cancelled")
    require(os.path.exists(ms_copy), "cancelling must not remove anything", t)
