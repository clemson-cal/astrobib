//! Every user action: whether it is available, and what it does.

use super::*;

/// Every user action; the panel lists them all, dimming the unavailable.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Select,
    Manuscript,
    Share,
    Tag,
    Download,
    OpenPdf,
    ClearPdf,
    BrowserDl,
    Remove,
    Copy,
    Filter,
    Card,
    Log,
    Help,
    Columns,
    GlobalTier,
    QueryHome,
    CloseScope,
    Quit,
}

/// The key press a panel row stands for: a code and the modifiers it
/// carries, since not every row is a bare key.
pub(super) type Press = (KeyCode, KeyModifiers);

const fn k(c: char) -> Press {
    (KeyCode::Char(c), KeyModifiers::NONE)
}

const fn ctrl(c: char) -> Press {
    (KeyCode::Char(c), KeyModifiers::CONTROL)
}

// (shown key, label, availability probe, key a click synthesizes)
pub(super) const HELP_ENTRIES: &[(&str, &str, Option<Action>, Press)] = &[
    ("␣", "select / toggle row", Some(Action::Select), k(' ')),
    ("a", "select visible", Some(Action::Select), k('a')),
    ("A", "select all", Some(Action::Select), k('A')),
    ("j k", "move cursor", None, k('j')),
    ("g G", "first / last row", None, k('g')),
    ("m", "manuscript ± (selection)", Some(Action::Manuscript), k('m')),
    ("i", "import paper", None, k('i')),
    ("I", "import + share ↑", None, k('I')),
    ("s", "share to global ±", Some(Action::Share), k('s')),
    ("T", "tag ± (selection)", Some(Action::Tag), k('T')),
    ("p", "download PDF", Some(Action::Download), k('p')),
    ("B", "browser download", Some(Action::BrowserDl), k('B')),
    ("o", "open PDF", Some(Action::OpenPdf), k('o')),
    ("X", "clear PDF / cancel DL", Some(Action::ClearPdf), k('X')),
    ("y", "copy…", Some(Action::Copy), k('y')),
    ("⌫", "remove…", Some(Action::Remove), (KeyCode::Delete, KeyModifiers::NONE)),
    ("/", "filter", Some(Action::Filter), k('/')),
    ("D", "pub card", Some(Action::Card), k('D')),
    ("|", "table columns…", Some(Action::Columns), k('|')),
    ("N", "name this query…", None, k('N')),
    ("E", "edit this query…", None, k('E')),
    ("⌃w", "close this query", Some(Action::CloseScope), ctrl('w')),
    ("H", "query home ±", Some(Action::QueryHome), k('H')),
    ("y q", "copy this query", None, k('y')),
    ("P", "open query on clipboard", None, k('P')),
    ("L", "event log", Some(Action::Log), k('L')),
    ("t", "global tier", Some(Action::GlobalTier), k('t')),
    ("v", "pub view", None, k('v')),
    ("e", "export selection…", None, k('e')),
    ("M", "metric column", None, k('M')),
    (".", "priority → 1.0", None, k('.')),
    ("0", "priority → 0", None, k('0')),
    ("< >", "priority down / up", None, k('>')),
    ("@", "about", None, k('@')),
    ("C", "citations", None, k('C')),
    ("R", "references", None, k('R')),
    ("?", "this cheat-sheet", Some(Action::Help), k('?')),
    ("q", "quit", Some(Action::Quit), k('q')),
];

/// The keys panel's entries and fixed column width.
const HELP_COLW: u16 = 30;

impl App {
    /// Availability policy: single-target actions dim under multi-selection,
    /// content-dependent actions dim when no target qualifies.
    pub(super) fn available(&self, a: Action) -> bool {
        let keys = self.action_keys();
        let single = keys.len() == 1;
        let entry = |k: &String| self.lib.get(k);
        match a {
            Action::Select | Action::Filter | Action::Card | Action::Log | Action::Help
            | Action::Columns | Action::Quit => true,
            Action::GlobalTier => self.lib.manuscript.is_some(),
            // with no manuscript open there is no second home to move a
            // query to, so the gesture is not offered rather than being
            // offered and refusing
            Action::QueryHome => self.on_query() && self.manuscript_root().is_some(),
            // the library and manuscript scopes are permanent, so the
            // gesture is offered only where there is something to close
            Action::CloseScope => self.on_query(),
            Action::Manuscript => {
                self.lib.manuscript.is_some() && self.lib.global_on && !keys.is_empty()
            }
            // sharing needs two tiers to move a paper between: with only
            // the global library there is nowhere to promote from
            Action::Share => self.lib.manuscript.is_some() && !keys.is_empty(),
            // unlike the manuscript ±, tagging needs no local db: with
            // none, the tag is written to the global library
            Action::Tag => !keys.is_empty(),
            Action::Download => {
                self.dl_rx.is_none()
                    && keys.iter().any(|k| {
                        !pdf::is_cached(k)
                            && entry(k).is_some_and(|e| {
                                !e.eprint().is_empty() || !e.adsurl().is_empty()
                            })
                    })
            }
            Action::OpenPdf => keys.iter().any(|k| pdf::is_cached(k)),
            Action::ClearPdf => {
                self.poll_cancel.is_some() || keys.iter().any(|k| pdf::is_cached(k))
            }
            Action::BrowserDl => {
                single
                    && self.dl_rx.is_none()
                    && entry(&keys[0]).is_some_and(|e| {
                        !e.doi().is_empty() || !e.adsurl().is_empty() || !e.eprint().is_empty()
                    })
            }
            Action::Remove => !keys.is_empty(),
            Action::Copy => !keys.is_empty() || self.on_query(),
        }
    }

    /// Why an action is unavailable right now, in plain words. Pressing
    /// a key (or clicking its dimmed row) must never be silent: every
    /// variant either acts or explains itself through this.
    pub(super) fn unavailable_reason(&self, a: Action) -> String {
        let keys = self.action_keys();
        // every entry action needs a library cite key; on a query page a
        // row that was never imported has none — but an imported twin
        // does, so "import it first" must not be the answer for it
        let unimported = self.on_query() && keys.is_empty();
        let import_first =
            |why: &str| format!("import the paper first (i) — {why}");
        match a {
            Action::Manuscript if self.lib.manuscript.is_none() => {
                "no manuscript db (run inside a manuscript repo)".to_string()
            }
            Action::Manuscript if !self.lib.global_on => {
                // with the global tier hidden every resolvable paper is
                // already a manuscript member, so ± would only ever mean
                // "remove" — press t first and say which you meant
                "manuscript ± needs the global tier shown — press t".to_string()
            }
            Action::Manuscript if unimported => {
                import_first("the manuscript db holds library entries")
            }
            Action::Manuscript => "no paper under the cursor".to_string(),
            Action::Share if self.lib.manuscript.is_none() => {
                "no local db here — every paper is already in the global library".to_string()
            }
            Action::Share if unimported => import_first("sharing copies a library entry up"),
            Action::Share => "no paper to share".to_string(),
            Action::Download | Action::BrowserDl if self.dl_rx.is_some() => {
                "a download is already running".to_string()
            }
            Action::Download | Action::OpenPdf | Action::ClearPdf | Action::BrowserDl
                if unimported =>
            {
                import_first("PDFs are cached under the cite key")
            }
            Action::Download => "nothing to download (cached, or no arXiv ID / ADS URL)".to_string(),
            Action::OpenPdf => "no cached PDF here  (p downloads)".to_string(),
            Action::ClearPdf => "no cached PDF to clear".to_string(),
            Action::BrowserDl if keys.len() > 1 => {
                "browser download takes one paper at a time".to_string()
            }
            Action::BrowserDl => "no DOI, ADS URL, or arXiv ID to open".to_string(),
            Action::Tag if unimported => import_first("a tag names a library cite key"),
            Action::Tag => "no paper to tag".to_string(),
            Action::Remove if unimported => import_first("removal acts on the library entry"),
            Action::Remove => "no paper to remove".to_string(),
            Action::Copy => "nothing to copy".to_string(),
            Action::GlobalTier => {
                "no local db here — the global library is all there is".to_string()
            }
            Action::QueryHome if self.manuscript_root().is_none() => {
                "no manuscript here — every saved query is already global".to_string()
            }
            Action::QueryHome => "no query on screen to move".to_string(),
            Action::CloseScope => {
                "only a query capsule closes — the library and manuscript stay".to_string()
            }
            // always available; unreachable, but the match stays total
            Action::Select | Action::Filter | Action::Card | Action::Log | Action::Help
            | Action::Columns | Action::Quit => "not available here".to_string(),
        }
    }

    /// Run an action if available — shared by shortcut keys, panel clicks,
    /// and pub card buttons.
    pub(super) fn run_action(&mut self, a: Action) {
        if !self.available(a) {
            let why = self.unavailable_reason(a);
            self.note(MsgCat::Warn, why);
            return;
        }
        match a {
            Action::Select => {
                let was = self.select_mode;
                self.select_mode = true;
                if let Some(pos) = self.table.selected() {
                    self.toggle_row_selected(pos);
                }
                // an unselectable row (a cite resolving to no paper)
                // must not strand the user in an empty selection mode
                if !was && self.selected.is_empty() {
                    self.select_mode = false;
                }
            }
            Action::Manuscript => self.toggle_manuscript(),
            Action::Share => self.toggle_share(),
            Action::Tag => self.open_tag_prompt(),
            Action::Download => self.download_pdfs(),
            Action::OpenPdf => self.open_pdfs(),
            Action::ClearPdf => self.clear_pdfs(),
            Action::BrowserDl => self.browser_download(),
            Action::Remove => self.remove_papers(),
            Action::Copy => self.enter_copy_mode(),
            Action::Filter => {
                if self.active_scope == 0 {
                    self.mode = Mode::Filter;
                } else {
                    self.note(MsgCat::Warn, "the filter applies to the Library scope".to_string());
                }
            }
            // showing or hiding a view is its own confirmation: the view
            // appears. Logging it only pushes out the messages that are
            // there because you would otherwise have missed them.
            Action::Card => self.show_detail = !self.show_detail,
            Action::Log => self.show_log = !self.show_log,
            Action::Help => self.show_help = !self.show_help,
            Action::Columns => {
                self.show_columns = !self.show_columns;
                // opening it hands over the arrow keys, closing it gives
                // them back — otherwise the table would go quietly dead
                self.focus = if self.show_columns { Focus::Columns } else { Focus::Table };
                self.col_sel = self.col_sel.min(self.panel_rows().len().saturating_sub(1));
            }
            Action::GlobalTier => self.toggle_global(),
            Action::QueryHome => self.move_query_home(),
            Action::CloseScope => self.close_scope(),
            Action::Quit => self.quit = true,
        }
    }

    /// The control panel, tabbed: Actions lists every action with key,
    /// label, and click target, unavailable ones dimmed rather than
    /// hidden; Copy lists the clipboard targets of the y-chord the same
    /// way. Tab headers are clickable.
    /// Centered which-key modal for the y chord: clickable rows, items
    /// without a value dimmed; clicking elsewhere or Esc cancels.
    /// Rows the keys panel needs at this width (it flows into as many
    /// columns as fit), plus its border rows.
    pub(super) fn help_height(width: u16) -> u16 {
        let cols = (width.saturating_sub(2) / HELP_COLW).max(1) as usize;
        HELP_ENTRIES.len().div_ceil(cols) as u16 + 2
    }

    pub(super) fn draw_help(&mut self, f: &mut Frame, area: Rect) {
        let cols = (area.width.saturating_sub(2) / HELP_COLW).max(1) as usize;
        let rows = HELP_ENTRIES.len().div_ceil(cols);
        // the heading takes the row the top border used to, so the
        // entries below it keep the rows their click rects assume
        let mut lines: Vec<Line> =
            vec![Line::from(Span::styled(" keys ", Style::default().fg(Color::DarkGray)))];
        for r in 0..rows {
            let mut spans: Vec<Span> = vec![];
            for c in 0..cols {
                if let Some((key, label, action, click)) = HELP_ENTRIES.get(r + c * rows) {
                    let avail = action.is_none_or(|a| self.available(a));
                    // every row is a click target for its own key
                    let rect = Rect {
                        x: area.x + 1 + (c as u16) * HELP_COLW,
                        y: area.y + 1 + r as u16,
                        width: HELP_COLW,
                        height: 1,
                    };
                    let hov = avail && hit(rect, self.hover.0, self.hover.1);
                    // unavailable rows stay clickable: the key they
                    // synthesize explains itself in the footer, which
                    // beats a dimmed row that swallows the click
                    self.hits.add(rect, Target::Help(*click));
                    // an unavailable key is the inactive form of an
                    // available one, so it dims cyan rather than piling
                    // DIM onto gray — the row is still clickable, and
                    // doubly-dimmed gray disappears on lighter themes
                    let (mut ks, mut ls) = if avail {
                        (Style::default().fg(Color::Cyan), Style::default())
                    } else {
                        (
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                            Style::default().fg(Color::DarkGray),
                        )
                    };
                    if hov {
                        ks = ks.bg(row_hover_bg());
                        ls = ls.bg(row_hover_bg());
                    }
                    let text = format!(" {key:>3}  {label}");
                    let pad = (HELP_COLW as usize).saturating_sub(text.chars().count());
                    spans.push(Span::styled(format!(" {key:>3}  "), ks));
                    spans.push(Span::styled(format!("{label}{}", " ".repeat(pad)), ls));
                }
            }
            lines.push(Line::from(spans));
        }
        f.render_widget(Block::default().style(Style::default().bg(help_bg())), area);
        f.render_widget(Paragraph::new(Text::from(lines)), panel_body(area));
    }
}
