"""PDFs are only ever cached under a library cite key: an un-imported
query result offers no PDF actions, and pressing them creates nothing."""

import json
import os

from driver import require

DESCRIPTION = "no PDF is cached under a bibcode"

_ARTICLE = {
    "bibcode": "2020Ghost...1..1Z",
    "title": ["An un-imported paper"],
    "author": ["Ghost, G."],
    "year": "2020",
    "abstract": "Not in the library.",
    "doi": ["10.1/ghost"],
    "identifier": ["arXiv:2001.00001"],
    "citation_count": 3,
    "pub": "TestJ",
    "volume": "1",
    "issue": "",
    "page": ["1"],
}


def _pre_launch(state_dir):
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {"global": [
            {"id": "g1", "query": "ghost", "label": "ghost", "limit": 20, "created": 0}
        ]}}, f)
    with open(os.path.join(state_dir, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"g1": [_ARTICLE]}}, f)


PRE_LAUNCH = _pre_launch


def run(t):
    t.send("]")  # into the restored query scope
    t.wait_for(lambda: "An un-imported paper" in t.text(), what="cached query row")
    txt = t.text()
    # the card offers no PDF actions for a paper that is not in the library
    for label in ("arXiv ↓", "ADS OA ↓", "browser ↓", "pick …"):
        require(label not in txt, f"PDF action {label!r} offered pre-import", t)
    # and the keys say why instead of silently doing nothing
    t.send("p")
    t.wait_for(lambda: "import the paper first" in t.text(), what="p explanation")
    cache = os.path.join(t.home, ".cache", "astrobib", "pdfs")
    files = os.listdir(cache) if os.path.isdir(cache) else []
    require(not files, f"a PDF was cached pre-import: {files}", t)
