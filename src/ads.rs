//! Direct ADS API client: search, BibTeX export, link resolver, and
//! rate-limit quota tracking.

use crate::bib::{self, Data};
use anyhow::{anyhow, bail, Result};
use regex::Regex;
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

/// Persist one field into state.json (creating it if absent),
/// preserving every other field.
pub fn save_state_field(key: &str, value: &str) -> std::io::Result<()> {
    let path = state_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut v: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({ "version": 1 }));
    v[key] = serde_json::Value::String(value.to_string());
    std::fs::write(&path, serde_json::to_string_pretty(&v)? + "\n")
}

/// The same, for a field whose value is structured rather than a
/// string — the per-scope column configuration, for instance.
pub fn save_state_value(key: &str, value: serde_json::Value) -> std::io::Result<()> {
    let path = state_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut v: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({ "version": 1 }));
    v[key] = value;
    std::fs::write(&path, serde_json::to_string_pretty(&v)? + "\n")
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

fn check(resp: std::result::Result<ureq::Response, ureq::Error>) -> Result<ureq::Response> {
    match resp {
        Ok(r) => {
            update_quota(&r);
            Ok(r)
        }
        Err(ureq::Error::Status(code, r)) => {
            update_quota(&r);
            let detail = r.into_string().unwrap_or_default();
            bail!("ADS API error {code}: {}", detail.trim().chars().take(200).collect::<String>())
        }
        Err(e) => bail!("ADS request failed: {e}"),
    }
}

/// Rate-limit state from the most recent ADS response.
#[derive(Clone, Copy, Debug)]
pub struct Quota {
    pub limit: i64,
    pub remaining: i64,
    pub reset: i64,
}

static QUOTA: Mutex<Option<Quota>> = Mutex::new(None);

/// The quota captured from the last API round-trip, if any.
pub fn get_quota() -> Option<Quota> {
    *QUOTA.lock().unwrap()
}

/// Parse the X-RateLimit-* headers into the quota slot: only when the
/// Limit header is present, and skipped entirely if any present header
/// fails to parse (missing ones read 0).
fn update_quota(resp: &ureq::Response) {
    if resp.header("X-RateLimit-Limit").is_none() {
        return;
    }
    let h = |name: &str| resp.header(name).unwrap_or("0").parse::<i64>().ok();
    if let (Some(limit), Some(remaining), Some(reset)) = (
        h("X-RateLimit-Limit"),
        h("X-RateLimit-Remaining"),
        h("X-RateLimit-Reset"),
    ) {
        *QUOTA.lock().unwrap() = Some(Quota { limit, remaining, reset });
    }
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
