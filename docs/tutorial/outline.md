# The astrobib tutorial — outline

*Working draft for review. Each chapter states its promise ("after this chapter you can …"); each section gets a one-line synopsis. Target: a small markdown book on GitHub Pages.*

## About this tutorial

Who it's for: astrophysics researchers and grad students who frequently use LaTeX, ADS, and arXiv, are comfortable in a terminal, and are new to astrobib. It assumes you can install a command-line tool, have (or can create) a free ADS account, and write papers using the bibtex system. Feature reference lives in the README, whereas this book illustrates useful workflows.

**Running example, threaded through the book:** you're starting a paper on magnetar engines for fast radio bursts. You build a library around the topic, start the paper's git repo with its own `bib/`, write `main.tex` against auto-generated `refs.bib`, hand the repo to a co-author who doesn't use astrobib, and tidy up before submission — while your personal library quietly accrues everything you touched.

---

## 1. Why astrobib

*After this chapter you can explain what astrobib stores on disk and why you'd trust it with your references.*

- **Plain text everything** — the library is a directory of `bib/*.bib`, one paper per file; use git for version control.
- **Stable and universal cite keys** — keys are derive from a paper's stable identity (arXiv ID or bibcode), so references to the same paper generate a stable, universal, and memorable key.
- **ADS-native** — search, fetch, citation graphs, and metadata refresh all interact directly with the NASA/Harvard ADS.
- **Fast** — instant startup and quit; astrobib is a TUI and a CLI at the same time.

## 2. Installation and first use

*After this chapter you have astrobib installed, an ADS token saved, and your first paper in the library.*

- **Install** — `uv tool install astrobib` or `pipx install astrobib`; platform wheels for macOS and Linux, `cargo install` from source.
- **Get an ADS token** — where to create it on the ADS website; `export ADS_API_TOKEN=...` for shell users.
- **First run** — launch `astrobib`; with no token saved, pressing `S` prompts for the token (and optional email) right in the footer and remembers it.
- **First import** — run your first ADS query with `S`, import a paper with `i`, and see the `.bib` file it wrote.

## 3. Reading the screen

*After this chapter you can navigate the TUI without thinking about it.*

- **The layout** — scope capsules on top, the paper table, the pub card on the right, event log and clickable view badges at the bottom.
- **Moving around** — `j k g G`, `[` `]` between scopes, mouse everywhere (rows, capsules, headers, card), hover hints in the footer.
- **The safety nets** — `?` opens the keyboard cheat-sheet, `L` the event log, `T` (or the `⧗N` badge) the pending-tasks overlay, `@` the about modal with version and update check.
- **Panels on demand** — `D` toggles the pub card, `L` the log; everything closes with a key, `q` quits instantly.

## 4. Building a library from ADS

*After this chapter you can turn a research question into a curated set of papers.*

- **Ad-hoc queries** — `S` sends your query to ADS unmodified, so the full search syntax works (`^author`, `bibstem:ApJL`, `citations(...)`, boolean grouping); ↑/↓ sets the result count.
- **Query scopes are tabs** — each query becomes a persistent capsule (saved globally, or with the manuscript you are in; `H` moves it), refreshed with `r`, resized with `+`/`-`, closed with `ctrl+w`.
- **Importing results** — `i` on a row or a selection; the results table already shows the cite key each paper *would* get.
- **Direct adds** — paste a DOI or ADS URL into the `S` prompt to import in one step; `astrobib add <bibcode>` from the shell.
- **Running example** — build the FRB paper's starting set: `^lyubarsky year:2014-`, `kw:"fast radio bursts"`, a `bibstem:` sweep.

## 5. Finding things again: the filter language

*After this chapter you can pull any paper out of a thousand-entry library in a few keystrokes.*

- **Live filtering** — `/` filters as you type; terms AND together, case-insensitive, partial matches; a half-typed query never errors.
- **Field prefixes** — `author:`, `^first-author`, `title:`, `abs:"phrase"`, `kw:`, `year:2015-2020` ranges.
- **Combinators and states** — uppercase `OR`, leading `-` or `NOT` to negate, `is:pdf` (cached PDF) and `is:ms` (local/manuscript member).
- **Escalating to ADS** — with a filter active, `S` pre-fills the equivalent ADS query: filter locally, search globally in one keystroke.

## 6. The pub card

*After this chapter you can read, copy, and act on everything astrobib knows about a paper.*

- **Anatomy** — title, byline with "cited by N", publication line (journal, volume, pages), abstract; hover the citekey column to preview other papers.
- **The link stack** — badged rows: ↗ opens ADS/arXiv/DOI in the browser, ⌕ acts inside astrobib; unavailable rows stay visible but dimmed.
- **The permanent copy column** — every ⧉ target click-to-copy on the card, or the `y` chord from the keyboard (`yy` key, `yb` bibcode, `yt` title, …).
- **The bib source view** — `v` (or the `@ bib` corner toggle) shows the raw `.bib` verbatim; on un-imported query results it previews exactly what an import would write.

## 7. Walking the citation graph

*After this chapter you can trace a subfield's literature outward from any paper.*

- **Citations and references** — `C` / `R` (or the card's ⌕ rows) open `citations(...)` / `references(...)` query scopes for the shown paper.
- **Why it's reliable** — the queries are identifier-based, so preprint-imported papers resolve to their canonical ADS record.
- **Graph-walking as workflow** — chains of citation scopes persist as tabs; prune them with `ctrl+w` when a trail goes cold.
- **Running example** — from the FRB 121102 discovery paper, walk citations forward to the modern magnetar-engine literature.

## 8. PDFs

*After this chapter you have a local PDF for every paper you actually read.*

- **Auto-download** — `p` tries ADS open access, then arXiv, then ADS's scan service; works on selections; yellow "no auto PDF" is an expected outcome, not an error.
- **The browser fallback** — `B` opens the publisher page and watches `~/Downloads` for the arriving PDF; "pick …" grabs an already-downloaded file via a modal picker.
- **Living with PDFs** — `o` or double-click opens, `X` clears (or cancels a download), `is:pdf` filters to what's cached.

## 9. Starting a paper: the two-tier model

*After this chapter you can give a paper repo its own self-contained bibliography without giving up your personal library.*

- **Two tiers** — tier 1 is your global library (`~/.local/share/astrobib/library/`); tier 2 is any directory holding `bib/`, found by walk-up or named as `astrobib [LIBRARY_DIR]`.
- **Reads merge, writes go to both** — the paper repo stands alone for co-authors while your collection accrues; `m` toggles a paper's local membership.
- **The `t` toggle** — hide the global tier (or click the `global` badge) for purely local reads and writes; `--no-global` starts that way.
- **Sole-copy rescue** — removing a local paper never destroys the only copy; it's rescued into the global library first.
- **Running example** — `mkdir frb-magnetar && cd frb-magnetar`, create `bib/` by importing, and watch the Manuscript scope appear once `main.tex` exists.

## 10. Writing in LaTeX

*After this chapter you can `\cite` freely and never hand-edit a `.bib` file again.*

- **Citation tracking** — `.tex` sources beside `bib/` are scanned (`main.tex` root, `\input`/`\include` expanded); the Manuscript scope classifies every cite: ok, in-library, missing, ambiguous, plus uncited members.
- **refs.bib writes itself** — regenerated silently on every rescan, each entry emitted under the string you actually cited, so hash suffixes never appear in your `.tex`.
- **External edits just work** — file mtimes are polled, so editing in your editor (or a `git pull`) refreshes the view and `refs.bib` with no keypress.
- **From a missing cite to a fix** — `S` pre-fills a search from a missing key; `astrobib refs [--prune] [--dry-run]` is the same sync from the CLI.

## 11. Markdown manuscripts and notes

*After this chapter you can run a literature review or Obsidian vault as a first-class manuscript.*

- **Same policy as TeX** — `.md` files beside `bib/` are scanned (`main.md` sole root when present; `![[embeds]]` expand like `\input`); code blocks and comments never scan.
- **Two citation dialects** — pandoc-style `@Key` / `[@A; @B]`, and Obsidian `[[wikilinks]]` that count as citations only when they resolve in the library.
- **A rendered bibliography** — `astrobib refs` writes a sorted, linked reference list between `<!-- astrobib:references -->` markers; regenerate any time, your prose is never touched.
- **Running example** — a `notes.md` review of FRB progenitor models that later feeds the paper's introduction.

## 12. Co-authors, upkeep, and the CLI

*After this chapter you can run a multi-author paper to submission and keep your library current after it.*

- **Co-authors don't need astrobib** — they paste ADS BibTeX into `bib/any-name.bib`; `astrobib tidy` later canonicalizes (re-key via ADS, rename to `{Key}.bib`, dedupe), prints copy-pasteable cite-key replacements, and regenerates `refs.bib`.
- **Adopting a legacy bibliography** — `astrobib import refs.bib` resolves foreign entries against ADS (arXiv ID → DOI → title+author+year), with `--global-only` / `--local-only` targeting. `--dry-run` first: it resolves everything and shows the whole map — files, tiers, and the cites `--rename-citekeys` would rewrite — before anything is written.
- **Preprints grow up** — `astrobib update [--all]` refreshes published metadata in place, same key and filename forever, manuscript copies included.
- **The CLI tour** — `list`, `search [--ads]`, `add`, `show`, `refs`, `tidy`, `update`, plus `--library` and `--no-global`; scripting and cron-able upkeep.
- **Running example, closing loop** — co-author drops two raw entries, you `tidy`, `update` before resubmission, and the final repo compiles for anyone with just `git clone` and `latex`.

---

## Production notes (for the author — not part of the book)

- **Tooling: mdBook.** One static binary from the same ecosystem as astrobib, first-class GitHub Pages deploy via `peaceiris/actions-gh-pages` or the official starter workflow, built-in client-side search and dark/light themes that suit a terminal-tool audience. Jekyll adds a Ruby toolchain for no gain; bare markdown loses search and navigation. The book can live in `docs/tutorial/` with `src/` per mdBook convention; chapters above map 1:1 to `SUMMARY.md` entries.
- **Screenshots from the pyte harness.** `tests/tui/driver.py` already drives the real binary against scratch fixtures and reconstructs full screens with pyte, including colors and mouse events. A small capture script can reuse it to (a) build a deterministic demo library from fixtures, (b) script a key sequence per figure, and (c) dump the pyte buffer with SGR attributes to HTML (or ANSI → `ansi2html`/`aha`) for crisp, reproducible "screenshots" that regenerate when the UI changes — better than PNGs for diffing. Animated captures: replay the same scripted sequences under `asciinema rec` in a pty, or emit asciicast v2 JSON directly from the driver's timed frames.
- **Doc changes the tutorial surfaces:**
  - The 0.6.0 changelog says `c`/`C` open citations/references, but the shipped cheat-sheet binds `C`/`R`. Worth a changelog correction or at least consistency in the book (the tutorial uses `C`/`R`, matching `?`).
  - Several clap flags have empty help strings (`search --ads`, `add --force`, `list/search -n`); one-line help texts would let the book quote `--help` verbatim.
  - README documents the filter and key surface tersely by design; the book should link back to it as the reference card rather than duplicating, and the README could gain a one-line pointer to the tutorial once published.
  - DESIGN.md mentions `config.toml` only speculatively ("when one exists") — the tutorial should stay silent on configuration to avoid promising a file that isn't there.
