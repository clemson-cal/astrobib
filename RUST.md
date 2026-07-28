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
- `src/tui.rs` — first-cut ratatui TUI: year-sorted table (↓ ★ columns live), live filter on `/` with the full query language, j/k/g/G navigation, instant quit.

Warm full-library load + query + print measures ~7 ms end to end (vs ~1 s Python+Textual startup floor).

## Not yet ported

ADS client (search tabs, import, update), manuscript databases and `refs` sync, UAT browser/keyword tree, PDF fetch/browser flows, export, config/state writing, star toggling — the TUI is read-only so far. Distribution plan when ready: maturin `bindings = "bin"` wheels to PyPI so `pipx install astrobib` keeps working, per the discussion in the main repo.

## Parity rules

Anything both implementations write must be byte-identical: cite keys (golden-tested), short keys (diff-tested), `.bib` serialization (FIELD_ORDER). The bib database format is the contract — see DESIGN.md.
