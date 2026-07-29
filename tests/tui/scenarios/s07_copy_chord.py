"""The permanent ⧉ copy column on the card, and the y chord acting from
the keyboard (its which-key menu now lives in the footer)."""

from driver import require

DESCRIPTION = "permanent copy column + y chord"


def run(t):
    txt = t.text()
    # the card's right column always shows the copy menu
    for label in ("cite key", "full key", "bibcode", "ADS URL"):
        require(label in txt, f"copy row {label!r} missing from card", t)
    require("│" in txt, "vertical column divider missing", t)
    # y arms the chord: the footer shows the which-key line
    t.send("y")
    t.wait_for(lambda: "copy:" in t.text(), what="footer which-key line")
    require("A abstract" in t.text(), "footer chord line incomplete", t)
    # yy copies the cite key
    t.send("y")
    t.wait_for(lambda: "Copied:" in t.text(), what="copy confirmation")
    t.wait_gone("copy: ")
