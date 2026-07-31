# Build a LaTeX manuscript whose bibliography astrobib maintains.
#
#   make          compile main.pdf
#   make check    verify refs.bib is in sync, changing nothing (CI, pre-commit)
#   make clean    remove build products, refs.bib among them
#
# astrobib is required. refs.bib is a build product — gitignore it. What is
# version controlled is bib/, one .bib file per paper; refs.bib is the
# flattened view LaTeX consumes, keyed by the strings you actually cite.
#
# The dependency graph, which is acyclic:
#
#   main.pdf  <- main.tex, refs.bib
#   bib/      <- main.tex          (citing a paper pulls it out of your library)
#   refs.bib  <- main.tex, bib/
#
# `astrobib refs` implements both of the middle edges in one pass: it copies
# newly cited papers from your global library into bib/, then writes refs.bib
# from what is there. It also stamps refs.bib's mtime even when the content is
# unchanged, so the rule settles instead of re-running every build — the
# `touch $@` that this Makefile used to need is gone.
#
# Requires astrobib >= 0.9.

MAIN      := main
FIGURE    := figures/analytic_MC_validation_plot.pdf
FIGSCRIPT := scripts/analytic_MC_validation_plot.py
FIGDATA   := figdata/gridpdf_b4f4_n5000_mc5000_gb4dcc269.npz

VENV    := .venv
PYTHON  := $(VENV)/bin/python
LATEXMK := latexmk -pdf -halt-on-error -interaction=nonstopmode

.PHONY: all figures check clean distclean

all: $(MAIN).pdf

# latexmk resolves the bibtex and rerun-for-crossreference passes itself.
$(MAIN).pdf: $(MAIN).tex refs.bib $(FIGURE)
	$(LATEXMK) $(MAIN).tex

figures: $(FIGURE)

# The venv is an order-only prerequisite: recreating it must not force the
# figure to be redrawn.
$(FIGURE): $(FIGSCRIPT) $(FIGDATA) | $(PYTHON)
	$(PYTHON) $(FIGSCRIPT)

$(PYTHON):
	python3 -m venv $(VENV)
	$(VENV)/bin/pip install --quiet --upgrade pip
	$(VENV)/bin/pip install --quiet numpy matplotlib

# Three kinds of change to the bibliography must reach refs.bib, and it takes
# two prerequisites to catch them all:
#
#   edit an entry   -> that file's mtime advances    -> the wildcard sees it
#   add an entry    -> the directory's mtime advances
#   delete an entry -> ONLY the directory's mtime advances, since the file has
#                      left the wildcard; without `bib` a removed paper would
#                      linger in refs.bib forever
#
# The wildcard is re-expanded on every invocation, so files added to bib/
# become prerequisites next run with no edit here.
refs.bib: $(MAIN).tex bib $(wildcard bib/*.bib)
	@command -v astrobib >/dev/null 2>&1 || { \
	  echo "error: astrobib is required to build $@ from bib/; see README" >&2; \
	  exit 1; }
	astrobib refs

# Writes nothing; exits nonzero if refs.bib is stale or something you cite is
# still missing from bib/. Answers "did someone add a \cite and not rebuild?"
check:
	astrobib refs --check

clean:
	$(LATEXMK) -C $(MAIN).tex
	rm -f $(MAIN).bbl $(MAIN).synctex.gz $(FIGURE) refs.bib

distclean: clean
	rm -rf $(VENV)

# A note on CI: the rule above deliberately lets the build write into bib/,
# because that is the graph — citing a paper is what puts it in the repo. If
# you would rather a CI job never modify tracked files, use
#     astrobib refs --no-sync
# there instead: it writes refs.bib from what bib/ already holds and reports
# anything cited that is missing, rather than fetching it.
