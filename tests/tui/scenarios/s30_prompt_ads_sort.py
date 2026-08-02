"""The S prompt shows and sets what ADS will select by.

Two properties of a query are not query *syntax* — the result limit and
the selection sort ride alongside `q` as the `rows` and `sort` API
parameters — and both are now surfaced where the query is composed. The
limit already was; the sort was only reachable from the table panel,
after the tab existed, so a first run could never use anything but the
default.

They must not read as text you could type: the prompt draws them in
their own colour past the text cursor's reach, and ⏎ sends only the
query string to ADS.
"""

import json
import os

from driver import require

DESCRIPTION = "S prompt: the ADS selection sort is shown and settable"


def _pre_launch(state_dir):
    # a token, so S composes a query instead of opening first-run setup
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)


PRE_LAUNCH = _pre_launch

# the ADS sorts the prompt cycles, in order, wrapping
SORTS = ["newest posting", "newest published", "most cited", "most relevant"]


def run(t):
    t.send("S")
    t.wait_for("ADS query:", what="the ADS query prompt")

    footer = len(t.lines()) - 1
    require(
        SORTS[0] in t.lines()[footer],
        f"the prompt should open on {SORTS[0]!r}: {t.lines()[footer]!r}",
        t,
    )
    require("n=20" in t.lines()[footer], "the prompt should show the result limit", t)

    # typing goes to the query, not to the parameters beside it
    t.send("little red dots")
    t.wait_for(
        lambda: "little red dots" in t.lines()[footer]
        and SORTS[0] in t.lines()[footer],
        what="the typed query with the parameters still shown",
    )

    # ⇥ cycles the selection sort, and wraps
    for want in SORTS[1:] + [SORTS[0]]:
        t.send("\t")
        t.wait_for(
            lambda w=want: w in t.lines()[footer],
            what=f"the selection sort stepping to {want!r}",
        )

    # ↑ still steps the limit, so the two parameters do not fight
    t.key("up")
    t.wait_for(
        lambda: "n=50" in t.lines()[footer] and SORTS[0] in t.lines()[footer],
        what="the limit stepping while the sort holds",
    )

    # Esc leaves without querying — this scenario has no network
    t.key("esc")
    t.wait_gone("ADS query:")
