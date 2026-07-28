# Design study: is the personal-library / manuscript-db split worth it?

**Question.** astrobib maintains a global personal library (`~/.local/share/astrobib/library/`) *and* an optional per-manuscript `bib/`, joined by a merged view, dual writes, membership toggling, and a last-copy rescue rule. Would a unified model — "astrobib operates only on a local bib dir and optionally on a main.tex" — be simpler without being meaningfully less convenient?

**Answer, up front.** Keep the split (Model 1). The unified models are genuinely simpler in code — roughly 300 lines and four user-facing concepts would vanish — but both of them degrade the single most frequent journey the tool exists for: citing a paper you already collected, while writing, offline, in one keystroke. The split's complexity is real but bounded, its bug class has been closed by construction (rescue rule, single dual-write path, personal-wins merge), and the identity-derived key policy already guarantees the two stores can never disagree about content, only about presence. The full argument, including the strongest case against this recommendation, is below.

---

## The three models

- **Model 1 (status quo).** Global personal library, plus an optional manuscript `bib/` discovered by walk-up (`bib/` + `.git`). `MergedLibrary` reads span both (personal wins on key collision); imports write to both; `m` / the `◆` card chip toggle membership; removing the last copy from a manuscript rescues it into the personal library first.
- **Model 2 (fully local).** No global store at all. astrobib operates on the `bib/` found by walk-up (or cwd), optionally with `main.tex` for citation classification. Every paper directory is self-contained; there is no cross-directory anything.
- **Model 3 (single active library).** Exactly one library is active at a time: the walk-up result inside a manuscript repo, else the global default directory. No merging, no membership, no `●`; imports write only the active library. Cross-project reuse happens by re-fetching from ADS (which, thanks to identity-derived keys, acts as the true global library) or an explicit copy command.
- **Model 4 (inheritance with promotion).** Local bibs *inherit* more global ones: inside a paper repo the visible collection is the local `bib/` overlaid on the personal library (local wins), and the chain generalizes to any depth (paper ← group ← personal). Writes automatically target the **local** db only. Explicit commands move entries *outward* — `promote <key>` copies a locally-imported entry into a more global library — and inherited entries materialize *inward* on demand (cite-and-localize, the analogue of today's `m`). Removal of a sole local copy auto-promotes instead of destroying (rescue and promotion become one primitive).

---

## (a) Code audit: what exists only to support the split

All counts are against the current Rust crate (5,434 lines of `.rs` across `src/`).

### library.rs (516 lines — the split owns ~30% of this file)

| Item | Lines | Notes |
|---|---|---|
| `MergedLibrary` struct + main impl | 305–426 (~122) | merged `entries()`/`get()`/`resolve()`/`possible_matches()`/`get_by_bibcode()` with personal-wins precedence; `in_manuscript`/`in_personal`; dual-write `save_entry`; dual `remove_entry`; `add_to_manuscript` (copy with `astrobib_starred` stripped); `remove_from_manuscript` (last-copy rescue) |
| `write_entry` (fixed-key write) | 284–298 (15) | exists *only* for membership copies and the rescue path — the normal path always regenerates the key |
| `CiteState::Library` variant | 434–435 | the 4th classification state ("resolves, but only in the personal library") exists only because there are two stores |
| `find_manuscript_db` | 473–485 (13) | survives in Models 2/3, repurposed as active-library selection; note the subtle exclusion of the active library root from the walk-up — a rule that exists only because the library root is itself a `bib/` dir that must not be mistaken for a manuscript |

Strictly split-only in library.rs: **~155 lines**. `resolve_citation` itself (443–468) survives in every model but sheds one state.

### tui.rs (3,188 lines — 47 occurrences of `manuscript`/`in_manuscript`)

Strictly split-only:

| Item | Lines | ~Count |
|---|---|---|
| `toggle_manuscript` (`m` action: any-missing→add-all, else remove-all, rescue counting) | 1036–1080 | 45 |
| `◆/◇` membership chip on the pub card: render + click handler (with rescue message) | 2784–2805, 1381–1401 | 43 |
| `Action::Manuscript` plumbing: variant, `available()` arm gating on `lib.manuscript.is_some()`, dispatch, `m` binding, cheat-sheet row | 706, 748, 1663, 1941 | ~8 |
| `●` membership column: cell, contextual header (renders only inside a manuscript context — itself a fix, 6a83498), sort handling | 2328, 2368–2369, scattered | ~15 |
| `in_manuscript` closure built for the query context (feeds `is:ms`) | 914–921 | 8 |
| live reclassification hooks after membership changes | 786, 989–995 | ~10 |

Subtotal: **~130 lines** strictly split-only.

Separately, the **Manuscript scope** (`MsRow`, `ms_rows` 447–483, `rescan_manuscript` 485–494, its renderer at 2068 ff., ctrl+w/selection gating at 740–741, 1445, resolved-row action bridging at 895–897) is **~120 lines** — but it survives in *every* model, including Model 2, because classifying `.tex` citations against a bib dir is the point of "optionally on a main.tex". Only its `○ library` state (cited, resolvable, not yet a member — "press m") is split-specific.

### query.rs and elsewhere

- `is:ms`: the `in_manuscript` field on `QueryContext` (line 115) and the match arm (line 136) — ~6 lines, plus README/help documentation. Meaningless in Models 2/3.
- `main.rs`: constructs `MergedLibrary` (lines 48–49); would become plain `Library` — trivial.
- `tabs.rs`: keys saved ADS tabs per manuscript context — survives in all models (keyed per active context) but its "global vs per-manuscript" distinction collapses in Model 2.
- Python-era surface that the Rust port has *not* (yet) reproduced but the model implies: `--personal-only` / `--ms-only` import flags, the `M` hide-personal-only toggle, `refs`' pull-from-personal sync. Model 1 obliges the port to eventually grow these; Models 2/3 delete the obligation.

**Total strictly split-only: ~300 lines (~5.5% of the crate), concentrated at ~30% of library.rs**, plus four user-facing concepts (membership, merge precedence, rescue, dual-write) that every future feature must position itself against.

### Git-log evidence of maintenance cost

Of ~140 commits in the history, the split accounts for three feature commits and a measurable fix tail:

- `46fe285` — introduces the split (Python): MergedLibrary, ◆ column, Manuscript tab, dual writes.
- `d8f98f4`, `189da53` — the Rust port of the read side and the manuscript scope (two of the port's larger commits).
- `c32a57c` — **"Rescue ms-only entries to personal library on removal… Closes the --ms-only data-loss gap."** A genuine data-loss bug that can only exist when two stores disagree about who holds the only copy. The rescue rule (and its DESIGN.md clause) is permanent complexity purchased to close it.
- `c6a7727` — membership toggling re-sorts the table and lost the cursor; needed same-state cursor advancement.
- `1f32f32` — ctrl+w could close the Manuscript tab; needed gating that exists because scopes are heterogeneous.
- `3a7cf5d` — saved query tabs had to become per-manuscript-context rather than global.
- `1ff38f1` — CLI/TUI parity for merged reads: keeping *two frontends* agreed on *two stores* is an ongoing obligation, not a one-time cost.
- `531eaca`, `6a83498` — display fixes to membership indicators (suffix dimming; `●` header rendered even with no manuscript active).
- Same family, adjacent cause: `deac792` "Keep PDF cache status in sync across views" — a cross-view desync (bibcode-keyed vs cite-key-keyed cache status). Not the library split itself, but the same disease: two representations of one fact drifting apart.

Reading: **~6–8 fix commits over the feature's lifetime, including one data-loss class**. Real, but the curve flattened — the Rust port reproduced the whole feature with (so far) zero follow-up fixes, because the invariants (rescue, dual-write through one path, personal-wins) were known by then. The maintenance cost was mostly the one-time cost of *discovering* the invariants.

---

## (b) User journeys

Friction: **low** / **med** / **high**. The user is assumed to have years of accumulated library (the code is explicitly sized for 1e4 entries) and to write papers in git repos shared with coauthors.

| Journey | Model 1 (split) | Model 2 (fully local) | Model 3 (single active) |
|---|---|---|---|
| Start a new paper | **low** — `mkdir bib` in the repo; library view instantly shows your whole collection with an empty `◆` column | **low** — `mkdir bib`; view starts empty | **low** — `mkdir bib`; view starts empty |
| Import while writing | **low** — `i` writes both; the paper repo stands alone *and* your library accrues | **low** — one write, but accrues only to this paper | **low today, med over years** — one write to the manuscript; your personal library silently stops growing, because most importing happens while writing. The long-run asset erodes. |
| Cite an old paper from a previous project | **low** — type `\citep{Zrake2020}`; Manuscript scope shows `○ library`; press `m`. Offline, instant, no network. | **high** — the paper lives in *some* previous repo. Grep old checkouts, or re-fetch from ADS. There is no "my collection". | **med–high** — the scope shows `✗ missing`; `S` pre-fills an ADS search: network, token, quota, and the paper must be re-findable. An offline `copy` command fixes this — by reintroducing a second, now-invisible library. |
| Share the repo with coauthors | **low** — identical in all three: `bib/` is committed, self-contained copies, coauthors without astrobib use `refs.bib` | **low** | **low** |
| Browse "everything I've collected" | **low** — the Library scope, from anywhere | **high / impossible** — that set does not exist; it is the union of every repo you ever made | **low outside a paper, med inside** — you must leave the repo (or pass `--library`) to see your own collection while writing, which is exactly when you want it |
| Generate refs.bib | **low** — cited-but-not-member entries are visible (`○`) and pullable; `refs` can copy them in from personal | **low** — `refs` writes from local `bib/`; missing keys are simply missing | **low** — same as Model 2 |

Two journeys decide it. **"Cite an old paper"** is the highest-frequency event in the tool's life — a typical introduction cites tens of previously-collected papers — and both unified models turn a one-keystroke offline action into a network round-trip or an archaeology dig. And **"browse everything"** is the reason a personal library exists at all; Model 2 abolishes it outright, which is why Model 2 is strictly dominated: it keeps Model 3's frictions and adds the loss of the collection. (Model 2 also leaves `astrobib` with nothing to operate on outside a paper directory.)

A subtle third: in Model 3, `d` (remove) inside a manuscript deletes the *only* copy of a coauthor-added entry — the rescue rule protects against exactly this today, and it can only exist when there is somewhere to rescue *to*.

---

## (c) Migration from today's on-disk state

- **Model 1**: none (status quo).
- **Model 3**: **zero on-disk migration.** This is worth stating clearly: because the split was never encoded in the data — both stores are plain `bib/*.bib`, indistinguishable by design — Model 3 is a delete-only code change. The global library keeps working as the default context; every existing manuscript repo keeps working as the active context when you cd into it. `tabs.json` keying is unchanged. The only additions would be an optional `astrobib copy <key> [--from <path>]` seeding command. Users would *feel* the change (no `●`, no `m`, no merged view), but nothing on disk moves.
- **Model 2**: no file moves either, but the global library is demoted to "a directory you can cd into", and the default-root resolution (`ASTROBIB_LIBRARY` / `ASTROBIB_STATE_DIR`) loses its meaning. Users with an accumulated library must adopt a habit change with no tool support. Trivial mechanically, costly behaviorally.

The clean migration story for Models 2/3 is a genuine point in their favor — and it is a *consequence of the dumb-format principle*, which also means the option never expires. astrobib can adopt Model 3 in any future release without a migration, so there is no urgency premium on deciding now.

## (d) Impact on DESIGN.md contracts, the query language, and TUI scopes

- **Dumb formats**: unaffected in all models. No model requires touching the on-disk format; that all three are even possible is the dumb-format principle working as intended.
- **Copies, not links**: Model 1 implements it via membership copies; Model 3 via re-fetch (identity-derived keys guarantee a re-fetched copy is byte-agreed on key and content-agreed with any older copy); Model 2 trivially. The contract survives everywhere; only the copying mechanism differs.
- **No tool state in the repo**: unaffected — no model tempts a violation.
- **Rescue rule** (DESIGN.md: "Removal from a manuscript database is never destructive"): only expressible in Model 1. Models 2/3 must either accept destructive removal of sole copies or add a trash mechanism.
- **`is:ms`** and the `●` column: deleted in Models 2/3 — with one store, membership is not a property. Any saved filters using `is:ms` degrade (per the query grammar, unknown `is:` terms match nothing).
- **Manuscript scope semantics**: survives in all models (it is the "optionally on a main.tex" half of the unified idea). Classification collapses from four states to three in Models 2/3: `● ok / ✗ missing / ≈ ambiguous` (+ uncited members). The `○ library — press m` state, the scope's most-used affordance, is Model-1-only.
- **Python interop**: the Python tool at `v0.4.0` implements Model 1. A Rust tool on Model 2/3 still reads/writes compatible files (format is the contract), but the two would disagree about *behavior* on the same directories (e.g., Python dual-writes an import the Rust tool made single). Mild, since Rust is now official, but nonzero while v0.4.0 remains installed anywhere.

---

## Model 4 examined: is it basically what we have already?

**On the read side — yes, almost exactly.** `MergedLibrary` *is* single-level inheritance: inside a paper repo you see the local `bib/` overlaid on the personal collection, the `○ library` classification is precisely "inherited but not yet materialized," and `m` / the `◇` chip is the localize step. Nothing about Model 4's visibility story requires new machinery; the audit in section (a) already prices it.

**Three things are genuinely different**, all on the write side:

1. **Write targeting.** Today an import inside a manuscript dual-writes (local *and* personal, atomically); Model 4 writes local-only and makes the personal library grow **only through explicit `promote`**. This converts the split's one implicit behavior ("importing here also writes over there") into two explicit ones — more predictable, but it reintroduces Model 3's erosion hazard in softened form: the collection accrues only as diligently as you promote. A `promote --review` (list local-only entries across recent repos) would be the countermeasure, and is itself new surface.
2. **Precedence.** Today personal wins on key collision; Model 4's nearest-wins is the natural inheritance rule. Since starring was removed, the copies can differ only in curated keywords — personal-wins existed to protect personal fields, and with those gone the inversion is nearly moot. (Worth deciding deliberately if Model 4 is ever adopted: nearest-wins means a paper repo can shadow your curated keywords with a coauthor's copy.)
3. **Depth.** Model 1 hardcodes exactly two levels; Model 4 generalizes to chains (paper ← group ← personal). Nobody has asked for level three; the generality is free conceptually but not free in resolution rules, `tabs.json` contexting, or the scope strip.

**Net assessment.** Model 4 = Model 1's read model + Model 3's write model + an explicit promotion verb, with rescue elegantly absorbed into promotion. It is the strongest *refinement* candidate: it deletes no safety (rescue survives as auto-promote), keeps the offline-reuse journey intact (inheritance preserves `○ → localize`), and replaces the one behavior users must currently be told about (dual writes) with commands they invoke. What it does **not** do is reduce the machinery this study audited — the merged view, membership classification, and copy plumbing all remain; a second write path (`promote`) is added while one implicit write is removed. So the honest answer to "is this basically what we have?" is: **yes for reads and membership; the difference is purely who decides when the personal library grows — the tool (today) or you (Model 4).** If the dual-write ever feels surprising or wrong in practice, Model 4 is the adjustment to make; it is reachable from Model 1 by changing the import target and adding `promote`, with zero on-disk migration, and is therefore — like Model 3 — an option that never expires.

---

## Recommendation

**Keep Model 1** (with Model 4 as the designated refinement if dual-write ever chafes — see above). The split is ~300 lines and four concepts, purchased against the tool's two defining journeys: one-keystroke offline reuse of a decade of collected papers, and a browsable personal collection that accrues as a side effect of writing. The bug tail that justified this study was real but is now structurally closed — dual writes go through one path, removal goes through the rescue invariant, and identity-derived keys make content divergence impossible (the stores can differ only in *presence*, never in *what a key means*). The Rust port reproduced the whole feature against those invariants without incident, which is evidence the complexity is now specified rather than exploratory.

Two consolations for the simplicity instinct that motivated this study:

1. **Model 3 is the designated fallback, and it never expires.** Because the format never encoded the split, unification remains a delete-only change at any future date. If the fix tail resumes, adopt Model 3 then.
2. **Harvestable trims within Model 1**: don't port `--ms-only` (the sole *user-initiated* source of the rescue-needed state; coauthor-added entries still justify the rescue rule itself); don't port the `M` hide toggle (`is:ms` subsumes it); keep the `●` header contextual as it already is. The revised DESIGN draft (docs/DESIGN-draft.md) codifies the split's invariants explicitly so future work inherits them as rules rather than rediscovering them as bugs.

### The strongest argument against this recommendation, stated fairly

The key policy already did the hard part of unification, and Model 1 refuses to cash it in. Keys derive from the paper's stable identity, so any two copies fetched at any time by any person agree — which means **ADS is already the true global library**, coordinated better than any local store, and the personal library is, in information terms, a cache of it. `MergedLibrary`, the `●` column, `○ library`, `is:ms`, dual writes, and the rescue rule are then a hand-rolled cache-coherence layer maintained forever in exchange for saving an HTTP request per old citation. The git log shows what coherence layers cost: a data-loss gap (`c32a57c`), cursor and gating fixes, per-context tab keying, and a standing CLI/TUI parity obligation — and every future feature (export, update, UAT tagging) must answer "which store? which scope? what does the merge show?" before it can ship. Under Model 3 those questions cannot arise, a new user learns "astrobib manages the bib/ next to you," and the README loses three sections. The offline objection is the weakest kind of objection — a `copy` command over `~/.local/share/astrobib/library` restores offline reuse in twenty lines — and "the personal library stops accruing" assumes the personal library ought to be the asset, when the model's whole claim is that ADS is. If astrobib were being designed today, from scratch, with these keys, it is not obvious Model 1 would be chosen.

The counter-counter is the journey table: the coherence layer is 300 lines that run in microseconds and have been quiet since their invariants were written down, while the HTTP request it saves is one that a writing session performs dozens of times, sometimes on a plane. But the against-case is coherent, and Model 3 remains permanently available if experience tilts the table.
