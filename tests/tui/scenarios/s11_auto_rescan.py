"""Editing a manuscript source in an external editor is picked up by the
mtime poll: the Manuscript scope reclassifies the new cite and refs.bib
regenerates, all without pressing r (the Python app's _poll_manuscript)."""

import os

from driver import require

DESCRIPTION = "silent auto-rescan on external main.tex edits"

MANUSCRIPT = {
    "main.tex": (
        "\\documentclass{article}\n\\begin{document}\n"
        "Citing \\citep{Andersson2021}.\n"
        "\\end{document}\n"
    ),
    # both papers are manuscript-db members, so each cite lands in
    # refs.bib the moment the scan sees it
    "bib/Andersson2021pombz.bib": '@article{Andersson2021pombz,\n  author           = {{Andersson}, Freya and {Blomqvist}, Karin},\n  title            = {{Relativistic jet braking in dense circumstellar environments}},\n  year             = {2021},\n  journal          = {\\apj},\n  volume           = {912},\n  pages            = {77},\n  month            = {may},\n  eprint           = {2103.04156},\n  doi              = {10.3847/1538-4357/abf123},\n  adsurl           = {https://ui.adsabs.harvard.edu/abs/2021ApJ...912...77A},\n  adsnote          = {Provided by the SAO/NASA Astrophysics Data System},\n  keywords         = {Relativistic jets, Circumstellar matter, High energy astrophysics},\n  abstract         = {We study the deceleration of relativistic jets launched into dense circumstellar shells.},\n  primaryclass     = {astro-ph.HE},\n  archiveprefix    = {arXiv},\n  eid              = {77},\n}\n',
    "bib/Baxter2019equxm.bib": '@article{Baxter2019equxm,\n  author           = {{Baxter}, Miles},\n  title            = {{A census of runaway white dwarfs in the galactic halo}},\n  year             = {2019},\n  journal          = {\\mnras},\n  volume           = {487},\n  pages            = {1234-1250},\n  month            = {aug},\n  doi              = {10.1093/mnras/stz1234},\n  adsurl           = {https://ui.adsabs.harvard.edu/abs/2019MNRAS.487.1234B},\n  adsnote          = {Provided by the SAO/NASA Astrophysics Data System},\n  keywords         = {White dwarf stars, Stellar kinematics, Galactic halo},\n  abstract         = {Using astrometry from the second data release of Gaia, we identify 211 white dwarfs on unbound galactic orbits.},\n}\n',
}


def _row_with(t, needle):
    for ln in t.lines():
        if needle in ln:
            return ln
    return ""


def run(t):
    t.send("]")  # switch to the Manuscript scope
    t.wait_for("Andersson2021", what="manuscript rows")
    # Baxter is a db member but not yet cited
    require("uncited" in _row_with(t, "Baxter2019"), "Baxter2019 should start uncited", t)
    refs = os.path.join(t.cwd, "refs.bib")
    t.wait_for(lambda: os.path.exists(refs), what="refs.bib to appear")
    require("Baxter2019" not in open(refs).read(), "uncited member leaked into refs.bib", t)

    # external editor: append a cite to main.tex on disk, no keypress
    with open(os.path.join(t.cwd, "main.tex"), "a") as f:
        f.write("Also citing \\citep{Baxter2019}.\n")

    t.wait_for(
        lambda: "ok" in _row_with(t, "Baxter2019"),
        what="Baxter2019 row to turn cited without pressing r",
    )
    t.wait_for(
        lambda: "@article{Baxter2019," in open(refs).read(),
        what="refs.bib to pick up the new cite",
    )
    require("uncited" not in _row_with(t, "Baxter2019"), "row still shows uncited", t)

    # external change to bib/ (coauthor pull, hand-deleted file): the
    # library tier reloads and the cite reclassifies as library-only
    os.remove(os.path.join(t.cwd, "bib", "Baxter2019equxm.bib"))
    t.wait_for(
        lambda: "library" in _row_with(t, "Baxter2019"),
        what="Baxter2019 row to reclassify after bib/ change",
    )
    t.wait_for(
        lambda: "Baxter2019" not in open(refs).read(),
        what="refs.bib to drop the ex-member",
    )
