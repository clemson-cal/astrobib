from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

import bibtexparser
from bibtexparser.bparser import BibTexParser
from bibtexparser.customization import convert_to_unicode


FIELD_ORDER = [
    "author",
    "title",
    "year",
    "journal",
    "booktitle",
    "volume",
    "number",
    "pages",
    "month",
    "publisher",
    "eprint",
    "archivePrefix",
    "primaryClass",
    "doi",
    "adsurl",
    "adsnote",
    "keywords",
    "abstract",
]


@dataclass
class Entry:
    data: dict
    path: Path

    @property
    def key(self) -> str:
        return self.data["ID"]

    @property
    def entry_type(self) -> str:
        return self.data["ENTRYTYPE"]

    @property
    def title(self) -> str:
        return self.data.get("title", "")

    @property
    def author(self) -> str:
        return self.data.get("author", "")

    @property
    def year(self) -> str:
        return self.data.get("year", "")

    @property
    def eprint(self) -> str:
        return self.data.get("eprint", "")

    @property
    def doi(self) -> str:
        return self.data.get("doi", "")

    @property
    def adsurl(self) -> str:
        return self.data.get("adsurl", "")

    @property
    def keywords(self) -> list[str]:
        raw = self.data.get("keywords", "")
        return [k.strip() for k in raw.split(",") if k.strip()]

    @property
    def first_author_last(self) -> str:
        author = self.author
        if not author:
            return ""
        first = author.split(" and ")[0].strip()
        return first.split(",")[0].strip()


@dataclass
class Library:
    root: Path
    _entries: dict[str, Entry] = field(default_factory=dict, repr=False)

    def __post_init__(self):
        self._load()

    def _load(self):
        bib_dir = self.root / "bib"
        bib_dir.mkdir(exist_ok=True)
        for bib_file in bib_dir.glob("*.bib"):
            try:
                entry = _parse_bib_file(bib_file)
                if entry:
                    self._entries[entry.key] = entry
            except Exception:
                pass

    def entries(self) -> list[Entry]:
        return list(self._entries.values())

    def get(self, key: str) -> Entry | None:
        return self._entries.get(key)

    def has(self, key: str) -> bool:
        return key in self._entries

    def by_keyword(self, label: str, descendant_labels: set[str] | None = None) -> list[Entry]:
        """Return entries whose keywords include label or any of its UAT descendants."""
        match_labels: set[str] = descendant_labels or {label}
        match_lower = {l.lower() for l in match_labels}
        return [
            e for e in self._entries.values()
            if any(k.lower() in match_lower for k in e.keywords)
        ]

    def save_entry(self, data: dict) -> Entry:
        key = data["ID"]
        path = self.root / "bib" / f"{key}.bib"
        path.write_text(format_bib_entry(data))
        entry = Entry(data=data, path=path)
        self._entries[key] = entry
        return entry

    def git_root(self) -> Path:
        return self.root

    def all_keywords(self) -> list[str]:
        """Collect all unique keyword strings from the library entries."""
        seen: set[str] = set()
        result: list[str] = []
        for entry in self._entries.values():
            for kw in entry.keywords:
                if kw not in seen:
                    seen.add(kw)
                    result.append(kw)
        return sorted(result)


class MergedLibrary:
    """Read-only union view across multiple Library instances.

    For reads (browse, search, export) all databases are merged.
    Cite key conflicts are resolved first-seen-wins; in practice the
    same bibcode always produces the same key and identical content.
    """

    def __init__(self, libraries: list[Library]):
        self._libs = libraries
        self._entries: dict[str, Entry] = {}
        for lib in libraries:
            for entry in lib.entries():
                self._entries.setdefault(entry.key, entry)

    def entries(self) -> list[Entry]:
        return list(self._entries.values())

    def get(self, key: str) -> Entry | None:
        return self._entries.get(key)

    def has(self, key: str) -> bool:
        return key in self._entries

    def by_keyword(self, label: str, descendant_labels: set[str] | None = None) -> list[Entry]:
        match_labels: set[str] = descendant_labels or {label}
        match_lower = {lbl.lower() for lbl in match_labels}
        return [
            e for e in self._entries.values()
            if any(k.lower() in match_lower for k in e.keywords)
        ]

    def all_keywords(self) -> list[str]:
        seen: set[str] = set()
        result: list[str] = []
        for entry in self._entries.values():
            for kw in entry.keywords:
                if kw not in seen:
                    seen.add(kw)
                    result.append(kw)
        return sorted(result)


def _parse_bib_file(path: Path) -> Entry | None:
    with open(path) as f:
        parser = BibTexParser(common_strings=True)
        parser.customization = convert_to_unicode
        bib = bibtexparser.load(f, parser=parser)
    if not bib.entries:
        return None
    return Entry(data=bib.entries[0], path=path)


def format_bib_entry(data: dict) -> str:
    key = data["ID"]
    etype = data["ENTRYTYPE"]
    skip = {"ID", "ENTRYTYPE"}
    seen: set[str] = set()
    lines = [f"@{etype}{{{key},"]

    for field_name in FIELD_ORDER:
        if field_name in data and field_name not in skip:
            val = data[field_name]
            lines.append(f"  {field_name:<16} = {{{val}}},")
            seen.add(field_name)

    for field_name, val in data.items():
        if field_name not in skip and field_name not in seen:
            lines.append(f"  {field_name:<16} = {{{val}}},")

    lines.append("}\n")
    return "\n".join(lines)
