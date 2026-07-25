#!/usr/bin/env python3
"""Release astrobib to PyPI.

Usage:
    .venv/bin/python scripts/release.py X.Y.Z [--dry-run]

Prerequisites:
    - CHANGELOG.md contains a "## X.Y.Z" entry (may be uncommitted;
      the script commits it together with the version bump)
    - working tree otherwise clean, on main
    - build and twine installed in the venv running this script
    - PyPI token in the [astrobib] section of ~/.pypirc

Steps: validate -> bump __version__ -> build -> twine check ->
[stop here if --dry-run] -> commit "Release X.Y.Z" -> twine upload ->
tag vX.Y.Z -> push main and tag -> clean artifacts.

Ordering guarantees: nothing is committed, uploaded, tagged, or pushed
until the build and twine check pass; the tag and push happen only
after a successful upload.
"""
from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION_FILE = ROOT / "astrobib" / "__init__.py"


def run(*cmd: str, capture: bool = False) -> str:
    result = subprocess.run(cmd, cwd=ROOT, check=True, text=True,
                            capture_output=capture)
    return result.stdout if capture else ""


def die(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    raise SystemExit(1)


def main() -> int:
    args = [a for a in sys.argv[1:] if a != "--dry-run"]
    dry_run = "--dry-run" in sys.argv[1:]
    if len(args) != 1 or not re.fullmatch(r"\d+\.\d+\.\d+", args[0]):
        print(__doc__)
        return 1
    version = args[0]

    # ── validate ──────────────────────────────────────────────────────
    for mod in ("build", "twine"):
        try:
            __import__(mod)
        except ImportError:
            die(f"{mod} not installed — run: pip install build twine")
    branch = run("git", "rev-parse", "--abbrev-ref", "HEAD", capture=True).strip()
    if branch != "main":
        die(f"on branch {branch!r}, expected main")
    dirty = [l for l in run("git", "status", "--porcelain", capture=True).splitlines()
             if l.strip() and not l.startswith("??") and not l.endswith("CHANGELOG.md")]
    if dirty:
        die("working tree not clean (only CHANGELOG.md may be modified):\n"
            + "\n".join(dirty))
    if f"## {version}" not in (ROOT / "CHANGELOG.md").read_text():
        die(f"CHANGELOG.md has no '## {version}' entry — write it first")
    tags = run("git", "tag", "-l", f"v{version}", capture=True).strip()
    if tags:
        die(f"tag v{version} already exists")
    current = re.search(r'__version__ = "([^"]+)"', VERSION_FILE.read_text()).group(1)
    print(f"releasing {current} -> {version}" + (" (dry run)" if dry_run else ""))

    # ── bump + build + check ──────────────────────────────────────────
    VERSION_FILE.write_text(re.sub(r'__version__ = "[^"]+"',
                                   f'__version__ = "{version}"',
                                   VERSION_FILE.read_text()))
    for d in ("dist", "build"):
        shutil.rmtree(ROOT / d, ignore_errors=True)
    try:
        run(sys.executable, "-m", "build")
        artifacts = sorted(str(p) for p in (ROOT / "dist").iterdir())
        run(sys.executable, "-m", "twine", "check", "--strict", *artifacts)
    except Exception:
        run("git", "checkout", "--", str(VERSION_FILE))
        raise

    if dry_run:
        run("git", "checkout", "--", str(VERSION_FILE))
        for d in ("dist", "build"):
            shutil.rmtree(ROOT / d, ignore_errors=True)
        print(f"dry run OK — {version} builds and passes twine check")
        return 0

    # ── commit, upload, tag, push ─────────────────────────────────────
    run("git", "add", str(VERSION_FILE), "CHANGELOG.md")
    run("git", "commit", "-m", f"Release {version}")
    run(sys.executable, "-m", "twine", "upload", "--repository", "astrobib",
        *artifacts)
    run("git", "tag", f"v{version}")
    run("git", "push", "origin", "main", f"v{version}")
    for d in ("dist", "build"):
        shutil.rmtree(ROOT / d, ignore_errors=True)
    print(f"released: https://pypi.org/project/astrobib/{version}/")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
