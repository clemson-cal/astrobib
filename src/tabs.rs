//! Persistent ADS query tabs, stored in the app's own tabs.json state
//! file: user-local, never written into a manuscript repo.
//!
//! A query has one of two homes. The global set is visible from every
//! directory; a manuscript's set appears when that manuscript is the
//! active one. A session reads both and can move a query between them.
//! On disk they are two ordinary keys of the same `contexts` map — the
//! home is the key holding the tab, never a field on the tab.

use std::path::{Path, PathBuf};

/// Which of the two sets a saved query lives in. Deliberately not a
/// stored field: the context key already says it, and a tab carrying
/// its own copy could disagree with the key holding it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Home {
    Global,
    Local,
}

/// The context key of the set that is not any manuscript's.
const GLOBAL: &str = "global";

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub id: String,
    pub query: String,
    pub label: String,
    pub limit: usize,
    pub created: i64,
    pub refreshed: Option<i64>,
    /// How this tab's results are ordered on screen — one sort per tab,
    /// so switching between queries does not disturb their orders. This
    /// is a display sort, applied locally to results already in hand; it
    /// is not the ADS `sort` parameter, which decides *which* records
    /// come back and is a property of the query itself.
    pub sort_col: String,
    pub sort_asc: bool,
    /// The ADS `sort` parameter this query runs with — what decides
    /// *which* records come back, so "the 20 newest postings" rather
    /// than an arbitrary 20. Not the same thing as `sort_col`, which
    /// only reorders the records already in hand; the two are never
    /// interchangeable, because the display sort ranks within whatever
    /// this selected.
    ///
    /// Persisted with the rest, because it is part of what makes a
    /// query reproducible: text, result count and this together are the
    /// whole configuration, and a tab that came back with a different
    /// one was not the query you saved. (It was once left out on the
    /// argument that it is chosen while composing — but that argument
    /// applies just as well to the query text.)
    pub ads_sort: String,
}

/// A tab with no stored sort ordered by year, newest first — what every
/// scope did before the sort became per-tab.
pub const DEFAULT_SORT: (&str, bool) = ("year", false);

pub const DEFAULT_LIMIT: usize = 100;

fn state_file() -> PathBuf {
    let base = std::env::var("ASTROBIB_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/astrobib")
        });
    base.join("tabs.json")
}

/// The context key of a manuscript's own set: its root path, as this
/// process resolved it.
fn context_key(ms_root: &Path) -> String {
    ms_root.to_string_lossy().into_owned()
}

fn read_contexts() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(state_file())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("contexts").and_then(|c| c.as_object()).cloned())
        .unwrap_or_default()
}

/// One stored tab, decoded as tolerantly as the format promises: `id`
/// and `query` are the only things a tab cannot be without, and every
/// other field falls back to what a tab written before it existed ran
/// with. This is what lets a new field ship without a version gate, so
/// it must not become a derive that rejects older files.
fn tab_from_json(t: &serde_json::Value) -> Option<Tab> {
    Some(Tab {
        id: t.get("id")?.as_str()?.to_string(),
        query: t.get("query")?.as_str()?.to_string(),
        label: t
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        limit: t
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_LIMIT as u64) as usize,
        created: t.get("created").and_then(|v| v.as_i64()).unwrap_or(0),
        refreshed: t.get("refreshed").and_then(|v| v.as_i64()),
        sort_col: t
            .get("sort_col")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_SORT.0)
            .to_string(),
        sort_asc: t
            .get("sort_asc")
            .and_then(|v| v.as_bool())
            .unwrap_or(DEFAULT_SORT.1),
        // tabs written before this was stored simply have no
        // entry, and the default is what they ran with anyway
        ads_sort: t
            .get("ads_sort")
            .and_then(|v| v.as_str())
            .unwrap_or(crate::ads::DEFAULT_ADS_SORT)
            .to_string(),
    })
}

fn tabs_under(
    contexts: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Vec<Tab> {
    contexts
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(tab_from_json).collect())
        .unwrap_or_default()
}

/// Every query this session can see: the global set, then the active
/// manuscript's own. Global first because that is the order the strip
/// groups them in, and the order has to be decided once.
///
/// An id seen twice is kept once, the first copy winning. Two sets can
/// name the same tab — a move interrupted between the read and the
/// write, or a manuscript reached by two spellings of its path — and a
/// duplicate id is not cosmetic: results route to the first scope whose
/// id matches, so the twin would wait for a result that never arrives,
/// and closing either one drops the cache entry both were reading.
pub fn load(ms_root: Option<&Path>) -> Vec<(Tab, Home)> {
    collect(&read_contexts(), ms_root.map(context_key).as_deref())
}

/// `load` without the file, which is where its whole behaviour lives.
fn collect(
    contexts: &serde_json::Map<String, serde_json::Value>,
    ms_key: Option<&str>,
) -> Vec<(Tab, Home)> {
    let mut out: Vec<(Tab, Home)> = tabs_under(contexts, GLOBAL)
        .into_iter()
        .map(|t| (t, Home::Global))
        .collect();
    if let Some(key) = ms_key {
        for t in tabs_under(contexts, key) {
            if !out.iter().any(|(seen, _)| seen.id == t.id) {
                out.push((t, Home::Local));
            }
        }
    }
    out
}

fn tab_to_json(t: &Tab) -> serde_json::Value {
    serde_json::json!({
        "id": t.id,
        "query": t.query,
        "label": t.label,
        "limit": t.limit,
        "created": t.created,
        "refreshed": t.refreshed,
        "sort_col": t.sort_col,
        "sort_asc": t.sort_asc,
        "ads_sort": t.ads_sort,
    })
}

fn arr_of(tabs: &[(Tab, Home)], home: Home) -> serde_json::Value {
    serde_json::Value::Array(
        tabs.iter().filter(|(_, h)| *h == home).map(|(t, _)| tab_to_json(t)).collect(),
    )
}

/// `save` without the file: this session's two sets laid over whatever
/// the file already held, every other key carried across untouched.
fn filed(
    mut contexts: serde_json::Map<String, serde_json::Value>,
    tabs: &[(Tab, Home)],
    ms_key: Option<&str>,
) -> serde_json::Map<String, serde_json::Value> {
    contexts.insert(GLOBAL.to_string(), arr_of(tabs, Home::Global));
    // with no manuscript active there is no second home to write to,
    // and nothing can be filed under one — the gesture that would do it
    // is not offered
    if let Some(key) = ms_key {
        contexts.insert(key.to_string(), arr_of(tabs, Home::Local));
    }
    contexts
}

/// Write both of this session's sets, preserving every other context
/// untouched — another manuscript's queries are not this session's to
/// rewrite. Writing them together is what makes moving a query between
/// homes one step: it leaves one key and joins the other in a single
/// atomic write, so no crash can leave it in both or neither.
///
/// The caller reports a failure: saved queries are user state, and a
/// state dir that has gone unwritable must not be discovered only at
/// the next launch.
pub fn save(tabs: &[(Tab, Home)], ms_root: Option<&Path>) -> std::io::Result<()> {
    let contexts = filed(read_contexts(), tabs, ms_root.map(context_key).as_deref());
    let file = state_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let doc = serde_json::json!({ "contexts": contexts });
    crate::library::write_atomic(&file, &serde_json::to_string_pretty(&doc).unwrap_or_default())
}

/// Readable tab label: drop field names, quotes, and operator wrapping,
/// then truncate to 22 characters.
pub fn short_label(query: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static OP: OnceLock<Regex> = OnceLock::new();
    static FIELD: OnceLock<Regex> = OnceLock::new();
    static WS: OnceLock<Regex> = OnceLock::new();
    let op = OP.get_or_init(|| {
        Regex::new(r"^(references|citations|similar|trending|useful)\((.+)\)$").unwrap()
    });
    let field = FIELD.get_or_init(|| Regex::new(r"\b\w+:").unwrap());
    let ws = WS.get_or_init(|| Regex::new(r"\s+").unwrap());
    let q = query.trim();
    let (prefix, body) = match op.captures(q) {
        Some(c) => {
            let p = match c.get(1).unwrap().as_str() {
                "references" => "refs←",
                "citations" => "cites→",
                "similar" => "~",
                "trending" => "trend:",
                _ => "use:",
            };
            (p, c.get(2).unwrap().as_str().to_string())
        }
        None => ("", q.to_string()),
    };
    let body = field.replace_all(&body, "");
    let body = body.replace(['"', '(', ')'], "");
    let body = ws.replace_all(&body, " ");
    let label: String = format!("{prefix}{}", body.trim()).chars().take(22).collect();
    if label.is_empty() {
        query.chars().take(22).collect()
    } else {
        label
    }
}

/// Unix seconds now; also the entropy source for tab ids.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn make_tab(query: &str, limit: usize) -> Tab {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let id = format!("{:08x}", (nanos ^ (std::process::id() as u64) << 20) & 0xffff_ffff);
    Tab {
        id,
        query: query.to_string(),
        label: short_label(query),
        limit,
        created: now_secs(),
        refreshed: None,
        sort_col: DEFAULT_SORT.0.to_string(),
        sort_asc: DEFAULT_SORT.1,
        ads_sort: crate::ads::DEFAULT_ADS_SORT.to_string(),
    }
}

// ── cached query results ────────────────────────────────────────────
//
// The last results of every saved tab: startup restores scopes from
// here instantly (and offline) instead of re-querying ADS; r refreshes
// on demand.
//
// This is cache, not state — every byte of it is one ADS round-trip
// away from coming back — so it lives in the machine-local cache dir
// beside the PDFs rather than next to tabs.json. `rm -rf ~/.cache/
// astrobib` is then the supported way to reclaim it, with nothing
// curated in the blast radius. (A file left at the old state-dir path
// by an earlier build is simply ignored. Safe here for a narrow reason
// that does not generalise: the cache was introduced and moved within
// 0.8.0's development, so the state-dir path was never in a released
// build. Every other state file has real versions in the wild.)

pub fn cache_file() -> PathBuf {
    crate::library::cache_dir().join("query_cache.json")
}

fn read_cache() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(cache_file())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("tabs").and_then(|c| c.as_object()).cloned())
        .unwrap_or_default()
}

fn write_cache(tabs: serde_json::Map<String, serde_json::Value>) {
    let v = serde_json::json!({ "version": 1, "tabs": tabs });
    if let Some(dir) = cache_file().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        cache_file(),
        serde_json::to_string(&v).unwrap_or_default(),
    );
}

pub fn load_cached_articles(tab_id: &str) -> Vec<crate::ads::Article> {
    read_cache()
        .get(tab_id)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(crate::ads::article_from_doc).collect())
        .unwrap_or_default()
}

pub fn save_cached_articles(tab_id: &str, articles: &[crate::ads::Article]) {
    let mut tabs = read_cache();
    tabs.insert(
        tab_id.to_string(),
        serde_json::Value::Array(articles.iter().map(crate::ads::article_to_json).collect()),
    );
    write_cache(tabs);
}

pub fn drop_cached_articles(tab_id: &str) {
    let mut tabs = read_cache();
    if tabs.remove(tab_id).is_some() {
        write_cache(tabs);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_labels() {
        assert_eq!(super::short_label(r#"author:"^andersson" year:2019-"#), "^andersson 2019-");
        // the 22-char truncation cuts mid-bibcode by design
        assert_eq!(
            super::short_label(r#"references(bibcode:"2020ApJ...123..456Z")"#),
            "refs←2020ApJ...123..45"
        );
        assert_eq!(super::short_label("kilonova ejecta"), "kilonova ejecta");
    }

    use super::{collect, filed, Home};

    /// The contexts map of a tabs.json written as a literal — no file,
    /// no environment. `state_file()` reads a process-global env var,
    /// and cargo runs these on threads of one process, so a test that
    /// pointed it somewhere would race every other test that reads it.
    fn ctx(json: &str) -> serde_json::Map<String, serde_json::Value> {
        serde_json::from_str::<serde_json::Value>(json)
            .unwrap()
            .get("contexts")
            .and_then(|c| c.as_object())
            .cloned()
            .unwrap()
    }

    fn ids(v: &[(super::Tab, Home)]) -> Vec<(&str, Home)> {
        v.iter().map(|(t, h)| (t.id.as_str(), *h)).collect()
    }

    const MS: &str = "/u/paper";

    #[test]
    fn global_queries_come_first_then_the_manuscript_s_own() {
        let c = ctx(
            r#"{"contexts": {
                "/u/paper": [{"id": "l1", "query": "citations(identifier:X)"}],
                "global":   [{"id": "g1", "query": "kilonova"},
                             {"id": "g2", "query": "magnetar"}]
            }}"#,
        );
        // global first whichever order the file happens to hold them in:
        // the strip groups them that way and the order is decided here
        assert_eq!(
            ids(&collect(&c, Some(MS))),
            vec![("g1", Home::Global), ("g2", Home::Global), ("l1", Home::Local)]
        );
    }

    /// Without a manuscript there is no second set to read. A stored
    /// manuscript key is another directory's business, not this
    /// session's — reading it would put queries on screen that no
    /// gesture here could explain.
    #[test]
    fn no_manuscript_reads_only_the_global_set() {
        let c = ctx(
            r#"{"contexts": {
                "global":   [{"id": "g1", "query": "kilonova"}],
                "/u/paper": [{"id": "l1", "query": "magnetar"}]
            }}"#,
        );
        assert_eq!(ids(&collect(&c, None)), vec![("g1", Home::Global)]);
    }

    /// One id, one tab. Two sets can name the same tab — a manuscript
    /// reached by two spellings of its path, or a move interrupted
    /// between the read and the write — and the duplicate is not
    /// cosmetic: results route to the first scope whose id matches, so
    /// the twin would wait forever for a result delivered elsewhere,
    /// and closing either would drop the cache entry both were reading.
    #[test]
    fn an_id_in_both_sets_is_kept_once_as_global() {
        let c = ctx(
            r#"{"contexts": {
                "global":   [{"id": "dup", "query": "kilonova", "label": "the global copy"}],
                "/u/paper": [{"id": "dup", "query": "kilonova", "label": "the local copy"}]
            }}"#,
        );
        let got = collect(&c, Some(MS));
        assert_eq!(ids(&got), vec![("dup", Home::Global)]);
        assert_eq!(got[0].0.label, "the global copy");
    }

    #[test]
    fn filing_writes_both_sets_and_leaves_other_manuscripts_alone() {
        let c = ctx(
            r#"{"contexts": {
                "global":      [{"id": "old", "query": "gone"}],
                "/u/other":    [{"id": "x", "query": "someone else's paper"}]
            }}"#,
        );
        let tabs = collect(
            &ctx(
                r#"{"contexts": {
                    "global":   [{"id": "g1", "query": "kilonova"}],
                    "/u/paper": [{"id": "l1", "query": "magnetar"}]
                }}"#,
            ),
            Some(MS),
        );
        let out = filed(c, &tabs, Some(MS));
        assert_eq!(out["global"].as_array().unwrap().len(), 1);
        assert_eq!(out["global"][0]["id"], "g1");
        assert_eq!(out[MS][0]["id"], "l1");
        // another manuscript's queries are not this session's to rewrite
        assert_eq!(out["/u/other"][0]["id"], "x");
    }

    /// Moving a query is one write of both sets, so it can never be in
    /// both (a duplicate id) or in neither.
    #[test]
    fn moving_a_query_leaves_one_set_and_joins_the_other() {
        let tabs = vec![(
            collect(&ctx(r#"{"contexts": {"global": [{"id": "t", "query": "q"}]}}"#), None)
                .remove(0)
                .0,
            Home::Local,
        )];
        let out = filed(serde_json::Map::new(), &tabs, Some(MS));
        assert!(out["global"].as_array().unwrap().is_empty());
        assert_eq!(out[MS][0]["id"], "t");
    }

    /// A round trip through the file's shape changes nothing, which is
    /// what lets a session save repeatedly without drift.
    #[test]
    fn filing_then_collecting_returns_what_went_in() {
        let before = collect(
            &ctx(
                r#"{"contexts": {
                    "global":   [{"id": "g1", "query": "kilonova"}],
                    "/u/paper": [{"id": "l1", "query": "magnetar"}]
                }}"#,
            ),
            Some(MS),
        );
        let after = collect(&filed(serde_json::Map::new(), &before, Some(MS)), Some(MS));
        assert_eq!(before, after);
    }

    /// The tolerant decode, which is what lets a field ship without a
    /// version gate. Junk is skipped rather than fatal, and a tab that
    /// predates a field gets what it ran with.
    #[test]
    fn junk_is_skipped_and_missing_fields_default() {
        let c = ctx(
            r#"{"contexts": {
                "global": [
                    "not a tab",
                    {"query": "no id"},
                    {"id": "no query"},
                    {"id": "ok", "query": "kilonova", "unknown_future_field": 7}
                ],
                "/u/paper": "not even an array"
            }}"#,
        );
        let got = collect(&c, Some(MS));
        assert_eq!(ids(&got), vec![("ok", Home::Global)]);
        let t = &got[0].0;
        assert_eq!(t.limit, super::DEFAULT_LIMIT);
        assert_eq!((t.sort_col.as_str(), t.sort_asc), super::DEFAULT_SORT);
        assert_eq!(t.ads_sort, crate::ads::DEFAULT_ADS_SORT);
    }
}
