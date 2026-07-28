"""Delete opens the confirm modal; cancelling leaves the library untouched."""

import os

from driver import require

DESCRIPTION = "Delete confirm modal cancels safely"


def run(t):
    key = "Cabrera2024txuze"  # newest entry: under the cursor at startup
    path = os.path.join(t.bib_dir, key + ".bib")
    require(os.path.exists(path), "fixture file missing before the test", t)

    t.key("delete")
    t.wait_for("Remove 1 paper(s) from the library?")
    require(key in t.text(), "confirm modal should list the target key", t)
    require("remove" in t.text() and "cancel" in t.text(), "modal buttons missing", t)

    t.key("esc")
    t.wait_gone("Remove 1 paper(s)")
    t.wait_for("removal cancelled")
    require(os.path.exists(path), "cancel must not touch the .bib file", t)
    require(
        "Turbulent magnetic amplification" in t.text(),
        "the entry's row should still be in the table",
        t,
    )
