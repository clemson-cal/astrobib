# Publishing

astrobib releases to PyPI as maturin `bindings = "bin"` wheels: the compiled Rust binary is packaged as the `astrobib` entry point, per platform (macOS arm64/x86_64, manylinux x86_64/aarch64) plus an sdist, so `pipx install astrobib` (or `pip install astrobib`) keeps working with no Rust toolchain on the user's machine.

**Nothing is ever published without the user's explicit request.** Do not push a `v*` tag, run `maturin publish`/`maturin upload`, or otherwise upload to PyPI unless the user asks for a release in so many words.

---

## Release procedure

1. **Bump the version in two places** — it is not single-sourced:
   - `Cargo.toml` → `[package] version` (this is what `astrobib --version` reports, via clap)
   - `pyproject.toml` → `[project] version` (this is what PyPI sees)

   The two must match; CI wheels are named from pyproject while the binary reports Cargo's number. Patch = fixes only, minor = features.

2. **Write the CHANGELOG entry.** Add a `## X.Y.Z — YYYY-MM-DD` section at the top of `docs/CHANGELOG.md`, with `### Added` / `### Changed` / `### Fixed` subsections as applicable and one bullet per user-visible change, in the same prose style as the existing entries.

   Check that what is at the top is genuinely unreleased before you date it: `git log --oneline $(git describe --tags --abbrev=0)..HEAD` is the release's real contents, and `git show vX.Y.Z:docs/CHANGELOG.md | head` says what the last one already claimed. 0.19.0 was cut with a `## Unreleased` heading that a later commit had written over the shipped `## 0.18.0` section, so two already-released bullets were sitting in the pending pile waiting to be announced twice. A heading named for a version is a fact about a tag; only the section above the newest tag's is yours to edit.

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

The action versions are Node 24 throughout — `checkout` and `upload-artifact` at v7, `download-artifact` at v8 — and 0.20.0 was the first tag to run them. All five build jobs and the publish job passed with no deprecation warning about the runtime, so that question is settled; do not "fix" them back to v4/v5, where two of the three still declare `node20`.

**The publish step's own command is deprecated.** `maturin upload` and `maturin publish` are both slated for removal ([PyO3/maturin#2334](https://github.com/PyO3/maturin/issues/2334)), and the 0.20.0 run said so as an annotation while still succeeding. The replacement is `pypa/gh-action-pypi-publish` on the `dist/` the download step already assembles — the same artifacts, a different uploader, and trusted publishing available instead of `PYPI_API_TOKEN` if that is wanted at the same time. Nothing is urgent until a maturin release actually drops the command, but a release is the worst moment to discover it has: change it between releases, not during one.

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

