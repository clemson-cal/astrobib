//! Per-paper scalar metrics, user-local in metrics.json beside
//! state.json — never in any bib database. `priority` is curated user
//! data — a 0..1 level that decays over time; `citations` is
//! cache-like, refreshable from ADS.

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default, Clone)]
pub struct PaperMetrics {
    pub priority: Option<f64>,
    pub priority_at: Option<i64>,
    pub citations: Option<i64>,
    pub citations_at: Option<i64>,
}

/// Priority half-life: a level halves every week untouched.
pub const HALF_LIFE_SECS: f64 = 7.0 * 24.0 * 3600.0;

impl PaperMetrics {
    /// The decayed, displayed priority.
    pub fn effective_priority(&self, now: i64) -> Option<f64> {
        let p = self.priority?;
        let at = self.priority_at.unwrap_or(now);
        let dt = (now - at).max(0) as f64;
        Some((p * 0.5f64.powf(dt / HALF_LIFE_SECS)).clamp(0.0, 1.0))
    }
}

#[derive(Default)]
pub struct Metrics {
    pub papers: HashMap<String, PaperMetrics>,
    dirty: bool,
    /// Why the last write failed, cleared by the next success. `save`
    /// reports through this rather than returning a Result: priorities
    /// are flushed from idle ticks and from fire-and-forget CLI paths,
    /// and a must_use return would force noise on every one of them.
    error: Option<String>,
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
                        priority: pm.get("priority").and_then(|x| x.as_f64()),
                        priority_at: pm.get("priority_at").and_then(|x| x.as_i64()),
                        citations: pm.get("citations").and_then(|x| x.as_i64()),
                        citations_at: pm.get("citations_at").and_then(|x| x.as_i64()),
                    },
                );
            }
        }
        m
    }

    /// The last write failure, if the store is still carrying one.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Write-on-change; a no-op while clean. False means the write
    /// failed: the store stays dirty (so the next flush retries) and
    /// `error()` says why — curated priorities are user data, and losing
    /// them silently is the one outcome worth reporting.
    pub fn save(&mut self) -> bool {
        if !self.dirty {
            return true;
        }
        let papers: serde_json::Map<String, serde_json::Value> = self
            .papers
            .iter()
            .map(|(k, p)| {
                let mut o = serde_json::Map::new();
                if let Some(v) = p.priority {
                    o.insert("priority".into(), serde_json::json!(v));
                }
                if let Some(t) = p.priority_at {
                    o.insert("priority_at".into(), t.into());
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
            if let Err(e) = std::fs::create_dir_all(dir) {
                self.error = Some(e.to_string());
                return false;
            }
        }
        match std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default() + "\n") {
            Ok(()) => {
                self.dirty = false;
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e.to_string());
                false
            }
        }
    }

    /// Set the priority level outright (`.` → 1.0, `0` → 0.0); the
    /// decay clock restarts from now.
    pub fn set_priority(&mut self, key: &str, level: f64) -> f64 {
        let level = level.clamp(0.0, 1.0);
        let p = self.papers.entry(key.to_string()).or_default();
        p.priority = Some(level);
        p.priority_at = Some(now());
        self.dirty = true;
        level
    }

    /// Scale the *effective* priority multiplicatively and restart
    /// decay from the result — what `<` / `>` do. Growing from zero
    /// starts at a small floor so the gesture always does something.
    /// Returns the new level.
    pub fn scale_priority(&mut self, key: &str, factor: f64) -> f64 {
        let n = now();
        let cur = self
            .papers
            .get(key)
            .and_then(|p| p.effective_priority(n))
            .unwrap_or(0.0);
        let new = if factor > 1.0 {
            (cur.max(0.05) * factor).min(1.0)
        } else {
            cur * factor
        };
        self.set_priority(key, new)
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

    /// Drop metrics for papers that no longer exist. The caller must
    /// pass EVERY live key across both tiers — pruning against a
    /// partial view (a hidden global tier, a relocated library) would
    /// destroy curated priorities, so callers guard accordingly.
    pub fn prune(&mut self, live: &std::collections::HashSet<String>) -> usize {
        let before = self.papers.len();
        self.papers.retain(|k, _| live.contains(k));
        let dropped = before - self.papers.len();
        if dropped > 0 {
            self.dirty = true;
        }
        dropped
    }
}
