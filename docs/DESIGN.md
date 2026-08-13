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

**Writes are local-first, and the global tier is opted into.** When a local database is active it is the destination: `astrobib add`, `astrobib import`, `astrobib tidy` and the TUI's `i` write there and nowhere else. `--global` (`I` in the TUI) adds the global tier to the same write, `--global-only` takes the local one out of it, and with no local database in scope the global library is the only tier there is, so every write goes to it unflagged.

The rule it replaced wrote both tiers, on the argument that a paper worth having is worth keeping. The analogy that settles it is the virtualenv: a system-wide install is the convenient choice at the moment you make it and the expensive one every time afterwards, because the environment you did not choose is the one that accumulates. A paper read while working on one manuscript is evidence about that manuscript; whether it belongs in the collection that outlives the manuscript is a second question, and one that can only be answered later. Local-first asks it later. The cost is real and worth naming: a paper imported in a project and never shared is invisible from anywhere else, which is the same cost a virtualenv has and the same remedy — you knew where you were standing when you installed it.

So the promotion has to be one gesture, or the default is a trap. `s` shares the selection up into the global library and, pressed again on papers already there, drops the global copies and keeps the local ones — the mirror of `m`, sharing the same add-all-missing-else-remove-all reading. It has the mirror of the rescue rule too: un-sharing an entry the local tier does not hold would destroy the last copy, so it is refused rather than performed. Deletion is `⌫`, which says what it will do and asks first.

An explicit share is not gated by the two-tier display switch. `t` (and `--no-global`) says which tier is on screen; `--global`/`s` names the tier it means, which is the more specific statement of the two — the same precedence the rescue path already assumes when it writes a sole copy into a hidden global tier.

The view follows the same local-first reasoning as the writes: a TUI session started where a local database is in scope opens with the global tier hidden. The CLI does not, and the difference is not an inconsistency — a session is a place you stand and look around in, so it opens on where you are standing, while `list` and `search` are single questions about everything you have, asked and answered before there is anywhere to be.

Cite resolution is the one read that ignores the switch. Whether a cite names a paper you hold is a question about the databases, not about which of them is on screen: gated, `resolve_citation` reports a paper sitting in the global library as `Missing` as soon as that tier is hidden, and the remedy `Missing` implies — go and import it — is the wrong thing to do about a paper you already have. `Library` is the state that says it exactly, and it is only reachable by consulting both tiers.

Manuscript sources are `.tex` files (cites via `\cite*{…}`, expanded through `\input`/`\include`) and `.md` files, with the same root policy for each (`main.tex`/`main.md` when present, else every top-level file of that extension). Markdown citations are pandoc-style — bare `@Key` or bracketed `[@A; @B]` — plus Obsidian wikilinks `[[Key]]` (alias `|` and heading `#` suffixes tolerated), which count as citations only when they resolve in the library: an unresolved wikilink is an ordinary note link, while an unresolved `@cite` surfaces as missing. Obsidian embeds `![[file]]` expand as sources, the `\input` analogue. Code blocks, inline code, and HTML comments never scan.

The rendered markdown bibliography (`astrobib refs`) lives between `<!-- astrobib:references -->` and `<!-- /astrobib:references -->` markers, regenerated wholesale on each run (appended as a `## References` section when absent); everything outside the markers belongs to the user.

## Keys denote papers, not revisions

Two formats depend on this, not one: cite keys written into manuscripts, and the cite keys listed in tag files. Weakening it breaks both.

A cite key identifies a paper for life: both the hash and the year in `AuthorYYYYhhhhh` derive from the paper's stable identifier (arXiv ID, else bibcode), never from mutable record state, so every user holding any phase of the paper (preprint or published) generates the same key. `astrobib update` refreshes metadata beneath an existing key and never rekeys. Citing a specific arXiv revision (v1, v2) is out of scope for keys; that rare need is served by a hand-written `@misc` entry with a versioned eprint field.

## A foreign entry is resolved against ADS, not trusted

An entry arriving from outside — a coauthor's `refs.bib`, a publisher export — is only adopted under its own key if that key is already the one astrobib would generate. Otherwise the entry is looked up at ADS and the canonical record replaces it, because the key must derive from the paper's stable identifier and a foreign entry may not carry one. The lookup prefers arXiv ID, then DOI (both unique), and falls back to exact title + first author + year, which must match exactly one record.

**Ambiguity is currently a dead end, and should not be.** When the title/author/year fallback matches more than one record, `lookup_entry` (`src/ads.rs`) refuses, and `import` prints `⚠ <key> skipped — ambiguous — multiple ADS matches for …` and moves on. The user learns a paper was dropped but never which records it collided with, and has no way to settle it short of finding the DOI by hand. Two things are wanted, and are worth designing together: auto-resolving the common false collision, where the several matches are one paper twice (preprint and published version — same DOI, or same title under differing bibcodes), and, for a genuine collision, showing the candidates and letting the user choose. Note the query is capped at two results today, so the count of matches is not even known; lifting that cap is a prerequisite for either. The real case that prompted this: `peters_gravitational_1964`, "Gravitational radiation and the motion of two point masses".

Whatever the resolution, it must not weaken the rule above it: a chosen record supplies the metadata, and the key still derives from that record's stable identifier rather than from anything the foreign entry asserted.

**The import applies its own re-key map, under a flag.** Import of a Zotero or Overleaf export re-keys nearly every entry — `bartos_rapid_2017 → Bartos2016`, `dorazio_accretion_2013 → DOrazio2012` — and the manuscript went on citing the old strings. `import` printed the map under "old cites resolve by prefix or bibcode; others show as missing", which read honestly means that the ones which *don't* resolve are silently broken cites, and that the only record of how to fix them was terminal scrollback. `astrobib import <file> --rename-citekeys` now rewrites them in the sources.

It could not be a later command, which is the alternative that was weighed and lost. `convert` derives its map from `resolve_citation`, which knows full keys, unambiguous prefixes and bibcodes — a foreign key like `krauth_disappearing_2023` resolves to nothing at all, so the mapping is unrecoverable once the import that computed it has exited. Recovering it later would mean persisting a rename log, which is user-local state by "What the database is and is not", and which then has to be aged, invalidated and reconciled against sources that may have moved on. What it does take is the same care `convert` takes: it prints what it changed per file, because this is the one destructive thing astrobib does to files it does not own.

**`import --dry-run` previews both halves.** It was left out at first on the argument that the map is a product of the import — an entry's canonical key comes from the record ADS resolved it to — so there is nothing to preview. That argument was wrong, and in a way worth recording: resolution is a *read*. Only `save_entry` writes, so the run resolves every entry, computes the whole map, and reports both halves of what it would do — which `.bib` files it would write to which tier, and which cites it would rewrite in which sources — while touching nothing. That is the run a coauthor's export deserves before it is let near either.

That is why the command is three passes rather than one loop: resolve, write, report. Only the middle one writes, so a dry run runs the first and the last and skips it, and the two runs cannot drift because they are the same code deciding the same things.

One thing makes it more than a flag on a branch. A rename's right-hand side is the *short* key, the shortest unambiguous prefix, and that is a function of the library as it stands after the import: two papers by the same author and year shorten differently once both are in. A dry run therefore cannot ask the library what the key would be — it has to resolve the whole batch first and shorten against the library plus the pending entries, or it will preview keys the real run would not produce. The same reasoning applies to the entries it reports as skipped-because-present.

Reporting after the whole batch is what the *real* run needs too, which is what the three passes bought. Shortening as each entry landed was correct at the moment it was printed and wrong by the end of the same import: the first of two same-author-same-year papers took the bare `Delacroix2018`, unambiguous until the second arrived — and `--rename-citekeys` then wrote that key into the sources, where `refs` reports it as ambiguous and drops the paper from the bibliography. A short key is a function of a set, so `keys::shortest_unambiguous` takes the set as an argument and neither pass owns it: the real run reads the library it has just written, and the preview shortens against each tier's keys plus what the run would add to that tier, resolving which tier answers by `MergedLibrary::get`'s own rule.

The flag refuses outside a manuscript with sources, and refuses *before* importing anything: discovering there was nowhere to apply the rewrite after the fact would leave the map on the terminal and the library changed, which is the state the flag exists to end. `--local-only` is checked in the same preflight and for the same reason — a tier flag naming a tier that is not there used to print a destination it could not honour and then fail on the first entry.

Rewriting covers `.tex` and `.md` alike, which is the rule rather than a courtesy: a cite means the same thing in both, so a rewrite that reached one would leave half a manuscript pointing at keys that no longer exist. `manuscript_source_files` is the union, and markdown cites are rewritten in their own syntaxes — pandoc `@Key` and `[@A; @B]`, Obsidian `[[Key]]` with alias and heading suffixes preserved. Code fences, inline code and HTML comments are skipped, as they are for scanning; an `![[embed]]` names a file rather than a paper and is left alone; and a wikilink is rewritten only where the map names its target, which is the same rule that makes one a citation at all.

**`convert` reads both source kinds.** It always shared the rewriter, so it wrote markdown correctly while *scanning* only through `manuscript_tex_files` — which meant "No .tex sources found" in a markdown-only manuscript, and in a mixed one a re-key of just the half the `.tex` files happened to cite. It now scans `scan_md_files` beside `scan_tex_files` and merges them into one first-seen list.

The wikilink rule is the piece of design in it. A `[[Key]]` is a citation only where it resolves; everywhere else it is an ordinary note link, so one that resolves to nothing is dropped before the map is built rather than reported as "unresolved (left alone)", which would turn every link in a linked note collection into a line of output. A key cited both ways is a citation, so the flag survives only while every sighting of that key is a wikilink — otherwise a `\citep{Key}` in the TeX would be silenced by a `[[Key]]` in a note.

What is *not* regenerated is `refs.bib`. `convert` regenerates it because rewriting cites is the whole of what `convert` does; an import may have been told to write one tier only, and `refs` syncs the manuscript db as it goes, so running it would be a side effect reaching past what was asked for. The import says the bibliography is now stale and names the command that fixes it.

## Persistent searches are user-local

Saved ADS query tabs live in user-local app state (e.g., `~/.local/share/astrobib/`), not in the bib database. They are not synced to other group members. Each user maintains their own set of active searches.

A query has one of two homes. The global set is visible from every directory; a manuscript database (by its root path) also has its own, which appears when that manuscript is the active one. A session reads both, marks where one group ends and the other begins, and can move a query between them. The storage stays user-local either way — a manuscript's queries are never written into the manuscript repo.

Both were once keyed the same way, which made "which queries do I have?" a question about where you were standing. Discovery is a cwd walk-up on the presence of `bib/`, and that is right for *entries*, because an entry belongs to a paper — but a saved search belongs to you, and an empty `bib/`, which git cannot even record, was enough to take your queries off screen with nothing said. So a query you typed is global by default. The exception is a query that names one paper: `citations(…)` and `references(…)` are made from a card with one keystroke and are spent as soon as they are followed, so they are filed with the manuscript rather than trailing every paper you ever opened through every directory. The rule reads off the query's shape, not off which gesture made it, so one typed at the prompt is filed like one made by `C`.

The home is the context key holding the tab, never a field on the tab: two places to say the same thing is two places to disagree. An id appearing in both sets is read once, the global copy winning — results route to the first scope whose id matches, so a duplicate would leave its twin waiting on a result delivered elsewhere.

A tab carries the query, a name, a result limit, how its results are ordered on screen, and what ADS *returns* — the API `sort` that decides which records come back. The last of those was once left out, on the argument that it is chosen while composing and matters only while the query is being worked. That argument was wrong, and in a way worth recording: it applies just as well to the query text, which nobody proposed discarding. The query, the result count and the selection sort are one configuration — a tab restored with a different one is not the query that was saved, and a query handed to a colleague that arrives configured differently is not the query that was sent.

**A scope you can open by clicking says how it is closed.** `ctrl+w` closed the active scope and was written down in the README and the tutorial outline — nowhere the app itself would tell you. Three things now say it, and each answers a different half of the question. Every query capsule carries a `✕`, so the strip shows which capsules go away and which do not: the library and the manuscript carry none, which states that they are permanent without spending a word on it. Hovering a capsule or its mark fills the footer hint, as the card buttons and column headers already do. And `⌃w` is a row on the keys panel like every other key, dimmed off a query scope with the reason given, because a key that quietly does nothing is the thing the panel exists to prevent.

The mark is on every query capsule rather than on the active one alone. It costs two cells each — the strip wraps a capsule or two sooner — but a mark that appears only under the cursor, or only on the active tab, changes the strip's width as you move around it, and a row of capsules that reflows while you are aiming at one is worse than the width. It also makes each `✕` close the capsule it is drawn on, which `ctrl+w` cannot do: pruning four spent citation trails is four clicks rather than four visits.

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

Every scope offers the same columns, less the ones its rows cannot answer. A manuscript row is a cite and a query row is an ADS record, but both name a paper, so the paper columns — the metric swatch, `↓`, Year, Author, Title, Key — are on offer in all three scopes and are drawn, sorted and configured the same way in each. What differs is only what a scope alone can say: `Entered` is a fact about an ADS record, `Cited` and `State` are facts about a citation, and the library's `●` says a thing the manuscript's own cite glyph already says in more detail. The defaults still differ per scope, because the width a scope's own columns take is the width its optional ones have to fit around.

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
