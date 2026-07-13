"""Persistent ADS query tab state stored in user-local app state dir."""
from __future__ import annotations

import json
import time
import uuid
from pathlib import Path

from ..state import _BASE

_STATE_FILE = _BASE / "tabs.json"
SCHEMA_VERSION = 1


def load() -> list[dict]:
    if not _STATE_FILE.exists():
        return []
    try:
        data = json.loads(_STATE_FILE.read_text())
        if data.get("version") != SCHEMA_VERSION:
            return []
        return data.get("tabs", [])
    except Exception:
        return []


def save(tabs: list[dict]) -> None:
    try:
        _STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
        _STATE_FILE.write_text(
            json.dumps({"version": SCHEMA_VERSION, "tabs": tabs}, indent=2)
        )
    except Exception:
        pass


def make_tab(query: str) -> dict:
    return {
        "id": uuid.uuid4().hex[:8],
        "query": query,
        "label": query[:22],
        "created": int(time.time()),
        "refreshed": None,
        "bibcodes": [],
    }
