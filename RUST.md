# astrobib Rust port

Status doc for the `rust` branch. The crate lives at the repo root alongside the Python package, which stays checked in as the reference implementation during porting.

## Run it

```
cargo run --release              # TUI (default)
cargo run --release -- list
cargo run --release -- search '^zrake OR kw:"compact objects"'
cargo run --release -- show Zrake2022i
cargo test                       # includes golden key vectors
```

## What's ported

- `src/keys.rs` — cite key generation. Byte-for-byte compatible with keys.py; guarded by `tests/golden_keys.json` (38 vectors generated from the Python implementation over the real library plus edge cases). Regenerate vectors from Python when adding cases; never hand-edit expected keys.
- `src/bib.rs` — tolerant single-entry BibTeX parser (brace-aware, field names lowercased like bibtexparser) + writer matching library.py FIELD_ORDER, and a LaTeX-accent→Unicode converter covering the common ADS patterns.
- `src/library.rs` — Entry, Library: load `bib/*.bib`, bibcode index, O(N log N) short keys (verified identical to Python on the real library), key/prefix/bibcode resolution. No parse cache — Rust parses the whole library faster than Python reads its cache.
- `src/query.rs` — the full filter language incl. uppercase OR/AND/NOT, bare `^author` sugar, year ranges, `is:` terms, and `to_ads_query` translation. Test battery mirrors the Python one.
- `src/tui.rs` — ratatui TUI: year-sorted table (↓ ● ★ columns live), live filter on `/` with the full query language (incl. `is:ms`/`is:pdf`), toggleable pub card (`d`), star toggle (`s`), j/k/g/G navigation, instant quit.
- Manuscript databases (read side + stars): walk-up discovery matching state.find_manuscript_db, MergedLibrary with personal-wins merge, ● indicator; membership toggling and refs sync not yet ported.
- `star` CLI subcommand (Rust-only; the Python CLI stars via the TUI) — used to verify the write path: star/unstar sequences produce byte-identical files to Python when run one process per operation.

A format quirk worth knowing, faithfully reproduced: bibtexparser v1 stores fields in reverse file order, so every parse→rewrite cycle flips the trailing (non-FIELD_ORDER) fields. Both implementations do it identically; files oscillate between two stable forms.

Warm full-library load + query + print measures ~7 ms end to end (vs ~1 s Python+Textual startup floor).

## Not yet ported

ADS client (search tabs, import, update), manuscript databases and `refs` sync, UAT browser/keyword tree, PDF fetch/browser flows, export, config/state writing, star toggling — the TUI is read-only so far. Distribution plan when ready: maturin `bindings = "bin"` wheels to PyPI so `pipx install astrobib` keeps working, per the discussion in the main repo.

## Parity rules

Anything both implementations write must be byte-identical: cite keys (golden-tested), short keys (diff-tested), `.bib` serialization (FIELD_ORDER). The bib database format is the contract — see DESIGN.md.
