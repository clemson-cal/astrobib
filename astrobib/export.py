"""Generate refs.bib from TeX source by scanning for cite keys."""
from __future__ import annotations

import re
from pathlib import Path

# Matches \cite, \citep, \citet, \citealt, \citealp, \citeauthor, \citeyear,
# \citenum, \Citet, \Citep, and starred variants — all with optional pre/post notes
_CITE_RE = re.compile(r"\\[Cc]ite[a-zA-Z*]*(?:\[[^\]]*\]){0,2}\{([^}]+)\}")

_INPUT_RE = re.compile(r"\\(?:input|include)\s*\{([^}]+)\}")
_COMMENT_RE = re.compile(r"(?<!\\)%.*")


def expand_tex_sources(roots: list[Path], base: Path) -> list[Path]:
    r"""Return roots plus every file reachable through \input/\include.

    Arguments to \input/\include are resolved against base (TeX resolves
    them against the compilation directory, normally the manuscript root),
    with .tex appended when no suffix is given.
    """
    ordered: list[Path] = []
    visited: set[Path] = set()
    stack = list(roots)
    while stack:
        path = stack.pop(0)
        resolved = path.resolve()
        if resolved in visited or not path.is_file():
            continue
        visited.add(resolved)
        ordered.append(path)
        try:
            text = _COMMENT_RE.sub("", path.read_text(errors="replace"))
        except OSError:
            continue
        for m in _INPUT_RE.finditer(text):
            name = m.group(1).strip()
            child = base / name
            if not child.suffix:
                child = child.with_suffix(".tex")
            stack.append(child)
    return ordered


def manuscript_tex_files(ms_root: Path) -> list[Path]:
    r"""TeX sources for a manuscript.

    If main.tex exists it is the sole root document; otherwise every
    top-level .tex file is a root. Roots are expanded recursively through
    \input/\include, so multi-file papers are fully scanned and stale
    sibling .tex files are ignored when main.tex is present.
    """
    main = ms_root / "main.tex"
    roots = [main] if main.is_file() else sorted(ms_root.glob("*.tex"))
    return expand_tex_sources(roots, ms_root)


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


def convert_citations(path: Path, mapper, write: bool = True) -> int:
    r"""Rewrite cite keys inside \cite... commands.

    mapper(key) returns the replacement key, or None to leave the key
    untouched. Only text inside cite-command braces is modified; spacing
    between keys is preserved. Returns the number of keys changed.
    """
    text = path.read_text(errors="replace")
    changed = 0

    def repl(m: re.Match) -> str:
        nonlocal changed
        head = m.group(0)[: m.start(1) - m.start(0)]
        pieces = m.group(1).split(",")
        out = []
        for piece in pieces:
            key = piece.strip()
            new = mapper(key) if key else None
            if new and new != key:
                changed += 1
                out.append(piece.replace(key, new, 1))
            else:
                out.append(piece)
        return head + ",".join(out) + "}"

    new_text = _CITE_RE.sub(repl, text)
    if changed and write:
        path.write_text(new_text)
    return changed


def export_refs(
    tex_files: list[Path],
    output: Path,
    library,
) -> tuple[list[str], list[str]]:
    """Write refs.bib for the given TeX files.

    Cited strings may be full keys or unambiguous prefixes; each emitted
    entry is keyed by the string actually cited, so hash suffixes never
    need to appear in the manuscript. Returns (found_keys, missing_keys);
    ambiguous prefixes count as missing.
    """
    from .library import format_bib_entry

    keys = scan_tex_files(tex_files)
    found: list[str] = []
    missing: list[str] = []

    bib_blocks: list[str] = []
    for cited in sorted(keys):
        entry = library.get(cited) or library.resolve(cited)
        if entry is not None:
            data = dict(entry.data)
            data["ID"] = cited
            bib_blocks.append(format_bib_entry(data))
            found.append(cited)
        else:
            missing.append(cited)

    output.write_text("\n".join(bib_blocks))
    return found, missing
