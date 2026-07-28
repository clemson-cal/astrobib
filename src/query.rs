//! Local filter query language — port of astrobib/query.py.
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

/// Translate a local filter string into an ADS search query — port of
/// to_ads_query: drops is:/key:/negations, quotes field values, maps kw:
/// to keyword:, parenthesizes multi-term OR-groups.
pub fn to_ads_query(text: &str) -> String {
    let mut groups: Vec<Vec<String>> = vec![];
    for group in tokenize(text) {
        let mut parts = vec![];
        for t in group {
            if t.neg || matches!(t.field, Some(Field::Is) | Some(Field::Key)) || t.value.is_empty()
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
        let zrake = entry("Zrake", "Zrake, J.", "relativistic turbulence", "2019");
        let metzger = entry("Metzger", "Metzger, B.", "kilonovae", "2017");
        let sironi = entry("Sironi", "Sironi, L.", "reconnection or shocks", "2021");
        let ctx = QueryContext::default();
        let g = tokenize(q);
        [("zrake", zrake), ("metzger", metzger), ("sironi", sironi)]
            .into_iter()
            .filter(|(_, e)| matches(&g, e, &ctx))
            .map(|(n, _)| n)
            .collect()
    }

    #[test]
    fn or_of_first_author_groups() {
        assert_eq!(names("author:^zrake OR author:^metzger"), ["zrake", "metzger"]);
        assert_eq!(names("^zrake OR ^metzger"), ["zrake", "metzger"]);
    }

    #[test]
    fn and_binds_tighter_than_or() {
        assert_eq!(names("year:2019 ^zrake OR ^sironi"), ["zrake", "sironi"]);
        assert_eq!(names("^zrake year:2020- OR ^sironi"), ["sironi"]);
    }

    #[test]
    fn lowercase_operators_are_bare_terms() {
        assert_eq!(names("or"), ["sironi"]);
    }

    #[test]
    fn explicit_and_and_not() {
        assert_eq!(names("^sironi AND year:2021"), ["sironi"]);
        assert_eq!(names("NOT ^zrake"), ["metzger", "sironi"]);
        assert_eq!(names("NOT ^zrake NOT ^sironi"), ["metzger"]);
    }

    #[test]
    fn dangling_operators_never_error() {
        assert_eq!(names("^zrake OR"), ["zrake"]);
        assert_eq!(names("OR ^zrake"), ["zrake"]);
        assert_eq!(names("^zrake NOT"), ["zrake"]);
        assert_eq!(names(""), ["zrake", "metzger", "sironi"]);
    }

    #[test]
    fn negation_inside_group() {
        assert_eq!(
            names("year:2015- -^metzger OR ^metzger"),
            ["zrake", "metzger", "sironi"]
        );
    }

    #[test]
    fn year_ranges() {
        assert_eq!(names("year:2015-2020"), ["zrake", "metzger"]);
        assert_eq!(names("year:-2017"), ["metzger"]);
        assert_eq!(names("year:2019-"), ["zrake", "sironi"]);
    }

    #[test]
    fn ads_translation() {
        assert_eq!(
            to_ads_query("^zrake OR ^metzger"),
            r#"author:"^zrake" OR author:"^metzger""#
        );
        assert_eq!(
            to_ads_query("author:^zrake year:2019-"),
            r#"author:"^zrake" year:2019-"#
        );
        assert_eq!(
            to_ads_query("^zrake year:2019- OR kw:jets"),
            r#"(author:"^zrake" year:2019-) OR keyword:"jets""#
        );
        assert_eq!(to_ads_query("is:starred ^zrake"), r#"author:"^zrake""#);
        assert_eq!(to_ads_query("is:starred OR is:pdf"), "");
        assert_eq!(to_ads_query(""), "");
        assert_eq!(to_ads_query(r#""gamma ray" OR jets"#), r#""gamma ray" OR jets"#);
    }
}
