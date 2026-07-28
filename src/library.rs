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

    pub fn has(&self, key: &str) -> bool {
        self.by_key.contains_key(key)
    }

    /// Write an entry under its generated key — port of save_entry.
    /// Returns the key; overwrites any existing entry with the same key.
    pub fn save_entry(&mut self, data: &Data) -> std::io::Result<String> {
        let mut data = data.clone();
        let key = crate::keys::generate_key(&data);
        data.insert("ID".to_string(), key.clone());
        let bib_dir = self.root.join("bib");
        std::fs::create_dir_all(&bib_dir)?;
        let path = bib_dir.join(format!("{key}.bib"));
        std::fs::write(&path, bib::format_entry(&data))?;
        let entry = Entry::new(data, path);
        if let Some(&i) = self.by_key.get(&key) {
            self.entries[i] = entry;
        } else {
            self.entries.push(entry);
        }
        self.reindex();
        self.compute_short_keys();
        Ok(key)
    }

    /// Delete an entry's file and drop it from the indexes — port of
    /// remove_entry.
    pub fn remove_entry(&mut self, key: &str) -> std::io::Result<()> {
        let path = self.root.join("bib").join(format!("{key}.bib"));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        if let Some(&i) = self.by_key.get(key) {
            self.entries.remove(i);
            self.reindex();
            self.compute_short_keys();
        }
        Ok(())
    }

    /// Write a prepared data map under a FIXED key (no key generation) —
    /// used by manuscript membership copies.
    fn write_entry(&mut self, key: &str, data: Data) -> std::io::Result<()> {
        let bib_dir = self.root.join("bib");
        std::fs::create_dir_all(&bib_dir)?;
        let path = bib_dir.join(format!("{key}.bib"));
        std::fs::write(&path, bib::format_entry(&data))?;
        let entry = Entry::new(data, path);
        if let Some(&i) = self.by_key.get(key) {
            self.entries[i] = entry;
        } else {
            self.entries.push(entry);
        }
        self.reindex();
        self.compute_short_keys();
        Ok(())
    }

}

/// Personal library merged with an optional manuscript database — port of
/// the read side of MergedLibrary. The personal entry wins when a key
/// exists in both; stars are personal and never written to the manuscript.
pub struct MergedLibrary {
    pub personal: Library,
    pub manuscript: Option<Library>,
}

impl MergedLibrary {
    pub fn load(ms_root: Option<&Path>) -> std::io::Result<MergedLibrary> {
        Ok(MergedLibrary {
            personal: Library::load(&default_library_root())?,
            manuscript: match ms_root {
                Some(r) => Some(Library::load(r)?),
                None => None,
            },
        })
    }

    /// Merged view: every personal entry, plus manuscript-only entries.
    pub fn entries(&self) -> Vec<&Entry> {
        let mut out: Vec<&Entry> = vec![];
        if let Some(ms) = &self.manuscript {
            out.extend(ms.entries().iter().filter(|e| !self.personal.has(e.key())));
        }
        out.extend(self.personal.entries());
        out
    }

    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.personal
            .get(key)
            .or_else(|| self.manuscript.as_ref().and_then(|m| m.get(key)))
    }

    pub fn resolve(&self, input: &str) -> Option<&Entry> {
        if let Some(e) = self.get(input) {
            return Some(e);
        }
        let matches = self.possible_matches(input);
        match matches.len() {
            1 => Some(matches[0]),
            0 => self.get_by_bibcode(input),
            _ => None,
        }
    }

    pub fn possible_matches(&self, input: &str) -> Vec<&Entry> {
        self.entries()
            .into_iter()
            .filter(|e| e.key().starts_with(input))
            .collect()
    }

    pub fn get_by_bibcode(&self, bibcode: &str) -> Option<&Entry> {
        self.personal
            .get_by_bibcode(bibcode)
            .or_else(|| self.manuscript.as_ref().and_then(|m| m.get_by_bibcode(bibcode)))
    }

    pub fn in_manuscript(&self, key: &str) -> bool {
        self.manuscript.as_ref().is_some_and(|m| m.has(key))
    }

    /// Import: write to the personal library and the manuscript db (if any).
    pub fn save_entry(&mut self, data: &crate::bib::Data) -> std::io::Result<String> {
        let key = self.personal.save_entry(data)?;
        if let Some(ms) = &mut self.manuscript {
            ms.save_entry(data)?;
        }
        Ok(key)
    }

    pub fn in_personal(&self, key: &str) -> bool {
        self.personal.has(key)
    }

    /// Remove an entry from both databases — port of remove_entry.
    pub fn remove_entry(&mut self, key: &str) -> std::io::Result<()> {
        self.personal.remove_entry(key)?;
        if let Some(ms) = &mut self.manuscript {
            ms.remove_entry(key)?;
        }
        Ok(())
    }

    /// Copy an entry into the manuscript db (personal fields stripped) —
    /// port of add_to_manuscript. Returns false if there is no manuscript,
    /// the entry is unknown, or it is already a member.
    pub fn add_to_manuscript(&mut self, key: &str) -> std::io::Result<bool> {
        let Some(entry) = self.get(key) else {
            return Ok(false);
        };
        let mut data = entry.data.clone();
        // legacy personal field from the Python tool never enters a
        // shared manuscript db
        data.shift_remove("astrobib_starred");
        let Some(ms) = &mut self.manuscript else {
            return Ok(false);
        };
        if ms.has(key) {
            return Ok(false);
        }
        ms.write_entry(key, data)?;
        Ok(true)
    }

    /// Remove an entry from the manuscript db, first rescuing it into the
    /// personal library when the manuscript holds the only copy — port of
    /// remove_from_manuscript. Removal never destroys bibdata.
    pub fn remove_from_manuscript(&mut self, key: &str) -> std::io::Result<bool> {
        let Some(ms) = &self.manuscript else {
            return Ok(false);
        };
        if !ms.has(key) {
            return Ok(false);
        }
        if !self.personal.has(key) {
            let data = ms.get(key).unwrap().data.clone();
            self.personal.write_entry(key, data)?;
        }
        self.manuscript.as_mut().unwrap().remove_entry(key)?;
        Ok(true)
    }
}

/// How a cite string from a manuscript resolves — port of
/// resolve_citation's states.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CiteState {
    /// resolves to a manuscript-db member
    Ok,
    /// resolves, but only in the personal library
    Library,
    /// prefix of several keys
    Ambiguous,
    /// no match anywhere
    Missing,
}

impl MergedLibrary {
    /// Classify a cite string: full key, unambiguous prefix, or raw
    /// bibcode — port of resolve_citation.
    pub fn resolve_citation(&self, cited: &str) -> (CiteState, Option<&Entry>) {
        let entry = match self.get(cited) {
            Some(e) => Some(e),
            None => {
                let matches = self.possible_matches(cited);
                match matches.len() {
                    1 => Some(matches[0]),
                    0 => self.get_by_bibcode(cited),
                    _ => return (CiteState::Ambiguous, None),
                }
            }
        };
        match entry {
            None => (CiteState::Missing, None),
            Some(e) => {
                if self.in_manuscript(e.key()) {
                    (CiteState::Ok, Some(e))
                } else {
                    (CiteState::Library, Some(e))
                }
            }
        }
    }
}

/// Walk up from cwd to find a manuscript database: a directory holding
/// both bib/ and .git, excluding the active library root — port of
/// state.find_manuscript_db.
pub fn find_manuscript_db() -> Option<PathBuf> {
    let lib_root = default_library_root();
    let lib_root = lib_root.canonicalize().unwrap_or(lib_root);
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("bib").is_dir() && dir.join(".git").exists() && dir != lib_root {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
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
