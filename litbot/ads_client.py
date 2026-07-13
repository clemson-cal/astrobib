"""ADS API access via the official `ads` package."""
from __future__ import annotations

import bibtexparser
import httpx
from bibtexparser.bparser import BibTexParser
from bibtexparser.customization import convert_to_unicode

import ads as _ads

from .config import get_config


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

_quota: dict | None = None  # {"limit": int, "remaining": int, "reset": int}


def get_quota() -> dict | None:
    """Return cached ADS rate-limit info."""
    return _quota


def refresh_quota() -> dict | None:
    """Make a minimal ADS call via httpx to update the rate-limit cache."""
    global _quota
    try:
        config = get_config()
        if not config.ads_token:
            return None
        resp = httpx.get(
            ADS_SEARCH_URL,
            params={"q": "*:*", "rows": "1", "fl": "bibcode"},
            headers={"Authorization": f"Bearer {config.ads_token}"},
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
    # Try to pick up quota from ads package internals; fall through on failure
    global _quota
    try:
        rl = q._rate_limits
        if rl and rl.get("limit"):
            _quota = {k: int(v) for k, v in rl.items()}
    except Exception:
        pass
    return results


def fetch_bibtex(bibcode: str) -> dict | None:
    """Fetch a paper from ADS and return parsed BibTeX data dict, or None."""
    _set_token()
    exporter = _ads.ExportQuery(bibcodes=[bibcode], format="bibtex")
    raw = exporter.execute()
    if not raw:
        return None
    return _parse_bibtex_string(raw)


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
