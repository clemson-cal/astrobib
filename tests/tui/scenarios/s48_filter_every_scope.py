"""/ narrows whatever table you are standing in, and each scope keeps
its own filter.

The filter used to be a library-only gesture — pressed anywhere else it
said so and did nothing — which left the two pages most likely to be
long (a hundred ADS results, a bibliography of two hundred cites) with
no way to narrow them at all. Every scope draws a table of papers, so
every scope answers the same language: an ADS row is filtered on what
ADS returned about it, a cite that resolves is filtered as the paper it
names, and one that resolves to nothing is filtered on the string the
page shows, which is the only way a missing cite could be found.

Per scope, because a filter is a property of the page it narrows — the
same reasoning that gives each scope its own sort. Carrying `tag:disks`
onto a page of ADS results would empty it and read as a broken query.
"""

import json
import os

from driver import require

DESCRIPTION = "/ filters every scope, and each keeps its own filter"

MANUSCRIPT = {
    "main.tex": (
        "\\documentclass{article}\n\\begin{document}\n"
        "As shown \\citep{Andersson2021pombz} and \\citet{Baxter2019equxm}, "
        "unlike \\citep{Nowhere2020}.\n\\end{document}\n"
    ),
}

_ARTICLES = [
    {
        "bibcode": "2020Test..K...1Z",
        "title": ["A cached result about kilonovae"],
        "author": ["Cachette, Q."],
        "year": "2020",
        "abstract": "From the cache, not the network.",
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
        "bibcode": "2022Test..M...2Z",
        "title": ["Magnetar wind nebulae at radio wavelengths"],
        "author": ["Okonkwo, D."],
        "year": "2022",
        "abstract": "Also from the cache.",
        "doi": [],
        "identifier": [],
        "citation_count": 3,
        "entry_date": "2024-03-03T00:00:00Z",
        "pub": "TestJ",
        "volume": "2",
        "issue": "",
        "page": ["2"],
    },
]


def _pre_launch(state_dir):
    ms = os.path.join(os.path.dirname(state_dir), "home", "ms")
    tab = {"id": "q1", "query": "kilonovae", "label": "kilonovae", "limit": 20, "created": 0}
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {ms: [tab], os.path.realpath(ms): [tab]}}, f)
    cache = os.path.join(os.path.dirname(state_dir), "home", ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"q1": _ARTICLES}}, f)


PRE_LAUNCH = _pre_launch


def _type_filter(t, text):
    t.send("/")
    t.send(text)
    t.key("enter")  # the prompt occupies the footer the counts live on


def run(t):
    t.wait_for("Manuscript", what="manuscript capsule")
    # the session opens on the project; these five papers are the global
    # library's, so show it
    t.send("t")
    t.wait_for(lambda: "global tier shown" in t.text(), what="the global tier")

    # 1. the library, as it always did
    _type_filter(t, "cabrera")
    t.wait_for(lambda: "1/5" in t.text(), what="the library filtered to one of five")

    # 2. the manuscript page: a fresh filter, not the library's
    t.send("]")
    t.wait_for(lambda: "Nowhere2020" in t.text(), what="the manuscript rows")
    require(
        "/ cabrera" not in t.text(),
        "the library's filter should have stayed with the library",
        t,
    )
    _type_filter(t, "baxter")
    t.wait_for(lambda: "1/3" in t.text(), what="the manuscript filtered to one of three")
    require("Baxter" in t.text(), "the surviving row should be the Baxter cite", t)

    # a cite that resolves to nothing is still findable by its string —
    # the row has no library entry behind it to be filtered as
    t.send("/")
    for _ in range(len("baxter")):
        t.key("backspace")
    t.send("nowhere")
    t.key("enter")
    t.wait_for(lambda: "1/3" in t.text(), what="the manuscript filtered to the missing cite")
    require("Nowhere2020" in t.text(), "the missing cite should be the row that survived", t)

    # 3. the query page: filtered on what ADS returned about each row
    t.send("]")
    t.wait_for(lambda: "kilonovae" in t.text(), what="the restored query rows")
    _type_filter(t, "magnetar")
    t.wait_for(lambda: "1/2" in t.text(), what="the query filtered to one of two")
    # and the row that survived is the one the card describes: a filtered
    # page must not draw one paper and act on another
    t.wait_for(
        lambda: "Magnetar wind nebulae" in t.text() and "A cached result" not in t.text(),
        what="the card and the table naming the same paper",
    )

    # 4. back round the strip: every filter is where it was left
    t.send("[")
    t.wait_for(lambda: "/ nowhere" in t.text(), what="the manuscript's own filter, restored")
    t.send("[")
    t.wait_for(lambda: "/ cabrera" in t.text(), what="the library's own filter, restored")
    t.wait_for(lambda: "1/5" in t.text(), what="the library narrowed as it was left")

    # Esc clears the one you are on, and only that one
    t.key("esc")
    t.wait_for(lambda: "5/5" in t.text(), what="the library filter cleared")
    t.send("]")
    t.wait_for(lambda: "/ nowhere" in t.text(), what="the manuscript's filter, still there")
