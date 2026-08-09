//! The manuscript view: what the sources cite, and refs.bib.

use super::*;

/// One manuscript-view row: a cited string (or an uncited db member).
pub(super) struct MsRow {
    pub(super) cited: String,
    pub(super) state: crate::library::CiteState,
    pub(super) uncited: bool,
    pub(super) key: Option<String>,
    pub(super) title: String,
}

/// Cite states in the order the manuscript view groups them when sorted
/// by state: what needs attention first.
pub(super) fn ms_state_rank(r: &MsRow) -> u8 {
    use crate::library::CiteState;
    if r.uncited {
        return 4;
    }
    match r.state {
        CiteState::Missing => 0,
        CiteState::Ambiguous => 1,
        CiteState::Library => 2,
        CiteState::Ok => 3,
    }
}

impl App {
    /// The manuscript view's row set: scan .tex and .md sources and
    /// classify every cited key, then append uncited manuscript-db
    /// members. Markdown wikilinks count only when they resolve (an
    /// unresolved [[link]] is an ordinary note link, not a citation);
    /// pandoc @cites always count and surface as missing.
    pub(super) fn ms_rows(&self) -> Vec<MsRow> {
        use crate::library::CiteState;
        let Some(root) = self.ms_root() else { return vec![] };
        let files = crate::export::manuscript_tex_files(&root);
        let mut cited = crate::export::scan_tex_files(&files);
        let md_files = crate::export::manuscript_md_files(&root);
        let mut seen: std::collections::HashSet<String> = cited.iter().cloned().collect();
        for c in crate::export::scan_md_files(&md_files) {
            if (!c.wikilink || self.lib.resolve_citation(&c.raw).1.is_some())
                && seen.insert(c.raw.clone())
            {
                cited.push(c.raw);
            }
        }
        let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut rows: Vec<MsRow> = vec![];
        for c in cited {
            let (state, entry) = self.lib.resolve_citation(&c);
            if let Some(e) = entry {
                covered.insert(e.key().to_string());
            }
            rows.push(MsRow {
                cited: c,
                state,
                uncited: false,
                key: entry.map(|e| e.key().to_string()),
                title: entry
                    .map(|e| e.title().trim_matches(['{', '}']).to_string())
                    .unwrap_or_default(),
            });
        }
        if let Some(ms) = &self.lib.manuscript {
            for e in ms.entries() {
                if !covered.contains(e.key()) {
                    rows.push(MsRow {
                        cited: e.short_key.clone(),
                        state: CiteState::Ok,
                        uncited: true,
                        key: Some(e.key().to_string()),
                        title: e.title().trim_matches(['{', '}']).to_string(),
                    });
                }
            }
        }
        rows
    }

    pub(super) fn rescan_manuscript(&mut self) {
        if self.lib.manuscript.is_none() {
            return;
        }
        let rows = self.ms_rows();
        self.sync_refs_bib(&rows);
        match self.scopes.get_mut(1) {
            Some(s @ Scope::Manuscript { .. }) => *s = Scope::Manuscript { rows },
            _ => self.scopes.insert(1, Scope::Manuscript { rows }),
        }
        self.watch = self.watch_snapshot();
    }

    /// Keep refs.bib beside the manuscript in step with its citations,
    /// regenerating silently on every rescan. Created only for TeX
    /// manuscripts (astrobib refs opts a markdown one in); once the
    /// file exists it is kept fresh whatever the sources.
    pub(super) fn sync_refs_bib(&mut self, rows: &[MsRow]) {
        let Some(root) = self.ms_root() else { return };
        let out = root.join("refs.bib");
        if !out.exists() && crate::export::manuscript_tex_files(&root).is_empty() {
            return;
        }
        let cited: Vec<String> = rows
            .iter()
            .filter(|r| !r.uncited)
            .map(|r| r.cited.clone())
            .collect();
        let content = crate::export::refs_bib_content(&cited, &self.lib);
        let res = crate::export::write_refs_bib(&out, &content);
        self.state_write("refs.bib", res.err().map(|e| e.to_string()));
    }

    pub(super) fn ms_root(&self) -> Option<std::path::PathBuf> {
        self.lib.manuscript.as_ref().map(|m| m.root.clone())
    }

    /// The library entries an action applies to: the selection (in
    /// display order) when selection mode is active and non-empty, else
    /// the highlighted row — one convention for every bulk-capable
    /// action, in every scope.
    ///
    /// A query page lists ADS records, not library entries, so each row
    /// resolves through its imported twin (`get_by_bibcode`): actions
    /// there act on the paper the user can see is already in the
    /// library, and an un-imported row yields no key at all, so the
    /// actions that need one dim and say why instead of doing nothing.
    /// The selected manuscript rows' keys, in row order.
    pub(super) fn selected_ms_keys(&self) -> Vec<String> {
        let Some(Scope::Manuscript { rows }) = self.scopes.get(self.active_scope) else {
            return vec![];
        };
        rows.iter()
            .filter_map(|r| r.key.as_ref())
            .filter(|k| self.selected.contains(*k))
            .cloned()
            .collect()
    }

    /// m — the library-view manuscript toggle rule: if any target is
    /// missing from the manuscript db, add all missing; else (all
    /// present) remove all.
    pub(super) fn toggle_manuscript(&mut self) {
        if self.lib.manuscript.is_none() {
            self.note(MsgCat::Warn, "no manuscript db (run inside a manuscript repo)".to_string());
            return;
        }
        let keys = self.action_keys();
        if keys.is_empty() {
            let msg = if self.on_query() {
                "import the paper first (i) — the manuscript db holds library entries"
            } else {
                "no paper under the cursor"
            };
            self.note(MsgCat::Warn, msg.to_string());
            return;
        }
        let missing: Vec<String> = keys
            .iter()
            .filter(|k| !self.lib.in_manuscript(k))
            .cloned()
            .collect();
        if !missing.is_empty() {
            let mut n = 0;
            for k in &missing {
                if matches!(self.lib.add_to_manuscript(k), Ok(true)) {
                    // entering the manuscript signals top priority
                    self.metrics.set_priority(k, 1.0);
                    n += 1;
                }
            }
            self.note(MsgCat::Ok, format!("◆ Added {n} paper(s) to manuscript db"));
        } else {
            let mut n = 0;
            let mut rescued = 0;
            for k in &keys {
                if !self.lib.in_personal(k) {
                    rescued += 1;
                }
                if matches!(self.lib.remove_from_manuscript(k), Ok(true)) {
                    n += 1;
                }
            }
            let note = if rescued > 0 {
                format!("  ({rescued} copied to personal library)")
            } else {
                String::new()
            };
            self.note(MsgCat::Ok, format!("Removed {n} paper(s) from manuscript db{note}"));
        }
        self.rebuild_order();
    }

    /// Manuscript: cite string, resolution state, resolved title. Rows
    /// arrive in scan order — the order the cites appear in the source —
    /// and stay there until a header is clicked; sorting by State is the
    /// useful one, since it gathers the missing cites at the top.
    pub(super) fn manuscript_model(&self, rows: &[MsRow]) -> table::TableModel {
        use crate::library::CiteState;
        use ratatui::widgets::Cell;
        let columns = self.columns_for(ScopeKind::Manuscript, 0);
        let cursor = self.table.selected();
        let hov_row = self.hovered_table_pos();
        let trows: Vec<Row<'static>> = rows
            .iter()
            .enumerate()
            .map(|(pos, r)| {
                let lit = hov_row == Some(pos);
                let (icon, word, style) = match (r.uncited, r.state) {
                    (true, _) => ("·", "uncited", Style::default().fg(Color::DarkGray)),
                    (_, CiteState::Ok) => ("●", "ok", Style::default().fg(Color::Green)),
                    (_, CiteState::Library) => {
                        ("○", "library", Style::default().fg(Color::Yellow))
                    }
                    (_, CiteState::Ambiguous) => {
                        ("?", "ambiguous", Style::default().fg(Color::Magenta))
                    }
                    (_, CiteState::Missing) => ("✗", "missing", Style::default().fg(Color::Red)),
                };
                let cite_style = if lit {
                    Style::default().fg(text_strong())
                } else {
                    Style::default().fg(Color::Cyan)
                };
                let title_style = if lit {
                    Style::default().fg(text_strong()).add_modifier(Modifier::ITALIC)
                } else {
                    Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC)
                };
                let at_cursor = cursor == Some(pos);
                let (circle, circle_style) = self.gutter(r.key.as_deref(), at_cursor);
                let cells: Vec<Cell> = columns
                    .iter()
                    .map(|c| match c.id {
                        Col::Sel => Cell::from(Span::styled(circle, circle_style)),
                        Col::CiteIcon => Cell::from(Span::styled(icon, style)),
                        Col::Cited => Cell::from(Span::styled(r.cited.clone(), cite_style)),
                        Col::State => Cell::from(Span::styled(word, style)),
                        Col::Title => Cell::from(Span::styled(r.title.clone(), title_style)),
                        _ => Cell::from(""),
                    })
                    .collect();
                let row = Row::new(cells);
                if at_cursor {
                    row.style(Style::default().bg(cursor_fill()))
                } else {
                    row
                }
            })
            .collect();
        table::TableModel { columns, rows: trows, sort: self.sort(), hover: self.hover }
    }
}
