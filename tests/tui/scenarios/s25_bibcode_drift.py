"""A query result is matched to the library by paper identity, not by
bibcode: an entry imported at one phase of publication still shows its
in-library and cached-PDF indicators when ADS returns the other phase."""

import json
import os

from driver import require

DESCRIPTION = "query rows match the library across bibcode drift"

# the library fixture carries the PUBLISHED bibcode (2021ApJ...912...77A);
# this result is the PREPRINT record — different bibcode, same arXiv id
_ARTICLE = {
    "bibcode": "2021arXiv210304156A",
    "title": ["Relativistic jet braking in dense circumstellar environments"],
    "author": ["Andersson, Freya", "Blomqvist, Karin"],
    "year": "2021",
    "abstract": "Same paper, earlier phase.",
    "doi": [],
    "identifier": ["arXiv:2103.04156"],
    "citation_count": 9,
    "pub": "arXiv e-prints",
    "volume": "",
    "issue": "",
    "page": [""],
}


def _pre_launch(state_dir):
    home = os.path.join(os.path.dirname(state_dir), "home")
    cache = os.path.join(home, ".cache", "astrobib")
    os.makedirs(os.path.join(cache, "pdfs"), exist_ok=True)
    # a cached PDF under the library's cite key
    with open(os.path.join(cache, "pdfs", "Andersson2021pombz.pdf"), "wb") as f:
        f.write(b"%PDF-1.4 fixture")
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {"global": [
            {"id": "d1", "query": "jets", "label": "jets", "limit": 20, "created": 0}
        ]}}, f)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"d1": [_ARTICLE]}}, f)


PRE_LAUNCH = _pre_launch


def run(t):
    t.send("]")
    t.wait_for(lambda: "Relativistic jet braking" in t.text(), what="drifted query row")
    # the card repeats the title, so the table has to be isolated before
    # looking for the row. The card is a fixed 48 columns on the right and
    # no longer draws an edge rule — its tint is its boundary now — so the
    # split is by column, not by a divider glyph.
    table = [l[: t.screen.columns - 48] for l in t.lines()]
    row = next(l for l in table if "Relativistic jet braking" in l)
    # ● says "in your library" and ↓ says "PDF cached" — both are found
    # through the cite key, which is stable across the bibcode change
    require("●" in row, f"in-library indicator missing on a drifted row: {row!r}", t)
    require("↓" in row, f"cached-PDF indicator missing on a drifted row: {row!r}", t)
