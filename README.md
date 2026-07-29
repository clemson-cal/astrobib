# astrobib
*A terminal-based literature manager for astrophysics research*

astrobib connects to the [NASA/Harvard ADS](https://ui.adsabs.harvard.edu) to search, fetch, and organize papers as plain BibTeX files, with a fast terminal UI. It is written in Rust and ships as a single binary — installs via pip-style tools involve no Python runtime.

The earlier Python implementation is preserved at tag [`v0.4.0`](https://github.com/clemson-cal/astrobib/tree/v0.4.0). Libraries and cite keys are fully compatible between the two: keys derive from each paper's stable identity (arXiv ID or bibcode), so any two copies of a paper agree on its key forever.

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
`astrobib [LIBRARY_DIR]` points tier 2 at any directory holding `bib/`; with no argument, the nearest ancestor of the current directory with a `bib/` is used. A `.tex` manuscript alongside activates citation tracking, but any bib directory works.
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
- `t` — show/hide the global tier; `D` — pub card; `L` — event log; `?` — keys; `q` — quit

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
`S` passes your query to the [ADS API](https://ui.adsabs.harvard.edu/help/search/search-syntax) unmodified, so the full Solr language works (`bibstem:ApJL`, `citations(...)`, boolean grouping, …). Each query becomes a scope capsule, persisted per manuscript context and shared with the Python implementation via `tabs.json`.

---
## CLI
`list`, `search [--ads]`, `add <bibcode|ADS URL>`, `show <key>`, `import <file.bib> [--global-only|--local-only]`, plus `--library PATH` (relocate the global tier) and `--no-global`.
`import` resolves each entry against ADS (arXiv ID → DOI → exact title+author+year) unless its cite key is already reproducible from its own data — canonical astrobib entries import byte-identically — and prints copy-pasteable key replacements for your `.tex` files.

---
See [docs/DESIGN.md](docs/DESIGN.md) for the data-format contract and [docs/STATUS.md](docs/STATUS.md) for implementation status.
