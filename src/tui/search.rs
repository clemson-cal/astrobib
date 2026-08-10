//! Composing an ADS query, sending it, and taking delivery.
//!
//! Also the two affordances that hang off the prompt — the samples
//! offered to an empty one, and the menu of what ADS returns — because
//! both are part of configuring the query rather than of the footer
//! that happens to host them.

use super::*;

pub(super) enum AdsMsg {
    Done {
        id: u64,
        tab: crate::tabs::Tab,
        result: Result<Vec<crate::ads::Article>, String>,
    },
    Imported {
        id: u64,
        bibcode: String,
        result: Result<crate::bib::Data, String>,
    },
}

/// The ADS `sort` parameters a query tab can select records with, in the
/// order the panel offers them. The first is the default and the reason
/// the feature exists: a query tab that reads as a feed of postings.
/// `(name, the ADS sort parameter)`. Named in full in the prompt rather
/// than reduced to a glyph: a symbol you have to have learned is weak
/// feedback for a mode, and the prompt has room to say it.
/// Everything ADS will sort by, which is what decides *which* records
/// come back rather than how they are arranged: paired with `rows`, this
/// selects the top n, so changing it changes the papers, not the order.
///
/// `(menu key, field, primary name, reverse name)`. The primary
/// direction is whichever is normally wanted — newest, most cited, A→Z —
/// and shift on the menu key takes the other one.
///
/// ADS's own dropdown also offers Title. It is left out because it does
/// not work: `title asc`, `title desc` and `score desc` return identical
/// results, so it would be a mode that quietly does nothing. Nothing here
/// is guessed — each was run against the live API, which matters because
/// ADS *silently drops* a sort field it does not know (it answers 200
/// with `"sort": ""` and default ordering), so a wrong field name would
/// leave the app naming an order it is not getting.
pub(super) const ADS_SORTS: [(&str, bool, &str, &str); 10] = [
    ("entry_date", true, "newest posting", "oldest posting"),
    ("date", true, "newest published", "oldest published"),
    ("citation_count", true, "most cited", "least cited"),
    ("citation_count_norm", true, "most cited (normalized)", "least cited (normalized)"),
    ("classic_factor", true, "highest classic factor", "lowest classic factor"),
    ("read_count", true, "most read", "least read"),
    ("author_count", true, "most authors", "fewest authors"),
    // names read forwards: the useful direction here is ascending, so
    // that is the one the list offers first
    ("first_author", false, "first author A→Z", "first author Z→A"),
    ("bibcode", false, "bibcode A→Z", "bibcode Z→A"),
    ("score", true, "most relevant", "least relevant"),
];

/// The `sort` parameter for a field in its primary or reverse direction.
/// Which way "primary" points is the field's own business — newest for a
/// date, most for a count, A→Z for a name.
pub(super) fn ads_sort_value(field: &str, primary: bool) -> String {
    let desc = ADS_SORTS
        .iter()
        .find(|(f, ..)| *f == field)
        .map(|(_, d, ..)| *d)
        .unwrap_or(true);
    format!("{field} {}", if desc == primary { "desc" } else { "asc" })
}

/// The name for a sort parameter, falling back to the default — a state
/// file or a pasted URL could name one we do not know.
pub(super) fn ads_sort_name(sort: &str) -> &'static str {
    let (field, dir) = sort.split_once(' ').unwrap_or((sort, "desc"));
    ADS_SORTS
        .iter()
        .find(|(f, ..)| *f == field)
        .map(|(_, desc, primary, reverse)| {
            if (dir == "desc") == *desc {
                *primary
            } else {
                *reverse
            }
        })
        .unwrap_or(ADS_SORTS[0].2)
}

/// `(query, what it demonstrates)`. Four of each, chosen to span the
/// language rather than to be individually useful — between them they
/// show every shape of term the syntax has.
///
/// The ADS set is passed to ADS unmodified, so each one is a claim about
/// Solr that has to hold: all four were run against the live API and
/// return results (checked 2026-08-03). The filter set is checked by a
/// unit test against `src/query.rs`: an example using a field the
/// tokenizer does not know degrades to a bare term and quietly matches
/// nothing, which is the one way a sample can lie without erroring.
///
/// No sample carries an absolute upper year. `year:2020-2026` would go
/// on looking authoritative while silently excluding the newest work
/// from 2027 on; recency is the prompt's own control (⌃r), not
/// something to bake into the text.
pub(super) const ADS_SAMPLES: [(&str, &str); 4] = [
    ("abs:\"little red dot\" -doctype:abstract", "phrase, minus meeting abstracts"),
    ("author:\"^Andersson, K.\" year:2020-", "first author, from a year on"),
    ("bibstem:ApJL abs:\"magnetar\"", "one journal"),
    ("arxiv_class:astro-ph.HE", "an arXiv subject class"),
];

pub(super) const FILTER_SAMPLES: [(&str, &str); 4] = [
    ("^andersson year:2019-", "first author, open-ended years"),
    ("abs:\"fast radio burst\"", "phrase in the abstract"),
    ("is:pdf pri:>0.5", "has a PDF, high priority"),
    ("kw:\"compact objects\" -abs:neutrino", "keyword, and a negation"),
];

impl App {
    /// S — compose an ADS query. Pre-filled from the active local filter
    /// via to_ads_query (filter locally, escalate in one keystroke).
    pub(super) fn open_ads_prompt(&mut self) {
        if crate::ads::get_token().is_none() {
            // first run: collect the token right here, then come back
            self.mode = Mode::Setup { input: tui_input::Input::default(), email: false, resume: true };
            return;
        }
        let mut input = if self.filter.value().is_empty() {
            String::new()
        } else {
            query::to_ads_query(self.filter.value())
        };
        if let Some(Scope::Manuscript { rows }) = self.scopes.get(self.active_scope) {
            if let Some(r) = self.table.selected().and_then(|p| rows.get(p)) {
                if matches!(r.state, crate::library::CiteState::Missing) {
                    input = r.cited.clone();
                }
            }
        }
        self.mode = Mode::AdsPrompt {
            input: tui_input::Input::from(input),
            limit: 20,
            sort: crate::ads::DEFAULT_ADS_SORT.to_string(),
            edit: None,
        };
    }

    /// Run a query on a worker thread into a scope. A pasted DOI or ADS
    /// abstract URL short-circuits: DOI becomes a fielded query, an ADS
    /// URL imports the paper directly.
    pub(super) fn run_ads_query_limit(
        &mut self,
        raw: String,
        refresh_of: Option<usize>,
        limit: usize,
        sort: Option<String>,
    ) {
        let raw = raw.trim().to_string();
        if raw.is_empty() {
            return;
        }
        if let Some(bc) = crate::ads::bibcode_from_url(&raw) {
            self.import_bibcode(bc);
            return;
        }
        let query = match crate::ads::doi_from_text(&raw) {
            Some(doi) => format!("doi:\"{doi}\""),
            None => raw.clone(),
        };
        // refreshing keeps the existing tab identity; new queries mint one
        let tab = match refresh_of.and_then(|i| self.scopes.get(i)) {
            Some(Scope::Ads { tab, .. }) => {
                let mut t = tab.clone();
                t.limit = limit;
                // A plain refresh passes the query back unchanged; an
                // edit passes a new one. This used to keep the old text
                // either way, which nothing exercised — but it would have
                // meant searching for one thing and reporting another.
                if t.query != query {
                    // an unnamed tab is labelled from its query, so the
                    // label follows the text; a name someone typed is a
                    // decision and outlives the edit
                    if t.label == crate::tabs::short_label(&t.query) {
                        t.label = crate::tabs::short_label(&query);
                    }
                    t.query = query.clone();
                }
                if let Some(sort) = sort.clone() {
                    t.ads_sort = sort;
                }
                t
            }
            _ => {
                let mut t = crate::tabs::make_tab(&query, limit);
                // a fresh query takes the selection sort chosen at the
                // prompt; a refresh keeps whatever the tab already has
                if let Some(sort) = sort {
                    t.ads_sort = sort;
                }
                t
            }
        };
        // replacing the channel orphans any in-flight ADS work: its
        // receiver drops, so those tasks can never report back
        self.tasks
            .retain(|t| !matches!(t.kind, TaskKind::Query | TaskKind::Import));
        let id = self.add_task(TaskKind::Query, format!("⌕ ADS query — '{query}'"), vec![]);
        let (tx, rx) = std::sync::mpsc::channel();
        self.ads_rx = Some(rx);
        self.note(MsgCat::Info, format!("Searching ADS: {query}"));
        // the tab appears now, not when the results do. An ADS query can
        // take a minute, and a tab that only materialises at the end
        // leaves nothing on screen to say the query was even sent.
        if refresh_of.is_none() {
            let home = self.default_home(&query);
            self.scopes.push(Scope::Ads {
                tab: tab.clone(),
                articles: vec![],
                state: QueryState::Pending,
                home,
                seq: self.scope_seq,
            });
            self.scope_seq += 1;
            self.set_scope(self.scopes.len() - 1);
            self.regroup_scopes();
        } else if let Some(Scope::Ads { state, .. }) =
            refresh_of.and_then(|i| self.scopes.get_mut(i))
        {
            *state = QueryState::Pending;
        }
        let ads_sort = tab.ads_sort.clone();
        std::thread::spawn(move || {
            // the tab's own ADS sort selects the records — "the newest
            // n postings matching this", not an arbitrary n
            let result = crate::ads::search_sorted(&query, limit, &ads_sort)
                .map_err(|e| e.to_string());
            let _ = tx.send(AdsMsg::Done { id, tab, result });
        });
    }

    pub(super) fn drain_ads(&mut self) {
        let mut msgs = vec![];
        let mut done = false;
        if let Some(rx) = &self.ads_rx {
            loop {
                match rx.try_recv() {
                    Ok(m) => msgs.push(m),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        done = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                }
            }
        }
        if done {
            self.ads_rx = None;
        }
        for m in msgs {
            match m {
                AdsMsg::Done { id, mut tab, result } => {
                    let cancelled = self.finish_task(id).is_some_and(|t| t.cancelled);
                    // the tab was opened when the query was sent, so by
                    // now it may have been closed — in which case there
                    // is nowhere for the result to go
                    let Some(idx) = self.scope_of_tab(&tab.id) else { continue };
                    if cancelled {
                        self.note(MsgCat::Info, "cancelled — result discarded".to_string());
                        self.scopes.remove(idx);
                        self.set_scope(self.active_scope.min(self.scopes.len() - 1));
                        continue;
                    }
                    match result {
                        Ok(articles) => {
                            for a in &articles {
                                if let (Some(e), Some(c)) =
                                    (self.article_entry(a), a.citation_count)
                                {
                                    let key = e.key().to_string();
                                    self.metrics.set_citations(&key, c);
                                }
                            }
                            self.save_metrics();
                            let n = articles.len();
                            crate::tabs::save_cached_articles(&tab.id, &articles);
                            tab.refreshed = Some(crate::tabs::now_secs());
                            // the scope is rebuilt, not edited, so its
                            // home and its place in the order have to be
                            // carried across — a refresh must not re-file
                            // the query it refreshed, nor move it
                            let (home, seq) = match &self.scopes[idx] {
                                Scope::Ads { home, seq, .. } => (*home, *seq),
                                _ => (self.default_home(&tab.query), self.scope_seq),
                            };
                            self.scopes[idx] =
                                Scope::Ads { tab, articles, state: QueryState::Ready, home, seq };
                            // ADS hands results back in its own order;
                            // the tab's own sort decides what is shown
                            self.sort_ads_at(idx);
                            self.save_tabs();
                            // the search endpoint's meter, reported where
                            // it was just spent (@ shows the standing
                            // figures for every endpoint)
                            let spent = crate::ads::quotas()
                                .into_iter()
                                .find(|(ep, q)| *ep == "search" && !q.stale())
                                .map(|(_, q)| format!(" · {}/{} searches today", q.used(), q.limit))
                                .unwrap_or_default();
                            self.note(MsgCat::Ok, format!("{n} ADS result(s){spent}"));
                        }
                        Err(e) => {
                            // the page keeps the reason; a log line would
                            // scroll away while the tab still sat there
                            // saying nothing about why it is empty
                            if let Some(Scope::Ads { state, .. }) = self.scopes.get_mut(idx) {
                                *state = QueryState::Failed(e.clone());
                            }
                            self.note(MsgCat::Err, format!("ADS search failed: {e}"));
                        }
                    }
                }
                AdsMsg::Imported { id, bibcode, result } => {
                    // discard a cancelled import: nothing was written —
                    // save_entry only runs here, on the applied path
                    if let Some(t) = self.finish_task(id).filter(|t| t.cancelled) {
                        self.note(
                            MsgCat::Info,
                            format!("cancelled — {} (result discarded)", t.label),
                        );
                        continue;
                    }
                    match result {
                        Ok(data) => match self.lib.save_entry(&data) {
                            Ok(key) => {
                                if self.lib.in_manuscript(&key) {
                                    // entering via a manuscript: priority 1.0
                                    self.metrics.set_priority(&key, 1.0);
                                }
                                self.rebuild_order();
                                self.note(MsgCat::Ok, format!("Added {key}"));
                            }
                            Err(e) => self.note(MsgCat::Err, format!("import failed: {e}")),
                        },
                        Err(e) => {
                            self.note(MsgCat::Err, format!("import of {bibcode} failed: {e}"))
                        }
                    }
                }
            }
        }
        if done {
            // the channel can deliver nothing further; drop leftover
            // ADS tasks (safety net for orphaned work)
            self.tasks
                .retain(|t| !matches!(t.kind, TaskKind::Query | TaskKind::Import));
        }
    }

    /// The entries an action applies to: the selection (in display order)
    /// when selection mode is active and non-empty, else the highlighted
    /// row — one convention for every bulk-capable action.
    /// e — prompt for a destination path and export the selection (or
    /// the cursor entry) as one .bib file.
    /// E — edit the active query in place. S always composes a new one;
    /// this reopens the same prompt over the tab you are looking at, so
    /// every part of it the prompt owns can be changed at once without
    /// losing the tab's name or its place in the strip.
    pub(super) fn open_edit_query_prompt(&mut self) {
        let Some(Scope::Ads { tab, .. }) = self.scopes.get(self.active_scope) else {
            self.note(
                MsgCat::Warn,
                "only a saved query can be edited — open one with S".to_string(),
            );
            return;
        };
        self.mode = Mode::AdsPrompt {
            input: tui_input::Input::from(tab.query.clone()),
            limit: tab.limit,
            sort: tab.ads_sort.clone(),
            edit: Some(self.active_scope),
        };
    }

    /// Load a sample into the prompt being composed. The two prompts
    /// keep their text in different places: an ADS query lives on the
    /// mode, while the filter is an App field the mode does not own.
    pub(super) fn use_sample(&mut self, sample: &'static str) {
        match &mut self.mode {
            Mode::AdsPrompt { input, .. } => {
                *input = tui_input::Input::from(sample.to_string());
            }
            Mode::Filter => {
                self.filter = tui_input::Input::from(sample.to_string());
                self.refilter();
            }
            _ => {}
        }
    }

    /// N — rename the active query. Named queries are the point of
    /// saving them: "kilonova ejecta" says what you were doing where
    /// `abs:"kilonova" year:2020-` says only what you typed.
    /// Apply a name typed at the rename prompt. An empty name restores
    /// the one derived from the query text, which is the only way back.
    pub(super) fn rename_query(&mut self, name: String) {
        let Some(Scope::Ads { tab, .. }) = self.scopes.get_mut(self.active_scope) else {
            return;
        };
        let derived = crate::tabs::short_label(&tab.query);
        let (label, msg) = if name.is_empty() {
            (derived.clone(), format!("name cleared — showing '{derived}'"))
        } else {
            (name.clone(), format!("query named '{name}'"))
        };
        if tab.label == label {
            self.note(MsgCat::Info, "name unchanged".to_string());
            return;
        }
        tab.label = label;
        self.save_tabs();
        self.note(MsgCat::Ok, msg);
    }

    pub(super) fn open_rename_prompt(&mut self) {
        let Some(Scope::Ads { tab, .. }) = self.scopes.get(self.active_scope) else {
            self.note(
                MsgCat::Warn,
                "only a saved query can be renamed — the library and manuscript are fixed"
                    .to_string(),
            );
            return;
        };
        self.mode = Mode::Rename { input: tui_input::Input::from(tab.label.clone()) };
    }

    /// The library entry the pub card's buttons act on: in an ADS scope
    /// the shown article's imported twin (if any), else the selected
    /// entry. Distinct from selected_key because the card can preview a
    /// hovered row.
    /// Open a citations(...) (or references(...)) query scope for the
    /// card's paper — the C / R keys and the ⌕ card rows.
    pub(super) fn spawn_citation_query(&mut self, refs: bool) {
        // asking for citations already known to be zero costs a round
        // trip to be told what the card in front of you already says
        if !refs && self.card_citation_count() == Some(0) {
            self.note(
                MsgCat::Info,
                "ADS records nothing citing this paper yet".to_string(),
            );
            return;
        }
        if let Some(bc) = self.card_bibcode() {
            // identifier: (not bibcode:) — a preprint-imported entry
            // carries the arXiv bibcode, and citations(bibcode:…) finds
            // nothing on an alternate bibcode; identifier: resolves to
            // the canonical record (0 vs 9642 on GW170817's preprint)
            let q = if refs {
                format!("references(identifier:{bc})")
            } else {
                format!("citations(identifier:{bc})")
            };
            self.run_ads_query_limit(q, None, crate::tabs::DEFAULT_LIMIT, None);
        }
    }

    /// Re-sort one ADS scope's articles in place, by that scope's own
    /// sort (decorate-sort: cache and library lookups happen before the
    /// mutable put-back). A no-op for any other kind of scope.
    pub(super) fn sort_ads_at(&mut self, idx: usize) {
        let Some(Scope::Ads { tab, .. }) = self.scopes.get(idx) else {
            return;
        };
        let (col, asc) = (Col::from_tag(&tab.sort_col), tab.sort_asc);
        let Some(Scope::Ads { articles, .. }) = self.scopes.get_mut(idx) else {
            return;
        };
        let arts = std::mem::take(articles);
        let mut decorated: Vec<(String, crate::ads::Article)> = arts
            .into_iter()
            .map(|a| {
                let key = match col {
                    Col::Key => String::new(), // filled below with lib access
                    Col::Metric => format!("{:012}", a.citation_count.unwrap_or(0)),
                    Col::Year => a.year.clone(),
                    Col::Entered => a.entry_date.clone(),
                    Col::Author => a
                        .author
                        .first()
                        .map(|s| s.split(',').next().unwrap_or("").trim().to_lowercase())
                        .unwrap_or_default(),
                    Col::Title => a.title.to_lowercase(),
                    _ => String::new(), // Pdf/InLib filled below with lib access
                };
                (key, a)
            })
            .collect();
        if matches!(col, Col::Pdf | Col::InLib | Col::Key) {
            for (k, a) in decorated.iter_mut() {
                let entry = self.article_entry(a);
                *k = match col {
                    Col::Pdf => {
                        let ck = entry
                            .map(|e| e.key().to_string())
                            .unwrap_or_else(|| a.bibcode.clone());
                        u8::from(pdf::is_cached(&ck)).to_string()
                    }
                    Col::Key => self.hypothetical_key(a),
                    _ => u8::from(entry.is_some()).to_string(),
                };
            }
        }
        decorated.sort_by(|x, y| {
            let ord = x.0.cmp(&y.0).then(x.1.bibcode.cmp(&y.1.bibcode));
            if asc { ord } else { ord.reverse() }
        });
        let sorted: Vec<crate::ads::Article> = decorated.into_iter().map(|(_, a)| a).collect();
        if let Some(Scope::Ads { articles, .. }) = self.scopes.get_mut(idx) {
            *articles = sorted;
        }
    }

    /// `P` — take a query configuration off the clipboard and open it.
    ///
    /// Deliberately loud when the clipboard holds something else: the
    /// whole value of this is that it is one keystroke, and one
    /// keystroke that silently did nothing would be worse than none.
    pub(super) fn paste_query_config(&mut self) {
        let Some(text) = read_clipboard() else {
            self.note(
                MsgCat::Err,
                "could not read the clipboard on this system".to_string(),
            );
            return;
        };
        match crate::ads::parse_search_url(&text) {
            Some(_) => self.on_paste(text),
            None => {
                let what: String = text.trim().chars().take(40).collect();
                self.note(
                    MsgCat::Warn,
                    if what.is_empty() {
                        "the clipboard is empty — nothing to open".to_string()
                    } else {
                        format!("not an ADS query URL: {what}…")
                    },
                );
            }
        }
    }

    /// The keys cheat-sheet, a non-modal panel above the log: keys keep
    /// working while it shows; ? (or Esc, or the footer badge) closes.
    /// The samples for whichever prompt is up, or None when none is.
    fn active_samples(&self) -> Option<&'static [(&'static str, &'static str); 4]> {
        match self.mode {
            Mode::AdsPrompt { .. } => Some(&ADS_SAMPLES),
            Mode::Filter => Some(&FILTER_SAMPLES),
            _ => None,
        }
    }

    /// Rows the samples band wants, given what the centre column has
    /// left after the strip, the keys sheet and the log. Zero unless a
    /// prompt is up — and zero when taking them would leave the table
    /// too short to read, since a reference aid that hides the results
    /// it is helping you find is worse than none.
    /// Deliberately independent of whether the prompt is empty: the
    /// armed/inert flip happens on the first keystroke, and a height
    /// that moved with it would jump the table one row into every query
    /// you type.
    pub(super) fn samples_height(&self, spare: u16, width: u16) -> u16 {
        let Some(rows) = self.active_samples() else {
            return 0;
        };
        let want = rows.len() as u16 + 2; // heading line + a row of inset below
        let qw = rows.iter().map(|(q, _)| q.chars().count()).max().unwrap_or(0) as u16;
        // never truncate a query: what is shown must be what a click
        // loads, so the band stands down rather than lie about it
        if spare < want + 4 || width.saturating_sub(2) < qw + 2 {
            0
        } else {
            want
        }
    }

    /// A keypress while the ADS-returns menu is open. Returns whether
    /// it was claimed — everything is, so a stray key cannot type into
    /// the query behind the menu.
    ///
    /// Arrows only: ↑/↓ walk the fields, ←/→ turn the whole list around.
    /// Direction is one axis rather than a property of each row — "most
    /// or least" is the same question whichever field you are on — and
    /// there are no letter shortcuts, so nothing depends on shift, which
    /// is what broke on the digit key.
    ///
    /// Every move applies at once, so the prompt behind the menu always
    /// reads as what a search would do; the menu closes rather than
    /// commits.
    pub(super) fn sort_menu_key(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        let n = ADS_SORTS.len();
        match code {
            KeyCode::Esc | KeyCode::Enter => self.sort_menu = false,
            KeyCode::Char('r') if mods.contains(KeyModifiers::CONTROL) => self.sort_menu = false,
            KeyCode::Up => {
                self.sort_menu_sel = (self.sort_menu_sel + n - 1) % n;
                self.apply_sort_menu();
            }
            KeyCode::Down => {
                self.sort_menu_sel = (self.sort_menu_sel + 1) % n;
                self.apply_sort_menu();
            }
            // either arrow turns the list around: with two directions
            // there is nowhere else to go, so a key that only ever moved
            // one way would be dead half the time
            KeyCode::Left | KeyCode::Right => {
                self.sort_menu_primary = !self.sort_menu_primary;
                self.apply_sort_menu();
            }
            _ => {}
        }
        true
    }

    /// Put the highlighted field, in the direction the list is showing,
    /// on the prompt.
    pub(super) fn apply_sort_menu(&mut self) {
        let (field, ..) = ADS_SORTS[self.sort_menu_sel.min(ADS_SORTS.len() - 1)];
        let value = ads_sort_value(field, self.sort_menu_primary);
        if let Mode::AdsPrompt { sort, .. } = &mut self.mode {
            *sort = value.clone();
        }
        self.note_latest(
            MsgCat::Info,
            "ads-returns",
            format!("ADS returns {}", ads_sort_name(&value)),
        );
    }

    /// Open the menu on whatever the prompt is currently set to, so the
    /// cursor starts where you are rather than at the top.
    pub(super) fn open_sort_menu(&mut self) {
        let current = match &self.mode {
            Mode::AdsPrompt { sort, .. } => sort.clone(),
            _ => return,
        };
        let (field, dir) = current.split_once(' ').unwrap_or((current.as_str(), "desc"));
        self.sort_menu_sel = ADS_SORTS.iter().position(|(f, ..)| *f == field).unwrap_or(0);
        self.sort_menu_primary = ADS_SORTS
            .iter()
            .find(|(f, ..)| *f == field)
            .map(|(_, desc, ..)| (dir == "desc") == *desc)
            .unwrap_or(true);
        self.sort_menu = true;
    }

    /// Rows the ADS-returns menu wants: a heading, the list, and the
    /// closing inset — windowed to what the band can spare, so a short
    /// terminal gets a scrolling list rather than no menu at all.
    pub(super) fn sort_menu_height(&self, spare: u16, _width: u16) -> u16 {
        if !matches!(self.mode, Mode::AdsPrompt { .. }) {
            return 0;
        }
        let want = ADS_SORTS.len() as u16 + 2;
        // leave the table at least four rows; below that show fewer
        // fields rather than nothing
        want.min(spare.saturating_sub(4)).max(4)
    }

    /// Everything ADS will sort by, one per row.
    ///
    /// This was a four-way cycle on ⌃r until it became twenty — ten
    /// fields, each either way round — and a cycle cannot carry twenty.
    /// A list can: ↑/↓ choose the field and ←/→ turn every row around at
    /// once, since "most or least" is one question, not ten.
    pub(super) fn draw_sort_menu(&mut self, f: &mut Frame, area: Rect) {
        if area.height == 0 || !matches!(self.mode, Mode::AdsPrompt { .. }) {
            return;
        }
        let dim = Style::default().fg(Color::DarkGray);
        let primary = self.sort_menu_primary;
        let sel = self.sort_menu_sel.min(ADS_SORTS.len() - 1);
        // the visible window: the list scrolls only as far as it takes to
        // keep the cursor on screen, so the rows stay put while you walk
        let rows = (area.height.saturating_sub(2)) as usize;
        let first = if rows == 0 || sel < rows { 0 } else { sel + 1 - rows };
        let mut lines = vec![Line::from(Span::styled(
            format!(
                " ADS returns  ·  ↑↓ what to rank by  ·  ←→ {}  ·  ⏎ or Esc closes",
                if primary { "most first" } else { "least first" }
            ),
            dim,
        ))];
        for (i, (field, _, name_p, name_r)) in
            ADS_SORTS.iter().enumerate().skip(first).take(rows.max(1))
        {
            let name = if primary { name_p } else { name_r };
            let text = format!(" {} {name}", if i == sel { "▸" } else { " " });
            let rect = Rect {
                x: area.x + 1,
                y: area.y + 1 + (i - first) as u16,
                width: area.width.saturating_sub(2),
                height: 1,
            };
            self.hits.add(rect, Target::SortMenu(ads_sort_value(field, primary)));
            let hov = hit(rect, self.hover.0, self.hover.1);
            let style = match (i == sel, hov) {
                (true, _) => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                (false, true) => {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED)
                }
                (false, false) => Style::default().fg(table_text()),
            };
            lines.push(Line::from(Span::styled(text, style)));
        }
        f.render_widget(Block::default().style(Style::default().bg(help_bg())), area);
        f.render_widget(Paragraph::new(Text::from(lines)), panel_body(area));
    }

    /// One row per sample, each loading itself into the prompt.
    ///
    /// This exists because the syntax is needed *while* you type, which
    /// a modal cannot do — you would have to dismiss it to reach the
    /// prompt — and because a TUI has no copy-paste to carry an example
    /// across. Clicking sidesteps both.
    pub(super) fn draw_samples(&mut self, f: &mut Frame, area: Rect) {
        // stood down for want of room: without this the rows would still
        // be registered, over the footer, and clicking the footer would
        // load a sample
        if area.height == 0 {
            return;
        }
        let Some(rows) = self.active_samples() else {
            return;
        };
        let empty = self.prompt_is_empty();
        let dim = Style::default().fg(Color::DarkGray);
        // the heading names the surface and states the rule, in the one
        // row the box used to spend on a border. The rule has to be
        // stated here at all because the footer would normally carry it,
        // and the prompt is in the footer
        let mut lines = vec![Line::from(Span::styled(
            if empty {
                " examples  ·  click one to use it"
            } else {
                " examples  ·  clear the query to use one"
            },
            dim,
        ))];
        let qw = rows.iter().map(|(q, _)| q.chars().count()).max().unwrap_or(0);
        let pw = rows.iter().map(|(_, p)| p.chars().count()).max().unwrap_or(0);
        // the purpose is what gives way when the column is tight
        let show_purpose = (area.width.saturating_sub(2) as usize) >= qw + 3 + pw;
        for (i, (query, purpose)) in rows.iter().enumerate() {
            let y = area.y + 1 + i as u16;
            let r = Rect { x: area.x + 1, y, width: area.width.saturating_sub(2), height: 1 };
            // registered whether or not it can act: an unregistered row
            // would let the click through to the click-away dismissal,
            // which closes the prompt — worse than doing nothing
            self.hits.add(r, Target::Sample(query));
            let hov = empty && hit(r, self.hover.0, self.hover.1);
            let style = if hov {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED)
            } else if empty {
                Style::default().fg(table_text())
            } else {
                dim
            };
            let mut spans = vec![Span::styled(format!(" {query:<qw$}"), style)];
            if show_purpose {
                spans.push(Span::styled(format!("   {purpose}"), dim));
            }
            lines.push(Line::from(spans));
        }
        // the tint is the boundary: no border, so the heading takes the
        // top row and a row of inset closes the panel at the bottom
        f.render_widget(Block::default().style(Style::default().bg(help_bg())), area);
        f.render_widget(Paragraph::new(Text::from(lines)), panel_body(area));
    }

    /// ADS results: ↓ from the canonical cache key (the cite key once
    /// imported, the bibcode otherwise), ● from paper identity.
    pub(super) fn ads_model(&self, articles: &[crate::ads::Article], width: u16) -> table::TableModel {
        use ratatui::widgets::Cell;
        let columns = self.columns_for(ScopeKind::Query, width);
        let author_w = col_width(&columns, width, Col::Author);
        let (mvals, mknown) = self.metric_values();
        let cursor = self.table.selected();
        let hov_row = self.hovered_table_pos();
        let rows: Vec<Row<'static>> = articles
            .iter()
            .enumerate()
            .map(|(pos, a)| {
                let entry = self.article_entry(a);
                let cache_key = entry.map(|e| e.key()).unwrap_or(&a.bibcode);
                let author = a.author.join(" and ");
                let lit = hov_row == Some(pos);
                let (au_style, ti_style, yr_style) = if lit {
                    (
                        Style::default().fg(text_strong()),
                        Style::default().fg(text_strong()).add_modifier(Modifier::ITALIC),
                        Style::default().fg(Color::Green),
                    )
                } else {
                    (
                        Style::default().fg(table_text()),
                        Style::default().fg(table_text()).add_modifier(Modifier::ITALIC),
                        Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
                    )
                };
                let at_cursor = cursor == Some(pos);
                let (circle, circle_style) = self.gutter(Some(&a.bibcode), at_cursor);
                let cells: Vec<Cell> = columns
                    .iter()
                    .map(|c| match c.id {
                        Col::Sel => Cell::from(Span::styled(circle, circle_style)),
                        Col::Pdf => Cell::from(Span::styled(
                            if pdf::is_cached(cache_key) { "↓" } else { "" },
                            Style::default().fg(Color::Green),
                        )),
                        Col::InLib => Cell::from(Span::styled(
                            if entry.is_some() { "●" } else { "" },
                            Style::default().fg(Color::Magenta),
                        )),
                        Col::Metric => {
                            metric_cell(self.metric_col, mvals.get(pos).copied().flatten(), &mknown)
                        }
                        Col::Year => Cell::from(Span::styled(a.year.clone(), yr_style)),
                        Col::Entered => {
                            Cell::from(Span::styled(a.entry_date.clone(), yr_style))
                        }
                        Col::Author => Cell::from(Span::styled(
                            fit_authors(&author, author_w as usize),
                            au_style,
                        )),
                        Col::Title => Cell::from(Span::styled(a.title.clone(), ti_style)),
                        Col::Key => Cell::from(Span::styled(
                            self.hypothetical_key(a),
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                        )),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The ADS-returns table is the one place a wrong field name would
    /// go unnoticed: ADS *silently drops* a sort it does not know — it
    /// answers 200 with `"sort": ""` and default ordering — so the app
    /// would name an order it is not getting. Every field and both of
    /// its directions were checked against the live API; what a unit
    /// test can hold is the table's shape.
    #[test]
    fn ads_sort_table_is_well_formed() {
        let mut fields: Vec<&str> = ADS_SORTS.iter().map(|(f, ..)| *f).collect();
        for f in &fields {
            assert!(!f.contains(' '), "{f:?} is a field, not a sort value");
        }
        fields.sort_unstable();
        let n = fields.len();
        fields.dedup();
        assert_eq!(fields.len(), n, "each field appears once");
        // ADS offers Title in its own dropdown; it does nothing —
        // `title asc`, `title desc` and `score desc` come back identical
        assert!(!fields.contains(&"title"), "title sorts nothing at ADS");
    }

    #[test]
    fn every_sort_value_names_itself() {
        for (field, _, primary, reverse) in ADS_SORTS {
            assert_eq!(ads_sort_name(&ads_sort_value(field, true)), primary);
            assert_eq!(ads_sort_name(&ads_sort_value(field, false)), reverse);
            assert_ne!(ads_sort_value(field, true), ads_sort_value(field, false));
        }
        // names read forwards, counts and dates read biggest-first
        assert_eq!(ads_sort_value("bibcode", true), "bibcode asc");
        assert_eq!(ads_sort_value("first_author", true), "first_author asc");
        assert_eq!(ads_sort_value("citation_count", true), "citation_count desc");
        assert_eq!(ads_sort_value("entry_date", true), "entry_date desc");
    }

    /// A sort that is not in the table still has to render as something,
    /// since one can arrive from a pasted URL or an older state file.
    #[test]
    fn an_unknown_sort_falls_back_to_the_default_name() {
        assert_eq!(ads_sort_name("title desc"), ADS_SORTS[0].2);
        assert_eq!(ads_sort_name(""), ADS_SORTS[0].2);
    }

    /// A filter sample using a field the tokenizer does not know does
    /// not error — it degrades to a bare term and matches nothing,
    /// silently. That is the one way one of these can lie, so the check
    /// is mechanical rather than remembered.
    #[test]
    fn filter_samples_use_fields_the_tokenizer_knows() {
        for (q, _) in super::FILTER_SAMPLES {
            let groups = crate::query::tokenize(q);
            assert!(!groups.is_empty(), "{q}: tokenized to nothing");
            for t in groups.iter().flatten() {
                // a colon can only survive into a value by degrading
                assert!(
                    t.field.is_some() || !t.value.contains(':'),
                    "{q}: `{}` degraded to a bare term — unknown field?",
                    t.value
                );
                if t.field == Some(crate::query::Field::Is) {
                    assert!(
                        matches!(t.value.as_str(), "ms" | "pdf")
                            || t.value == crate::query::IS_TAGGED,
                        "{q}: is:{} matches nothing",
                        t.value
                    );
                }
            }
        }
    }

    /// The samples shown in the app and the ones documented in the
    /// README are the same list, or one of them is wrong.
    #[test]
    fn readme_documents_every_sample() {
        let readme = include_str!("../../README.md");
        for (q, _) in super::ADS_SAMPLES.iter().chain(super::FILTER_SAMPLES.iter()) {
            assert!(readme.contains(q), "README does not document the sample `{q}`");
        }
    }
}
