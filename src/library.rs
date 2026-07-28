//! Entry and Library — port of the read side of astrobib/library.py.
//!
//! No parse cache: Rust parses the whole library faster than the Python
//! side reads its cache, so the mtime-keyed JSON cache has no analogue here.

use crate::bib::{self, Data};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct SearchDoc {
    pub author: String,
    pub first: String,
    pub title: String,
    pub abs: String,
    pub key: String,
    pub kw: String,
    pub all: String,
}

pub struct Entry {
    pub data: Data,
    pub path: PathBuf,
    pub short_key: String,
    search: OnceCell<SearchDoc>,
}

impl Entry {
    pub fn new(data: Data, path: PathBuf) -> Self {
        Entry {
            data,
            path,
            short_key: String::new(),
            search: OnceCell::new(),
        }
    }

    fn get(&self, k: &str) -> &str {
        self.data.get(k).map(String::as_str).unwrap_or("")
    }

    pub fn key(&self) -> &str {
        self.get("ID")
    }
    pub fn title(&self) -> &str {
        self.get("title")
    }
    pub fn author(&self) -> &str {
        self.get("author")
    }
    pub fn year(&self) -> String {
        self.get("year").to_string()
    }
    pub fn eprint(&self) -> &str {
        self.get("eprint")
    }
    pub fn doi(&self) -> &str {
        self.get("doi")
    }
    pub fn adsurl(&self) -> &str {
        self.get("adsurl")
    }
    pub fn abstract_(&self) -> &str {
        self.get("abstract")
    }

    pub fn keywords(&self) -> Vec<&str> {
        self.get("keywords")
            .split(',')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .collect()
    }

    pub fn starred(&self) -> bool {
        self.get("astrobib_starred").trim().eq_ignore_ascii_case("true")
    }

    pub fn first_author_last(&self) -> &str {
        let author = self.author();
        author
            .split(" and ")
            .next()
            .unwrap_or("")
            .trim()
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
    }

    pub fn bibcode(&self) -> Option<&str> {
        let adsurl = self.adsurl();
        if adsurl.is_empty() {
            return None;
        }
        let t = adsurl.trim_end_matches('/');
        Some(t.rsplit('/').next().unwrap_or(t))
    }

    /// Lowercased field cache for filtering — built once per entry so
    /// per-keystroke matching never re-lowers every abstract.
    pub fn search_doc(&self) -> &SearchDoc {
        self.search.get_or_init(|| {
            let author = self.author().to_lowercase();
            let title = self.title().to_lowercase();
            let abs = self.abstract_().to_lowercase();
            let key = self.key().to_lowercase();
            let kw = self.get("keywords").to_lowercase();
            let all = format!("{author} {title} {abs} {key} {kw} {}", self.year());
            SearchDoc {
                first: self
                    .first_author_last()
                    .to_lowercase()
                    .trim_start_matches('{')
                    .to_string(),
                author,
                title,
                abs,
                key,
                kw,
                all,
            }
        })
    }
}

pub struct Library {
    pub root: PathBuf,
    entries: Vec<Entry>,
    by_key: HashMap<String, usize>,
    by_bibcode: HashMap<String, usize>,
}

impl Library {
    pub fn load(root: &Path) -> std::io::Result<Library> {
        let bib_dir = root.join("bib");
        let mut entries: Vec<Entry> = vec![];
        if bib_dir.is_dir() {
            let mut paths: Vec<PathBuf> = std::fs::read_dir(&bib_dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "bib"))
                .collect();
            paths.sort();
            for path in paths {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Some(data) = bib::parse_entry(&text) {
                    entries.push(Entry::new(data, path));
                }
            }
        }
        let mut lib = Library {
            root: root.to_path_buf(),
            entries,
            by_key: HashMap::new(),
            by_bibcode: HashMap::new(),
        };
        lib.reindex();
        lib.compute_short_keys();
        Ok(lib)
    }

    fn reindex(&mut self) {
        self.by_key.clear();
        self.by_bibcode.clear();
        for (i, e) in self.entries.iter().enumerate() {
            self.by_key.insert(e.key().to_string(), i);
            if let Some(bc) = e.bibcode() {
                self.by_bibcode.insert(bc.to_string(), i);
            }
        }
    }

    /// Shortest unambiguous prefix per key (the AuthorYYYY base when unique,
    /// else base + minimal hash prefix) — port of _compute_short_keys.
    fn compute_short_keys(&mut self) {
        let mut sorted_keys: Vec<String> = self.entries.iter().map(|e| e.key().to_string()).collect();
        sorted_keys.sort();
        let prefix_count = |prefix: &str| {
            let lo = sorted_keys.partition_point(|k| k.as_str() < prefix);
            let hi = sorted_keys.partition_point(|k| k.as_str() < prefix || k.starts_with(prefix));
            hi - lo
        };
        let shorts: Vec<String> = self
            .entries
            .iter()
            .map(|e| {
                let key = e.key();
                let base_len = key.chars().count().saturating_sub(5);
                let base: String = key.chars().take(base_len).collect();
                if prefix_count(&base) == 1 {
                    return base;
                }
                for n in 1..=5usize {
                    let prefix: String = key.chars().take(base_len + n).collect();
                    if prefix_count(&prefix) == 1 {
                        return prefix;
                    }
                }
                key.to_string()
            })
            .collect();
        for (e, s) in self.entries.iter_mut().zip(shorts) {
            e.short_key = s;
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.by_key.get(key).map(|&i| &self.entries[i])
    }

    pub fn get_by_bibcode(&self, bibcode: &str) -> Option<&Entry> {
        self.by_bibcode.get(bibcode).map(|&i| &self.entries[i])
    }

    /// Resolve a full key, unambiguous prefix, or bibcode — port of resolve().
    pub fn resolve(&self, input: &str) -> Option<&Entry> {
        if let Some(e) = self.get(input) {
            return Some(e);
        }
        let matches: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.key().starts_with(input))
            .collect();
        match matches.len() {
            1 => Some(matches[0]),
            0 => self.get_by_bibcode(input),
            _ => None,
        }
    }

    pub fn possible_matches(&self, input: &str) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|e| e.key().starts_with(input))
            .collect()
    }
}

/// Library root: $ASTROBIB_LIBRARY, else $ASTROBIB_STATE_DIR/library, else
/// ~/.local/share/astrobib/library — port of state.py resolution.
pub fn default_library_root() -> PathBuf {
    if let Ok(p) = std::env::var("ASTROBIB_LIBRARY") {
        return PathBuf::from(shellexpand_home(&p));
    }
    let base = std::env::var("ASTROBIB_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".local/share/astrobib"));
    base.join("library")
}

pub fn pdf_cache_dir() -> PathBuf {
    home_dir().join(".cache/astrobib/pdfs")
}

pub fn has_cached_pdf(key: &str) -> bool {
    pdf_cache_dir().join(format!("{key}.pdf")).exists()
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
}

fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        return format!("{}/{}", home_dir().display(), rest);
    }
    p.to_string()
}
