//! Manuscript cite-key scanning — TeX (a port of the read side of
//! astrobib/export.py) and Markdown — plus the rendered Markdown
//! bibliography. refs.bib generation comes with the sync flow.

use crate::library::Entry;
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn cite_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\\[Cc]ite[a-zA-Z*]*(?:\[[^\]]*\]){0,2}\{([^}]+)\}").unwrap())
}

fn input_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\\(?:input|include)\s*\{([^}]+)\}").unwrap())
}

/// Strip % comments (a \% is not a comment start).
fn strip_comments(text: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(^|[^\\])%.*").unwrap());
    re.replace_all(text, "$1").into_owned()
}

/// Roots plus every file reachable through \input/\include, resolved
/// against the manuscript root with .tex appended when suffixless.
pub fn expand_tex_sources(roots: Vec<PathBuf>, base: &Path) -> Vec<PathBuf> {
    let mut ordered = vec![];
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<PathBuf> = roots;
    while !stack.is_empty() {
        let path = stack.remove(0);
        let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
        if visited.contains(&resolved) || !path.is_file() {
            continue;
        }
        visited.insert(resolved);
        ordered.push(path.clone());
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let text = strip_comments(&text);
        for m in input_re().captures_iter(&text) {
            let name = m[1].trim();
            let mut child = base.join(name);
            if child.extension().is_none() {
                child.set_extension("tex");
            }
            stack.push(child);
        }
    }
    ordered
}

/// TeX sources for a manuscript: main.tex is the sole root when present,
/// else every top-level .tex file; expanded through \input/\include.
pub fn manuscript_tex_files(ms_root: &Path) -> Vec<PathBuf> {
    let main = ms_root.join("main.tex");
    let roots = if main.is_file() {
        vec![main]
    } else {
        let mut v: Vec<PathBuf> = std::fs::read_dir(ms_root)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "tex"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    expand_tex_sources(roots, ms_root)
}

/// Every cited key across the given files, in first-seen order.
pub fn scan_tex_files(paths: &[PathBuf]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut ordered: Vec<String> = vec![];
    for p in paths {
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        let text = strip_comments(&text);
        for m in cite_re().captures_iter(&text) {
            for key in m[1].split(',') {
                let k = key.trim();
                if !k.is_empty() && seen.insert(k.to_string()) {
                    ordered.push(k.to_string());
                }
            }
        }
    }
    ordered
}

// ── Markdown manuscripts ────────────────────────────────────────────
//
// Citations use pandoc syntax — bare `@Key2020abcde` or bracketed
// `[@Key2020; @Other2019]` — and Obsidian wikilinks `[[Key2020]]`
// (alias/heading suffixes tolerated). A wikilink is a citation only
// when it resolves in the library; unresolved wikilinks are ordinary
// note links, so the caller filters MdCite { wikilink: true } through
// resolution while pandoc cites always count (and surface as missing).

/// One citation-shaped token from a markdown source.
pub struct MdCite {
    pub raw: String,
    pub wikilink: bool,
}

fn md_cite_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // an @key preceded by start-of-line or a non-word opener, so
    // emails (user@host) never match; keys admit bibcode characters
    RE.get_or_init(|| {
        Regex::new(r"(?m)(?:^|[\s\[(;:*_])@([A-Za-z0-9][A-Za-z0-9&.+_:/-]*)").unwrap()
    })
}

fn wikilink_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // [[target]], [[target|alias]], [[target#heading]]; a leading !
    // marks an embed (transclusion), which is a source file, not a cite
    RE.get_or_init(|| Regex::new(r"(!?)\[\[([^\[\]|#]+)(?:[|#][^\[\]]*)?\]\]").unwrap())
}

/// Remove fenced code blocks, inline code, and HTML comments so their
/// contents never scan as citations — the markdown analogue of TeX
/// comment stripping.
fn md_strip(text: &str) -> String {
    static INLINE: OnceLock<Regex> = OnceLock::new();
    static COMMENT: OnceLock<Regex> = OnceLock::new();
    let inline = INLINE.get_or_init(|| Regex::new(r"`[^`\n]*`").unwrap());
    let comment = COMMENT.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
    // fenced blocks line-by-line (the regex crate has no backreferences)
    let mut kept = String::with_capacity(text.len());
    let mut fence: Option<&str> = None;
    for line in text.lines() {
        let t = line.trim_start();
        match fence {
            Some(f) => {
                if t.starts_with(f) {
                    fence = None;
                }
            }
            None => {
                if t.starts_with("```") {
                    fence = Some("```");
                } else if t.starts_with("~~~") {
                    fence = Some("~~~");
                } else {
                    kept.push_str(line);
                    kept.push('\n');
                }
            }
        }
    }
    let t = comment.replace_all(&kept, "");
    inline.replace_all(&t, "").into_owned()
}

/// Markdown sources for a manuscript: main.md is the sole root when
/// present, else every top-level .md file; expanded through Obsidian
/// `![[embed]]` transclusions (resolved against the root, .md appended
/// when suffixless) — the \input/\include analogue.
pub fn manuscript_md_files(ms_root: &Path) -> Vec<PathBuf> {
    let main = ms_root.join("main.md");
    let roots = if main.is_file() {
        vec![main]
    } else {
        let mut v: Vec<PathBuf> = std::fs::read_dir(ms_root)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "md"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    let mut ordered = vec![];
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack = roots;
    while !stack.is_empty() {
        let path = stack.remove(0);
        let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
        if visited.contains(&resolved) || !path.is_file() {
            continue;
        }
        visited.insert(resolved);
        ordered.push(path.clone());
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for m in wikilink_re().captures_iter(&md_strip(&text)) {
            if &m[1] == "!" {
                let mut child = ms_root.join(m[2].trim());
                if child.extension().is_none() {
                    child.set_extension("md");
                }
                stack.push(child);
            }
        }
    }
    ordered
}

/// Every citation-shaped token across the given markdown files, in
/// first-seen order (pandoc cites and wikilinks interleaved as read).
pub fn scan_md_files(paths: &[PathBuf]) -> Vec<MdCite> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut ordered: Vec<MdCite> = vec![];
    for p in paths {
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        let text = md_strip(&text);
        // one pass in document order so the two syntaxes interleave
        let mut hits: Vec<(usize, String, bool)> = vec![];
        for m in md_cite_re().captures_iter(&text) {
            let g = m.get(1).unwrap();
            let key = g.as_str().trim_end_matches(['.', ',', ';', ':']);
            if !key.is_empty() {
                hits.push((g.start(), key.to_string(), false));
            }
        }
        for m in wikilink_re().captures_iter(&text) {
            if &m[1] != "!" {
                let g = m.get(2).unwrap();
                let key = g.as_str().trim();
                if !key.is_empty() {
                    hits.push((g.start(), key.to_string(), true));
                }
            }
        }
        hits.sort_by_key(|&(pos, _, _)| pos);
        for (_, raw, wikilink) in hits {
            if seen.insert(raw.clone()) {
                ordered.push(MdCite { raw, wikilink });
            }
        }
    }
    ordered
}

// ── Rendered Markdown bibliography ──────────────────────────────────

pub const MD_BIB_BEGIN: &str = "<!-- astrobib:references -->";
pub const MD_BIB_END: &str = "<!-- /astrobib:references -->";

/// AAS journal macros as they appear in ADS BibTeX journal fields.
fn journal_name(raw: &str) -> String {
    let name = match raw.trim().trim_start_matches('\\') {
        "apj" => "ApJ",
        "apjl" => "ApJL",
        "apjs" => "ApJS",
        "aj" => "AJ",
        "aap" => "A&A",
        "aapr" => "A&A Rev.",
        "araa" => "ARA&A",
        "mnras" => "MNRAS",
        "pasp" => "PASP",
        "pasj" => "PASJ",
        "pasa" => "PASA",
        "prd" => "Phys. Rev. D",
        "prl" => "Phys. Rev. Lett.",
        "physrep" => "Phys. Rep.",
        "jcap" => "JCAP",
        "apss" => "Ap&SS",
        "ssr" => "Space Sci. Rev.",
        "na" => "New Astron.",
        "nar" => "New Astron. Rev.",
        "nat" => "Nature",
        "icarus" => "Icarus",
        other => other,
    };
    name.trim_matches(['{', '}']).to_string()
}

/// "Surname, I. J." from one "Surname, Given Names" BibTeX author.
fn author_initials(one: &str) -> String {
    let mut parts = one.trim().splitn(2, ',');
    let surname = parts.next().unwrap_or("").trim().trim_matches(['{', '}']);
    let given = parts.next().unwrap_or("").trim();
    if given.is_empty() {
        return surname.to_string();
    }
    let initials: Vec<String> = given
        .split_whitespace()
        .filter_map(|w| {
            let w = w.trim_matches(['{', '}', '~']);
            w.chars().next().map(|c| format!("{c}."))
        })
        .collect();
    format!("{surname}, {}", initials.join(" "))
}

/// "Zrake, J., & MacFadyen, A. I." — full list up to five names, then
/// first author et al.
fn bib_authors(author: &str) -> String {
    let names: Vec<String> = author
        .split(" and ")
        .map(author_initials)
        .filter(|s| !s.is_empty())
        .collect();
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{}, & {}", names[0], names[1]),
        3..=5 => format!(
            "{}, & {}",
            names[..names.len() - 1].join(", "),
            names[names.len() - 1]
        ),
        _ => format!("{}, et al.", names[0]),
    }
}

/// One markdown bibliography item for an entry.
pub fn md_bib_item(e: &Entry) -> String {
    let mut s = format!("- {} ({}).", bib_authors(e.author()), e.year());
    let title = e.title().trim_matches(['{', '}']).trim_end_matches('.');
    if !title.is_empty() {
        s.push_str(&format!(" *{title}*."));
    }
    let journal = journal_name(e.journal());
    if !journal.is_empty() {
        s.push_str(&format!(" {journal}"));
        if !e.volume().is_empty() {
            s.push_str(&format!(" **{}**", e.volume()));
        }
        if !e.pages().is_empty() {
            s.push_str(&format!(", {}", e.pages()));
        }
        s.push('.');
    }
    let mut links: Vec<String> = vec![];
    if !e.adsurl().is_empty() {
        links.push(format!("[ADS]({})", e.adsurl()));
    }
    if !e.eprint().is_empty() {
        links.push(format!("[arXiv](https://arxiv.org/abs/{})", e.eprint()));
    }
    if !e.doi().is_empty() {
        links.push(format!("[DOI](https://doi.org/{})", e.doi()));
    }
    if !links.is_empty() {
        s.push_str(&format!(" {}", links.join(" · ")));
    }
    s
}

/// The full marker-delimited bibliography block, entries sorted by
/// first-author surname then year.
pub fn render_md_bibliography(entries: &[&Entry]) -> String {
    let mut sorted: Vec<&&Entry> = entries.iter().collect();
    sorted.sort_by_key(|e| {
        (
            e.first_author_last().to_lowercase(),
            e.year(),
            e.key().to_string(),
        )
    });
    let mut out = String::from(MD_BIB_BEGIN);
    out.push('\n');
    for e in sorted {
        out.push_str(&md_bib_item(e));
        out.push('\n');
    }
    out.push_str(MD_BIB_END);
    out
}

/// Insert or replace the marker-delimited bibliography in a markdown
/// file. Without markers a `## References` section is appended.
/// Returns whether the file changed.
pub fn update_md_bibliography(path: &Path, block: &str) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(path)?;
    let updated = match (text.find(MD_BIB_BEGIN), text.find(MD_BIB_END)) {
        (Some(a), Some(b)) if b >= a => {
            format!("{}{}{}", &text[..a], block, &text[b + MD_BIB_END.len()..])
        }
        _ => {
            let sep = if text.ends_with("\n\n") {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{text}{sep}## References\n\n{block}\n")
        }
    };
    if updated == text {
        return Ok(false);
    }
    std::fs::write(path, updated)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn scans_cite_variants() {
        let text = r"\citep[e.g.][]{Zrake2019, Metzger2017*} and \Citet{Kasen2017}
% \cite{Commented2000}
\citeauthor{Zrake2019}";
        let re = super::cite_re();
        let stripped = super::strip_comments(text);
        let mut keys = vec![];
        for m in re.captures_iter(&stripped) {
            for k in m[1].split(',') {
                keys.push(k.trim().to_string());
            }
        }
        assert!(keys.contains(&"Zrake2019".to_string()));
        assert!(keys.contains(&"Kasen2017".to_string()));
        assert!(!keys.iter().any(|k| k.contains("Commented")));
    }

    fn md_keys(text: &str) -> Vec<(String, bool)> {
        let text = super::md_strip(text);
        let mut hits: Vec<(usize, String, bool)> = vec![];
        for m in super::md_cite_re().captures_iter(&text) {
            let g = m.get(1).unwrap();
            let k = g.as_str().trim_end_matches(['.', ',', ';', ':']);
            hits.push((g.start(), k.to_string(), false));
        }
        for m in super::wikilink_re().captures_iter(&text) {
            if &m[1] != "!" {
                let g = m.get(2).unwrap();
                hits.push((g.start(), g.as_str().trim().to_string(), true));
            }
        }
        hits.sort_by_key(|&(p, _, _)| p);
        hits.into_iter().map(|(_, k, w)| (k, w)).collect()
    }

    #[test]
    fn scans_md_pandoc_cites() {
        let got = md_keys("As shown by @Zrake2019 and [@Metzger2017abcde; @Kasen2017].");
        assert_eq!(
            got,
            vec![
                ("Zrake2019".to_string(), false),
                ("Metzger2017abcde".to_string(), false),
                ("Kasen2017".to_string(), false),
            ]
        );
    }

    #[test]
    fn md_cites_skip_emails_and_code() {
        let got = md_keys("mail me at jane@example.com; `@NotACite` and\n```\n@AlsoNot2020\n```\n<!-- @Hidden2020 -->\nbut @Real2020 counts.");
        assert_eq!(got, vec![("Real2020".to_string(), false)]);
    }

    #[test]
    fn scans_md_wikilinks() {
        let got = md_keys("See [[Zrake2019]] and [[Metzger2017|the kilonova review]], also [[notes#section]]; ![[embedded-file]] is a source.");
        assert_eq!(
            got,
            vec![
                ("Zrake2019".to_string(), true),
                ("Metzger2017".to_string(), true),
                ("notes".to_string(), true),
            ]
        );
    }

    #[test]
    fn bib_author_formatting() {
        assert_eq!(
            super::bib_authors("{Zrake}, Jonathan and {MacFadyen}, Andrew I."),
            "Zrake, J., & MacFadyen, A. I."
        );
        let many = (0..8)
            .map(|i| format!("{{A{i}}}, X."))
            .collect::<Vec<_>>()
            .join(" and ");
        assert_eq!(super::bib_authors(&many), "A0, X., et al.");
    }

    #[test]
    fn md_bib_update_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("astrobib-mdbib-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("main.md");
        std::fs::write(&p, "# Review\n\nSee @Zrake2019.\n").unwrap();
        let block = format!("{}\n- item\n{}", super::MD_BIB_BEGIN, super::MD_BIB_END);
        assert!(super::update_md_bibliography(&p, &block).unwrap());
        let once = std::fs::read_to_string(&p).unwrap();
        assert!(once.contains("## References"));
        assert!(!super::update_md_bibliography(&p, &block).unwrap());
        assert_eq!(once, std::fs::read_to_string(&p).unwrap());
        let block2 = format!("{}\n- other\n{}", super::MD_BIB_BEGIN, super::MD_BIB_END);
        assert!(super::update_md_bibliography(&p, &block2).unwrap());
        let twice = std::fs::read_to_string(&p).unwrap();
        assert!(twice.contains("- other") && !twice.contains("- item"));
        assert_eq!(twice.matches("## References").count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
