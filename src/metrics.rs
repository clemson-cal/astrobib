//! Per-paper scalar metrics, user-local in metrics.json beside
//! state.json — never in any bib database. `touched` is curated user
//! data (the manually-resettable "age"); `citations` is cache-like,
//! refreshable from ADS.

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default, Clone)]
pub struct PaperMetrics {
    pub touched: Option<i64>,
    pub citations: Option<i64>,
    pub citations_at: Option<i64>,
}

#[derive(Default)]
pub struct Metrics {
    pub papers: HashMap<String, PaperMetrics>,
    dirty: bool,
}

fn metrics_file() -> PathBuf {
    let base = std::env::var("ASTROBIB_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/astrobib")
        });
    base.join("metrics.json")
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Metrics {
    pub fn load() -> Metrics {
        let mut m = Metrics::default();
        let Ok(text) = std::fs::read_to_string(metrics_file()) else {
            return m;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            return m;
        };
        if let Some(papers) = v.get("papers").and_then(|p| p.as_object()) {
            for (key, pm) in papers {
                m.papers.insert(
                    key.clone(),
                    PaperMetrics {
                        touched: pm.get("touched").and_then(|x| x.as_i64()),
                        citations: pm.get("citations").and_then(|x| x.as_i64()),
                        citations_at: pm.get("citations_at").and_then(|x| x.as_i64()),
                    },
                );
            }
        }
        m
    }

    /// Write-on-change; a no-op while clean.
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let papers: serde_json::Map<String, serde_json::Value> = self
            .papers
            .iter()
            .map(|(k, p)| {
                let mut o = serde_json::Map::new();
                if let Some(t) = p.touched {
                    o.insert("touched".into(), t.into());
                }
                if let Some(c) = p.citations {
                    o.insert("citations".into(), c.into());
                }
                if let Some(t) = p.citations_at {
                    o.insert("citations_at".into(), t.into());
                }
                (k.clone(), serde_json::Value::Object(o))
            })
            .collect();
        let v = serde_json::json!({ "version": 1, "papers": papers });
        let path = metrics_file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default() + "\n")
            .is_ok()
        {
            self.dirty = false;
        }
    }

    /// Seed a paper's age from its file's creation time the first time
    /// it is seen — existing history migrates in and survives clones.
    pub fn seed_touched(&mut self, key: &str, ts: i64) {
        let p = self.papers.entry(key.to_string()).or_default();
        if p.touched.is_none() {
            p.touched = Some(ts);
            self.dirty = true;
        }
    }

    /// `.` — reset the age to now.
    pub fn touch(&mut self, key: &str) {
        self.papers.entry(key.to_string()).or_default().touched = Some(now());
        self.dirty = true;
    }

    pub fn set_citations(&mut self, key: &str, n: i64) {
        let p = self.papers.entry(key.to_string()).or_default();
        if p.citations != Some(n) || p.citations_at.is_none() {
            p.citations = Some(n);
            p.citations_at = Some(now());
            self.dirty = true;
        }
    }

    pub fn get(&self, key: &str) -> Option<&PaperMetrics> {
        self.papers.get(key)
    }
}
