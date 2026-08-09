//! Entries moving in and out of the two tiers.

use super::*;

/// What removing one paper will actually do. Delete means three
/// different things depending on context, each defensible on its own
/// but unpredictable from the keystroke — so the decision is made once,
/// stated by the confirm modal, and executed from the same value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RemovalKind {
    /// the ordinary case: gone from every tier that holds it
    BothTiers,
    /// global tier hidden: only the local tier's copy goes, and a sole
    /// copy is rescued into the global tier first
    ManuscriptOnly,
    /// a query page, and the active manuscript cites this paper: its
    /// manuscript copy stays, only the global one goes
    GlobalOnly,
}

impl RemovalKind {
    /// The consequence in plain words. `n` is how many targets share
    /// this outcome; `ms` whether a local (tier-2) db exists at all.
    pub(super) fn sentence(self, n: usize, ms: bool) -> String {
        match self {
            RemovalKind::BothTiers if ms => {
                "removes from both tiers — the global library's copy and this manuscript's"
                    .to_string()
            }
            RemovalKind::BothTiers => {
                let f = if n == 1 { "file is" } else { "files are" };
                format!("removes from the library — the .bib {f} deleted")
            }
            RemovalKind::ManuscriptOnly => {
                "removes from this manuscript; sole copies are rescued to the global library"
                    .to_string()
            }
            RemovalKind::GlobalOnly => {
                format!("removes from the global library (kept in the manuscript: {n} cited)")
            }
        }
    }
}

impl App {
    /// The PDF-cache key for a table row, per scope (cite key when the
    /// paper is in the library, bibcode for unimported ADS rows).
    /// The cache key of a row — always a library cite key. An
    /// un-imported query result has none: PDFs are never cached under
    /// a bibcode, so there is nothing to open or store for it.
    pub(super) fn row_cache_key(&self, pos: usize) -> Option<String> {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => articles
                .get(pos)
                .and_then(|a| self.article_entry(a))
                .map(|e| e.key().to_string()),
            Some(Scope::Manuscript { rows }) => rows.get(pos).and_then(|r| r.key.clone()),
            _ => self.filtered.get(pos).and_then(|&i| self.order.get(i).cloned()),
        }
    }

    /// The library entry at an `order` position — the one place that
    /// resolves a display index, so a position that outlives the entry
    /// it named yields None instead of indexing or unwrapping.
    pub(super) fn entry_at(&self, i: usize) -> Option<&crate::library::Entry> {
        self.lib.get(self.order.get(i)?)
    }

    /// i — import the highlighted ADS result into the library (and the
    /// manuscript db when active), via the parity-verified save path.
    pub(super) fn import_highlighted(&mut self) {
        let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) else {
            return;
        };
        // selection imports in bulk; otherwise the highlighted row
        let bibcodes: Vec<String> = if self.select_mode && !self.selected.is_empty() {
            articles
                .iter()
                .filter(|a| self.selected.contains(&a.bibcode))
                .filter(|a| self.article_entry(a).is_none())
                .map(|a| a.bibcode.clone())
                .collect()
        } else {
            match self.table.selected().and_then(|p| articles.get(p)) {
                Some(a) if self.article_entry(a).is_none() => {
                    vec![a.bibcode.clone()]
                }
                Some(a) => {
                    let bc = a.bibcode.clone();
                    self.note(MsgCat::Warn, format!("{bc} already in library"));
                    return;
                }
                None => return,
            }
        };
        if bibcodes.is_empty() {
            self.note(MsgCat::Warn, "nothing to import".to_string());
            return;
        }
        if self.select_mode {
            self.exit_select_mode();
        }
        self.import_bibcodes(bibcodes);
    }

    pub(super) fn import_bibcode(&mut self, bibcode: String) {
        self.import_bibcodes(vec![bibcode]);
    }

    fn import_bibcodes(&mut self, bibcodes: Vec<String>) {
        if self.ads_rx.is_some() {
            self.note(MsgCat::Warn, "an ADS request is already running".to_string());
            return;
        }
        let items: Vec<(u64, String)> = bibcodes
            .into_iter()
            .map(|bc| {
                let id = self.add_task(TaskKind::Import, format!("⤓ import {bc}"), vec![]);
                (id, bc)
            })
            .collect();
        let (tx, rx) = std::sync::mpsc::channel();
        self.ads_rx = Some(rx);
        self.note(MsgCat::Info, format!("Importing {} paper(s)…", items.len()));
        std::thread::spawn(move || {
            for (id, bibcode) in items {
                let result = match crate::ads::fetch_bibtex(&bibcode) {
                    Ok(Some(data)) => Ok(data),
                    Ok(None) => Err("no BibTeX returned".to_string()),
                    Err(e) => Err(e.to_string()),
                };
                let _ = tx.send(AdsMsg::Imported { id, bibcode, result });
            }
        });
    }

    /// t — show/hide the global (tier-1) library. Hidden means: global
    /// entries invisible, imports write only the local tier; the rescue
    /// path still protects sole copies by writing to the global tier.
    pub(super) fn toggle_global(&mut self) {
        self.lib.global_on = !self.lib.global_on;
        if self.select_mode {
            self.exit_select_mode();
        }
        self.rebuild_order();
        self.note(
            MsgCat::Info,
            if self.lib.global_on {
                format!("global tier shown — {} papers merged", self.order.len())
            } else {
                format!("global tier hidden — {} local papers", self.order.len())
            },
        );
    }

    pub(super) fn open_export_prompt(&mut self) {
        let keys = self.action_keys();
        if keys.is_empty() {
            let msg = if self.on_query() {
                "import the paper first (i) — export writes library entries"
            } else {
                "nothing to export — Space selects"
            };
            self.note(MsgCat::Warn, msg.to_string());
            return;
        }
        self.mode = Mode::Export { input: tui_input::Input::from("refs.bib".to_string()), keys };
    }

    /// Write the export: entries under their own keys, refs.bib style.
    /// Intermediate directories are never created; a bad path fails.
    pub(super) fn do_export(&mut self, path: &str, keys: &[String]) {
        let path = std::path::PathBuf::from(crate::library::shellexpand_home(path));
        let blocks: Vec<String> = keys
            .iter()
            .filter_map(|k| self.lib.get(k))
            .map(|e| crate::bib::format_entry(&e.data))
            .collect();
        match std::fs::write(&path, blocks.join("\n")) {
            Ok(()) => self.note(
                MsgCat::Ok,
                format!("Exported {} entr{} → {}", blocks.len(),
                    if blocks.len() == 1 { "y" } else { "ies" }, path.display()),
            ),
            Err(e) => self.note(MsgCat::Err, format!("could not write {}: {e}", path.display())),
        }
    }

    /// The cite key an article WOULD get on import — computable locally
    /// because keys derive from stable identity (arXiv ID / bibcode,
    /// first-author surname, identity year), all present in the article.
    /// The library entry a query result refers to, matched by paper
    /// identity rather than by bibcode.
    ///
    /// A paper imported as a preprint carries the arXiv bibcode, while
    /// a later search returns the published one — comparing bibcodes
    /// calls those two different papers, so an imported preprint shows
    /// as un-imported the moment it is published. Cite keys derive from
    /// the stable identifier (arXiv ID before bibcode), so the key is
    /// the same on both sides of that transition.
    pub(super) fn article_entry(&self, a: &crate::ads::Article) -> Option<&crate::library::Entry> {
        self.lib.get(&self.hypothetical_key(a))
    }

    pub(super) fn hypothetical_key(&self, a: &crate::ads::Article) -> String {
        if let Some(e) = self.lib.get_by_bibcode(&a.bibcode) {
            return e.key().to_string();
        }
        let mut d = crate::bib::Data::new();
        d.insert("author".into(), a.author.join(" and "));
        d.insert("year".into(), a.year.clone());
        if let Some(ep) = crate::ads::arxiv_id(a) {
            d.insert("eprint".into(), ep.to_string());
            d.insert("archiveprefix".into(), "arXiv".into());
        }
        d.insert(
            "adsurl".into(),
            format!("https://ui.adsabs.harvard.edu/abs/{}", a.bibcode),
        );
        crate::keys::generate_key(&d)
    }

    /// Delete — ask for confirmation before removing. The targets are
    /// the usual ones (selection, else the shown row — on a query page
    /// their imported twins).
    pub(super) fn remove_papers(&mut self) {
        let plan = self.removal_plan(self.action_keys());
        if !plan.is_empty() {
            self.mode = Mode::Confirm { plan };
        }
    }

    /// Decide, once, what removing each target will do — the single
    /// source of truth for Delete. The modal renders this and
    /// remove_confirmed consumes it, so the words and the deed cannot
    /// drift apart (and the manuscript scan behind `cited` runs once,
    /// not once per frame the modal is up).
    fn removal_plan(&self, keys: Vec<String>) -> Vec<(String, RemovalKind)> {
        let local_only = self.lib.manuscript.is_some() && !self.lib.global_on;
        // query pages: a paper the active manuscript cites keeps its
        // tier-2 copy — only the global (tier-1) copy is removed. That
        // only means anything when a tier-2 copy actually exists;
        // otherwise removal is the ordinary kind (removing from a tier
        // that does not hold the paper is a no-op either way).
        let cited = if self.on_query() { self.cited_keys() } else { Default::default() };
        keys.into_iter()
            .map(|k| {
                let kind = if cited.contains(&k) && self.lib.in_manuscript(&k) {
                    RemovalKind::GlobalOnly
                } else if local_only {
                    RemovalKind::ManuscriptOnly
                } else {
                    RemovalKind::BothTiers
                };
                (k, kind)
            })
            .collect()
    }

    /// Keys cited by the active manuscript's sources (not merely db
    /// members): removing these keeps the tier-2 copy the paper needs.
    fn cited_keys(&self) -> std::collections::HashSet<String> {
        if self.lib.manuscript.is_none() {
            return Default::default();
        }
        self.ms_rows()
            .into_iter()
            .filter(|r| !r.uncited)
            .filter_map(|r| r.key)
            .collect()
    }

    /// Execute the plan the confirm modal stated — nothing is decided
    /// here, so what happens is what the user was told would happen.
    pub(super) fn remove_confirmed(&mut self, plan: &[(String, RemovalKind)]) {
        let mut n = 0;
        let mut kept = 0;
        for (k, kind) in plan {
            let ok = match kind {
                RemovalKind::GlobalOnly => {
                    kept += 1;
                    self.lib.personal.remove_entry(k).is_ok()
                }
                RemovalKind::ManuscriptOnly => {
                    matches!(self.lib.remove_from_manuscript(k), Ok(true))
                }
                RemovalKind::BothTiers => self.lib.remove_entry(k).is_ok(),
            };
            if ok {
                n += 1;
            }
        }
        if kept > 0 {
            self.note(
                MsgCat::Info,
                format!("{kept} cited paper(s) kept their manuscript copy"),
            );
        }
        if self.select_mode {
            self.select_mode = false;
            self.selected.clear();
        }
        self.rebuild_order();
        self.note(MsgCat::Ok, format!("Removed {n} paper(s)"));
    }

    pub(super) fn import_picked(&mut self, key: &str, file: &std::path::Path) {
        match pdf::import_file(key, file) {
            Some(dest) => {
                let kb = dest.metadata().map(|m| m.len() / 1024).unwrap_or(0);
                let msg = format!(
                    "Imported {} for {key}  ({kb} KB)",
                    file.file_name().unwrap_or_default().to_string_lossy()
                );
                self.note(MsgCat::Ok, msg);
            }
            None => {
                let msg = format!(
                    "{} does not look like a PDF",
                    file.file_name().unwrap_or_default().to_string_lossy()
                );
                self.note(MsgCat::Err, msg);
            }
        }
    }

    /// The library: a subtle per-column palette, with the terminal theme
    /// supplying the hues. The cursor row takes a faint cool fill and a
    /// ◉; a hovered row takes no fill — its text lifts one level instead.
    pub(super) fn library_model(&self, width: u16) -> table::TableModel {
        use ratatui::widgets::Cell;
        let palette = |lit: bool| {
            if lit {
                (
                    Style::default().fg(Color::Green),
                    Style::default().fg(Color::Magenta),
                    Style::default().fg(Color::Green),
                    Style::default().fg(text_strong()),
                    Style::default().fg(Color::Cyan),
                )
            } else {
                (
                    Style::default().fg(Color::Green),
                    Style::default().fg(Color::Magenta),
                    Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
                    Style::default().fg(table_text()),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                )
            }
        };
        let columns = self.columns_for(ScopeKind::Library, width);
        let author_w = col_width(&columns, width, Col::Author);
        let (mvals, mknown) = self.metric_values();
        let hov_row = self.hovered_table_pos();
        let cursor = self.table.selected();
        let show_membership = self.lib.manuscript.is_some() && self.lib.global_on;
        let rows: Vec<Row<'static>> = self
            .filtered
            .iter()
            .enumerate()
            .map(|(pos, &i)| {
                // refilter only admits keys the library holds, so this
                // resolves; if the two ever disagree the row renders as
                // an orphan rather than panicking mid-frame — and every
                // later position stays aligned with `filtered`
                let Some(e) = self.entry_at(i) else {
                    return Row::new(vec![Cell::from(Span::styled(
                        format!(
                            "· {} (no longer in the library)",
                            self.order.get(i).map(String::as_str).unwrap_or("?")
                        ),
                        Style::default().fg(Color::DarkGray),
                    ))]);
                };
                let at_cursor = cursor == Some(pos);
                let lit = hov_row == Some(pos);
                let (c_pdf, c_ms, c_year, c_author, c_key) = palette(lit);
                let (circle, circle_style) = self.gutter(Some(e.key()), at_cursor);
                let cells: Vec<Cell> = columns
                    .iter()
                    .map(|c| match c.id {
                        Col::Sel => Cell::from(Span::styled(circle, circle_style)),
                        Col::Pdf => Cell::from(Span::styled(
                            if has_cached_pdf(e.key()) { "↓" } else { "" },
                            c_pdf,
                        )),
                        Col::InLib => Cell::from(Span::styled(
                            if show_membership && self.lib.in_manuscript(e.key()) {
                                "●"
                            } else {
                                ""
                            },
                            c_ms,
                        )),
                        Col::Metric => {
                            metric_cell(self.metric_col, mvals.get(pos).copied().flatten(), &mknown)
                        }
                        Col::Year => Cell::from(Span::styled(e.year(), c_year)),
                        Col::Author => Cell::from(Span::styled(
                            fit_authors(e.author(), author_w as usize),
                            c_author,
                        )),
                        Col::Title => Cell::from(Span::styled(
                            e.title().trim_matches(['{', '}']).to_string(),
                            if lit {
                                Style::default().fg(text_strong()).add_modifier(Modifier::ITALIC)
                            } else {
                                Style::default().fg(table_text()).add_modifier(Modifier::ITALIC)
                            },
                        )),
                        Col::Key => Cell::from(Span::styled(e.short_key.clone(), c_key)),
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
        table::TableModel { columns, rows, sort: self.sort(), hover: self.hover }
    }
}
