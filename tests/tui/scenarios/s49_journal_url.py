"""A journal link is a paper, and the prompt takes it as one.

The address bar of the page you are reading is the thing you actually
have when you decide you want a paper — not its bibcode, and usually not
its DOI either. Most publishers put the DOI in the path; Nature hides it
behind an article id that is the DOI's own suffix; Oxford leaves it out
entirely but prints the citation in the path instead, which ADS answers
by volume and page. All of it is decided locally, so what the prompt
sends is a fielded query naming one paper.

The half worth pinning is the refusal. A link nothing here can identify
must say so *without* sending anything: a URL is never a query, so
handing it to ADS as search text would fail a round trip later and
somewhere else, with a capsule left sitting open over it. This scenario
runs with an unusable token, so what it can assert about the queries that
do go out is the query itself — the capsule's label, which is derived
from it — and not what comes back.
"""

import json
import os

from driver import require

DESCRIPTION = "a journal, arXiv or DOI link at the prompt names one paper"

STRIP = 1  # the row of scope capsules

# no DOI in the path, and a manuscript number ADS does not index
UNIDENTIFIABLE = "https://www.aanda.org/articles/aa/full_html/2024/06/aa48123-23/aa48123-23.html"


def _pre_launch(state_dir):
    # a token, so ⏎ sends a query instead of opening first-run setup. It
    # is not a real one: every round trip here fails, and every
    # assertion is about what was sent rather than what came back.
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)


PRE_LAUNCH = _pre_launch


def _compose(t, text):
    t.send("S")
    t.wait_for("ADS query:", what="the ADS query prompt")
    t.send(text)
    t.key("enter")


def run(t):
    before = t.lines()[STRIP]

    # ── a link that names no paper is refused, and nothing is sent
    _compose(t, UNIDENTIFIABLE)
    t.wait_for("no paper identified", what="the refusal naming the link")
    require(
        "Searching ADS" not in t.text(),
        "an unidentifiable link must not be sent to ADS as search text",
        t,
    )
    require(
        t.lines()[STRIP] == before,
        f"the refusal must not leave a capsule behind: {t.lines()[STRIP]!r}",
        t,
    )
    require("ADS query:" not in t.text(), "the prompt should have closed", t)

    # ── Nature keeps the DOI out of the URL; its article id is the suffix
    _compose(t, "https://www.nature.com/articles/nature24291")
    t.wait_for(
        lambda: 'doi:"10.1038/nature24291"' in t.text(),
        what="the DOI derived from the Nature article id",
    )
    t.wait_for(
        lambda: "10.1038/nature24291" in t.lines()[STRIP],
        what="the capsule labelled by the derived query",
    )

    # ── Oxford prints the citation instead of the DOI: MNRAS 537, 3620
    _compose(t, "https://academic.oup.com/mnras/article/537/4/3620/7965432")
    t.wait_for(
        lambda: "bibstem:MNRAS volume:537 page:3620" in t.text(),
        what="the ADS query derived from the OUP citation path",
    )

    # ── an arXiv link is the eprint, which identifies the paper whether
    # or not it has been published since
    _compose(t, "https://arxiv.org/abs/1710.05834v2")
    t.wait_for(
        lambda: 'identifier:"arXiv:1710.05834"' in t.text(),
        what="the eprint identifier, with the version dropped",
    )
