//! Collision-resistant cite key generation from BibTeX data.
//!
//! Byte-for-byte port of astrobib/keys.py: keys denote papers for life, so
//! this must generate the identical AuthorYYYYlllll key as the Python
//! implementation for every record either tool has ever seen. Any change
//! here must keep tests/golden_keys.json passing.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

/// Submission year encoded in an arXiv ID, or None if unparseable.
///
/// New style 2405.12345 -> 2024; old style astro-ph/0501001 -> 2005
/// (YY >= 91 means 19YY: arXiv started in 1991).
fn year_from_arxiv_id(eprint: &str) -> Option<String> {
    let e = eprint.trim();
    let e = if e.len() >= 6 && e[..6].eq_ignore_ascii_case("arxiv:") {
        &e[6..]
    } else {
        e
    };
    let new_re = regex::Regex::new(r"^(\d{2})\d{2}\.\d{4,5}(?:v\d+)?$").unwrap();
    if let Some(m) = new_re.captures(e) {
        return Some(format!("20{}", &m[1]));
    }
    let old_re = regex::Regex::new(r"^(?:[A-Za-z.\-]+/)?(\d{2})\d{2}\d+(?:v\d+)?$").unwrap();
    if let Some(m) = old_re.captures(e) {
        let yy: u32 = m[1].parse().unwrap();
        return Some(if yy >= 91 {
            format!("19{yy}")
        } else {
            format!("20{yy:02}")
        });
    }
    None
}

fn get<'a>(data: &'a HashMap<String, String>, k: &str) -> &'a str {
    data.get(k).map(String::as_str).unwrap_or("")
}

/// Return a cite key of the form AuthorYYYYlllll.
///
/// The suffix is 5 lowercase letters derived from SHA-256 applied to a
/// stable identifier: the arXiv ID when present, otherwise the ADS bibcode
/// extracted from adsurl. The year derives from the same identifier, so
/// preprint and published records yield the same key.
pub fn generate_key(data: &HashMap<String, String>) -> String {
    let mut year = get(data, "year").to_string();
    let archive_prefix = {
        let a = get(data, "archivePrefix");
        if a.is_empty() { get(data, "archiveprefix") } else { a }
    };
    let eprint = get(data, "eprint");
    let stable_id: String;
    if archive_prefix.eq_ignore_ascii_case("arxiv") && !eprint.is_empty() {
        stable_id = eprint.to_string();
        if let Some(y) = year_from_arxiv_id(&stable_id) {
            year = y;
        }
    } else {
        let adsurl = get(data, "adsurl");
        stable_id = if adsurl.is_empty() {
            get(data, "ID").to_string()
        } else {
            let trimmed = adsurl.trim_end_matches('/');
            trimmed.rsplit('/').next().unwrap_or(trimmed).to_string()
        };
        // Python: stable_id[:4].isdigit() — a possibly-shorter prefix
        let prefix: String = stable_id.chars().take(4).collect();
        if !adsurl.is_empty() && !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
            year = prefix;
        }
    }

    let h = Sha256::digest(stable_id.as_bytes());
    let suffix: String = h[..5].iter().map(|b| (b'a' + b % 26) as char).collect();

    let author = get(data, "author");
    let first_author = author
        .split(" and ")
        .next()
        .unwrap_or("")
        .trim()
        .split(',')
        .next()
        .unwrap_or("")
        .trim();
    // NFD, then keep letters only (category L*): drops combining marks and
    // punctuation, exactly like Python's `category != Mn and isalpha`.
    let ascii_name: String = first_author
        .nfd()
        .filter(|c| c.general_category_group() == GeneralCategoryGroup::Letter)
        .collect();

    format!("{ascii_name}{year}{suffix}")
}
