# astrobib design principles (draft revision)

> Draft produced by the unified-model design study (docs/unified-model-study.md).
> It retains the two-database model and revises DESIGN.md to (1) state that
> model and its invariants explicitly, (2) remove stale text from the retired
> multi-database design (`db push` / `db pull`, group-visible framing), and
> (3) make implementation references neutral between the Python reference
> (v0.4.0) and the official Rust implementation. It does not change any
> behavior or format.

Read this before adding features or changing data formats.

## Future-proof by dumbness

The highest-priority design constraint is that data formats stay dumb enough that they never need a migration. Formats should be passively forward- and backward-compatible, not actively versioned. Dumb formats degrade gracefully across tool versions, editors, and git clients, and never need migration scripts.

Concretely:

- A bib database is a directory of BibTeX files in `bib/`. That is the entire format. No other files belong there.
- BibTeX fields in `.bib` files come from ADS. Do not embed astrobib-specific semantics in `keywords` or any other BibTeX field. A `.bib` file written by astrobib must be indistinguishable from one written by hand from the ADS website. (The legacy `astrobib_starred` field written by pre-v0.4.0 Python versions violated this; it is ignored on read and stripped from manuscript copies, and nothing like it may be added again.)
- Configuration and app state use only standard formats (TOML, JSON). Unknown keys are silently ignored. New keys always have defaults. Keys are never renamed without a migration shim that stays in the codebase.
- Persistent app state (saved searches, tokens, caches) is user-local, never stored in or synced via any bib database.

## The two databases

astrobib maintains exactly two kinds of bib database, both in the identical dumb format — flat `.bib` files, one per paper, in `bib/`:

- **The personal library** (`~/.local/share/astrobib/library/`, relocatable via `--library` / `ASTROBIB_LIBRARY`; root via `ASTROBIB_STATE_DIR`). The user's accumulated collection, sized for ~1e4 entries. It grows as a side effect of writing: every import lands here.
- **A manuscript database**: a `bib/` directory inside a manuscript's git repo, so the repo stands alone for coauthors. At most one is active per session.

A bib database is a collection of BibTeX entries and nothing else. It is not:

- A place for personal annotations, notes, or reading status
- A place for per-user metadata
- A place for app configuration or UI state
- A place for anything astrobib-specific beyond the BibTeX entries themselves

Features like "mark as read", "add a note", "reading list" are personal and social, not bibliographic. They belong in personal tools outside the databases, not in astrobib.

## The merged view and its invariants

When a manuscript database is active, reads span both databases and writes follow fixed rules. These invariants are load-bearing; every feature that touches entries must position itself against them rather than invent a variant:

- **Merged reads, personal wins.** The library view is every personal entry plus manuscript-only entries. On a key collision the personal copy wins. (Collisions can differ only in field freshness, never in identity — see the key policy below.)
- **Imports write to both.** An import while a manuscript is active lands in the personal library *and* the manuscript database, so the repo stands alone and the collection still accrues.
- **Membership is explicit.** Copying an existing library entry into (or out of) the manuscript database is always a deliberate act (`m`, the card chip, or `refs`), never a side effect. Nothing is auto-copied or auto-pruned.
- **Removal from a manuscript database is never destructive.** If the manuscript holds the only copy of an entry (e.g. added by a coauthor), removing it first copies it into the personal library. Removal demotes; it never destroys bibdata.
- **Membership is queryable, not stored.** `is:ms` and the `●` indicator are computed from presence in the active manuscript database. Membership must never be recorded as data in either store.

## Manuscript databases

A manuscript database uses exactly the standard database format and is therefore indistinguishable from any other bib database. Rules:

- Discovery is by directory walk-up (cwd ancestor containing `bib/` and `.git`), never by registration in config. The active personal library root is excluded from the walk-up. At most one manuscript database is active per session.
- astrobib never runs git on a manuscript repo. Versioning rides along in the user's own paper commits.
- Copies, not links: an entry added to a manuscript database is a self-contained copy of the `.bib` file, so the repo stands alone for coauthors. Identical content yields identical keys, so copies agree across databases without coordination.
- Legacy personal fields (`astrobib_starred`) are stripped from manuscript copies. The manuscript database is shared with coauthors; nothing personal enters it.
- The sync flow (`astrobib refs`) may add cited entries and, only with an explicit flag, remove uncited ones. It never removes anything from the personal library.

## Keys denote papers, not revisions

A cite key identifies a paper for life: both the hash and the year in `AuthorYYYYhhhhh` derive from the paper's stable identifier (arXiv ID, else bibcode), never from mutable record state, so every user holding any phase of the paper (preprint or published) generates the same key. This is what makes the two-database model safe: copies of a paper made at different times by different people always agree on identity, so the databases can differ only in presence, never in meaning. `astrobib update` refreshes metadata beneath an existing key and never rekeys. Citing a specific arXiv revision (v1, v2) is out of scope for keys; that rare need is served by a hand-written `@misc` entry with a versioned eprint field.

## Persistent searches are user-local

Saved ADS query tabs live in user-local app state (`tabs.json` under the state dir), not in any bib database. Tabs are keyed by context: each manuscript database (by its root path) has its own tab set, and sessions with no active manuscript share a global set. The storage location stays user-local either way — per-manuscript tabs are never written into the manuscript repo.

## Adding to the bib database format

The only acceptable addition to the bib database layout is:

- More `.bib` files in `bib/`

Any new directory or file added to a bib database must be safely ignored by all astrobib versions that predate it. If this cannot be guaranteed, the addition is wrong.

## Config and app state versioning

Structured config or state files include a `schema_version` integer. Version changes follow this rule:

- Additive changes (new optional keys with defaults): bump schema_version, no migration needed
- Destructive changes (renames, removals): write an explicit migration function that runs on load when an old version is detected, and keep it in the codebase

Files shared with the v0.4.0 Python implementation (`state.json`, `tabs.json`, the bib databases themselves) must remain readable and writable by it unchanged.

## Dev vs. stable coexistence

When running a dev build alongside an installed version, use `ASTROBIB_STATE_DIR` to redirect user-local app state to a scratch path. The bib databases and config are shared between versions and must remain compatible.
