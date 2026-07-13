"""Collision-resistant cite key generation from BibTeX data."""
from __future__ import annotations

import hashlib
import unicodedata


def generate_key(data: dict) -> str:
    """Return a cite key of the form AuthorYYYYhhhh.

    The suffix is the first 5 hex chars of SHA-256 applied to a stable
    identifier: the arXiv ID when present (survives the arXiv→published
    transition), otherwise the ADS bibcode extracted from adsurl.
    """
    if data.get("archivePrefix", "").lower() == "arxiv" and data.get("eprint"):
        stable_id = data["eprint"]
    else:
        adsurl = data.get("adsurl", "")
        stable_id = adsurl.rstrip("/").rsplit("/", 1)[-1] if adsurl else data.get("ID", "")

    suffix = hashlib.sha256(stable_id.encode()).hexdigest()[:5]

    author = data.get("author", "")
    first_author = author.split(" and ")[0].strip().split(",")[0].strip()
    normalized = unicodedata.normalize("NFD", first_author)
    ascii_name = "".join(
        c for c in normalized
        if unicodedata.category(c) != "Mn" and c.isalpha()
    )

    year = data.get("year", "")
    return f"{ascii_name}{year}{suffix}"
