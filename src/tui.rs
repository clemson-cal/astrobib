//! Ratatui TUI: library table with live filter, pub card, star toggle.
//!
//! Feature parity with the Textual app comes incrementally; the current cut
//! covers browsing — instant startup, live filtering with the full query
//! language, manuscript ● and star ★ indicators, a toggleable pub card,
//! star toggling, and instant quit.

use crate::library::{has_cached_pdf, MergedLibrary};
use crate::pdf;
use crate::query::{self, QueryContext};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, TableState};
use ratatui::Frame;
use std::collections::HashSet;
use std::time::Duration;

pub fn run(lib: MergedLibrary) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let result = App::new(lib).run(&mut terminal);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

enum Mode {
    Normal,
    Filter,
    /// Modal ~/Downloads PDF picker (pub card "pick …").
    Pick {
        key: String,
        files: Vec<std::path::PathBuf>,
        sel: usize,
    },
    /// Confirm modal for removing papers (Delete key).
    Confirm { keys: Vec<String> },
    /// y pressed — the next key picks what to copy (the Copy panel tab
    /// shows the menu, which-key style); Esc cancels.
    Copy,
}

/// Every user action; the panel lists them all, dimming the unavailable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Select,
    Manuscript,
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
    Quit,
}

/// Message-log categories; each renders in its own color in the log
/// pane and the footer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MsgCat {
    Info,
    Ok,
    Warn,
    Err,
}

impl MsgCat {
    fn color(self) -> Color {
        match self {
            MsgCat::Info => Color::Gray,
            MsgCat::Ok => Color::Green,
            MsgCat::Warn => Color::Yellow,
            MsgCat::Err => Color::Red,
        }
    }
}

/// What the copy chord / Copy tab can put on the clipboard. Cite keys
/// and bibcodes join with ", " under multi-selection (the Python TUI's
/// convention); URLs, paths, and titles join with newlines.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyItem {
    Key,
    FullKey,
    Bibcode,
    AdsUrl,
    ArxivUrl,
    DoiUrl,
    PdfPath,
    Title,
    Abstract,
}

/// Sortable table columns; clicking a header toggles direction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortCol {
    Pdf,
    Year,
    Author,
    Title,
    Key,
}

/// Pub card buttons; they act on the card's (highlighted) entry.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CardBtn {
    Arxiv,
    Oa,
    Browser,
    Pick,
    Open,
    Clear,
    Cancel,
    MsToggle,
}

struct App {
    lib: MergedLibrary,
    order: Vec<String>,   // entry keys, year-descending
    filtered: Vec<usize>, // positions into `order` that pass the filter
    filter: String,
    mode: Mode,
    table: TableState,
    show_detail: bool,
    status: String,
    quit: bool,
    // iOS-style selection mode: circles appear, Space/click toggles rows,
    // Esc exits and clears; bulk actions apply to the selection
    select_mode: bool,
    selected: HashSet<String>,
    table_area: Rect, // last drawn table region, for mouse hit-testing
    dl_rx: Option<std::sync::mpsc::Receiver<DlMsg>>,
    // ctrl+p keyboard cheat-sheet overlay; any key or click dismisses
    show_help: bool,
    // copy-chord modal region and its clickable rows
    panel_area: Rect,
    panel_copy_rows: Vec<(Rect, CopyItem)>,
    // last known mouse position, for roll-over styling of clickables
    hover: (u16, u16),
    // transient footer hint while hovering a copy-region (never logged)
    hover_hint: Option<String>,
    // event log: (category, seconds-since-start, message); L toggles the
    // pane, the footer always shows the newest entry color-coded
    log: Vec<(MsgCat, u64, String)>,
    show_log: bool,
    started: std::time::Instant,
    // table sort (clickable column headers) and their header hit rects
    sort: (SortCol, bool), // (column, ascending)
    sort_headers: Vec<(Rect, SortCol)>,
    // footer view badges: clickable show/hide toggles per app-wide view
    footer_badges: Vec<(Rect, Action)>,
    // pub card button and link rects, rebuilt each draw
    card_buttons: Vec<(Rect, CardBtn)>,
    card_links: Vec<(Rect, String)>,
    card_yanks: Vec<(Rect, CopyItem)>,
    // transient PDF status shown on the card (waiting/result), like the
    // Python card's #pdf-status line
    pdf_status: String,
    // browser-download watcher cancel flag (X / clear cancels the poll)
    poll_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pick_area: Rect,
    confirm_btns: Vec<(Rect, bool)>, // (rect, is_confirm)
}

fn hit(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

/// Plain chips instead of powerline pills, for terminals without Nerd
/// Font glyphs (set ASTROBIB_ASCII=1).
fn ascii_chips() -> bool {
    static ASCII: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ASCII.get_or_init(|| std::env::var("ASTROBIB_ASCII").is_ok_and(|v| v != "0"))
}

/// Rendered cell width of a pill/chip: end caps (or padding) + label.
fn pill_width(label: &str) -> u16 {
    label.chars().count() as u16 + 2
}

/// A clickable rounded chip: powerline semicircle caps drawn in the chip
/// color around the label, so the row reads as a pill. ASCII mode renders
/// a plain padded chip of identical width, keeping click rects valid.
fn push_pill<'a>(spans: &mut Vec<Span<'a>>, label: &str, bg: Color, fg: Color) {
    if ascii_chips() {
        spans.push(Span::styled(
            format!(" {label} "),
            Style::default().bg(bg).fg(fg),
        ));
    } else {
        spans.push(Span::styled("\u{e0b6}".to_string(), Style::default().fg(bg)));
        spans.push(Span::styled(label.to_string(), Style::default().bg(bg).fg(fg)));
        spans.push(Span::styled("\u{e0b4}".to_string(), Style::default().fg(bg)));
    }
}

/// Greedy word wrap producing the exact lines we render — placement math
/// (links/buttons following variable-height text) depends on the count.
fn wrap_text(s: &str, w: usize) -> Vec<String> {
    if w == 0 {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = vec![];
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let wl = word.chars().count();
        let cl = cur.chars().count();
        if cl == 0 {
            cur = word.chars().take(w).collect();
        } else if cl + 1 + wl <= w {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.chars().take(w).collect();
        }
    }
    lines.push(cur);
    lines
}

enum DlMsg {
    Progress(String),
    Done { done: usize, failed: Vec<String> },
}

impl App {
    fn new(lib: MergedLibrary) -> Self {
        let mut order: Vec<String> = lib.entries().iter().map(|e| e.key().to_string()).collect();
        order.sort_by(|a, b| {
            let (ea, eb) = (lib.get(a).unwrap(), lib.get(b).unwrap());
            eb.year().cmp(&ea.year()).then(a.cmp(b))
        });
        let filtered = (0..order.len()).collect::<Vec<_>>();
        let mut table = TableState::default();
        if !filtered.is_empty() {
            table.select(Some(0));
        }
        let status = format!(
            "{} papers{}",
            order.len(),
            lib.manuscript
                .as_ref()
                .map(|m| format!(
                    "  ·  ms: {}",
                    m.root.file_name().unwrap_or_default().to_string_lossy()
                ))
                .unwrap_or_default()
        );
        App {
            lib,
            order,
            filtered,
            filter: String::new(),
            mode: Mode::Normal,
            table,
            show_detail: true,
            status,
            quit: false,
            select_mode: false,
            selected: HashSet::new(),
            table_area: Rect::default(),
            dl_rx: None,
            show_help: false,
            panel_area: Rect::default(),
            panel_copy_rows: vec![],
            hover: (u16::MAX, u16::MAX),
            hover_hint: None,
            log: vec![],
            show_log: false,
            started: std::time::Instant::now(),
            sort: (SortCol::Year, false),
            sort_headers: vec![],
            footer_badges: vec![],
            card_buttons: vec![],
            card_links: vec![],
            card_yanks: vec![],
            pdf_status: String::new(),
            poll_cancel: None,
            pick_area: Rect::default(),
            confirm_btns: vec![],
        }
    }

    /// Emit an event message: color-coded in the log pane and shown in
    /// the footer while it is the newest entry.
    fn note(&mut self, cat: MsgCat, msg: String) {
        self.status = msg.clone();
        self.log.push((cat, self.started.elapsed().as_secs(), msg));
    }

    /// Availability policy: single-target actions dim under multi-selection,
    /// content-dependent actions dim when no target qualifies.
    fn available(&self, a: Action) -> bool {
        let keys = self.action_keys();
        let single = keys.len() == 1;
        let entry = |k: &String| self.lib.get(k);
        match a {
            Action::Select | Action::Filter | Action::Card | Action::Log | Action::Help
            | Action::Quit => true,
            Action::Manuscript => self.lib.manuscript.is_some() && !keys.is_empty(),
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
            Action::Copy => !keys.is_empty(),
        }
    }

    /// Run an action if available — shared by shortcut keys, panel clicks,
    /// and pub card buttons.
    fn run_action(&mut self, a: Action) {
        if !self.available(a) {
            return;
        }
        match a {
            Action::Select => {
                self.select_mode = true;
                if let Some(pos) = self.table.selected() {
                    self.toggle_row_selected(pos);
                }
            }
            Action::Manuscript => self.toggle_manuscript(),
            Action::Download => self.download_pdfs(),
            Action::OpenPdf => self.open_pdfs(),
            Action::ClearPdf => self.clear_pdfs(),
            Action::BrowserDl => self.browser_download(),
            Action::Remove => self.remove_papers(),
            Action::Copy => self.enter_copy_mode(),
            Action::Filter => self.mode = Mode::Filter,
            Action::Card => self.show_detail = !self.show_detail,
            Action::Log => self.show_log = !self.show_log,
            Action::Help => self.show_help = !self.show_help,
            Action::Quit => self.quit = true,
        }
    }

    /// The entries an action applies to: the selection (in display order)
    /// when selection mode is active and non-empty, else the highlighted
    /// row — the Python TUI's convention.
    fn action_keys(&self) -> Vec<String> {
        if self.select_mode && !self.selected.is_empty() {
            return self
                .order
                .iter()
                .filter(|k| self.selected.contains(*k))
                .cloned()
                .collect();
        }
        self.selected_key().map(str::to_string).into_iter().collect()
    }

    fn run(mut self, terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
        let t0 = std::time::Instant::now();
        // Hold the first paint until the pty size settles. Some terminals
        // (Warp) resize the pty in reaction to alt-screen entry — often
        // before crossterm's SIGWINCH handler exists, so no Resize event
        // arrives. Painting at the transient size shows as a visible
        // reflow; a blank alt screen for ~50ms does not. Wait until the
        // size is stable for 50ms (cap 250ms); any user input ends the
        // wait and is handled after the first paint.
        let mut pending: Option<Event> = None;
        let mut size = terminal.size()?;
        let mut stable = std::time::Instant::now();
        while t0.elapsed() < Duration::from_millis(250) {
            if event::poll(Duration::from_millis(10))? {
                let ev = event::read()?;
                if !matches!(ev, Event::Resize(..)) {
                    pending = Some(ev);
                    break;
                }
            }
            let now_size = terminal.size()?;
            if now_size != size {
                size = now_size;
                stable = std::time::Instant::now();
            }
            if stable.elapsed() >= Duration::from_millis(50) {
                break;
            }
        }
        debug_layout(&format!(
            "{:>6}ms settled at {size:?}",
            t0.elapsed().as_millis()
        ));
        while !self.quit {
            self.drain_downloads();
            terminal.draw(|f| self.draw(f))?;
            if let Some(ev) = pending.take() {
                debug_layout(&format!("{:>6}ms pending {ev:?}", t0.elapsed().as_millis()));
                match ev {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.on_key(key.code, key.modifiers)
                    }
                    Event::Mouse(m) => self.on_mouse(m),
                    _ => {}
                }
                continue;
            }
            // fast cadence for the first second (late terminal resizes that
            // slip past the settle window) and while downloads report progress
            let tick = if t0.elapsed() < Duration::from_secs(1) || self.dl_rx.is_some() {
                25
            } else {
                250
            };
            debug_layout(&format!(
                "{:>6}ms draw frame={:?} table={:?}",
                t0.elapsed().as_millis(),
                terminal.get_frame().area(),
                self.table_area,
            ));
            if event::poll(Duration::from_millis(tick))? {
                let ev = event::read()?;
                debug_layout(&format!("{:>6}ms event {ev:?}", t0.elapsed().as_millis()));
                match ev {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.on_key(key.code, key.modifiers)
                    }
                    Event::Mouse(m) => self.on_mouse(m),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// The table position under the mouse, if any (header and rule
    /// rows excluded).
    fn hovered_table_pos(&self) -> Option<usize> {
        let a = self.table_area;
        if !hit(a, self.hover.0, self.hover.1) || self.hover.1 <= a.y + 1 {
            return None;
        }
        let pos = self.table.offset() + (self.hover.1 - a.y - 2) as usize;
        (pos < self.filtered.len()).then_some(pos)
    }

    /// The entry the pub card shows: a hovered table row previews in the
    /// card; otherwise the cursor row.
    fn card_key(&self) -> Option<&str> {
        if let Some(pos) = self.hovered_table_pos() {
            return self.filtered.get(pos).map(|&i| self.order[i].as_str());
        }
        self.selected_key()
    }

    fn selected_key(&self) -> Option<&str> {
        let pos = self.table.selected()?;
        let idx = *self.filtered.get(pos)?;
        Some(self.order[idx].as_str())
    }

    fn refilter(&mut self) {
        let groups = query::tokenize(&self.filter);
        let in_ms: Vec<String> = self
            .lib
            .manuscript
            .as_ref()
            .map(|m| m.entries().iter().map(|e| e.key().to_string()).collect())
            .unwrap_or_default();
        let ctx = QueryContext {
            in_manuscript: Some(Box::new(move |k: &str| in_ms.iter().any(|x| x == k))),
            has_pdf: Some(Box::new(|k: &str| has_cached_pdf(k))),
        };
        self.filtered = self
            .order
            .iter()
            .enumerate()
            .filter(|(_, key)| query::matches(&groups, self.lib.get(key).unwrap(), &ctx))
            .map(|(i, _)| i)
            .collect();
        let sel = self.table.selected().unwrap_or(0);
        self.table.select(if self.filtered.is_empty() {
            None
        } else {
            Some(sel.min(self.filtered.len() - 1))
        });
    }

    fn move_sel(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let cur = self.table.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.filtered.len() as isize - 1);
        self.table.select(Some(next as usize));
        self.pdf_status.clear(); // stale per-entry message
    }

    /// Toggle selection membership of the row at a filtered position.
    /// A selection emptied by toggling exits selection mode, same as Esc.
    fn toggle_row_selected(&mut self, pos: usize) {
        let Some(&idx) = self.filtered.get(pos) else {
            return;
        };
        let key = self.order[idx].clone();
        if !self.selected.remove(&key) {
            self.selected.insert(key);
        }
        if self.select_mode && self.selected.is_empty() {
            self.exit_select_mode();
        } else {
            self.status = format!("{} selected", self.selected.len());
        }
    }

    fn exit_select_mode(&mut self) {
        self.select_mode = false;
        self.selected.clear();
        self.status = format!("{} papers", self.order.len());
    }

    /// Rebuild the display order (entries changed or sort changed).
    fn rebuild_order(&mut self) {
        self.order = self.lib.entries().iter().map(|e| e.key().to_string()).collect();
        let lib = &self.lib;
        let (col, asc) = self.sort;
        self.order.sort_by(|a, b| {
            let (ea, eb) = (lib.get(a).unwrap(), lib.get(b).unwrap());
            let ord = match col {
                SortCol::Pdf => has_cached_pdf(ea.key()).cmp(&has_cached_pdf(eb.key())),
                SortCol::Year => ea.year().cmp(&eb.year()),
                SortCol::Author => ea
                    .first_author_last()
                    .to_lowercase()
                    .cmp(&eb.first_author_last().to_lowercase()),
                SortCol::Title => ea
                    .title()
                    .trim_matches(['{', '}'])
                    .to_lowercase()
                    .cmp(&eb.title().trim_matches(['{', '}']).to_lowercase()),
                SortCol::Key => ea.key().cmp(eb.key()),
            };
            let ord = if asc { ord } else { ord.reverse() };
            ord.then(a.cmp(b))
        });
        self.selected.retain(|k| lib.get(k).is_some());
        self.refilter();
    }

    /// Header click: same column flips direction, a new column starts
    /// descending for Year (newest first) and ascending otherwise.
    fn sort_by(&mut self, col: SortCol) {
        self.sort = if self.sort.0 == col {
            (col, !self.sort.1)
        } else {
            // bool-ish and recency columns start with the interesting side
            // up: cached/starred/newest first; text columns start A→Z
            (col, !matches!(col, SortCol::Year | SortCol::Pdf))
        };
        self.rebuild_order();
    }

    /// m — port of action_toggle_manuscript's library-view rule: if any
    /// target is missing from the manuscript db, add all missing; else
    /// (all present) remove all.
    fn toggle_manuscript(&mut self) {
        if self.lib.manuscript.is_none() {
            self.note(MsgCat::Warn, "no manuscript db (run inside a manuscript repo)".to_string());
            return;
        }
        let keys = self.action_keys();
        if keys.is_empty() {
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

    /// Delete — ask for confirmation before removing.
    fn remove_papers(&mut self) {
        let keys = self.action_keys();
        if !keys.is_empty() {
            self.mode = Mode::Confirm { keys };
        }
    }

    /// Confirmed removal from both databases; exits selection mode after.
    fn remove_confirmed(&mut self, keys: &[String]) {
        let mut n = 0;
        for k in keys {
            if self.lib.remove_entry(k).is_ok() {
                n += 1;
            }
        }
        if self.select_mode {
            self.select_mode = false;
            self.selected.clear();
        }
        self.rebuild_order();
        self.note(MsgCat::Ok, format!("Removed {n} paper(s)"));
    }

    /// o — open every cached PDF among the targets.
    fn open_pdfs(&mut self) {
        let paths: Vec<_> = self
            .action_keys()
            .iter()
            .filter(|k| pdf::is_cached(k))
            .map(|k| pdf::cache_path(k))
            .collect();
        if paths.is_empty() {
            self.note(MsgCat::Warn, "no cached PDFs in selection  (p to download)".to_string());
            return;
        }
        let n = paths.len();
        pdf::open_paths(&paths);
        self.note(MsgCat::Ok, format!("Opened {n} PDF(s)"));
    }

    /// X — cancel a running browser-download watch, else clear cached PDFs.
    fn clear_pdfs(&mut self) {
        if let Some(cancel) = self.poll_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.note(MsgCat::Warn, "browser download cancelled".to_string());
            return;
        }
        self.pdf_status.clear();
        let mut n = 0;
        for k in self.action_keys() {
            let p = pdf::cache_path(&k);
            if p.exists() && std::fs::remove_file(&p).is_ok() {
                n += 1;
            }
        }
        self.note(MsgCat::Ok, format!("Cleared {n} cached PDF(s)"));
    }

    /// Fetch one entry's PDF from a specific source (pub card buttons),
    /// on the download worker channel.
    fn download_single(&mut self, key: String, source: pdf::Source) {
        if self.dl_rx.is_some() {
            self.note(MsgCat::Warn, "a download is already running".to_string());
            return;
        }
        let Some(e) = self.lib.get(&key) else { return };
        let (eprint, adsurl) = (e.eprint().to_string(), e.adsurl().to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
        self.note(MsgCat::Info, format!("Downloading {key}…"));
        std::thread::spawn(move || {
            let ok = pdf::fetch_source(&key, &eprint, &adsurl, source).is_some();
            let _ = tx.send(DlMsg::Done {
                done: ok as usize,
                failed: if ok { vec![] } else { vec![key] },
            });
        });
    }

    fn browser_download(&mut self) {
        if let Some(k) = self.action_keys().into_iter().next() {
            self.browser_download_for(k);
        }
    }

    /// B — resolve the best manual-download URL, open the browser, and
    /// watch ~/Downloads for the PDF (60s, cancellable with X).
    fn browser_download_for(&mut self, key: String) {
        if self.dl_rx.is_some() {
            self.note(MsgCat::Warn, "a download is already running".to_string());
            return;
        }
        let Some(e) = self.lib.get(&key) else {
            return;
        };
        let (key, doi, adsurl, eprint) = (
            e.key().to_string(),
            e.doi().to_string(),
            e.adsurl().to_string(),
            e.eprint().to_string(),
        );
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.poll_cancel = Some(cancel.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
        self.note(MsgCat::Info, format!("Resolving browser source for {key}…"));
        std::thread::spawn(move || {
            let Some(url) = pdf::browser_resolve_url(&doi, &adsurl, &eprint) else {
                let _ = tx.send(DlMsg::Done { done: 0, failed: vec![key] });
                return;
            };
            let before = pdf::downloads_snapshot();
            pdf::browser_open(&url);
            let _ = tx.send(DlMsg::Progress(format!(
                "Browser opened — waiting for {key} in ~/Downloads (60s, X cancels)…"
            )));
            let got = pdf::poll_downloads(&key, &before, 60, &cancel);
            let _ = tx.send(DlMsg::Done {
                done: got.is_some() as usize,
                failed: if got.is_some() { vec![] } else { vec![key] },
            });
        });
    }

    /// pick … — open the modal ~/Downloads PDF picker for one entry.
    fn open_picker_for(&mut self, key: String) {
        let files = pdf::downloads_pdfs();
        if files.is_empty() {
            self.note(MsgCat::Info, "no PDFs in ~/Downloads".to_string());
            return;
        }
        self.mode = Mode::Pick { key, files, sel: 0 };
    }

    /// p — download PDFs for targets not yet cached, on a background
    /// thread so the UI stays live; progress arrives over a channel.
    fn download_pdfs(&mut self) {
        if self.dl_rx.is_some() {
            self.note(MsgCat::Warn, "a download is already running".to_string());
            return;
        }
        let items: Vec<(String, String, String)> = self
            .action_keys()
            .iter()
            .filter(|k| !pdf::is_cached(k))
            .filter_map(|k| {
                let e = self.lib.get(k)?;
                if e.eprint().is_empty() && e.adsurl().is_empty() {
                    return None;
                }
                Some((k.clone(), e.eprint().to_string(), e.adsurl().to_string()))
            })
            .collect();
        if items.is_empty() {
            self.note(MsgCat::Warn, "nothing to download (cached, or no arXiv ID / ADS URL)".to_string());
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
        let total = items.len();
        std::thread::spawn(move || {
            let mut done = 0;
            let mut failed = vec![];
            for (i, (key, eprint, adsurl)) in items.iter().enumerate() {
                let _ = tx.send(DlMsg::Progress(format!(
                    "Downloading [{}/{total}] {key}…",
                    i + 1
                )));
                if pdf::fetch(key, eprint, adsurl).is_some() {
                    done += 1;
                } else {
                    failed.push(key.clone());
                }
            }
            let _ = tx.send(DlMsg::Done { done, failed });
        });
        self.note(MsgCat::Info, format!("Downloading {total} PDF(s)…"));
    }

    fn drain_downloads(&mut self) {
        let mut msgs = vec![];
        if let Some(rx) = &self.dl_rx {
            while let Ok(m) = rx.try_recv() {
                msgs.push(m);
            }
        }
        for m in msgs {
            match m {
                DlMsg::Progress(s) => self.status = s,
                DlMsg::Done { done, failed } => {
                    let cat = if failed.is_empty() { MsgCat::Ok } else { MsgCat::Err };
                    let msg = if failed.is_empty() {
                        format!("Downloaded {done} PDF(s)")
                    } else {
                        format!(
                            "Downloaded {done} PDF(s) — failed: {}{}",
                            failed[..failed.len().min(3)].join(", "),
                            if failed.len() > 3 { "…" } else { "" }
                        )
                    };
                    self.note(cat, msg);
                    self.pdf_status = if failed.is_empty() && done > 0 {
                        "✓ downloaded".to_string()
                    } else if done == 0 && !failed.is_empty() {
                        "✗ no PDF found — try browser ↓".to_string()
                    } else {
                        String::new()
                    };
                    self.dl_rx = None;
                    self.poll_cancel = None;
                }
            }
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollDown => self.move_sel(3),
            MouseEventKind::ScrollUp => self.move_sel(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                self.on_click(m.column, m.row, m.modifiers)
            }
            MouseEventKind::Moved => self.hover = (m.column, m.row),
            _ => {}
        }
    }

    fn on_click(&mut self, x: u16, y: u16, mods: KeyModifiers) {
        // modal picker swallows all clicks: row click imports, outside closes
        if let Mode::Pick { key, files, .. } = &self.mode {
            if hit(self.pick_area, x, y) && y > self.pick_area.y {
                let i = (y - self.pick_area.y - 1) as usize;
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
        if self.show_help {
            self.show_help = false; // any click dismisses the cheat-sheet
            return;
        }
        // confirm modal: only its two buttons act; other clicks are inert
        if let Mode::Confirm { keys } = &self.mode {
            if let Some(&(_, is_confirm)) = self.confirm_btns.iter().find(|(r, _)| hit(*r, x, y)) {
                let keys = keys.clone();
                self.mode = Mode::Normal;
                if is_confirm {
                    self.remove_confirmed(&keys);
                } else {
                    self.note(MsgCat::Warn, "removal cancelled".to_string());
                }
            }
            return;
        }
        // copy-regions: the card text copies its own entry's datum
        if let Some(&(_, item)) = self.card_yanks.iter().find(|(r, _)| hit(*r, x, y)) {
            if let Some(key) = self.selected_key().map(str::to_string) {
                self.do_copy_single(key, item);
            }
            return;
        }
        // pub card links open the browser
        if let Some((_, url)) = self.card_links.iter().find(|(r, _)| hit(*r, x, y)) {
            let url = url.clone();
            pdf::browser_open(&url);
            self.note(MsgCat::Info, "opened in browser".to_string());
            return;
        }
        // control panel rows: the copy menu while the y-chord is active,
        // the actions list otherwise
        // copy-chord modal: a row copies, anything else cancels the chord
        if matches!(self.mode, Mode::Copy) {
            if let Some(&(_, item)) = self.panel_copy_rows.iter().find(|(r, _)| hit(*r, x, y)) {
                self.do_copy(item);
            } else {
                self.exit_copy_mode();
                self.note(MsgCat::Warn, "copy cancelled".to_string());
            }
            return;
        }
        // pub card buttons (act on the card's entry)
        if let Some(&(_, btn)) = self.card_buttons.iter().find(|(r, _)| hit(*r, x, y)) {
            if let Some(key) = self.selected_key().map(str::to_string) {
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
                                Ok(true) => Some(format!("◆ Added {key} to manuscript db")),
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
        // footer view badges
        if let Some(&(_, action)) = self.footer_badges.iter().find(|(r, _)| hit(*r, x, y)) {
            self.run_action(action);
            return;
        }
        // column headers sort
        if let Some(&(_, col)) = self.sort_headers.iter().find(|(r, _)| hit(*r, x, y)) {
            self.sort_by(col);
            return;
        }
        // table: header at a.y, rule below it, data rows after
        let a = self.table_area;
        if !hit(a, x, y) || y <= a.y + 1 {
            return;
        }
        let pos = self.table.offset() + (y - a.y - 2) as usize;
        if pos >= self.filtered.len() {
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
        }
    }

    /// Clear (or cancel a pending browser watch for) the card entry's PDF.
    fn clear_card_pdf(&mut self, key: &str) {
        if let Some(cancel) = self.poll_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.note(MsgCat::Warn, "browser download cancelled".to_string());
            return;
        }
        self.pdf_status.clear();
        let p = pdf::cache_path(key);
        if p.exists() && std::fs::remove_file(&p).is_ok() {
            self.note(MsgCat::Ok, format!("Cleared cached PDF for {key}"));
        }
    }

    /// y — await a target key; the panel force-shows the Copy tab as a
    /// which-key menu (restored by exit_copy_mode).
    fn enter_copy_mode(&mut self) {
        if self.action_keys().is_empty() {
            self.note(MsgCat::Warn, "nothing to copy".to_string());
            return;
        }
        self.mode = Mode::Copy;
    }

    fn exit_copy_mode(&mut self) {
        if matches!(self.mode, Mode::Copy) {
            self.mode = Mode::Normal;
        }
    }

    /// The clipboard text an item yields over the current targets, or
    /// None when no target has the field (also the panel's dimming test).
    fn copy_value(&self, item: CopyItem) -> Option<String> {
        self.copy_value_keys(&self.action_keys(), item)
    }

    fn copy_value_keys(&self, keys: &[String], item: CopyItem) -> Option<String> {
        let mut vals: Vec<String> = vec![];
        for k in keys {
            let Some(e) = self.lib.get(k) else { continue };
            let v = match item {
                CopyItem::Key => Some(if e.short_key.is_empty() {
                    e.key().to_string()
                } else {
                    e.short_key.clone()
                }),
                CopyItem::FullKey => Some(e.key().to_string()),
                CopyItem::Bibcode => e.bibcode().map(str::to_string),
                CopyItem::AdsUrl => (!e.adsurl().is_empty()).then(|| e.adsurl().to_string()),
                CopyItem::ArxivUrl => (!e.eprint().is_empty())
                    .then(|| format!("https://arxiv.org/abs/{}", e.eprint())),
                CopyItem::DoiUrl => {
                    (!e.doi().is_empty()).then(|| format!("https://doi.org/{}", e.doi()))
                }
                CopyItem::PdfPath => pdf::is_cached(k)
                    .then(|| pdf::cache_path(k).to_string_lossy().into_owned()),
                CopyItem::Abstract => {
                    (!e.abstract_().is_empty()).then(|| e.abstract_().to_string())
                }
                CopyItem::Title => {
                    let t = e.title().trim_matches(['{', '}']).to_string();
                    (!t.is_empty()).then_some(t)
                }
            };
            if let Some(v) = v {
                vals.push(v);
            }
        }
        if vals.is_empty() {
            return None;
        }
        let sep = match item {
            CopyItem::Key | CopyItem::FullKey | CopyItem::Bibcode => ", ",
            _ => "\n",
        };
        Some(vals.join(sep))
    }

    /// Copy one entry's datum (the card's copy-regions path).
    fn do_copy_single(&mut self, key: String, item: CopyItem) {
        match self.copy_value_keys(&[key], item) {
            Some(text) => self.finish_copy(&text.clone()),
            None => self.note(MsgCat::Warn, "nothing to copy".to_string()),
        }
    }

    fn do_copy(&mut self, item: CopyItem) {
        self.exit_copy_mode();
        let Some(text) = self.copy_value(item) else {
            self.note(MsgCat::Warn, "nothing to copy".to_string());
            return;
        };
        self.finish_copy(&text);
    }

    fn finish_copy(&mut self, text: &str) {
        if copy_to_clipboard(&text) {
            let first = text.lines().next().unwrap_or("");
            let mut shown: String = first.chars().take(60).collect();
            if shown.len() < text.len() {
                shown.push('…');
            }
            self.note(MsgCat::Ok, format!("Copied: {shown}"));
        } else {
            self.note(
                MsgCat::Err,
                "clipboard copy failed — terminal may not support OSC 52".to_string(),
            );
        }
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.show_help {
            self.show_help = false; // any key dismisses the cheat-sheet
            return;
        }
        match &mut self.mode {
            Mode::Filter => match code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.mode = Mode::Normal;
                    self.refilter();
                }
                KeyCode::Enter => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.refilter();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.refilter();
                }
                _ => {}
            },
            Mode::Pick { key, files, sel } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Char('j') | KeyCode::Down => {
                    *sel = (*sel + 1).min(files.len().saturating_sub(1))
                }
                KeyCode::Char('k') | KeyCode::Up => *sel = sel.saturating_sub(1),
                KeyCode::Enter => {
                    let (key, file) = (key.clone(), files[*sel].clone());
                    self.mode = Mode::Normal;
                    self.import_picked(&key, &file);
                }
                _ => {}
            },
            Mode::Copy => {
                let item = match code {
                    KeyCode::Char('y') => Some(CopyItem::Key),
                    KeyCode::Char('Y') => Some(CopyItem::FullKey),
                    KeyCode::Char('b') => Some(CopyItem::Bibcode),
                    KeyCode::Char('a') => Some(CopyItem::AdsUrl),
                    KeyCode::Char('x') => Some(CopyItem::ArxivUrl),
                    KeyCode::Char('d') => Some(CopyItem::DoiUrl),
                    KeyCode::Char('p') => Some(CopyItem::PdfPath),
                    KeyCode::Char('t') => Some(CopyItem::Title),
                    KeyCode::Char('A') => Some(CopyItem::Abstract),
                    _ => None,
                };
                match item {
                    Some(item) => self.do_copy(item),
                    None => {
                        self.exit_copy_mode();
                        self.note(MsgCat::Warn, "copy cancelled".to_string());
                    }
                }
            }
            Mode::Confirm { keys } => match code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let keys = keys.clone();
                    self.mode = Mode::Normal;
                    self.remove_confirmed(&keys);
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.mode = Mode::Normal;
                    self.note(MsgCat::Warn, "removal cancelled".to_string());
                }
                _ => {}
            },
            Mode::Normal => match code {
                KeyCode::Char('q') => self.run_action(Action::Quit),
                KeyCode::Char('/') => self.run_action(Action::Filter),
                KeyCode::Char('m') => self.run_action(Action::Manuscript),
                KeyCode::Delete | KeyCode::Backspace => self.run_action(Action::Remove),
                KeyCode::Char('p') => self.run_action(Action::Download),
                KeyCode::Char('o') => self.run_action(Action::OpenPdf),
                KeyCode::Char('X') => self.run_action(Action::ClearPdf),
                KeyCode::Char('B') => self.run_action(Action::BrowserDl),
                KeyCode::Char('D') | KeyCode::Char('z') => self.run_action(Action::Card),
                KeyCode::Char('?') => self.run_action(Action::Help),
                KeyCode::Char('L') => self.run_action(Action::Log),
                KeyCode::Char('y') => self.run_action(Action::Copy),
                KeyCode::Char('Y') => self.do_copy(CopyItem::FullKey),
                KeyCode::Char(' ') => self.run_action(Action::Select),
                KeyCode::Esc => {
                    if self.select_mode {
                        self.exit_select_mode();
                    } else if !self.filter.is_empty() {
                        self.filter.clear();
                        self.refilter();
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
                KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
                KeyCode::Char('g') | KeyCode::Home => {
                    self.table.select((!self.filtered.is_empty()).then_some(0))
                }
                KeyCode::Char('G') | KeyCode::End => {
                    self.table.select(self.filtered.len().checked_sub(1))
                }
                KeyCode::PageDown => self.move_sel(20),
                KeyCode::PageUp => self.move_sel(-20),
                _ => {}
            },
        }
    }

    fn import_picked(&mut self, key: &str, file: &std::path::Path) {
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

    fn draw(&mut self, f: &mut Frame) {
        self.card_buttons.clear();
        self.hover_hint = None;
        self.panel_copy_rows.clear();
        let log_h = if self.show_log {
            (self.log.len().min(8) + 2) as u16
        } else {
            0
        };
        let [main, log_area, status] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(log_h),
            Constraint::Length(1),
        ])
        .areas(f.area());
        let mut constraints = vec![Constraint::Min(40)];
        if self.show_detail {
            constraints.push(Constraint::Length(48));
        }
        let areas = Layout::horizontal(constraints).split(main);
        let mut it = areas.iter();
        let table_area = *it.next().unwrap();
        let detail_area = self.show_detail.then(|| *it.next().unwrap());

        self.draw_table(f, table_area);
        if let Some(area) = detail_area {
            self.draw_detail(f, area);
        } else {
            self.card_yanks.clear();
        }
        if matches!(self.mode, Mode::Copy) {
            self.draw_copy_modal(f);
        } else {
            self.panel_area = Rect::default();
        }
        if self.show_help {
            self.draw_help(f);
        }
        if self.show_log {
            self.draw_log(f, log_area);
        }
        self.draw_status(f, status);
        if let Mode::Pick { .. } = self.mode {
            self.draw_picker(f);
        } else {
            self.pick_area = Rect::default();
        }
        if let Mode::Confirm { .. } = self.mode {
            self.draw_confirm(f);
        } else {
            self.confirm_btns.clear();
        }
    }

    /// Centered confirm modal for Delete: lists the targets, offers
    /// clickable remove/cancel (⏎/y confirms, Esc/n cancels).
    fn draw_confirm(&mut self, f: &mut Frame) {
        self.confirm_btns.clear();
        let Mode::Confirm { keys } = &self.mode else { return };
        let frame = f.area();
        let listed: Vec<&String> = keys.iter().take(8).collect();
        let extra = keys.len().saturating_sub(listed.len());
        let h = (listed.len() + if extra > 0 { 1 } else { 0 } + 4) as u16;
        let w = 52.min(frame.width.saturating_sub(4));
        let area = Rect {
            x: frame.width.saturating_sub(w) / 2,
            y: frame.height.saturating_sub(h) / 2,
            width: w,
            height: h.min(frame.height),
        };
        f.render_widget(ratatui::widgets::Clear, area);
        let mut lines: Vec<Line> = vec![];
        for k in &listed {
            lines.push(Line::from(Span::styled(
                k.to_string(),
                Style::default().fg(Color::Cyan),
            )));
        }
        if extra > 0 {
            lines.push(Line::from(Span::styled(
                format!("… and {extra} more"),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::default());
        let by = area.y + h.saturating_sub(2);
        let bx = area.x + 1;
        let (rw, cw) = (pill_width("remove"), pill_width("cancel"));
        self.confirm_btns
            .push((Rect { x: bx, y: by, width: rw, height: 1 }, true));
        self.confirm_btns.push((
            Rect { x: bx + rw + 2, y: by, width: cw, height: 1 },
            false,
        ));
        let hov_remove = self
            .confirm_btns
            .first()
            .is_some_and(|(r, _)| hit(*r, self.hover.0, self.hover.1));
        let hov_cancel = self
            .confirm_btns
            .get(1)
            .is_some_and(|(r, _)| hit(*r, self.hover.0, self.hover.1));
        let mut bspans: Vec<Span> = vec![];
        push_pill(
            &mut bspans,
            "remove",
            if hov_remove { Color::LightRed } else { Color::Red },
            Color::White,
        );
        bspans.push(Span::raw("  "));
        push_pill(
            &mut bspans,
            "cancel",
            if hov_cancel { Color::Rgb(58, 63, 72) } else { Color::Rgb(40, 44, 52) },
            Color::White,
        );
        lines.push(Line::from(bspans));
        let title = format!(
            " Remove {} paper(s) from the library? ",
            keys.len()
        );
        let p = Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title(title));
        f.render_widget(p, area);
    }

    /// The ctrl+p control panel, tabbed: Actions lists every action with
    /// key, label, and click target, unavailable ones dimmed (the Python
    /// key panel's behavior); Copy lists the clipboard targets of the
    /// y-chord the same way. Tab headers are clickable.
    /// Centered which-key modal for the y chord: clickable rows, items
    /// without a value dimmed; clicking elsewhere or Esc cancels.
    fn draw_copy_modal(&mut self, f: &mut Frame) {
        self.panel_copy_rows.clear();
        let entries: &[(&str, &str, CopyItem)] = &[
            ("y", "cite key", CopyItem::Key),
            ("Y", "full key", CopyItem::FullKey),
            ("b", "bibcode", CopyItem::Bibcode),
            ("a", "ADS URL", CopyItem::AdsUrl),
            ("x", "arXiv URL", CopyItem::ArxivUrl),
            ("d", "DOI URL", CopyItem::DoiUrl),
            ("p", "PDF path", CopyItem::PdfPath),
            ("t", "title", CopyItem::Title),
            ("A", "abstract", CopyItem::Abstract),
        ];
        let frame = f.area();
        let h = entries.len() as u16 + 3;
        let w = 30.min(frame.width.saturating_sub(4));
        let area = Rect {
            x: frame.width.saturating_sub(w) / 2,
            y: frame.height.saturating_sub(h) / 2,
            width: w,
            height: h.min(frame.height),
        };
        self.panel_area = area;
        f.render_widget(ratatui::widgets::Clear, area);
        let mut lines: Vec<Line> = vec![];
        for (i, (key, label, item)) in entries.iter().enumerate() {
            let y = area.y + 1 + i as u16;
            let avail = self.copy_value(*item).is_some();
            if avail {
                self.panel_copy_rows.push((
                    Rect { x: area.x + 1, y, width: w.saturating_sub(2), height: 1 },
                    *item,
                ));
            }
            let hov = avail && hit(
                Rect { x: area.x + 1, y, width: w.saturating_sub(2), height: 1 },
                self.hover.0,
                self.hover.1,
            );
            let (mut ks, mut ls) = if avail {
                (Style::default().fg(Color::Cyan), Style::default())
            } else {
                (
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                )
            };
            if hov {
                ks = ks.bg(Color::Rgb(50, 54, 62));
                ls = ls.bg(Color::Rgb(50, 54, 62)).add_modifier(Modifier::BOLD);
            }
            lines.push(Line::from(vec![
                Span::styled(format!(" {key:>2}  "), ks),
                Span::styled((*label).to_string(), ls),
            ]));
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            " Esc cancel",
            Style::default().fg(Color::DarkGray),
        )));
        let p = Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" copy → clipboard "),
        );
        f.render_widget(p, area);
    }

    /// Centered keyboard cheat-sheet (ctrl+p); any key or click dismisses.
    fn draw_help(&mut self, f: &mut Frame) {
        let entries: &[(&str, &str)] = &[
            ("␣", "select / toggle row"),
            ("j k", "move cursor"),
            ("g G", "first / last row"),
            ("m", "manuscript ± (selection)"),
            ("p", "download PDF"),
            ("B", "browser download"),
            ("o", "open PDF"),
            ("X", "clear PDF / cancel DL"),
            ("y", "copy…"),
            ("⌫", "remove…"),
            ("/", "filter"),
            ("D", "pub card"),
            ("L", "event log"),
            ("?", "this cheat-sheet"),
            ("q", "quit"),
        ];
        let frame = f.area();
        let rows = entries.len().div_ceil(2) as u16;
        let h = rows + 2;
        let w = 62.min(frame.width.saturating_sub(4));
        let colw = (w - 2) / 2;
        let area = Rect {
            x: frame.width.saturating_sub(w) / 2,
            y: frame.height.saturating_sub(h) / 2,
            width: w,
            height: h.min(frame.height),
        };
        f.render_widget(ratatui::widgets::Clear, area);
        let mut lines: Vec<Line> = vec![];
        for r in 0..rows as usize {
            let mut spans: Vec<Span> = vec![];
            for c in 0..2usize {
                if let Some((key, label)) = entries.get(r + c * rows as usize) {
                    let text = format!(" {key:>3}  {label}");
                    let pad = (colw as usize).saturating_sub(text.chars().count());
                    spans.push(Span::styled(
                        format!(" {key:>3}  "),
                        Style::default().fg(Color::Cyan),
                    ));
                    spans.push(Span::raw(format!("{label}{}", " ".repeat(pad))));
                }
            }
            lines.push(Line::from(spans));
        }
        let p = Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title(" keys "));
        f.render_widget(p, area);
    }

    /// Centered modal list of ~/Downloads PDFs for the pick action.
    fn draw_picker(&mut self, f: &mut Frame) {
        let Mode::Pick { files, sel, key } = &self.mode else {
            return;
        };
        let frame = f.area();
        let h = (files.len().min(15) + 2) as u16;
        let w = 64.min(frame.width.saturating_sub(4));
        let area = Rect {
            x: frame.width.saturating_sub(w) / 2,
            y: frame.height.saturating_sub(h) / 2,
            width: w,
            height: h.min(frame.height),
        };
        self.pick_area = area;
        f.render_widget(ratatui::widgets::Clear, area);
        let mut lines: Vec<Line> = vec![];
        for (i, p) in files.iter().take(15).enumerate() {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            let row_y = area.y + 1 + i as u16;
            let hov = self.hover.1 == row_y && hit(area, self.hover.0, self.hover.1);
            let style = if i == *sel {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else if hov {
                Style::default().bg(Color::Rgb(50, 54, 62))
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(name, style)));
        }
        let p = Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" pick PDF for {key} — ⏎ import · Esc cancel ")),
        );
        f.render_widget(p, area);
    }

    fn draw_table(&mut self, f: &mut Frame, area: Rect) {
        use ratatui::widgets::Cell;
        self.table_area = area;
        self.sort_headers.clear();
        // subtle per-column palette; the terminal theme supplies the hues
        let c_ind = Style::default().fg(Color::Cyan);
        let c_pdf = Style::default().fg(Color::Green);
        let c_ms = Style::default().fg(Color::Magenta);
        let c_year = Style::default().fg(Color::Green).add_modifier(Modifier::DIM);
        let c_author = Style::default().fg(Color::Gray);
        let c_key = Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM);
        // responsive columns: author scales, Key drops first when tight
        let (author_w, show_key) = column_layout(area.width);
        let hov_row = self.hovered_table_pos();
        let rows: Vec<Row> = self
            .filtered
            .iter()
            .enumerate()
            .map(|(pos, &i)| {
                let e = self.lib.get(&self.order[i]).unwrap();
                let circle = if !self.select_mode {
                    ""
                } else if self.selected.contains(e.key()) {
                    "◉"
                } else {
                    "◯"
                };
                let mut cells = vec![
                    Cell::from(Span::styled(circle, c_ind)),
                    Cell::from(Span::styled(
                        if has_cached_pdf(e.key()) { "↓" } else { "" },
                        c_pdf,
                    )),
                    Cell::from(Span::styled(
                        if self.lib.in_manuscript(e.key()) { "●" } else { "" },
                        c_ms,
                    )),
                    Cell::from(Span::styled(e.year(), c_year)),
                    Cell::from(Span::styled(
                        fit_authors(e.author(), author_w as usize),
                        c_author,
                    )),
                    Cell::from(Span::styled(
                        e.title().trim_matches(['{', '}']).to_string(),
                        Style::default().add_modifier(Modifier::ITALIC),
                    )),
                ];
                if show_key {
                    cells.push(Cell::from(Span::styled(e.short_key.clone(), c_key)));
                }
                let row = Row::new(cells);
                if hov_row == Some(pos) {
                    row.style(Style::default().bg(Color::Rgb(38, 42, 50)))
                } else {
                    row
                }
            })
            .collect();

        // header: sortable columns get a click rect and a ▲/▼ marker
        let mut widths: Vec<u16> = vec![2, 2, 2, 6, author_w, 0];
        if show_key {
            widths.push(20);
        }
        let ncols = widths.len() as u16;
        let (sort_col, asc) = self.sort;
        let mut hx = area.x;
        let title_w = area
            .width
            .saturating_sub(widths.iter().sum::<u16>() + ncols);
        let ms_header = if self.lib.manuscript.is_some() { "●" } else { "" };
        let mut headers: Vec<&str> = vec!["", "↓", ms_header, "Year", "Author", "Title"];
        if show_key {
            headers.push("Key");
        }
        let mut header_spans: Vec<Span> = vec![];
        for (ci, base) in headers.iter().enumerate() {
            let cw = if ci == 5 { title_w } else { widths[ci] };
            let col = match ci {
                1 => Some(SortCol::Pdf),
                3 => Some(SortCol::Year),
                4 => Some(SortCol::Author),
                5 => Some(SortCol::Title),
                6 => Some(SortCol::Key),
                _ => None,
            };
            let mut label = base.to_string();
            let mut style = Style::default().add_modifier(Modifier::BOLD);
            if let Some(col) = col {
                let r = Rect { x: hx, y: area.y, width: cw, height: 1 };
                self.sort_headers.push((r, col));
                if sort_col == col {
                    let arrow = if asc { "▲" } else { "▼" };
                    // narrow indicator columns fit glyph+arrow only
                    label = if cw <= 2 { format!("{base}{arrow}") } else { format!("{base} {arrow}") };
                }
                if hit(r, self.hover.0, self.hover.1) {
                    style = style.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED);
                }
            }
            let pad = (cw as usize).saturating_sub(label.chars().count());
            header_spans.push(Span::styled(label, style));
            header_spans.push(Span::raw(" ".repeat(pad + 1)));
            hx += cw + 1;
        }
        f.render_widget(
            Paragraph::new(Line::from(header_spans)),
            Rect { x: area.x, y: area.y, width: area.width, height: 1 },
        );
        f.render_widget(
            Paragraph::new(Span::styled(
                "─".repeat(area.width as usize),
                Style::default().fg(Color::DarkGray),
            )),
            Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 },
        );
        let data_area = Rect {
            x: area.x,
            y: area.y + 2,
            width: area.width,
            height: area.height.saturating_sub(2),
        };

        let mut constraints = vec![
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(6),
            Constraint::Length(author_w),
            Constraint::Min(20),
        ];
        if show_key {
            constraints.push(Constraint::Length(20));
        }
        let table = Table::new(rows, constraints)
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::NONE));
        f.render_stateful_widget(table, data_area, &mut self.table);
    }

    /// The pub card, emulating the Python DetailPanel's flow: body (title,
    /// authors · year, abstract), a bordered links row (ADS · arXiv:id ·
    /// DOI, cyan, browser-opening), the PDF buttons (Python labels/colors;
    /// ineligible ones hidden, not dimmed), a transient PDF status line,
    /// and a footer (keywords, cite key with dim hash suffix, preprint
    /// note). Text is pre-wrapped so every row's click rect is exact.
    fn draw_detail(&mut self, f: &mut Frame, area: Rect) {
        self.card_links.clear();
        f.render_widget(Block::default().borders(Borders::LEFT), area);
        let Some(key) = self.card_key().map(str::to_string) else {
            return;
        };
        let Some(e) = self.lib.get(&key) else { return };
        let x0 = area.x + 3; // border + 2 padding
        let w = area.width.saturating_sub(5) as usize;
        let bottom = area.y + area.height;
        let mut y = area.y + 1;
        let line_at = |f: &mut Frame, y: u16, line: Line| {
            if y < bottom {
                f.render_widget(
                    Paragraph::new(line),
                    Rect { x: x0, y, width: w as u16, height: 1 },
                );
            }
        };

        let hv = self.hover;
        // copy-regions: the text itself is the click target; hovering any
        // line of a region tints the whole region and hints in the footer
        let hov_region: Option<CopyItem> = self
            .card_yanks
            .iter()
            .find(|(r, _)| hit(*r, hv.0, hv.1))
            .map(|&(_, item)| item);
        if let Some(item) = hov_region {
            let what = match item {
                CopyItem::Title => "title",
                CopyItem::Abstract => "abstract",
                _ => "cite key",
            };
            self.hover_hint = Some(format!("⧉ click to copy {what}"));
        }
        let mut yanks: Vec<(Rect, CopyItem)> = vec![];
        let tint = Style::default().bg(Color::Rgb(44, 48, 56));
        let region_style = |base: Style, item: CopyItem| {
            if hov_region == Some(item) { base.patch(tint) } else { base }
        };
        // ── body ─────────────────────────────────────────────────────
        for l in wrap_text(e.title().trim_matches(['{', '}']), w) {
            let lw = l.chars().count() as u16;
            yanks.push((Rect { x: x0, y, width: lw.max(1), height: 1 }, CopyItem::Title));
            line_at(
                f,
                y,
                Line::from(Span::styled(
                    l,
                    region_style(
                        Style::default().add_modifier(Modifier::BOLD | Modifier::ITALIC),
                        CopyItem::Title,
                    ),
                )),
            );
            y += 1;
        }
        y += 1;
        let year = e.year();
        let byline = format!("{}   ·   {year}", format_authors(e.author()));
        let by_lines = wrap_text(&byline, w);
        for (i, l) in by_lines.iter().enumerate() {
            let line = if i == by_lines.len() - 1 && l.chars().count() > year.chars().count() {
                let split = l.chars().count() - year.chars().count();
                let head: String = l.chars().take(split).collect();
                Line::from(vec![
                    Span::styled(head, Style::default().fg(Color::DarkGray)),
                    Span::styled(year.clone(), Style::default().fg(Color::Green)),
                ])
            } else {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::DarkGray)))
            };
            line_at(f, y, line);
            y += 1;
        }
        let (eprint, adsurl, doi) = (
            e.eprint().to_string(),
            e.adsurl().to_string(),
            e.doi().to_string(),
        );
        // footer + links/buttons need this much room below the abstract
        let kws = e.keywords().join(" · ");
        let kw_lines = if kws.is_empty() { 0 } else { wrap_text(&kws, w).len() as u16 + 1 };
        let has_ms = self.lib.manuscript.is_some();
        let rest = 3 + 1 + u16::from(has_ms) + 1 + kw_lines + 2;
        let abs = e.abstract_();
        if !abs.is_empty() && y + rest < bottom {
            y += 1;
            let shown: String = abs.chars().take(1000).collect();
            let avail = (bottom - y).saturating_sub(rest);
            for l in wrap_text(&shown, w).into_iter().take(avail as usize) {
                let lw = l.chars().count() as u16;
                yanks.push((
                    Rect { x: x0, y, width: lw.max(1), height: 1 },
                    CopyItem::Abstract,
                ));
                line_at(
                    f,
                    y,
                    Line::from(Span::styled(
                        l,
                        region_style(Style::default(), CopyItem::Abstract),
                    )),
                );
                y += 1;
            }
        }

        // ── links row (bordered top and bottom, like #detail-links) ──
        let sep = "─".repeat(w);
        let dimsep = Style::default().fg(Color::DarkGray);
        line_at(f, y, Line::from(Span::styled(sep.clone(), dimsep)));
        y += 1;
        let mut spans: Vec<Span> = vec![];
        let mut lx = x0;
        let cyan = Style::default().fg(Color::Cyan);
        let link = |label: String, url: String, spans: &mut Vec<Span>, lx: &mut u16, links: &mut Vec<(Rect, String)>| {
            let wl = label.chars().count() as u16;
            if y < bottom {
                links.push((Rect { x: *lx, y, width: wl, height: 1 }, url));
            }
            let r = Rect { x: *lx, y, width: wl, height: 1 };
            let style = if hit(r, hv.0, hv.1) {
                cyan.add_modifier(Modifier::UNDERLINED)
            } else {
                cyan
            };
            spans.push(Span::styled(label, style));
            spans.push(Span::raw("  "));
            *lx += wl + 2;
        };
        if !adsurl.is_empty() {
            link("ADS".into(), adsurl.clone(), &mut spans, &mut lx, &mut self.card_links);
        }
        if !eprint.is_empty() {
            link(
                format!("arXiv:{eprint}"),
                format!("https://arxiv.org/abs/{eprint}"),
                &mut spans,
                &mut lx,
                &mut self.card_links,
            );
        }
        if !doi.is_empty() {
            link("DOI".into(), format!("https://doi.org/{doi}"), &mut spans, &mut lx, &mut self.card_links);
        }
        line_at(f, y, Line::from(spans));
        y += 1;
        line_at(f, y, Line::from(Span::styled(sep, dimsep)));
        y += 2; // a little air below the links row

        // ── PDF buttons (Python labels, colors, and visibility rules),
        //    drawn as rounded pills ─────────────────────────────────────
        let cached = pdf::is_cached(&key);
        let muted = Style::default().fg(Color::Gray);
        let mut buttons: Vec<(&str, CardBtn, Color)> = vec![];
        if !cached && !eprint.is_empty() {
            buttons.push(("arXiv ↓", CardBtn::Arxiv, Color::Cyan));
        }
        if !cached && !adsurl.is_empty() {
            buttons.push(("ADS OA ↓", CardBtn::Oa, Color::Cyan));
        }
        if !cached && (!doi.is_empty() || !adsurl.is_empty()) {
            buttons.push(("browser ↓", CardBtn::Browser, Color::Yellow));
        }
        if !cached {
            buttons.push(("pick …", CardBtn::Pick, Color::Magenta));
        }
        if cached {
            buttons.push(("Open ↗", CardBtn::Open, Color::Green));
            buttons.push(("Clear ✕", CardBtn::Clear, Color::Gray));
        }
        let mut spans: Vec<Span> = vec![];
        let mut bx = x0;
        for (label, btn, fg) in buttons {
            let wl = pill_width(label);
            let r = Rect { x: bx, y, width: wl, height: 1 };
            if y < bottom {
                self.card_buttons.push((r, btn));
            }
            let bg = if hit(r, hv.0, hv.1) {
                Color::Rgb(58, 63, 72)
            } else {
                Color::Rgb(40, 44, 52)
            };
            push_pill(&mut spans, label, bg, fg);
            spans.push(Span::raw(" "));
            bx += wl + 1;
        }
        line_at(f, y, Line::from(spans));
        y += 1;

        // ── manuscript membership chip (acts on the card's entry) ────
        if has_ms {
            let in_ms = self.lib.in_manuscript(&key);
            let label = if in_ms { "◆ in manuscript" } else { "◇ add to manuscript" };
            let wl = pill_width(label);
            let r = Rect { x: x0, y, width: wl, height: 1 };
            self.card_buttons.push((r, CardBtn::MsToggle));
            let bg = if hit(r, hv.0, hv.1) {
                Color::Rgb(58, 63, 72)
            } else {
                Color::Rgb(40, 44, 52)
            };
            let mut spans: Vec<Span> = vec![];
            push_pill(
                &mut spans,
                label,
                bg,
                if in_ms { Color::Magenta } else { Color::Gray },
            );
            line_at(f, y, Line::from(spans));
            y += 1;
        }

        // ── PDF status (⏳ waiting…, ✓/✗ results) ────────────────────
        if self.poll_cancel.is_some() {
            let label = "⏳ waiting for download…  cancel ✕";
            if y < bottom {
                self.card_buttons.push((
                    Rect { x: x0, y, width: label.chars().count() as u16, height: 1 },
                    CardBtn::Cancel,
                ));
            }
            line_at(f, y, Line::from(Span::styled(label, Style::default().fg(Color::Yellow))));
        } else if !self.pdf_status.is_empty() {
            line_at(f, y, Line::from(Span::styled(self.pdf_status.clone(), muted)));
        }
        y += 2;

        // ── footer ───────────────────────────────────────────────────
        if !kws.is_empty() {
            for l in wrap_text(&kws, w) {
                line_at(f, y, Line::from(Span::styled(l, Style::default().fg(Color::DarkGray))));
                y += 1;
            }
            y += 1;
        }
        let short = if e.short_key.is_empty() { e.key() } else { &e.short_key };
        let suffix: String = e.key().chars().skip(short.chars().count()).collect();
        let mut spans = vec![
            Span::styled(short.to_string(), cyan),
            Span::styled(suffix, Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)),
        ];
        let preprint = e
            .adsurl()
            .rsplit('/')
            .next()
            .is_some_and(|bc| bc.len() > 9 && bc[..4].chars().all(|c| c.is_ascii_digit()) && &bc[4..9] == "arXiv");
        if preprint {
            spans.push(Span::styled("  (preprint)", Style::default().fg(Color::DarkGray)));
        }
        let used: u16 = spans.iter().map(|s| s.content.chars().count() as u16).sum();
        yanks.push((Rect { x: x0, y, width: used.max(1), height: 1 }, CopyItem::Key));
        if hov_region == Some(CopyItem::Key) {
            for s in &mut spans {
                s.style = s.style.patch(tint);
            }
        }
        line_at(f, y, Line::from(spans));
        self.card_yanks = yanks;
    }


    /// Right-aligned clickable show/hide badges for each app-wide view.
    fn draw_badges(&mut self, f: &mut Frame, area: Rect) {
        self.footer_badges.clear();
        let badges: [(&str, bool, Action); 3] = [
            ("card[D]", self.show_detail, Action::Card),
            ("log[L]", self.show_log, Action::Log),
            ("keys[?]", self.show_help, Action::Help),
        ];
        let total: u16 = badges.iter().map(|(l, _, _)| l.chars().count() as u16 + 3).sum();
        let mut bx = (area.x + area.width).saturating_sub(total);
        let mut spans: Vec<Span> = vec![];
        for (label, on, action) in badges {
            let wl = label.chars().count() as u16 + 2;
            let r = Rect { x: bx, y: area.y, width: wl, height: 1 };
            self.footer_badges.push((r, action));
            let hov = hit(r, self.hover.0, self.hover.1);
            let style = match (on, hov) {
                (true, true) => Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED),
                (true, false) => Style::default().fg(Color::Cyan),
                (false, true) => Style::default().fg(Color::Gray).add_modifier(Modifier::UNDERLINED),
                (false, false) => Style::default().fg(Color::DarkGray),
            };
            spans.push(Span::styled(
                format!("{} {label}", if on { "◼" } else { "◻" }),
                style,
            ));
            spans.push(Span::raw(" "));
            bx += wl + 1;
        }
        let w = total.min(area.width);
        let badge_area = Rect {
            x: (area.x + area.width).saturating_sub(w),
            y: area.y,
            width: w,
            height: 1,
        };
        f.render_widget(Paragraph::new(Line::from(spans)), badge_area);
    }

    /// The event-log pane: newest entries at the bottom, one line each,
    /// color-coded by category, mm:ss timestamps since launch.
    fn draw_log(&self, f: &mut Frame, area: Rect) {
        let n = area.height.saturating_sub(2) as usize;
        let start = self.log.len().saturating_sub(n);
        let mut lines: Vec<Line> = vec![];
        for (cat, secs, msg) in &self.log[start..] {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:02}:{:02}  ", secs / 60, secs % 60),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(msg.clone(), Style::default().fg(cat.color())),
            ]));
        }
        let block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .title(Span::styled(" Log ", Style::default().fg(Color::DarkGray)));
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    fn draw_status(&mut self, f: &mut Frame, area: Rect) {
        let line = match self.mode {
            Mode::Filter => Line::from(vec![
                Span::styled("/", Style::default().fg(Color::Cyan)),
                Span::raw(self.filter.clone()),
                Span::styled("▏", Style::default().fg(Color::Cyan)),
            ]),
            Mode::Copy => Line::from(vec![
                Span::styled("copy: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    "y key · Y full key · b bibcode · a ADS · x arXiv · d DOI · p PDF path · t title · Esc cancel",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Mode::Normal | Mode::Pick { .. } | Mode::Confirm { .. } if self.select_mode => Line::from(vec![
                Span::styled(
                    format!("◉ {} selected", self.selected.len()),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    "  ·  Space/click ◯ toggle · Esc done · ctrl+p actions".to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Mode::Normal | Mode::Pick { .. } | Mode::Confirm { .. } => {
                let n = self.filtered.len();
                let total = self.order.len();
                let filt = if self.filter.is_empty() {
                    String::new()
                } else {
                    format!("  ·  /{}", self.filter)
                };
                // a fresh confirmation ("Copied: …") outranks the hover
                // hint; once it ages out the hint takes over again
                let now = self.started.elapsed().as_secs();
                let fresh = self
                    .log
                    .last()
                    .filter(|(_, t, m)| *m == self.status && now.saturating_sub(*t) < 4);
                let (msg, msg_color) = if let Some((cat, _, m)) = fresh {
                    (m.clone(), cat.color())
                } else if let Some(hint) = &self.hover_hint {
                    (hint.clone(), Color::Cyan)
                } else {
                    match self.log.last() {
                        Some((cat, _, m)) if *m == self.status => {
                            (self.status.clone(), cat.color())
                        }
                        _ => (self.status.clone(), Color::Gray),
                    }
                };
                Line::from(vec![
                    Span::styled(
                        format!("{n}/{total}  ·  "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(msg, Style::default().fg(msg_color)),
                    Span::styled(filt, Style::default().fg(Color::DarkGray)),
                ])
            }
        };
        f.render_widget(line, area);
        self.draw_badges(f, area);
    }
}

/// Append a line to $ASTROBIB_DEBUG_LAYOUT (a file path) when set —
/// temporary instrumentation for layout/resize investigations.
fn debug_layout(line: &str) {
    if let Ok(path) = std::env::var("ASTROBIB_DEBUG_LAYOUT") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// System clipboard: pbcopy on macOS (reliable in any terminal), else
/// the OSC 52 escape (terminal-dependent, but works over SSH) — the
/// Python TUI's _copy_text strategy.
fn copy_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    if cfg!(target_os = "macos") {
        use std::process::{Command, Stdio};
        if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
            let wrote = child
                .stdin
                .take()
                .map(|mut s| s.write_all(text.as_bytes()).is_ok())
                .unwrap_or(false);
            if wrote && child.wait().is_ok_and(|s| s.success()) {
                return true;
            }
        }
    }
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes())).is_ok() && out.flush().is_ok()
}

/// Minimal RFC 4648 base64 for the OSC 52 payload (not worth a crate).
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn column_layout_priorities() {
        // wide: scaled author, Key visible
        assert_eq!(super::column_layout(150), (25, true));
        assert_eq!(super::column_layout(100), (16, true));
        // tight: Key drops first, author keeps its scaled width
        let (a, key) = super::column_layout(84);
        assert!(!key);
        assert!(a >= 14);
        // very tight: author sits at its floor, Key still gone
        assert_eq!(super::column_layout(55), (14, false));
        // Key never returns below the comfort threshold boundary
        let (_, key_90) = super::column_layout(90);
        assert!(key_90);
    }

    #[test]
    fn fit_authors_candidates() {
        let a3 = "{Zrake}, J. and {Clyburn}, M. and {Fearing}, S.";
        assert_eq!(super::fit_authors(a3, 40), "Zrake, Clyburn, and Fearing");
        assert_eq!(super::fit_authors(a3, 20), "Zrake, Clyburn, +1");
        assert_eq!(super::fit_authors(a3, 14), "Zrake, +2");
        assert_eq!(super::fit_authors(a3, 9), "Zrake, +2");
        assert_eq!(super::fit_authors("{Zrake}, J.", 20), "Zrake");
        assert_eq!(
            super::fit_authors("{Zrake}, J. and {MacFadyen}, A.", 30),
            "Zrake and MacFadyen"
        );
        let many = (0..13)
            .map(|i| format!("{{A{i}}}, X."))
            .collect::<Vec<_>>()
            .join(" and ");
        assert_eq!(super::fit_authors(&many, 14), "A0, A1, +11");
        assert_eq!(super::fit_authors(&many, 10), "A0, +12");
        assert_eq!(super::fit_authors("{Verylongsurname}, Q. and {B}, C.", 8), "Verylon…");
    }

    #[test]
    fn base64_rfc4648_vectors() {
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
            ("Zrake2019abcde", "WnJha2UyMDE5YWJjZGU="),
        ] {
            assert_eq!(super::base64(input.as_bytes()), want);
        }
    }
}

/// Responsive column plan for the table: (author width, show Key).
/// Degradation order: the Key column drops first (it is redundant with
/// the card footer) as soon as titles would fall below a comfortable
/// width; then the author column shrinks toward its floor; the title
/// keeps a hard minimum via its Min constraint.
fn column_layout(width: u16) -> (u16, bool) {
    const FIXED: u16 = 2 + 2 + 2 + 6; // gutter, ↓, ●, year
    const KEY_W: u16 = 20;
    const TITLE_COMFORT: u16 = 32; // drop Key before squeezing titles below this
    const TITLE_MIN: u16 = 20; // author shrinks to protect this
    let scaled = (width / 6).clamp(14, 30);
    let need_with_key = FIXED + scaled + KEY_W + TITLE_COMFORT + 7;
    if need_with_key <= width {
        return (scaled, true);
    }
    let mut author = scaled;
    while FIXED + author + TITLE_MIN + 6 > width && author > 14 {
        author -= 1;
    }
    (author, false)
}

/// Densest author description that fits `width`. Candidates from most
/// to least verbose — the full "A, B, and C" list, then "A, B, +N"
/// prefixes with a count, then "A et al." — and the first that fits
/// wins; a truncated surname is the last resort.
fn fit_authors(author: &str, width: usize) -> String {
    let surnames: Vec<String> = author
        .split(" and ")
        .map(|a| {
            a.trim()
                .split(',')
                .next()
                .unwrap_or("")
                .trim_matches(['{', '}'])
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect();
    let n = surnames.len();
    if n == 0 {
        return String::new();
    }
    let mut candidates: Vec<String> = vec![match n {
        1 => surnames[0].clone(),
        2 => format!("{} and {}", surnames[0], surnames[1]),
        _ => format!("{}, and {}", surnames[..n - 1].join(", "), surnames[n - 1]),
    }];
    for k in (1..n).rev() {
        candidates.push(format!("{}, +{}", surnames[..k].join(", "), n - k));
    }
    if n > 1 {
        candidates.push(format!("{} et al.", surnames[0]));
    }
    for c in &candidates {
        if c.chars().count() <= width {
            return c.clone();
        }
    }
    let mut s: String = surnames[0].chars().take(width.saturating_sub(1)).collect();
    s.push('…');
    s
}

/// "Zrake, J. and MacFadyen, A." → "Zrake, MacFadyen" (surnames, truncated).
fn format_authors(author: &str) -> String {
    let surnames: Vec<&str> = author
        .split(" and ")
        .map(|a| a.trim().split(',').next().unwrap_or("").trim_matches(['{', '}']))
        .collect();
    match surnames.len() {
        0 => String::new(),
        1..=4 => surnames.join(", "),
        _ => format!("{} + {} more", surnames[..3].join(", "), surnames.len() - 3),
    }
}
