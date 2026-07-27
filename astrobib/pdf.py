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


def cached_keys() -> set[str]:
    """All cite keys with a cached PDF — one directory listing instead of
    per-entry stat calls."""
    try:
        return {p.stem for p in PDF_CACHE_DIR.glob("*.pdf")}
    except OSError:
        return set()


def _bibcode_from_adsurl(adsurl: str | None) -> str | None:
    if not adsurl:
        return None
    return adsurl.rstrip("/").rsplit("/", 1)[-1] or None


# ── Browser download helpers ──────────────────────────────────────────────────

def browser_open_url(*, doi: str | None = None, adsurl: str | None = None,
                     eprint: str | None = None) -> str | None:
    """Cheap candidate URL for manual PDF download (no network check)."""
    bibcode = _bibcode_from_adsurl(adsurl)
    if bibcode:
        return f"https://ui.adsabs.harvard.edu/link_gateway/{bibcode}/PUB_PDF"
    if doi:
        return f"https://doi.org/{doi}"
    if eprint:
        return f"https://arxiv.org/abs/{eprint.strip()}"
    return None


def browser_resolve_url(*, doi: str | None = None, adsurl: str | None = None,
                        eprint: str | None = None) -> str | None:
    """Best URL for manual PDF download, verified against the ADS resolver.

    The link gateway serves an error page when the record has no publisher
    PDF, so only use it after the resolver confirms one exists; otherwise
    fall back to the DOI landing page, then the arXiv abstract page.
    Makes a network call — run off the UI thread.
    """
    bibcode = _bibcode_from_adsurl(adsurl)
    if bibcode:
        from . import ads_client
        url = ads_client.resolve_pdf_url(bibcode, "PUB_PDF")
        if url:
            return url
    if doi:
        return f"https://doi.org/{doi}"
    if eprint:
        return f"https://arxiv.org/abs/{eprint.strip()}"
    return None


def browser_open(url: str) -> None:
    """Open url in the system browser."""
    if sys.platform == "darwin":
        subprocess.run(["open", url], check=False)
    elif sys.platform.startswith("linux"):
        subprocess.run(["xdg-open", url], check=False)
    else:
        subprocess.run(["start", url], shell=True, check=False)


def downloads_error() -> str | None:
    """None if ~/Downloads is readable, else a human-readable reason.

    macOS privacy protection (TCC) can deny a terminal app access to
    ~/Downloads with EPERM; pathlib's glob suppresses that error, so
    without this check the watcher would silently see an empty directory
    forever.
    """
    d = Path.home() / "Downloads"
    try:
        next(iter(d.iterdir()), None)
        return None
    except PermissionError:
        return ("macOS is blocking access to ~/Downloads — grant your terminal "
                "access in System Settings → Privacy & Security → Files and Folders")
    except FileNotFoundError:
        return "~/Downloads does not exist"
    except OSError as e:
        return f"cannot read ~/Downloads ({e.strerror or e})"


def downloads_diagnosis(before: dict[Path, tuple[int, int]]) -> str | None:
    """After a failed poll, explain what the watcher saw (None = nothing new)."""
    err = downloads_error()
    if err:
        return err
    changed = [f.name for f, sig in downloads_snapshot().items() if before.get(f) != sig]
    if changed:
        return (f"saw {', '.join(sorted(changed)[:3])} but rejected it "
                "(no %PDF header — an HTML error page?)")
    return None


def downloads_snapshot() -> dict[Path, tuple[int, int]]:
    """PDFs currently in ~/Downloads: path → (size, mtime_ns).

    Recording size and mtime lets the poller catch a download that
    overwrites a file of the same name, which a bare path-set misses.
    """
    out: dict[Path, tuple[int, int]] = {}
    try:
        for f in (Path.home() / "Downloads").iterdir():
            if f.suffix.lower() == ".pdf" and f.is_file():
                st = f.stat()
                out[f] = (st.st_size, st.st_mtime_ns)
    except OSError:
        pass
    return out


def poll_downloads(citekey: str, before: dict[Path, tuple[int, int]],
                   timeout: int = 60, cancel=None) -> Path | None:
    """Watch ~/Downloads for a new or rewritten PDF; move it to cache on arrival.

    Uses a two-poll size-stability check so we don't grab a partial download.
    cancel: a threading.Event — when set, returns None immediately.
    """
    deadline = time.monotonic() + timeout
    prev_sizes: dict[Path, int] = {}
    while time.monotonic() < deadline:
        if cancel is not None and cancel.is_set():
            return None
        time.sleep(1)
        for f, (size, _mtime) in downloads_snapshot().items():
            if before.get(f) == (size, _mtime):
                continue  # pre-existing file, unchanged
            if prev_sizes.get(f) == size and size > 0:
                try:
                    # header may follow a short preamble in the wild
                    with f.open("rb") as fh:
                        head = fh.read(1024)
                except OSError:
                    continue
                if b"%PDF" not in head:
                    continue
                dest = cache_path(citekey)
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(str(f), str(dest))
                return dest
            prev_sizes[f] = size
    return None


def import_file(citekey: str, source: Path) -> Path | None:
    """Copy a user-chosen PDF into the cache for citekey.

    Copies rather than moves — the source may be a curated original, not
    a disposable download. Returns None if the file doesn't look like a
    PDF (header check, tolerant of a short preamble).
    """
    try:
        with source.open("rb") as fh:
            head = fh.read(1024)
    except OSError:
        return None
    if b"%PDF" not in head:
        return None
    dest = cache_path(citekey)
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, dest)
    return dest


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
        url = browser_resolve_url(doi=doi, adsurl=adsurl, eprint=eprint)
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
