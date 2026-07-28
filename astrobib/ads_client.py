"""Direct ADS API client: search, BibTeX export, link resolver (httpx)."""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from urllib.parse import unquote

import bibtexparser
import httpx
from bibtexparser.bparser import BibTexParser
from bibtexparser.customization import convert_to_unicode

from .state import get_token

ADS_API = "https://api.adsabs.harvard.edu/v1"
ADS_SEARCH_URL = f"{ADS_API}/search/query"

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
    "citation_count",
]

_quota: dict | None = None

_ABS_URL_RE = re.compile(r"(?:https?://)?(?:ui\.)?adsabs\.harvard\.edu/abs/([^/?#\s]+)")
_PREPRINT_RE = re.compile(r"^\d{4}arXiv")
_DOI_RE = re.compile(
    r"^(?:(?:https?://)?(?:dx\.)?doi\.org/|doi:\s*)?(10\.\d{4,9}/[^\s\"]+)$",
    re.IGNORECASE,
)


@dataclass
class Article:
    """One ADS search result document."""
    _raw: dict = field(default_factory=dict, repr=False)
    bibcode: str = ""
    title: list = field(default_factory=list)
    author: list = field(default_factory=list)
    year: str = ""
    abstract: str = ""
    identifier: list = field(default_factory=list)
    doi: list = field(default_factory=list)
    esources: list = field(default_factory=list)
    arxiv_class: list = field(default_factory=list)
    citation_count: "int | None" = None

    @classmethod
    def from_doc(cls, doc: dict) -> "Article":
        return cls(
            _raw=doc,
            bibcode=doc.get("bibcode", ""),
            title=doc.get("title") or [],
            author=doc.get("author") or [],
            year=str(doc.get("year") or ""),
            abstract=doc.get("abstract") or "",
            identifier=doc.get("identifier") or [],
            doi=doc.get("doi") or [],
            esources=doc.get("esources") or [],
            arxiv_class=doc.get("arxiv_class") or [],
            citation_count=doc.get("citation_count"),
        )


def bibcode_from_url(text: str) -> str | None:
    """Extract the bibcode from a pasted ADS abstract URL, or None."""
    m = _ABS_URL_RE.search(text.strip())
    return unquote(m.group(1)) if m else None


def doi_from_text(text: str) -> str | None:
    """Extract the DOI from a pasted doi.org URL, doi: prefix, or bare DOI.

    Matches only when the whole string is the identifier, so ordinary
    queries that merely mention a DOI are left alone.
    """
    m = _DOI_RE.match(text.strip())
    return unquote(m.group(1)) if m else None


def is_preprint_bibcode(bibcode: str | None) -> bool:
    """True for arXiv-only records (bibcodes of the form 2024arXiv...)."""
    return bool(bibcode and _PREPRINT_RE.match(bibcode))


def get_quota() -> dict | None:
    return _quota


# ── HTTP plumbing ─────────────────────────────────────────────────────────────

def _require_token() -> str:
    token = get_token()
    if not token:
        raise RuntimeError(
            "No ADS API token.\n"
            "Run: astrobib config token\n"
            "Get one at: https://ui.adsabs.harvard.edu/user/settings/token"
        )
    return token


def _update_quota(resp: httpx.Response) -> None:
    global _quota
    h = resp.headers
    if h.get("X-RateLimit-Limit"):
        try:
            _quota = {
                "limit": int(h.get("X-RateLimit-Limit", 0)),
                "remaining": int(h.get("X-RateLimit-Remaining", 0)),
                "reset": int(h.get("X-RateLimit-Reset", 0)),
            }
        except ValueError:
            pass


def _api_request(method: str, url: str, **kwargs) -> httpx.Response:
    headers = {"Authorization": f"Bearer {_require_token()}"}
    try:
        resp = httpx.request(method, url, headers=headers, timeout=30, **kwargs)
    except httpx.HTTPError as exc:
        raise RuntimeError(f"ADS request failed: {exc}") from exc
    _update_quota(resp)
    if resp.status_code != 200:
        detail = resp.text[:200].strip()
        raise RuntimeError(f"ADS API error {resp.status_code}: {detail}")
    return resp


# ── Public API ────────────────────────────────────────────────────────────────

def refresh_quota() -> dict | None:
    try:
        _api_request("GET", ADS_SEARCH_URL,
                     params={"q": "*:*", "rows": "1", "fl": "bibcode"})
        return _quota
    except Exception:
        return None


def search(query: str, limit: int = 20) -> list[Article]:
    resp = _api_request("GET", ADS_SEARCH_URL, params={
        "q": query,
        "fl": ",".join(SEARCH_FIELDS),
        "rows": limit,
        "sort": "date desc",
    })
    docs = resp.json().get("response", {}).get("docs", [])
    return [Article.from_doc(d) for d in docs]


def _export_bibtex(bibcodes: list[str]) -> str:
    resp = _api_request("POST", f"{ADS_API}/export/bibtex",
                        json={"bibcode": bibcodes})
    return resp.json().get("export", "")


def fetch_bibtex(bibcode: str) -> dict | None:
    raw = _export_bibtex([bibcode])
    if not raw:
        return None
    data = _parse_bibtex_string(raw)
    if data is None:
        return None
    # BibTeX export omits the abstract; fetch it separately
    try:
        results = search(f"bibcode:{bibcode}", limit=1)
        if results and results[0].abstract:
            data["abstract"] = _clean_abstract(results[0].abstract)
    except Exception:
        pass
    return data


def fetch_bibtex_bulk(bibcodes: list[str]) -> list[dict]:
    if not bibcodes:
        return []
    raw = _export_bibtex(bibcodes)
    if not raw:
        return []
    return _parse_bibtex_string_multi(raw)


def resolve_pdf_url(bibcode: str, link_type: str = "OA_PDF") -> str | None:
    """Query the ADS link resolver and return the direct URL, or None.

    link_type: OA_PDF, EPRINT_PDF, PUB_PDF, ADS_PDF, AUTHOR_PDF
    Returns None if no token, bibcode unknown, or link type unavailable.
    """
    token = get_token()
    if not token:
        return None
    try:
        resp = httpx.get(
            f"{ADS_API}/resolver/{bibcode}/{link_type}",
            headers={"Authorization": f"Bearer {token}"},
            timeout=10,
            follow_redirects=False,
        )
        if resp.status_code != 200:
            return None
        data = resp.json()
        return data.get("link") or None
    except Exception:
        return None


def arxiv_id_from_article(article: Article) -> str | None:
    for ident in article.identifier or []:
        if ident.startswith("arXiv:"):
            return ident[6:]
    return None


# ── BibTeX parsing helpers ────────────────────────────────────────────────────

def _clean_abstract(text: str) -> str:
    """Strip HTML tags and LaTeX braces from abstract text (display-only field)."""
    text = re.sub(r'<[^>]+>', '', text)   # strip HTML tags (<SUB>, <i>, etc.)
    text = text.replace('{', '').replace('}', '')  # remove LaTeX brace groups
    return ' '.join(text.split())          # normalize whitespace


def _parse_bibtex_string(raw: str) -> dict | None:
    parser = BibTexParser(common_strings=True)
    parser.ignore_nonstandard_types = False
    parser.customization = convert_to_unicode
    bib = bibtexparser.loads(raw, parser=parser)
    if not bib.entries:
        return None
    return bib.entries[0]


def _parse_bibtex_string_multi(raw: str) -> list[dict]:
    parser = BibTexParser(common_strings=True)
    parser.ignore_nonstandard_types = False
    parser.customization = convert_to_unicode
    bib = bibtexparser.loads(raw, parser=parser)
    return bib.entries
