from __future__ import annotations

import json
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
    "astrobib_starred",
]


@dataclass
class Entry:
    data: dict
    path: Path
    short_key: str = ""  # set by Library after all entries are loaded
    _search: dict | None = field(default=None, repr=False, compare=False)

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
        return self.data.get("astrobib_starred", "").strip().lower() == "true"

    @property
    def abstract(self) -> str:
        return self.data.get("abstract", "")

    @property
    def first_author_last(self) -> str:
        author = self.author
        if not author:
            return ""
        first = author.split(" and ")[0].strip()
        return first.split(",")[0].strip()

    def search_doc(self) -> dict:
        """Lowercased field cache for filtering — built once per entry so
        per-keystroke matching never re-lowers 10^4 abstracts."""
        if self._search is None:
            author = self.author.lower()
            title = self.title.lower()
            abs_ = self.abstract.lower()
            key = self.key.lower()
            kw = self.data.get("keywords", "").lower()
            self._search = {
                "author": author,
                "first": self.first_author_last.lower().lstrip("{"),
                "title": title,
                "abs": abs_,
                "key": key,
                "kw": kw,
                "all": " ".join((author, title, abs_, key, kw, self.year)),
            }
        return self._search


class _ParseCache:
    """mtime-keyed cache of parsed bib entries, one JSON file per library
    root under ~/.cache/astrobib/parsecache/. Purely a disposable cache —
    deleting it just costs a re-parse (10-30 s at 10^4 entries, ~nothing
    below 10^3).
    """

    def __init__(self, root: Path):
        import hashlib
        from .state import PARSE_CACHE_DIR
        digest = hashlib.sha1(str(root).encode()).hexdigest()[:16]
        self._file = PARSE_CACHE_DIR / f"{digest}.json"
        self._seen: set[str] = set()
        self._dirty = False
        try:
            self._data: dict = json.loads(self._file.read_text())
        except Exception:
            self._data = {}

    def load(self, path: Path) -> Entry | None:
        key = str(path)
        try:
            mtime = path.stat().st_mtime
        except OSError:
            return None
        self._seen.add(key)
        rec = self._data.get(key)
        if rec and rec.get("mtime") == mtime:
            return Entry(data=rec["data"], path=path)
        entry = _parse_bib_file(path)
        if entry is not None:
            self._data[key] = {"mtime": mtime, "data": entry.data}
            self._dirty = True
        return entry

    def flush(self) -> None:
        stale = set(self._data) - self._seen
        if stale:
            for key in stale:
                del self._data[key]
            self._dirty = True
        if not self._dirty:
            return
        try:
            self._file.parent.mkdir(parents=True, exist_ok=True)
            self._file.write_text(json.dumps(self._data))
        except OSError:
            pass


def _bibcode_of(entry: Entry) -> str:
    adsurl = entry.data.get("adsurl", "")
    return adsurl.rstrip("/").rsplit("/", 1)[-1] if adsurl else ""


@dataclass
class Library:
    root: Path
    _entries: dict[str, Entry] = field(default_factory=dict, repr=False)
    _by_bibcode: dict[str, str] = field(default_factory=dict, repr=False)

    def __post_init__(self):
        self._load()

    def _load(self):
        bib_dir = self.root / "bib"
        bib_dir.mkdir(exist_ok=True)
        cache = _ParseCache(self.root)
        for bib_file in sorted(bib_dir.glob("*.bib")):
            try:
                entry = cache.load(bib_file)
                if entry:
                    self._entries[entry.key] = entry
            except Exception:
                pass
        cache.flush()
        self._reindex_bibcodes()
        self._compute_short_keys()

    def _reindex_bibcodes(self) -> None:
        self._by_bibcode = {}
        for key, entry in self._entries.items():
            bc = _bibcode_of(entry)
            if bc:
                self._by_bibcode[bc] = key

    def _compute_short_keys(self) -> None:
        """Populate Entry.short_key for all entries: shortest unambiguous prefix.

        Prefix counting via bisect over the sorted key list — O(N log N),
        not O(N^2); matters from a few thousand entries up.
        """
        import bisect
        sorted_keys = sorted(self._entries)

        def prefix_count(prefix: str) -> int:
            lo = bisect.bisect_left(sorted_keys, prefix)
            hi = bisect.bisect_left(sorted_keys, prefix + "￿")
            return hi - lo

        for key, entry in self._entries.items():
            base = key[:-5]  # strip the 5-char sha256 suffix
            if prefix_count(base) == 1:
                entry.short_key = base
            else:
                for n in range(1, 6):
                    prefix = key[:len(base) + n]
                    if prefix_count(prefix) == 1:
                        entry.short_key = prefix
                        break
                else:
                    entry.short_key = key

    def entries(self) -> list[Entry]:
        return list(self._entries.values())

    def get(self, key: str) -> Entry | None:
        return self._entries.get(key)

    def resolve(self, input_key: str) -> Entry | None:
        """Resolve a full key, shortened key, or bibcode to an entry.

        Tries exact match first, then unambiguous prefix match, then the
        bibcode index (bibcodes start with digits so they can never
        collide with key prefixes). Returns None if not found or ambiguous.
        """
        if input_key in self._entries:
            return self._entries[input_key]
        matches = [e for k, e in self._entries.items() if k.startswith(input_key)]
        if len(matches) == 1:
            return matches[0]
        if not matches:
            return self.get_by_bibcode(input_key)
        return None

    def has(self, key: str) -> bool:
        return key in self._entries

    def possible_matches(self, input_key: str) -> list[Entry]:
        """Return all entries whose key starts with input_key (for ambiguity reporting)."""
        return [e for e in self._entries.values() if e.key.startswith(input_key)]

    def has_bibcode(self, bibcode: str) -> bool:
        return bibcode in self._by_bibcode

    def get_by_bibcode(self, bibcode: str) -> Entry | None:
        key = self._by_bibcode.get(bibcode)
        return self._entries.get(key) if key else None

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
        bc = _bibcode_of(entry)
        if bc:
            self._by_bibcode[bc] = key
        self._compute_short_keys()
        return entry

    def set_starred(self, key: str, starred: bool) -> None:
        entry = self._entries.get(key)
        if entry is None:
            return
        if starred:
            entry.data["astrobib_starred"] = "true"
        else:
            entry.data.pop("astrobib_starred", None)
        entry.path.write_text(format_bib_entry(entry.data))

    def update_entry_data(self, key: str, data: dict) -> Entry | None:
        """Rewrite an existing entry's data in place, keeping its key.

        Used by the arXiv->published refresh: the file name and cite key
        never change; the bibcode index follows the new adsurl.
        """
        old = self._entries.get(key)
        if old is None:
            return None
        data = dict(data)
        data["ID"] = key
        path = self.root / "bib" / f"{key}.bib"
        path.write_text(format_bib_entry(data))
        self._by_bibcode.pop(_bibcode_of(old), None)
        entry = Entry(data=data, path=path, short_key=old.short_key)
        self._entries[key] = entry
        bc = _bibcode_of(entry)
        if bc:
            self._by_bibcode[bc] = key
        return entry

    def remove_entry(self, key: str) -> None:
        path = self.root / "bib" / f"{key}.bib"
        if path.exists():
            path.unlink()
        entry = self._entries.pop(key, None)
        if entry is not None:
            self._by_bibcode.pop(_bibcode_of(entry), None)
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


@dataclass
class MergedLibrary:
    """Personal library merged with an optional manuscript database.

    Reads span both; the personal entry wins when a key exists in both
    (it may carry personal fields like astrobib_starred). Imports write to
    both. Manuscript membership is toggled explicitly.
    """
    personal: Library
    manuscript: Library | None = None
    _merged_cache: "dict[str, Entry] | None" = field(
        default=None, init=False, repr=False, compare=False)

    @property
    def manuscript_root(self) -> Path | None:
        return self.manuscript.root if self.manuscript else None

    def _merged(self) -> dict[str, Entry]:
        # Memoized: this runs per cited key per 2 s manuscript poll, so
        # rebuilding a 10^4-entry dict each call would be hot-path waste.
        if self._merged_cache is None:
            merged = dict(self.manuscript._entries) if self.manuscript else {}
            merged.update(self.personal._entries)
            self._merged_cache = merged
        return self._merged_cache

    def _invalidate(self) -> None:
        self._merged_cache = None

    def entries(self) -> list[Entry]:
        return list(self._merged().values())

    def get(self, key: str) -> Entry | None:
        return self._merged().get(key)

    def has(self, key: str) -> bool:
        return key in self._merged()

    def resolve(self, input_key: str) -> Entry | None:
        merged = self._merged()
        if input_key in merged:
            return merged[input_key]
        matches = [e for k, e in merged.items() if k.startswith(input_key)]
        if len(matches) == 1:
            return matches[0]
        if not matches:
            return self.get_by_bibcode(input_key)
        return None

    def possible_matches(self, input_key: str) -> list[Entry]:
        return [e for k, e in self._merged().items() if k.startswith(input_key)]

    def by_keyword(self, label: str, descendant_labels: set[str] | None = None) -> list[Entry]:
        match_lower = {lbl.lower() for lbl in (descendant_labels or {label})}
        return [
            e for e in self._merged().values()
            if any(k.lower() in match_lower for k in e.keywords)
        ]

    def all_keywords(self) -> list[str]:
        seen: set[str] = set()
        for entry in self._merged().values():
            seen.update(entry.keywords)
        return sorted(seen)

    def resolve_citation(self, cited: str) -> "tuple[str, Entry | None]":
        """Classify a cite string from a manuscript. Accepted forms: a full
        key, an unambiguous key prefix (so hash suffixes can stay out of
        the .tex), or a raw ADS bibcode (globally unique by construction).

        Returns (state, entry): 'ok' (resolves to a manuscript entry),
        'library' (resolves, but only in the personal library),
        'ambiguous' (prefix of several keys), or 'missing' (no match).
        """
        entry = self.get(cited)
        if entry is None:
            matches = self.possible_matches(cited)
            if len(matches) == 1:
                entry = matches[0]
            elif matches:
                return "ambiguous", None
            else:
                entry = self.get_by_bibcode(cited)
                if entry is None:
                    return "missing", None
        if self.in_manuscript(entry.key):
            return "ok", entry
        return "library", entry

    def in_manuscript(self, key: str) -> bool:
        return self.manuscript is not None and self.manuscript.has(key)

    def in_personal(self, key: str) -> bool:
        return self.personal.has(key)

    def has_bibcode(self, bibcode: str) -> bool:
        return self.personal.has_bibcode(bibcode) or (
            self.manuscript is not None and self.manuscript.has_bibcode(bibcode)
        )

    def get_by_bibcode(self, bibcode: str) -> Entry | None:
        return self.personal.get_by_bibcode(bibcode) or (
            self.manuscript.get_by_bibcode(bibcode) if self.manuscript else None
        )

    def save_entry(self, data: dict) -> Entry:
        """Import: write to the personal library and the manuscript db (if any)."""
        self._invalidate()
        entry = self.personal.save_entry(data)
        if self.manuscript is not None:
            self.manuscript.save_entry(dict(data))
        return entry

    def remove_entry(self, key: str) -> None:
        self._invalidate()
        self.personal.remove_entry(key)
        if self.manuscript is not None:
            self.manuscript.remove_entry(key)

    def set_starred(self, key: str, starred: bool) -> None:
        """Stars are personal — never written into the manuscript db."""
        self.personal.set_starred(key, starred)

    def update_entry(self, key: str, data: dict) -> Entry | None:
        """Refresh an entry's metadata under the same key in both databases.

        Preserves the personal copy's astrobib_starred flag and each
        copy's user-curated keywords (when non-empty); everything else
        comes from the new record.
        """
        self._invalidate()
        result = None
        pe = self.personal.get(key)
        if pe is not None:
            d = dict(data)
            if pe.data.get("astrobib_starred"):
                d["astrobib_starred"] = pe.data["astrobib_starred"]
            if pe.data.get("keywords"):
                d["keywords"] = pe.data["keywords"]
            result = self.personal.update_entry_data(key, d)
        if self.manuscript is not None and self.manuscript.has(key):
            d = dict(data)
            me = self.manuscript.get(key)
            if me.data.get("keywords"):
                d["keywords"] = me.data["keywords"]
            d.pop("astrobib_starred", None)
            updated = self.manuscript.update_entry_data(key, d)
            result = result or updated
        return result

    def add_to_manuscript(self, key: str) -> bool:
        if self.manuscript is None or self.manuscript.has(key):
            return False
        entry = self.get(key)
        if entry is None:
            return False
        self._invalidate()
        data = dict(entry.data)
        data.pop("astrobib_starred", None)
        path = self.manuscript.root / "bib" / f"{key}.bib"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(format_bib_entry(data))
        new_entry = Entry(data=data, path=path)
        self.manuscript._entries[key] = new_entry
        bc = _bibcode_of(new_entry)
        if bc:
            self.manuscript._by_bibcode[bc] = key
        self.manuscript._compute_short_keys()
        return True

    def remove_from_manuscript(self, key: str) -> bool:
        """Remove an entry from the manuscript db.

        If the manuscript holds the only copy, it is first copied into
        the personal library — removal never destroys bibdata, it just
        demotes the entry to personal-only.
        """
        if self.manuscript is None or not self.manuscript.has(key):
            return False
        self._invalidate()
        if not self.personal.has(key):
            entry = self.manuscript.get(key)
            path = self.personal.root / "bib" / f"{key}.bib"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(format_bib_entry(entry.data))
            rescued = Entry(data=dict(entry.data), path=path)
            self.personal._entries[key] = rescued
            bc = _bibcode_of(rescued)
            if bc:
                self.personal._by_bibcode[bc] = key
            self.personal._compute_short_keys()
        self.manuscript.remove_entry(key)
        return True


def _parse_bib_file(path: Path) -> Entry | None:
    with open(path) as f:
        parser = BibTexParser(common_strings=True)
        parser.ignore_nonstandard_types = False
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
