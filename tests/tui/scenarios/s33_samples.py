"""Sample queries, offered while composing and loaded by clicking.

Neither query language is discoverable from inside the app, and the
usual answer — a reference card — fails on the detail that matters: you
need the syntax *while* you type, and a modal has to be dismissed before
you can reach the prompt. A TUI has no copy-paste to carry an example
across either. So the samples appear beside the prompt and load
themselves when clicked.

The rule that makes clicking safe is that a sample replaces the whole
query, so it only acts on an empty prompt. The subtle half of that is
what a click must do when it *cannot* act: it has to be consumed. Left
to fall through it would reach the click-away dismissal and close the
very prompt the sample was meant to fill — which is what the last
section here pins.
"""

import json
import os

from driver import require

DESCRIPTION = "sample queries offered while composing, loaded by clicking"


def _pre_launch(state_dir):
    # a token, so S composes a query instead of opening first-run setup
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump({"version": 1, "ads_token": "not-a-real-token", "email": "a@b.c"}, f)


PRE_LAUNCH = _pre_launch

ADS_SAMPLE = 'bibstem:ApJL abs:"magnetar"'
FILTER_SAMPLE = "is:pdf pri:>0.5"


def _row_of(t, needle):
    return next(i for i, l in enumerate(t.lines()) if needle in l)


def run(t):
    footer = len(t.lines()) - 1

    # it costs nothing when no prompt is up
    require("examples" not in t.text(), "no band without a prompt", t)

    # the ADS set for the ADS prompt
    t.send("S")
    t.wait_for("ADS query:", what="the ADS query prompt")
    t.wait_for(ADS_SAMPLE, what="the ADS samples beside the prompt")
    require(
        FILTER_SAMPLE not in t.text(),
        "the filter samples belong to the filter, not to the ADS prompt",
        t,
    )

    # a sample looks clickable, and loads itself
    y = _row_of(t, ADS_SAMPLE)
    t.hover(6, y)
    t.wait_for(
        lambda: t.screen.buffer[y][6].underscore,
        what="the sample underlining under the pointer",
    )
    t.click(6, y)
    t.wait_for(
        lambda: ADS_SAMPLE in t.lines()[footer],
        what="the sample loaded into the prompt",
    )
    require("ADS query:" in t.lines()[footer], "the prompt should still be open", t)

    # with something typed, a sample cannot act — and says why, since the
    # footer that would normally carry that is holding the prompt
    t.wait_for("clear the query to use one", what="the heading explaining the refusal")
    before = t.lines()[footer]
    t.click(6, _row_of(t, 'abs:"little red dot"'))
    t.wait_quiet()
    require(
        t.lines()[footer] == before,
        f"a sample must not overwrite a typed query: {t.lines()[footer]!r}",
        t,
    )
    # …and above all it must not close the prompt, which is where the
    # click would land if the row let it through
    require(
        "ADS query:" in t.lines()[footer],
        "clicking an inert sample closed the prompt",
        t,
    )

    t.key("esc")
    t.wait_gone("ADS query:")

    # the filter set for the filter, and clicking applies it live
    t.send("/")
    t.wait_for(FILTER_SAMPLE, what="the filter samples beside the filter prompt")
    require(
        ADS_SAMPLE not in t.text(),
        "the ADS samples belong to the ADS prompt, not to the filter",
        t,
    )
    # the footer is holding the filter prompt, so the table itself is
    # the witness that the filter took effect
    require("Cabrera, +1" in t.text(), "expected the fixture rows before filtering", t)
    t.click(6, _row_of(t, FILTER_SAMPLE))
    # none of the fixtures has a cached PDF or a priority, so this
    # matches nothing — a live application of the filter, not a no-op
    t.wait_for(
        lambda: "Cabrera, +1" not in t.text(),
        what="the filter applied the moment the sample was clicked",
    )
    require(
        FILTER_SAMPLE in t.lines()[footer],
        f"the filter prompt should hold the sample: {t.lines()[footer]!r}",
        t,
    )
