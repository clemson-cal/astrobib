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

# the default, which the prompt opens on
MODES = ["newest posting"]
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

    # ⌃r opens the menu of everything ADS will sort by. It cycled four
    # modes until there were twenty — ten fields, each either way round —
    # and a cycle cannot carry twenty.
    t.send(CTRL_R)
    t.wait_for("ADS returns  ·  the key picks it", what="the ADS-returns menu")
    # names read forwards and counts read biggest-first, so the menu
    # offers each field in the direction that is normally wanted
    for name in ("most cited", "most read", "highest classic factor", "bibcode A→Z"):
        require(name in t.text(), f"the menu should list {name!r}", t)
    require(
        "ADS query:" in t.lines()[footer],
        "the menu must not close the prompt underneath it",
        t,
    )

    # a key picks its field, closes the menu, and the prompt says so
    t.send("c")
    t.wait_gone("the key picks it")
    t.wait_for(
        lambda: "most cited (⌃r)" in t.lines()[footer],
        what="the picked mode named on the prompt",
    )
    require(
        "little red dots" in t.lines()[footer],
        f"picking a mode must not disturb the query: {t.lines()[footer]!r}",
        t,
    )

    # shift takes the same field the other way round, which is how all
    # twenty stay one keystroke away
    t.send(CTRL_R)
    t.wait_for("ADS returns  ·  the key picks it", what="the menu reopening")
    t.send("C")
    t.wait_for(
        lambda: "least cited (⌃r)" in t.lines()[footer],
        what="shift reversing the direction",
    )

    # ⌃r closes it again without changing anything
    t.send(CTRL_R)
    t.wait_for("ADS returns  ·  the key picks it", what="the menu reopening")
    t.send(CTRL_R)
    t.wait_gone("the key picks it")
    require(
        "least cited (⌃r)" in t.lines()[footer],
        f"closing the menu should leave the mode alone: {t.lines()[footer]!r}",
        t,
    )

    # and clicking the affordance opens it too
    gx = t.lines()[footer].index("least cited") + 2
    t.click(gx, footer)
    t.wait_for("ADS returns  ·  the key picks it", what="clicking the mode opening the menu")
    t.key("esc")
    t.wait_gone("the key picks it")
    require(
        "ADS query:" in t.lines()[footer],
        "Esc should close the menu, not the prompt",
        t,
    )

    # ↑ still steps the limit, so the two parameters do not fight
    t.key("up")
    t.wait_for(
        lambda: "ADS returns 50 (↑↓)" in t.lines()[footer]
        and "least cited" in t.lines()[footer],
        what="the limit stepping while the mode holds",
    )

    # Esc leaves without querying — this scenario has no network
    t.key("esc")
    t.wait_gone("ADS query:")
