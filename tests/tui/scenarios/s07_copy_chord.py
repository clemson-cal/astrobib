"""The permanent ⧉ copy column on the card, and the y chord acting from
the keyboard (its which-key menu now lives in the footer)."""

from driver import require

DESCRIPTION = "permanent copy column + y chord"


def run(t):
    # the card's right column always shows the copy menu (wait: the
    # card can land a frame after the table's first paint)
    t.wait_for("ADS URL", what="copy column")
    txt = t.text()
    for label in ("cite key", "full key", "bibcode"):
        require(label in txt, f"copy row {label!r} missing from card", t)
    require("│" in txt, "vertical column divider missing", t)
    # y arms the chord: the footer shows the which-key line
    t.send("y")
    t.wait_for(
        lambda: "copy:" in t.text() and "A abstract" in t.text(),
        what="the complete footer which-key line (through 'A abstract')",
    )
    # yy copies the cite key
    t.send("y")
    t.wait_for(lambda: "Copied:" in t.text(), what="copy confirmation")
    t.wait_gone("copy: ")
