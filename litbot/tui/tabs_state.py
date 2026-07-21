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


_LIMIT_STEPS = [20, 50, 100, 200]


def make_tab(query: str) -> dict:
    return {
        "id": uuid.uuid4().hex[:8],
        "query": query,
        "label": query[:22],
        "limit": _LIMIT_STEPS[0],
        "created": int(time.time()),
        "refreshed": None,
        "bibcodes": [],
    }


def step_limit(tab_data: dict, direction: int) -> int:
    """Cycle limit through _LIMIT_STEPS by direction (+1 or -1). Returns new limit."""
    current = tab_data.get("limit", _LIMIT_STEPS[0])
    try:
        idx = _LIMIT_STEPS.index(current)
    except ValueError:
        idx = 0
    idx = max(0, min(len(_LIMIT_STEPS) - 1, idx + direction))
    tab_data["limit"] = _LIMIT_STEPS[idx]
    return tab_data["limit"]
