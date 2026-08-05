"""A query tab appears when the query is sent, not when it comes back.

An ADS query can take a minute. The tab used to materialise only once
the results landed, so for that minute there was nothing on screen to say
anything had been asked — the app looked idle, and re-pressing ⏎ was the
natural thing to do.

The tab is the acknowledgement. It appears at once, and the page carries
what it is waiting for; when the round trip ends, the same place carries
the outcome. That is why a failure has to land on the page too: the tab
is already open, so a footer line that scrolls away would leave it
sitting there empty with no account of why.

The scenario runs without a usable ADS token, so the round trip fails —
which is the deterministic half. The pending text is the same code path
(`empty_hint`), and appears in the same place, so the pending assertion
is a `wait_for` on whichever of the two the poll first sees.
"""

import json
import os

from driver import require

DESCRIPTION = "a query tab appears immediately, and says what it is waiting for"


def _pre_launch(state_dir):
    # a token, so ⏎ sends a query instead of opening first-run setup.
    # It is not a real one, so the round trip fails — deliberately: what
    # is under test is that the tab exists for the whole of it.
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)


PRE_LAUNCH = _pre_launch


def run(t):
    footer = len(t.lines()) - 1
    strip = 1  # the scope capsules

    require(
        "magnetar" not in t.lines()[strip],
        f"no query capsule before one is made: {t.lines()[strip]!r}",
        t,
    )

    t.send("S")
    t.wait_for("ADS query:", what="the ADS query prompt")
    t.send("magnetar")
    t.key("enter")

    # the capsule is there at once — this is the regression: it used to
    # arrive only with the results
    t.wait_for(
        lambda: "magnetar" in t.lines()[strip],
        what="the query capsule appearing as the query is sent",
    )
    require(
        "ADS query:" not in t.lines()[footer],
        "the prompt should have closed when the query was sent",
        t,
    )

    # …and the page says what is happening, in the place that will later
    # say how it ended
    t.wait_for(
        lambda: "searching ADS" in t.text() or "ADS search failed" in t.text(),
        what="the page reporting the query's state",
    )

    # the failure lands on the page, not only in the footer: the tab is
    # already open, and an empty page with no account of itself is what
    # this whole change is about
    t.wait_for(
        lambda: "ADS search failed" in t.text(),
        what="the failure reported on the query page",
        timeout=30,
    )
    page = "\n".join(t.lines()[3:footer])
    require(
        "ADS search failed" in page,
        f"the failure belongs on the page, not only the footer:\n{page}",
        t,
    )
    require(
        "r retries" in page,
        f"the page should say what to do about it:\n{page}",
        t,
    )
    # and the tab is still there to retry from
    require(
        "magnetar" in t.lines()[strip],
        f"a failed query keeps its tab: {t.lines()[strip]!r}",
        t,
    )
