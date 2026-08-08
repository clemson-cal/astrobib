"""The copy chord offers only what this screen can actually copy.

The menu was a hand-written string and the resolution a separate `match`,
so the two drifted: the library offered "q this query" where there is no
query, and every paper was offered a bibcode, a DOI and a cached PDF path
whether or not it had one. Both now come from one table filtered by the
same function that does the copying, so an option can be offered only
when pressing it would copy something.

The clipboard itself stays out of reach — a copy here would write to the
real pasteboard, which the pty sandbox does not contain — so this reads
the menu, which is the thing that was wrong.
"""

import json
import os

from driver import require

DESCRIPTION = "the copy chord lists only what the current screen can copy"

_ARTICLE = {
    "bibcode": "2020TestA...1..1Z",
    "title": ["A cached result about magnetars"],
    "author": ["Cachette, Q."],
    "year": "2020",
    "abstract": "From the cache, not the network.",
    "doi": [],
    "identifier": [],
    "citation_count": 7,
    "pub": "TestJ",
    "volume": "1",
    "issue": "",
    "page": ["1"],
}


def _pre_launch(state_dir):
    cache = os.path.join(os.path.dirname(state_dir), "home", ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {"global": [
            {"id": "tt1", "query": "magnetar", "label": "magnetar", "limit": 20, "created": 0}
        ]}}, f)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"tt1": [_ARTICLE]}}, f)


PRE_LAUNCH = _pre_launch


# Each option's two renderings, long then short, from COPY_CHORD. The
# menu sheds words before it sheds options — that is the behaviour under
# test — so which one appears depends on how much of the footer the view
# badges and the card ⇄ bib toggler beside them have left it.
_LABELS = {
    "y": ("key", "key"),
    "Y": ("full key", "full"),
    "b": ("bibcode", "bib"),
    "a": ("ADS", "ADS"),
    "x": ("arXiv", "arXiv"),
    "d": ("DOI", "DOI"),
    "p": ("PDF path", "PDF"),
    "t": ("title", "title"),
    "A": ("abstract", "abs"),
    "q": ("this query", "query"),
}


def _offers(line, key):
    """Is this option on the menu, under either of its renderings?"""
    long_, short = _LABELS[key]
    return f"{key} {long_}" in line or f"{key} {short}" in line


def _menu(t):
    footer = len(t.lines()) - 1
    t.send("y")
    t.wait_for(
        lambda: t.lines()[footer].startswith("copy:"),
        what="the copy chord menu on the footer",
    )
    return t.lines()[footer]


def _close_menu(t):
    """Esc, and wait for it to land.

    Waiting matters beyond tidiness: a lone ESC written back-to-back with
    the next byte arrives in one pty read and crossterm parses the pair
    as alt+<key>, so the following keystroke would be swallowed.
    """
    footer = len(t.lines()) - 1
    t.key("esc")
    t.wait_for(
        lambda: not t.lines()[footer].startswith("copy:"),
        what="the copy menu closing",
    )


def run(t):
    t.wait_for("Cabrera, +1", what="the fixture rows")

    # the library has no query, so it must not offer to copy one
    lib = _menu(t)
    require(
        not _offers(lib, "q"),
        f"the library should not offer to copy a query: {lib!r}",
        t,
    )
    # what it does have is offered: the fixture carries an ADS URL and an
    # arXiv id, and every entry has a cite key
    for key in ("y", "Y", "a", "x", "t", "A"):
        require(_offers(lib, key), f"the library should offer {key!r}: {lib!r}", t)
    # …and what it does not have is not: no fixture has a DOI or a cached
    # PDF, and these are arXiv preprints with no journal bibcode of their
    # own in the file
    require(not _offers(lib, "p"), f"no PDF is cached in the fixtures: {lib!r}", t)
    require(not _offers(lib, "d"), f"no fixture carries a DOI: {lib!r}", t)
    _close_menu(t)

    # a query scope has one, so there it is offered
    t.send("]")
    t.wait_for("A cached result about magnetars", what="the restored query's rows")
    q = _menu(t)
    require(_offers(q, "q"), f"a query scope should offer to copy its query: {q!r}", t)
    # the cached article has a bibcode but no DOI, and the menu says so
    require(_offers(q, "b"), f"the query result has a bibcode: {q!r}", t)
    require(not _offers(q, "d"), f"the query result has no DOI: {q!r}", t)
    _close_menu(t)

    # the badges share this line and are drawn over it, so the menu has
    # to give way rather than run underneath them. It sheds the Esc
    # hint, then its separators, then its words — never an option, since
    # hiding an available one is the failure this scenario is about.
    for w in (120, 100, 80):
        t.resize(w)
        t.wait_for(
            lambda: "keys" in t.lines()[len(t.lines()) - 1],
            what=f"the footer re-laid-out at {w} columns",
        )
        line = _menu(t)
        badges = line.find("■ card")
        require(
            badges > 0,
            f"the view badges should still be on the footer at w={w}: {line!r}",
            t,
        )
        require(
            line[:badges].rstrip() == line[:badges].rstrip().rstrip("·"),
            f"the menu should not be cut mid-separator at w={w}: {line!r}",
            t,
        )
        require(
            len(line[:badges].rstrip()) < badges,
            f"the menu should not touch the badges at w={w}: {line!r}",
            t,
        )
        _close_menu(t)
    t.resize(140)
    t.wait_for(
        lambda: "keys" in t.lines()[len(t.lines()) - 1],
        what="the footer back at full width",
    )

    # and pressing a key the menu did not offer explains itself rather
    # than being silently swallowed
    t.send("[")
    t.wait_for("Cabrera, +1", what="back on the library")
    footer = len(t.lines()) - 1
    t.send("yq")
    t.wait_for(
        lambda: "no query here" in t.lines()[footer],
        what="the refusal naming what is missing",
    )
