# astrobib
*A terminal-based literature manager for astrophysics research*

[![PyPI](https://img.shields.io/pypi/v/astrobib.svg)](https://pypi.org/project/astrobib/) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

astrobib connects to the [NASA/Harvard ADS](https://ui.adsabs.harvard.edu) to search, fetch, and organize papers as plain BibTeX files, with a fast terminal UI. It ships as a single native binary: instant startup, instant quit, no runtime dependencies.

Your library is just a directory of `.bib` files, indistinguishable from hand-written BibTeX, and cite keys derive from each paper's stable identity (arXiv ID or bibcode) — so any two copies of a paper, fetched by anyone at any time, agree on the key forever. Libraries from all earlier astrobib versions work unchanged.

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
astrobib search '^zrake OR kw:"compact objects"'
astrobib add 2020ApJ...123..456Z
astrobib import refs.bib     # resolve a foreign .bib against ADS
```

---
## The two-tier library model
astrobib always works on up to two libraries: a **local** bib directory (tier 2) and your **global** personal library (tier 1, at `~/.local/share/astrobib/library/`).
`astrobib [LIBRARY_DIR]` points tier 2 at any directory holding `bib/`; with no argument, the nearest ancestor of the current directory with a `bib/` is used. A `.tex` or `.md` manuscript alongside activates citation tracking, but any bib directory works.
With the global tier enabled (the default), reads merge both tiers and imports write to both — the paper repo stands alone for coauthors while your collection accrues. Press `t` (or click the `global` badge) to hide the global tier: reads and writes become purely local. Removing a local paper never destroys a sole copy — it is rescued into the global library.
Both stores are plain `bib/*.bib` files, one paper per file, indistinguishable from hand-written BibTeX. Nothing else is ever written into your repos.

---
## TUI overview
Scope capsules at the top switch between your library, saved ADS query tabs, and the manuscript view. The pub card on the right shows the highlighted paper (hover the citekey column to preview others); an event log and clickable view badges sit at the bottom. Most things are clickable; every action has a key (`?` shows the cheat-sheet).

### Keys
- `/` — live filter (query language below); `S` — new ADS query (↑/↓ sets result count; pasting a DOI or ADS URL imports directly)
- `j k g G` — move; `[` `]` — switch scope; `ctrl+w` — close query scope; `r` — refresh; `+` `-` — result count
- `Space` — select row (iOS-style selection mode); `a` — select visible; `A` — select all; Esc — done
- `i` — import ADS result(s); `m` — toggle manuscript/local membership; `⌫` — remove (with confirmation)
- `p` — download PDFs (ADS open-access, then arXiv); `B` — browser download (watches ~/Downloads); `o` — open PDF; `X` — clear PDF; double-click a row — open its PDF
- `y` — copy chord: `yy` cite key, `yY` full key, `yb` bibcode, `ya`/`yx`/`yd` ADS/arXiv/DOI URL, `yp` PDF path, `yt` title, `yA` abstract; card title/abstract/key are click-to-copy
- `t` — show/hide the global tier; `T` — pending-tasks overlay (also: click `⧗N`); `D` — pub card; `L` — event log; `?` — keys; `q` — quit

---
## Filtering the library
Press `/` to filter as you type. Whitespace-separated terms AND together; each term is a case-insensitive partial match. Bare terms match author, title, abstract, key, keywords, and year; field prefixes narrow:
```
author:sironi          author anywhere in the list
^zrake                 first-author papers (= author:^zrake)
title:magnetar         word in title
abs:"fast radio burst" phrase in abstract
kw:"compact objects"   keyword
year:2015-2020         ranges; year:2020- open-ended
is:ms                  local/manuscript members;  is:pdf  cached PDFs
-abs:neutrino          leading - negates (NOT works too)
^zrake OR ^metzger     uppercase OR separates alternatives; AND binds tighter
```
A half-typed query never errors. With a filter active, `S` pre-fills the equivalent ADS query — filter locally, escalate in one keystroke.

---
## ADS queries
`S` passes your query to the [ADS API](https://ui.adsabs.harvard.edu/help/search/search-syntax) unmodified, so the full Solr language works (`bibstem:ApJL`, `citations(...)`, boolean grouping, …). Each query becomes a scope capsule, persisted per library context in `tabs.json`.
The pub card walks the citation graph directly: click "cited by N" (or the citations/references affordances) to open a `citations(...)` or `references(...)` scope for the shown paper.

---
## Markdown manuscripts
Literature reviews and notes work as manuscripts too: any `.md` files beside `bib/` are scanned for citations (`main.md` is the sole root when present; Obsidian `![[embeds]]` pull in more files, like `\input`).
Cite pandoc-style — bare `@Zrake2019` or bracketed `[@Zrake2019; @Metzger2017]` — or with Obsidian wikilinks: `[[Zrake2019]]` counts as a citation when it resolves in the library, and stays an ordinary note link when it doesn't. An unresolved `@cite` shows as missing in the Manuscript view, same as LaTeX.
`astrobib refs` renders the bibliography of everything cited into the manuscript — a sorted, linked reference list (authors, year, italic title, journal, ADS/arXiv/DOI links) kept between `<!-- astrobib:references -->` markers, appended as a `## References` section the first time. Regenerate any time; your prose is never touched.

---
## refs.bib and co-authors
For TeX manuscripts, `refs.bib` regenerates silently whenever the TUI rescans — and the TUI rescans itself when you edit sources externally (mtimes are polled, like the original app): every cited manuscript-db member, emitted under the string you actually cited (full key or unambiguous prefix), so hash suffixes never need to appear in your `.tex`. `astrobib refs [--prune]` does the same from the CLI, first copying cited-but-missing entries into the manuscript db (`--prune` also removes uncited ones, rescuing sole copies).
Co-authors don't need astrobib. They add a reference by pasting BibTeX from the ADS website into `bib/any-name.bib` (and, if they like, appending it to `refs.bib` by hand so the paper still compiles). Next time you check out the repo, `astrobib tidy` canonicalizes those files — re-keys them through ADS when needed, renames them to `{Key}.bib`, dedupes, rewrites the old keys inside your sources — and regenerates `refs.bib` for the commit.
Migrating a manuscript that predates astrobib works the same way: in a directory with sources and its own loose `.bib` files but no `bib/`, `astrobib tidy` adopts it wholesale — builds `bib/`, resolves everything against ADS, rewrites your `\cite` keys in place, and regenerates `refs.bib`.

---
## CLI
`list`, `search [--ads]`, `add <bibcode|ADS URL>`, `show <key>`, `rm <key> [--local-only]` (sole copies rescued), `import <file.bib> [--global-only|--local-only]`, `refs [FILE] [--prune] [--dry-run]`, `tidy [--dry-run]`, `convert bibcode|full|short` (uniform cite keys, rewritten in your sources), `update [--all]` (arXiv → published refresh, same key forever), `config` (the resolved environment), plus `--library PATH` (relocate the global tier) and `--no-global`.
`import` resolves each entry against ADS (arXiv ID → DOI → exact title+author+year) unless its cite key is already reproducible from its own data — canonical astrobib entries import byte-identically — and prints copy-pasteable key replacements for your `.tex` files.

---
See [docs/DESIGN.md](docs/DESIGN.md) for the data-format contract. Bugs and feature requests: [github.com/clemson-cal/astrobib/issues](https://github.com/clemson-cal/astrobib/issues).

© 2026 Jonathan Zrake · MIT license · Clemson University Physics and Astronomy · Supported by NSF award number 2408034
Development assisted by Claude (Fable 5).
