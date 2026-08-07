//! tags/ — the versioned collections beside bib/.
//!
//! One file per tag, named for the tag; one cite key per line. The
//! format cannot fail to parse: every line is blank, a `#` comment, or
//! a key, and a key that names nothing in the library is skipped rather
//! than rejected (docs/DESIGN.md). So the only errors here are I/O ones
//! — a file that will not read, or bytes that are not UTF-8 — and they
//! are collected per file rather than failing the load, since one bad
//! file must not cost the user the other tags.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The tags/ directory of one database tier.
#[derive(Default)]
pub struct Tags {
    /// tag name → its keys. Sorted and deduped by construction, which
    /// is also the on-disk order the format asks for.
    tags: BTreeMap<String, BTreeSet<String>>,
    /// (file name, why) for files that could not be read at all.
    errors: Vec<(String, String)>,
}

pub fn dir(root: &Path) -> PathBuf {
    root.join("tags")
}

/// The tag files of a database, sorted.
///
/// Dotfiles are skipped: macOS drops `.DS_Store` into any directory a
/// Finder window has visited, and a tag named `.DS_Store` full of
/// binary is a worse outcome than an editor swap file going unread.
/// Subdirectories are skipped for the same reason `bib/` is flat —
/// there is no nesting in the format to give them a meaning.
fn files(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir(root)) else {
        return vec![];
    };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    paths.sort();
    paths
}

/// Every tag file paired with its own mtime, for change detection.
///
/// Per file, not the directory: a tag is edited by appending a line to
/// a file that already exists, which never moves the directory's mtime.
/// Re-enumerating on each call is what catches a tag file added or
/// deleted, so no directory mtime is needed here either.
pub fn watch(root: &Path) -> Vec<(PathBuf, SystemTime)> {
    files(root)
        .into_iter()
        .filter_map(|p| {
            let m = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((p, m))
        })
        .collect()
}

/// One tag file's keys. Blank lines and whole-line `#` comments are
/// ignored; nothing else is interpreted. Trailing comments are not a
/// thing on purpose — the file's whole value is that it *is* the citekey
/// dump, and anything after a key on its line would have to be stripped
/// before the file could be handed to someone.
fn parse(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

impl Tags {
    /// Read every tag file under `root/tags`. A missing directory is the
    /// common case, not an error: most databases have no tags.
    pub fn load(root: &Path) -> Tags {
        let mut out = Tags::default();
        for path in files(root) {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    out.tags.insert(name, parse(&text));
                }
                Err(e) => out.errors.push((name, e.to_string())),
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&BTreeSet<String>> {
        self.tags.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tags.keys().map(String::as_str)
    }

    /// Every tag this key belongs to, sorted.
    pub fn of(&self, key: &str) -> Vec<&str> {
        self.tags
            .iter()
            .filter(|(_, keys)| keys.contains(key))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Files that would not read, as (name, why).
    pub fn errors(&self) -> &[(String, String)] {
        &self.errors
    }

    /// Fold these tags into a cross-tier view. Union, never shadow: a
    /// paper tagged `disk-instability` in the library and `section-3` in
    /// a manuscript is genuinely both, and picking a winning tier would
    /// silently discard one of them. Entries resolve the other way —
    /// see MergedLibrary::get — and copying that pattern here is the
    /// mistake to avoid.
    pub fn union_into(&self, out: &mut BTreeMap<String, BTreeSet<String>>) {
        for (name, keys) in &self.tags {
            out.entry(name.clone()).or_default().extend(keys.iter().cloned());
        }
    }
}

/// A tag name that cannot become a filename, or would escape tags/.
/// Empty, `.`-leading (a dotfile is skipped on read, so writing one
/// would create a tag that vanishes), or path-bearing.
pub fn bad_name(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("a tag needs a name")
    } else if name.starts_with('.') {
        Some("a tag name cannot start with a dot — dotfiles are not read back")
    } else if name.contains('/') || name.contains('\\') || name == ".." {
        Some("a tag name is a filename, not a path")
    } else {
        None
    }
}

/// The comment lines of an existing tag file, in the order they appear.
fn comments(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return vec![];
    };
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Rewrite one tag file in canonical form: its comments first, in the
/// order they were written, then one key per line, sorted and deduped.
///
/// Comments migrate to the top rather than staying beside the keys they
/// were written near — sorting moves the keys out from under them
/// anyway, and dropping the text entirely is the one outcome worth
/// ruling out. Keys are never dropped for failing to resolve: whether a
/// key names a paper is the library's question, and a line naming a
/// paper not yet imported is the format working as intended.
///
/// A tag left with no keys and no comments is deleted rather than left
/// as an empty file, since there is then nothing in it to lose.
pub fn write(root: &Path, name: &str, keys: &BTreeSet<String>) -> std::io::Result<()> {
    let path = dir(root).join(name);
    let head = comments(&path);
    if keys.is_empty() && head.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        };
    }
    std::fs::create_dir_all(dir(root))?;
    let mut out = String::new();
    for line in head {
        out.push_str(&line);
        out.push('\n');
    }
    for key in keys {
        out.push_str(key);
        out.push('\n');
    }
    std::fs::write(&path, out)
}

/// Rewrite every tag file in canonical form — `astrobib tidy`'s share
/// of the format. Returns the tags whose file actually changed.
pub fn tidy(root: &Path) -> Vec<String> {
    let mut changed = vec![];
    for path in files(root) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let keys = parse(&text);
        let before = text;
        if write(root, &name, &keys).is_ok()
            && std::fs::read_to_string(&path).map(|t| t != before).unwrap_or(false)
        {
            changed.push(name);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ignores_blanks_and_comments_and_sorts() {
        let keys = parse(
            "# the spiral-shock references\n\
             Zrake2019abcde\n\
             \n\
               Andersson2024fghij  \n\
             # trailing note\n\
             Zrake2019abcde\n",
        );
        let v: Vec<&str> = keys.iter().map(String::as_str).collect();
        // sorted, and the repeat collapsed
        assert_eq!(v, ["Andersson2024fghij", "Zrake2019abcde"]);
    }

    #[test]
    fn a_key_that_resolves_to_nothing_is_still_a_key() {
        // parsing never rejects: whether a key names a paper is the
        // library's question, asked later and answered by skipping
        let keys = parse("not a cite key at all\n");
        assert_eq!(keys.len(), 1);
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("astrobib-tags-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tags")).unwrap();
        dir
    }

    #[test]
    fn load_reads_every_file_skipping_dotfiles() {
        let root = temp_root("load");
        std::fs::write(dir(&root).join("section-3"), "Zrake2019abcde\n").unwrap();
        std::fs::write(dir(&root).join("disks"), "# empty for now\n").unwrap();
        std::fs::write(dir(&root).join(".DS_Store"), [0u8, 159, 146, 150]).unwrap();
        let tags = Tags::load(&root);
        let mut view = BTreeMap::new();
        tags.union_into(&mut view);
        assert_eq!(view.keys().map(String::as_str).collect::<Vec<_>>(), ["disks", "section-3"]);
        assert!(view["disks"].is_empty());
        assert!(view["section-3"].contains("Zrake2019abcde"));
        // the dotfile is neither a tag nor an error
        assert!(tags.errors().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_file_is_an_error_and_costs_no_other_tag() {
        let root = temp_root("errors");
        std::fs::write(dir(&root).join("good"), "Zrake2019abcde\n").unwrap();
        // invalid UTF-8: read_to_string fails where the others succeed
        std::fs::write(dir(&root).join("binary"), [0xff, 0xfe, 0x00]).unwrap();
        let tags = Tags::load(&root);
        assert_eq!(tags.errors().len(), 1);
        assert_eq!(tags.errors()[0].0, "binary");
        let mut view = BTreeMap::new();
        tags.union_into(&mut view);
        assert_eq!(view.len(), 1);
        assert!(view.contains_key("good"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_tags_directory_is_not_an_error() {
        let root = std::env::temp_dir().join(format!("astrobib-tags-test-{}-none", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let tags = Tags::load(&root);
        assert!(tags.is_empty());
        assert!(tags.errors().is_empty());
        assert!(watch(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_sorts_and_dedupes_and_keeps_the_comments() {
        let root = temp_root("write");
        let path = dir(&root).join("section-3");
        std::fs::write(&path, "# spiral shocks\nZrake2019abcde\n# and disks\n").unwrap();
        let keys: BTreeSet<String> = ["Zrake2019abcde", "Andersson2024fghij", "Zrake2019abcde"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        write(&root, "section-3", &keys).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# spiral shocks\n# and disks\nAndersson2024fghij\nZrake2019abcde\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_emptied_tag_goes_away_unless_it_still_says_something() {
        let root = temp_root("empty");
        let bare = dir(&root).join("bare");
        let noted = dir(&root).join("noted");
        std::fs::write(&bare, "Zrake2019abcde\n").unwrap();
        std::fs::write(&noted, "# section 3 references\nZrake2019abcde\n").unwrap();
        write(&root, "bare", &BTreeSet::new()).unwrap();
        write(&root, "noted", &BTreeSet::new()).unwrap();
        assert!(!bare.exists(), "an empty tag with nothing in it should go");
        assert_eq!(std::fs::read_to_string(&noted).unwrap(), "# section 3 references\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tidy_canonicalizes_and_reports_only_what_moved() {
        let root = temp_root("tidy");
        std::fs::write(dir(&root).join("messy"), "  Zrake2019abcde\nAndersson2024fghij\nZrake2019abcde\n").unwrap();
        std::fs::write(dir(&root).join("clean"), "Andersson2024fghij\nZrake2019abcde\n").unwrap();
        assert_eq!(tidy(&root), ["messy"]);
        assert_eq!(
            std::fs::read_to_string(dir(&root).join("messy")).unwrap(),
            "Andersson2024fghij\nZrake2019abcde\n"
        );
        // a second pass is a no-op: the canonical form is a fixed point
        assert!(tidy(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn names_that_cannot_round_trip_are_refused() {
        assert!(bad_name("section-3").is_none());
        assert!(bad_name("").is_some());
        assert!(bad_name(".hidden").is_some());
        assert!(bad_name("a/b").is_some());
        assert!(bad_name("..").is_some());
    }

    /// The property the whole watching scheme rests on: an edit inside
    /// an existing file has to be visible, because that is how a tag is
    /// actually changed. A directory mtime would not move here.
    #[test]
    fn watch_moves_when_a_file_is_edited_in_place() {
        let root = temp_root("watch");
        let path = dir(&root).join("section-3");
        std::fs::write(&path, "Zrake2019abcde\n").unwrap();
        let before = watch(&root);
        assert_eq!(before.len(), 1);
        // filesystem mtime resolution is coarse enough that an immediate
        // rewrite can land in the same tick, so rewrite until it moves
        // rather than assuming one write is enough
        for _ in 0..50 {
            std::fs::write(&path, "Zrake2019abcde\nAndersson2024fghij\n").unwrap();
            if watch(&root) != before {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_ne!(watch(&root), before);
        // and a tag file appearing shows up as a new element
        std::fs::write(dir(&root).join("disks"), "").unwrap();
        assert_eq!(watch(&root).len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
