"""The y chord opens the which-key copy modal; Esc cancels it.

The scenario never actually copies: on macOS that would go through pbcopy
and overwrite the user's real clipboard (the pty sandbox does not contain
the pasteboard).
"""

from driver import require

DESCRIPTION = "y copy chord opens its modal"


def run(t):
    t.send("y")
    t.wait_for("copy → clipboard")
    for label in (
        "cite key",
        "full key",
        "bibcode",
        "ADS URL",
        "arXiv URL",
        "DOI URL",
        "PDF path",
        "title",
        "abstract",
    ):
        require(label in t.text(), f"copy target {label!r} missing from modal", t)
    require("Esc cancel" in t.text(), "cancel hint missing", t)
    t.key("esc")
    t.wait_gone("copy → clipboard")
