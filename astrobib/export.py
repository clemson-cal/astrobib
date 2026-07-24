"""Generate refs.bib from TeX source by scanning for cite keys."""
from __future__ import annotations

import re
from pathlib import Path

from .library import Library


# Matches \cite, \citep, \citet, \citealt, \citealp, \citeauthor, \citeyear,
# \citenum, \Citet, \Citep, and starred variants — all with optional pre/post notes
_CITE_RE = re.compile(r"\\[Cc]ite[a-zA-Z*]*(?:\[[^\]]*\]){0,2}\{([^}]+)\}")


def scan_tex_keys(path: Path) -> set[str]:
    text = path.read_text(errors="replace")
    keys: set[str] = set()
    for match in _CITE_RE.finditer(text):
        for key in match.group(1).split(","):
            stripped = key.strip()
            if stripped:
                keys.add(stripped)
    return keys


def scan_tex_files(paths: list[Path]) -> set[str]:
    keys: set[str] = set()
    for p in paths:
        keys |= scan_tex_keys(p)
    return keys


def export_refs(
    tex_files: list[Path],
    output: Path,
    library: Library,
) -> tuple[list[str], list[str]]:
    """Write refs.bib for the given TeX files.

    Returns (found_keys, missing_keys).
    """
    keys = scan_tex_files(tex_files)
    found: list[str] = []
    missing: list[str] = []

    bib_blocks: list[str] = []
    for key in sorted(keys):
        entry = library.get(key)
        if entry:
            bib_blocks.append(entry.path.read_text())
            found.append(key)
        else:
            missing.append(key)

    output.write_text("\n".join(bib_blocks))
    return found, missing
