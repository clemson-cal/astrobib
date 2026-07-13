"""Bib database management: clone, pull, push via git subprocess."""
from __future__ import annotations

import subprocess
from pathlib import Path


class DatabaseError(RuntimeError):
    pass


def _git(db_path: Path, *args: str, check: bool = False) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", *args],
        cwd=db_path,
        capture_output=True,
        text=True,
        check=check,
    )


def clone(url: str, dest: Path) -> None:
    result = subprocess.run(
        ["git", "clone", url, str(dest)],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise DatabaseError(result.stderr.strip())


def init_empty(dest: Path) -> None:
    """Create a new empty bib database at dest."""
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "bib").mkdir(exist_ok=True)
    (dest / "bib" / ".gitkeep").touch()
    (dest / "keywords.yaml").write_text(
        "# UAT-aligned keyword navigation for this group.\n"
        "# See https://astrothesaurus.org for UAT concept labels.\n\n"
        "focus:\n"
        "  - Computational methods\n"
        "  - Compact objects\n\n"
        "_local: []\n"
    )
    _git(dest, "init", check=True)
    _git(dest, "add", "-A", check=True)
    _git(dest, "commit", "-m", "Initialize bib database", check=True)


def pull(db_path: Path) -> str:
    r = _git(db_path, "pull")
    out = (r.stdout + r.stderr).strip()
    if r.returncode != 0:
        raise DatabaseError(out)
    return out


def push(db_path: Path, message: str = "Update library") -> str:
    _git(db_path, "add", "-A")
    commit = _git(db_path, "commit", "-m", message)
    if commit.returncode != 0:
        stdout = commit.stdout.strip()
        if "nothing to commit" in stdout:
            return "Nothing to commit."
        raise DatabaseError(commit.stderr.strip() or stdout)
    push_r = _git(db_path, "push")
    out = (push_r.stdout + push_r.stderr).strip()
    if push_r.returncode != 0:
        raise DatabaseError(out)
    return out or "Pushed."


def status(db_path: Path) -> str:
    r = _git(db_path, "status", "--short")
    return r.stdout.strip()


def log(db_path: Path, n: int = 10) -> str:
    r = _git(db_path, "log", f"-{n}", "--oneline")
    return r.stdout.strip()


def remote_url(db_path: Path) -> str:
    r = _git(db_path, "remote", "get-url", "origin")
    return r.stdout.strip()
