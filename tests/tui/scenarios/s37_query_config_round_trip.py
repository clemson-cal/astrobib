"""A query's whole configuration copies out, survives a restart, and
pastes back in.

A saved query is three things: the text, how many records to ask for, and
which records ADS should pick — and only the first is query *syntax*. The
other two are the `rows` and `sort` API parameters, so the ADS search URL
is the one line that carries all three.

`y q` copies it from the tab rather than from a prompt. Copying used to
mean opening `E` on the query and abandoning the edit, which risks
changing the query you meant only to read.

The selection sort now persists with everything else. It once did not, on
the argument that it is chosen while composing — but a tab that came back
configured differently is not the query that was saved, and that argument
applies just as well to the query text.

The clipboard itself is out of reach here: writes go to the real
pasteboard, which the pty sandbox does not contain, and reads would take
it too. So this drives the two halves the harness *can* see — what the
tab persists, and what arrives on a paste — and the URL builder's own
round trip is unit-tested in `src/ads.rs`.
"""

import json
import os

from driver import require

DESCRIPTION = "a query config persists across a restart and pastes back"

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

# a saved tab whose selection sort is *not* the default: if it were, a
# restored default would pass this scenario without persisting anything
SORT = "citation_count desc"


def _pre_launch(state_dir):
    cache = os.path.join(os.path.dirname(state_dir), "home", ".cache", "astrobib")
    os.makedirs(cache, exist_ok=True)
    # a token, so a pasted config opens the query prompt rather than
    # first-run setup
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump(
            {
                "contexts": {
                    "global": [
                        {
                            "id": "tt1",
                            "query": "magnetar",
                            "label": "magnetar",
                            "limit": 50,
                            "created": 0,
                            "ads_sort": SORT,
                        }
                    ]
                }
            },
            f,
        )
    with open(os.path.join(cache, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"tt1": [_ARTICLE]}}, f)


PRE_LAUNCH = _pre_launch


def run(t):
    footer = len(t.lines()) - 1
    t.wait_for(
        lambda: "restored 1 saved query from cache" in t.text(),
        what="the restored query",
    )
    t.send("]")
    t.wait_for("A cached result about magnetars", what="the restored query's rows")

    # a restart brought back the configuration, not just the text: E
    # shows what the tab is actually set to
    t.send("E")
    t.wait_for("edit query:", what="the edit prompt")
    line = t.lines()[footer]
    require("magnetar" in line, f"the query text should come back: {line!r}", t)
    require("50" in line, f"the result count should come back: {line!r}", t)
    require(
        "most cited" in line,
        f"the selection sort should come back, not reset to the default: {line!r}",
        t,
    )
    t.key("esc")
    t.wait_gone("edit query:")

    # y q reports what it copied, which is the whole configuration — the
    # clipboard write itself cannot be inspected from here
    t.send("yq")
    t.wait_for(
        lambda: "Copied" in t.lines()[footer] and "adsabs.harvard.edu/search" in t.lines()[footer],
        what="the copied search URL echoed back",
    )

    # and the same shape pasted in comes back apart into its three parts
    t.paste(
        "https://ui.adsabs.harvard.edu/search/"
        "q=abs%3A%22kilonova%22&rows=200&sort=entry_date%20desc"
    )
    t.wait_for("ADS query:", what="the prompt opened by the pasted config")
    t.wait_for(
        lambda: 'abs:"kilonova"' in t.lines()[footer]
        and "200" in t.lines()[footer]
        and "newest posting" in t.lines()[footer],
        what="all three parts of the pasted configuration",
    )
    t.key("esc")

    # a paste that is not a query config must not be mistaken for one
    t.paste("https://example.com/not-a-query")
    t.wait_quiet()
    require(
        "ADS query:" not in t.lines()[footer],
        f"an unrelated URL should not open a query prompt: {t.lines()[footer]!r}",
        t,
    )
