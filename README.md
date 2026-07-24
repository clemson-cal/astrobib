# litbot

Personal astrophysics literature manager. Connects to the
[NASA/Harvard ADS](https://ui.adsabs.harvard.edu) to search and fetch papers,
stores BibTeX in `~/.local/share/litbot/library/`, and generates `refs.bib`
files for LaTeX manuscripts by scanning for cite keys.

Keywords follow the
[Unified Astronomy Thesaurus (UAT)](https://astrothesaurus.org), the
controlled vocabulary used by AAS journals.

---

## Quick start

```bash
# Download the UAT concept hierarchy (one-time, ~2 MB)
litbot uat update

# Launch the TUI
litbot

# Set your ADS API token (https://ui.adsabs.harvard.edu/user/settings/token)
litbot token
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
| `/` | Filter library by author, title, key, or keyword |
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
| `e` | Export selected papers to `litbot-export.bib` |
| `B` | Download PDF via system browser (watches `~/Downloads`) |
| `X` | Clear cached PDF (or cancel a browser download) |
| `u` | Open UAT concept browser |
| `C` | Configuration (ADS API token) |
| `[` / `]` | Switch to previous / next tab |
| `Ctrl+W` | Close current ADS tab |
| `←` / `→` | Decrease / increase ADS result count (then `r` to reload) |
| `Escape` | Clear filter |
| `z` | Zoom detail panel |

---

## CLI reference

### Adding papers

```bash
# Search ADS
litbot search --ads "magnetohydrodynamical simulations"

# Add by ADS bibcode
litbot add 2020ApJ...900...12S

# Add with extra keywords
litbot add 2020ApJ...900...12S --keywords "Magnetohydrodynamical simulations"
```

### Sharing papers

Export selected papers from the TUI (`Space` to select, `e` to export) and
share the resulting `litbot-export.bib` file. The recipient can import it:

```bash
litbot import shared-papers.bib
```

Collision-resistant cite keys (`AuthorYYYYhhhh`) are deterministic from the
arXiv ID, so the same paper always gets the same key regardless of who added it.

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

### ADS token

```bash
litbot token                   # show current token or prompt to enter one
litbot token <your-token>      # set token directly
litbot quota                   # check ADS API rate limit usage
```

The token can also be set via the `ADS_API_TOKEN` environment variable.

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

## Library layout

Papers are stored in `~/.local/share/litbot/library/bib/`, one `.bib` file
per paper. The directory is created automatically on first use.

Cite keys have the form `AuthorYYYYhhhh` where `hhhh` is the first 5 hex
characters of the SHA-256 of the arXiv ID (or ADS bibcode for non-arXiv
papers). This makes keys collision-resistant and stable across the
arXiv→journal publishing transition.

---

## PDF handling

PDFs are ephemeral — never stored in the library. When you open a paper
(`o` in the TUI or `litbot open <key>`), litbot:

1. Checks `~/.cache/litbot/pdfs/<key>.pdf`
2. If absent, fetches from `https://arxiv.org/pdf/<eprint>`
3. Caches and opens in the system PDF viewer

The cache can be deleted freely; everything is re-fetchable.
