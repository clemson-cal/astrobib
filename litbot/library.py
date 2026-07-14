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
    "litbot_starred",
]


@dataclass
class Entry:
    data: dict
    path: Path
    short_key: str = ""  # set by Library after all entries are loaded

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
    def starred(self) -> bool:
        return self.data.get("litbot_starred", "").strip().lower() == "true"

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
        for bib_file in sorted(bib_dir.glob("*.bib")):
            try:
                entry = _parse_bib_file(bib_file)
                if entry:
                    self._entries[entry.key] = entry
            except Exception:
                pass
        self._compute_short_keys()

    def _compute_short_keys(self) -> None:
        """Populate Entry.short_key for all entries: shortest unambiguous prefix."""
        all_keys = list(self._entries)
        for key, entry in self._entries.items():
            base = key[:-5]  # strip the 5-char sha256 suffix
            if sum(1 for k in all_keys if k.startswith(base)) == 1:
                entry.short_key = base
            else:
                for n in range(1, 6):
                    prefix = key[:len(base) + n]
                    if sum(1 for k in all_keys if k.startswith(prefix)) == 1:
                        entry.short_key = prefix
                        break
                else:
                    entry.short_key = key

    def entries(self) -> list[Entry]:
        return list(self._entries.values())

    def get(self, key: str) -> Entry | None:
        return self._entries.get(key)

    def resolve(self, input_key: str) -> Entry | None:
        """Resolve a full or shortened key to an entry.

        Tries exact match first, then unambiguous prefix match.
        Returns None if not found or ambiguous.
        """
        if input_key in self._entries:
            return self._entries[input_key]
        matches = [e for k, e in self._entries.items() if k.startswith(input_key)]
        return matches[0] if len(matches) == 1 else None

    def has(self, key: str) -> bool:
        return key in self._entries

    def possible_matches(self, input_key: str) -> list[Entry]:
        """Return all entries whose key starts with input_key (for ambiguity reporting)."""
        return [e for e in self._entries.values() if e.key.startswith(input_key)]

    def has_bibcode(self, bibcode: str) -> bool:
        return any(bibcode in e.data.get("adsurl", "") for e in self._entries.values())

    def get_by_bibcode(self, bibcode: str) -> Entry | None:
        return next(
            (e for e in self._entries.values() if bibcode in e.data.get("adsurl", "")),
            None,
        )

    def by_keyword(self, label: str, descendant_labels: set[str] | None = None) -> list[Entry]:
        match_labels: set[str] = descendant_labels or {label}
        match_lower = {lbl.lower() for lbl in match_labels}
        return [
            e for e in self._entries.values()
            if any(k.lower() in match_lower for k in e.keywords)
        ]

    def save_entry(self, data: dict) -> Entry:
        from .keys import generate_key
        data = dict(data)
        key = generate_key(data)
        data["ID"] = key
        path = self.root / "bib" / f"{key}.bib"
        path.write_text(format_bib_entry(data))
        entry = Entry(data=data, path=path)
        self._entries[key] = entry
        self._compute_short_keys()
        return entry

    def set_starred(self, key: str, starred: bool) -> None:
        entry = self._entries.get(key)
        if entry is None:
            return
        if starred:
            entry.data["litbot_starred"] = "true"
        else:
            entry.data.pop("litbot_starred", None)
        entry.path.write_text(format_bib_entry(entry.data))

    def remove_entry(self, key: str) -> None:
        path = self.root / "bib" / f"{key}.bib"
        if path.exists():
            path.unlink()
        self._entries.pop(key, None)
        self._compute_short_keys()

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
