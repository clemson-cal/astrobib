//! The swatch column's two sources: your priority, ADS's citations.

use super::*;

/// The one-cell metric swatch column: one scalar per paper, colored
/// by a per-metric colormap so the hue family names the metric.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MetricCol {
    Priority,  // viridis — user-curated 0..1 level, decaying over time
    Citations, // magma — ADS citation count
}

impl MetricCol {
    /// M picks *which* metric the swatch shows. Whether it shows at all
    /// is the columns panel's business, like every other column — so
    /// there is no "off" here to cycle through.
    pub(super) fn next(self) -> Self {
        match self {
            MetricCol::Priority => MetricCol::Citations,
            MetricCol::Citations => MetricCol::Priority,
        }
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            MetricCol::Priority => "priority (viridis)",
            MetricCol::Citations => "citations (magma)",
        }
    }
    pub(super) fn state_tag(self) -> &'static str {
        match self {
            MetricCol::Priority => "priority",
            MetricCol::Citations => "citations",
        }
    }
    pub(super) fn from_tag(s: &str) -> Self {
        match s {
            "citations" => MetricCol::Citations,
            _ => MetricCol::Priority,
        }
    }
}

/// A priority edit: set outright or nudge the effective level.
#[derive(Clone, Copy)]
pub(super) enum PriorityOp {
    Set(f64),
    Scale(f64),
}

/// The metric swatch: one cell beside the table rather than inside it,
/// but a column like any other — now literally, drawn inside the table
/// rather than as a strip beside it, which is what gives it the same
/// column order, header hover and click handling as the rest. Off until
/// asked for, and never resizable. M chooses which metric it shows.
pub(super) fn metric_column(metric: MetricCol) -> table::ColumnSpec {
    table::fixed(Col::Metric, "⣿", 2, true)
        .default_off()
        .fixed_size()
        // the legend carries its colormap's hue, the only thing on
        // screen naming which metric is showing
        .styled_header(
            Style::default()
                .fg(metric_color(metric, 0.65))
                .add_modifier(Modifier::BOLD),
        )
}

/// One row's metric swatch. Priority IS a 0..1 level, so it is coloured
/// absolutely and an edit recolours in place; citations rank-normalize
/// over the scope, so the whole ramp gets used.
pub(super) fn metric_cell(
    metric: MetricCol,
    v: Option<f64>,
    known: &[f64],
) -> ratatui::widgets::Cell<'static> {
    match v {
        Some(v) => {
            let t = match metric {
                MetricCol::Priority => v,
                _ => rank_norm(known, v),
            };
            ratatui::widgets::Cell::from(Span::styled(
                " ",
                Style::default().bg(metric_color(metric, t)),
            ))
        }
        None => ratatui::widgets::Cell::from(Span::styled("·", divider())),
    }
}

/// Map a normalized [0,1] value through the metric's colormap.
fn metric_color(metric: MetricCol, t: f64) -> Color {
    let g = match metric {
        MetricCol::Priority => colorous::VIRIDIS,
        _ => colorous::MAGMA,
    };
    let c = g.eval_continuous(t.clamp(0.0, 1.0));
    Color::Rgb(c.r, c.g, c.b)
}

/// Rank-normalize a row's value against the visible set: robust to
/// outliers, and every colormap stop gets used.
fn rank_norm(vals: &[f64], v: f64) -> f64 {
    if vals.len() < 2 {
        return 0.5;
    }
    let below = vals.iter().filter(|x| **x < v).count();
    below as f64 / (vals.len() - 1) as f64
}

impl App {
    /// Flush the metrics store — priorities are hand-curated user data,
    /// so a write that quietly stops working gets said out loud.
    pub(super) fn save_metrics(&mut self) {
        let err = (!self.metrics.save())
            .then(|| self.metrics.error().unwrap_or("write failed").to_string());
        self.state_write("metrics.json", err);
    }

    /// Priority targets: the selection, else the cursor entry (in a
    /// query scope, the imported twin).
    fn priority_targets(&mut self) -> Vec<String> {
        let mut keys = self.action_keys();
        if keys.is_empty() {
            if let Some(k) = self.card_entry_key() {
                keys.push(k);
            }
        }
        keys
    }

    /// `.` → 1.0, `0` → 0.0, `<`/`>` scale by ×0.8/×1.25 — multi-select
    /// aware, with the resulting level in the footer and the swatch
    /// recoloring on the next frame.
    pub(super) fn adjust_priority(&mut self, op: PriorityOp) {
        let keys = self.priority_targets();
        if keys.is_empty() {
            let msg = if self.on_query() {
                "import the paper first (i) — priority is per library entry"
            } else {
                "no paper to prioritize"
            };
            self.note(MsgCat::Warn, msg.to_string());
            return;
        }
        let mut last = 0.0;
        for k in &keys {
            last = match op {
                PriorityOp::Set(v) => self.metrics.set_priority(k, v),
                PriorityOp::Scale(f) => self.metrics.scale_priority(k, f),
            };
        }
        // no disk write here: repeated keys must stay instant — the
        // idle tick (and quit) flushes the dirty store
        let what = if keys.len() == 1 {
            keys[0].clone()
        } else {
            format!("{} papers", keys.len())
        };
        self.note(MsgCat::Ok, format!("priority {last:.2} — {what}"));
    }

    /// r in the library scope: one batched ADS query refreshes the
    /// citation counts of every visible entry.
    pub(super) fn refresh_citation_counts(&mut self) {
        if self.cit_rx.is_some() {
            return;
        }
        let bibs: Vec<(String, String)> = self
            .filtered
            .iter()
            .filter_map(|&i| self.entry_at(i))
            .filter_map(|e| e.bibcode().map(|b| (b.to_string(), e.key().to_string())))
            .collect();
        if bibs.is_empty() || crate::ads::get_token().is_none() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.cit_rx = Some(rx);
        self.add_task(TaskKind::Query, "⌕ citation counts".to_string(), vec![]);
        std::thread::spawn(move || {
            let q = format!(
                "bibcode:({})",
                bibs.iter().map(|(b, _)| b.as_str()).collect::<Vec<_>>().join(" OR ")
            );
            let n = bibs.len();
            let out: Vec<(String, i64)> = match crate::ads::search(&q, n) {
                Ok(arts) => arts
                    .into_iter()
                    .filter_map(|a| {
                        let key = bibs.iter().find(|(b, _)| *b == a.bibcode)?.1.clone();
                        Some((key, a.citation_count?))
                    })
                    .collect(),
                Err(_) => vec![],
            };
            let _ = tx.send(out);
        });
        self.note(MsgCat::Info, "refreshing citation counts…".to_string());
    }

    /// ⟳ on the card — refresh one paper's citation count.
    pub(super) fn refresh_citation_count_for(&mut self, key: &str) {
        if self.cit_rx.is_some() {
            self.note(MsgCat::Warn, "a citation refresh is already running".to_string());
            return;
        }
        let Some(bc) = self.lib.get(key).and_then(|e| e.bibcode().map(str::to_string)) else {
            self.note(MsgCat::Warn, "no bibcode for that entry".to_string());
            return;
        };
        if crate::ads::get_token().is_none() {
            self.note(MsgCat::Warn, "no ADS token — press S to set one".to_string());
            return;
        }
        let key = key.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.cit_rx = Some(rx);
        self.add_task(TaskKind::Query, "⌕ citation counts".to_string(), vec![]);
        std::thread::spawn(move || {
            let out: Vec<(String, i64)> =
                match crate::ads::search(&format!("identifier:{bc}"), 1) {
                    Ok(arts) => arts
                        .into_iter()
                        .filter_map(|a| Some((key.clone(), a.citation_count?)))
                        .collect(),
                    Err(_) => vec![],
                };
            let _ = tx.send(out);
        });
    }

    pub(super) fn drain_citations(&mut self) {
        let Some(rx) = &self.cit_rx else { return };
        match rx.try_recv() {
            Ok(counts) => {
                let n = counts.len();
                let bcs: Vec<(String, i64)> = counts
                    .iter()
                    .filter_map(|(k, c)| {
                        self.lib.get(k).and_then(|e| e.bibcode()).map(|b| (b.to_string(), *c))
                    })
                    .collect();
                for (k, c) in counts {
                    self.metrics.set_citations(&k, c);
                }
                for s in &mut self.scopes {
                    if let Scope::Ads { articles, .. } = s {
                        for a in articles.iter_mut() {
                            if let Some((_, c)) = bcs.iter().find(|(b, _)| *b == a.bibcode) {
                                a.citation_count = Some(*c);
                            }
                        }
                    }
                }
                self.save_metrics();
                if let Some(t) = self.tasks.iter().find(|t| t.label == "⌕ citation counts") {
                    let id = t.id;
                    self.finish_task(id);
                }
                self.note(MsgCat::Ok, format!("citation counts refreshed ({n})"));
                self.cit_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.cit_rx = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Every row's metric value in the active scope, in row order, with
    /// the known ones pooled for rank-normalizing. None where a paper
    /// has no value for the metric on show.
    pub(super) fn metric_values(&self) -> (Vec<Option<f64>>, Vec<f64>) {
        let metric = self.metric_col;
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let values: Vec<Option<f64>> = match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => articles
                .iter()
                .map(|a| match metric {
                    MetricCol::Citations => a.citation_count.map(|c| c as f64),
                    MetricCol::Priority => self
                        .article_entry(a)
                        .and_then(|e| self.metrics.get(e.key()))
                        .and_then(|m| m.effective_priority(now_ts)),
                })
                .collect(),
            _ => self
                .filtered
                .iter()
                .filter_map(|&i| self.entry_at(i))
                .map(|e| {
                    let m = self.metrics.get(e.key());
                    match metric {
                        MetricCol::Priority => m.and_then(|m| m.effective_priority(now_ts)),
                        MetricCol::Citations => m.and_then(|m| m.citations).map(|c| c as f64),
                    }
                })
                .collect(),
        };
        let known: Vec<f64> = values.iter().flatten().copied().collect();
        (values, known)
    }
}
