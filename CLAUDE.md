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

**Tool vs. data separation.** litbot is a pip-installable tool. Bib databases are separate git repositories registered in `~/.config/litbot/config.toml`. The tool never stores data inside its own package directory.

**Package layout:**

- `litbot/config.py` — config loading, `UAT_CACHE`, `PDF_CACHE_DIR`, `DEFAULT_DB_DIR` constants
- `litbot/library.py` — `Entry`, `Library`, `MergedLibrary`; reads `bib/*.bib` files
- `litbot/db.py` — git subprocess wrapper: `clone`, `init_empty`, `commit_entry`, `pull`, `push`, `publish`
- `litbot/ads_client.py` — ADS search and BibTeX export via the `ads` package; `refresh_quota()` uses httpx directly
- `litbot/uat.py` — UAT loader and hierarchy traversal; cached at `UAT_CACHE`
- `litbot/export.py` — scans `.tex` files for cite keys, writes `refs.bib`
- `litbot/pdf.py` — ephemeral PDF cache at `PDF_CACHE_DIR`
- `litbot/cli.py` — Click command group: `db`, `uat`, `add`, `search`, `export`, `show`, `open`, `list`, `keywords`, `quota`
- `litbot/tui/app.py` — Textual TUI: library tab, ADS search tab (via `S`), UAT browser panel (via `u`)
- `litbot/tui/uat_browser.py` — standalone UAT browser app and screen
- `litbot/tui/help_screen.py` — modal help screen; content loaded from `litbot/help.md` (symlink to `../README.md`)

**Multi-database reads.** All configured databases are merged transparently into a `MergedLibrary` for browsing, search, and export. Writes go to `default_database`.

**`litbot add` auto-commits.** After saving `bib/<key>.bib`, it runs `git add` + `git commit` on the target database. The user then runs `db push` to share.

**UAT.** The Unified Astronomy Thesaurus JSON is a plain recursive tree (not SKOS). Cached at `~/.cache/litbot/uat.json`. The TUI keyword tree groups library entries by top-level UAT ancestor of their keywords.

**DB commands (git-analogous):**
- `db clone <url>` — git clone + register
- `db init [path]` — git init empty db + register (or register existing)
- `db pull` — git pull
- `db push` — git push only (assumes already committed)
- `db publish -m msg` — git add -A + commit + push

---

## Key constraints

- Python ≥ 3.11 (`tomllib`, match statements, walrus operator)
- `bibtexparser` v1 (not v2 — the API differs significantly)
- Textual ≥ 0.60
- `pyyaml` is not a dependency (was removed when `keywords.yaml` was eliminated)
