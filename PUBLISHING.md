# Publishing

Releases are not wired for the Rust implementation yet. The plan (see CLAUDE.md): maturin `bindings = "bin"` wheels — the compiled binary packaged per platform (macOS arm64/x86_64, manylinux x86_64/aarch64) so `pipx install astrobib` continues to work — published from CI on tag push, starting at 0.5.0.

The Python 0.4.0 release process is preserved at tag `v0.4.0` (scripts/release.py there).
