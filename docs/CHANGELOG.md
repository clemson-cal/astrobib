# Changelog

## Unreleased

### Added
- Markdown manuscripts, same policy as LaTeX: `.md` sources beside `bib/` are scanned for citations (`main.md` sole root when present, Obsidian `![[embeds]]` expand like `\input`), with pandoc-style `@key` / `[@a; @b]` cites and Obsidian wikilinks — `[[key]]` counts as a citation when it resolves in the library and stays a note link when it doesn't. Code blocks, inline code, and HTML comments never scan.
- `astrobib refs [FILE] [--dry-run]`: renders the cited-works bibliography into the manuscript's markdown — sorted by author, with year, italic title, journal (AAS macros expanded), and ADS/arXiv/DOI links — between `<!-- astrobib:references -->` markers (a `## References` section is appended on first run); unresolved cites are reported.
- refs.bib is back: for TeX manuscripts the TUI regenerates it silently on every rescan (write-on-change), each entry keyed by the string actually cited so hash suffixes stay out of the manuscript; `astrobib refs [--prune] [-o PATH]` is the CLI sync flow ported from 0.4.0 (copy cited entries into the manuscript db, optionally prune uncited ones with sole-copy rescue, write refs.bib).
- `astrobib tidy` (alias `regularize`): co-author interop — colleagues without astrobib drop raw ADS BibTeX into `bib/` under any filename; tidy canonicalizes those files (reproducible-key fast path, else the ADS lookup ladder), renames them to `{Key}.bib`, dedupes against the library, prints cite-key replacement one-liners, and regenerates refs.bib.
- The pub card in a query scope grows the library card's PDF buttons (and download status) the moment the shown article is imported, acting on the imported entry.
- The manuscript rescans itself: source and bib/ mtimes are polled every ~1.5 s, so editing main.tex/main.md in an external editor (or a git pull) refreshes the Manuscript view and refs.bib with no keypress; externally added or removed bib/ entries reload the tier.
- `astrobib update [--all]`, ported from 0.4.0: refreshes entries whose arXiv preprint has since been published — canonical ADS BibTeX rewritten in place under the same cite key and filename, per-tier user-curated keywords preserved, manuscript copies updated, ADS quota reported.
- Citation-graph navigation from the pub card: "cited by N" on a query card is clickable, and both cards grow citations/references affordances that spawn `citations(bibcode:…)` / `references(bibcode:…)` query scopes — persisted like any saved query.
- A pending-tasks overlay on `T` (or click the `⧗N` footer indicator): every in-flight download, query, import, and browser watch listed with elapsed time and per-task cancel (digits 1-9 or ✕). Thread-backed work can't be killed mid-flight, so cancel marks the task and its result is discarded on arrival — including deleting a cancelled download's late-cached PDFs; the browser watch cancels for real.
- Option/alt+arrow (and emacs alt+b/f) word motions in the filter and query inputs.

### Changed
- Clicking away from the filter prompt leaves entry mode with the filter still applied (Esc still clears), matching the query prompt's click-away policy.

## 0.5.1 — 2026-07-28

### Fixed
- Clicking a table row beyond the library's entry count in a query-results or manuscript scope did nothing: the click handler bounds-checked against the library filter set instead of the active scope's row count (keyboard navigation was unaffected).

### Changed
- ADS query results grew library-side polish: sortable column headers, the hypothetical cite key each article would get on import (hover it to preview the pub card, click to copy), and a full article card with copyable title/abstract/key and clickable ADS/arXiv/DOI links.
- Importing from the card is the footer's `→ import` (clickable, `i` still works); the separate button is gone.
- An article with no open-access PDF is reported as an expected outcome in yellow — "no auto PDF (try browser ↓)" — rather than a red error; red is reserved for actual failures.

## 0.5.0 — 2026-07-28

astrobib is now a single native binary, distributed in platform wheels — `pipx install astrobib` / `uv tool install astrobib` work as before, with no runtime dependencies. Startup and quit are instantaneous. Existing libraries, manuscript databases, and cite keys carry over unchanged.

### Added
- Instant startup and quit; the whole library parses in milliseconds with no cache.
- Scope tabs above the table: the library, one scope per saved ADS query (shared tabs.json, per-manuscript contexts), and a Manuscript scope classifying every `.tex` citation (ok / library / missing / ambiguous, plus uncited members) with `S` pre-filling a search from a missing key.
- Two-tier library model: `astrobib [LIBRARY_DIR]` points at any local bib root (walk-up default, `.git` no longer required); `t` or the `global` badge toggles the global tier — hidden means local-only reads and writes, with sole-copy rescue still protecting removals. `--no-global` starts hidden.
- iOS-style selection mode (Space, gutter click, or option/ctrl+click; `a`/`A` select visible/all) with bulk import, download, membership, and removal; removal sits behind a confirm modal on Delete/Backspace.
- Mouse throughout: clickable scope pills, sortable column headers, hover previews from the citekey column, roll-over styling, double-click opens a cached PDF, clickable pub-card links/buttons/copy-regions, footer view badges.
- Copy without a mouse: `y` which-key chord (key, full key, bibcode, ADS/arXiv/DOI URLs, PDF path, title, abstract); card text regions click-to-copy.
- Event log (`L`) with categories and scrollback; readline-style editing (tui-input, real terminal cursor) in the filter and query prompts; result-count control in the prompt (↑/↓) and `+`/`-` on live scopes.
- Dynamic layout: candidate-fitted author column, responsive column priorities, height-driven abstract display.
- `astrobib import <file.bib>`: canonical astrobib entries (reproducible cite key) import directly; foreign entries resolve against ADS by arXiv ID, DOI, or exact title+author+year, with bibcode-level dedup and copy-pasteable cite-key replacement commands.

### Changed
- Cite keys, short keys, and on-disk `.bib` serialization are byte-identical to 0.4.0; libraries and manuscript dbs need no migration.
- The key panel is a transient `?` cheat-sheet; actions live where their objects are (card buttons, chips, badges).

### Removed
- Starring (`astrobib_starred` is ignored on read and still stripped from manuscript copies) and the `M` ms-only view. (The 0.4.x Python-based implementation remains installable by version pin.)

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
