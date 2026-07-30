"""v (or the card's @ bib affordance) flips the pub card to the verbatim
.bib file; v again returns to the formatted card."""

from driver import require

DESCRIPTION = "v shows the verbatim .bib in the card"


def run(t):
    # the card can land a frame after the table's startup paint
    t.wait_for("@ bib", what="card toggler")
    t.send("v")
    t.wait_for(lambda: "bib/Cabrera2024txuze.bib" in t.text(), what="bib source header")
    txt = t.text()
    require("@article{Cabrera2024txuze," in txt, "verbatim entry body missing", t)
    require("▤ card" in txt, "back-to-card segment missing", t)
    t.send("v")
    t.wait_gone("bib/Cabrera2024txuze.bib")
    require("@ bib" in t.text(), "formatted card did not return", t)
