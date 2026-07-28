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
- Selection mode (iOS-style, replaces the Python TUI's check-marks): Space, a click in the leftmost gutter, or option/ctrl+click anywhere on a row enters the mode and toggles rows (◯ unselected, ◉ selected); Esc exits and clears, and so does toggling the last selected row off. cmd+click can't work: the SGR mouse protocol has no cmd bit and macOS terminals reserve it. Mouse support includes click-to-highlight and wheel scrolling.
- Buttons render as rounded powerline pills ( caps in the chip color); ASTROBIB_ASCII=1 falls back to flat chips of identical width for terminals without Nerd Font glyphs.
- Actions on the highlighted entry or the whole selection (Python-TUI key map): `s` star, `m` manuscript-db toggle (any-missing → add all, else remove all, with last-copy rescue into the personal library), `p` download PDFs on a background thread (ADS OA resolver → arXiv fallback, live progress in the status bar), `o` open cached PDFs, `X` clear cached PDFs (or cancel a pending browser watch), `B` browser download (resolver-verified URL, ~/Downloads watched 60s with the two-poll stability check), `d` remove from both databases (no confirmation, as in Python). Membership/removal writes verified byte-identical to Python, including the rescue path.
- Control panel (`ctrl+p`, rightmost), tabbed with clickable headers. The Actions tab lists every action with its key, unavailable ones dimmed per a single-vs-multi policy (browser DL and pick are single-target; download/open/clear dim when no target qualifies; manuscript dims without an active db). Rows are clickable; the same dispatcher backs keys, panel clicks, and card buttons. New specialized tabs slot in alongside.
- Copy-to-clipboard: `y` opens a which-key chord (the panel force-shows the Copy tab as the menu, then restores): `yy` cite key (short, the Python TUI's `y`), `yY` full key, `yb` bibcode, `ya` ADS URL, `yx` arXiv URL, `yd` DOI URL, `yp` cached-PDF path, `yt` title; top-level `Y` copies full keys directly (Python parity). Multi-selection joins keys/bibcodes with `", "` (Python convention) and URLs/paths/titles with newlines; entries lacking a field are skipped, and items with no value dim in the menu. Clipboard via pbcopy on macOS, OSC 52 elsewhere (the Python `_copy_text` strategy).
- Text-selection mode: `v` releases mouse capture so the terminal's native drag-select and copy work (the status bar shows a persistent ✂ banner); `v` again re-captures. Modifier-drag (option on macOS, shift on Linux) bypasses capture without the toggle.
- Pub card emulating the Python DetailPanel: body, then a bordered cyan links row (`ADS · arXiv:<id> · DOI`, click opens the browser), then the PDF buttons with Python's labels/colors and visibility rules (`arXiv ↓`/`ADS OA ↓` cyan, `browser ↓` yellow, `pick …` magenta when uncached and eligible; `Open ↗` green / `Clear ✕` muted when cached), a transient PDF-status line (⏳ waiting with clickable cancel, ✓/✗ results), and a footer (keywords, cite key with dim hash suffix, preprint note). ANSI palette colors, so the terminal theme applies as in Textual. `pick …` opens a modal ~/Downloads PDF list (newest first, ⏎ imports a copy after a %PDF header check). Pub card toggle is `D`/`z`.
- Removal is on the Delete key behind a confirm modal (lists targets; ⏎/y or clicking `remove` confirms, Esc/n/`cancel` aborts) — a deliberate departure from Python's unconfirmed `d`.
- Manuscript databases (read side + stars): walk-up discovery matching state.find_manuscript_db, MergedLibrary with personal-wins merge, ● indicator; membership toggling and refs sync not yet ported.
- `star` CLI subcommand (Rust-only; the Python CLI stars via the TUI) — used to verify the write path: star/unstar sequences produce byte-identical files to Python when run one process per operation.

A format quirk worth knowing, faithfully reproduced: bibtexparser v1 stores fields in reverse file order, so every parse→rewrite cycle flips the trailing (non-FIELD_ORDER) fields. Both implementations do it identically; files oscillate between two stable forms.

Warm full-library load + query + print measures ~7 ms end to end (vs ~1 s Python+Textual startup floor).

## Not yet ported

ADS search tabs in the TUI, the `refs`/`.tex` sync flow and export, UAT browser/keyword tree, the `update` (arXiv→published) command, config/token writing (the ADS token is read from state.json — set it with the Python tool). Distribution plan when ready: maturin `bindings = "bin"` wheels to PyPI so `pipx install astrobib` keeps working, per the discussion in the main repo.

## Parity rules

Anything both implementations write must be byte-identical: cite keys (golden-tested), short keys (diff-tested), `.bib` serialization (FIELD_ORDER). The bib database format is the contract — see DESIGN.md.
