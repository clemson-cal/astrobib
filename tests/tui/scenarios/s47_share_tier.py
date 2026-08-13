"""s shares a paper up into the global library, and un-shares it again.

The counterpart to the local-first import: a paper fetched inside a
project lands in the project's db, and `s` is the one gesture that
promotes it to the library every other project sees. It reads as ± the
way `m` does — add all missing, else remove all — and it refuses the one
removal that would destroy bibdata: a paper the local db does not hold
is not un-shared by `s`, it is deleted by ⌫, which is a different key
and asks first.
"""

import os

from driver import require

DESCRIPTION = "s shares a paper to the global library and back"

MANUSCRIPT = {
    "main.md": "A manuscript with no cites yet.\n",
}


def _bibs(d):
    return sorted(n for n in os.listdir(d) if n.endswith(".bib"))


def run(t):
    ms_bib = os.path.join(os.path.dirname(t.state_dir), "home", "ms", "bib")
    require(_bibs(t.bib_dir) and not _bibs(ms_bib), "the local db should start empty", t)
    before = _bibs(t.bib_dir)
    # the session opens on the project alone, which here is empty: the
    # papers this scenario shares are in the global tier, so show it
    t.send("t")
    t.wait_for(lambda: "global tier shown" in t.text(), what="the global tier")

    # the cursor paper is global-only, so ± reads as "remove" — and that
    # is the removal the sole-copy rule refuses, since nothing else holds
    # the entry
    t.send("s")
    t.wait_for(
        lambda: "sole copy" in t.text(),
        what="the sole-copy refusal on the footer",
    )
    require(_bibs(t.bib_dir) == before, "no .bib file should have been removed", t)

    # give the local db a copy, and now s has somewhere to fall back to
    t.send("m")
    t.wait_for(lambda: "Added 1 paper(s) to manuscript db" in t.text(), what="the m confirmation")
    t.wait_for(lambda: len(_bibs(ms_bib)) == 1, what="the paper's copy in the local db")
    shared = _bibs(ms_bib)[0]

    t.send("s")
    t.wait_for(
        lambda: "Removed 1 paper(s) from the global library" in t.text(),
        what="the un-share confirmation",
    )
    t.wait_for(
        lambda: shared not in _bibs(t.bib_dir),
        what=f"{shared} to leave the global library",
    )
    # the paper itself survives: this is a tier gesture, not a deletion
    require(_bibs(ms_bib) == [shared], f"the local copy should stay: {_bibs(ms_bib)}", t)

    # and back up again, which is the gesture I (import + share) ends in
    t.send("s")
    t.wait_for(
        lambda: "Shared 1 paper(s) to the global library" in t.text(),
        what="the share confirmation",
    )
    t.wait_for(
        lambda: _bibs(t.bib_dir) == before,
        what=f"{shared} back in the global library",
    )

    # both gestures are on the cheat-sheet, next to the m they mirror
    t.send("?")
    t.wait_for("share to global", what="the s row on the keys panel")
    require("import + share" in t.text(), "the I row should be on the keys panel", t)
