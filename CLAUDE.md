# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Read [docs/DESIGN.md](docs/DESIGN.md) before adding features or changing data formats. It is the authoritative statement of design constraints.

**Parallel agents: one writer per working tree.** Concurrent agent sessions must not share this working tree — spawn writing subagents with worktree isolation (`isolation: "worktree"` / `git worktree add`) and integrate through commits. When committing here, stage explicit paths (never `git add -A`) and review `git status --short` first: another session's uncommitted work may be present.

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
