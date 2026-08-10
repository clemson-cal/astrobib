# astrobib design principles

Read this before adding features or changing data formats.

## Future-proof by dumbness

The highest-priority design constraint is that data formats stay dumb enough that they never need a migration. Formats should be passively forward- and backward-compatible, not actively versioned. Dumb formats degrade gracefully across tool versions, editors, and git clients, and never need migration scripts.

Concretely:

- The bib database is a git repository containing BibTeX files in `bib/`, and optionally a sibling `tags/` directory of plain-text collections. No file that is not a `.bib` file belongs in `bib/`.
- BibTeX fields in `.bib` files come from ADS. Do not embed astrobib-specific semantics in `keywords` or any other BibTeX field. A `.bib` file written by astrobib must be indistinguishable from one written by hand from the ADS website.
- Any config file (e.g. `~/.config/astrobib/config.toml`, when one exists) uses only standard TOML. Unknown keys are silently ignored. New keys always have defaults. Keys are never renamed without a migration shim that stays in the codebase.
- Any future persistent app state (e.g., saved searches) is user-local, never stored in or synced via the bib database. Tags are the one thing a user curates that does belong in the database, and they are not app state: see "Tags are collections of papers" below for the test that separates them.

## What the database is and is not

The bib database is a shared, group-visible record of BibTeX entries. It is:

- A flat collection of `.bib` files, one per paper, in `bib/`
- Optionally a `tags/` directory beside `bib/`: one plain-text file per collection, each line a cite key
- Versioned by git, synced with ordinary git push/pull

It is not:

- A place for personal annotations, notes, or reading status
- A place for per-user or per-student metadata
- A place for app configuration or UI state
- A place for anything astrobib-specific beyond the BibTeX entries and their tags

Features like "mark as read", "add a note", "reading list" are personal and social, not bibliographic. They belong in personal tools outside the shared database, not in astrobib.

The line is topic versus status. "Spiral shock waves in disks" is a claim about the papers: it groups the literature, it is useful to anyone working in the same field, and it belongs in the database. "To read", "skeptical of this one", "assigned to Priya" describe you and your week rather than the papers, and stay in user-local state. A tag you would not want a coauthor to pull is a status tag.

## Manuscript databases

A manuscript database is a `bib/` directory inside a manuscript's git repo, optionally beside a `tags/` directory. It uses exactly the standard database format and is therefore indistinguishable from any other bib database, tags included. Rules:

- Discovery is by directory walk-up (nearest cwd ancestor containing `bib/`, stopping at `$HOME`), never by registration in config. At most one manuscript database is active per session.
- astrobib never runs git on a manuscript repo. Versioning rides along in the user's own paper commits.
- Copies, not links: an entry added to a manuscript database is a self-contained copy of the `.bib` file, so the repo stands alone for coauthors. Identical content yields identical keys, so copies agree across databases.
- The sync flow (`astrobib refs`) may add cited entries and, only with an explicit flag, remove uncited ones. It never removes anything from the personal library, and it never touches `tags/`: a tag file naming a key the sync removed keeps the line, because deleting curated lines to reflect a sync is exactly the destructive behaviour the rule below forbids.
- Co-authors without astrobib are first-class: hand-dropped `.bib` files under any filename/key are legitimate manuscript-db content, and `astrobib tidy` later canonicalizes them (re-key, rename, dedupe) without changing what they cite. Hand-edited tag files are first-class the same way: `tidy` sorts and dedupes them, and never drops a key it cannot resolve.
- Removal from a manuscript database is never destructive: if the manuscript holds the only copy of an entry (e.g. imported `--ms-only`, or added by a coauthor), removing it first copies it into the personal library.

Manuscript sources are `.tex` files (cites via `\cite*{…}`, expanded through `\input`/`\include`) and `.md` files, with the same root policy for each (`main.tex`/`main.md` when present, else every top-level file of that extension). Markdown citations are pandoc-style — bare `@Key` or bracketed `[@A; @B]` — plus Obsidian wikilinks `[[Key]]` (alias `|` and heading `#` suffixes tolerated), which count as citations only when they resolve in the library: an unresolved wikilink is an ordinary note link, while an unresolved `@cite` surfaces as missing. Obsidian embeds `![[file]]` expand as sources, the `\input` analogue. Code blocks, inline code, and HTML comments never scan.

The rendered markdown bibliography (`astrobib refs`) lives between `<!-- astrobib:references -->` and `<!-- /astrobib:references -->` markers, regenerated wholesale on each run (appended as a `## References` section when absent); everything outside the markers belongs to the user.

## Keys denote papers, not revisions

Two formats depend on this, not one: cite keys written into manuscripts, and the cite keys listed in tag files. Weakening it breaks both.

A cite key identifies a paper for life: both the hash and the year in `AuthorYYYYhhhhh` derive from the paper's stable identifier (arXiv ID, else bibcode), never from mutable record state, so every user holding any phase of the paper (preprint or published) generates the same key. `astrobib update` refreshes metadata beneath an existing key and never rekeys. Citing a specific arXiv revision (v1, v2) is out of scope for keys; that rare need is served by a hand-written `@misc` entry with a versioned eprint field.

## Persistent searches are user-local

Saved ADS query tabs live in user-local app state (e.g., `~/.local/share/astrobib/`), not in the bib database. They are not synced to other group members. Each user maintains their own set of active searches.

A query has one of two homes. The global set is visible from every directory; a manuscript database (by its root path) also has its own, which appears when that manuscript is the active one. A session reads both, marks where one group ends and the other begins, and can move a query between them. The storage stays user-local either way — a manuscript's queries are never written into the manuscript repo.

Both were once keyed the same way, which made "which queries do I have?" a question about where you were standing. Discovery is a cwd walk-up on the presence of `bib/`, and that is right for *entries*, because an entry belongs to a paper — but a saved search belongs to you, and an empty `bib/`, which git cannot even record, was enough to take your queries off screen with nothing said. So a query you typed is global by default. The exception is a query that names one paper: `citations(…)` and `references(…)` are made from a card with one keystroke and are spent as soon as they are followed, so they are filed with the manuscript rather than trailing every paper you ever opened through every directory. The rule reads off the query's shape, not off which gesture made it, so one typed at the prompt is filed like one made by `C`.

The home is the context key holding the tab, never a field on the tab: two places to say the same thing is two places to disagree. An id appearing in both sets is read once, the global copy winning — results route to the first scope whose id matches, so a duplicate would leave its twin waiting on a result delivered elsewhere.

A tab carries the query, a name, a result limit, how its results are ordered on screen, and what ADS *returns* — the API `sort` that decides which records come back. The last of those was once left out, on the argument that it is chosen while composing and matters only while the query is being worked. That argument was wrong, and in a way worth recording: it applies just as well to the query text, which nobody proposed discarding. The query, the result count and the selection sort are one configuration — a tab restored with a different one is not the query that was saved, and a query handed to a colleague that arrives configured differently is not the query that was sent.

## Frame-scoped TUI hit targets

Clickable and wheel geometry is transient UI state, not persistent library or app data. The TUI rebuilds a typed hit registry during every draw frame; surfaces that do not draw in the current frame therefore cannot answer later input. The table's solved rectangle is the sole exception: it remains as `last_table_area` because row selection and column navigation use the last drawn layout outside the draw pass. The implementation record is [docs/plans/hit-registry.md](plans/hit-registry.md).

Two orderings are involved and they are never interchangeable. The ADS `sort` parameter decides *which* records arrive; the display sort reorders the ones already in hand. Ranking a feed by citations therefore gives the most cited among the newest n, not the most cited overall. Neither can be written as query syntax — `sort:"entry_date desc"` inside `q` is a Solr error, not a sort — which is why both ride alongside `q` as parameters and are surfaced where the query is composed rather than typed into it.

## Tags are collections of papers

A tag is a named collection — "the spiral-shock references for section 3", "disk instabilities" — and it lives in the database under version control, because a topical grouping is a statement about the literature that coauthors and group members benefit from.

The format is a `tags/` directory beside `bib/`, one file per tag, named for the tag. Each line is one cite key; blank lines and whole-line `#` comments are ignored; lines are kept sorted. Nothing may follow a key on its line — a trailing comment would have to be stripped before the file could be handed to anyone, which is exactly the property the format is built to have. Dotfiles and subdirectories in `tags/` are not tags: `.DS_Store` lands in any directory a Finder window has visited, and a tag named for it is a worse outcome than an editor swap file going unread.

- One file per tag, not one file holding all tags. `bib/` is one file per paper because that is what merges: two people adding papers never conflict. A single `tags.json` would be the one file everybody edits, and a conflicted JSON blob is a merge nobody wants to resolve by hand at the end of a semester. Sorted line-oriented files make merging two tags the operation git already performs on any text file.
- One key per line, so the file *is* the citekey dump. Handing a collection to a colleague is `cat`, with no export path to write and nothing that can fall out of step with the reader.
- A key that no longer resolves is skipped, never deleted. Cite keys denote papers for life, so a dangling line is far more likely to be a paper not yet imported than a mistake. Skipped but counted: astrobib reports how many keys a tag file names that it cannot find, because a line that silently does nothing is indistinguishable from a typo. The count is information, not an error — the file is not wrong, it is ahead of the library.
- A tag is a property of the database, not of the entry. Tags are not BibTeX fields, so copying an entry between tiers moves no tags — by construction rather than by rule, which is the stronger form: there is no code path that could forget.

Both tiers may carry tags, and the merge rule is the opposite of the one entries use:

- **Tags union across tiers; entries shadow.** A paper's entry resolves to the first tier holding it, because two copies are the same record. But a paper tagged `disk-instability` in the library and `section-3` in a manuscript is genuinely both, and shadowing would silently discard one. Two tag files of the same name in different tiers merge by union of their lines — order-free, with no precedence rule to define. Reach for the entry resolution as a model here and you will get this wrong.
- The two-tier switch gates tag reads exactly as it gates entry reads. With the global tier hidden, its tags leave the union with it; otherwise a filter would match rows that are not on screen.
- A new tag is written to the database you are pointed at: the local tier when one is active, the global library otherwise. Section groupings then live in the manuscript repo, which is the whole reason for versioning them.
- Untagging removes the key from every active tier that lists it, and reports which. Removing from one tier only would leave the tag still visible, so the gesture would appear to do nothing.

One gesture covers both directions, the same ± reading `m` uses for manuscript membership: adding, unless every selected paper already carries the tag, in which case it removes. The direction is therefore a property of the name being typed rather than of the keystroke that opened the prompt, so it is recomputed and shown as the name is typed — a prompt that decided at open time would be lying by the second character.

Rewriting a tag file keeps its comment lines and moves them to the top. Sorting takes the keys out from under a comment written beside them anyway, so there is no position left to preserve; dropping the text entirely is the only outcome actually worth ruling out.

## What is user state, and what is not

Anything the user curates that describes *the user* — saved queries, per-paper priority, which columns a view shows and how wide — is user-local app state under `~/.local/share/astrobib/`, never written into a bib database. The test is not whether the thing is a paper, because tags are not papers either and they are versioned with the database. The test is whether it describes the literature or your attention: priority is a fact about your week, a topic tag is a fact about the papers. Disposable derivatives (fetched PDFs, cached query results) live under `~/.cache/astrobib/`, which is always safe to delete wholesale.

Column configuration stores only what the user has actually changed. An empty configuration is not a special case: with nothing stored every column keeps its responsive default — the author column scales with the terminal, the cite-key column drops first when space is tight, the metric swatch stays off — so opening the configuration panel and closing it again changes nothing.

## Adding to the bib database format

The only acceptable additions to the bib database layout are:

- More `.bib` files in `bib/`
- More tag files in `tags/`

Any new directory or file added to the bib repo must be safely ignored by all astrobib versions that predate it. If this cannot be guaranteed, the addition is wrong. This was checked before `tags/` was admitted rather than assumed: the library loader and `astrobib tidy` both enumerate `bib/` filtered on `extension == "bib"`, and local-library discovery keys on the presence of `bib/` alone, so no earlier build ever looks at a sibling directory. The TUI's separate Manuscript scope is source-driven: it appears only when `.tex` or `.md` sources are present.

## Config and app state versioning

Both `config.toml` and any future app state files must include a `schema_version` integer. Version changes follow this rule:

- Additive changes (new optional keys with defaults): bump schema_version, no migration needed
- Destructive changes (renames, removals): write an explicit migration function in the config module that runs on load when an old version is detected

This rule stops at the bib database. Tag files carry no version field and never will: they are line-oriented text whose whole value is that `cat` and any text editor are sufficient tools. A format that needs a header is the wrong format for that job.

**Unresolved, and deliberately so.** This section and "Future-proof by dumbness" above do not agree, and the code follows neither. `state.json`, `metrics.json` and `query_cache.json` carry a `"version": 1` that no reader ever branches on; `tabs.json` carries nothing. What has actually kept these files compatible is the tolerant reader — unknown keys ignored, missing keys defaulted to what the older build ran with — which is the first section's rule, not this one. Both changes that might have needed a migration (the tab's `ads_sort`, and a query's two homes) were instead shaped so that none was required. The open question is whether to rewrite this section to describe that, or to make the code honour it; it should be settled the next time a state file changes shape, and not before, because the answer wants a real case to argue over.

## Dev vs. stable coexistence

When running a dev install alongside the system-installed version, use `ASTROBIB_STATE_DIR` to redirect user-local app state to a scratch path. The bib database and config are shared between versions and must remain compatible.
