//! Direct ADS API client — port of astrobib/ads_client.py (search,
//! BibTeX export, link resolver, rate-limit quota).

use crate::bib::{self, Data};
use anyhow::{anyhow, bail, Result};
use regex::Regex;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const ADS_API: &str = "https://api.adsabs.harvard.edu/v1";

const SEARCH_FIELDS: &str =
    "bibcode,title,author,year,abstract,identifier,doi,esources,arxiv_class,citation_count";

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
}

fn state_file() -> PathBuf {
    let base = std::env::var("ASTROBIB_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/astrobib")
        });
    base.join("state.json")
}

/// ADS token from $ADS_API_TOKEN or saved state — port of state.get_token.
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
            "No ADS API token.\nSet ADS_API_TOKEN or configure the Python astrobib.\n\
             Get one at: https://ui.adsabs.harvard.edu/user/settings/token"
        )
    })
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

/// Rate-limit state from the most recent ADS response — port of the
/// ads_client module-level `_quota`.
#[derive(Clone, Copy, Debug)]
pub struct Quota {
    pub limit: i64,
    pub remaining: i64,
    pub reset: i64,
}

static QUOTA: Mutex<Option<Quota>> = Mutex::new(None);

/// The quota captured from the last API round-trip, if any — port of
/// get_quota.
pub fn get_quota() -> Option<Quota> {
    *QUOTA.lock().unwrap()
}

/// Parse the X-RateLimit-* headers into the quota slot — port of
/// _update_quota: only when the Limit header is present, and skipped
/// entirely if any present header fails to parse (missing ones read 0).
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

pub fn search(query: &str, limit: usize) -> Result<Vec<Article>> {
    let token = require_token()?;
    let resp = check(
        agent()
            .get(&format!("{ADS_API}/search/query"))
            .set("Authorization", &format!("Bearer {token}"))
            .query("q", query)
            .query("fl", SEARCH_FIELDS)
            .query("rows", &limit.to_string())
            .query("sort", "date desc")
            .call(),
    )?;
    let v: serde_json::Value = resp.into_json()?;
    let docs = v["response"]["docs"].as_array().cloned().unwrap_or_default();
    Ok(docs.iter().map(article_from_doc).collect())
}

fn article_from_doc(d: &serde_json::Value) -> Article {
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
    }
}

/// Fetch canonical BibTeX for a bibcode; the export omits the abstract,
/// so it is fetched separately and appended — port of fetch_bibtex.
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

/// Query the ADS link resolver for a direct PDF URL — port of
/// resolve_pdf_url. Returns None if no token, unknown bibcode, or the
/// link type is unavailable; redirects are NOT followed (the JSON body
/// carries the link).
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
    v["link"].as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Resolve a foreign bib entry to a unique ADS record — port of
/// _ads_lookup. Query preference: arXiv ID, then DOI (both unique),
/// then exact title + first author + year, which must match exactly
/// one record. Returns the canonical BibTeX data or a reason.
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
/// whole-string matches only, like the Python port.
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

/// True for arXiv-only records (bibcodes of the form 2024arXiv...) —
/// port of is_preprint_bibcode.
pub fn is_preprint_bibcode(bibcode: &str) -> bool {
    let b = bibcode.as_bytes();
    b.len() >= 9 && b[..4].iter().all(u8::is_ascii_digit) && &b[4..9] == b"arXiv"
}

/// Candidate selection for `astrobib update`: the bibcode to check when
/// the entry has an ADS URL and is preprint-form with an eprint — or any
/// ADS record at all under `all` — port of the update command's filter.
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

/// "journal volume pages" venue display for a newly published record;
/// leading backslashes on the journal (ADS macros like \apj) stripped —
/// port of the update command's report line.
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(doi_from_text("author:^zrake year:2020-"), None);
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
