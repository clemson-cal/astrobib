"""Ephemeral PDF management: fetch on demand, cache locally.

Fetch chain: cached → arXiv (eprint) → Unpaywall OA (doi) → None.
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import httpx

from .state import PDF_CACHE_DIR, get_email

_UNPAYWALL_EMAIL_FALLBACK = "litbot@example.com"


def _unpaywall_email() -> str:
    return get_email() or _UNPAYWALL_EMAIL_FALLBACK


def cache_path(citekey: str) -> Path:
    return PDF_CACHE_DIR / f"{citekey}.pdf"


def is_cached(citekey: str) -> bool:
    return cache_path(citekey).exists()


def oa_url_with_detail(doi: str) -> tuple[str | None, dict | None]:
    """Like oa_url but also returns the raw Unpaywall response dict."""
    if not doi:
        return None, None
    try:
        resp = httpx.get(
            f"https://api.unpaywall.org/v2/{doi}",
            params={"email": _unpaywall_email()},
            timeout=10,
            follow_redirects=True,
        )
        if resp.status_code != 200:
            return None, None
        data = resp.json()
        best = data.get("best_oa_location") or {}
        if best.get("url_for_pdf"):
            return best["url_for_pdf"], data
        for loc in data.get("oa_locations") or []:
            if loc.get("url_for_pdf"):
                return loc["url_for_pdf"], data
        return None, data
    except Exception:
        return None, None


def oa_url(doi: str) -> str | None:
    """Return a direct PDF URL from Unpaywall, or None if unavailable.

    Checks best_oa_location.url_for_pdf first, then scans all oa_locations
    for any direct PDF link, since Unpaywall sometimes omits url_for_pdf on
    the best location even when other locations have it.
    """
    if not doi:
        return None
    try:
        resp = httpx.get(
            f"https://api.unpaywall.org/v2/{doi}",
            params={"email": _unpaywall_email()},
            timeout=10,
            follow_redirects=True,
        )
        if resp.status_code != 200:
            return None
        data = resp.json()
        best = data.get("best_oa_location") or {}
        if best.get("url_for_pdf"):
            return best["url_for_pdf"]
        # Scan all OA locations for any direct PDF link
        for loc in data.get("oa_locations") or []:
            if loc.get("url_for_pdf"):
                return loc["url_for_pdf"]
        return None
    except Exception:
        return None


def fetch(citekey: str, *, eprint: str | None = None, doi: str | None = None) -> Path | None:
    """Return cached PDF path, downloading if needed.

    Tries arXiv first (if eprint given), then Unpaywall (if doi given).
    """
    path = cache_path(citekey)
    if path.exists():
        return path

    if doi:
        url: str | None = oa_url(doi)
        if url is None and eprint:
            url = f"https://arxiv.org/pdf/{eprint.strip()}"
    elif eprint:
        url = f"https://arxiv.org/pdf/{eprint.strip()}"
    else:
        return None

    if not url:
        return None

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


def open_pdf(citekey: str, *, eprint: str | None = None, doi: str | None = None) -> bool:
    path = fetch(citekey, eprint=eprint, doi=doi)
    if path is None:
        return False

    if sys.platform == "darwin":
        subprocess.run(["open", str(path)], check=False)
    elif sys.platform.startswith("linux"):
        subprocess.run(["xdg-open", str(path)], check=False)
    else:
        subprocess.run(["start", str(path)], shell=True, check=False)

    return True
