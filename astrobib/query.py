"""Local filter query language: ADS-flavored, evaluated against Entry objects.

Grammar — whitespace-separated terms, all AND'd together:

    term         := [-] [field:] value
    value        := bare-word | "quoted phrase"   (case-insensitive substring)
    field        := author | title | abs/abstract | key | kw/keyword | year | is

Field semantics:
    author:sironi     substring of the full author list
    author:^zrake     first-author surname prefix (ADS ^ convention)
    year:2020         exact year;  year:2015-2020 / year:2020- / year:-2015 ranges
    is:starred        starred entries
    is:ms             manuscript-db members    (needs context)
    is:pdf            entries with a cached PDF (needs context)
    <bare term>       substring across author, title, abstract, key, keywords, year

Unknown fields fall back to treating the whole token as a bare term, so a
half-typed query never errors — this is a live filter.
"""
from __future__ import annotations

import re
from typing import Callable, Optional

from .library import Entry

_TOKEN_RE = re.compile(
    r"""(?P<neg>-?)
        (?:(?P<field>[A-Za-z]+):)?
        (?P<value>"[^"]*"?|[^\s"]+)
    """,
    re.VERBOSE,
)

_YEAR_RANGE_RE = re.compile(r"^(\d{4})?-(\d{4})?$")

_FIELD_ALIASES = {
    "author": "author",
    "title": "title",
    "abs": "abs",
    "abstract": "abs",
    "key": "key",
    "kw": "kw",
    "keyword": "kw",
    "keywords": "kw",
    "year": "year",
    "is": "is",
}


def tokenize(text: str) -> list[tuple[bool, str | None, str]]:
    """Return (negated, field, value) triples; field is canonical or None."""
    terms: list[tuple[bool, str | None, str]] = []
    for m in _TOKEN_RE.finditer(text):
        neg = m.group("neg") == "-"
        field = m.group("field")
        value = m.group("value").strip('"')
        if field is not None:
            canon = _FIELD_ALIASES.get(field.lower())
            if canon is None:
                # unknown field: treat the whole token as a bare term
                field, value = None, f"{field}:{value}"
            else:
                field = canon
        if value or field:
            terms.append((neg, field, value))
    return terms


def _year_predicate(value: str) -> Callable[[Entry], bool]:
    m = _YEAR_RANGE_RE.match(value)
    if m and (m.group(1) or m.group(2)):
        lo = int(m.group(1)) if m.group(1) else None
        hi = int(m.group(2)) if m.group(2) else None

        def pred(e: Entry) -> bool:
            try:
                y = int(e.year)
            except ValueError:
                return False
            return (lo is None or y >= lo) and (hi is None or y <= hi)

        return pred
    if value.isdigit() and len(value) == 4:
        return lambda e: e.year == value
    return lambda e: value in e.year


def compile_query(
    text: str,
    *,
    in_manuscript: Optional[Callable[[str], bool]] = None,
    has_pdf: Optional[Callable[[str], bool]] = None,
) -> Callable[[Entry], bool]:
    """Compile a filter string to a predicate over Entry.

    in_manuscript / has_pdf supply context for the is:ms / is:pdf terms;
    when absent those terms match nothing.
    """
    preds: list[tuple[bool, Callable[[Entry], bool]]] = []
    for neg, fieldname, value in tokenize(text):
        v = value.lower()
        if fieldname == "year":
            pred = _year_predicate(value)
        elif fieldname == "author":
            if v.startswith("^"):
                first = v[1:]
                pred = (lambda e, _f=first: e.search_doc()["first"].startswith(_f))
            else:
                pred = (lambda e, _v=v: _v in e.search_doc()["author"])
        elif fieldname == "title":
            pred = (lambda e, _v=v: _v in e.search_doc()["title"])
        elif fieldname == "abs":
            pred = (lambda e, _v=v: _v in e.search_doc()["abs"])
        elif fieldname == "key":
            pred = (lambda e, _v=v: _v in e.search_doc()["key"])
        elif fieldname == "kw":
            pred = (lambda e, _v=v: _v in e.search_doc()["kw"])
        elif fieldname == "is":
            if v == "starred":
                pred = (lambda e: e.starred)
            elif v == "ms":
                pred = ((lambda e: in_manuscript(e.key)) if in_manuscript
                        else (lambda e: False))
            elif v == "pdf":
                pred = ((lambda e: has_pdf(e.key)) if has_pdf
                        else (lambda e: False))
            else:
                pred = (lambda e: False)
        else:
            pred = (lambda e, _v=v: _v in e.search_doc()["all"])
        preds.append((neg, pred))

    if not preds:
        return lambda e: True

    def matcher(e: Entry) -> bool:
        for neg, pred in preds:
            if pred(e) == neg:
                return False
        return True

    return matcher


def to_ads_query(text: str) -> str:
    """Translate a local filter string into an ADS search query.

    Drops astrobib-local terms (is:, key:, and negations), quotes field
    values, and maps kw: to keyword:.
    """
    parts: list[str] = []
    for neg, fieldname, value in tokenize(text):
        if neg or fieldname in ("is", "key") or not value:
            continue
        if fieldname == "year":
            parts.append(f"year:{value}")
        elif fieldname == "author":
            parts.append(f'author:"{value}"')
        elif fieldname in ("title", "abs"):
            parts.append(f'{fieldname}:"{value}"')
        elif fieldname == "kw":
            parts.append(f'keyword:"{value}"')
        elif " " in value:
            parts.append(f'"{value}"')
        else:
            parts.append(value)
    return " ".join(parts)
