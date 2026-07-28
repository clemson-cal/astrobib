"""Local filter query language: ADS-flavored, evaluated against Entry objects.

Grammar — whitespace-separated terms AND together; uppercase OR separates
alternative groups (AND binds tighter than OR, as in ADS):

    query        := group (OR group)*
    group        := term+
    term         := [- | NOT ] [field:] value
    value        := bare-word | "quoted phrase"   (case-insensitive substring)
    field        := author | title | abs/abstract | key | kw/keyword | year | is

Operators are uppercase-only (lowercase or/and/not are ordinary bare terms,
matching ADS's own convention). AND is a no-op — terms already AND — and
NOT is an alias for the '-' prefix. There is no parenthesized grouping;
queries needing it belong on ADS (press S).

Field semantics:
    author:sironi     substring of the full author list
    author:^zrake     first-author surname prefix (ADS ^ convention)
    ^zrake            bare ^ term: sugar for author:^zrake
    year:2020         exact year;  year:2015-2020 / year:2020- / year:-2015 ranges
    is:starred        starred entries
    is:ms             manuscript-db members    (needs context)
    is:pdf            entries with a cached PDF (needs context)
    <bare term>       substring across author, title, abstract, key, keywords, year

Unknown fields fall back to treating the whole token as a bare term, and a
dangling OR/NOT is ignored, so a half-typed query never errors — this is a
live filter.
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


def tokenize(text: str) -> list[list[tuple[bool, str | None, str]]]:
    """Split a filter string into OR-groups of (negated, field, value)
    triples; field is canonical or None. Uppercase OR starts a new group,
    AND is skipped, NOT negates the following term."""
    groups: list[list[tuple[bool, str | None, str]]] = [[]]
    pending_not = False
    for m in _TOKEN_RE.finditer(text):
        raw = m.group(0)
        if raw == "OR":
            if groups[-1]:  # a leading or doubled OR is ignored
                groups.append([])
            continue
        if raw == "AND":
            continue
        if raw == "NOT":
            pending_not = True
            continue
        neg = m.group("neg") == "-" or pending_not
        pending_not = False
        field = m.group("field")
        value = m.group("value").strip('"')
        if field is not None:
            canon = _FIELD_ALIASES.get(field.lower())
            if canon is None:
                # unknown field: treat the whole token as a bare term
                field, value = None, f"{field}:{value}"
            else:
                field = canon
        if field is None and value.startswith("^") and len(value) > 1:
            field = "author"  # bare ^name is first-author sugar
        if value or field:
            groups[-1].append((neg, field, value))
    return [g for g in groups if g]


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

    Terms within a group AND together; OR-groups are OR'd.
    in_manuscript / has_pdf supply context for the is:ms / is:pdf terms;
    when absent those terms match nothing.
    """
    def term_pred(fieldname: str | None, value: str) -> Callable[[Entry], bool]:
        v = value.lower()
        if fieldname == "year":
            return _year_predicate(value)
        if fieldname == "author":
            if v.startswith("^"):
                first = v[1:]
                return (lambda e, _f=first: e.search_doc()["first"].startswith(_f))
            return (lambda e, _v=v: _v in e.search_doc()["author"])
        if fieldname == "title":
            return (lambda e, _v=v: _v in e.search_doc()["title"])
        if fieldname == "abs":
            return (lambda e, _v=v: _v in e.search_doc()["abs"])
        if fieldname == "key":
            return (lambda e, _v=v: _v in e.search_doc()["key"])
        if fieldname == "kw":
            return (lambda e, _v=v: _v in e.search_doc()["kw"])
        if fieldname == "is":
            if v == "starred":
                return (lambda e: e.starred)
            if v == "ms":
                return ((lambda e: in_manuscript(e.key)) if in_manuscript
                        else (lambda e: False))
            if v == "pdf":
                return ((lambda e: has_pdf(e.key)) if has_pdf
                        else (lambda e: False))
            return (lambda e: False)
        return (lambda e, _v=v: _v in e.search_doc()["all"])

    groups = [[(neg, term_pred(f, v)) for neg, f, v in group]
              for group in tokenize(text)]
    if not groups:
        return lambda e: True

    def matcher(e: Entry) -> bool:
        return any(all(pred(e) != neg for neg, pred in preds)
                   for preds in groups)

    return matcher


def to_ads_query(text: str) -> str:
    """Translate a local filter string into an ADS search query.

    Drops astrobib-local terms (is:, key:, and negations), quotes field
    values, and maps kw: to keyword:. OR-groups are parenthesized so ADS
    precedence matches the local semantics.
    """
    groups: list[list[str]] = []
    for group in tokenize(text):
        parts: list[str] = []
        for neg, fieldname, value in group:
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
        if parts:
            groups.append(parts)
    if len(groups) > 1:
        return " OR ".join(" ".join(p) if len(p) == 1 else f"({' '.join(p)})"
                           for p in groups)
    return " ".join(groups[0]) if groups else ""
