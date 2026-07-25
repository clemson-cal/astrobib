# astrobib

Personal astrophysics literature manager. Connects to the
[NASA/Harvard ADS](https://ui.adsabs.harvard.edu) to search and fetch papers,
stores BibTeX in `~/.local/share/astrobib/library/`, and generates `refs.bib`
files for LaTeX manuscripts by scanning for cite keys.

Keywords follow the
[Unified Astronomy Thesaurus (UAT)](https://astrothesaurus.org), the
controlled vocabulary used by AAS journals.

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

| Key | Action |
|-----|--------|
| `i` | Import highlighted/selected papers to library (ADS tabs) |
| `d` | Remove highlighted paper from library |
| `p` | Download PDF for highlighted paper |
| `o` | Open cached PDF (all selected, or highlighted) |
| `/` | Library: filter by author, title, key, or keyword. ADS tab: view/edit the tab's query |
| `S` | Open new ADS search tab |
| `r` | Refresh current ADS tab |
| `?` | Show this help |
| `q` | Quit |

Footer actions keep fixed positions; a greyed-out action is unavailable
in the current context (wrong tab, no PDF cached, already imported, …).

### More keys

| Key | Action |
|-----|--------|
| `Space` | Toggle selection of highlighted row |
| `s` | Star / unstar highlighted paper |
| `m` | Add/remove highlighted or selected papers in the manuscript db |
| `M` | Toggle manuscript-only view (hide personal-library-only papers) |
| `R` | Browse references of highlighted paper (opens ADS tab) |
| `c` | Browse citations of highlighted paper (opens ADS tab) |
| `e` | Export selected papers to `astrobib-export.bib` |
| `B` | Download PDF via system browser (watches `~/Downloads`) |
| `X` | Clear cached PDF (or cancel a browser download) |
| `u` | Open UAT concept browser |
| `C` | Configuration (ADS API token) |
| `[` / `]` | Switch to previous / next tab |
| `Ctrl+W` | Close current ADS tab |
| `+` / `-` | Increase / decrease ADS result count (then `r` to reload) |
| `Escape` | Clear filter |
| `z` | Zoom detail panel |

---

## ADS query syntax

The search box (`S`) passes your query straight to the
[ADS search API](https://ui.adsabs.harvard.edu/help/search/search-syntax),
so the full ADS/Solr query language is available. Pasting an ADS abstract
URL imports that paper directly instead of searching. Examples:

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

## Manuscript databases

A manuscript can carry its own bib database: a `bib/` directory inside the
manuscript's git repo, holding one `.bib` file per cited paper. There is no
registration step — launching astrobib from inside the repo (any directory
with `bib/` alongside `.git`) activates it, shown as `ms: <name>` in the
header. Coauthors who clone the repo get the same database automatically;
coauthors without astrobib just use the committed `refs.bib`.

While a manuscript db is active:

- The library view merges your personal library with the manuscript db;
  the `◆` column marks manuscript members (`M` hides everything else)
- Importing from ADS (`i`) writes to **both** the personal library and
  the manuscript db
- `m` toggles manuscript membership for existing library entries

astrobib never runs git on the manuscript repo — bib files ride along in
your normal paper commits.

### The Manuscript tab

A **Manuscript** tab appears next to Library, showing the union of cite
keys found in the `.tex` sources and entries in `bib/`, color-coded:

| State | Meaning |
|-------|---------|
| normal `◆` | cited and in `bib/` — healthy |
| yellow `○` | cited, in your personal library but not `bib/` — press `m` to add |
| red `✗` | cited but found nowhere — fix the key, or `S` to search ADS (pre-filled) |
| magenta `≈` | cite key is an ambiguous prefix of several entries — lengthen it |
| cyan `·` | in `bib/` but cited by nothing — press `m` to remove |

### Citing by short key

Cite keys in the `.tex` may be **any unambiguous prefix** of a database
key: `\citep{Zrake2020}` finds `Zrake2020axbxt` as long as no other key
starts with `Zrake2020`. The generated `refs.bib` keys each entry by the
string actually cited, so the hash suffixes that protect the shared
databases never appear in the manuscript. If a prefix later becomes
ambiguous (a second Zrake-2020 paper arrives), the Manuscript tab and
`astrobib refs` flag it and list the candidates — lengthen the key by a
character or two.

If `main.tex` exists it is the sole root document — other top-level
`.tex` files (old drafts, notes) are ignored; otherwise every top-level
`.tex` file is a root. Roots are expanded recursively through
`\input`/`\include`, so multi-file papers are fully scanned.

The tab watches the `.tex` sources and `bib/` (2 s poll) and refreshes
itself as you write. `refs.bib` is regenerated automatically from the
cited entries in `bib/` whenever its content would change. Nothing is
copied or removed automatically — membership changes always go through
`m` (or `astrobib refs` / `--prune` on the command line). The status bar
summarizes health: `42 cited · 2 missing · 5 uncited`.

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

Export selected papers from the TUI (`Space` to select, `e` to export) and
share the resulting `astrobib-export.bib` file. The recipient can import it:

```bash
astrobib import shared-papers.bib
```

Collision-resistant cite keys (`AuthorYYYYhhhh`) are deterministic from the
arXiv ID, so the same paper always gets the same key regardless of who added it.

### Importing a foreign .bib file

`astrobib import` accepts any `.bib` file — e.g. the bibliography of another
paper with arbitrary cite keys. Every entry is resolved against ADS (by
arXiv ID, DOI, or exact title + first author + year) and imported with
canonical ADS BibTeX and a regenerated astrobib cite key. Entries whose
key already matches their content-derived astrobib key (i.e. bibdata
from an astrobib export) are recognized automatically and imported
directly, with no ADS round-trip. Entries that
cannot be resolved to exactly one ADS record are skipped with a warning.
Entries already present are kept as-is (pass `--verify` to be prompted
to replace them).
After importing, astrobib prints copy-pasteable `perl -pi -e` commands that
rewrite the old cite keys to the new ones in your `.tex` files.

Inside a manuscript repo, `add` and `import` write to both the personal
library and the manuscript database (matching the TUI); use a flag to
restrict:

```bash
astrobib import other-paper.bib                  # personal + manuscript db
astrobib import --personal-only other-paper.bib  # personal library only
astrobib import --ms-only other-paper.bib        # manuscript db only
```

CLI read commands (`list`, `show`, `search`, `export`, `pdf`) see the
same merged personal + manuscript view as the TUI, with the same
indicators: `↓` PDF cached, `◆` in manuscript db, `★` starred.

### Generating refs.bib for a manuscript

Run this from inside the manuscript directory:

```bash
astrobib export                  # scans all .tex files in cwd
astrobib export paper.tex        # explicit file
astrobib export -o refs.bib      # explicit output path
```

The tool scans for `\cite`, `\citep`, `\citet`, and related commands,
looks each key up in the library, and writes a `refs.bib` containing only
the entries that are actually cited.

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

Papers are stored in `~/.local/share/astrobib/library/bib/`, one `.bib` file
per paper. The directory is created automatically on first use.

Cite keys have the form `AuthorYYYYhhhh` where `hhhh` is the first 5 hex
characters of the SHA-256 of the arXiv ID (or ADS bibcode for non-arXiv
papers). This makes keys collision-resistant and stable across the
arXiv→journal publishing transition.

---

## PDF handling

PDFs are ephemeral — never stored in the library. When you open a paper
(`o` in the TUI or `astrobib open <key>`), astrobib:

1. Checks `~/.cache/astrobib/pdfs/<key>.pdf`
2. If absent, fetches from `https://arxiv.org/pdf/<eprint>`
3. Caches and opens in the system PDF viewer

The cache can be deleted freely; everything is re-fetchable.
