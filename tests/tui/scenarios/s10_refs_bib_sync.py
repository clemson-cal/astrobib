"""A TeX manuscript's refs.bib is regenerated silently on scan, keyed by
the strings actually cited (the Python app's regenerate-on-change)."""

import os

from driver import require

DESCRIPTION = "refs.bib auto-generated for TeX manuscripts"

MANUSCRIPT = {
    "main.tex": (
        "\\documentclass{article}\n\\begin{document}\n"
        "Citing \\citep{Andersson2021} and \\citet{Baxter2019}.\n"
        "\\end{document}\n"
    ),
    # a manuscript-db member: refs.bib holds only these, keyed as cited
    "bib/Andersson2021pombz.bib": '@article{Andersson2021pombz,\n  author           = {{Andersson}, Freya and {Blomqvist}, Karin},\n  title            = {{Relativistic jet braking in dense circumstellar environments}},\n  year             = {2021},\n  journal          = {\\apj},\n  volume           = {912},\n  pages            = {77},\n  month            = {may},\n  eprint           = {2103.04156},\n  doi              = {10.3847/1538-4357/abf123},\n  adsurl           = {https://ui.adsabs.harvard.edu/abs/2021ApJ...912...77A},\n  adsnote          = {Provided by the SAO/NASA Astrophysics Data System},\n  keywords         = {Relativistic jets, Circumstellar matter, High energy astrophysics},\n  abstract         = {We study the deceleration of relativistic jets launched into dense circumstellar shells. Semi-analytic models are compared with two-dimensional simulations, and we find that jet braking imprints a characteristic break in the afterglow light curve.},\n  primaryclass     = {astro-ph.HE},\n  archiveprefix    = {arXiv},\n  eid              = {77},\n}\n',
}


def run(t):
    require("Manuscript" in t.text(), "Manuscript scope pill missing", t)
    refs = os.path.join(t.cwd, "refs.bib")
    t.wait_for(lambda: os.path.exists(refs), what="refs.bib to appear")
    content = open(refs).read()
    require("@article{Andersson2021," in content, "cited-string key missing", t)
    require("Andersson2021pombz" not in content, "hash suffix leaked into refs.bib", t)
    # cited but only in the personal library: not a manuscript-db member,
    # so (as in Python) it stays out of refs.bib until added with m
    require("Baxter2019" not in content, "library-only cite leaked into refs.bib", t)
