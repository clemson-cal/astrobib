"""Each scope sorts independently, and remembers how.

Sort used to be one app-wide field, so sorting the library reordered
every query tab with it and vice versa. Now the library and the
manuscript keep theirs in state.json and each query tab keeps its own on
the Tab in tabs.json — this walks all three and then reads the state
files back, which is what proves the sort would survive a restart
without needing a second session to observe it.
"""

import json
import os

from driver import require

DESCRIPTION = "library, manuscript and query sort independently"

MANUSCRIPT = {
    "main.tex": (
        "\\documentclass{article}\n\\begin{document}\n"
        "Citing \\citep{Andersson2021} and \\citet{Baxter2019} "
        "and \\citep{Nowhere2020}.\n\\end{document}\n"
    ),
}

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
        "pub": "TestJ",
        "volume": "9",
        "issue": "",
        "page": ["44"],
    },
]


def _pre_launch(state_dir):
    home = os.path.join(os.path.dirname(state_dir), "home")
    ms_root = os.path.join(home, "ms")
    cache = os.path.join(home, ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    tab = {"id": "tt1", "query": "kilonova", "label": "kilonova", "limit": 20, "created": 0}
    # getcwd resolves symlinks, so the app may see either spelling of the
    # scratch path as the manuscript root — write the context under both
    keys = {ms_root, os.path.realpath(ms_root)}
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {k: [tab] for k in keys}}, f)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"tt1": _ARTICLES}}, f)


PRE_LAUNCH = _pre_launch


def _first_data_row(t):
    """The row under the header rule."""
    for i, ln in enumerate(t.lines()):
        if ln[:20] == "─" * 20:
            return t.lines()[i + 1]
    raise AssertionError(f"no table header rule on screen\n{t.dump()}")


def _click_header(t, label):
    x, y = t.find(label)
    t.click(x, y)


def run(t):
    require("Year ▼" in t.text(), "library should start at Year ▼", t)

    # library: sort by author
    _click_header(t, "Author")
    t.wait_for("Author ▲", what="library sorted by Author, ascending")

    # manuscript: its own sort, and the library's does not follow it here
    t.send("]")
    t.wait_for("✗", what="manuscript rows")
    require(
        "Author ▲" not in t.text(),
        "the library's sort leaked into the manuscript scope",
        t,
    )
    _click_header(t, "State")
    t.wait_for("State ▲", what="manuscript sorted by State")
    # ascending State is attention-first, so the missing cite comes top
    t.wait_for(
        lambda: "✗" in _first_data_row(t),
        what="the missing cite on the first data row",
    )

    # query: still its own default, untouched by either of the above
    t.send("]")
    t.wait_for("2015", what="restored query rows")
    require("Year ▼" in t.text(), "query tab should be at its own Year ▼", t)
    require("Author ▲" not in t.text(), "the library's sort leaked into the query", t)
    _click_header(t, "Year")
    t.wait_for("Year ▲", what="query sorted by Year, ascending")
    t.wait_for(
        lambda: "2015" in _first_data_row(t),
        what="the older result on the first data row",
    )

    # back to the library: its own sort survived all of that
    t.send("[")
    t.send("[")
    t.wait_for("Author ▲", what="the library still sorted by Author")
    require("Year ▲" not in t.text(), "the query's sort leaked into the library", t)

    # and every one of them is on disk
    with open(os.path.join(t.state_dir, "state.json")) as f:
        state = json.load(f)
    require(
        state.get("library_sort") == "author:asc",
        f"library_sort should be author:asc, got {state.get('library_sort')!r}",
        t,
    )
    require(
        state.get("manuscript_sort") == "state:asc",
        f"manuscript_sort should be state:asc, got {state.get('manuscript_sort')!r}",
        t,
    )
    with open(os.path.join(t.state_dir, "tabs.json")) as f:
        tabs = json.load(f)
    # PRE_LAUNCH seeded the tab under both spellings of the scratch path,
    # and the app rewrites only the context it actually resolved to — so
    # the other copy still carries no sort fields at all
    saved = [tab for ctx in tabs["contexts"].values() for tab in ctx]
    require(saved, "no saved tabs written back", t)
    require(
        any(s.get("sort_col") == "year" and s.get("sort_asc") is True for s in saved),
        f"no tab saved with year/ascending, got {[(s.get('sort_col'), s.get('sort_asc')) for s in saved]}",
        t,
    )
