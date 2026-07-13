"""ADS API access via the official `ads` package."""
from __future__ import annotations

import bibtexparser
from bibtexparser.bparser import BibTexParser
from bibtexparser.customization import convert_to_unicode

import ads as _ads

from .config import get_config


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

_quota: dict | None = None  # {"limit": int, "remaining": int, "reset": int}


def get_quota() -> dict | None:
    """Return cached ADS rate-limit info from the most recent API call."""
    return _quota


def _capture_quota(response) -> None:
    global _quota
    try:
        h = response.headers
        _quota = {
            "limit": int(h.get("X-RateLimit-Limit", 0)),
            "remaining": int(h.get("X-RateLimit-Remaining", 0)),
            "reset": int(h.get("X-RateLimit-Reset", 0)),
        }
    except Exception:
        pass


def _set_token():
    config = get_config()
    if not config.ads_token:
        raise RuntimeError(
            "No ADS API token found.\n"
            "Set ADS_API_TOKEN env var or add ads_token to ~/.config/litbot/config.toml.\n"
            "Get a token at https://ui.adsabs.harvard.edu/user/settings/token"
        )
    _ads.config.token = config.ads_token


def search(query: str, limit: int = 20) -> list[_ads.search.Article]:
    _set_token()
    q = _ads.SearchQuery(q=query, fl=SEARCH_FIELDS, rows=limit, sort="date desc")
    results = list(q)
    try:
        _capture_quota(q.response)
    except Exception:
        pass
    return results


def fetch_bibtex(bibcode: str) -> dict | None:
    """Fetch a paper from ADS and return parsed BibTeX data dict, or None."""
    _set_token()
    exporter = _ads.ExportQuery(bibcodes=[bibcode], format="bibtex")
    raw = exporter.execute()
    try:
        _capture_quota(exporter.response)
    except Exception:
        pass
    if not raw:
        return None
    return _parse_bibtex_string(raw)


def fetch_bibtex_bulk(bibcodes: list[str]) -> list[dict]:
    _set_token()
    if not bibcodes:
        return []
    exporter = _ads.ExportQuery(bibcodes=bibcodes, format="bibtex")
    raw = exporter.execute()
    try:
        _capture_quota(exporter.response)
    except Exception:
        pass
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
