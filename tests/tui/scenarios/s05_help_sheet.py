"""? opens the keys panel above the log (non-modal: other keys still
work); ? or Esc closes it."""

from driver import require

DESCRIPTION = "? keys panel is non-modal; ?/Esc close"


def run(t):
    t.send("?")
    t.wait_for(lambda: "this cheat-sheet" in t.text(), what="keys panel")
    require(" keys " in t.text(), "keys panel title missing from the pane border", t)

    # non-modal: navigation keys act while the panel is up. Rather than
    # sleeping and hoping j was processed, wait for proof that it was —
    # the cursor moves from the 2024 entry to the 2023 one, and the card
    # (still visible beside the panel) repaints with its full cite key.
    t.send("j")
    t.wait_for("Ekwueme2023ophaj", what="cursor moved to the 2023 entry (j acted)")
    require(
        "this cheat-sheet" in t.text(),
        "j moved the cursor but also dismissed the keys panel; it is non-modal",
        t,
    )

    t.send("?")
    t.wait_gone("this cheat-sheet")
    t.send("?")
    t.wait_for(lambda: "this cheat-sheet" in t.text(), what="keys panel reopens on ?")
    t.key("esc")
    t.wait_gone("this cheat-sheet")
