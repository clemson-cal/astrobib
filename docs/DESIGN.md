# astrobib design principles

Read this before adding features or changing data formats.

## Future-proof by dumbness

The highest-priority design constraint is that data formats stay dumb enough that they never need a migration. Formats should be passively forward- and backward-compatible, not actively versioned. Dumb formats degrade gracefully across tool versions, editors, and git clients, and never need migration scripts.

Concretely:

- The bib database is a git repository containing BibTeX files in `bib/`. That is the entire format. No other files belong there.
- BibTeX fields in `.bib` files come from ADS. Do not embed astrobib-specific semantics in `keywords` or any other BibTeX field. A `.bib` file written by astrobib must be indistinguishable from one written by hand from the ADS website.
- The config file (`~/.config/astrobib/config.toml`) uses only standard TOML. Unknown keys are silently ignored. New keys always have defaults. Keys are never renamed without a migration shim that stays in the codebase.
- Any future persistent app state (e.g., saved searches) is user-local, never stored in or synced via the bib database.

## What the database is and is not

The bib database is a shared, group-visible record of BibTeX entries. It is:

- A flat collection of `.bib` files, one per paper, in `bib/`
- Versioned by git
- Synced with `db push` / `db pull`

It is not:

- A place for personal annotations, notes, or reading status
- A place for per-user or per-student metadata
- A place for app configuration or UI state
- A place for anything astrobib-specific beyond the BibTeX entries themselves

Features like "mark as read", "add a note", "reading list" are personal and social, not bibliographic. They belong in personal tools outside the shared database, not in astrobib.

## Manuscript databases

A manuscript database is a `bib/` directory inside a manuscript's git repo. It uses exactly the standard database format — flat `.bib` files, nothing else — and is therefore indistinguishable from any other bib database. Rules:

- Discovery is by directory walk-up (nearest cwd ancestor containing `bib/`, stopping at `$HOME`), never by registration in config. At most one manuscript database is active per session.
- astrobib never runs git on a manuscript repo. Versioning rides along in the user's own paper commits.
- Copies, not links: an entry added to a manuscript database is a self-contained copy of the `.bib` file, so the repo stands alone for coauthors. Identical content yields identical keys, so copies agree across databases.
- Personal fields (`astrobib_starred`) are stripped from manuscript copies. The manuscript database is shared; stars are personal.
- The sync flow (`astrobib refs`) may add cited entries and, only with an explicit flag, remove uncited ones. It never removes anything from the personal library.
- Co-authors without astrobib are first-class: hand-dropped `.bib` files under any filename/key are legitimate manuscript-db content, and `astrobib tidy` later canonicalizes them (re-key, rename, dedupe) without changing what they cite.
- Removal from a manuscript database is never destructive: if the manuscript holds the only copy of an entry (e.g. imported `--ms-only`, or added by a coauthor), removing it first copies it into the personal library.

Manuscript sources are `.tex` files (cites via `\cite*{…}`, expanded through `\input`/`\include`) and `.md` files, with the same root policy for each (`main.tex`/`main.md` when present, else every top-level file of that extension). Markdown citations are pandoc-style — bare `@Key` or bracketed `[@A; @B]` — plus Obsidian wikilinks `[[Key]]` (alias `|` and heading `#` suffixes tolerated), which count as citations only when they resolve in the library: an unresolved wikilink is an ordinary note link, while an unresolved `@cite` surfaces as missing. Obsidian embeds `![[file]]` expand as sources, the `\input` analogue. Code blocks, inline code, and HTML comments never scan.

The rendered markdown bibliography (`astrobib refs`) lives between `<!-- astrobib:references -->` and `<!-- /astrobib:references -->` markers, regenerated wholesale on each run (appended as a `## References` section when absent); everything outside the markers belongs to the user.

## Keys denote papers, not revisions

A cite key identifies a paper for life: both the hash and the year in `AuthorYYYYhhhhh` derive from the paper's stable identifier (arXiv ID, else bibcode), never from mutable record state, so every user holding any phase of the paper (preprint or published) generates the same key. `astrobib update` refreshes metadata beneath an existing key and never rekeys. Citing a specific arXiv revision (v1, v2) is out of scope for keys; that rare need is served by a hand-written `@misc` entry with a versioned eprint field.

## Persistent searches are user-local

Saved ADS query tabs live in user-local app state (e.g., `~/.local/share/astrobib/`), not in the bib database. They are not synced to other group members. Each user maintains their own set of active searches.

Tabs are keyed by context: each manuscript database (by its root path) has its own tab set, and sessions with no active manuscript share a global set. The storage location stays user-local either way — per-manuscript tabs are never written into the manuscript repo.

## Adding to the bib database format

The only acceptable additions to the bib database layout are:

- More `.bib` files in `bib/`

Any new directory or file added to the bib repo must be safely ignored by all astrobib versions that predate it. If this cannot be guaranteed, the addition is wrong.

## Config and app state versioning

Both `config.toml` and any future app state files must include a `schema_version` integer. Version changes follow this rule:

- Additive changes (new optional keys with defaults): bump schema_version, no migration needed
- Destructive changes (renames, removals): write an explicit migration function in `config.py` that runs on load when an old version is detected

## Dev vs. stable coexistence

When running a dev install alongside the system-installed version, use `ASTROBIB_STATE_DIR` to redirect user-local app state to a scratch path. The bib database and config are shared between versions and must remain compatible.
