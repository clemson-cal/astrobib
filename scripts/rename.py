#!/usr/bin/env python3
"""Rename this project: rewrites the name in all tracked files and
git-mv's the package directory.

Usage:
    python scripts/rename.py OLD NEW      # current name, desired name

Handles three case variants: lowercase (module/CLI name), Capitalized
(class-name prefix, e.g. AstrobibApp), and UPPERCASE (env vars, e.g.
ASTROBIB_STATE_DIR).

After running, finish with:
    .venv/bin/pip uninstall -y OLD && .venv/bin/pip install -e .
    rm -f .venv/bin/OLD
    mv ~/.local/share/OLD ~/.local/share/NEW    # user data (if it exists)
    mv ~/.cache/OLD ~/.cache/NEW                # caches (if it exists)
and optionally rename the repo directory itself.
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def tracked_files(root: Path) -> list[Path]:
    out = subprocess.run(["git", "ls-files"], cwd=root, capture_output=True,
                         text=True, check=True).stdout
    return [root / line for line in out.splitlines() if line]


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 1
    old, new = sys.argv[1].lower(), sys.argv[2].lower()
    if not (old.isidentifier() and new.isidentifier()):
        print("OLD and NEW must be valid Python identifiers (package names).")
        return 1
    root = Path(__file__).resolve().parent.parent
    variants = [
        (old, new),
        (old.capitalize(), new.capitalize()),
        (old.upper(), new.upper()),
    ]

    changed = 0
    for path in tracked_files(root):
        if not path.is_file() or path.is_symlink():
            continue
        try:
            text = path.read_text()
        except (UnicodeDecodeError, OSError):
            continue  # binary or unreadable
        replaced = text
        for a, b in variants:
            replaced = replaced.replace(a, b)
        if replaced != text:
            path.write_text(replaced)
            changed += 1
            print(f"  rewrote {path.relative_to(root)}")

    pkg_old, pkg_new = root / old, root / new
    if pkg_old.is_dir():
        subprocess.run(["git", "mv", old, new], cwd=root, check=True)
        print(f"  git mv {old}/ {new}/")

    print(f"\n{changed} file(s) rewritten.")
    print("\nFinish with:")
    print(f"  .venv/bin/pip uninstall -y {old} && .venv/bin/pip install -e .")
    print(f"  rm -f .venv/bin/{old}")
    print(f"  mv ~/.local/share/{old} ~/.local/share/{new}    # if it exists")
    print(f"  mv ~/.cache/{old} ~/.cache/{new}                # if it exists")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
