"""Persistent ADS query tab state stored in user-local app state dir."""
from __future__ import annotations

import json
import re
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
DEFAULT_LIMIT = 100


_OP_PREFIXES = {
    "references": "refs←",
    "citations": "cites→",
    "similar": "~",
    "trending": "trend:",
    "useful": "use:",
}


def short_label(query: str) -> str:
    """Readable tab label: drop field names, quotes, and operator wrapping."""
    q = query.strip()
    prefix = ""
    m = re.fullmatch(r"(references|citations|similar|trending|useful)\((.+)\)", q)
    if m:
        prefix = _OP_PREFIXES[m.group(1)]
        q = m.group(2)
    q = re.sub(r"\b\w+:", "", q)      # author:"^zrake" → "^zrake"
    q = q.replace('"', "").replace("(", "").replace(")", "")
    q = re.sub(r"\s+", " ", q).strip()
    return (prefix + q)[:22] or query[:22]


def make_tab(query: str, label: str | None = None, limit: int = DEFAULT_LIMIT) -> dict:
    return {
        "id": uuid.uuid4().hex[:8],
        "query": query,
        "label": (label or short_label(query))[:22],
        "limit": limit,
        "created": int(time.time()),
        "refreshed": None,
        "bibcodes": [],
    }


def step_limit(tab_data: dict, direction: int) -> int:
    """Step limit through _LIMIT_STEPS by direction (+1 or -1). Returns new limit.

    A hand-typed limit between steps snaps to the next step in the
    requested direction.
    """
    import bisect
    current = tab_data.get("limit", DEFAULT_LIMIT)
    if current in _LIMIT_STEPS:
        idx = _LIMIT_STEPS.index(current) + direction
    else:
        idx = bisect.bisect_left(_LIMIT_STEPS, current)
        if direction < 0:
            idx -= 1
    idx = max(0, min(len(_LIMIT_STEPS) - 1, idx))
    tab_data["limit"] = _LIMIT_STEPS[idx]
    return tab_data["limit"]
