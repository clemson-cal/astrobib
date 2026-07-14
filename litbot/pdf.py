"""Ephemeral PDF management: fetch on demand, cache locally.

Fetch chain (auto): cached → Unpaywall OA → arXiv → None.
Fetch chain (journal): cached → Unpaywall OA → Playwright browser → None.
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import httpx

from .state import PDF_CACHE_DIR, get_email

_UNPAYWALL_EMAIL_FALLBACK = "litbot@example.com"

PLAYWRIGHT_HINT = (
    "pip install playwright && playwright install chromium"
)


def _unpaywall_email() -> str:
    return get_email() or _UNPAYWALL_EMAIL_FALLBACK


def cache_path(citekey: str) -> Path:
    return PDF_CACHE_DIR / f"{citekey}.pdf"


def is_cached(citekey: str) -> bool:
    return cache_path(citekey).exists()


def playwright_ready() -> bool:
    """Return True if playwright package and Chromium browser are both available."""
    try:
        from playwright.sync_api import sync_playwright
        import os
        with sync_playwright() as p:
            return os.path.exists(p.chromium.executable_path)
    except Exception:
        return False


def _playwright_fetch(path: Path, url: str) -> Path | None:
    """Navigate to url with headless Chromium and intercept the PDF response.

    Raises RuntimeError with install instructions if playwright or browser
    are not set up — caller should surface this to the user.
    """
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        raise RuntimeError(
            f"Publisher PDFs require a browser — install with:\n  {PLAYWRIGHT_HINT}"
        )

    pdf_url: list[str | None] = [None]

    def on_response(response) -> None:
        if pdf_url[0]:
            return
        if "pdf" in response.headers.get("content-type", ""):
            pdf_url[0] = response.url

    try:
        with sync_playwright() as p:
            try:
                browser = p.chromium.launch(headless=True)
            except Exception as e:
                raise RuntimeError(
                    f"Chromium not installed — run: playwright install chromium"
                ) from e
            page = browser.new_page()
            page.on("response", on_response)
            try:
                page.goto(url, wait_until="networkidle", timeout=30_000)
            except Exception:
                pass  # timeout is fine if we already captured the PDF URL
            browser.close()
    except RuntimeError:
        raise
    except Exception:
        return None

    return _download_url(path, pdf_url[0]) if pdf_url[0] else None


def _bibcode_from_adsurl(adsurl: str) -> str | None:
    if not adsurl:
        return None
    return adsurl.rstrip("/").rsplit("/", 1)[-1] or None


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
    """Return a direct PDF URL from Unpaywall, or None if unavailable."""
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
        for loc in data.get("oa_locations") or []:
            if loc.get("url_for_pdf"):
                return loc["url_for_pdf"]
        return None
    except Exception:
        return None


SOURCE_AUTO = "auto"
SOURCE_ARXIV = "arxiv"
SOURCE_JOURNAL = "journal"


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


def fetch(citekey: str, *, eprint: str | None = None, doi: str | None = None,
          adsurl: str | None = None, source: str = SOURCE_AUTO,
          force: bool = False) -> Path | None:
    """Return cached PDF path, downloading if needed.

    source: 'auto'    — Unpaywall then arXiv fallback (no browser)
            'journal' — Unpaywall then Playwright browser fallback
            'arxiv'   — arXiv only
    force=True re-downloads even if cached.

    Raises RuntimeError (with install instructions) when source='journal'
    or source='auto' and playwright would be needed but isn't installed.
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

    if source == SOURCE_JOURNAL:
        if not doi:
            return None
        url = oa_url(doi)
        if url:
            result = _download_url(path, url)
            if result:
                return result
        # Playwright fallback: navigate via ADS link gateway or DOI redirect
        bibcode = _bibcode_from_adsurl(adsurl)
        nav_url = (
            f"https://ui.adsabs.harvard.edu/link_gateway/{bibcode}/PUB_PDF"
            if bibcode else f"https://doi.org/{doi}"
        )
        return _playwright_fetch(path, nav_url)  # raises if not installed

    # auto: Unpaywall → arXiv (no Playwright, arXiv is reliable)
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
