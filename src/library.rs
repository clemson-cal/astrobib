//! Entry and Library — the one-file-per-entry bib database on disk.
//!
//! No parse cache: parsing the whole library is fast enough that
//! caching would cost more in invalidation logic than it saves.

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
    added: OnceCell<i64>,
}

impl Entry {
    pub fn new(data: Data, path: PathBuf) -> Self {
        Entry {
            data,
            path,
            short_key: String::new(),
            search: OnceCell::new(),
            added: OnceCell::new(),
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
    pub fn journal(&self) -> &str {
        self.get("journal")
    }
    pub fn volume(&self) -> &str {
        self.get("volume")
    }
    pub fn pages(&self) -> &str {
        self.get("pages")
    }
    pub fn number(&self) -> &str {
        self.get("number")
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

    /// When the paper joined the library: the .bib file's creation
    /// time (mtime where birthtime is unavailable), unix seconds.
    pub fn added_ts(&self) -> i64 {
        *self.added.get_or_init(|| {
            std::fs::metadata(&self.path)
                .ok()
                .and_then(|m| m.created().or_else(|_| m.modified()).ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        })
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
    tags: crate::tags::Tags,
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
            // eagerly, beside the entries: tags/ is a handful of small
            // files next to a directory we have just read in full, and
            // loading it lazily would buy nothing but a second code path
            tags: crate::tags::Tags::load(root),
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

    /// Shortest unambiguous prefix per key: the AuthorYYYY base when
    /// unique, else base + minimal hash prefix.
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

    pub fn tags(&self) -> &crate::tags::Tags {
        &self.tags
    }

    /// Re-read tags/ alone. A tag file changing says nothing about the
    /// entries, and re-parsing every .bib to notice one edited line is
    /// the kind of cost that grows with the library.
    pub fn reload_tags(&mut self) {
        self.tags = crate::tags::Tags::load(&self.root);
    }

    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.by_key.get(key).map(|&i| &self.entries[i])
    }

    pub fn get_by_bibcode(&self, bibcode: &str) -> Option<&Entry> {
        self.by_bibcode.get(bibcode).map(|&i| &self.entries[i])
    }

    /// Resolve a full key, unambiguous prefix, or bibcode to an entry.
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

    /// Write an entry under its generated key.
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

    /// Delete an entry's file and drop it from the indexes.
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

/// Personal library merged with an optional manuscript database. The
/// personal entry wins when a key exists in both, because the two are
/// copies of one record; tags are the exception and union instead.
pub struct MergedLibrary {
    /// Tier 1: the global personal library.
    pub personal: Library,
    /// Tier 2: the pointed-to local bib library (historically the
    /// manuscript db; now first-class with or without a manuscript).
    pub manuscript: Option<Library>,
    /// Two-tier switch: when false (and a local tier exists), the global
    /// tier is hidden from reads and excluded from normal writes. The
    /// rescue path still writes to it — safety beats visibility.
    pub global_on: bool,
}

impl MergedLibrary {
    pub fn load(ms_root: Option<&Path>) -> std::io::Result<MergedLibrary> {
        Ok(MergedLibrary {
            personal: Library::load(&default_library_root())?,
            manuscript: match ms_root {
                Some(r) => Some(Library::load(r)?),
                None => None,
            },
            global_on: true,
        })
    }

    /// True when the global tier participates in reads/writes: either
    /// it is enabled, or there is no local tier to fall back to.
    fn global_active(&self) -> bool {
        self.global_on || self.manuscript.is_none()
    }

    /// Merged view: every personal entry plus manuscript-only entries —
    /// or the local tier alone when the global tier is toggled off.
    pub fn entries(&self) -> Vec<&Entry> {
        if !self.global_active() {
            return self.manuscript.as_ref().map(|m| m.entries().iter().collect()).unwrap_or_default();
        }
        let mut out: Vec<&Entry> = vec![];
        if let Some(ms) = &self.manuscript {
            out.extend(ms.entries().iter().filter(|e| !self.personal.has(e.key())));
        }
        out.extend(self.personal.entries());
        out
    }

    pub fn get(&self, key: &str) -> Option<&Entry> {
        if !self.global_active() {
            return self.manuscript.as_ref().and_then(|m| m.get(key));
        }
        self.personal
            .get(key)
            .or_else(|| self.manuscript.as_ref().and_then(|m| m.get(key)))
    }

    /// Merged tags: the union of the active tiers', name by name.
    ///
    /// The opposite rule to `get` above, and deliberately so. Two copies
    /// of an entry are one record, so the first tier holding it wins.
    /// Two tag files of the same name are two halves of one collection,
    /// so first-tier-wins would silently drop the other half. The
    /// two-tier switch gates them exactly as it gates entries: tags of a
    /// hidden tier leave the union with it, or a filter would match rows
    /// that are not on screen.
    pub fn tags(&self) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
        let mut out = std::collections::BTreeMap::new();
        if self.global_active() {
            self.personal.tags().union_into(&mut out);
        }
        if let Some(ms) = &self.manuscript {
            ms.tags().union_into(&mut out);
        }
        out
    }

    /// Every tag the active tiers give this key, sorted and deduped —
    /// the union, for the reason `tags` above unions: a paper tagged in
    /// both tiers is genuinely tagged by both.
    pub fn tags_of(&self, key: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        if self.global_active() {
            out.extend(self.personal.tags().of(key).into_iter().map(str::to_string));
        }
        if let Some(ms) = &self.manuscript {
            out.extend(ms.tags().of(key).into_iter().map(str::to_string));
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn reload_tags(&mut self) {
        self.personal.reload_tags();
        if let Some(ms) = &mut self.manuscript {
            ms.reload_tags();
        }
    }

    /// True when any active tier tags this key.
    pub fn has_tag(&self, name: &str, key: &str) -> bool {
        let tier = |l: &Library| l.tags().get(name).is_some_and(|ks| ks.contains(key));
        (self.global_active() && tier(&self.personal))
            || self.manuscript.as_ref().is_some_and(tier)
    }

    /// The tier a new tag is written to: the local one when there is
    /// one, else the global library. Section groupings then live in the
    /// manuscript repo, which is the whole reason for versioning them.
    fn tag_target(&mut self) -> &mut Library {
        match &mut self.manuscript {
            Some(ms) => ms,
            None => &mut self.personal,
        }
    }

    /// Add keys to a tag in the write target. Returns the tier's label
    /// for the report, since which repo just gained a line is exactly
    /// what the user needs to know before committing.
    pub fn tag(&mut self, name: &str, keys: &[String]) -> std::io::Result<&'static str> {
        let local = self.manuscript.is_some();
        let target = self.tag_target();
        let mut set = target.tags().get(name).cloned().unwrap_or_default();
        set.extend(keys.iter().cloned());
        let root = target.root.clone();
        crate::tags::write(&root, name, &set)?;
        target.reload_tags();
        Ok(if local { "local db" } else { "library" })
    }

    /// Remove keys from the tag in *every* active tier that lists them,
    /// and report which. Removing from one tier only would leave the tag
    /// still showing, so the gesture would appear to do nothing.
    pub fn untag(&mut self, name: &str, keys: &[String]) -> std::io::Result<Vec<&'static str>> {
        /// Drop the keys from one tier's copy of the tag; true if that
        /// tier held any of them.
        fn strip(lib: &mut Library, name: &str, keys: &[String]) -> std::io::Result<bool> {
            let Some(set) = lib.tags().get(name) else {
                return Ok(false);
            };
            let mut set = set.clone();
            // filter().count(), not any(): any() stops at the first hit
            // and would leave the rest of a multi-paper untag in place
            if keys.iter().filter(|k| set.remove(*k)).count() == 0 {
                return Ok(false);
            }
            let root = lib.root.clone();
            crate::tags::write(&root, name, &set)?;
            lib.reload_tags();
            Ok(true)
        }
        let mut touched = vec![];
        if self.global_active() && strip(&mut self.personal, name, keys)? {
            touched.push("library");
        }
        if let Some(ms) = &mut self.manuscript {
            if strip(ms, name, keys)? {
                touched.push("local db");
            }
        }
        Ok(touched)
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
        if !self.global_active() {
            return self.manuscript.as_ref().and_then(|m| m.get_by_bibcode(bibcode));
        }
        self.personal
            .get_by_bibcode(bibcode)
            .or_else(|| self.manuscript.as_ref().and_then(|m| m.get_by_bibcode(bibcode)))
    }

    pub fn in_manuscript(&self, key: &str) -> bool {
        self.manuscript.as_ref().is_some_and(|m| m.has(key))
    }

    /// Import: write to both tiers — or only the local tier when the
    /// global tier is toggled off.
    pub fn save_entry(&mut self, data: &crate::bib::Data) -> std::io::Result<String> {
        if !self.global_active() {
            return self.manuscript.as_mut().unwrap().save_entry(data);
        }
        let key = self.personal.save_entry(data)?;
        if let Some(ms) = &mut self.manuscript {
            ms.save_entry(data)?;
        }
        Ok(key)
    }

    pub fn in_personal(&self, key: &str) -> bool {
        self.personal.has(key)
    }

    /// Remove an entry from both databases.
    pub fn remove_entry(&mut self, key: &str) -> std::io::Result<()> {
        self.personal.remove_entry(key)?;
        if let Some(ms) = &mut self.manuscript {
            ms.remove_entry(key)?;
        }
        Ok(())
    }

    /// Copy an entry into the manuscript db. Returns false if there is
    /// no manuscript, the entry is unknown, or it is already a member.
    pub fn add_to_manuscript(&mut self, key: &str) -> std::io::Result<bool> {
        let Some(entry) = self.get(key) else {
            return Ok(false);
        };
        let data = entry.data.clone();
        let Some(ms) = &mut self.manuscript else {
            return Ok(false);
        };
        if ms.has(key) {
            return Ok(false);
        }
        ms.write_entry(key, data)?;
        Ok(true)
    }

    /// Refresh an entry's metadata under the same key in every tier that
    /// holds it. The cite key and filename never change; each copy keeps
    /// its own user-curated keywords. Not gated by global_on: a refresh
    /// rewrites existing copies wherever they live, so the tiers stay in
    /// agreement.
    pub fn update_entry(&mut self, key: &str, data: &Data) -> std::io::Result<bool> {
        let mut any = false;
        if let Some(old) = self.personal.get(key).map(|e| e.data.clone()) {
            let mut d = refreshed_data(data, &old);
            d.insert("ID".to_string(), key.to_string());
            self.personal.write_entry(key, d)?;
            any = true;
        }
        if let Some(ms) = &mut self.manuscript {
            if let Some(old) = ms.get(key).map(|e| e.data.clone()) {
                let mut d = refreshed_data(data, &old);
                d.insert("ID".to_string(), key.to_string());
                ms.write_entry(key, d)?;
                any = true;
            }
        }
        Ok(any)
    }

    /// Remove an entry from the manuscript db, first rescuing it into
    /// the personal library when the manuscript holds the only copy.
    /// Removal never destroys bibdata.
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

/// Field-merge for a metadata refresh: the new ADS record wins every
/// field, except the old copy's user-curated keywords survive when
/// non-empty.
pub fn refreshed_data(new: &Data, old: &Data) -> Data {
    let mut d = new.clone();
    if let Some(kw) = old.get("keywords") {
        if !kw.is_empty() {
            d.insert("keywords".to_string(), kw.clone());
        }
    }
    d
}

/// How a cite string from a manuscript resolves.
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
    /// bibcode.
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

/// Walk up from cwd to find the tier-2 local bib root: any ancestor
/// holding a bib/ directory, excluding the global library root. Under
/// the two-tier model a .git is no longer required — bare bib dirs are
/// first-class. The walk stops at $HOME (exclusive of anything above).
pub fn find_manuscript_db() -> Option<PathBuf> {
    let lib_root = default_library_root();
    let lib_root = lib_root.canonicalize().unwrap_or(lib_root);
    let home = std::env::var("HOME").map(PathBuf::from).ok();
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("bib").is_dir() && dir != lib_root {
            return Some(dir);
        }
        if home.as_deref() == Some(dir.as_path()) || !dir.pop() {
            return None;
        }
    }
}

/// Library root resolution: $ASTROBIB_LIBRARY, else
/// $ASTROBIB_STATE_DIR/library, else ~/.local/share/astrobib/library.
pub fn default_library_root() -> PathBuf {
    if let Ok(p) = std::env::var("ASTROBIB_LIBRARY") {
        return PathBuf::from(shellexpand_home(&p));
    }
    let base = std::env::var("ASTROBIB_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".local/share/astrobib"));
    base.join("library")
}

/// Machine-local cache root. Everything under here is derived data —
/// downloaded PDFs, ADS query results — re-fetchable at any time, so
/// XDG's rule for ~/.cache holds: deleting the directory wholesale is
/// always safe and is how astrobib expects it to be reclaimed. Curated
/// state (metrics.json, tabs.json, state.json) never lives here.
pub fn cache_dir() -> PathBuf {
    home_dir().join(".cache/astrobib")
}

pub fn pdf_cache_dir() -> PathBuf {
    cache_dir().join("pdfs")
}

pub fn has_cached_pdf(key: &str) -> bool {
    pdf_cache_dir().join(format!("{key}.pdf")).exists()
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
}

pub fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        return format!("{}/{}", home_dir().display(), rest);
    }
    p.to_string()
}

/// Replace a file's contents in one indivisible step: write a sibling
/// temp file, flush it to disk, then rename over the target. A rename
/// within a directory is atomic, so a crash leaves either the whole old
/// file or the whole new one, never a half-written one.
///
/// The user-state files (state.json, tabs.json, metrics.json) are
/// written this way because a truncate-then-write is at its most
/// destructive exactly where they are read: a state file that fails to
/// parse is treated as absent and rewritten from scratch, so a ⌃c
/// landing mid-write would not read as damage at the next launch — it
/// would read as no token, or no saved queries. Cache files under
/// ~/.cache are re-fetchable and do not need this.
///
/// The temp name carries the pid, since two astrobib processes can be
/// saving the same file at once and must not share a scratch file. The
/// sync matters as much as the rename: without it the directory entry
/// can reach disk ahead of the bytes it names.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}", std::process::id()));
    let tmp = path.with_file_name(name);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        // an aborted write leaves no litter in the state dir
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(pairs: &[(&str, &str)]) -> Data {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn refresh_merge_keeps_user_keywords() {
        let new = data(&[("title", "New"), ("keywords", "ads-supplied"), ("ID", "K")]);
        let old = data(&[("title", "Old"), ("keywords", "my-topic, other"), ("ID", "K")]);
        let merged = refreshed_data(&new, &old);
        assert_eq!(merged["title"], "New");
        assert_eq!(merged["keywords"], "my-topic, other");
        // empty old keywords: the new record's win
        let bare = data(&[("title", "Old"), ("ID", "K")]);
        assert_eq!(refreshed_data(&new, &bare)["keywords"], "ads-supplied");
        let empty = data(&[("title", "Old"), ("keywords", ""), ("ID", "K")]);
        assert_eq!(refreshed_data(&new, &empty)["keywords"], "ads-supplied");
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("astrobib-update-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bib")).unwrap();
        dir
    }

    fn seed(root: &Path, key: &str, d: &Data) {
        let path = root.join("bib").join(format!("{key}.bib"));
        std::fs::write(path, crate::bib::format_entry(d)).unwrap();
    }

    #[test]
    fn tags_union_across_tiers_and_follow_the_two_tier_switch() {
        let p_root = temp_root("tags-personal");
        let m_root = temp_root("tags-manuscript");
        for root in [&p_root, &m_root] {
            std::fs::create_dir_all(crate::tags::dir(root)).unwrap();
        }
        // the same tag in both tiers, plus one tag private to each
        std::fs::write(crate::tags::dir(&p_root).join("disks"), "A2019aaaaa\nB2020bbbbb\n").unwrap();
        std::fs::write(crate::tags::dir(&p_root).join("read-later"), "A2019aaaaa\n").unwrap();
        std::fs::write(crate::tags::dir(&m_root).join("disks"), "C2021ccccc\n").unwrap();
        std::fs::write(crate::tags::dir(&m_root).join("section-3"), "C2021ccccc\n").unwrap();
        let mut lib = MergedLibrary {
            personal: Library::load(&p_root).unwrap(),
            manuscript: Some(Library::load(&m_root).unwrap()),
            global_on: true,
        };
        let tags = lib.tags();
        assert_eq!(
            tags.keys().map(String::as_str).collect::<Vec<_>>(),
            ["disks", "read-later", "section-3"]
        );
        // union, not shadow: the manuscript's `disks` did not replace
        // the library's, which is where entry resolution would differ
        assert_eq!(tags["disks"].len(), 3);
        assert!(tags["disks"].contains("A2019aaaaa"));
        assert!(tags["disks"].contains("C2021ccccc"));
        // hiding the global tier takes its tags with it
        lib.global_on = false;
        let local = lib.tags();
        assert_eq!(local.keys().map(String::as_str).collect::<Vec<_>>(), ["disks", "section-3"]);
        assert_eq!(local["disks"].len(), 1);
        let _ = std::fs::remove_dir_all(&p_root);
        let _ = std::fs::remove_dir_all(&m_root);
    }

    #[test]
    fn update_entry_rewrites_both_tiers_under_same_key() {
        let key = "Andersson2024abcde";
        let preprint = data(&[
            ("adsurl", "https://ui.adsabs.harvard.edu/abs/2024arXiv240512345Z"),
            ("eprint", "2405.12345"),
            ("year", "2024"),
            ("title", "Old preprint title"),
            ("keywords", "curated-personal"),
            ("ENTRYTYPE", "article"),
            ("ID", key),
        ]);
        let p_root = temp_root("personal");
        let m_root = temp_root("manuscript");
        seed(&p_root, key, &preprint);
        let mut ms_copy = preprint.clone();
        ms_copy.insert("keywords".to_string(), "curated-manuscript".to_string());
        seed(&m_root, key, &ms_copy);
        let mut lib = MergedLibrary {
            personal: Library::load(&p_root).unwrap(),
            manuscript: Some(Library::load(&m_root).unwrap()),
            global_on: true,
        };
        // the refreshed record ADS would return for the published paper
        let published = data(&[
            ("adsurl", "https://ui.adsabs.harvard.edu/abs/2025ApJ...999...1Z"),
            ("eprint", "2405.12345"),
            ("doi", "10.3847/xyz"),
            ("journal", "\\apj"),
            ("year", "2025"),
            ("title", "Published title"),
            ("keywords", "ads-keyword"),
            ("ENTRYTYPE", "article"),
            ("ID", "WrongKey2025zzzzz"),
        ]);
        assert!(lib.update_entry(key, &published).unwrap());
        // key and filename survive in both tiers; metadata is the new record's
        let pe = lib.personal.get(key).unwrap();
        assert_eq!(pe.key(), key);
        assert_eq!(pe.title(), "Published title");
        assert_eq!(pe.data["keywords"], "curated-personal");
        assert!(pe.path.ends_with(format!("bib/{key}.bib")));
        let me = lib.manuscript.as_ref().unwrap().get(key).unwrap();
        assert_eq!(me.key(), key);
        assert_eq!(me.title(), "Published title");
        assert_eq!(me.data["keywords"], "curated-manuscript");
        // bibcode index follows the new adsurl in each tier
        assert!(lib.personal.get_by_bibcode("2025ApJ...999...1Z").is_some());
        assert!(lib.personal.get_by_bibcode("2024arXiv240512345Z").is_none());
        // files on disk really rewrote under the old names
        let on_disk = std::fs::read_to_string(p_root.join("bib").join(format!("{key}.bib"))).unwrap();
        assert!(on_disk.contains("Published title"));
        assert!(!p_root.join("bib").join("WrongKey2025zzzzz.bib").exists());
        // unknown key: no-op
        assert!(!lib.update_entry("Nobody2020aaaaa", &published).unwrap());
        let _ = std::fs::remove_dir_all(&p_root);
        let _ = std::fs::remove_dir_all(&m_root);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("astrobib-atomic-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_no_scratch_behind() {
        let dir = temp_dir("write");
        let path = dir.join("state.json");
        write_atomic(&path, "{\n  \"ads_token\": \"first\"\n}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\n  \"ads_token\": \"first\"\n}\n");
        // rewriting is a replacement, not an append, and the shorter
        // second write must not leave a tail of the first behind
        write_atomic(&path, "{\n  \"x\": 1\n}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\n  \"x\": 1\n}\n");
        // a state dir that accumulates scratch files is a state dir in
        // which nothing says which file is the real one
        assert_eq!(names_in(&dir), ["state.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_that_cannot_finish_keeps_the_old_file_and_cleans_up() {
        let dir = temp_dir("fail");
        let path = dir.join("tabs.json");
        write_atomic(&path, "saved queries\n").unwrap();
        // a directory in the target's place: the rename is what fails,
        // so this exercises the late cleanup path, after the scratch
        // file has already been written and synced
        let blocked = dir.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        assert!(write_atomic(&blocked, "never lands\n").is_err());
        assert_eq!(names_in(&dir), ["blocked", "tabs.json"]);
        // the write that did succeed is untouched by the one that did not
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved queries\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
