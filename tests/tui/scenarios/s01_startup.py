"""Startup renders the scope strip, sorted table, pub card, and footer badges."""

from driver import require

DESCRIPTION = "startup: scope strip + table + card + badges"


def run(t):
    # wait_ready guarantees a complete *table* frame; the pub card is
    # filled from the cursor entry and can land one frame later, so anchor
    # on a card-only needle before reading the screen once.
    t.wait_for("arXiv:2407.20112", what="pub card populated (arXiv link)")
    txt = t.text()
    # scope strip
    require("Library" in txt, "Library scope missing from scope strip", t)
    # table header with the default sort marker (Year, descending)
    for h in ("Year", "Author", "Title", "Key"):
        require(h in txt, f"table header {h!r} missing", t)
    require("Year ▼" in txt, "default Year-descending sort marker missing", t)
    # year-descending: 2024 entry above the 2023 entry, and all five present.
    # Author cells ("Cabrera, +1") are table-only needles — long titles are
    # truncated in the table and reflowed in the card, so they can't anchor
    # row positions.
    top = t.row_of("Cabrera, +1")
    below = t.row_of("Ekwueme, +4")
    require(top < below, "table is not sorted year-descending", t)
    for frag in (
        "Relativistic jet braking",
        "A census of runaway white dwarfs",
        "On the stability of self-gravitating",
    ):
        require(frag in txt, f"fixture row {frag!r} missing", t)
    # pub card is on by default and shows the highlighted (newest) entry:
    # its cite key appears in the card footer
    require("Cabrera2024" in txt, "pub card cite key footer missing", t)
    # citation-graph affordances in the card links row (the entry has an
    # adsurl, so both query affordances render alongside the URL links)
    require("citations" in txt, "pub card citations affordance missing", t)
    require("references" in txt, "pub card references affordance missing", t)
    # footer view badges: card on, log and keys off
    require("■ card" in txt, "card badge should be on (■)", t)
    require("□ log" in txt, "log badge should be off (□)", t)
    require("□ keys" in txt, "keys badge should be off (□)", t)

    # …and each says in the footer what clicking it does and which key
    # does the same. The badges sit on the footer line itself, so find
    # them there — "card" also names the pub card's own bib toggler.
    footer = len(t.lines()) - 1
    for label, hint in (
        ("log", "show the event log  ·  L"),
        ("keys", "show the cheat-sheet  ·  ?"),
        ("card", "hide the pub card  ·  D"),
    ):
        t.hover(t.lines()[footer].index(label), footer)
        t.wait_for(hint, what=f"the {label} badge's rollover hint")
