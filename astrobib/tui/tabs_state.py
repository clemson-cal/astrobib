"""Persistent ADS query tab state, stored user-locally but keyed by
context: each manuscript gets its own tab set; sessions with no active
manuscript share the "global" set. Never written into a manuscript repo.
"""
from __future__ import annotations

import json
import re
import time
import uuid
from pathlib import Path

from ..state import _BASE

_STATE_FILE = _BASE / "tabs.json"
_GLOBAL = "global"


def _context_key(ms_root: Path | None) -> str:
    return str(ms_root) if ms_root else _GLOBAL


def _read_contexts() -> dict[str, list[dict]]:
    if not _STATE_FILE.exists():
        return {}
    try:
        data = json.loads(_STATE_FILE.read_text())
        return data.get("contexts", {})
    except Exception:
        return {}


def load(ms_root: Path | None = None) -> list[dict]:
    return _read_contexts().get(_context_key(ms_root), [])


def save(tabs: list[dict], ms_root: Path | None = None) -> None:
    try:
        contexts = _read_contexts()
        contexts[_context_key(ms_root)] = tabs
        _STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
        _STATE_FILE.write_text(json.dumps({"contexts": contexts}, indent=2))
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
