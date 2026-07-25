"""Build hooks for astrobib.

In the git checkout, ``astrobib/help.md`` is a symlink to ``../README.md`` so
the TUI help screen and the README stay in sync during development. Symlinks
do not survive sdists/wheels reliably (and break entirely on filesystems
without symlink support), so both build commands below replace the symlink
with a real copy of README.md in their output trees:

- ``sdist``: after the release tree is assembled, ``astrobib/help.md`` is
  rewritten as a regular file, so the tarball never contains a symlink.
- ``build_py``: the file placed in ``build/lib`` (and hence in the wheel) is
  rewritten as a regular file with the README content.

All project metadata lives in pyproject.toml; this file exists only for the
command overrides.
"""

from __future__ import annotations

import shutil
from pathlib import Path

from setuptools import setup
from setuptools.command.build_py import build_py as _build_py
from setuptools.command.sdist import sdist as _sdist

HERE = Path(__file__).resolve().parent
README = HERE / "README.md"


def _materialize_help(target: Path) -> None:
    """Replace *target* (astrobib/help.md, possibly a symlink) with a real
    copy of README.md."""
    if target.is_symlink() or target.exists():
        target.unlink()
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(README, target)


class build_py(_build_py):
    def run(self) -> None:
        super().run()
        _materialize_help(Path(self.build_lib) / "astrobib" / "help.md")


class sdist(_sdist):
    def make_release_tree(self, base_dir: str, files) -> None:
        super().make_release_tree(base_dir, files)
        _materialize_help(Path(base_dir) / "astrobib" / "help.md")


setup(cmdclass={"build_py": build_py, "sdist": sdist})
