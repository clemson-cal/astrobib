"""Golden screens for the table chrome, across all three scopes.

This is a refactor oracle, not a feature test. draw_table renders the
library, manuscript, and query views through three separate branches
that duplicate their chrome — header spans, sort markers, hover, click
rects, the divider rule, column constraints, cursor fill. Unifying them
is a large edit to dense conditional rendering, and inspection alone is
not enough to prove it changed nothing.

So: capture the table region in every scope at four terminal widths with
the pub card open and closed, plus a few sort states, and require the
bytes to match a committed baseline. The responsive rules (author width
scaling, the Key column dropping when tight) only appear under resize,
which is why driver.resize exists.

Re-bless deliberately with ASTROBIB_BLESS=1, never as a fixture refresh:
a diff here means the rendered screen changed, which is the whole point.
"""

import difflib
import json
import os

from driver import require

DESCRIPTION = "table chrome golden screens (refactor oracle)"

COLS = 140

BASELINE = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "baselines",
    "table_chrome.txt",
)

MANUSCRIPT = {
    "main.tex": (
        "\\documentclass{article}\n\\begin{document}\n"
        "Citing \\citep{Andersson2021} and \\citet{Baxter2019} "
        "and \\citep{Nowhere2020}.\n\\end{document}\n"
    ),
}

# two cached results, so row ordering is observable in the capture
_ARTICLES = [
    {
        "bibcode": "2020TestA...1..1Z",
        "title": ["A cached result about kilonovae"],
        "author": ["Cachette, Q.", "Zylstra, R."],
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
        "bibcode": "2015TestB...9..44Q",
        "title": ["Older cached companion paper"],
        "author": ["Quist, M."],
        "year": "2015",
        "abstract": "Also from the cache.",
        "doi": [],
        "identifier": [],
        "citation_count": 112,
        "entry_date": "2026-01-19T00:00:00Z",
        "pub": "TestJ",
        "volume": "9",
        "issue": "",
        "page": ["44"],
    },
]


def _pre_launch(state_dir):
    # a manuscript is present, so saved tabs live under the manuscript
    # root's context key rather than "global"; the query cache is
    # disposable data and lives under $HOME/.cache
    home = os.path.join(os.path.dirname(state_dir), "home")
    ms_root = os.path.join(home, "ms")
    cache = os.path.join(home, ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    tab = {"id": "tt1", "query": "kilonova", "label": "kilonova", "limit": 20, "created": 0}
    # the app keys contexts by the manuscript root as the process sees
    # it, and getcwd resolves symlinks — on macOS the scratch dir is
    # /var/folders/… to Python but /private/var/folders/… to the app, so
    # write both spellings
    keys = {ms_root, os.path.realpath(ms_root)}
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {k: [tab] for k in keys}}, f)
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"tt1": _ARTICLES}}, f)


PRE_LAUNCH = _pre_launch

# Scope needles have to survive the narrow widths, where titles and cite
# keys are truncated or dropped — the Year column never is. 2018 is a
# library fixture and appears in no cached result; 2015 is a cached
# result and is in no fixture.
LIB_YEAR = "2018"
QUERY_YEAR = "2015"
WIDTHS = (140, 110, 84, 64)


def _rule(t):
    """(row index, width) of the table's header rule, or (None, 0).

    The scope strip above the table changes height as the capsules wrap,
    so a fixed row index would lie; the rule is the reliable anchor. Its
    width is the table's width, which is what makes it a re-layout
    signal as well.
    """
    for i, ln in enumerate(t.lines()):
        if ln[:20] == "─" * 20:
            # the leading run, not the trailing one: with the card open
            # the rule is followed by the card's own content on the same
            # line, so rstrip would measure nothing
            return i, len(ln) - len(ln.lstrip("─"))
    return None, 0


def _painted(t):
    """The rightmost column the app has painted anything on.

    Every width in WIDTHS gives this a different value: the footer's
    badge cluster is right-aligned, so the app's own content reaches
    within a cell or two of the edge whatever the width is.
    """
    return max((len(ln.rstrip()) for ln in t.lines()), default=0)


def _relayout(t, w):
    """Resize, and wait for the app's repaint to actually land.

    wait_quiet on its own is unsafe straight after a resize: the stream
    is already quiet at that moment, so it returns before SIGWINCH has
    been handled and hands the scenario the previous geometry — which
    then makes every click land on stale coordinates.

    The signal is how far right the app paints, not the header rule's
    width: the table floors at a minimum width, so 84 and 64 columns
    give the same rule and the wait passed only by catching a transient
    mid-repaint frame — a race that held until a change to the strip
    above the table stopped producing it.
    """
    before = _painted(t)
    t.resize(w)
    t.wait_for(lambda: _painted(t) != before, what=f"re-layout at {w} columns")
    t.wait_quiet()


def _block(t):
    """The table region: header row, rule, and every data row."""
    row, _ = _rule(t)
    require(row is not None, "no table header rule on screen", t)
    # the bottom band is the footer, now a single tinted line — it lost
    # the rule above it, and the body gained that row
    return [ln.rstrip() for ln in t.lines()[row - 1 : t.rows - 1]]


def run(t):
    shots = []

    def shot(label):
        t.wait_quiet()
        shots.append((label, _block(t)))

    for w in WIDTHS:
        if w != t.cols:
            _relayout(t, w)
        # library — the card is open at startup
        shot(f"library w={w} card")
        t.send("D")
        t.wait_quiet()
        shot(f"library w={w} nocard")
        t.send("D")
        t.wait_quiet()

        t.send("]")
        # the missing-cite glyph: one character, so narrow widths cannot
        # truncate it away, and the only ✗ the app draws anywhere
        t.wait_for("✗", what="manuscript rows")
        shot(f"manuscript w={w} card")

        t.send("]")
        t.wait_for(QUERY_YEAR, what="restored query rows")
        shot(f"query w={w} card")

        t.send("[")
        t.send("[")
        t.wait_for(
            lambda: LIB_YEAR in t.text() and QUERY_YEAR not in t.text(),
            what="back in the library scope",
        )

    # sort states: the ▲/▼ marker, the moved marker, and the reordered
    # rows all come out of the chrome being refactored. Back to the wide
    # layout first — the narrow ones truncate the header labels away
    _relayout(t, WIDTHS[0])
    x, y = t.find("Title")
    t.click(x, y)
    t.wait_for("Title ▲", what="Title marker ascending")
    shot("library sort=title asc")
    t.click(x, y)
    t.wait_for("Title ▼", what="Title marker descending")
    shot("library sort=title desc")
    x, y = t.find("Year")
    t.click(x, y)
    t.wait_for("Year", what="Year header")
    shot("library sort=year")

    got = "".join(f"## {label}\n" + "\n".join(lines) + "\n\n" for label, lines in shots)

    bless = os.environ.get("ASTROBIB_BLESS") == "1"
    if bless or not os.path.exists(BASELINE):
        os.makedirs(os.path.dirname(BASELINE), exist_ok=True)
        with open(BASELINE, "w") as f:
            f.write(got)
        if not bless:
            raise AssertionError(
                f"no baseline yet — wrote {BASELINE}. Review it by eye, then commit it."
            )
        return
    with open(BASELINE) as f:
        want = f.read()
    if got == want:
        return
    diff = list(
        difflib.unified_diff(
            want.splitlines(), got.splitlines(), "baseline", "rendered", lineterm="", n=2
        )
    )
    shown = "\n".join(diff[:80])
    more = f"\n… {len(diff) - 80} more diff lines" if len(diff) > 80 else ""
    raise AssertionError(
        "table chrome changed against the committed baseline.\n"
        "If the change is intended, re-bless with ASTROBIB_BLESS=1 "
        "and review the diff in the commit.\n" + shown + more
    )
