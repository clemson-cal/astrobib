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
.venv/bin/litbot

# Run CLI commands
.venv/bin/litbot db clone <url>
.venv/bin/litbot uat update
.venv/bin/litbot search --ads "query"

# Python is 3.11 from /opt/homebrew/bin/python3.11
# If recreating the venv: /opt/homebrew/bin/python3.11 -m venv .venv
```

There are no tests. Verify changes by importing the affected modules and running the CLI/TUI manually.

---

## Architecture

**Tool vs. data separation.** litbot is a pip-installable tool. The personal library lives at `~/.local/share/litbot/library/` (override root with `LITBOT_STATE_DIR`); manuscript databases live inside manuscript repos. The tool never stores data inside its own package directory.

**Package layout:**

- `litbot/state.py` — user-local app state: library path, ADS token, cache constants, `find_manuscript_db()`
- `litbot/library.py` — `Entry`, `Library`, `MergedLibrary`; reads `bib/*.bib` files
- `litbot/keys.py` — deterministic cite key generation (`AuthorYYYY` + 5-char hash of arXiv ID/bibcode)
- `litbot/ads_client.py` — ADS search and BibTeX export via the `ads` package; `refresh_quota()` and `resolve_pdf_url()` use httpx directly
- `litbot/uat.py` — UAT loader and hierarchy traversal; cached at `UAT_CACHE`
- `litbot/export.py` — scans `.tex` files for cite keys, writes `refs.bib`
- `litbot/pdf.py` — ephemeral PDF cache at `PDF_CACHE_DIR`
- `litbot/cli.py` — Click commands: `add`, `import`, `export`, `refs`, `search`, `show`, `list`, `keywords`, `quota`, plus `config`, `pdf`, `uat` groups
- `litbot/tui/app.py` — Textual TUI: library tab, ADS search tab (via `S`), UAT browser panel (via `u`)
- `litbot/tui/tabs_state.py` — persistent ADS query tabs (`tabs.json`), tab labels, result limits
- `litbot/tui/uat_browser.py` — standalone UAT browser app and screen
- `litbot/tui/help_screen.py` — modal help screen; content loaded from `litbot/help.md` (symlink to `../README.md`)

Note: `litbot/config.py` and `litbot/db.py` are dead code from an earlier multi-database design — nothing imports them.

**Manuscript databases.** A `bib/` directory inside a manuscript's git repo, discovered by walk-up from cwd (`bib/` + `.git`), at most one active per session. `MergedLibrary` merges it with the personal library for reads; imports write to both; `m` toggles membership; `litbot refs` syncs it against `.tex` cite keys and writes `refs.bib`. The TUI adds a Manuscript tab (`ManuscriptView`) that polls `.tex` mtimes and `bib/` (2 s `set_interval`), classifies each key as ok/library/missing/uncited, and auto-regenerates `refs.bib` on content change — but never auto-copies or auto-prunes entries; membership changes go through `m`. litbot never runs git on the manuscript repo. See DESIGN.md.

**UAT.** The Unified Astronomy Thesaurus JSON is a plain recursive tree (not SKOS). Cached at `~/.cache/litbot/uat.json`. The TUI keyword tree groups library entries by top-level UAT ancestor of their keywords.

---

## Key constraints

- Python ≥ 3.11 (`tomllib`, match statements, walrus operator)
- `bibtexparser` v1 (not v2 — the API differs significantly)
- Textual ≥ 0.60
- `pyyaml` is not a dependency (was removed when `keywords.yaml` was eliminated)
