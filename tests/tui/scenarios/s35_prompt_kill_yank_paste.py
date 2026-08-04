"""Kill and yank in the prompt, and a pasted query that configures itself.

`^k` used to reach tui-input, which implements it as a plain delete: the
killed tail was gone and `^y` had nothing to yank, because there was no
kill ring to yank from. Both now behave as emacs does.

The paste half is the reverse of copying. A query is three things — the
text, the result limit, and what ADS returns — and only the first is
query *syntax*; the other two are API parameters, and Solr has no comment
to smuggle them into. The ADS search URL is the one line that carries all
three, so pasting one back restores all three, and pasting one with no
prompt open opens the prompt already configured.

`⌥w` (the copy half) is deliberately not exercised: clipboard writes go
to the real pasteboard, which the pty sandbox does not contain. The URL
it builds is covered by unit tests in `src/ads.rs` instead.
"""

import json
import os

from driver import require

DESCRIPTION = "prompt kill/yank, and a pasted ADS URL configuring the query"


def _pre_launch(state_dir):
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)


PRE_LAUNCH = _pre_launch

URL = (
    "https://ui.adsabs.harvard.edu/search/"
    "q=abs%3A%22magnetar%22&rows=100&sort=citation_count%20desc"
)


def run(t):
    footer = len(t.lines()) - 1

    t.send("S")
    t.wait_for("ADS query:", what="the ADS query prompt")
    t.send("kilonova ejecta")
    t.wait_for(lambda: "kilonova ejecta" in t.lines()[footer], what="the typed query")

    # kill from the cursor: walk back over " ejecta", then ^k
    for _ in range(len("ejecta")):
        t.key("left")
    t.send("\x0b")  # ^k
    t.wait_for(
        lambda: "ejecta" not in t.lines()[footer],
        what="the tail killed",
    )
    require("kilonova" in t.lines()[footer], "the head should survive the kill", t)

    # and yank it straight back
    t.send("\x19")  # ^y
    t.wait_for(
        lambda: "kilonova ejecta" in t.lines()[footer],
        what="the killed tail yanked back",
    )

    # a pasted search URL replaces all three at once. The prompt shows
    # the limit and the mode, so the footer is the whole witness.
    require(
        "20" in t.lines()[footer] and "newest posting" in t.lines()[footer],
        f"expected the default limit and mode first: {t.lines()[footer]!r}",
        t,
    )
    t.paste(URL)
    t.wait_for(
        lambda: 'abs:"magnetar"' in t.lines()[footer],
        what="the pasted query text",
    )
    line = t.lines()[footer]
    require("100" in line, f"the pasted result limit should apply: {line!r}", t)
    require("most cited" in line, f"the pasted sort should apply: {line!r}", t)
    require(
        "kilonova" not in line,
        f"a pasted query replaces what was there: {line!r}",
        t,
    )
    # the URL itself must not have been inserted as text
    require("https" not in line, f"the URL should not be typed in: {line!r}", t)

    t.key("esc")
    t.wait_gone("ADS query:")

    # pasted with no prompt up, it opens one — already configured
    t.paste(URL)
    t.wait_for("ADS query:", what="the prompt opening for a pasted query")
    t.wait_for(
        lambda: 'abs:"magnetar"' in t.lines()[footer] and "most cited" in t.lines()[footer],
        what="the pasted query configuring the prompt it opened",
    )

    # ordinary pasted text is still just text
    t.key("esc")
    t.wait_gone("ADS query:")
    t.send("/")
    t.wait_for(
        lambda: t.lines()[footer].startswith("/"),
        what="the filter prompt taking the footer",
    )
    t.paste("magnetar")
    t.wait_for(
        lambda: "magnetar" in t.lines()[footer],
        what="plain text pasted into the filter",
    )
