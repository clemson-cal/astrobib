# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Read [DESIGN.md](DESIGN.md) before adding features or changing data formats. It is the authoritative statement of design constraints.

---

## Development commands

```bash
# Always use the project-local venv
source .venv/bin/activate
# or prefix commands with .venv/bin/

# Install in editable mode
.venv/bin/pip install -e .

# Run the TUI
.venv/bin/astrobib

# Run CLI commands
.venv/bin/astrobib db clone <url>
.venv/bin/astrobib uat update
.venv/bin/astrobib search --ads "query"

# Python is 3.11 from /opt/homebrew/bin/python3.11
# If recreating the venv: /opt/homebrew/bin/python3.11 -m venv .venv
```

There are no tests. Verify changes by importing the affected modules and running the CLI/TUI manually.

---

## Architecture

**Tool vs. data separation.** astrobib is a pip-installable tool. The personal library lives at `~/.local/share/astrobib/library/` (override root with `ASTROBIB_STATE_DIR`); manuscript databases live inside manuscript repos. The tool never stores data inside its own package directory.

**Package layout:**

- `astrobib/state.py` — user-local app state: library path, ADS token, cache constants, `find_manuscript_db()`
- `astrobib/library.py` — `Entry`, `Library`, `MergedLibrary`; reads `bib/*.bib` files
- `astrobib/keys.py` — deterministic cite key generation (`AuthorYYYY` + 5-char hash of arXiv ID/bibcode)
- `astrobib/ads_client.py` — ADS search and BibTeX export via the `ads` package; `refresh_quota()` and `resolve_pdf_url()` use httpx directly
- `astrobib/uat.py` — UAT loader and hierarchy traversal; cached at `UAT_CACHE`
- `astrobib/export.py` — scans `.tex` files for cite keys, writes `refs.bib`; `manuscript_tex_files()`: `main.tex` is the sole root when present (else all top-level `.tex`), expanded recursively via `\input`/`\include`
- `astrobib/pdf.py` — ephemeral PDF cache at `PDF_CACHE_DIR`
- `astrobib/cli.py` — Click commands: `add`, `import`, `export`, `refs`, `search`, `show`, `list`, `keywords`, `quota`, plus `config`, `pdf`, `uat` groups
- `astrobib/tui/app.py` — Textual TUI: library tab, ADS search tab (via `S`), UAT browser panel (via `u`)
- `astrobib/tui/tabs_state.py` — persistent ADS query tabs (`tabs.json`), tab labels, result limits
- `astrobib/tui/uat_browser.py` — standalone UAT browser app and screen
- `astrobib/tui/help_screen.py` — modal help screen; content loaded from `astrobib/help.md` (symlink to `../README.md`)

Note: `astrobib/config.py` and `astrobib/db.py` are dead code from an earlier multi-database design — nothing imports them.

**Manuscript databases.** A `bib/` directory inside a manuscript's git repo, discovered by walk-up from cwd (`bib/` + `.git`), at most one active per session. `MergedLibrary` merges it with the personal library for reads; imports write to both; `m` toggles membership; `astrobib refs` syncs it against `.tex` cite keys and writes `refs.bib`. The TUI adds a Manuscript tab (`ManuscriptView`) that polls `.tex` mtimes and `bib/` (2 s `set_interval`), classifies each key as ok/library/missing/uncited, and auto-regenerates `refs.bib` on content change — but never auto-copies or auto-prunes entries; membership changes go through `m`. astrobib never runs git on the manuscript repo. See DESIGN.md.

**UAT.** The Unified Astronomy Thesaurus JSON is a plain recursive tree (not SKOS). Cached at `~/.cache/astrobib/uat.json`. The TUI keyword tree groups library entries by top-level UAT ancestor of their keywords.

---

## Key constraints

- Python ≥ 3.11 (`tomllib`, match statements, walrus operator)
- `bibtexparser` v1 (not v2 — the API differs significantly)
- Textual ≥ 0.60
- `pyyaml` is not a dependency (was removed when `keywords.yaml` was eliminated)
