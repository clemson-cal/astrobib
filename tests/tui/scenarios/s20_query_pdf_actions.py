"""Entry actions on a query page act on the imported twin: X clears the
cached PDF of a result that IS in the library, and an un-imported result
explains itself instead of silently doing nothing."""

import json
import os

from driver import require

DESCRIPTION = "query page: X clears the imported twin's PDF"

# the fixture entry Andersson2021pombz, as ADS would return it
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

_GHOST = {
    "bibcode": "2020Ghost...1..1Z",
    "title": ["A paper nobody imported"],
    "author": ["Ghost, G."],
    "year": "2020",
    "abstract": "Not in the library.",
    "doi": [],
    "identifier": [],
    "citation_count": 1,
    "pub": "TestJ",
    "volume": "1",
    "issue": "",
    "page": ["1"],
}

CACHED_PDF = "Andersson2021pombz.pdf"


def _pre_launch(state_dir):
    scratch = os.path.dirname(state_dir)
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {"global": [
            {"id": "q1", "query": "jets", "label": "jets", "limit": 20, "created": 0}
        ]}}, f)
    with open(os.path.join(state_dir, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"q1": [_IMPORTED, _GHOST]}}, f)
    # a PDF already cached under the twin's cite key
    cache = os.path.join(scratch, "home", ".cache", "astrobib", "pdfs")
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(cache, CACHED_PDF), "wb") as f:
        f.write(b"%PDF-1.4\n%stub\n")


PRE_LAUNCH = _pre_launch


def run(t):
    pdf = os.path.join(t.home, ".cache", "astrobib", "pdfs", CACHED_PDF)
    require(os.path.exists(pdf), "fixture PDF missing before the test", t)

    t.send("]")  # into the restored query scope
    t.wait_for(lambda: "Relativistic jet braking" in t.text(), what="imported query row")
    require("● in library" in t.text(), "the twin should be marked in the card", t)

    # X on a query row acts on the imported twin's cached PDF
    t.send("X")
    t.wait_for(lambda: "Cleared 1 cached PDF" in t.text(), what="clear feedback")
    require(not os.path.exists(pdf), "X did not clear the twin's cached PDF", t)

    # nothing left to clear: it says so rather than reporting a lie
    t.send("X")
    t.wait_for(lambda: "no cached PDF to clear" in t.text(), what="empty-clear reason")

    # the un-imported row has no library key at all — X explains why
    t.send("j")
    t.wait_for(lambda: "A paper nobody imported" in t.text(), what="ghost row card")
    t.send("X")
    t.wait_for(lambda: "import the paper first" in t.text(), what="un-imported reason")
