# astrobib
*A terminal-based literature manager for astrophysics research*

[![PyPI](https://img.shields.io/pypi/v/astrobib.svg)](https://pypi.org/project/astrobib/) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

astrobib connects to the [NASA/Harvard ADS](https://ui.adsabs.harvard.edu) to search, fetch, and organize papers as plain BibTeX files, with a fast terminal UI. It ships as a single native binary: instant startup, instant quit, no runtime dependencies.

Your library is just a directory of `.bib` files, indistinguishable from hand-written BibTeX, and cite keys derive from each paper's stable identity (arXiv ID or bibcode) — so any two copies of a paper, fetched by anyone at any time, agree on the key forever. Libraries from all earlier astrobib versions work unchanged.

### Development status

0.19.0 ships `import --rename-citekeys` (the re-key map applied to your sources, TeX and markdown alike) and `import --dry-run` (the whole map previewed against the library the import would leave behind, writing nothing), together with a `✕` on every query capsule and `⌃w` named on the keys panel. Since then the manuscript page has grown the library's columns — the metric swatch, `↓`, `Year`, `Author` and `Key` are drawn, sorted and configured there like anywhere else, alongside `Cited` and `State`. Next up: `convert` scanning markdown as well as it scans TeX. Rust tests, clippy, and the headless TUI suite pass; the completed TUI hit-test registry refactor is recorded in [docs/plans/hit-registry.md](docs/plans/hit-registry.md).

---
## Installation
```bash
uv tool install astrobib     # or: pipx install astrobib
```
Binary wheels cover macOS (arm64, x86_64) and Linux (x86_64, aarch64). Building from source needs a Rust toolchain: `cargo install --git https://github.com/clemson-cal/astrobib`.

---
## Quick start
```bash
# set your ADS token (https://ui.adsabs.harvard.edu/user/settings/token)
export ADS_API_TOKEN=...

astrobib                     # launch the TUI
astrobib list                # CLI: newest papers
astrobib search '^andersson OR kw:"compact objects"'
astrobib add 2020ApJ...123..456Z
astrobib import refs.bib     # resolve a foreign .bib against ADS
```

---
## The two-tier library model
astrobib always works on up to two libraries: a **local** bib directory (tier 2) and your **global** personal library (tier 1, at `~/.local/share/astrobib/library/`).
`astrobib [LIBRARY_DIR]` points tier 2 at any directory holding `bib/`; with no argument, the nearest ancestor of the current directory with a `bib/` is used. Any `bib/` directory activates the local library. A `.tex` or `.md` manuscript source alongside it additionally activates the Manuscript scope and citation tracking; conventional project docs such as `README.md` and `CHANGELOG.md` are ignored.
With the global tier enabled (the default), reads merge both tiers and imports write to both — the paper repo stands alone for coauthors while your collection accrues. Press `t` (or click the `global` badge) to hide the global tier: reads and writes become purely local. Removing a local paper never destroys a sole copy — it is rescued into the global library.
Both stores are plain `bib/*.bib` files, one paper per file, indistinguishable from hand-written BibTeX. The only other thing astrobib ever writes into your repos is `tags/`, and only once you make a tag.

---
## TUI overview
Scope capsules at the top switch between your library, saved ADS query tabs, and the manuscript view. A query's tab appears the moment it is sent — an ADS query can take a minute — and the page says what it is waiting for, then how it ended: results, nothing found, or why it failed. Each keeps its own sort, and remembers it. An optional one-cell colour swatch column shows a metric — your own decaying priority, or ADS citation counts; switch it on from the table panel (`|`), like any other column. The pub card on the right shows the highlighted paper (hover the citekey column to preview others); an event log and clickable view badges sit at the bottom. Most things are clickable; every action has a key (`?` shows the cheat-sheet).

### Keys
- `/` — live filter (query language below); `S` — new ADS query (`↑`/`↓` sets the result count, `⌃r` opens the menu of what ADS returns — both also clickable; pasting a DOI or ADS URL imports directly)
- `j k g G` — move; `[` `]` — switch scope (`]` past the last one composes a new query); `ctrl+w` — close the query you are on, or click the `✕` on any query capsule to close that one; `r` — refresh; `+` `-` — result count
- `Space` — select row (iOS-style selection mode); `a` — select visible; `A` — select all; Esc — done
- `i` — import ADS result(s); `m` — toggle manuscript/local membership; `⌫` — remove (with confirmation)
- `T` — tag ± the selection: type a name, `⏎` applies. It adds, unless every selected paper already carries that tag, in which case it untags — the prompt says which before you commit. With an empty name it lists the tags you have, so one is harder to mistype into existence
- `p` — download PDFs (ADS open-access, then arXiv); `B` — browser download (watches ~/Downloads); `o` — open PDF; `X` — clear PDF; double-click a row — open its PDF
- `y` — copy chord: `yy` cite key, `yY` full key, `yb` bibcode, `ya`/`yx`/`yd` ADS/arXiv/DOI URL, `yp` PDF path, `yt` title, `yA` abstract; card title/abstract/key are click-to-copy
- `y q` — copy the active query's whole configuration: its text, its result count and what ADS returns, as an ADS search URL. `P` — open the query configuration on the clipboard (it says so if the clipboard holds something else). Both round-trip, so a query pasted to a colleague arrives as the query you sent
- In any prompt: `⌃k` / `⌃u` / `⌃w` kill to end of line, to start of line, and the previous word, and `⌃y` yanks the last of them back; `⌥w` copies what you are composing (from a query, the same URL as `y q`)
- `E` — edit the active query in place (text, result count and what ADS returns); `S` always composes a new one
- `N` — name the active query (the capsule label; persists across edits to the query, empty restores the derived name)
- `H` — move the active query between its two homes: the global set, visible from every directory, and the manuscript you are in. The footer says which, as `⌂ everywhere` or `⌂ this paper`, and clicking it does the same thing as the key. Pressing it twice puts the query back where it was
- `C` / `R` — open a citations / references query for the shown paper; `v` — pub view (the raw .bib); `e` — export the selection to a .bib file
- `M` — pick the metric the swatch column shows (priority or citations, distinct colormaps; show the column itself from `|`); `.` — priority 1.0, `0` — clear, `<` `>` — scale it (decays weekly); the wheel over a swatch does the same for that row
- `|` — table panel: show/hide columns, `←`/`→` to resize, `s` to sort by any of them (shown or not); `Tab` swaps the arrow keys between the panel and the table, `Esc` hands them back
- `t` — show/hide the global tier; `D` — pub card; `L` — event log; `?` — keys; `@` — about; `q` — quit

The side panels, the pub card and the footer are separated from the table by a faint tint rather than by border lines, and the tint is chosen from your terminal's own background: darker than it on a light theme, lighter on a dark one. astrobib asks the terminal at startup (OSC 11) and falls back to dark for terminals that do not answer — set `ASTROBIB_THEME=light` or `dark` to decide it yourself.

---
## Filtering the library
Press `/` to filter as you type. Whitespace-separated terms AND together; each term is a case-insensitive partial match. Bare terms match author, title, abstract, key, keywords, and year; field prefixes narrow:
```
author:cabrera         author anywhere in the list
^andersson             first-author papers (= author:^andersson)
title:magnetar         word in title
abs:"fast radio burst" phrase in abstract
kw:"compact objects"   keyword
year:2015-2020         ranges; year:2020- open-ended
is:ms                  local/manuscript members;  is:pdf  cached PDFs
is:tagged              carries at least one tag;  -is:tagged  carries none
tag:section-3          papers in a tag (substring; tags are yours, not ADS's)
pri:>0.5  cit:>100    metric comparisons (> < or bare for ≥); no metric never matches
-abs:neutrino          leading - negates (NOT works too)
^andersson OR ^baxter  uppercase OR separates alternatives; AND binds tighter
```
Long queries wrap across as many rows as they need rather than scrolling out of sight; the text stays one line, and ⏎ still runs it. A half-typed query never errors. With a filter active, `S` pre-fills the equivalent ADS query — filter locally, escalate in one keystroke.

Pressing `/` offers these four as a starting point; click one to load it into an empty filter.
```
^andersson year:2019-                 first author, open-ended years
abs:"fast radio burst"                phrase in the abstract
is:pdf pri:>0.5                       has a PDF, high priority
kw:"compact objects" -abs:neutrino    keyword, and a negation
```

---
## Tags
A tag is a named collection of papers — "the spiral-shock references for section 3" — and it lives in the database under version control, because a topical grouping is a statement about the literature that your coauthors benefit from.
It is a `tags/` directory beside `bib/`, one plain-text file per tag, one cite key per line, sorted. The file *is* the citekey dump: handing a collection to a colleague is `cat tags/section-3`, with no export step that can fall out of step with it.
`T` tags the selection, `tag:` filters on the result. The pub card names the tags each paper carries, and clicking one filters the library to it — following a grouping is a click, not a thing to retype. Tags are written to the tier you are pointed at — the local db when there is one, so a section's references live in the manuscript repo, else your global library. Reads take the union of both tiers rather than letting one shadow the other: a paper tagged `disk-instability` in your library and `section-3` in a manuscript is genuinely both.
Hand-written tag files are first-class, and a key naming a paper you have not imported yet is kept rather than dropped — astrobib only says how many it could not find. `astrobib tidy` sorts and dedupes them, keeping your comment lines.

---
## ADS queries
`S` passes your query to the [ADS API](https://ui.adsabs.harvard.edu/help/search/search-syntax) unmodified, so the full Solr language works (`bibstem:ApJL`, `citations(...)`, boolean grouping, …). Each query becomes a scope capsule, persisted in `tabs.json`.
A query is saved either globally — visible from every directory — or with the manuscript you are in, and the strip marks where one group ends and the other begins. What you type is global; `citations(…)` and `references(…)` are about one paper, so they stay with the manuscript. The footer names where the query you are on is visible — `⌂ everywhere` or `⌂ this paper` — and `H`, or a click on it, moves it to the other. Without a manuscript, all queries are global and no home indicator is shown.

**What ADS returns** (`⌃r` while composing) is the ADS `sort` parameter — the same thing the sort dropdown on the ADS website sets — but it does a different job here. Paired with the result count it decides *which* records come back, not how they are arranged: "most cited" gives you the most cited among the n selected, so changing it changes the papers rather than the order. Every field ADS sorts by is offered, under ADS's own names: entry date, publication date, citation count, normalized citation count, classic factor, read count, author count, first author, bibcode and relevance. The menu is arrow-driven: `↑`/`↓` choose what to rank by, and either of `←`/`→` turns the whole list between most-first and least-first, since that is one question rather than one per field. Every move applies at once, so the prompt always reads as what a search would do. ADS's dropdown also lists Title; it is left out because it sorts nothing — `title asc`, `title desc` and relevance return identical results.
The pub card walks the citation graph directly: click "cited by N" (or the citations/references affordances) to open a `citations(...)` or `references(...)` scope for the shown paper.

Pressing `S` offers these four as a starting point; click one to load it into an empty prompt.
```
abs:"little red dot" -doctype:abstract    phrase, minus meeting abstracts
author:"^Andersson, K." year:2020-        first author, from a year on
bibstem:ApJL abs:"magnetar"               one journal
arxiv_class:astro-ph.HE                   an arXiv subject class
```
Samples never carry an absolute upper year: recency is the prompt's own control (`⌃r`), and a baked-in end year would silently exclude the newest work once it passed.

A saved query reads as a feed: ADS is asked for the newest records by *entry date* — when it indexed them — not by publication date, so `r` brings back what has appeared since you last looked rather than what was published most recently. The `Entered` column shows that date beside `Year`; the table panel picks which of the four selection sorts a query uses. That sort chooses which records come back, so ordering a feed by citations gives the most cited among the newest n, not the most cited overall.

---
## Markdown manuscripts
Literature reviews and notes work as manuscripts too: any `.md` files beside `bib/` are scanned for citations (`main.md` is the sole root when present; Obsidian `![[embeds]]` pull in more files, like `\input`).
Cite pandoc-style — bare `@Andersson2021` or bracketed `[@Andersson2021; @Baxter2019]` — or with Obsidian wikilinks: `[[Andersson2021]]` counts as a citation when it resolves in the library, and stays an ordinary note link when it doesn't. An unresolved `@cite` shows as missing in the Manuscript view, same as LaTeX.
`astrobib refs` renders the bibliography of everything cited into the manuscript — a sorted, linked reference list (authors, year, italic title, journal, ADS/arXiv/DOI links) kept between `<!-- astrobib:references -->` markers, appended as a `## References` section the first time. Regenerate any time; your prose is never touched.

---
## Building with make
The dependency graph of a paper is acyclic, and `astrobib refs` implements the middle of it:

```
main.pdf  <-  main.tex, refs.bib
bib/      <-  main.tex          (citing a paper pulls it out of your library)
refs.bib  <-  main.tex, bib/
```

`astrobib refs` — copy newly cited papers into `bib/`, then write `refs.bib` from what is there. It stamps `refs.bib`'s mtime even when the content is unchanged, so a make rule settles instead of re-running every build; no `touch $@` is needed.
`astrobib refs --check` — verify only: writes nothing, exits nonzero if `refs.bib` is stale or a cited paper is still missing from `bib/`. For CI and pre-commit hooks.
`astrobib refs --no-sync` — write `refs.bib` from what `bib/` already holds and fetch nothing, for a CI job that must not modify tracked files.

A complete Makefile is in [docs/examples/Makefile](docs/examples/Makefile).

---
## refs.bib and co-authors
For TeX manuscripts, `refs.bib` regenerates silently whenever the TUI rescans — and the TUI rescans itself when you edit sources externally (mtimes are polled, like the original app): every cited manuscript-db member, emitted under the string you actually cited (full key or unambiguous prefix), so hash suffixes never need to appear in your `.tex`. `astrobib refs [--prune]` does the same from the CLI, first copying cited-but-missing entries into the manuscript db (`--prune` also removes uncited ones, rescuing sole copies).
Co-authors don't need astrobib. They add a reference by pasting BibTeX from the ADS website into `bib/any-name.bib` (and, if they like, appending it to `refs.bib` by hand so the paper still compiles). Next time you check out the repo, `astrobib tidy` canonicalizes those files — re-keys them through ADS when needed, renames them to `{Key}.bib`, dedupes, rewrites the old keys inside your sources — and regenerates `refs.bib` for the commit.
Migrating a manuscript that predates astrobib works the same way: in a directory with sources and its own loose `.bib` files but no `bib/`, `astrobib tidy` adopts it wholesale — builds `bib/`, resolves everything against ADS, rewrites your `\cite` keys in place, and regenerates `refs.bib`.

---
## CLI
`list`, `search [--ads]`, `add <bibcode|ADS URL>`, `show <key>`, `rm <key> [--local-only]` (sole copies rescued), `import <file.bib> [--global-only|--local-only] [--cited-only] [--rename-citekeys] [--dry-run]`, `refs [FILE] [--prune|--no-sync|--check] [--dry-run]`, `tidy [--dry-run]`, `convert bibcode|full|short` (uniform cite keys, rewritten in your sources), `update [--all]` (arXiv → published refresh, same key forever), `config [ads_token|email <value>]` (show or set the environment, including how much of the day's ADS allowance the token has spent), `gc` (report what the machine-local caches cost), plus `--library PATH` (relocate the global tier) and `--no-global`.
`import` resolves each entry against ADS (arXiv ID → DOI → exact title+author+year) unless its cite key is already reproducible from its own data — canonical astrobib entries import byte-identically — and prints the old→new key map it produced.
`import --rename-citekeys` applies that map to your sources, so a Zotero or Overleaf export that re-keys nearly every entry does not leave the manuscript citing strings that no longer exist. `.tex` and `.md` alike — pandoc `@Key`, bracketed `[@A; @B]` and Obsidian `[[Key]]` are rewritten in their own syntaxes, prose and code are never touched — and it prints what it changed per file. It refuses, before importing anything, unless you are in a manuscript with sources. Your bibliography is stale afterwards: run `astrobib refs`.
`import --dry-run` resolves the whole file and reports what it would do — which `.bib` files into which tier, and which cites it would rewrite in which sources — without writing any of it. The keys it shows are the ones the real run produces: a short key is the shortest unambiguous prefix in the library the import leaves behind, so the preview shortens against your library plus the entries it is about to add.
`import --cited-only` takes just the entries your manuscript cites and leaves the rest of the file alone — the sane way to accept a coauthor's `refs.bib` when it is really their whole collection. Cites are read from your `.tex`/`.md` sources and matched against the file's own keys (full key, unambiguous prefix, or bibcode), before anything is sent to ADS.

---
See [docs/DESIGN.md](docs/DESIGN.md) for the data-format contract. Bugs and feature requests: [github.com/clemson-cal/astrobib/issues](https://github.com/clemson-cal/astrobib/issues).

© 2026 Jonathan Zrake · MIT license · Clemson University Physics and Astronomy · Supported by NSF award number 2408034
Development assisted by Claude (Fable 5).
