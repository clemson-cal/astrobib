//! Direct ADS API client: search, BibTeX export, link resolver, and
//! rate-limit quota tracking.

use crate::bib::{self, Data};
use anyhow::{anyhow, bail, Result};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const ADS_API: &str = "https://api.adsabs.harvard.edu/v1";

const SEARCH_FIELDS: &str =
    "bibcode,title,author,year,abstract,identifier,doi,esources,arxiv_class,citation_count,pub,volume,issue,page,entry_date";

#[derive(Debug, Default)]
pub struct Article {
    pub bibcode: String,
    pub title: String,
    pub author: Vec<String>,
    pub year: String,
    pub abstract_: String,
    pub doi: Vec<String>,
    pub identifier: Vec<String>,
    pub citation_count: Option<i64>,
    /// When ADS first indexed the record — the posting clock, distinct
    /// from `year`, which is the publication clock. A 2019 paper indexed
    /// this week is new by this measure and old by the other.
    pub entry_date: String,
    pub journal: String,
    pub volume: String,
    pub issue: String,
    pub page: String,
}

fn state_file() -> PathBuf {
    let base = std::env::var("ASTROBIB_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/astrobib")
        });
    base.join("state.json")
}

/// ADS token from $ADS_API_TOKEN, or the ads_token field of state.json.
pub fn get_token() -> Option<String> {
    if let Ok(t) = std::env::var("ADS_API_TOKEN") {
        if !t.is_empty() {
            return Some(t);
        }
    }
    let raw = std::fs::read_to_string(state_file()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("ads_token")?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

fn require_token() -> Result<String> {
    get_token().ok_or_else(|| {
        anyhow!(
            "No ADS API token.\nSet ADS_API_TOKEN or add ads_token to astrobib's state.json.\n\
             Get one at: https://ui.adsabs.harvard.edu/user/settings/token"
        )
    })
}

/// Every state.json write is read-modify-write, and the quota is
/// written from whichever worker thread finished an ADS call — without
/// this the two could interleave and one of them would lose its field.
static STATE_WRITE: Mutex<()> = Mutex::new(());

/// Persist one field into state.json (creating it if absent),
/// preserving every other field.
pub fn save_state_field(key: &str, value: &str) -> std::io::Result<()> {
    save_state_value(key, serde_json::Value::String(value.to_string()))
}

/// The same, for a field whose value is structured rather than a
/// string — the per-scope column configuration, for instance.
pub fn save_state_value(key: &str, value: serde_json::Value) -> std::io::Result<()> {
    let _guard = STATE_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let path = state_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut v: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({ "version": 1 }));
    v[key] = value;
    // the mutex above orders this process's writers against each other;
    // the temp-and-rename is what keeps a crash, or another process,
    // from leaving a half-written state.json behind
    crate::library::write_atomic(&path, &(serde_json::to_string_pretty(&v)? + "\n"))
}

/// Any structured field from state.json.
pub fn get_state_value(key: &str) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(state_file()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get(key).cloned()
}

/// Any string field from state.json.
pub fn get_state_field(key: &str) -> Option<String> {
    let raw = std::fs::read_to_string(state_file()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get(key)?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Saved email from state.json (a courtesy identifier for API use).
pub fn get_email() -> Option<String> {
    let raw = std::fs::read_to_string(state_file()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("email")?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
}

fn check(
    endpoint: &str,
    resp: std::result::Result<ureq::Response, ureq::Error>,
) -> Result<ureq::Response> {
    match resp {
        Ok(r) => {
            update_quota(endpoint, &r);
            Ok(r)
        }
        Err(ureq::Error::Status(code, r)) => {
            update_quota(endpoint, &r);
            let detail = r.into_string().unwrap_or_default();
            bail!("ADS API error {code}: {}", detail.trim().chars().take(200).collect::<String>())
        }
        Err(e) => bail!("ADS request failed: {e}"),
    }
}

/// Rate-limit state from the most recent ADS response on one endpoint.
#[derive(Clone, Copy, Debug)]
pub struct Quota {
    pub limit: i64,
    pub remaining: i64,
    /// Unix time at which the counter rolls back up to `limit`.
    pub reset: i64,
}

impl Quota {
    /// Calls spent out of the day's allowance for this endpoint.
    pub fn used(&self) -> i64 {
        (self.limit - self.remaining).max(0)
    }

    /// Whether the window these counts were captured in has since
    /// rolled over, which makes them history rather than the state of
    /// the allowance now. The next call to that endpoint replaces them.
    pub fn stale(&self) -> bool {
        now_secs() >= self.reset
    }

    /// "4h 12m" until the counter resets, or None once it has.
    pub fn resets_in(&self) -> Option<String> {
        let left = self.reset - now_secs();
        (left > 0).then(|| human_span(left))
    }

    /// This endpoint's line for a human: what is spent, and when the
    /// allowance comes back.
    pub fn describe(&self) -> String {
        match self.resets_in() {
            Some(span) => format!("{} of {} used · resets in {span}", self.used(), self.limit),
            // the counts predate a reset, so nothing is known to be
            // spent — only the size of the allowance survives
            None => format!("0 of {} used · window reset since the last call", self.limit),
        }
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn human_span(secs: i64) -> String {
    let m = secs / 60;
    match m {
        0 => "<1m".to_string(),
        1..=59 => format!("{m}m"),
        _ => format!("{}h {}m", m / 60, m % 60),
    }
}

/// ADS meters each endpoint separately — a day of searching does not
/// touch the export allowance — so the quotas are kept apart and never
/// summed. These are the endpoints this app calls, in reporting order.
pub const QUOTA_ENDPOINTS: [&str; 3] = ["search", "export", "resolver"];

/// None until first use, then the quotas seeded from state.json and
/// updated by every round-trip this process makes.
static QUOTA: Mutex<Option<BTreeMap<String, Quota>>> = Mutex::new(None);

fn with_quotas<R>(f: impl FnOnce(&mut BTreeMap<String, Quota>) -> R) -> R {
    let mut slot = QUOTA.lock().unwrap_or_else(|e| e.into_inner());
    let map = slot.get_or_insert_with(load_quotas);
    f(map)
}

/// The last-seen quota per endpoint — from this process if it has
/// called ADS, else from what an earlier run recorded in state.json.
/// Endpoints never called are simply absent.
pub fn quotas() -> Vec<(&'static str, Quota)> {
    with_quotas(|m| {
        QUOTA_ENDPOINTS
            .iter()
            .filter_map(|ep| m.get(*ep).map(|q| (*ep, *q)))
            .collect()
    })
}

/// One line's worth of how much of the token's day is gone:
/// "search 12/5000 · export 3/100". None when no call was ever
/// recorded; endpoints whose window has since reset read 0.
pub fn quota_summary() -> Option<String> {
    let qs = quotas();
    if qs.is_empty() {
        return None;
    }
    Some(
        qs.iter()
            .map(|(ep, q)| {
                let used = if q.stale() { 0 } else { q.used() };
                format!("{ep} {used}/{}", q.limit)
            })
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

fn load_quotas() -> BTreeMap<String, Quota> {
    let mut m = BTreeMap::new();
    let Some(v) = get_state_value("ads_quota") else {
        return m;
    };
    for ep in QUOTA_ENDPOINTS {
        let q = &v[ep];
        if let (Some(limit), Some(remaining), Some(reset)) =
            (q["limit"].as_i64(), q["remaining"].as_i64(), q["reset"].as_i64())
        {
            m.insert(ep.to_string(), Quota { limit, remaining, reset });
        }
    }
    m
}

/// Parse the X-RateLimit-* headers into the endpoint's quota slot: only
/// when the Limit header is present, and skipped entirely if any
/// present header fails to parse (missing ones read 0).
///
/// Every update is mirrored to state.json, so a later process — the CLI
/// reporting the day's use, say — can say what this one spent without
/// having to spend another call to find out.
fn update_quota(endpoint: &str, resp: &ureq::Response) {
    if resp.header("X-RateLimit-Limit").is_none() {
        return;
    }
    let h = |name: &str| resp.header(name).unwrap_or("0").parse::<i64>().ok();
    let (Some(limit), Some(remaining), Some(reset)) = (
        h("X-RateLimit-Limit"),
        h("X-RateLimit-Remaining"),
        h("X-RateLimit-Reset"),
    ) else {
        return;
    };
    let snapshot = with_quotas(|m| {
        m.insert(endpoint.to_string(), Quota { limit, remaining, reset });
        m.clone()
    });
    let obj: serde_json::Map<String, serde_json::Value> = snapshot
        .iter()
        .map(|(ep, q)| {
            (
                ep.clone(),
                serde_json::json!({ "limit": q.limit, "remaining": q.remaining, "reset": q.reset }),
            )
        })
        .collect();
    let _ = save_state_value("ads_quota", serde_json::Value::Object(obj));
}

/// What a saved query selects by when it has no stored preference:
/// the newest *postings*, which is what makes a query tab read as a
/// feed. Note this is the ADS `sort` parameter — it decides which
/// records come back, not how the ones in hand are displayed.
pub const DEFAULT_ADS_SORT: &str = "entry_date desc";

/// A one-off lookup, ordered by publication date. Callers that fetch a
/// single record by identifier do not care; `search_sorted` is for the
/// saved queries, which do.
pub fn search(query: &str, limit: usize) -> Result<Vec<Article>> {
    search_sorted(query, limit, "date desc")
}

/// `sort` is an ADS API parameter, not query syntax — putting
/// `sort:"entry_date desc"` inside `q` is a Solr error, not a sort.
pub fn search_sorted(query: &str, limit: usize, sort: &str) -> Result<Vec<Article>> {
    let token = require_token()?;
    let resp = check(
        "search",
        agent()
            .get(&format!("{ADS_API}/search/query"))
            .set("Authorization", &format!("Bearer {token}"))
            .query("q", query)
            .query("fl", SEARCH_FIELDS)
            .query("rows", &limit.to_string())
            .query("sort", sort)
            .call(),
    )?;
    let v: serde_json::Value = resp.into_json()?;
    let docs = v["response"]["docs"].as_array().cloned().unwrap_or_default();
    Ok(docs.iter().map(article_from_doc).collect())
}

/// Serialize an Article in the same shape the ADS docs use, so the
/// query cache round-trips through article_from_doc.
pub(crate) fn article_to_json(a: &Article) -> serde_json::Value {
    serde_json::json!({
        "bibcode": a.bibcode,
        "title": [a.title],
        "author": a.author,
        "year": a.year,
        "abstract": a.abstract_,
        "doi": a.doi,
        "identifier": a.identifier,
        "citation_count": a.citation_count,
        "entry_date": a.entry_date,
        "pub": a.journal,
        "volume": a.volume,
        "issue": a.issue,
        "page": [a.page],
    })
}

pub(crate) fn article_from_doc(d: &serde_json::Value) -> Article {
    let strs = |k: &str| -> Vec<String> {
        d[k].as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    Article {
        bibcode: d["bibcode"].as_str().unwrap_or_default().to_string(),
        title: strs("title").into_iter().next().unwrap_or_default(),
        author: strs("author"),
        year: match &d["year"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => String::new(),
        },
        abstract_: d["abstract"].as_str().unwrap_or_default().to_string(),
        doi: strs("doi"),
        identifier: strs("identifier"),
        citation_count: d["citation_count"].as_i64(),
        // ADS returns an ISO timestamp; only the date part is shown or
        // compared, and lexical order on it is chronological order
        entry_date: d["entry_date"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(10)
            .collect(),
        journal: d["pub"].as_str().unwrap_or_default().to_string(),
        volume: d["volume"].as_str().unwrap_or_default().to_string(),
        issue: d["issue"].as_str().unwrap_or_default().to_string(),
        page: strs("page").into_iter().next().unwrap_or_default(),
    }
}

/// Fetch canonical BibTeX for a bibcode; the export omits the abstract,
/// so it is fetched separately (one extra search call) and appended.
pub fn fetch_bibtex(bibcode: &str) -> Result<Option<Data>> {
    let token = require_token()?;
    let resp = check(
        "export",
        agent()
            .post(&format!("{ADS_API}/export/bibtex"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(serde_json::json!({ "bibcode": [bibcode] })),
    )?;
    let v: serde_json::Value = resp.into_json()?;
    let raw = v["export"].as_str().unwrap_or_default();
    if raw.is_empty() {
        return Ok(None);
    }
    let Some(mut data) = bib::parse_entry(raw) else {
        return Ok(None);
    };
    if let Ok(results) = search(&format!("bibcode:{bibcode}"), 1) {
        if let Some(a) = results.first() {
            if !a.abstract_.is_empty() {
                data.insert("abstract".to_string(), clean_abstract(&a.abstract_));
            }
        }
    }
    Ok(Some(data))
}

/// Query the ADS link resolver for a direct PDF URL. Returns None if no
/// token, unknown bibcode, or the link type is unavailable; redirects
/// are NOT followed (the JSON body carries the link).
pub fn resolve_pdf_url(bibcode: &str, link_type: &str) -> Option<String> {
    let token = get_token()?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .redirects(0)
        .build();
    let resp = agent
        .get(&format!("{ADS_API}/resolver/{bibcode}/{link_type}"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .ok()?;
    // this one does not go through check() — it swallows its errors
    // rather than reporting them — but its allowance is metered like
    // any other, so the headers are still read
    update_quota("resolver", &resp);
    if resp.status() != 200 {
        return None;
    }
    let v: serde_json::Value = resp.into_json().ok()?;
    // the resolver sometimes returns a bare identifier (e.g. a DOI like
    // "10.1086/158924") instead of a URL — only absolute URLs are usable
    v["link"]
        .as_str()
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .map(str::to_string)
}

/// Resolve a foreign bib entry to a unique ADS record. Query
/// preference: arXiv ID, then DOI (both unique), then exact title +
/// first author + year, which must match exactly one record. Returns
/// the canonical BibTeX data or a reason.
pub fn lookup_entry(data: &Data) -> Result<Data, String> {
    let get = |k: &str| data.get(k).map(String::as_str).unwrap_or("").trim();
    let eprint = get("eprint");
    let doi = get("doi");
    let (query, needs_unique) = if !eprint.is_empty() {
        let ident = if eprint.to_lowercase().starts_with("arxiv:") {
            eprint.to_string()
        } else {
            format!("arXiv:{eprint}")
        };
        (format!("identifier:\"{ident}\""), false)
    } else if !doi.is_empty() {
        (format!("doi:\"{doi}\""), false)
    } else {
        let title: String = get("title").chars().filter(|c| !"{}\"".contains(*c)).collect();
        let title = title.trim().to_string();
        let last: String = get("author")
            .split(" and ")
            .next()
            .unwrap_or("")
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .filter(|c| !"{}\\".contains(*c))
            .collect();
        let year = get("year").to_string();
        if title.is_empty() || last.is_empty() || year.is_empty() {
            return Err(
                "not enough information for an unambiguous ADS query (need arXiv ID, DOI, or title+author+year)"
                    .to_string(),
            );
        }
        (format!("title:\"{title}\" author:\"^{last}\" year:{year}"), true)
    };
    let results = search(&query, 2).map_err(|e| format!("ADS lookup failed: {e}"))?;
    if results.is_empty() {
        return Err(format!("no ADS match for {query}"));
    }
    if needs_unique && results.len() > 1 {
        return Err(format!("ambiguous — multiple ADS matches for {query}"));
    }
    match fetch_bibtex(&results[0].bibcode) {
        Ok(Some(d)) => Ok(d),
        Ok(None) => Err(format!("could not fetch BibTeX for {}", results[0].bibcode)),
        Err(e) => Err(format!("could not fetch BibTeX for {}: {e}", results[0].bibcode)),
    }
}

/// The arXiv ID among an article's identifiers, if any (entries look
/// like "arXiv:2405.12345").
pub fn arxiv_id(article: &Article) -> Option<&str> {
    article
        .identifier
        .iter()
        .find_map(|i| i.strip_prefix("arXiv:"))
}

/// Strip HTML tags and LaTeX braces from abstract text (display-only field).
pub fn clean_abstract(text: &str) -> String {
    static TAG: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| Regex::new(r"<[^>]+>").unwrap());
    let text = tag.replace_all(text, "");
    let text = text.replace(['{', '}'], "");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract the bibcode from a pasted ADS abstract URL, or None.
pub fn bibcode_from_url(text: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?:https?://)?(?:ui\.)?adsabs\.harvard\.edu/abs/([^/?#\s]+)").unwrap()
    });
    let m = re.captures(text.trim())?;
    Some(percent_decode(&m[1]))
}

/// Extract the DOI from a pasted doi.org URL, doi: prefix, or bare DOI —
/// whole-string matches only, so a DOI inside prose never matches.
pub fn doi_from_text(text: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"^(?i)(?:(?:https?://)?(?:dx\.)?doi\.org/|doi:\s*)?(10\.\d{4,9}/[^\s"]+)$"#)
            .unwrap()
    });
    let m = re.captures(text.trim())?;
    Some(percent_decode(&m[1]))
}

/// What a string typed or pasted where a query goes actually names.
///
/// A journal link is a paper, not a search: the reader who copies the
/// address bar of the page they are reading means "this one", and the
/// only thing standing between that URL and the record is an identifier
/// hiding somewhere in the path. `Bibcode` names one ADS record and is
/// imported outright; `Query` is the fielded query that finds the paper,
/// so the result page still shows what was matched before anything is
/// written; `UnknownUrl` is plainly a link that no rule here can turn
/// into a paper, which is worth saying rather than sending to ADS as
/// search text — a URL is never a query, so the search would fail
/// obscurely instead of failing where the reader can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Paste {
    Bibcode(String),
    Query(String),
    UnknownUrl,
}

/// Identify the paper a string names, or None when it is ordinary query
/// text. Rules run most-specific first, and every one of them is local:
/// nothing here fetches a page to find out what it is about.
pub fn paper_from_text(text: &str) -> Option<Paste> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(bc) = bibcode_from_url(t) {
        return Some(Paste::Bibcode(bc));
    }
    // Any other adsabs link is the search page, whose limit and sort a
    // paste reads in full (`parse_search_url`); a DOI sitting in its
    // query string is the text of a search, not the subject of one.
    if t.to_ascii_lowercase().contains("adsabs.harvard.edu") {
        return None;
    }
    if let Some(id) = arxiv_from_url(t) {
        return Some(Paste::Query(format!("identifier:\"arXiv:{id}\"")));
    }
    if let Some(doi) =
        doi_from_text(t).or_else(|| oup_doi(t)).or_else(|| doi_from_url(t)).or_else(|| nature_doi(t))
    {
        return Some(Paste::Query(format!("doi:\"{doi}\"")));
    }
    if let Some(q) = oup_query(t) {
        return Some(Paste::Query(q));
    }
    looks_like_url(t).then_some(Paste::UnknownUrl)
}

/// Resolve a query that is supposed to name one paper to that paper's
/// bibcode. Used where a bibcode is the only currency — `astrobib add`
/// — and deliberately strict: two matches means the URL identified a
/// paper less exactly than it appeared to.
pub fn unique_bibcode(query: &str) -> std::result::Result<String, String> {
    let results = search(query, 2).map_err(|e| format!("ADS lookup failed: {e}"))?;
    match results.len() {
        0 => Err(format!("no ADS match for {query}")),
        1 => Ok(results[0].bibcode.clone()),
        _ => Err(format!("ambiguous — more than one ADS match for {query}")),
    }
}

/// True for text shaped like a web address: a scheme, a `www.`, or a
/// dotted host followed by a path. A query never looks like this — the
/// filter and ADS languages are `field:value`, and a bare bibcode has no
/// slash — so the shape is what lets an unrecognized link be reported
/// as one instead of searched for.
fn looks_like_url(text: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:https?://|www\.|[a-z0-9-]+(?:\.[a-z0-9-]+)+/)").unwrap()
    });
    re.is_match(text.trim())
}

/// The DOI carried whole in a URL's path — the shape most publishers
/// use (`/doi/10.1126/…`, `iopscience…/article/10.3847/…`,
/// `journals.aps.org/prd/abstract/10.1103/…`). A host is required
/// before it, so a DOI mentioned in prose is not one of these.
fn doi_from_url(url: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:https?://)?[a-z0-9-]+(?:\.[a-z0-9-]+)+/\S*?(10\.\d{4,9}/[^\s?#]+)")
            .unwrap()
    });
    let m = re.captures(url.trim())?;
    let doi = trim_view_suffix(&percent_decode(&m[1]));
    (!doi.is_empty()).then_some(doi)
}

/// Publishers hang a view on the end of the DOI path — `/meta` at IOP,
/// `/full` and `/epdf` at Wiley. None of it belongs to the identifier.
fn trim_view_suffix(doi: &str) -> String {
    const VIEWS: [&str; 10] = [
        "/meta",
        "/fulltext",
        "/full",
        "/abstract",
        "/abs",
        "/epdf",
        "/pdf",
        "/html",
        "/references",
        "/citations",
    ];
    let mut s = doi.trim_end_matches('/');
    loop {
        let lower = s.to_ascii_lowercase();
        let Some(view) = VIEWS.iter().find(|v| lower.ends_with(**v)) else { break };
        s = s[..s.len() - view.len()].trim_end_matches('/');
    }
    s.to_string()
}

/// Nature keeps the DOI out of the URL, but its article id *is* the
/// DOI's suffix under Springer Nature's prefix: the page you are reading
/// at `nature.com/articles/s41586-026-10846-4` is `10.1038/` that id.
fn nature_doi(url: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:https?://)?(?:www\.)?nature\.com/articles/([a-z0-9._-]+)").unwrap()
    });
    let m = re.captures(url.trim())?;
    let id = m[1].trim_end_matches(".pdf").trim_end_matches('.');
    (!id.is_empty()).then(|| format!("10.1038/{id}"))
}

/// An arXiv link names the eprint, which ADS indexes as an identifier —
/// and identifies the paper whether or not it has since been published,
/// which is the reason to prefer it over anything else in the URL.
fn arxiv_from_url(url: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:https?://)?(?:www\.)?arxiv\.org/(?:abs|pdf|html)/([^\s?#]+)").unwrap()
    });
    static VER: OnceLock<Regex> = OnceLock::new();
    let ver = VER.get_or_init(|| Regex::new(r"(?i)v\d+$").unwrap());
    let m = re.captures(url.trim())?;
    let id = m[1].trim_end_matches('/');
    let id = id.strip_suffix(".pdf").unwrap_or(id);
    // a version is a copy of the paper, not a different one; ADS indexes
    // the base identifier
    let id = ver.replace(id, "");
    (!id.is_empty()).then(|| id.into_owned())
}

/// A paper published ahead of an issue has no volume or page yet, so
/// Oxford puts the DOI in the URL instead — followed by its own article
/// id, which the generic path scan would swallow as more DOI. Oxford's
/// DOIs are exactly `10.xxxx/<journal>/<id>`, so the boundary is known
/// here in a way it is not in general: a DOI may contain any number of
/// slashes, and nothing in the path says which of them ended it.
fn oup_doi(url: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)^(?:https?://)?(?:www\.)?academic\.oup\.com/[a-z]+/(?:advance-article|article|article-abstract|article-pdf)/doi/(10\.\d{4,9}/[^/\s?#]+/[^/\s?#]+)",
        )
        .unwrap()
    });
    let m = re.captures(url.trim())?;
    Some(percent_decode(&m[1]))
}

/// Oxford's URLs carry no DOI, but they do carry the citation:
/// `/mnras/article/512/3/3706/…` is MNRAS volume 512, page 3706, which
/// ADS answers exactly. Journals outside the map fall through to
/// `UnknownUrl` rather than guessing a bibstem.
fn oup_query(url: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)^(?:https?://)?(?:www\.)?academic\.oup\.com/([a-z]+)/(?:article|article-abstract|article-pdf|article-lookup|advance-article)/(\d+)/\d+/([a-z]?\d+)",
        )
        .unwrap()
    });
    let m = re.captures(url.trim())?;
    let bibstem = match m[1].to_ascii_lowercase().as_str() {
        "mnras" | "mnrasl" => "MNRAS",
        "pasj" => "PASJ",
        _ => return None,
    };
    Some(format!("bibstem:{bibstem} volume:{} page:{}", &m[2], &m[3]))
}

// ── astrobib update helpers (pure; the network loop lives in main.rs) ──

/// True for arXiv-only records (bibcodes of the form 2024arXiv...).
pub fn is_preprint_bibcode(bibcode: &str) -> bool {
    let b = bibcode.as_bytes();
    b.len() >= 9 && b[..4].iter().all(u8::is_ascii_digit) && &b[4..9] == b"arXiv"
}

/// Candidate selection for `astrobib update`: the bibcode to check when
/// the entry has an ADS URL and is preprint-form with an eprint — or any
/// ADS record at all under `all`.
pub fn update_candidate(adsurl: &str, eprint: &str, all: bool) -> Option<String> {
    let bc = bibcode_from_url(adsurl)?;
    if all || (!eprint.is_empty() && is_preprint_bibcode(&bc)) {
        Some(bc)
    } else {
        None
    }
}

/// The ADS query that re-locates an entry's record: by arXiv ID when the
/// entry has one (it survives publication), else by current bibcode.
pub fn refresh_query(eprint: &str, bibcode: &str) -> String {
    if !eprint.is_empty() {
        format!("identifier:\"arXiv:{eprint}\"")
    } else {
        format!("identifier:\"{bibcode}\"")
    }
}

/// "journal volume pages" venue display for a newly published record,
/// used in the update command's report line; leading backslashes on the
/// journal (ADS macros like \apj) are stripped.
pub fn published_where(data: &Data) -> String {
    let get = |k: &str| data.get(k).map(String::as_str).unwrap_or("");
    [get("journal").trim_start_matches('\\'), get("volume"), get("pages")]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode for a URL query value: RFC 3986 unreserved characters
/// pass through, everything else goes out as `%XX` per UTF-8 byte.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `percent_decode`, plus the `+`-means-space convention that query
/// strings use and paths do not.
fn percent_decode_query(s: &str) -> String {
    percent_decode(&s.replace('+', " "))
}

/// The ADS UI's own search URL: everything a saved query *is*, on one
/// pasteable line.
///
/// This exists because `q` cannot carry the rest. ADS query syntax is
/// Solr and has no comment token, and the result limit and the ordering
/// were never query syntax to begin with — they are the `rows` and
/// `sort` API parameters, which is the same reason `sort:"entry_date
/// desc"` inside `q` is an error rather than a sort. A URL is the one
/// format that holds all three, and ADS emits it itself.
pub fn search_url(query: &str, rows: usize, sort: &str) -> String {
    format!(
        "https://ui.adsabs.harvard.edu/search/q={}&rows={}&sort={}",
        percent_encode(query),
        rows,
        percent_encode(sort)
    )
}

/// The inverse of `search_url`, lenient enough to also accept a URL
/// copied out of the ADS website — which carries parameters we do not
/// write (`p_`, `sort` alone) and omits ones we do.
///
/// `rows` and `sort` come back as None when the URL does not say, so the
/// caller keeps whatever it already had rather than inventing a default.
pub fn parse_search_url(text: &str) -> Option<(String, Option<usize>, Option<String>)> {
    let t = text.trim();
    let i = t.find("adsabs.harvard.edu/search")?;
    let rest = &t[i + "adsabs.harvard.edu/search".len()..];
    // ours writes /search/q=…; the website has used ?q=… as well
    let rest = rest.strip_prefix('/').or_else(|| rest.strip_prefix('?')).unwrap_or(rest);
    let (mut query, mut rows, mut sort) = (None, None, None);
    for pair in rest.split('&') {
        let Some((k, v)) = pair.split_once('=') else { continue };
        match k {
            "q" => query = Some(percent_decode_query(v)),
            "rows" => rows = percent_decode_query(v).parse::<usize>().ok(),
            "sort" => sort = Some(percent_decode_query(v)),
            _ => {}
        }
    }
    let query = query?;
    if query.is_empty() {
        return None;
    }
    Some((query, rows, sort))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_url_round_trips() {
        let q = r#"abs:"little red dot" -doctype:abstract"#;
        let url = search_url(q, 50, "citation_count desc");
        let (q2, rows, sort) = parse_search_url(&url).unwrap();
        assert_eq!(q2, q);
        assert_eq!(rows, Some(50));
        assert_eq!(sort.as_deref(), Some("citation_count desc"));
        // the query has to survive as one parameter: a bare space or a
        // stray & would split it and silently truncate the query
        assert!(!url.contains(' '));
        assert_eq!(url.matches('&').count(), 2);
    }

    #[test]
    fn parses_a_url_from_the_ads_website() {
        // the website writes its own parameter set: no rows, a sort we
        // do not offer, and trailing state of its own
        let (q, rows, sort) = parse_search_url(
            "https://ui.adsabs.harvard.edu/search/q=author%3A%22Zrake%22&sort=date+desc&p_=0",
        )
        .unwrap();
        assert_eq!(q, r#"author:"Zrake""#);
        assert_eq!(rows, None); // absent, so the caller keeps its own
        assert_eq!(sort.as_deref(), Some("date desc"));
    }

    #[test]
    fn non_search_urls_are_not_queries() {
        // an abstract link is a paper, not a query; and a query string
        // that merely mentions ADS is text the user typed
        assert!(parse_search_url("https://ui.adsabs.harvard.edu/abs/2019ApJ...123..456Z").is_none());
        assert!(parse_search_url("abs:\"adsabs.harvard.edu/search\"").is_none());
        assert!(parse_search_url("https://ui.adsabs.harvard.edu/search/q=").is_none());
    }

    #[test]
    fn doi_extraction() {
        let doi = "10.1038/s41586-025-09900-4";
        for form in [
            format!("https://doi.org/{doi}"),
            format!("http://dx.doi.org/{doi}"),
            format!("doi.org/{doi}"),
            format!("doi:{doi}"),
            doi.to_string(),
        ] {
            assert_eq!(doi_from_text(&form).as_deref(), Some(doi), "{form}");
        }
        assert_eq!(doi_from_text("kilonova doi 10.1038 x"), None);
        assert_eq!(doi_from_text("author:^andersson year:2020-"), None);
        assert_eq!(
            doi_from_text("https://doi.org/10.3847/1538-4357%2Faa8b0e").as_deref(),
            Some("10.3847/1538-4357/aa8b0e")
        );
    }

    #[test]
    fn bibcode_extraction() {
        assert_eq!(
            bibcode_from_url("https://ui.adsabs.harvard.edu/abs/2019ApJ...123..456Z/abstract")
                .as_deref(),
            Some("2019ApJ...123..456Z")
        );
        assert_eq!(bibcode_from_url("plain text"), None);
    }

    #[test]
    fn journal_urls_carry_their_doi() {
        // the DOI sits whole in the path at most publishers
        for (url, doi) in [
            ("https://www.science.org/doi/10.1126/science.abc1234", "10.1126/science.abc1234"),
            (
                "https://iopscience.iop.org/article/10.3847/1538-4357/ad1234/meta",
                "10.3847/1538-4357/ad1234",
            ),
            (
                "https://journals.aps.org/prd/abstract/10.1103/PhysRevD.109.023001",
                "10.1103/PhysRevD.109.023001",
            ),
            ("https://onlinelibrary.wiley.com/doi/full/10.1002/qj.4801", "10.1002/qj.4801"),
            ("https://link.springer.com/article/10.1007/s11214-024-01055-4", "10.1007/s11214-024-01055-4"),
            (
                "https://academic.oup.com/mnras/advance-article/doi/10.1093/mnras/stae123/7612345",
                "10.1093/mnras/stae123",
            ),
        ] {
            assert_eq!(
                paper_from_text(url),
                Some(Paste::Query(format!("doi:\"{doi}\""))),
                "{url}"
            );
        }
        // a DOI in prose has no host in front of it and is not a link
        assert_eq!(doi_from_url("see 10.1126/science.abc1234 for the method"), None);
    }

    #[test]
    fn nature_article_ids_are_dois() {
        assert_eq!(
            paper_from_text("https://www.nature.com/articles/s41586-026-10846-4"),
            Some(Paste::Query("doi:\"10.1038/s41586-026-10846-4\"".to_string()))
        );
        // the PDF of the same page names the same paper
        assert_eq!(
            nature_doi("nature.com/articles/s41550-024-02244-5.pdf").as_deref(),
            Some("10.1038/s41550-024-02244-5")
        );
    }

    #[test]
    fn arxiv_urls_name_the_eprint() {
        for url in [
            "https://arxiv.org/abs/2405.12345",
            "arxiv.org/abs/2405.12345v3",
            "https://arxiv.org/pdf/2405.12345v2.pdf",
        ] {
            assert_eq!(
                paper_from_text(url),
                Some(Paste::Query("identifier:\"arXiv:2405.12345\"".to_string())),
                "{url}"
            );
        }
        // the old identifiers carry a slash of their own
        assert_eq!(
            arxiv_from_url("https://arxiv.org/abs/astro-ph/0601001v2").as_deref(),
            Some("astro-ph/0601001")
        );
    }

    #[test]
    fn oup_urls_become_the_citation() {
        assert_eq!(
            paper_from_text("https://academic.oup.com/mnras/article/512/3/3706/6553842"),
            Some(Paste::Query("bibstem:MNRAS volume:512 page:3706".to_string()))
        );
        // letters keep the page's letter, and the abstract view is the
        // same article
        assert_eq!(
            oup_query("academic.oup.com/mnrasl/article-abstract/491/1/L44/5637388").as_deref(),
            Some("bibstem:MNRAS volume:491 page:L44")
        );
        // a journal with no bibstem in the map is not guessed at
        assert_eq!(oup_query("https://academic.oup.com/gji/article/236/2/1234/7654321"), None);
    }

    #[test]
    fn unidentifiable_links_are_reported_not_searched() {
        // A&A's own URLs name a manuscript number ADS does not index,
        // and ScienceDirect names a PII — both are links, and saying so
        // is better than sending the URL to ADS as search text
        for url in [
            "https://www.aanda.org/articles/aa/full_html/2024/06/aa48123-23/aa48123-23.html",
            "https://www.sciencedirect.com/science/article/pii/S0019103524001234",
        ] {
            assert_eq!(paper_from_text(url), Some(Paste::UnknownUrl), "{url}");
        }
        // queries are not links, and neither is a bare bibcode
        for text in [
            "author:^andersson year:2020-",
            "2019ApJ...123..456Z",
            "title:\"fast radio bursts\"",
        ] {
            assert_eq!(paper_from_text(text), None, "{text}");
        }
    }

    #[test]
    fn ads_links_keep_their_meanings() {
        // an abstract link is the paper itself
        assert_eq!(
            paper_from_text("https://ui.adsabs.harvard.edu/abs/2019ApJ...123..456Z/abstract"),
            Some(Paste::Bibcode("2019ApJ...123..456Z".to_string()))
        );
        // a search URL is a query someone saved, even when a DOI is the
        // thing it searches for — the paste handler reads its rows and
        // sort, which this path would throw away
        assert_eq!(
            paper_from_text(
                "https://ui.adsabs.harvard.edu/search/q=doi%3A10.1126%2Fscience.abc1234&rows=50"
            ),
            None
        );
    }

    #[test]
    fn preprint_bibcodes() {
        assert!(is_preprint_bibcode("2024arXiv240512345Z"));
        assert!(!is_preprint_bibcode("2024ApJ...123..456Z"));
        assert!(!is_preprint_bibcode("arXiv2024"));
        assert!(!is_preprint_bibcode("2024arXi"));
        assert!(!is_preprint_bibcode(""));
    }

    #[test]
    fn update_candidates() {
        let pre = "https://ui.adsabs.harvard.edu/abs/2024arXiv240512345Z";
        // preprint-form entry with an eprint: always a candidate
        assert_eq!(
            update_candidate(pre, "2405.12345", false).as_deref(),
            Some("2024arXiv240512345Z")
        );
        // no eprint field: only under --all
        assert_eq!(update_candidate(pre, "", false), None);
        assert_eq!(update_candidate(pre, "", true).as_deref(), Some("2024arXiv240512345Z"));
        // already-published bibcode: only under --all
        let published = "https://ui.adsabs.harvard.edu/abs/2019ApJ...123..456Z/abstract";
        assert_eq!(update_candidate(published, "1901.00001", false), None);
        assert_eq!(
            update_candidate(published, "1901.00001", true).as_deref(),
            Some("2019ApJ...123..456Z")
        );
        // no ADS URL: never a candidate
        assert_eq!(update_candidate("", "2405.12345", true), None);
    }

    #[test]
    fn refresh_queries() {
        assert_eq!(
            refresh_query("2405.12345", "2024arXiv240512345Z"),
            "identifier:\"arXiv:2405.12345\""
        );
        assert_eq!(
            refresh_query("", "2024arXiv240512345Z"),
            "identifier:\"2024arXiv240512345Z\""
        );
    }

    #[test]
    fn published_where_formatting() {
        let mut d = Data::new();
        d.insert("journal".into(), "\\apj".into());
        d.insert("volume".into(), "973".into());
        d.insert("pages".into(), "12".into());
        assert_eq!(published_where(&d), "apj 973 12");
        d.shift_remove("volume");
        assert_eq!(published_where(&d), "apj 12");
        assert_eq!(published_where(&Data::new()), "");
    }

    #[test]
    fn abstract_cleaning() {
        assert_eq!(
            clean_abstract("Fast <SUB>x</SUB> {winds}\n  here"),
            "Fast x winds here"
        );
    }
}
