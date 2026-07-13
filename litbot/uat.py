"""Unified Astronomy Thesaurus (UAT) support.

Fetches and caches the UAT JSON from GitHub, then provides label-based
lookup and hierarchy traversal.

The UAT JSON is a recursive tree:
  { "children": [ { "name": "...", "uri": "...", "children": [...] }, ... ] }

UAT preferred labels are the canonical keyword strings stored in .bib
files and returned by ADS in the `keywords` field.
"""
from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path

import httpx

UAT_URL = "https://raw.githubusercontent.com/astrothesaurus/UAT/master/UAT.json"
_UAT_ID_BASE = "http://astrothesaurus.org/uat/"


@dataclass
class Concept:
    uid: str
    label: str
    alt_labels: list[str]
    definition: str
    broader: list[str]    # parent uids
    narrower: list[str]   # child uids


@dataclass
class UAT:
    _by_uid: dict[str, Concept] = field(default_factory=dict, repr=False)
    _by_label: dict[str, Concept] = field(default_factory=dict, repr=False)

    # ------------------------------------------------------------------
    # Loading
    # ------------------------------------------------------------------

    @classmethod
    def load(cls, cache_path: Path) -> "UAT":
        with open(cache_path) as f:
            raw = json.load(f)
        return cls._from_tree(raw)

    @classmethod
    def fetch_and_cache(cls, cache_path: Path, timeout: int = 60) -> "UAT":
        resp = httpx.get(UAT_URL, timeout=timeout, follow_redirects=True)
        resp.raise_for_status()
        cache_path.parent.mkdir(parents=True, exist_ok=True)
        cache_path.write_bytes(resp.content)
        return cls._from_tree(resp.json())

    @classmethod
    def _from_tree(cls, data: dict) -> "UAT":
        uat = cls()

        def walk(node: dict, parent_uid: str | None = None) -> None:
            uid = _extract_uid(node.get("uri", ""))
            label = node.get("name", "").strip()
            if not uid or not label:
                return

            raw_alt = node.get("altLabels") or []
            alt_labels = [a.strip() for a in raw_alt if isinstance(a, str) and a.strip()]

            definition = node.get("definition") or ""
            if not isinstance(definition, str):
                definition = ""

            child_nodes = node.get("children") or []
            child_uids = [
                _extract_uid(c.get("uri", ""))
                for c in child_nodes
                if _extract_uid(c.get("uri", ""))
            ]

            concept = Concept(
                uid=uid,
                label=label,
                alt_labels=alt_labels,
                definition=definition,
                broader=[parent_uid] if parent_uid else [],
                narrower=child_uids,
            )
            uat._by_uid[uid] = concept
            uat._by_label[label.lower()] = concept

            for child in child_nodes:
                walk(child, parent_uid=uid)

        for top_node in data.get("children", []):
            walk(top_node)

        return uat

    # ------------------------------------------------------------------
    # Query
    # ------------------------------------------------------------------

    def by_label(self, label: str) -> Concept | None:
        return self._by_label.get(label.lower())

    def by_uid(self, uid: str) -> Concept | None:
        return self._by_uid.get(uid)

    def children(self, uid: str) -> list[Concept]:
        concept = self._by_uid.get(uid)
        if not concept:
            return []
        return [c for c in (self._by_uid.get(n) for n in concept.narrower) if c]

    def parents(self, uid: str) -> list[Concept]:
        concept = self._by_uid.get(uid)
        if not concept:
            return []
        return [c for c in (self._by_uid.get(b) for b in concept.broader) if c]

    def descendants(self, uid: str) -> set[str]:
        """Return set of all descendant UIDs, inclusive of uid itself."""
        result: set[str] = set()
        stack = [uid]
        while stack:
            current = stack.pop()
            if current in result:
                continue
            result.add(current)
            concept = self._by_uid.get(current)
            if concept:
                stack.extend(concept.narrower)
        return result

    def descendant_labels(self, label: str) -> set[str]:
        """Return preferred labels for a concept and all its descendants."""
        concept = self.by_label(label)
        if not concept:
            return {label}
        uids = self.descendants(concept.uid)
        return {c.label for uid in uids if (c := self._by_uid.get(uid))}

    def search(self, query: str, limit: int = 20) -> list[Concept]:
        q = query.lower()
        results = [
            c for c in self._by_uid.values()
            if q in c.label.lower() or any(q in a.lower() for a in c.alt_labels)
        ]
        results.sort(key=lambda c: (not c.label.lower().startswith(q), c.label))
        return results[:limit]

    def top_level(self) -> list[Concept]:
        return [c for c in self._by_uid.values() if not c.broader]

    def __len__(self) -> int:
        return len(self._by_uid)


# ------------------------------------------------------------------
# Cache helpers
# ------------------------------------------------------------------

_instance: UAT | None = None


def get_uat(cache_path: Path, auto_fetch: bool = True) -> UAT | None:
    global _instance
    if _instance is not None:
        return _instance
    if cache_path.exists():
        try:
            _instance = UAT.load(cache_path)
            return _instance
        except Exception:
            pass
    if auto_fetch:
        try:
            _instance = UAT.fetch_and_cache(cache_path)
            return _instance
        except Exception:
            return None
    return None


def _extract_uid(uri: str) -> str:
    if uri.startswith(_UAT_ID_BASE):
        return uri[len(_UAT_ID_BASE):]
    return ""
