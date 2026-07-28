# Publishing

astrobib releases to PyPI as maturin `bindings = "bin"` wheels: the compiled Rust binary is packaged as the `astrobib` entry point, per platform (macOS arm64/x86_64, manylinux x86_64/aarch64) plus an sdist, so `pipx install astrobib` (or `pip install astrobib`) keeps working with no Rust toolchain on the user's machine.

0.5.0 is the first Rust release. It supersedes Python 0.4.0, the last release of the retired Python implementation (preserved at tag `v0.4.0`, along with its `scripts/release.py` process).

**Nothing is ever published without the user's explicit request.** Do not push a `v*` tag, run `maturin publish`/`maturin upload`, or otherwise upload to PyPI unless the user asks for a release in so many words.

---

## Release procedure

1. **Bump the version in two places** — it is not single-sourced:
   - `Cargo.toml` → `[package] version` (this is what `astrobib --version` reports, via clap)
   - `pyproject.toml` → `[project] version` (this is what PyPI sees)

   The two must match; CI wheels are named from pyproject while the binary reports Cargo's number. Patch = fixes only, minor = features.

2. **Write the CHANGELOG entry.** Add a `## X.Y.Z — YYYY-MM-DD` section at the top of `CHANGELOG.md`, with `### Added` / `### Changed` / `### Fixed` subsections as applicable and one bullet per user-visible change, in the same prose style as the existing entries.

3. **Verify locally.**

   ```bash
   cargo test --release
   maturin build --release        # wheel lands in target/wheels/
   ```

   Optionally install the wheel into a scratch venv and run `astrobib --version` / `astrobib list -n 1`.

4. **Commit, tag, push.** Commit the version bump and changelog, then:

   ```bash
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

   Pushing the `v*` tag triggers `.github/workflows/release.yml`, which builds the four platform wheels and the sdist and uploads everything to PyPI. Nothing is published until the tag is pushed.

---

## CI requirements

The publish job authenticates with a repository secret named `PYPI_API_TOKEN` (GitHub → Settings → Secrets and variables → Actions): a PyPI API token with upload permission for the `astrobib` project. It must be configured once before the first tagged release; without it the build jobs still run but the upload fails.

The same workflow also runs a plain build-and-test job (`cargo test` + `maturin build`) on pull requests; no publishing happens on that path.

---

## Manual local fallback

If CI is unavailable, wheels can be built and uploaded from a machine with the target toolchains — only on the user's explicit request:

```bash
pip install maturin
maturin publish --release          # builds for the host platform and uploads
```

`maturin publish` prompts for credentials, or reads `MATURIN_PYPI_TOKEN`. Repeat per platform (a single machine only produces its own platform's wheel), and `maturin sdist` + `maturin upload target/wheels/astrobib-X.Y.Z.tar.gz` covers the sdist. `--skip-existing` makes re-runs safe.

---

## Notes

- The retired Python package (`astrobib/`) still sits in the tree; `[tool.maturin] exclude = ["astrobib/**"]` in pyproject.toml keeps maturin from auto-detecting it as a mixed python/rust project and bundling the `.py` files into the wheel. If the directory is ever deleted, the exclude becomes a harmless no-op.
- `setup.py` and `MANIFEST.in` are leftovers of the Python build; the build backend is maturin, so they are inert.
