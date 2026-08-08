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

- **State-file versioning is unresolved.** `docs/DESIGN.md` "Config and app state versioning" contradicts "Future-proof by dumbness", and the code follows neither. Noted in DESIGN.md itself; settle it the next time a state file changes shape.
- **`src/tags.rs` still writes tag files with a plain `fs::write`.** Every other curated file goes through `library::write_atomic`. Tag files are git-versioned and so recoverable, which is why they were left — but they are the last truncate-then-write on data a user curates.
- **`load_cached_articles` re-reads and re-parses all of `query_cache.json` per tab** (`src/tabs.rs`, called per tab from `restore_tabs` in `src/tui.rs`). Invisible at a handful of tabs; now that a session shows both homes' queries at once, larger sets are likelier. Measure before fixing.
- **The `@` panel does not name the active manuscript.** It reports where the ADS token came from and what it has spent; which database you are pointed at is inferable only from the Manuscript capsule being present.

---


## Development commands

```bash
cargo build --release        # binary at target/release/astrobib
cargo test                   # includes the golden format/key vectors
cargo run --release          # TUI
cargo run --release -- list  # CLI
tests/tui/run.py             # TUI integration scenarios (see tests/tui/README.md)
```

Verify TUI changes headlessly with the committed pyte pty harness (`tests/tui/run.py`): it drives the real binary in a pseudo-terminal against a scratch `ASTROBIB_LIBRARY` / `ASTROBIB_STATE_DIR` built from `tests/tui/fixtures/` — never the real library — and reconstructs screens with pyte rather than grepping raw bytes. It bootstraps its own venv; add new scenarios under `tests/tui/scenarios/`.

---

## Format stability

Cite keys, short keys, and `.bib` serialization are frozen: keys denote papers for life (docs/DESIGN.md), so the algorithms behind them must never drift. The committed golden vectors (`tests/golden_keys.json`, `tests/golden_format.json`) pin them — `cargo test` fails on any change, and editing the vectors is a deliberate act, never a fixture refresh.

Serialization detail worth knowing: fields are stored in reverse file order with ENTRYTYPE/ID appended last, so a parse→rewrite cycle flips the trailing (non-FIELD_ORDER) fields; files oscillate between two stable forms, both canonical.

## Releasing

Maturin `bindings = "bin"` wheels to PyPI so `pipx install astrobib` keeps working: bump the version in both `Cargo.toml` and `pyproject.toml` (not single-sourced), write the CHANGELOG entry, then pushing a `v*` tag triggers `.github/workflows/release.yml` (per-platform wheels + sdist, uploaded with the `PYPI_API_TOKEN` secret). Full procedure in PUBLISHING.md. Do not publish anything — no tag push, no `maturin publish` — without the user's explicit request.
