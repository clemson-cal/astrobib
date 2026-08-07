//! Local filter query language for the library view.
//!
//! Grammar: whitespace-separated terms AND together; uppercase OR separates
//! alternative groups (AND binds tighter than OR, as in ADS). Uppercase-only
//! operators: AND is a no-op, NOT aliases the '-' prefix. A bare ^name term
//! is sugar for author:^name. Unknown fields degrade to bare terms and
//! dangling operators are ignored — a half-typed query never errors.

use crate::library::Entry;
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub neg: bool,
    pub field: Option<Field>,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Author,
    Title,
    Abs,
    Key,
    Kw,
    Year,
    Is,
    Pri,
    Cit,
    Tag,
}

fn canon_field(name: &str) -> Option<Field> {
    Some(match name.to_lowercase().as_str() {
        "author" => Field::Author,
        "title" => Field::Title,
        "abs" | "abstract" => Field::Abs,
        "key" => Field::Key,
        "kw" | "keyword" | "keywords" => Field::Kw,
        "year" => Field::Year,
        "is" => Field::Is,
        "pri" | "priority" => Field::Pri,
        "cit" | "cites" | "citations" => Field::Cit,
        "tag" | "tags" => Field::Tag,
        _ => return None,
    })
}

fn token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(-?)(?:([A-Za-z]+):)?("[^"]*"?|[^\s"]+)"#).unwrap())
}

/// Split a filter string into OR-groups of terms.
pub fn tokenize(text: &str) -> Vec<Vec<Term>> {
    let mut groups: Vec<Vec<Term>> = vec![vec![]];
    let mut pending_not = false;
    for cap in token_re().captures_iter(text) {
        let raw = cap.get(0).unwrap().as_str();
        match raw {
            "OR" => {
                if !groups.last().unwrap().is_empty() {
                    groups.push(vec![]);
                }
                continue;
            }
            "AND" => continue,
            "NOT" => {
                pending_not = true;
                continue;
            }
            _ => {}
        }
        let neg = &cap[1] == "-" || pending_not;
        pending_not = false;
        let mut value = cap[3].trim_matches('"').to_string();
        let mut field = None;
        if let Some(fname) = cap.get(2) {
            match canon_field(fname.as_str()) {
                Some(f) => field = Some(f),
                // unknown field: the whole token is a bare term
                None => value = format!("{}:{}", fname.as_str(), value),
            }
        }
        if field.is_none() && value.starts_with('^') && value.len() > 1 {
            field = Some(Field::Author); // bare ^name is first-author sugar
        }
        if !value.is_empty() || field.is_some() {
            groups.last_mut().unwrap().push(Term { neg, field, value });
        }
    }
    groups.retain(|g| !g.is_empty());
    groups
}

fn year_matches(value: &str, entry_year: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let range_re = RE.get_or_init(|| Regex::new(r"^(\d{4})?-(\d{4})?$").unwrap());
    if let Some(m) = range_re.captures(value) {
        let lo = m.get(1).map(|g| g.as_str().parse::<i32>().unwrap());
        let hi = m.get(2).map(|g| g.as_str().parse::<i32>().unwrap());
        if lo.is_some() || hi.is_some() {
            let y: i32 = match entry_year.parse() {
                Ok(y) => y,
                Err(_) => return false,
            };
            return lo.map_or(true, |l| y >= l) && hi.map_or(true, |h| y <= h);
        }
    }
    if value.len() == 4 && value.chars().all(|c| c.is_ascii_digit()) {
        return entry_year == value;
    }
    entry_year.contains(value)
}

/// Context for the is:ms / is:pdf terms; a term with no context matches
/// nothing, mirroring compile_query's optional callbacks.
#[derive(Default)]
pub struct QueryContext {
    pub in_manuscript: Option<Box<dyn Fn(&str) -> bool>>,
    pub has_pdf: Option<Box<dyn Fn(&str) -> bool>>,
    pub priority: Option<Box<dyn Fn(&str) -> Option<f64>>>,
    pub citations: Option<Box<dyn Fn(&str) -> Option<f64>>>,
    /// (key, tag substring) — true when the key carries a matching tag.
    pub tagged: Option<Box<dyn Fn(&str, &str) -> bool>>,
}

/// pri:>0.5 / cit:<100 — a comparison against a metric; a missing
/// metric never matches. Bare pri:0.5 means >=.
fn metric_matches(spec: &str, actual: Option<f64>) -> bool {
    let Some(actual) = actual else { return false };
    let (op, num) = match spec.as_bytes().first() {
        Some(b'>') => ('>', &spec[1..]),
        Some(b'<') => ('<', &spec[1..]),
        _ => ('.', spec),
    };
    let Ok(threshold) = num.trim_start_matches('=').parse::<f64>() else {
        return false;
    };
    match op {
        '>' => actual > threshold,
        '<' => actual < threshold,
        _ => actual >= threshold,
    }
}

fn term_matches(term: &Term, e: &Entry, ctx: &QueryContext) -> bool {
    let doc = e.search_doc();
    let v = term.value.to_lowercase();
    let hit = match term.field {
        Some(Field::Year) => year_matches(&term.value, &e.year()),
        Some(Field::Author) => {
            if let Some(first) = v.strip_prefix('^') {
                doc.first.starts_with(first)
            } else {
                doc.author.contains(&v)
            }
        }
        Some(Field::Title) => doc.title.contains(&v),
        Some(Field::Abs) => doc.abs.contains(&v),
        Some(Field::Key) => doc.key.contains(&v),
        Some(Field::Kw) => doc.kw.contains(&v),
        Some(Field::Is) => match v.as_str() {
            "ms" => ctx.in_manuscript.as_ref().is_some_and(|f| f(e.key())),
            "pdf" => ctx.has_pdf.as_ref().is_some_and(|f| f(e.key())),
            _ => false,
        },
        Some(Field::Pri) => {
            metric_matches(&term.value, ctx.priority.as_ref().and_then(|f| f(e.key())))
        }
        Some(Field::Cit) => {
            metric_matches(&term.value, ctx.citations.as_ref().and_then(|f| f(e.key())))
        }
        Some(Field::Tag) => ctx.tagged.as_ref().is_some_and(|f| f(e.key(), &v)),
        None => doc.all.contains(&v),
    };
    hit != term.neg
}

/// True when the entry matches the filter: any OR-group whose terms all pass.
pub fn matches(groups: &[Vec<Term>], e: &Entry, ctx: &QueryContext) -> bool {
    if groups.is_empty() {
        return true;
    }
    groups
        .iter()
        .any(|g| g.iter().all(|t| term_matches(t, e, ctx)))
}

/// Translate a local filter string into an ADS search query: drops
/// is:/key:/tag:/negations, quotes field values, maps kw: to keyword:,
/// parenthesizes multi-term OR-groups. tag: goes with is: rather than
/// with kw:: a tag is astrobib's own grouping of your library, and ADS
/// has never heard of it — sending it as a keyword would silently
/// return the wrong papers rather than nothing.
pub fn to_ads_query(text: &str) -> String {
    let mut groups: Vec<Vec<String>> = vec![];
    for group in tokenize(text) {
        let mut parts = vec![];
        for t in group {
            if t.neg
                || matches!(
                    t.field,
                    Some(Field::Is)
                        | Some(Field::Key)
                        | Some(Field::Pri)
                        | Some(Field::Cit)
                        | Some(Field::Tag)
                )
                || t.value.is_empty()
            {
                continue;
            }
            parts.push(match t.field {
                Some(Field::Year) => format!("year:{}", t.value),
                Some(Field::Author) => format!("author:\"{}\"", t.value),
                Some(Field::Title) => format!("title:\"{}\"", t.value),
                Some(Field::Abs) => format!("abs:\"{}\"", t.value),
                Some(Field::Kw) => format!("keyword:\"{}\"", t.value),
                _ if t.value.contains(' ') => format!("\"{}\"", t.value),
                _ => t.value,
            });
        }
        if !parts.is_empty() {
            groups.push(parts);
        }
    }
    if groups.len() > 1 {
        groups
            .iter()
            .map(|p| {
                if p.len() == 1 {
                    p[0].clone()
                } else {
                    format!("({})", p.join(" "))
                }
            })
            .collect::<Vec<_>>()
            .join(" OR ")
    } else {
        groups.first().map(|p| p.join(" ")).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bib::Data;
    use crate::library::Entry;
    use std::path::PathBuf;

    fn entry(first: &str, author: &str, title: &str, year: &str) -> Entry {
        let mut d = Data::new();
        d.insert("ID".to_string(), format!("{first}{year}aaaaa"));
        d.insert("ENTRYTYPE".to_string(), "article".to_string());
        d.insert("author".to_string(), format!("{author}"));
        d.insert("title".to_string(), title.to_string());
        d.insert("year".to_string(), year.to_string());
        Entry::new(d, PathBuf::new())
    }

    fn names(q: &str) -> Vec<&'static str> {
        let andersson = entry("Andersson", "Andersson, K.", "relativistic turbulence", "2019");
        let baxter = entry("Baxter", "Baxter, R.", "kilonovae", "2017");
        let cabrera = entry("Cabrera", "Cabrera, M.", "reconnection or shocks", "2021");
        let ctx = QueryContext::default();
        let g = tokenize(q);
        [("andersson", andersson), ("baxter", baxter), ("cabrera", cabrera)]
            .into_iter()
            .filter(|(_, e)| matches(&g, e, &ctx))
            .map(|(n, _)| n)
            .collect()
    }

    #[test]
    fn tag_terms_need_a_context_and_match_by_substring() {
        let e = entry("Andersson", "Andersson, K.", "relativistic turbulence", "2019");
        let key = e.key().to_string();
        let ctx = QueryContext {
            tagged: Some(Box::new(move |k: &str, v: &str| {
                k == key && "section-3".contains(v)
            })),
            ..Default::default()
        };
        assert!(matches(&tokenize("tag:section-3"), &e, &ctx));
        assert!(matches(&tokenize("tag:section"), &e, &ctx)); // substring, like kw:
        assert!(!matches(&tokenize("tag:disks"), &e, &ctx));
        assert!(matches(&tokenize("-tag:disks"), &e, &ctx));
        // no context at all: the term matches nothing rather than erroring
        assert!(!matches(&tokenize("tag:section-3"), &e, &QueryContext::default()));
    }

    #[test]
    fn metric_tokens() {
        assert!(super::metric_matches(">0.5", Some(0.7)));
        assert!(!super::metric_matches(">0.5", Some(0.5)));
        assert!(super::metric_matches("<100", Some(42.0)));
        assert!(!super::metric_matches("<100", None));
        assert!(super::metric_matches("0.5", Some(0.5)));
        assert!(!super::metric_matches("junk", Some(1.0)));
    }

    #[test]
    fn or_of_first_author_groups() {
        assert_eq!(names("author:^andersson OR author:^baxter"), ["andersson", "baxter"]);
        assert_eq!(names("^andersson OR ^baxter"), ["andersson", "baxter"]);
    }

    #[test]
    fn and_binds_tighter_than_or() {
        assert_eq!(names("year:2019 ^andersson OR ^cabrera"), ["andersson", "cabrera"]);
        assert_eq!(names("^andersson year:2020- OR ^cabrera"), ["cabrera"]);
    }

    #[test]
    fn lowercase_operators_are_bare_terms() {
        assert_eq!(names("or"), ["cabrera"]);
    }

    #[test]
    fn explicit_and_and_not() {
        assert_eq!(names("^cabrera AND year:2021"), ["cabrera"]);
        assert_eq!(names("NOT ^andersson"), ["baxter", "cabrera"]);
        assert_eq!(names("NOT ^andersson NOT ^cabrera"), ["baxter"]);
    }

    #[test]
    fn dangling_operators_never_error() {
        assert_eq!(names("^andersson OR"), ["andersson"]);
        assert_eq!(names("OR ^andersson"), ["andersson"]);
        assert_eq!(names("^andersson NOT"), ["andersson"]);
        assert_eq!(names(""), ["andersson", "baxter", "cabrera"]);
    }

    #[test]
    fn negation_inside_group() {
        assert_eq!(
            names("year:2015- -^baxter OR ^baxter"),
            ["andersson", "baxter", "cabrera"]
        );
    }

    #[test]
    fn year_ranges() {
        assert_eq!(names("year:2015-2020"), ["andersson", "baxter"]);
        assert_eq!(names("year:-2017"), ["baxter"]);
        assert_eq!(names("year:2019-"), ["andersson", "cabrera"]);
    }

    #[test]
    fn ads_translation() {
        assert_eq!(
            to_ads_query("^andersson OR ^baxter"),
            r#"author:"^andersson" OR author:"^baxter""#
        );
        assert_eq!(
            to_ads_query("author:^andersson year:2019-"),
            r#"author:"^andersson" year:2019-"#
        );
        assert_eq!(
            to_ads_query("^andersson year:2019- OR kw:jets"),
            r#"(author:"^andersson" year:2019-) OR keyword:"jets""#
        );
        assert_eq!(to_ads_query("is:ms ^andersson"), r#"author:"^andersson""#);
        assert_eq!(to_ads_query("is:ms OR is:pdf"), "");
        // a tag is astrobib's own grouping; ADS must not be sent it as
        // a keyword, which would answer with the wrong papers
        assert_eq!(to_ads_query("tag:section-3 ^andersson"), r#"author:"^andersson""#);
        assert_eq!(to_ads_query("tag:section-3"), "");
        assert_eq!(to_ads_query(""), "");
        assert_eq!(to_ads_query(r#""gamma ray" OR jets"#), r#""gamma ray" OR jets"#);
    }
}
