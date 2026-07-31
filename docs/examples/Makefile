# Build a LaTeX manuscript whose bibliography astrobib maintains.
#
#   make            compile main.pdf (regenerating refs.bib and figures as needed)
#   make refs       sync bib/ with the keys cited in the sources, then rewrite refs.bib
#   make check      verify refs.bib is current without changing anything (CI, hooks)
#   make clean      remove build artifacts
#
# Works in every mode:
#
#   * astrobib open as a watching TUI — it keeps refs.bib current while you
#     write; this Makefile's regeneration is then a cheap no-op.
#   * astrobib installed but not running — the build regenerates refs.bib.
#   * astrobib absent (a coauthor's checkout, arXiv, a CI runner) — the
#     committed refs.bib is used as shipped and nothing tries to rebuild it.
#
# Requires astrobib >= 0.9 for `refs --no-sync` / `refs --check`.

MAIN      := main
FIGURE    := figures/analytic_MC_validation_plot.pdf
FIGSCRIPT := scripts/analytic_MC_validation_plot.py
FIGDATA   := figdata/gridpdf_b4f4_n5000_mc5000_gb4dcc269.npz

VENV    := .venv
PYTHON  := $(VENV)/bin/python
LATEXMK := latexmk -pdf -halt-on-error -interaction=nonstopmode

# Empty when astrobib is not installed, which is a supported situation.
ASTROBIB := $(shell command -v astrobib 2>/dev/null)

.PHONY: all figures refs check clean distclean

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

ifdef ASTROBIB

# `--no-sync` is the build-safe mode: it writes refs.bib and nothing else.
# A plain `astrobib refs` would also copy cited entries from your global
# library into bib/ — useful when you invoke it (see the `refs` target),
# but a recipe must not modify its own prerequisites. It also stamps
# refs.bib's mtime even when the content is unchanged, so this rule
# settles instead of re-running on every build; no `touch` needed.
refs.bib: $(wildcard bib/*.bib) $(MAIN).tex
	$(ASTROBIB) refs --no-sync

# Pull cited-but-missing entries into bib/ as well. Run this yourself;
# it is deliberately not part of the build, because it changes tracked
# files and `git status` should not be surprised by a compile.
refs:
	$(ASTROBIB) refs

# Verify without writing: exits nonzero if refs.bib is stale or if a
# cited entry is still missing from bib/. Suitable for CI and hooks.
check:
	$(ASTROBIB) refs --check

else

# astrobib is not installed. refs.bib has no prerequisites here, so make
# treats an existing file as up to date and never tries to rebuild it —
# which is exactly right: the committed bibliography is authoritative for
# anyone without the tool. Only a missing file is an error.
refs.bib:
	@test -f $@ || { \
	  echo "error: $@ is missing and astrobib is not installed." >&2; \
	  echo "       Ask an author to commit it, or install astrobib." >&2; \
	  exit 1; }

refs check:
	@echo "astrobib is not installed — using the committed $(MAIN) bibliography as shipped." >&2

endif

clean:
	$(LATEXMK) -C $(MAIN).tex
	rm -f $(MAIN).bbl $(MAIN).synctex.gz $(FIGURE)

distclean: clean
	rm -rf $(VENV)
