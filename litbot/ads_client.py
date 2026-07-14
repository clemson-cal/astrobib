"""ADS API access via the official `ads` package."""
from __future__ import annotations

import bibtexparser
import httpx
from bibtexparser.bparser import BibTexParser
from bibtexparser.customization import convert_to_unicode

import ads as _ads

from .state import get_token


ADS_SEARCH_URL = "https://api.adsabs.harvard.edu/v1/search/query"

SEARCH_FIELDS = [
    "bibcode",
    "title",
    "author",
    "year",
    "abstract",
    "identifier",
    "doi",
    "esources",
    "arxiv_class",
]

_quota: dict | None = None


def get_quota() -> dict | None:
    return _quota


def refresh_quota() -> dict | None:
    global _quota
    try:
        token = get_token()
        if not token:
            return None
        resp = httpx.get(
            ADS_SEARCH_URL,
            params={"q": "*:*", "rows": "1", "fl": "bibcode"},
            headers={"Authorization": f"Bearer {token}"},
            timeout=5,
        )
        h = resp.headers
        _quota = {
            "limit": int(h.get("X-RateLimit-Limit", 0)),
            "remaining": int(h.get("X-RateLimit-Remaining", 0)),
            "reset": int(h.get("X-RateLimit-Reset", 0)),
        }
        return _quota
    except Exception:
        return None


def _set_token():
    token = get_token()
    if not token:
        raise RuntimeError(
            "No ADS API token.\n"
            "Run: litbot token\n"
            "Get one at: https://ui.adsabs.harvard.edu/user/settings/token"
        )
    _ads.config.token = token


def search(query: str, limit: int = 20) -> list[_ads.search.Article]:
    _set_token()
    q = _ads.SearchQuery(q=query, fl=SEARCH_FIELDS, rows=limit, sort="date desc")
    results = list(q)
    global _quota
    try:
        rl = q._rate_limits
        if rl and rl.get("limit"):
            _quota = {k: int(v) for k, v in rl.items()}
    except Exception:
        pass
    return results


def fetch_bibtex(bibcode: str) -> dict | None:
    _set_token()
    exporter = _ads.ExportQuery(bibcodes=[bibcode], format="bibtex")
    raw = exporter.execute()
    if not raw:
        return None
    data = _parse_bibtex_string(raw)
    if data is None:
        return None
    # BibTeX export omits the abstract; fetch it separately
    try:
        results = list(_ads.SearchQuery(q=f"bibcode:{bibcode}", fl=["abstract"], rows=1))
        if results and results[0].abstract:
            data["abstract"] = results[0].abstract
    except Exception:
        pass
    return data


def fetch_bibtex_bulk(bibcodes: list[str]) -> list[dict]:
    _set_token()
    if not bibcodes:
        return []
    exporter = _ads.ExportQuery(bibcodes=bibcodes, format="bibtex")
    raw = exporter.execute()
    if not raw:
        return []
    return _parse_bibtex_string_multi(raw)


def arxiv_id_from_article(article: _ads.search.Article) -> str | None:
    for ident in article.identifier or []:
        if ident.startswith("arXiv:"):
            return ident[6:]
    return None


def _parse_bibtex_string(raw: str) -> dict | None:
    parser = BibTexParser(common_strings=True)
    parser.customization = convert_to_unicode
    bib = bibtexparser.loads(raw, parser=parser)
    if not bib.entries:
        return None
    return bib.entries[0]


def _parse_bibtex_string_multi(raw: str) -> list[dict]:
    parser = BibTexParser(common_strings=True)
    parser.customization = convert_to_unicode
    bib = bibtexparser.loads(raw, parser=parser)
    return bib.entries
