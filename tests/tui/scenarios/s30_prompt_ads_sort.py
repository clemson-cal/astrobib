"""The query prompt shows, and sets, what ADS will return.

Two properties of a query are not query *syntax* — the result limit and
what ADS returns ride alongside `q` as the `rows` and `sort` API
parameters — and both belong where the query is composed. What ADS
returns had no home at all before: it could only be reached after the
tab existed, so a first run could never be anything but the default.

It is named in full rather than reduced to a symbol, because the name
changing *is* the feedback that the mode changed. The prompt replaces
the footer, minibuffer-style, so it must say everything it needs to on
its own line.
"""

import json
import os

from driver import require

DESCRIPTION = "query prompt: what ADS returns, named, chorded and clickable"

# the modes in the order ⌃r steps them, wrapping
MODES = ["newest posting", "newest published", "most cited", "most relevant"]
CTRL_R = "\x12"


def _pre_launch(state_dir):
    # a token, so S composes a query instead of opening first-run setup
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)


PRE_LAUNCH = _pre_launch


def run(t):
    footer = len(t.lines()) - 1
    t.send("S")
    t.wait_for("ADS query:", what="the ADS query prompt")

    # typing goes to the query; the parameters sit past it
    t.send("little red dots")
    t.wait_for(
        lambda: "little red dots" in t.lines()[footer],
        what="the typed query",
    )
    # one phrase: how many records, and by what
    require(
        f"ADS returns 20 (↑↓) {MODES[0]} (⌃r)" in t.lines()[footer],
        f"the prompt should read as one phrase: {t.lines()[footer]!r}",
        t,
    )

    # only the mode is a control — the limit has ↑↓, which no pointer
    # can press — so only the mode looks clickable when rolled over
    lx = t.lines()[footer].index("20 (↑↓)")
    t.hover(lx + 1, footer)
    t.wait_quiet()
    require(
        not t.screen.buffer[footer][lx + 1].underscore,
        "the result limit should not read as clickable",
        t,
    )
    gx = t.lines()[footer].index(MODES[0]) + 2
    t.hover(gx, footer)
    t.wait_for(
        lambda: t.screen.buffer[footer][gx].underscore,
        what="the mode underlining under the pointer",
    )

    # ⌃r steps it, and wraps. The name changing is the feedback — there
    # is no second line to echo it to, the prompt having taken the footer
    for name in MODES[1:] + [MODES[0]]:
        t.send(CTRL_R)
        t.wait_for(
            lambda n=name: f"{n} (⌃r)" in t.lines()[footer],
            what=f"⌃r stepping to {name!r}",
        )

    # and clicking does the same
    t.click(gx, footer)
    t.wait_for(f"{MODES[1]} (⌃r)", what="clicking the mode stepping it")

    # ↑ still steps the limit, so the two parameters do not fight
    t.key("up")
    t.wait_for(
        lambda: "ADS returns 50 (↑↓)" in t.lines()[footer]
        and MODES[1] in t.lines()[footer],
        what="the limit stepping while the mode holds",
    )

    # Esc leaves without querying — this scenario has no network
    t.key("esc")
    t.wait_gone("ADS query:")
