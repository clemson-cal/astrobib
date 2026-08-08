"""v (or the footer's @ bib affordance) flips the pub card to the verbatim
.bib file; v again returns to the formatted card.

The toggler lives at the footer's right edge, beyond the view badges,
rather than in the card it drives — so this also checks that it is on the
footer line and that it goes away with the card.
"""

from driver import require

DESCRIPTION = "v shows the verbatim .bib in the card"


def _footer(t):
    return t.lines()[len(t.lines()) - 1]


def run(t):
    # the card can land a frame after the table's startup paint
    t.wait_for(lambda: "@ bib" in _footer(t), what="the card toggler on the footer")
    t.send("v")
    t.wait_for(lambda: "bib/Cabrera2024txuze.bib" in t.text(), what="bib source header")
    require("@article{Cabrera2024txuze," in t.text(), "verbatim entry body missing", t)
    require("▤ card" in _footer(t), "back-to-card segment missing", t)
    t.send("v")
    t.wait_gone("bib/Cabrera2024txuze.bib")
    require("@ bib" in _footer(t), "formatted card did not return", t)

    # no card, nothing to toggle: the footer gives the room back
    t.send("D")
    t.wait_for(
        lambda: "@ bib" not in _footer(t),
        what="the toggler leaving with the pub card",
    )
    t.send("D")
    t.wait_for(lambda: "@ bib" in _footer(t), what="the toggler returning with it")
