"""Ephemeral PDF management: fetch on demand, cache locally.

Sources:
  auto    — Unpaywall OA → arXiv (fully automatic)
  arxiv   — arXiv only
  browser — open system browser, watch ~/Downloads for new PDF
"""
from __future__ import annotations

import shutil
import subprocess
import sys
import time
from pathlib import Path

import httpx

from .state import PDF_CACHE_DIR, get_email

_UNPAYWALL_EMAIL_FALLBACK = "litbot@example.com"

SOURCE_AUTO = "auto"
SOURCE_ARXIV = "arxiv"
SOURCE_BROWSER = "browser"


def _unpaywall_email() -> str:
    return get_email() or _UNPAYWALL_EMAIL_FALLBACK


def cache_path(citekey: str) -> Path:
    return PDF_CACHE_DIR / f"{citekey}.pdf"


def is_cached(citekey: str) -> bool:
    return cache_path(citekey).exists()


# ── Unpaywall ─────────────────────────────────────────────────────────────────

def oa_url_with_detail(doi: str) -> tuple[str | None, dict | None]:
    """Return (direct_pdf_url, raw_response) from Unpaywall."""
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
    url, _ = oa_url_with_detail(doi)
    return url


# ── Browser download helpers ──────────────────────────────────────────────────

def browser_open_url(*, doi: str | None = None, adsurl: str | None = None) -> str | None:
    """Return the URL to open in the system browser for manual PDF download."""
    if adsurl:
        bibcode = adsurl.rstrip("/").rsplit("/", 1)[-1]
        if bibcode:
            return f"https://ui.adsabs.harvard.edu/link_gateway/{bibcode}/PUB_PDF"
    if doi:
        return f"https://doi.org/{doi}"
    return None


def browser_open(url: str) -> None:
    """Open url in the system browser."""
    if sys.platform == "darwin":
        subprocess.run(["open", url], check=False)
    elif sys.platform.startswith("linux"):
        subprocess.run(["xdg-open", url], check=False)
    else:
        subprocess.run(["start", url], shell=True, check=False)


def downloads_snapshot() -> set[Path]:
    """Current set of PDF files in ~/Downloads."""
    d = Path.home() / "Downloads"
    return {f for f in d.glob("*.pdf")} if d.exists() else set()


def poll_downloads(citekey: str, before: set[Path], timeout: int = 60) -> Path | None:
    """Watch ~/Downloads for a new PDF not in before; move it to cache on arrival.

    Uses a two-poll size-stability check so we don't grab a partial Chrome download.
    """
    deadline = time.monotonic() + timeout
    prev_sizes: dict[Path, int] = {}
    while time.monotonic() < deadline:
        time.sleep(1)
        d = Path.home() / "Downloads"
        current = {f for f in d.glob("*.pdf")} if d.exists() else set()
        for f in current - before:
            try:
                size = f.stat().st_size
            except OSError:
                continue
            if prev_sizes.get(f) == size and size > 0:
                dest = cache_path(citekey)
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(str(f), str(dest))
                return dest
            prev_sizes[f] = size
    return None


# ── HTTP download ─────────────────────────────────────────────────────────────

def _download_url(path: Path, url: str) -> Path | None:
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
        return path
    except (httpx.HTTPError, OSError):
        return None


# ── Public API ────────────────────────────────────────────────────────────────

def fetch(citekey: str, *, eprint: str | None = None, doi: str | None = None,
          adsurl: str | None = None, source: str = SOURCE_AUTO,
          force: bool = False) -> Path | None:
    """Return cached PDF path, downloading if needed.

    source='auto'    — Unpaywall then arXiv fallback
    source='arxiv'   — arXiv only
    source='browser' — open system browser, poll ~/Downloads (blocks until found or timeout)
    force=True re-downloads even if cached.
    """
    path = cache_path(citekey)
    if path.exists() and not force:
        return path
    if path.exists() and force:
        path.unlink()

    if source == SOURCE_ARXIV:
        if not eprint:
            return None
        return _download_url(path, f"https://arxiv.org/pdf/{eprint.strip()}")

    if source == SOURCE_BROWSER:
        url = browser_open_url(doi=doi, adsurl=adsurl)
        if not url:
            return None
        before = downloads_snapshot()
        browser_open(url)
        return poll_downloads(citekey, before)

    # auto: Unpaywall → arXiv
    if doi:
        url = oa_url(doi)
        if url:
            result = _download_url(path, url)
            if result:
                return result
    if eprint:
        return _download_url(path, f"https://arxiv.org/pdf/{eprint.strip()}")
    return None


def open_pdf(citekey: str, *, eprint: str | None = None, doi: str | None = None,
             adsurl: str | None = None, source: str = SOURCE_AUTO,
             force: bool = False) -> bool:
    path = fetch(citekey, eprint=eprint, doi=doi, adsurl=adsurl,
                 source=source, force=force)
    if path is None:
        return False
    if sys.platform == "darwin":
        subprocess.run(["open", str(path)], check=False)
    elif sys.platform.startswith("linux"):
        subprocess.run(["xdg-open", str(path)], check=False)
    else:
        subprocess.run(["start", str(path)], shell=True, check=False)
    return True
