# litbot design principles

Read this before adding features or changing data formats.

## Future-proof by dumbness

The highest-priority design constraint is that data formats stay dumb enough that they never need a migration. Formats should be passively forward- and backward-compatible, not actively versioned. Dumb formats degrade gracefully across tool versions, editors, and git clients, and never need migration scripts.

Concretely:

- The bib database is a git repository containing BibTeX files in `bib/`. That is the entire format. No other files belong there.
- BibTeX fields in `.bib` files come from ADS. Do not embed litbot-specific semantics in `keywords` or any other BibTeX field. A `.bib` file written by litbot must be indistinguishable from one written by hand from the ADS website.
- The config file (`~/.config/litbot/config.toml`) uses only standard TOML. Unknown keys are silently ignored. New keys always have defaults. Keys are never renamed without a migration shim that stays in the codebase.
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
- A place for anything litbot-specific beyond the BibTeX entries themselves

Features like "mark as read", "add a note", "reading list" are personal and social, not bibliographic. They belong in personal tools outside the shared database, not in litbot.

## Persistent searches are user-local

Saved ADS query tabs live in user-local app state (e.g., `~/.local/share/litbot/`), not in the bib database. They are not synced to other group members. Each user maintains their own set of active searches.

## Adding to the bib database format

The only acceptable additions to the bib database layout are:

- More `.bib` files in `bib/`

Any new directory or file added to the bib repo must be safely ignored by all litbot versions that predate it. If this cannot be guaranteed, the addition is wrong.

## Config and app state versioning

Both `config.toml` and any future app state files must include a `schema_version` integer. Version changes follow this rule:

- Additive changes (new optional keys with defaults): bump schema_version, no migration needed
- Destructive changes (renames, removals): write an explicit migration function in `config.py` that runs on load when an old version is detected

## Dev vs. stable coexistence

When running a dev install alongside the system-installed version, use `LITBOT_STATE_DIR` to redirect user-local app state to a scratch path. The bib database and config are shared between versions and must remain compatible.
