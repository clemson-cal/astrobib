# litbot

Astrophysics literature manager for research groups. Connects to the
[NASA/Harvard ADS](https://ui.adsabs.harvard.edu) to fetch papers, stores
BibTeX in a shared git repository, and generates `refs.bib` files for
LaTeX manuscripts by scanning for cite keys.

Keywords follow the
[Unified Astronomy Thesaurus (UAT)](https://astrothesaurus.org), the
controlled vocabulary used by AAS journals.

---

## Quick start

```bash
pip install git+https://github.com/your-group/litbot

# Download the UAT concept hierarchy (one-time, ~2 MB)
litbot uat update

# Clone your group's bib database
litbot db clone https://github.com/your-group/bib-database

# Launch the TUI
litbot
```

---

## TUI key bindings

| Key | Action |
|-----|--------|
| `u` | Toggle UAT concept browser in the left panel |
| `a` | Add a paper from ADS by bibcode |
| `o` | Open PDF (fetched from arXiv and cached locally) |
| `/` | Search the library |
| `Escape` | Show all papers / clear search |
| `t` | Focus the left panel |
| `?` | Show this help |
| `q` | Quit |

---

## CLI reference

### Managing databases

```bash
# Clone an existing group database
litbot db clone https://github.com/group/bib-database

# Create a new empty database
litbot db init /path/to/new-database

# List configured databases
litbot db list

# Sync
litbot db pull                        # pull default database
litbot db push                        # push committed changes
litbot db publish -m "Add Smith 2020" # stage, commit, and push
litbot db pull --db collab            # pull a named database
```

Multiple databases are merged transparently for browsing, search, and
export. Writes go to the default database unless `--db <name>` is given.

### Adding papers

```bash
# Search ADS
litbot search "magnetohydrodynamical simulations" --ads

# Add by ADS bibcode (UAT keywords imported automatically; auto-committed)
litbot add 2020ApJ...900...12S

# Add with extra keywords
litbot add 2020ApJ...900...12S --keywords "Magnetohydrodynamical simulations"

# Write to a specific database
litbot add 2020ApJ...900...12S --db personal
```

`litbot add` saves the `.bib` file and automatically commits it. Run
`litbot db push` when you're ready to share.

### Generating refs.bib for a manuscript

Run this from inside the manuscript directory:

```bash
litbot export                  # scans all .tex files in cwd
litbot export paper.tex        # explicit file
litbot export -o refs.bib      # explicit output path
```

The tool scans for `\cite`, `\citep`, `\citet`, and related commands,
looks each key up in the library, and writes a `refs.bib` containing only
the entries that are actually cited.

### UAT commands

```bash
litbot uat update              # download / refresh UAT cache
litbot uat search hydrodynamics
litbot uat show "Hydrodynamics"
litbot uat browse              # standalone TUI browser
```

### Other commands

```bash
litbot show <key>              # print BibTeX entry
litbot open <key>              # open PDF (fetched from arXiv if needed)
litbot list                    # list all papers
litbot list --keyword "Compact objects"
litbot keywords                # list all keywords in the library
```

---

## Bib database layout

A bib database is a plain git repository:

```
my-bib/
└── bib/
    ├── smith2020_merger.bib
    └── jones2021_mhd.bib
```

Each `.bib` file holds one entry. UAT keywords are stored in the standard
`keywords` BibTeX field (populated automatically from ADS). Papers can
carry multiple keywords.

The TUI's keyword tree is built dynamically from the keywords actually
present in your bib entries, grouped by their top-level UAT concept.

---

## PDF handling

PDFs are ephemeral — never stored in the bib database. When you open a
paper (`o` in the TUI or `litbot open <key>`), litbot:

1. Checks `~/.cache/litbot/pdfs/<key>.pdf`
2. If absent, fetches from `https://arxiv.org/pdf/<eprint>`
3. Caches and opens in the system PDF viewer

The cache can be deleted freely; everything is re-fetchable.

---

## Configuration

`~/.config/litbot/config.toml`:

```toml
ads_token = "your-ADS-token"       # https://ui.adsabs.harvard.edu/user/settings/token
default_database = "cal"

[databases.cal]
path = "~/.local/share/litbot/databases/cal-library"

[databases.personal]
path = "~/personal-bib"
```

The ADS token can also be set via the `ADS_API_TOKEN` environment variable.
