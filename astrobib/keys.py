"""Collision-resistant cite key generation from BibTeX data."""
from __future__ import annotations

import hashlib
import re
import unicodedata

_ARXIV_NEW_RE = re.compile(r"^(\d{2})\d{2}\.\d{4,5}(?:v\d+)?$")
_ARXIV_OLD_RE = re.compile(r"^(?:[A-Za-z.\-]+/)?(\d{2})\d{2}\d+(?:v\d+)?$")


def _year_from_arxiv_id(eprint: str) -> str | None:
    """Submission year encoded in an arXiv ID, or None if unparseable.

    New style 2405.12345 -> 2024; old style astro-ph/0501001 -> 2005
    (YY >= 91 means 19YY: arXiv started in 1991).
    """
    e = eprint.strip()
    if e.lower().startswith("arxiv:"):
        e = e[6:]
    m = _ARXIV_NEW_RE.match(e)
    if m:
        return f"20{m.group(1)}"
    m = _ARXIV_OLD_RE.match(e)
    if m:
        yy = int(m.group(1))
        return f"19{yy}" if yy >= 91 else f"20{yy:02d}"
    return None


def generate_key(data: dict) -> str:
    """Return a cite key of the form AuthorYYYYlllll.

    The suffix is 5 lowercase letters derived from SHA-256 applied to a stable
    identifier: the arXiv ID when present (survives the arXiv→published
    transition), otherwise the ADS bibcode extracted from adsurl.

    The year is derived from the stable identifier too — arXiv submission
    year, or the bibcode's leading four digits — never from the record's
    year field, so the same paper yields the same key for every user
    regardless of whether they hold the preprint or the published record.
    """
    year = data.get("year", "")
    # bibtexparser lowercases field names, so accept both spellings
    archive_prefix = (data.get("archivePrefix") or data.get("archiveprefix") or "")
    if archive_prefix.lower() == "arxiv" and data.get("eprint"):
        stable_id = data["eprint"]
        year = _year_from_arxiv_id(stable_id) or year
    else:
        adsurl = data.get("adsurl", "")
        stable_id = adsurl.rstrip("/").rsplit("/", 1)[-1] if adsurl else data.get("ID", "")
        if adsurl and stable_id[:4].isdigit():
            year = stable_id[:4]

    h = hashlib.sha256(stable_id.encode()).digest()
    suffix = "".join(chr(ord("a") + b % 26) for b in h[:5])

    author = data.get("author", "")
    first_author = author.split(" and ")[0].strip().split(",")[0].strip()
    normalized = unicodedata.normalize("NFD", first_author)
    ascii_name = "".join(
        c for c in normalized
        if unicodedata.category(c) != "Mn" and c.isalpha()
    )

    return f"{ascii_name}{year}{suffix}"
