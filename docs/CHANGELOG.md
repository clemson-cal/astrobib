# Changelog

## 0.4.0 — 2026-07-28

### Added
- Pasted DOIs are recognized in ADS search: a doi.org URL, `doi:` prefix, or bare DOI entered in the TUI search modal or `search --ads` is rewritten to a `doi:"..."` fielded query, alongside the existing ADS-abstract-URL import path.
- The filter language gained uppercase `OR`, `AND`, and `NOT`: `OR` separates alternative groups (`AND` binds tighter, as in ADS), `NOT` aliases the `-` prefix, and a bare `^name` term implies `author:`. Lowercase or/and/not stay ordinary search words, and dangling operators are ignored so live filtering never errors. ADS escalation carries OR groups through, parenthesized where needed.
- A `pick …` button on the pub card: browse the filesystem in a modal tree (starting in ~/Downloads, PDFs only) and import a chosen file into the PDF cache for the shown paper. The file is copied, never moved, and rejected if it isn't a real PDF.
- The personal library can live anywhere: `astrobib --library PATH` (or `ASTROBIB_LIBRARY=PATH`) points every command and the TUI at a different library root. Caches (PDF, parse, UAT) and `state.json` stay in their usual machine-local locations, and `astrobib config` reports which library is active and whether it came from the flag or the environment.

### Fixed
- PDF cache status stays in sync across views: every cache mutation now refreshes all views through one helper, and ADS tabs key the cache by the library cite key whenever the paper is imported, so the library and search sides always agree.
- Quitting no longer blocks on in-flight network calls: ADS and PDF requests run on daemon threads that die with the process (previously quit could hang for up to the request timeout), and the browser-download poll is cancelled on exit.
- The pub card's PDF buttons now update the moment the cache changes: clearing swaps open/clear for the download buttons, and a completed download swaps them back — previously the card was stale until the cursor moved.
- The browser-download watcher no longer fails silently when macOS privacy protection blocks the terminal's access to ~/Downloads (pathlib suppresses the PermissionError, so the watcher saw an eternally empty directory): the condition is detected before waiting and reported with the System Settings fix. On timeout the watcher now also explains what it saw (e.g. a file rejected for not being a real PDF). Detection is more robust too: a download overwriting a same-named file is caught, `.PDF` uppercase names match, and preamble bytes before the `%PDF` header are tolerated.
- Clicking the pub card's browser-download button crashed with a TypeError: the handler was never updated for the arXiv-fallback parameter added to the browser resolver, which the `B` key path already passed.
- The key panel (ctrl+p → "Show keys and help panel") now dims unavailable actions like the footer does; Textual's stock panel ignores binding enablement. The `+`/`-` result-count bindings also gained their missing descriptions there.

## 0.3.0 — 2026-07-25

### Added
- Manuscripts may cite by raw ADS bibcode: citation resolution falls back to the bibcode index, so `\citep{2020ApJ...900...12Z}` resolves like any key.
- `astrobib convert bibcode|full|short [--dry-run]`: rewrites all manuscript cite keys to one uniform format and regenerates refs.bib to match.
- `astrobib update`: refreshes entries whose arXiv preprint has since been published, rewriting canonical ADS BibTeX in place under the same cite key; preserves stars and user-curated keywords, updates manuscript-db copies, and reports quota. `--all` re-fetches every entry with an ADS record.
- The pub card shows a dim `(preprint)` marker on entries whose ADS record is still arXiv-only.
- Import now also dedupes by bibcode, catching the same paper stored under a different key.

### Changed
- Cite keys are now fully identity-derived: the year comes from the arXiv submission year (or bibcode year), never the record's publication year, so the same paper yields the same key for every user regardless of preprint/published state. Also fixed the arXiv-ID hash branch, which never matched bibtexparser's lowercased `archiveprefix` field, so existing keys were silently hashing bibcodes.

### Removed
- The unmaintained `ads` package dependency: astrobib now talks to the ADS API directly via httpx (search, BibTeX export, link resolver, quota).

## 0.2.1 — 2026-07-25

### Fixed
- Silenced `SyntaxWarning`s emitted by the `ads` package at first import on Python 3.12 and later (invalid escape sequences in its regex literals; harmless at runtime).

### Added
- Python 3.14 classifier.

## 0.2.0 — 2026-07-25

### Added
- Actions apply to the check-selection: with rows selected via `Space`, remove (`d`), star (`s`), clear cached PDFs (`X`), and download PDFs (`p`) act on the whole selection. PDF downloads run as a sequential batch with progress and a downloaded/failed summary; starring follows the any-unstarred-then-star-all rule.
- Single-paper actions (references `R`, citations `c`, browser download `B`) target the selected row when exactly one is selected, and are disabled while several rows are selected.
- Key listings always show every action, with unavailable actions dimmed instead of hidden.
- `D` shows/hides the pub card; `z` re-shows a hidden card.
- The library table expands into the freed space when the pub card is hidden: the title column is elastic and recomputes on any resize.
- Installation section in the README covering uv, pipx, and venv + pip.

### Fixed
- Action enablement (for example Open PDF) now updates immediately when the selection changes on the library tab.
- Cursor position survives library table rebuilds.

## 0.1.0 — 2026-07-25

Initial release: ADS search tabs with persistent per-manuscript queries, personal library with content-derived cite keys, manuscript databases with a live Manuscript tab and `refs.bib` generation, citing by unambiguous key prefix, local filter query language with ADS escalation, PDF fetching and caching, UAT keyword browser, and a CLI (`add`, `import` with mandatory ADS resolution, `refs`, `export`, `search`, `list`, `show`, `pdf`, `uat`).
