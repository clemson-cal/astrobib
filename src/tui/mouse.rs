//! Clicks, hovers and the wheel, resolved against the drawn rects.

use super::*;

impl App {
    /// The table position under the mouse, if any (header and rule
    /// rows excluded).
    pub(super) fn hovered_table_pos(&self) -> Option<usize> {
        let a = self.last_table_area;
        if !hit(a, self.hover.0, self.hover.1) || self.hover.1 <= a.y + 1 {
            return None;
        }
        let pos = self.table.offset() + (self.hover.1 - a.y - 2) as usize;
        (pos < self.row_count()).then_some(pos)
    }

    pub(super) fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollDown => {
                if self.scroll_swatch(m.column, m.row, 0.8) {
                } else if self.show_detail
                    && matches!(self.hits.at(m.column, m.row), Some(Target::Card))
                {
                    self.card_scroll = self.card_scroll.saturating_add(3);
                } else {
                    self.move_sel(3);
                }
            }
            MouseEventKind::ScrollUp => {
                if self.scroll_swatch(m.column, m.row, 1.25) {
                } else if self.show_detail
                    && matches!(self.hits.at(m.column, m.row), Some(Target::Card))
                {
                    self.card_scroll = self.card_scroll.saturating_sub(3);
                } else {
                    self.move_sel(-3);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.on_click(m.column, m.row, m.modifiers)
            }
            MouseEventKind::Moved => self.hover = (m.column, m.row),
            _ => {}
        }
    }

    /// The wheel over a priority swatch scales THAT row's priority —
    /// the mouse-native form of < / >. Returns whether it acted.
    fn scroll_swatch(&mut self, x: u16, y: u16, factor: f64) -> bool {
        let Some(metric_area) = self
            .hits
            .0
            .iter()
            .rev()
            .find(|(_, t)| matches!(t, Target::Metric))
            .map(|(r, _)| *r)
        else {
            return false;
        };
        if self.metric_col != MetricCol::Priority || !hit(metric_area, x, y) {
            return false;
        }
        // the strip's rows start two lines below its top (header + rule)
        if y < metric_area.y + 2 {
            return false;
        }
        let pos = self.table.offset() + (y - metric_area.y - 2) as usize;
        let Some(key) = self.row_key_at(pos) else { return false };
        let level = self.metrics.scale_priority(&key, factor);
        self.note(MsgCat::Ok, format!("priority {level:.2} — {key}"));
        true
    }

    /// The library key of a row position in the active scope (query
    /// scopes resolve through the imported twin).
    fn row_key_at(&self, pos: usize) -> Option<String> {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => articles
                .get(pos)
                .and_then(|a| self.article_entry(a))
                .map(|e| e.key().to_string()),
            Some(Scope::Manuscript { rows }) => rows.get(pos).and_then(|r| r.key.clone()),
            _ => self.filtered.get(pos).and_then(|&i| self.order.get(i).cloned()),
        }
    }

    fn on_click(&mut self, x: u16, y: u16, mods: KeyModifiers) {
        let target = self.hits.at(x, y).cloned();
        // modal picker swallows all clicks: row click imports, outside closes
        if let Mode::Pick { key, files, .. } = &self.mode {
            if let Some(Target::PickRow(i)) = target.clone() {
                if i < files.len() {
                    let (key, file) = (key.clone(), files[i].clone());
                    self.mode = Mode::Normal;
                    self.import_picked(&key, &file);
                    return;
                }
            }
            self.mode = Mode::Normal;
            return;
        }
        if self.show_about {
            if let Some(Target::AboutLink(url)) = target.clone() {
                pdf::browser_open(&url);
                self.note(MsgCat::Info, "opened in browser".to_string());
            } else if matches!(target.as_ref(), Some(Target::AboutUpdate)) {
                self.check_updates();
            } else {
                self.show_about = false;
            }
            return;
        }
        // the prompt's ADS-returns glyph, which must be tested before the
        // click-away dismissal below or it would close the prompt instead
        if matches!(self.mode, Mode::AdsPrompt { .. })
            && matches!(target.as_ref(), Some(Target::PromptSort))
        {
            if self.sort_menu {
                self.sort_menu = false;
            } else {
                self.open_sort_menu();
            }
            return;
        }
        // a menu entry, before the click-away dismissal for the same
        // reason the samples are: reaching that would close the prompt
        if self.sort_menu && matches!(self.mode, Mode::AdsPrompt { .. }) {
            if let Some(Target::SortMenu(value)) = target.clone() {
                // a click is one gesture, so it chooses and leaves —
                // unlike the arrows, which are a walk through the list
                self.sort_menu_sel = ADS_SORTS
                    .iter()
                    .position(|(f, ..)| value.starts_with(f))
                    .unwrap_or(self.sort_menu_sel);
                self.apply_sort_menu();
                self.sort_menu = false;
                return;
            }
        }
        // a sample row, which must be tested before the click-away
        // dismissal below — reaching that would close the very prompt
        // the sample is meant to fill. Consumed either way: a row that
        // cannot act must not fall through and close the prompt instead.
        if let Some(Target::Sample(sample)) = target.clone() {
            if self.prompt_is_empty() {
                self.use_sample(sample);
            }
            return;
        }
        // clicking away from the query prompt dismisses it, and the click
        // then performs its normal action (e.g. switching scope); the
        // filter likewise leaves entry mode, but stays applied (as ⏎) —
        // clicking a row of the filtered results must not wipe them
        if matches!(
            self.mode,
            Mode::AdsPrompt { .. }
                | Mode::Filter
                | Mode::Setup { .. }
                | Mode::Export { .. }
                | Mode::Rename { .. }
                | Mode::Tag { .. }
        ) {
            self.mode = Mode::Normal;
        }
        // confirm modal: only its two buttons act; other clicks are inert
        if let Mode::Confirm { plan } = &self.mode {
            if let Some(Target::Confirm(is_confirm)) = target.clone() {
                let plan = plan.clone();
                self.mode = Mode::Normal;
                if is_confirm {
                    self.remove_confirmed(&plan);
                } else {
                    self.note(MsgCat::Warn, "removal cancelled".to_string());
                }
            }
            return;
        }
        // copy-regions: the card text copies its own entry's datum; in
        // ADS scopes values come from the shown article itself
        if let Some(Target::CardYank(item)) = target.clone() {
            // several rows selected: the row copies across the selection
            if self.select_mode && self.selected.len() > 1 {
                self.do_copy(item);
                return;
            }
            if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
                match self.article_copy_value(item) {
                    Some(text) => self.finish_copy(&text),
                    None => self.note(MsgCat::Warn, "nothing to copy".to_string()),
                }
                return;
            }
            if let Some(key) = self.selected_key().map(str::to_string) {
                self.do_copy_single(key, item);
            }
            return;
        }
        // pub card links open the browser
        if let Some(Target::CardLink(url)) = target.clone() {
            pdf::browser_open(&url);
            self.note(MsgCat::Info, "opened in browser".to_string());
            return;
        }
        // keys-panel rows act as their key
        if let Some(Target::Help(code)) = target.clone() {
            self.on_key(code, KeyModifiers::NONE);
            return;
        }
        // a click leaves an active y-chord and then acts normally —
        // the card's ⧉ rows are the visible copy menu
        if matches!(self.mode, Mode::Copy) {
            self.exit_copy_mode();
        }
        // a tag on the card filters the scope to itself
        if let Some(Target::CardTag(name)) = target.clone() {
            self.filter_by_tag(&name);
            return;
        }
        // pub card buttons (act on the card's entry)
        if let Some(Target::CardButton(btn)) = target.clone() {
            if btn == CardBtn::RemoveFromLib {
                self.remove_papers();
                return;
            }
            if btn == CardBtn::RefreshCites {
                match self.card_entry_key() {
                    Some(key) => self.refresh_citation_count_for(&key),
                    None => self.note(
                        MsgCat::Warn,
                        "import the paper first to track its citation count".to_string(),
                    ),
                }
                return;
            }
            if btn == CardBtn::BibView {
                self.show_bib_source = !self.show_bib_source;
                return;
            }
            if btn == CardBtn::Import {
                if let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) {
                    if let Some(a) = self.card_article_pos().and_then(|p| articles.get(p)) {
                        let bc = a.bibcode.clone();
                        self.import_bibcode(bc);
                    }
                }
                return;
            }
            // citation-graph affordances spawn a new ADS query scope for
            // the card's bibcode — same path as the S prompt (the scope
            // becomes active and its tab persists via save_tabs)
            if matches!(btn, CardBtn::Citations | CardBtn::Refs) {
                self.spawn_citation_query(btn == CardBtn::Refs);
                return;
            }
            if let Some(key) = self.card_entry_key() {
                match btn {
                    CardBtn::Arxiv => self.download_single(key, pdf::Source::Arxiv),
                    CardBtn::Oa => self.download_single(key, pdf::Source::Oa),
                    CardBtn::Browser => self.browser_download_for(key),
                    CardBtn::Pick => self.open_picker_for(key),
                    CardBtn::Open => {
                        pdf::open_paths(&[pdf::cache_path(&key)]);
                        self.note(MsgCat::Ok, format!("Opened {key}"));
                    }
                    CardBtn::Clear | CardBtn::Cancel => self.clear_card_pdf(&key),
                    // handled above (Import for ADS scopes; the
                    // citation-graph pair for every scope)
                    CardBtn::Import
                    | CardBtn::Citations
                    | CardBtn::Refs
                    | CardBtn::BibView
                    | CardBtn::RefreshCites
                    | CardBtn::RemoveFromLib => {}
                    CardBtn::MsToggle => {
                        let res = if self.lib.in_manuscript(&key) {
                            let rescued = !self.lib.in_personal(&key);
                            match self.lib.remove_from_manuscript(&key) {
                                Ok(true) => Some(format!(
                                    "Removed {key} from manuscript db{}",
                                    if rescued { "  (copied to personal library)" } else { "" }
                                )),
                                _ => None,
                            }
                        } else {
                            match self.lib.add_to_manuscript(&key) {
                                Ok(true) => {
                                    self.metrics.set_priority(&key, 1.0);
                                    Some(format!("◆ Added {key} to manuscript db"))
                                }
                                _ => None,
                            }
                        };
                        if let Some(msg) = res {
                            self.note(MsgCat::Ok, msg);
                            self.rebuild_order();
                        }
                    }
                }
            }
            return;
        }
        // scope strip (usize::MAX = the new-query affordance)
        if let Some(Target::Scope(idx)) = target.clone() {
            if idx == FILTER_CHIP {
                self.run_action(Action::Filter);
            } else if idx == usize::MAX {
                self.open_ads_prompt();
            } else {
                self.set_scope(idx);
            }
            return;
        }
        if matches!(target.as_ref(), Some(Target::EditQuery)) {
            if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
                self.open_edit_query_prompt();
            } else {
                self.open_ads_prompt();
            }
            return;
        }
        // footer view badges
        if let Some(Target::Footer(action)) = target.clone() {
            self.run_action(action);
            return;
        }
        // the columns panel: clicking anywhere in it takes focus and
        // selects the row; a control inside the row also acts. The
        // specific hits are searched first because each shares its row's
        // rect, and Row alone would swallow every one of them.
        if matches!(target.as_ref(), Some(Target::Column(_))) {
            self.focus = Focus::Columns;
            if let Some(Target::Column(action)) = target.clone() {
                if let PanelHit::Row(i) = action {
                    self.col_sel = i;
                }
                match action {
                    PanelHit::Toggle(id) => self.toggle_column(id),
                    PanelHit::Sort(id) => self.sort_by(id),
                    PanelHit::Narrower(_) => self.nudge_width(-1),
                    PanelHit::Wider(_) => self.nudge_width(1),
                    PanelHit::Row(_) => {}
                }
            }
            return;
        }
        // a click anywhere else is the table's, and takes focus with it
        self.focus = Focus::Table;
        // column headers sort
        if let Some(Target::SortHeader(col)) = target.clone() {
            self.sort_by(col);
            return;
        }
        // table: header at a.y, rule below it, data rows after
        let a = self.last_table_area;
        if !hit(a, x, y) || y <= a.y + 1 {
            return;
        }
        let pos = self.table.offset() + (y - a.y - 2) as usize;
        if pos >= self.row_count() {
            return;
        }
        self.table.select(Some(pos));
        // option/ctrl+click anywhere on a row enters multi-select and
        // toggles it. The SGR mouse protocol carries only shift/alt/ctrl
        // bits — there is no cmd bit, and macOS terminals keep cmd+click
        // for themselves (links) — so cmd cannot arrive; SUPER stays in
        // the mask in case a terminal ever forwards it that way.
        let modified = mods.intersects(
            KeyModifiers::SUPER | KeyModifiers::ALT | KeyModifiers::CONTROL,
        );
        if modified || x < a.x + 2 {
            self.select_mode = true;
            self.toggle_row_selected(pos);
            self.last_click = None;
            return;
        }
        // plain body click: arm/complete a double-click → open cached PDF
        let now = std::time::Instant::now();
        let is_double = matches!(
            self.last_click,
            Some((t, s, p)) if s == self.active_scope && p == pos
                && now.duration_since(t) < std::time::Duration::from_millis(400)
        );
        if is_double {
            self.last_click = None;
            if let Some(key) = self.row_cache_key(pos) {
                if pdf::is_cached(&key) {
                    pdf::open_paths(&[pdf::cache_path(&key)]);
                    self.note(MsgCat::Ok, format!("Opened {key}"));
                } else {
                    self.note(MsgCat::Warn, "no cached PDF — p downloads".to_string());
                }
            }
        } else {
            self.last_click = Some((now, self.active_scope, pos));
        }
    }
}
