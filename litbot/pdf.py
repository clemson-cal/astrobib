"""Ephemeral PDF management: fetch on demand, cache locally.

Sources:
  auto    — ADS OA_PDF resolver → arXiv direct (fully automatic)
  arxiv   — arXiv direct download only
  browser — open system browser, watch ~/Downloads for new PDF
"""
from __future__ import annotations

import shutil
import subprocess
import sys
import time
from pathlib import Path

import httpx

from .state import PDF_CACHE_DIR

SOURCE_AUTO = "auto"
SOURCE_ARXIV = "arxiv"
SOURCE_OA = "oa"
SOURCE_BROWSER = "browser"


def cache_path(citekey: str) -> Path:
    return PDF_CACHE_DIR / f"{citekey}.pdf"


def is_cached(citekey: str) -> bool:
    return cache_path(citekey).exists()


def _bibcode_from_adsurl(adsurl: str | None) -> str | None:
    if not adsurl:
        return None
    return adsurl.rstrip("/").rsplit("/", 1)[-1] or None


# ── Browser download helpers ──────────────────────────────────────────────────

def browser_open_url(*, doi: str | None = None, adsurl: str | None = None) -> str | None:
    """Return the URL to open in the system browser for manual PDF download."""
    bibcode = _bibcode_from_adsurl(adsurl)
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


def poll_downloads(citekey: str, before: set[Path], timeout: int = 60,
                   cancel=None) -> Path | None:
    """Watch ~/Downloads for a new PDF not in before; move it to cache on arrival.

    Uses a two-poll size-stability check so we don't grab a partial download.
    cancel: a threading.Event — when set, returns None immediately.
    """
    deadline = time.monotonic() + timeout
    prev_sizes: dict[Path, int] = {}
    while time.monotonic() < deadline:
        if cancel is not None and cancel.is_set():
            return None
        time.sleep(1)
        d = Path.home() / "Downloads"
        current = {f for f in d.glob("*.pdf")} if d.exists() else set()
        for f in current - before:
            try:
                size = f.stat().st_size
            except OSError:
                continue
            if prev_sizes.get(f) == size and size > 0:
                try:
                    if f.read_bytes()[:4] != b"%PDF":
                        continue
                except OSError:
                    continue
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

    source='auto'    — ADS OA_PDF resolver then arXiv fallback
    source='arxiv'   — arXiv only
    source='browser' — open system browser, poll ~/Downloads
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

    if source == SOURCE_OA:
        bibcode = _bibcode_from_adsurl(adsurl)
        if not bibcode:
            return None
        from . import ads_client
        url = ads_client.resolve_pdf_url(bibcode, "OA_PDF")
        if not url:
            return None
        return _download_url(path, url)

    if source == SOURCE_BROWSER:
        url = browser_open_url(doi=doi, adsurl=adsurl)
        if not url:
            return None
        before = downloads_snapshot()
        browser_open(url)
        return poll_downloads(citekey, before)

    # auto: ADS OA_PDF → arXiv
    bibcode = _bibcode_from_adsurl(adsurl)
    if bibcode:
        from . import ads_client
        url = ads_client.resolve_pdf_url(bibcode, "OA_PDF")
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
