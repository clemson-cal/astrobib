//! Which rows exist, in what order, and which of them are chosen.

use super::*;

/// Read a stored sort ("column:asc" / "column:desc") from state.json.
/// An absent or unparseable field means "no stored sort", which each
/// caller resolves to its own default.
pub(super) fn load_sort(field: &str) -> Option<(Col, bool)> {
    let raw = crate::ads::get_state_field(field)?;
    let (col, dir) = raw.split_once(':')?;
    Some((Col::from_tag(col), dir == "asc"))
}

/// Persist one. None writes an empty value, which reads back as absent.
fn store_sort(field: &str, v: Option<(Col, bool)>) -> std::io::Result<()> {
    let raw = match v {
        Some((c, asc)) => format!("{}:{}", c.tag(), if asc { "asc" } else { "desc" }),
        None => String::new(),
    };
    crate::ads::save_state_field(field, &raw)
}

impl App {
    /// a/A — bulk selection: `a` toggles all *visible* rows (the
    /// filtered set — so with a filter active it is select-all-in-
    /// filter; with the global tier hidden it is tier-2-only), `A`
    /// selects every item in the scope regardless of filter. Selecting
    /// when everything is already selected deselects (a), mirroring the
    /// single-row toggle; an emptied selection exits the mode.
    pub(super) fn select_all(&mut self, visible_only: bool) {
        let ids: Vec<String> = match self.scopes.get(self.active_scope) {
            // every row that resolves to a paper; missing and ambiguous
            // cites are skipped rather than blocking the whole gesture
            Some(Scope::Manuscript { rows }) => {
                rows.iter().filter_map(|r| r.key.clone()).collect()
            }
            Some(Scope::Ads { articles, .. }) => {
                articles.iter().map(|a| a.bibcode.clone()).collect()
            }
            _ if visible_only => self
                .filtered
                .iter()
                .filter_map(|&i| self.order.get(i).cloned())
                .collect(),
            _ => self.order.clone(),
        };
        if ids.is_empty() {
            return;
        }
        self.select_mode = true;
        let all_in = ids.iter().all(|k| self.selected.contains(k));
        if all_in && visible_only {
            for k in &ids {
                self.selected.remove(k);
            }
            if self.selected.is_empty() {
                self.exit_select_mode();
                return;
            }
        } else {
            self.selected.extend(ids);
        }
        self.status = format!("{} selected", self.selected.len());
    }

    pub(super) fn action_keys(&self) -> Vec<String> {
        if let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) {
            let sel = self.selected_articles();
            let pool: Vec<&crate::ads::Article> = if !sel.is_empty() {
                sel
            } else {
                self.card_article_pos().and_then(|p| articles.get(p)).into_iter().collect()
            };
            return pool
                .iter()
                .filter_map(|a| self.article_entry(a))
                .map(|e| e.key().to_string())
                .collect();
        }
        if self.select_mode && !self.selected.is_empty() {
            // in the manuscript scope the visible rows are the truth:
            // self.order is the library's ordering, which omits rows a
            // cite resolves to only in the personal tier
            if matches!(self.scopes.get(self.active_scope), Some(Scope::Manuscript { .. })) {
                return self.selected_ms_keys();
            }
            return self
                .order
                .iter()
                .filter(|k| self.selected.contains(*k))
                .cloned()
                .collect();
        }
        self.selected_key().map(str::to_string).into_iter().collect()
    }

    pub(super) fn selected_key(&self) -> Option<&str> {
        // the manuscript scope resolves to library entries, so entry
        // actions apply there; ADS rows don't resolve until imported
        if let Some(Scope::Manuscript { rows }) = self.scopes.get(self.active_scope) {
            return self
                .table
                .selected()
                .and_then(|p| rows.get(p))
                .and_then(|r| r.key.as_deref());
        }
        if self.active_scope != 0 {
            return None;
        }
        let pos = self.table.selected()?;
        let idx = *self.filtered.get(pos)?;
        self.order.get(idx).map(String::as_str)
    }

    pub(super) fn refilter(&mut self) {
        let groups = query::tokenize(self.filter.value());
        let in_ms: Vec<String> = self
            .lib
            .manuscript
            .as_ref()
            .map(|m| m.entries().iter().map(|e| e.key().to_string()).collect())
            .unwrap_or_default();
        // metric snapshots only when the filter asks for them: pri:/cit:
        // are rare, and refilter runs on every keystroke
        let wants = |f: query::Field| {
            groups.iter().flatten().any(|t| t.field == Some(f))
        };
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let pri: std::collections::HashMap<String, f64> = if wants(query::Field::Pri) {
            self.metrics
                .papers
                .iter()
                .filter_map(|(k, m)| m.effective_priority(now_ts).map(|v| (k.clone(), v)))
                .collect()
        } else {
            Default::default()
        };
        let cit: std::collections::HashMap<String, f64> = if wants(query::Field::Cit) {
            self.metrics
                .papers
                .iter()
                .filter_map(|(k, m)| m.citations.map(|v| (k.clone(), v as f64)))
                .collect()
        } else {
            Default::default()
        };
        // key → its tags, snapshotted only when the filter asks: tag:
        // and is:tagged are rare and refilter runs on every keystroke,
        // same reasoning as the metrics above. Which terms count as
        // asking is the query language's own question, not this one's.
        let tagged: std::collections::HashMap<String, Vec<String>> = if query::needs_tags(&groups) {
            let mut m: std::collections::HashMap<String, Vec<String>> = Default::default();
            for (name, keys) in self.lib.tags() {
                for k in keys {
                    m.entry(k).or_default().push(name.to_lowercase());
                }
            }
            m
        } else {
            Default::default()
        };
        let ctx = QueryContext {
            in_manuscript: Some(Box::new(move |k: &str| in_ms.iter().any(|x| x == k))),
            has_pdf: Some(Box::new(|k: &str| has_cached_pdf(k))),
            priority: Some(Box::new(move |k: &str| pri.get(k).copied())),
            citations: Some(Box::new(move |k: &str| cit.get(k).copied())),
            tagged: Some(Box::new(move |k: &str, v: &str| {
                tagged.get(k).is_some_and(|ts| ts.iter().any(|t| t.contains(v)))
            })),
        };
        // filter_map, not filter: a key the library no longer holds
        // simply drops out of the visible set — which also keeps every
        // position in `filtered` resolvable for the table and the
        // metric strip, whatever state `order` is in
        self.filtered = self
            .order
            .iter()
            .enumerate()
            .filter_map(|(i, key)| {
                let e = self.lib.get(key)?;
                query::matches(&groups, e, &ctx).then_some(i)
            })
            .collect();
        let sel = self.table.selected().unwrap_or(0);
        self.table.select(if self.filtered.is_empty() {
            None
        } else {
            Some(sel.min(self.filtered.len() - 1))
        });
    }

    pub(super) fn row_count(&self) -> usize {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => articles.len(),
            Some(Scope::Manuscript { rows }) => rows.len(),
            _ => self.filtered.len(),
        }
    }

    pub(super) fn move_sel(&mut self, delta: isize) {
        if self.row_count() == 0 {
            return;
        }
        let cur = self.table.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.row_count() as isize - 1);
        self.table.select(Some(next as usize));
        self.pdf_status.clear(); // stale per-entry message
    }

    /// Toggle selection membership of the row at a filtered position.
    /// A selection emptied by toggling exits selection mode, same as Esc.
    pub(super) fn toggle_row_selected(&mut self, pos: usize) {
        let key = match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => {
                let Some(a) = articles.get(pos) else { return };
                a.bibcode.clone()
            }
            // a manuscript row selects the paper it resolves to; a
            // missing or ambiguous cite names no paper, so it cannot
            // join a selection keyed by cite key
            Some(Scope::Manuscript { rows }) => {
                let Some(key) = rows.get(pos).and_then(|r| r.key.clone()) else {
                    self.note(
                        MsgCat::Warn,
                        "this cite resolves to no paper — nothing to select".to_string(),
                    );
                    return;
                };
                key
            }
            _ => {
                let Some(&idx) = self.filtered.get(pos) else {
                    return;
                };
                let Some(key) = self.order.get(idx) else { return };
                key.clone()
            }
        };
        if !self.selected.remove(&key) {
            self.selected.insert(key);
        }
        if self.select_mode && self.selected.is_empty() {
            self.exit_select_mode();
        } else {
            self.status = format!("{} selected", self.selected.len());
        }
    }

    pub(super) fn exit_select_mode(&mut self) {
        self.select_mode = false;
        self.selected.clear();
        self.status = format!("{} papers", self.order.len());
    }

    /// Rebuild the display order (entries changed or sort changed),
    /// and refresh the manuscript classification when present.
    pub(super) fn rebuild_order(&mut self) {
        if self.manuscript_root().is_some()
            && matches!(self.scopes.get(1), Some(Scope::Manuscript { .. }))
        {
            let rows = self.ms_rows();
            self.sync_refs_bib(&rows);
            if let Some(s) = self.scopes.get_mut(1) {
                *s = Scope::Manuscript { rows };
            }
            // a rescan hands back scan order; re-assert the manuscript's
            // own sort over it
            self.sort_ms_rows();
        }
        self.order = self.lib.entries().iter().map(|e| e.key().to_string()).collect();
        let lib = &self.lib;
        // the library's own column, never the front scope's: importing a
        // paper while a query is showing must not reorder the library by
        // the query's sort
        let (col, asc) = self.library_sort;
        self.order.sort_by(|a, b| {
            let (ea, eb) = match (lib.get(a), lib.get(b)) {
                (Some(ea), Some(eb)) => (ea, eb),
                (x, y) => return orphan_order(x.is_some(), y.is_some(), a, b),
            };
            let ord = match col {
                Col::Metric => match self.metric_col {
                    MetricCol::Priority => {
                        let n = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        let pa = self
                            .metrics
                            .get(ea.key())
                            .and_then(|m| m.effective_priority(n))
                            .unwrap_or(-1.0);
                        let pb = self
                            .metrics
                            .get(eb.key())
                            .and_then(|m| m.effective_priority(n))
                            .unwrap_or(-1.0);
                        pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    MetricCol::Citations => {
                        let ca =
                            self.metrics.get(ea.key()).and_then(|m| m.citations).unwrap_or(-1);
                        let cb =
                            self.metrics.get(eb.key()).and_then(|m| m.citations).unwrap_or(-1);
                        ca.cmp(&cb)
                    }
                },
                Col::Pdf => has_cached_pdf(ea.key()).cmp(&has_cached_pdf(eb.key())),
                Col::InLib => lib
                    .in_manuscript(ea.key())
                    .cmp(&lib.in_manuscript(eb.key())),
                Col::Year => ea.year().cmp(&eb.year()),
                Col::Author => ea
                    .first_author_last()
                    .to_lowercase()
                    .cmp(&eb.first_author_last().to_lowercase()),
                Col::Title => ea
                    .title()
                    .trim_matches(['{', '}'])
                    .to_lowercase()
                    .cmp(&eb.title().trim_matches(['{', '}']).to_lowercase()),
                Col::Key => ea.key().cmp(eb.key()),
                // the gutter, the manuscript-only columns, and Entered
                // (which only ADS records carry) never sort the library;
                // listed rather than caught by a wildcard so that a new
                // sortable column has to be handled here
                Col::Sel | Col::CiteIcon | Col::Cited | Col::State | Col::Entered => {
                    std::cmp::Ordering::Equal
                }
            };
            let ord = if asc { ord } else { ord.reverse() };
            ord.then(a.cmp(b))
        });
        self.selected.retain(|k| lib.get(k).is_some());
        self.refilter();
        // our own writes touch bib/; refresh the watch snapshot so the
        // poll doesn't bounce them back as a reload. No longer gated on
        // a manuscript existing: the personal bib/ is watched too, and
        // that is the one every import writes to.
        self.watch = self.watch_snapshot();
    }

    /// The active scope's sort, or None where its rows have an inherent
    /// order (a manuscript in scan order).
    pub(super) fn sort(&self) -> Option<(Col, bool)> {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { tab, .. }) => Some((Col::from_tag(&tab.sort_col), tab.sort_asc)),
            Some(Scope::Manuscript { .. }) => self.ms_sort,
            _ => Some(self.library_sort),
        }
    }

    /// Write the active scope's sort back where that scope keeps it and
    /// persist it: a query tab into tabs.json, the library and the
    /// manuscript into state.json.
    pub(super) fn set_sort(&mut self, v: (Col, bool)) {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { .. }) => {
                if let Some(Scope::Ads { tab, .. }) = self.scopes.get_mut(self.active_scope) {
                    tab.sort_col = v.0.tag().to_string();
                    tab.sort_asc = v.1;
                }
                self.save_tabs();
            }
            Some(Scope::Manuscript { .. }) => {
                self.ms_sort = Some(v);
                let res = store_sort("manuscript_sort", self.ms_sort);
                self.state_write("state.json", res.err().map(|e| e.to_string()));
            }
            _ => {
                self.library_sort = v;
                let res = store_sort("library_sort", Some(v));
                self.state_write("state.json", res.err().map(|e| e.to_string()));
            }
        }
    }

    /// Header click: same column flips direction, a new column starts
    /// descending for Year (newest first) and ascending otherwise.
    pub(super) fn sort_by(&mut self, col: Col) {
        let next = self.next_sort(col);
        self.set_sort(next);
        self.apply_sort();
        let dir = if next.1 { "ascending" } else { "descending" };
        self.note_latest(
            MsgCat::Info,
            "sort",
            format!("sorted by {} {dir}", self.column_hint(col)),
        );
    }

    /// What clicking a column's sort control would do. Sharing this with
    /// the panel is what lets it preview the result on hover rather than
    /// guess at it.
    pub(super) fn next_sort(&self, col: Col) -> (Col, bool) {
        match self.sort() {
            Some((c, asc)) if c == col => (col, !asc),
            // bool-ish and recency columns start with the interesting side
            // up: cached/in-library/newest first; text columns start A→Z
            _ => (
                col,
                !matches!(
                    col,
                    Col::Year | Col::Entered | Col::Pdf | Col::InLib | Col::Metric
                ),
            ),
        }
    }

    /// Re-order the active scope's rows to match its sort.
    pub(super) fn apply_sort(&mut self) {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { .. }) => self.sort_ads_at(self.active_scope),
            Some(Scope::Manuscript { .. }) => self.sort_ms_rows(),
            _ => self.rebuild_order(),
        }
    }

    /// Re-order the manuscript rows by the manuscript's own sort. With
    /// none set they keep scan order — the order the cites appear in the
    /// source, which is meaningful and is the default.
    pub(super) fn sort_ms_rows(&mut self) {
        let Some((col, asc)) = self.ms_sort else {
            return;
        };
        // the swatch's value lives in the metrics store, not on the row:
        // snapshot it for the rows on show before they are taken mutably
        let mvals: std::collections::HashMap<String, f64> = if col == Col::Metric {
            match self.scopes.get(1) {
                Some(Scope::Manuscript { rows }) => {
                    let now_ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    rows.iter()
                        .filter_map(|r| r.key.clone())
                        .filter_map(|k| {
                            let m = self.metrics.get(&k)?;
                            let v = match self.metric_col {
                                MetricCol::Priority => m.effective_priority(now_ts)?,
                                MetricCol::Citations => m.citations? as f64,
                            };
                            Some((k, v))
                        })
                        .collect()
                }
                _ => Default::default(),
            }
        } else {
            Default::default()
        };
        let Some(Scope::Manuscript { rows }) = self.scopes.get_mut(1) else {
            return;
        };
        // a paper column reads from the copies the row carries, so a
        // cite resolving to nothing sorts as the empty string it shows
        rows.sort_by(|a, b| {
            let ord = match col {
                Col::Cited => a.cited.to_lowercase().cmp(&b.cited.to_lowercase()),
                // both glyph and word render the same fact, so both sort
                // by it: attention-first, so missing cites come to the top
                Col::CiteIcon | Col::State => ms_state_rank(a).cmp(&ms_state_rank(b)),
                Col::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                Col::Year => a.year.cmp(&b.year),
                Col::Author => crate::library::first_author_last_of(&a.author)
                    .to_lowercase()
                    .cmp(&crate::library::first_author_last_of(&b.author).to_lowercase()),
                Col::Key => a.short_key.cmp(&b.short_key),
                Col::Pdf => a
                    .key
                    .as_deref()
                    .is_some_and(has_cached_pdf)
                    .cmp(&b.key.as_deref().is_some_and(has_cached_pdf)),
                Col::Metric => {
                    let v = |r: &MsRow| {
                        r.key.as_deref().and_then(|k| mvals.get(k)).copied().unwrap_or(-1.0)
                    };
                    v(a).partial_cmp(&v(b)).unwrap_or(std::cmp::Ordering::Equal)
                }
                // the gutter, ● (never drawn here) and Entered (an ADS
                // record's own clock) name nothing a cite row has
                Col::Sel | Col::InLib | Col::Entered => std::cmp::Ordering::Equal,
            };
            let ord = if asc { ord } else { ord.reverse() };
            // ties keep a stable, meaningful order rather than an arbitrary one
            ord.then_with(|| a.cited.cmp(&b.cited))
        });
    }

    /// Draw the active scope's table. Each scope contributes its own
    /// columns and rows; `table::draw` owns the chrome they share.
    /// Keep the cursor on a row that exists.
    ///
    /// A query refresh can hand back fewer records than the tab was
    /// holding — a smaller `+`/`-` limit, or simply a result set that
    /// moved — and nothing else brings the selection back into range, so
    /// the pub card renders blank and every row action has no target.
    /// The library has `refilter` for this; query and manuscript scopes
    /// had nothing, so it lives here, where row_count is authoritative.
    pub(super) fn clamp_cursor(&mut self) {
        let n = self.row_count();
        let sel = match (self.table.selected(), n) {
            (_, 0) => None,
            (Some(p), n) => Some(p.min(n - 1)),
            (None, _) => Some(0),
        };
        self.table.select(sel);
    }
}
