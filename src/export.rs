//! TeX cite-key scanning — port of the read side of astrobib/export.py
//! (refs.bib generation comes with the sync flow).

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
}
