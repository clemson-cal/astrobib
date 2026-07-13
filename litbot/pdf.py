"""Ephemeral PDF management: fetch on demand, cache locally."""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import httpx

from .config import get_config


def cache_path(citekey: str) -> Path:
    return get_config().pdf_cache_dir / f"{citekey}.pdf"


def is_cached(citekey: str) -> bool:
    return cache_path(citekey).exists()


def fetch(citekey: str, eprint: str | None = None) -> Path | None:
    """Return cached PDF path, downloading from arXiv if needed."""
    path = cache_path(citekey)
    if path.exists():
        return path

    if not eprint:
        return None

    # Normalise old-style arXiv IDs like astro-ph/0612345
    arxiv_id = eprint.strip()
    url = f"https://arxiv.org/pdf/{arxiv_id}"

    try:
        with httpx.stream("GET", url, follow_redirects=True, timeout=60) as resp:
            resp.raise_for_status()
            content_type = resp.headers.get("content-type", "")
            if "pdf" not in content_type and "octet-stream" not in content_type:
                return None
            path.parent.mkdir(parents=True, exist_ok=True)
            with open(path, "wb") as f:
                for chunk in resp.iter_bytes():
                    f.write(chunk)
    except (httpx.HTTPError, OSError):
        return None

    return path


def open_pdf(citekey: str, eprint: str | None = None) -> bool:
    """Fetch (if needed) and open a PDF in the system viewer. Returns success."""
    path = fetch(citekey, eprint=eprint)
    if path is None:
        return False

    if sys.platform == "darwin":
        subprocess.run(["open", str(path)], check=False)
    elif sys.platform.startswith("linux"):
        subprocess.run(["xdg-open", str(path)], check=False)
    else:
        subprocess.run(["start", str(path)], shell=True, check=False)

    return True
