# astrobib
*A terminal-based literature manager for astrophysics research*

astrobib is a personal astrophysics literature manager. It connects to the [NASA/Harvard ADS](https://ui.adsabs.harvard.edu) to search and fetch papers, stores BibTeX in `~/.local/share/astrobib/library/`, and generates `refs.bib` files for LaTeX manuscripts by scanning for cite keys.

Keywords follow the [Unified Astronomy Thesaurus (UAT)](https://astrothesaurus.org), the controlled vocabulary used by AAS journals.

---
## Installation
Requires Python 3.11 or later. Any of the following provides the `astrobib` command:
```bash
# uv (recommended): isolated per-user tool install; fetches a suitable Python if needed
uv tool install astrobib

# pipx: the same isolated-tool model
pipx install astrobib

# standard venv + pip
python3 -m venv ~/.venvs/astrobib
~/.venvs/astrobib/bin/pip install astrobib
# then invoke ~/.venvs/astrobib/bin/astrobib, or add it to your PATH
```
Upgrade later with `uv tool upgrade astrobib`, `pipx upgrade astrobib`, or `pip install -U astrobib` respectively.

---
## Quick start
```bash
# Download the UAT concept hierarchy (one-time, ~2 MB)
astrobib uat update

# Launch the TUI
astrobib

# Set your ADS API token (https://ui.adsabs.harvard.edu/user/settings/token)
astrobib token
```
---
## TUI key bindings
### Core actions (shown in footer)
- `i` — Import highlighted/selected papers to library (ADS tabs)
- `d` — Remove highlighted/selected papers from library
- `p` — Download PDFs for highlighted/selected papers
- `o` — Open cached PDF (all selected, or highlighted)
- `/` — Library: filter with query syntax (see [Filtering the library](#filtering-the-library)). ADS tab: view/edit the tab's query
- `S` — Open new ADS search tab
- `r` — Refresh current ADS tab
- `?` — Show this help
- `q` — Quit

Footer actions keep fixed positions; a greyed-out action is unavailable in the current context (wrong tab, no PDF cached, already imported, …). When rows are check-selected with `Space`, actions apply to the selection rather than the cursor row; single-paper actions (`R`, `c`, `B`) are dimmed while more than one row is selected.

### More keys
- `Space` — Toggle selection of highlighted row
- `y` — Copy highlighted/selected cite key(s), shortest unambiguous form
- `Y` — Copy highlighted/selected cite key(s), full form with hash
- `s` — Star / unstar highlighted/selected papers
- `m` — Add/remove highlighted or selected papers in the manuscript db
- `M` — Toggle manuscript-only view (hide personal-library-only papers)
- `R` — Browse references of the highlighted or single-selected paper (opens ADS tab)
- `c` — Browse citations of the highlighted or single-selected paper (opens ADS tab)
- `e` — Export selected papers to `astrobib-export.bib`
- `B` — Download PDF via system browser (watches `~/Downloads`)
- `X` — Clear cached PDFs for highlighted/selected papers (or cancel a browser download)
- `u` — Open UAT concept browser
- `C` — Configuration (ADS API token)
- `[` / `]` — Switch to previous / next tab
- `Ctrl+W` — Close current ADS tab
- `+` / `-` — Increase / decrease ADS result count (then `r` to reload)
- `Escape` — Clear filter
- `z` — Cycle pub card width (shows it if hidden)
- `D` — Show/hide the pub card

### Copying text from the TUI
Terminal applications such as astrobib enable *mouse reporting*, so drag-selection is captured by the application rather than the terminal, which is why ⌘C often copies nothing. There are two workarounds:

- Press `y` to copy the highlighted (or Space-selected) cite keys directly to the system clipboard. This is the intended workflow for placing keys in a `.tex` file.
- For arbitrary text, hold **⌥ Option** (macOS Terminal/iTerm2; **Shift** on most Linux terminals) while dragging: this bypasses mouse reporting and restores native selection, after which ⌘C works normally.
---
## Filtering the library
Press `/` on the Library tab to filter the list as you type. The filter is a local query language modeled on ADS syntax, evaluated live against your library. Whitespace-separated terms all AND together, and each term is a case-insensitive partial match. Bare terms match across author, title, abstract, cite key, keywords, and year; prefix a field name to narrow the match. Examples:
```
sironi shock                        bare terms — match author, title, abstract, key, keywords, year
author:sironi                       author anywhere in the author list
author:^zrake                       first-author papers only (ADS ^ convention)
title:magnetar                      word in title
abs:"fast radio burst"              phrase in abstract
key:Zrake2020                       cite key
kw:"compact objects"                keyword
year:2020                           single year
year:2015-2020                      year range
year:2020-                          2020 or later (year:-2015 for 2015 or earlier)
"quoted phrase"                     exact phrase, any field
-abs:neutrino  -is:ms               a leading - negates any term
is:starred                          starred papers
is:ms                               manuscript-db members
is:pdf                              papers with a cached PDF

author:^zrake year:2019- is:pdf     terms combine with implicit AND
```
A partially typed query never produces an error: unknown fields are treated as bare text, so the list refines smoothly as you type.

With a filter active, pressing `S` opens an ADS search tab pre-filled with the equivalent ADS query; local-only terms (`is:`, `key:`, and negations) are dropped. This allows you to filter locally and escalate the same query to ADS in one keystroke.

---
## ADS query syntax

The search box (`S`) passes your query straight to the [ADS search API](https://ui.adsabs.harvard.edu/help/search/search-syntax), so the full ADS/Solr query language is available. Pasting an ADS abstract URL imports that paper directly instead of searching. Examples:
```
relativistic jets                              plain text — searches everything
author:"zrake"                                 papers by an author
author:"^zrake"                                first-author papers only
author:"spitkovsky" author:"sironi"            both authors on the same paper
abs:"fast radio burst"                         phrase in abstract, title, or keywords
title:"magnetar"                               word in title only
year:2024                                      single year
year:2015-2020                                 year range
bibstem:ApJL                                   one journal (ApJ, MNRAS, PRL, arXiv, …)
arxiv_class:astro-ph.HE                        arXiv category
object:"SN 2023ixf"                            papers about a named object (via SIMBAD)
citation_count:[100 TO *]                      highly cited papers
property:refereed                              refereed only

author:"^zrake" year:2019-2024 bibstem:ApJ     terms combine with implicit AND
abs:"kilonova" NOT abs:"neutrino"              exclude a term
(abs:"jet" OR abs:"outflow") year:2023         boolean grouping

references(bibcode:"2020ApJ...900...12S")      papers this one cites (or press R)
citations(bibcode:"2020ApJ...900...12S")       papers citing this one (or press c)
citations(author:"^zrake")                     everything citing your first-author papers
similar(bibcode:"2020ApJ...900...12S")         textually similar papers
trending(abs:"gravitational waves")            what readers of this topic read now
useful(abs:"pulsar timing")                    methods/tools papers cited by this field
```
---
## Cite keys
astrobib's key policy separates what the **databases** store from what a **manuscript** types, so keys can be collision-proof in one place and clean in the other.

**Database keys are content-derived.** Every stored entry is keyed `AuthorYYYYhhhhh`: the first author's surname, the year, and five hash characters computed from the paper's arXiv ID (or, failing that, its ADS bibcode), e.g. `Zrake2020axbxt`. The key depends only on the paper's identity, so:

- the same paper receives the same key regardless of who imports it, or when, so personal libraries and manuscript databases merge without coordination
- two different Smith 2020 papers can never collide
- re-importing a paper is detected as a duplicate rather than creating a second entry
- the key remains stable across the arXiv-to-journal transition (the hash is computed from the arXiv ID when one exists)

Database `.bib` files are always stored under their full key, one file per paper.

**Manuscripts cite by any unambiguous prefix.** In your `.tex` you may write the full key or any prefix that matches exactly one database key; in practice, `\citep{Zrake2020}`. The generated `refs.bib` keys each entry by the string actually cited, so BibTeX sees exactly what the manuscript says and the hash suffix never appears in your prose. (Classic BibTeX has no key-alias mechanism, so this aliasing happens at the `refs.bib` boundary, which astrobib owns.)

**Ambiguity is detected, not guessed.** If a prefix matches several keys (for example, after a second Zrake 2020 paper is imported), no candidate is chosen silently: the Manuscript tab shows the cite as magenta `≈ ambiguous` with the candidates listed, and `astrobib refs` prints them and exits nonzero. Lengthening the key by a character or two resolves the ambiguity.

**Displayed keys are the shortest unambiguous form.** The TUI and CLI show short keys wherever possible, and `astrobib import` emits its cite-key replacement commands using short keys, so hash characters appear only when they are needed to disambiguate.

---
## Manuscript databases

A manuscript can carry its own bib database: a `bib/` directory inside the manuscript's git repository, holding one `.bib` file per cited paper. There is no registration step: launching astrobib from inside the repository (any directory with `bib/` alongside `.git`) activates it, indicated by `ms: <name>` in the header. Coauthors who clone the repository get the same database automatically; coauthors without astrobib use the committed `refs.bib`.

While a manuscript db is active:
- The library view merges your personal library with the manuscript db; the `◆` column marks manuscript members (`M` hides everything else)
- Importing from ADS (`i`) writes to **both** the personal library and the manuscript db
- `m` toggles manuscript membership for existing library entries

astrobib never runs git on the manuscript repository; bib files are committed as part of your normal work on the paper.

Removal from the manuscript db is never destructive: if it holds the only copy of an entry (imported `--ms-only`, or added by a coauthor), removing it via `m` or `refs --prune` first copies it into your personal library.

### The Manuscript tab
A **Manuscript** tab appears next to Library, showing the union of cite keys found in the `.tex` sources and entries in `bib/`, color-coded:

- `◆` normal — cited and in `bib/`: healthy
- `○` yellow — cited, in your personal library but not `bib/`: press `m` to add
- `✗` red — cited but found nowhere: fix the key, or `S` to search ADS (pre-filled)
- `≈` magenta — cite key is an ambiguous prefix of several entries: lengthen it
- `·` cyan — in `bib/` but cited by nothing: press `m` to remove

Cite keys in the `.tex` may be any unambiguous prefix of a database key (`\citep{Zrake2020}`), and `refs.bib` is keyed by the cited string; see [Cite keys](#cite-keys).

If `main.tex` exists, it is the sole root document and other top-level `.tex` files (old drafts, notes) are ignored; otherwise every top-level `.tex` file is a root. Roots are expanded recursively through `\input`/`\include`, so multi-file papers are fully scanned.

The tab watches the `.tex` sources and `bib/` (2 s poll) and refreshes itself as you write. `refs.bib` is regenerated automatically from the cited entries in `bib/` whenever its content would change. Nothing is copied or removed automatically; membership changes always go through `m` (or `astrobib refs` / `--prune` on the command line). The status bar summarizes health: `42 cited · 2 missing · 5 uncited`.

Keep the database in sync with what the paper actually cites:
```bash
cd ~/Work/Papers/my-paper
astrobib refs             # scan .tex, pull cited entries in from personal
                          # library, report unknowns, write refs.bib
astrobib refs --prune     # also drop entries nothing cites anymore
```
---
## CLI reference
### Adding papers
```bash
# Search ADS
astrobib search --ads "magnetohydrodynamical simulations"

# Add by ADS bibcode
astrobib add 2020ApJ...900...12S

# Add with extra keywords
astrobib add 2020ApJ...900...12S --keywords "Magnetohydrodynamical simulations"
```

### Sharing papers
Export selected papers from the TUI (`Space` to select, `e` to export) and share the resulting `astrobib-export.bib` file. The recipient can import it:
```bash
astrobib import shared-papers.bib
```
Because keys are content-derived (see [Cite keys](#cite-keys)), the same paper always gets the same key regardless of who added it, so shared files merge cleanly and duplicates are detected on import.

### Importing a foreign .bib file
`astrobib import` accepts any `.bib` file, for example the bibliography of another paper with arbitrary cite keys. Every entry is resolved against ADS (by arXiv ID, DOI, or exact title + first author + year) and imported with canonical ADS BibTeX and a regenerated astrobib cite key. Entries whose key already matches their content-derived astrobib key (i.e. bibdata from an astrobib export) are recognized automatically and imported directly, with no ADS round-trip. Entries that cannot be resolved to exactly one ADS record are skipped with a warning. Entries already present are kept as-is (pass `--verify` to be prompted to replace them). After importing, astrobib prints copy-pasteable `perl -pi -e` commands that rewrite the old cite keys to the new ones in your `.tex` files.

Inside a manuscript repo, `add` and `import` write to both the personal library and the manuscript database (matching the TUI); use a flag to restrict:
```bash
astrobib import other-paper.bib                  # personal + manuscript db
astrobib import --personal-only other-paper.bib  # personal library only
astrobib import --ms-only other-paper.bib        # manuscript db only
```
CLI read commands (`list`, `show`, `search`, `export`, `pdf`) see the same merged personal + manuscript view as the TUI, with the same indicators: `↓` PDF cached, `◆` in manuscript db, `★` starred.

### Generating refs.bib for a manuscript
Run this from inside the manuscript directory:
```bash
astrobib export                  # scans all .tex files in cwd
astrobib export paper.tex        # explicit file
astrobib export -o refs.bib      # explicit output path
```

The tool scans for `\cite`, `\citep`, `\citet`, and related commands, looks each key up in the library, and writes a `refs.bib` containing only the entries that are actually cited.

### ADS token
```bash
astrobib token                   # show current token or prompt to enter one
astrobib token <your-token>      # set token directly
astrobib quota                   # check ADS API rate limit usage
```
The token can also be set via the `ADS_API_TOKEN` environment variable.

### UAT commands
```bash
astrobib uat update              # download / refresh UAT cache
astrobib uat search hydrodynamics
astrobib uat show "Hydrodynamics"
astrobib uat browse              # standalone TUI browser
```

### Other commands
```bash
astrobib show <key>              # print BibTeX entry
astrobib open <key>              # open PDF (fetched from arXiv if needed)
astrobib list                    # list all papers
astrobib list --keyword "Compact objects"
astrobib keywords                # list all keywords in the library
```
---
## Library layout
Papers are stored in `~/.local/share/astrobib/library/bib/`, one `.bib` file per paper. The directory is created automatically on first use.

Cite keys have the form `AuthorYYYYhhhhh` where `hhhhh` is the first 5 hex characters of the SHA-256 of the arXiv ID (or ADS bibcode for non-arXiv papers). This makes keys collision-resistant and stable across the arXiv→journal publishing transition.

---
## PDF handling
PDFs are ephemeral and are never stored in the library. When you open a paper (`o` in the TUI or `astrobib open <key>`), astrobib:

1. Checks `~/.cache/astrobib/pdfs/<key>.pdf`
2. If absent, fetches from `https://arxiv.org/pdf/<eprint>`
3. Caches and opens in the system PDF viewer

The cache can be deleted freely; everything is re-fetchable.
