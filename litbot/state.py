"""User-local app state: library path, ADS token, cache locations."""
from __future__ import annotations

import json
import os
from pathlib import Path

_BASE = Path(os.environ.get("LITBOT_STATE_DIR", Path.home() / ".local" / "share" / "litbot"))

LIBRARY_DIR = _BASE / "library"
STATE_FILE = _BASE / "state.json"
UAT_CACHE = Path.home() / ".cache" / "litbot" / "uat.json"
PDF_CACHE_DIR = Path.home() / ".cache" / "litbot" / "pdfs"

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


def get_library_path() -> Path:
    """Return the library root, creating bib/ if needed."""
    path = LIBRARY_DIR
    (path / "bib").mkdir(parents=True, exist_ok=True)
    return path
