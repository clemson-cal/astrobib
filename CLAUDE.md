# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Read [docs/DESIGN.md](docs/DESIGN.md) before adding features or changing data formats. It is the authoritative statement of design constraints.

**Parallel agents: one writer per working tree.** Concurrent agent sessions must not share this working tree — spawn writing subagents with worktree isolation (`isolation: "worktree"` / `git worktree add`) and integrate through commits. When committing here, stage explicit paths (never `git add -A`) and review `git status --short` first: another session's uncommitted work may be present.

---

## "new task" protocol

When the user says **"new task"**, they are about to clear the context and start fresh. Before confirming, do the following:

1. **Check for incomplete work.** Verify that clearing the context would not interrupt an in-progress workflow: uncommitted changes, a half-finished refactor, failing tests, or an unresolved discussion. If work is incomplete, say so and either finish it or bring the tree to a coherent stopping point first.
2. **Update design docs.** If the project is at a suitable stopping point, update the design docs (`docs/`, and the README status section) so the next steps are easily discoverable by a fresh session with no prior context.
3. **Commit.** Commit the doc updates along with any outstanding work, with a clear message.
4. **Confirm.** Only then tell the user it is safe to clear the context.

---

## Open threads

Known and deliberately deferred, with the reasoning already done. None is a defect in shipped behaviour; each is a judgement call left for a session with a real case to argue over.

- **`maturin upload` is deprecated, and the publish job runs it.** `.github/workflows/release.yml` ends in `PyO3/maturin-action@v1` with `command: upload`; both it and `maturin publish` are slated for removal ([PyO3/maturin#2334](https://github.com/PyO3/maturin/issues/2334)), which the 0.20.0 run said as an annotation while still succeeding. The replacement is `pypa/gh-action-pypi-publish` over the same assembled `dist/`, optionally trading `PYPI_API_TOKEN` for trusted publishing. Deferred because nothing breaks until maturin actually drops the command — but the fix belongs between releases, since a tag push is the worst place to find out it has. Reasoning in PUBLISHING.md.
- **Below a certain width the table's headers and its cells disagree.** `table::solve` places headers at the declared fixed widths, while the body is laid out by ratatui's own constraint solver; once those widths over-subscribe the area (`Constraint::Min(20)` on the flex column included) ratatui shrinks the cells and the two no longer line up — the header sort rects then sit over the wrong columns. Pre-existing and visible in every scope: the committed golden shows it for the library and the manuscript alike at `w=64` and `w=84` (`tests/tui/baselines/table_chrome.txt`). Nothing was made worse than the responsive rules already allowed, and the fix is a judgement call — clamp in `solve`, or shed columns until the declared widths fit — best made by whoever first cares about 60-column terminals.
- **State-file versioning is unresolved.** `docs/DESIGN.md` "Config and app state versioning" contradicts "Future-proof by dumbness", and the code follows neither. Noted in DESIGN.md itself; settle it the next time a state file changes shape.
- **`load_cached_articles` re-reads and re-parses all of `query_cache.json` per tab** (`src/tabs.rs`, called per tab from `restore_tabs` in `src/tui/scopes.rs`). Invisible at a handful of tabs; now that a session shows both homes' queries at once, larger sets are likelier. Measure before fixing.
- **The per-frame hit-test registry refactor is complete.** `App` now has one fresh `Hits` registry for clickable and wheel geometry; only `last_table_area` persists because table event handlers need the last solved layout. The implementation and verification record are in [docs/plans/hit-registry.md](docs/plans/hit-registry.md).
- **Two scenarios set the TUI suite's floor at ~10s.** `s32_edit_query` and `s45_capsule_close` both wait out the ageing of a startup note so the footer hint they test can take the line. Everything else overlaps around them, so the whole roster costs about what one of them does; shortening both is the only thing that would make the suite meaningfully faster.

---


## Development commands

```bash
cargo build --release        # binary at target/release/astrobib
cargo test                   # includes the golden format/key vectors
cargo run --release          # TUI
cargo run --release -- list  # CLI
tests/tui/run.py             # TUI integration scenarios (see tests/tui/README.md)
tests/tui/run.py -k s26 -j 1 # one scenario, serially
```

Verify TUI changes headlessly with the committed pyte pty harness (`tests/tui/run.py`): it drives the real binary in a pseudo-terminal against a scratch `ASTROBIB_LIBRARY` / `ASTROBIB_STATE_DIR` built from `tests/tui/fixtures/` — never the real library — and reconstructs screens with pyte rather than grepping raw bytes. It bootstraps its own venv; add new scenarios under `tests/tui/scenarios/`. Scenarios run eight at a time (the whole roster in ~12s); they are isolated by construction, so `-j 1` is for reading the log, not for correctness.

---

## The TUI is one struct and twenty modules

`App`'s state is a single struct on purpose — nearly every keystroke reads across concerns, and splitting it would turn field access into plumbing — but its `impl` is spread across `src/tui/*.rs`, one topic per file. A method's home is what it is *about*, not which surface it draws on. Cross-module calls are `pub(super)` and nothing else is, so each file's public surface is the list of things other topics genuinely reach for; keep it that way rather than blanket-publishing. `src/tui/mod.rs` holds the struct, the event loop, `draw`'s frame layout, and the chrome helpers everyone uses; `card`, `table` and `theme` are older App-free widget modules.

A new module under `src/tui/` must be added to `SOURCES` in `tests/glyphs.rs`, which is enforced by a test rather than remembered.

---

## Format stability

Cite keys, short keys, and `.bib` serialization are frozen: keys denote papers for life (docs/DESIGN.md), so the algorithms behind them must never drift. The committed golden vectors (`tests/golden_keys.json`, `tests/golden_format.json`) pin them — `cargo test` fails on any change, and editing the vectors is a deliberate act, never a fixture refresh.

Serialization detail worth knowing: fields are stored in reverse file order with ENTRYTYPE/ID appended last, so a parse→rewrite cycle flips the trailing (non-FIELD_ORDER) fields; files oscillate between two stable forms, both canonical.

## Releasing

Maturin `bindings = "bin"` wheels to PyPI so `pipx install astrobib` keeps working: bump the version in both `Cargo.toml` and `pyproject.toml` (not single-sourced), write the CHANGELOG entry, then pushing a `v*` tag triggers `.github/workflows/release.yml` (per-platform wheels + sdist, uploaded with the `PYPI_API_TOKEN` secret). Full procedure in PUBLISHING.md. Do not publish anything — no tag push, no `maturin publish` — without the user's explicit request.
