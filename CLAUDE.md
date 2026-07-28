# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Read [DESIGN.md](DESIGN.md) before adding features or changing data formats. It is the authoritative statement of design constraints. [RUST.md](RUST.md) tracks implementation status.

**Parallel agents: one writer per working tree.** Concurrent agent sessions must not share this working tree — spawn writing subagents with worktree isolation (`isolation: "worktree"` / `git worktree add`) and integrate through commits. When committing here, stage explicit paths (never `git add -A`) and review `git status --short` first: another session's uncommitted work may be present.

---

## Development commands

```bash
cargo build --release        # binary at target/release/astrobib
cargo test                   # includes golden parity vectors
cargo run --release          # TUI
cargo run --release -- list  # CLI
tests/tui/run.py             # TUI integration scenarios (see tests/tui/README.md)
```

Verify TUI changes headlessly with the committed pyte pty harness (`tests/tui/run.py`): it drives the real binary in a pseudo-terminal against a scratch `ASTROBIB_LIBRARY` / `ASTROBIB_STATE_DIR` built from `tests/tui/fixtures/` — never the real library — and reconstructs screens with pyte rather than grepping raw bytes. It bootstraps its own venv; add new scenarios under `tests/tui/scenarios/`.

---

## History and parity

The Python implementation this port derives from lives at tag `v0.4.0` (also the latest Python release on PyPI). Golden test vectors (`tests/golden_keys.json`, `tests/golden_format.json`) were generated from it; `scripts/regen-golden.py` re-derives them from a temporary v0.4.0 worktree and diffs against the committed files (`--write` to update, never auto-committed). Anything both implementations wrote had to be byte-identical: cite keys, short keys, `.bib` serialization. The bib database format is the contract — see DESIGN.md.

Format quirk, faithfully reproduced: bibtexparser v1 stored fields in reverse file order, so every parse→rewrite cycle flips the trailing (non-FIELD_ORDER) fields; files oscillate between two stable forms.

## Releasing

Not wired yet. Plan: maturin `bindings = "bin"` wheels to PyPI (per-platform, CI matrix) so `pipx install astrobib` keeps working. Do not publish anything without the user's explicit request.
