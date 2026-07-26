"""User-local app state: library path, ADS token, cache locations."""
from __future__ import annotations

import json
import os
from pathlib import Path

_BASE = Path(os.environ.get("ASTROBIB_STATE_DIR", Path.home() / ".local" / "share" / "astrobib"))

LIBRARY_DIR = _BASE / "library"
STATE_FILE = _BASE / "state.json"
UAT_CACHE = Path.home() / ".cache" / "astrobib" / "uat.json"
PDF_CACHE_DIR = Path.home() / ".cache" / "astrobib" / "pdfs"
PARSE_CACHE_DIR = Path.home() / ".cache" / "astrobib" / "parsecache"

SCHEMA_VERSION = 1

_state: dict | None = None


def _load() -> dict:
    global _state
    if _state is not None:
        return _state
    if STATE_FILE.exists():
        try:
            data = json.loads(STATE_FILE.read_text())
            if data.get("version") == SCHEMA_VERSION:
                _state = data
                return _state
        except Exception:
            pass
    _state = {"version": SCHEMA_VERSION, "ads_token": None}
    return _state


def _save(state: dict) -> None:
    STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
    STATE_FILE.write_text(json.dumps(state, indent=2))


def get_token() -> str | None:
    """Return ADS token from env var or saved state."""
    return os.environ.get("ADS_API_TOKEN") or _load().get("ads_token") or None


def set_token(token: str) -> None:
    """Persist the ADS token to app state."""
    global _state
    state = _load()
    state["ads_token"] = token.strip()
    _save(state)
    _state = state


_library_override: Path | None = None


def set_library_path(path: Path | str) -> None:
    """Override the personal library root for this process (--library flag)."""
    global _library_override
    _library_override = Path(path).expanduser()


def _library_root() -> Path:
    if _library_override is not None:
        return _library_override
    env = os.environ.get("ASTROBIB_LIBRARY")
    return Path(env).expanduser() if env else LIBRARY_DIR


def library_source() -> str:
    """Where the active library path came from: 'flag', 'env', or 'default'."""
    if _library_override is not None:
        return "flag"
    return "env" if os.environ.get("ASTROBIB_LIBRARY") else "default"


def get_library_path() -> Path:
    """Return the library root, creating bib/ if needed.

    The root is --library if given, else $ASTROBIB_LIBRARY, else the
    default under the state dir. Caches (PDF, parse, UAT) and state.json
    are unaffected by the override — they are machine-local, not library
    data.
    """
    path = _library_root()
    (path / "bib").mkdir(parents=True, exist_ok=True)
    return path


def find_manuscript_db(start: Path | None = None) -> Path | None:
    """Walk up from start (default cwd) to find a manuscript database.

    A manuscript database is any directory containing bib/ alongside .git —
    typically the manuscript's own repo. Returns None if not inside one.
    """
    library_root = _library_root().resolve()
    path = (start or Path.cwd()).resolve()
    while path != path.parent:
        if (path / "bib").is_dir() and (path / ".git").exists() and path != library_root:
            return path
        path = path.parent
    return None
