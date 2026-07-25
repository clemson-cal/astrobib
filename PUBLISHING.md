# Publishing astrobib to PyPI

## One-time setup

- Create accounts on [test.pypi.org](https://test.pypi.org) and
  [pypi.org](https://pypi.org), and generate API tokens for each
  (Account settings → API tokens).
- Store the tokens in `~/.pypirc`:

  ```ini
  [testpypi]
  username = __token__
  password = pypi-...

  [pypi]
  username = __token__
  password = pypi-...
  ```

- Build tooling goes in a scratch venv, never the project `.venv`:

  ```bash
  /opt/homebrew/bin/python3.11 -m venv ~/.venvs/pkg
  ~/.venvs/pkg/bin/pip install build twine
  ```

## Release steps

1. **Bump the version** in `astrobib/__init__.py` (`__version__` is the
   single source; pyproject.toml reads it dynamically). Commit.

2. **Build** from a clean tree:

   ```bash
   rm -rf dist build
   ~/.venvs/pkg/bin/python -m build
   ```

   This produces `dist/astrobib-X.Y.Z.tar.gz` and
   `dist/astrobib-X.Y.Z-py3-none-any.whl`. The setup.py build hooks
   replace the `astrobib/help.md` symlink with a real copy of README.md in
   both artifacts — the symlink stays in the repo for development only.

3. **Check metadata**:

   ```bash
   ~/.venvs/pkg/bin/twine check dist/*
   ```

4. **Smoke-test the wheel** in a throwaway venv:

   ```bash
   python3.11 -m venv /tmp/astrobib-test
   /tmp/astrobib-test/bin/pip install dist/astrobib-*.whl
   /tmp/astrobib-test/bin/astrobib --help
   /tmp/astrobib-test/bin/python -c "import astrobib.tui.app; from astrobib.tui.help_screen import _load_help; assert _load_help()"
   ```

5. **Dry run on TestPyPI**:

   ```bash
   ~/.venvs/pkg/bin/twine upload --repository testpypi dist/*
   ```

   Then verify the listing at <https://test.pypi.org/project/astrobib/> and
   install from it (dependencies come from real PyPI):

   ```bash
   /tmp/astrobib-test/bin/pip install --index-url https://test.pypi.org/simple/ \
       --extra-index-url https://pypi.org/simple/ --force-reinstall astrobib
   ```

   Note: TestPyPI uploads are immutable per version, like PyPI. If a dry run
   needs a fix, use a `.devN` suffix (e.g. `0.1.0.dev1`) for the test upload.

6. **Upload to PyPI**:

   ```bash
   ~/.venvs/pkg/bin/twine upload dist/*
   ```

7. **Tag the release** and push:

   ```bash
   git tag -a vX.Y.Z -m "astrobib X.Y.Z"
   git push origin main vX.Y.Z
   ```

A version number can never be reused on PyPI, even after deleting a release —
when in doubt, test on TestPyPI first.
