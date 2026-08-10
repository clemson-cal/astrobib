//! What a scope is, and moving between them.
//!
//! The strip along the top is one row per data source: the library,
//! the manuscript when there is one, and a tab per saved ADS query.

use super::*;

/// A data source for the table: the library, or one ADS query's
/// results. One table widget, one interaction path — the scope only
/// decides rows and columns.
pub(super) enum Scope {
    Library,
    /// Cited keys from the manuscript's .tex files, classified.
    Manuscript { rows: Vec<MsRow> },
    Ads {
        tab: crate::tabs::Tab,
        articles: Vec<crate::ads::Article>,
        state: QueryState,
        /// Which set this query is saved in — the global one, or the
        /// active manuscript's. It decides where `save_tabs` files it
        /// and which group it sits in on the strip.
        home: crate::tabs::Home,
        /// The order this scope arrived in, which survives being
        /// regrouped. Session-local and never stored: on disk the order
        /// is the order of each set's array.
        seq: usize,
    },
}

/// Where a query scope is in its round trip to ADS.
///
/// A tab exists from the moment its query is sent, not from the moment
/// the results land: a query that takes a minute used to leave nothing
/// on screen at all, which reads as though nothing was asked. The tab
/// is the acknowledgement, and this is what it has to say meanwhile.
#[derive(Clone, PartialEq)]
pub(super) enum QueryState {
    Pending,
    Ready,
    /// ADS refused or the network did — the text is shown on the page,
    /// where it stays put, rather than only in a log line that scrolls
    /// away while you are reading it.
    Failed(String),
}

impl Scope {
    pub(super) fn kind(&self) -> ScopeKind {
        match self {
            Scope::Library => ScopeKind::Library,
            Scope::Manuscript { .. } => ScopeKind::Manuscript,
            Scope::Ads { .. } => ScopeKind::Query,
        }
    }

    pub(super) fn label(&self) -> &str {
        match self {
            Scope::Library => "Library",
            Scope::Manuscript { .. } => "Manuscript",
            Scope::Ads { tab, .. } => &tab.label,
        }
    }
}

/// Which kind of table a scope presents. Columns are configured per
/// kind, not per scope: every query tab holds the same kind of record,
/// so they all show the same columns, while each keeps its own sort.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ScopeKind {
    Library,
    Manuscript,
    Query,
}

impl ScopeKind {
    pub(super) fn tag(self) -> &'static str {
        match self {
            ScopeKind::Library => "library",
            ScopeKind::Manuscript => "manuscript",
            ScopeKind::Query => "query",
        }
    }
}

/// Sentinel scope-strip index for the active-filter chip ("+ new" is
/// usize::MAX).
pub(super) const FILTER_CHIP: usize = usize::MAX - 1;

/// The mark between the global queries and the active manuscript's own.
/// Not a scope and not clickable — it names a boundary, not a thing.
const GROUP_SEP: usize = usize::MAX - 2;

impl App {
    pub(super) fn active_ads(&self) -> Option<&Scope> {
        match self.scopes.get(self.active_scope) {
            Some(s @ Scope::Ads { .. }) => Some(s),
            _ => None,
        }
    }

    pub(super) fn set_scope(&mut self, idx: usize) {
        self.active_scope = idx.min(self.scopes.len().saturating_sub(1));
        self.table.select(Some(0));
        *self.table.offset_mut() = 0;
        self.pdf_status.clear();
        if self.select_mode {
            self.exit_select_mode();
        }
        // each scope keeps its own sort, so entering one re-asserts it;
        // the library and manuscript are already in their sorted order
        self.sort_ads_at(self.active_scope);
    }

    pub(super) fn cycle_scope(&mut self, d: isize) {
        let n = self.scopes.len() as isize;
        let cur = self.active_scope as isize;
        // The strip ends with the "+ new" affordance, so stepping right
        // off the last scope reaches *that* rather than wrapping round
        // to the library — which is what the strip already shows, and
        // makes ] read as "keep going right" all the way to composing a
        // new query. ] wraps as before: there is nothing beyond Library.
        if d > 0 && cur + d >= n {
            self.open_ads_prompt();
            return;
        }
        self.set_scope(((cur + d).rem_euclid(n)) as usize);
    }

    pub(super) fn close_scope(&mut self) {
        if matches!(
            self.scopes.get(self.active_scope),
            None | Some(Scope::Library) | Some(Scope::Manuscript { .. })
        ) {
            return; // library and manuscript scopes are permanent
        }
        if let Some(Scope::Ads { tab, .. }) = self.scopes.get(self.active_scope) {
            crate::tabs::drop_cached_articles(&tab.id);
        }
        self.scopes.remove(self.active_scope);
        // stay in place: the capsule that was to the right now holds
        // this index (set_scope clamps when we closed the last one)
        self.set_scope(self.active_scope);
        self.save_tabs();
    }

    /// The scope holding a given tab, by the tab's own id.
    ///
    /// Results are routed by identity rather than by the index the query
    /// was launched from: the tab is on screen for the whole round trip
    /// now, so it can be closed, or another opened beside it, while the
    /// query is still in flight — and an index captured a minute ago
    /// would by then address somebody else's scope.
    pub(super) fn scope_of_tab(&self, id: &str) -> Option<usize> {
        self.scopes
            .iter()
            .position(|s| matches!(s, Scope::Ads { tab, .. } if tab.id == id))
    }

    /// Persist the current ADS scopes to the tabs.json state file,
    /// user-local, each query filed under the home it carries.
    pub(super) fn save_tabs(&mut self) {
        let tabs: Vec<(crate::tabs::Tab, crate::tabs::Home)> = self
            .scopes
            .iter()
            .filter_map(|s| match s {
                Scope::Ads { tab, home, .. } => Some((tab.clone(), *home)),
                _ => None,
            })
            .collect();
        let res = crate::tabs::save(&tabs, self.manuscript_root().as_deref());
        self.state_write("saved queries (tabs.json)", res.err().map(|e| e.to_string()));
    }

    /// Move the active query to its other home — global to the
    /// manuscript's own set, or back. One gesture both ways, the same ±
    /// reading `m` and `T` have, because there are exactly two homes and
    /// naming which one you meant would be a question with one answer.
    ///
    /// The move is one write: `save` lays down both sets together, so
    /// the query leaves one key and joins the other atomically. It can
    /// never be in both (a duplicate id, which would strand one of the
    /// two scopes waiting for a result that routes to the other) or in
    /// neither.
    pub(super) fn move_query_home(&mut self) {
        let Some(Scope::Ads { tab, home, .. }) = self.scopes.get_mut(self.active_scope) else {
            return;
        };
        *home = match *home {
            crate::tabs::Home::Global => crate::tabs::Home::Manuscript,
            crate::tabs::Home::Manuscript => crate::tabs::Home::Global,
        };
        let (label, moved_to) = (tab.label.clone(), *home);
        self.regroup_scopes();
        self.save_tabs();
        // kept short: the footer gives this line only the room left in
        // front of the control it is reporting on
        let where_to = match moved_to {
            crate::tabs::Home::Global => "the global queries",
            crate::tabs::Home::Manuscript => "this manuscript",
        };
        self.note(MsgCat::Ok, format!("'{label}' moved to {where_to}"));
    }

    /// Keep the query scopes grouped global-first, following the active
    /// one by its tab id rather than by its index — the whole point of
    /// this is that indices move.
    ///
    /// The strip reads the groups straight off `scopes` order, so a
    /// query that arrives local, or one that has just been moved, has to
    /// be put back among its own kind; otherwise a lone capsule sits
    /// between the global ones and the grouping stops meaning anything.
    /// The sort is stable, so order within each group is untouched.
    pub(super) fn regroup_scopes(&mut self) {
        let active_id = match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { tab, .. }) => Some(tab.id.clone()),
            _ => None,
        };
        // Library and Manuscript hold the front; every query scope is in
        // the contiguous tail behind them
        let Some(first) = self.scopes.iter().position(|s| matches!(s, Scope::Ads { .. })) else {
            return;
        };
        let mut queries = self.scopes.split_off(first);
        // by home, then by the order they arrived in — the second half
        // is what makes moving a query out and back a no-op rather than
        // a quiet trip to the end of the group
        queries.sort_by_key(|s| match s {
            Scope::Ads { home, seq, .. } => {
                (if *home == crate::tabs::Home::Global { 0u8 } else { 1u8 }, *seq)
            }
            _ => (2u8, 0),
        });
        self.scopes.append(&mut queries);
        if let Some(id) = active_id {
            if let Some(i) = self
                .scopes
                .iter()
                .position(|s| matches!(s, Scope::Ads { tab, .. } if tab.id == id))
            {
                self.active_scope = i;
            }
        }
    }

    /// Where a newly minted query is filed. Yours by default: a search
    /// you typed is about your reading, not about whichever paper you
    /// happened to be standing in, and filing it locally is what made
    /// saved queries seem to vanish when you walked away from a `bib/`.
    ///
    /// The exception is a query that names one paper. `citations(…)` and
    /// `references(…)` are made from a card with a single keystroke and
    /// are exhausted the moment you have followed them; left global they
    /// would trail every paper you ever looked at through every
    /// directory. Judged by the query's shape rather than by which
    /// gesture made it, so one typed at the prompt is filed the same way
    /// as one made by C or R. The other operators — `similar`,
    /// `trending`, `useful` — take a query rather than a paper, so they
    /// are ordinary searches and stay global.
    pub(super) fn default_home(&self, query: &str) -> crate::tabs::Home {
        let q = query.trim_start().to_ascii_lowercase();
        let about_one_paper = q.starts_with("citations(") || q.starts_with("references(");
        if about_one_paper && self.manuscript_root().is_some() {
            crate::tabs::Home::Manuscript
        } else {
            crate::tabs::Home::Global
        }
    }

    /// Restore saved query scopes and refresh them all on one worker.
    /// Saved tabs restore with their cached results — instant and
    /// offline-friendly; nothing re-queries ADS until r asks.
    pub(super) fn restore_tabs(&mut self) {
        let saved = crate::tabs::load(self.manuscript_root().as_deref());
        if saved.is_empty() {
            return;
        }
        let mut cached = 0usize;
        // load hands them back global-first, which is the order the
        // strip groups them in — pushing in that order is what keeps the
        // groups contiguous without a sort
        for (t, home) in &saved {
            let articles = crate::tabs::load_cached_articles(&t.id);
            if !articles.is_empty() {
                cached += 1;
            }
            self.scopes.push(Scope::Ads {
                tab: t.clone(),
                articles,
                state: QueryState::Ready,
                home: *home,
                seq: self.scope_seq,
            });
            self.scope_seq += 1;
        }
        // the cache holds whatever order the results last arrived in;
        // each tab's stored sort is what it should come back showing
        for i in 0..self.scopes.len() {
            self.sort_ads_at(i);
        }
        self.note(
            MsgCat::Info,
            format!(
                "restored {} saved quer{} from cache — r refreshes",
                cached,
                if cached == 1 { "y" } else { "ies" }
            ),
        );
    }

    /// +/- — step the active ADS scope's result limit through the
    /// fixed steps (20/50/100/200) and re-run the query.
    pub(super) fn step_limit(&mut self, dir: isize) {
        const STEPS: [usize; 4] = [20, 50, 100, 200];
        let Some(Scope::Ads { tab, .. }) = self.scopes.get_mut(self.active_scope) else {
            return;
        };
        let idx = STEPS.iter().position(|&s| s >= tab.limit).unwrap_or(STEPS.len() - 1);
        let idx = (idx as isize + dir).clamp(0, STEPS.len() as isize - 1) as usize;
        if STEPS[idx] == tab.limit {
            return;
        }
        tab.limit = STEPS[idx];
        let (q, l) = (tab.query.clone(), tab.limit);
        self.note(MsgCat::Info, format!("limit → {l}"));
        self.run_ads_query_limit(q, Some(self.active_scope), l, None);
    }

    pub(super) fn refresh_scope(&mut self) {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { tab, .. }) => {
                self.run_ads_query_limit(tab.query.clone(), Some(self.active_scope), tab.limit, None)
            }
            Some(Scope::Manuscript { .. }) => {
                self.rescan_manuscript();
                self.note(MsgCat::Info, "manuscript rescanned".to_string());
            }
            // library scope: one batched query refreshes the visible
            // entries' citation counts
            _ => self.refresh_citation_counts(),
        }
    }

    /// Whether the active scope is a query (ADS results) page.
    pub(super) fn on_query(&self) -> bool {
        matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. }))
    }

    /// The scope capsules plus the trailing "+ new" chip, in draw order.
    fn scope_strip_items(&self) -> Vec<(String, usize, bool)> {
        let mut v: Vec<(String, usize, bool)> = self
            .scopes
            .iter()
            .enumerate()
            .map(|(i, s)| (s.label().to_string(), i, true))
            .collect();
        // The two homes are told apart by where a capsule sits, so the
        // boundary is marked once instead of every capsule spending
        // width on a badge. Only drawn when both groups have something
        // in them: a mark with nothing on one side of it says nothing.
        let first_local = self
            .scopes
            .iter()
            .position(|s| matches!(s, Scope::Ads { home: crate::tabs::Home::Manuscript, .. }));
        let any_global = self
            .scopes
            .iter()
            .any(|s| matches!(s, Scope::Ads { home: crate::tabs::Home::Global, .. }));
        if let Some(at) = first_local {
            if any_global {
                v.insert(at, ("│".to_string(), GROUP_SEP, false));
            }
        }
        v.push(("+ new".to_string(), usize::MAX, false));
        // an active filter is visible state: it rides the strip as its
        // own chip (click to edit; Esc still clears)
        if !self.filter.value().is_empty() {
            v.push((format!("/ {}", self.filter.value()), FILTER_CHIP, true));
        }
        v
    }

    /// Rows the capsules need at this width (they wrap), plus the blank
    /// row above and below.
    pub(super) fn scope_strip_height(&self, width: u16) -> u16 {
        let mut rows = 1u16;
        let mut x = 0u16;
        for (label, _, _) in self.scope_strip_items() {
            let wl = pill_width(&label);
            if x > 0 && x + wl > width {
                rows += 1;
                x = 0;
            }
            x += wl + 1;
        }
        rows + 2
    }

    /// Scope strip: Library │ query │ query …, clickable, the active
    /// scope bold cyan; wraps onto further rows when the capsules
    /// outgrow the width. [ and ] cycle, ctrl+w closes, r refreshes.
    pub(super) fn draw_scope_strip(&mut self, f: &mut Frame, area: Rect) {
        // font-height capsules, separated from the table by a blank row
        // above and below (glyph-built "taller" capsules read as
        // corruption across fonts)
        let mut y = area.y + 1;
        let mut spans: Vec<Span> = vec![];
        let mut x = area.x;
        for (label, idx, rounded) in self.scope_strip_items() {
            let wl = pill_width(&label);
            if x > area.x && x + wl > area.x + area.width {
                f.render_widget(
                    Paragraph::new(Line::from(std::mem::take(&mut spans))),
                    Rect { x: area.x, y, width: area.width, height: 1 },
                );
                x = area.x;
                y += 1;
            }
            // the group mark is chrome: no capsule, no hit rect, nothing
            // to hover — clicking it would have to mean something
            if idx == GROUP_SEP {
                spans.push(Span::styled(format!(" {label} "), divider()));
                spans.push(Span::raw(" "));
                x += wl + 1;
                continue;
            }
            let r = Rect { x, y, width: wl, height: 1 };
            self.hits.add(r, Target::Scope(idx));
            let hov = hit(r, self.hover.0, self.hover.1);
            if hov && idx == FILTER_CHIP {
                self.hover_hint =
                    Some("⌕ active filter — click to edit  ·  /  (Esc clears)".to_string());
            }
            // composing a *new* query puts you on the "+ new" slot, so the
            // highlight moves there and off the scope you came from —
            // editing an existing one leaves it where it is, since that
            // is the query being changed
            let composing_new = matches!(self.mode, Mode::AdsPrompt { edit: None, .. });
            // The two cyan arms below are deliberately separate. They
            // give the same colour for opposite reasons — one because
            // the "+ new" slot is where composing happens, one because
            // the active scope is where it does not — and folding them
            // into a single `||` would put the two rules where a reader
            // has to take them apart again to change either.
            #[allow(clippy::if_same_then_else)]
            let (bg, fg) = if idx == FILTER_CHIP {
                if hov {
                    (filter_chip_bg_hover(), filter_chip_fg())
                } else {
                    (filter_chip_bg(), filter_chip_fg())
                }
            } else if composing_new && idx == usize::MAX {
                (Color::Cyan, Color::Black)
            } else if idx == self.active_scope && !composing_new {
                (Color::Cyan, Color::Black)
            } else if hov {
                (chip_bg_hover(), chip_fg_strong())
            } else if idx == usize::MAX {
                (chip_bg(), chip_fg_dim())
            } else {
                (chip_bg(), chip_fg())
            };
            if rounded {
                push_pill(&mut spans, &label, bg, fg);
            } else {
                spans.push(Span::styled(
                    format!(" {label} "),
                    Style::default().bg(bg).fg(fg),
                ));
            }
            spans.push(Span::raw(" "));
            x += wl + 1;
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect { x: area.x, y, width: area.width, height: 1 },
        );
    }

    pub(super) fn active_kind(&self) -> ScopeKind {
        self.scopes
            .get(self.active_scope)
            .map(Scope::kind)
            .unwrap_or(ScopeKind::Library)
    }

    /// The home a query is in now, for deciding which side reads active.
    pub(super) fn active_query_home(&self) -> Option<crate::tabs::Home> {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { home, .. }) => Some(*home),
            _ => None,
        }
    }
}
