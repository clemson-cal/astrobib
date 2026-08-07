"""The personal library is watched, with no manuscript anywhere in
sight: a .bib arriving in bib/ (a git pull, an add from another
terminal) shows up in the table, and an edit *inside* an existing tag
file is noticed too — the case a directory mtime would sleep through."""

import os

from driver import require

DESCRIPTION = "external changes to the personal library and its tags"

# a sixth entry, dropped in after launch the way a git pull would
NEW_ENTRY = (
    "@article{Nakamura2022vhqrb,\n"
    "  author           = {{Nakamura}, Rei},\n"
    "  title            = {{Spiral shock dissipation in self-gravitating discs}},\n"
    "  year             = {2022},\n"
    "  journal          = {\\apj},\n"
    "  volume           = {930},\n"
    "  pages            = {14},\n"
    "  doi              = {10.3847/1538-4357/ac5678},\n"
    "  adsurl           = {https://ui.adsabs.harvard.edu/abs/2022ApJ...930...14N},\n"
    "  adsnote          = {Provided by the SAO/NASA Astrophysics Data System},\n"
    "  keywords         = {Accretion discs, Spiral density waves},\n"
    "  abstract         = {Angular momentum transport by spiral shocks in a "
    "self-gravitating disc is measured across a range of cooling times.},\n"
    "}\n"
)


def _tags_dir(state_dir):
    return os.path.join(os.path.dirname(state_dir), "library", "tags")


def _pre_launch(state_dir):
    # a tag file that is already there when the app starts, naming one
    # paper the library has and two it does not
    os.makedirs(_tags_dir(state_dir), exist_ok=True)
    with open(os.path.join(_tags_dir(state_dir), "spiral-shocks"), "w") as f:
        f.write(
            "# references for section 3\n"
            "\n"
            "Andersson2021pombz\n"
            "Nakamura2022vhqrb\n"
            "Okonkwo2020absent\n"
        )


PRE_LAUNCH = _pre_launch


def run(t):
    tags = _tags_dir(t.state_dir)
    # the event log holds the startup report; the footer shows one line
    # of it and later notes can supersede that
    t.send("L")
    t.wait_for(lambda: " Log " in t.text(), what="log pane")
    # read at startup, and the two keys the library cannot resolve are
    # counted rather than dropped in silence
    t.wait_for(lambda: "tags: 2 key" in t.text(), what="startup tag report")

    # an edit inside an existing tag file: the directory mtime does not
    # move here, so only per-file watching can see it
    with open(os.path.join(tags, "spiral-shocks"), "a") as f:
        f.write("Rutherford1998absent\n")
    t.wait_for(lambda: "tags: 3 key" in t.text(), what="report after an in-place tag edit")

    # a whole new tag file: found by re-enumerating, and named alongside
    with open(os.path.join(tags, "disc-instability"), "w") as f:
        f.write("Vasquez2015absent\n")
    t.wait_for(lambda: "tags: 4 key" in t.text(), what="report after a new tag file")
    require("disc-instability: 1" in t.text(), "the new tag is not named in the report", t)

    # a .bib appearing in the personal bib/ with no manuscript open —
    # the case the old manuscript-gated watcher never ran for at all
    bib = os.path.join(os.path.dirname(t.state_dir), "library", "bib")
    with open(os.path.join(bib, "Nakamura2022vhqrb.bib"), "w") as f:
        f.write(NEW_ENTRY)
    t.wait_for(lambda: "Nakamura" in t.text(), what="the pulled entry to appear in the table")
    # and it resolves a tagged key that was dangling a moment ago
    t.wait_for(lambda: "tags: 3 key" in t.text(), what="the report to drop the resolved key")
