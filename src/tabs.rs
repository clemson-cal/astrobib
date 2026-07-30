//! Persistent ADS query tabs, stored in the app's own tabs.json state
//! file: user-local, keyed by manuscript context ("global" when none),
//! never written into a manuscript repo.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: String,
    pub query: String,
    pub label: String,
    pub limit: usize,
    pub created: i64,
    pub refreshed: Option<i64>,
    pub bibcodes: Vec<String>,
}

pub const DEFAULT_LIMIT: usize = 100;

fn state_file() -> PathBuf {
    let base = std::env::var("ASTROBIB_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/astrobib")
        });
    base.join("tabs.json")
}

fn context_key(ms_root: Option<&Path>) -> String {
    ms_root
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "global".to_string())
}

fn read_contexts() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(state_file())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("contexts").and_then(|c| c.as_object()).cloned())
        .unwrap_or_default()
}

pub fn load(ms_root: Option<&Path>) -> Vec<Tab> {
    let contexts = read_contexts();
    let Some(tabs) = contexts.get(&context_key(ms_root)).and_then(|v| v.as_array()) else {
        return vec![];
    };
    tabs.iter()
        .filter_map(|t| {
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
                bibcodes: t
                    .get("bibcodes")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Write this context's tabs, preserving every other context untouched.
pub fn save(tabs: &[Tab], ms_root: Option<&Path>) {
    let mut contexts = read_contexts();
    let arr: Vec<serde_json::Value> = tabs
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "query": t.query,
                "label": t.label,
                "limit": t.limit,
                "created": t.created,
                "refreshed": t.refreshed,
                "bibcodes": t.bibcodes,
            })
        })
        .collect();
    contexts.insert(context_key(ms_root), serde_json::Value::Array(arr));
    let file = state_file();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let doc = serde_json::json!({ "contexts": contexts });
    let _ = std::fs::write(file, serde_json::to_string_pretty(&doc).unwrap_or_default());
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
        bibcodes: vec![],
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_labels() {
        assert_eq!(super::short_label(r#"author:"^zrake" year:2019-"#), "^zrake 2019-");
        // the 22-char truncation cuts mid-bibcode by design
        assert_eq!(
            super::short_label(r#"references(bibcode:"2020ApJ...123..456Z")"#),
            "refs←2020ApJ...123..45"
        );
        assert_eq!(super::short_label("kilonova ejecta"), "kilonova ejecta");
    }
}

// ── cached query results ────────────────────────────────────────────
//
// The last results of every saved tab, in query_cache.json beside
// tabs.json: startup restores scopes from here instantly (and offline)
// instead of re-querying ADS; r refreshes on demand.

fn cache_file() -> PathBuf {
    state_file().with_file_name("query_cache.json")
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

/// Every tab id across every context — what the query cache may
/// legitimately hold.
pub fn all_tab_ids() -> std::collections::HashSet<String> {
    read_contexts()
        .values()
        .filter_map(|v| v.as_array())
        .flatten()
        .filter_map(|t| t.get("id").and_then(|i| i.as_str()).map(str::to_string))
        .collect()
}

/// The tab ids the query cache currently holds, with their byte sizes.
pub fn cached_tab_entries() -> Vec<(String, usize)> {
    read_cache()
        .iter()
        .map(|(k, v)| (k.clone(), v.to_string().len()))
        .collect()
}
