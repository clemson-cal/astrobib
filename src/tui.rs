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
}

/// Every user action; the panel lists them all, dimming the unavailable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Select,
    Star,
    Manuscript,
    Download,
    OpenPdf,
    ClearPdf,
    BrowserDl,
    PickPdf,
    Remove,
    Filter,
    Card,
    Quit,
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
    // ctrl+p actions panel: every action listed, unavailable ones dimmed,
    // rows clickable (hit-tested via panel_rows rebuilt each draw)
    show_actions: bool,
    panel_rows: Vec<(u16, Action)>,
    panel_area: Rect,
    // pub card button and link rects, rebuilt each draw
    card_buttons: Vec<(Rect, CardBtn)>,
    card_links: Vec<(Rect, String)>,
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
            show_actions: true,
            panel_rows: vec![],
            panel_area: Rect::default(),
            card_buttons: vec![],
            card_links: vec![],
            pdf_status: String::new(),
            poll_cancel: None,
            pick_area: Rect::default(),
            confirm_btns: vec![],
        }
    }

    /// Availability policy: single-target actions dim under multi-selection,
    /// content-dependent actions dim when no target qualifies.
    fn available(&self, a: Action) -> bool {
        let keys = self.action_keys();
        let single = keys.len() == 1;
        let entry = |k: &String| self.lib.get(k);
        match a {
            Action::Select | Action::Filter | Action::Card | Action::Quit => true,
            Action::Star => keys.iter().any(|k| self.lib.in_personal(k)),
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
            Action::PickPdf => single,
            Action::Remove => !keys.is_empty(),
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
            Action::Star => self.toggle_star(),
            Action::Manuscript => self.toggle_manuscript(),
            Action::Download => self.download_pdfs(),
            Action::OpenPdf => self.open_pdfs(),
            Action::ClearPdf => self.clear_pdfs(),
            Action::BrowserDl => self.browser_download(),
            Action::PickPdf => self.open_picker(),
            Action::Remove => self.remove_papers(),
            Action::Filter => self.mode = Mode::Filter,
            Action::Card => self.show_detail = !self.show_detail,
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
    fn toggle_row_selected(&mut self, pos: usize) {
        let Some(&idx) = self.filtered.get(pos) else {
            return;
        };
        let key = self.order[idx].clone();
        if !self.selected.remove(&key) {
            self.selected.insert(key);
        }
        self.status = format!("{} selected", self.selected.len());
    }

    fn exit_select_mode(&mut self) {
        self.select_mode = false;
        self.selected.clear();
        self.status = format!("{} papers", self.order.len());
    }

    /// Rebuild the display order after entries were added or removed.
    fn rebuild_order(&mut self) {
        self.order = self.lib.entries().iter().map(|e| e.key().to_string()).collect();
        let lib = &self.lib;
        self.order.sort_by(|a, b| {
            let (ea, eb) = (lib.get(a).unwrap(), lib.get(b).unwrap());
            eb.year().cmp(&ea.year()).then(a.cmp(b))
        });
        self.selected.retain(|k| lib.get(k).is_some());
        self.refilter();
    }

    /// m — port of action_toggle_manuscript's library-view rule: if any
    /// target is missing from the manuscript db, add all missing; else
    /// (all present) remove all.
    fn toggle_manuscript(&mut self) {
        if self.lib.manuscript.is_none() {
            self.status = "no manuscript db (run inside a manuscript repo)".to_string();
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
            self.status = format!("◆ Added {n} paper(s) to manuscript db");
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
            self.status = format!("Removed {n} paper(s) from manuscript db{note}");
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
        self.status = format!("Removed {n} paper(s)");
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
            self.status = "no cached PDFs in selection  (p to download)".to_string();
            return;
        }
        let n = paths.len();
        pdf::open_paths(&paths);
        self.status = format!("Opened {n} PDF(s)");
    }

    /// X — cancel a running browser-download watch, else clear cached PDFs.
    fn clear_pdfs(&mut self) {
        if let Some(cancel) = self.poll_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.status = "browser download cancelled".to_string();
            return;
        }
        let mut n = 0;
        for k in self.action_keys() {
            let p = pdf::cache_path(&k);
            if p.exists() && std::fs::remove_file(&p).is_ok() {
                n += 1;
            }
        }
        self.status = format!("Cleared {n} cached PDF(s)");
    }

    /// Fetch one entry's PDF from a specific source (pub card buttons),
    /// on the download worker channel.
    fn download_single(&mut self, key: String, source: pdf::Source) {
        if self.dl_rx.is_some() {
            self.status = "a download is already running".to_string();
            return;
        }
        let Some(e) = self.lib.get(&key) else { return };
        let (eprint, adsurl) = (e.eprint().to_string(), e.adsurl().to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
        self.status = format!("Downloading {key}…");
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
            self.status = "a download is already running".to_string();
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
        self.status = format!("Resolving browser source for {key}…");
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

    fn open_picker(&mut self) {
        if let Some(k) = self.action_keys().into_iter().next() {
            self.open_picker_for(k);
        }
    }

    /// pick … — open the modal ~/Downloads PDF picker for one entry.
    fn open_picker_for(&mut self, key: String) {
        let files = pdf::downloads_pdfs();
        if files.is_empty() {
            self.status = "no PDFs in ~/Downloads".to_string();
            return;
        }
        self.mode = Mode::Pick { key, files, sel: 0 };
    }

    /// p — download PDFs for targets not yet cached, on a background
    /// thread so the UI stays live; progress arrives over a channel.
    fn download_pdfs(&mut self) {
        if self.dl_rx.is_some() {
            self.status = "a download is already running".to_string();
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
            self.status = "nothing to download (cached, or no arXiv ID / ADS URL)".to_string();
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
        self.status = format!("Downloading {total} PDF(s)…");
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
                    self.status = if failed.is_empty() {
                        format!("Downloaded {done} PDF(s)")
                    } else {
                        format!(
                            "Downloaded {done} PDF(s) — failed: {}{}",
                            failed[..failed.len().min(3)].join(", "),
                            if failed.len() > 3 { "…" } else { "" }
                        )
                    };
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
            MouseEventKind::Down(MouseButton::Left) => self.on_click(m.column, m.row),
            _ => {}
        }
    }

    fn on_click(&mut self, x: u16, y: u16) {
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
        // confirm modal: only its two buttons act; other clicks are inert
        if let Mode::Confirm { keys } = &self.mode {
            if let Some(&(_, is_confirm)) = self.confirm_btns.iter().find(|(r, _)| hit(*r, x, y)) {
                let keys = keys.clone();
                self.mode = Mode::Normal;
                if is_confirm {
                    self.remove_confirmed(&keys);
                } else {
                    self.status = "removal cancelled".to_string();
                }
            }
            return;
        }
        // pub card links open the browser
        if let Some((_, url)) = self.card_links.iter().find(|(r, _)| hit(*r, x, y)) {
            let url = url.clone();
            pdf::browser_open(&url);
            self.status = "opened in browser".to_string();
            return;
        }
        // actions panel rows
        if self.show_actions && hit(self.panel_area, x, y) {
            if let Some(&(_, action)) = self.panel_rows.iter().find(|(ry, _)| *ry == y) {
                self.run_action(action);
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
                        self.status = format!("Opened {key}");
                    }
                    CardBtn::Clear | CardBtn::Cancel => self.clear_card_pdf(&key),
                }
            }
            return;
        }
        // table: header at a.y, data rows below
        let a = self.table_area;
        if !hit(a, x, y) || y <= a.y {
            return;
        }
        let pos = self.table.offset() + (y - a.y - 1) as usize;
        if pos >= self.filtered.len() {
            return;
        }
        self.table.select(Some(pos));
        if x < a.x + 2 {
            // the ◯/◉ gutter: enter selection mode if needed, then toggle
            self.select_mode = true;
            self.toggle_row_selected(pos);
        }
    }

    /// Clear (or cancel a pending browser watch for) the card entry's PDF.
    fn clear_card_pdf(&mut self, key: &str) {
        if let Some(cancel) = self.poll_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.status = "browser download cancelled".to_string();
            return;
        }
        let p = pdf::cache_path(key);
        if p.exists() && std::fs::remove_file(&p).is_ok() {
            self.status = format!("Cleared cached PDF for {key}");
        }
    }

    /// Star/unstar: the whole selection in selection mode (any unstarred →
    /// star all, like the Python TUI), else the highlighted entry.
    fn toggle_star(&mut self) {
        if self.select_mode && !self.selected.is_empty() {
            let keys: Vec<String> = self
                .order
                .iter()
                .filter(|k| self.selected.contains(*k) && self.lib.personal.has(k))
                .cloned()
                .collect();
            if keys.is_empty() {
                self.status = "selection has no personal-library papers".to_string();
                return;
            }
            let target = keys
                .iter()
                .any(|k| !self.lib.get(k).map(|e| e.starred()).unwrap_or(false));
            let mut n = 0;
            for k in &keys {
                if self.lib.set_starred(k, target).is_ok() {
                    n += 1;
                }
            }
            self.status = format!(
                "{} {n} paper(s)",
                if target { "★ Starred" } else { "Unstarred" }
            );
            return;
        }
        let Some(key) = self.selected_key().map(str::to_string) else {
            return;
        };
        // stars are personal: entries not in the personal library can't star
        if !self.lib.personal.has(&key) {
            self.status = format!("{key} is not in the personal library");
            return;
        }
        let starred = self.lib.get(&key).map(|e| e.starred()).unwrap_or(false);
        match self.lib.set_starred(&key, !starred) {
            Ok(()) => {
                let e = self.lib.get(&key).unwrap();
                self.status = format!(
                    "{} {}",
                    if !starred { "★ Starred" } else { "Unstarred" },
                    if e.short_key.is_empty() { &key } else { &e.short_key }
                );
            }
            Err(err) => self.status = format!("star failed: {err}"),
        }
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('p') {
            self.show_actions = !self.show_actions;
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
            Mode::Confirm { keys } => match code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let keys = keys.clone();
                    self.mode = Mode::Normal;
                    self.remove_confirmed(&keys);
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.mode = Mode::Normal;
                    self.status = "removal cancelled".to_string();
                }
                _ => {}
            },
            Mode::Normal => match code {
                KeyCode::Char('q') => self.run_action(Action::Quit),
                KeyCode::Char('/') => self.run_action(Action::Filter),
                KeyCode::Char('s') => self.run_action(Action::Star),
                KeyCode::Char('m') => self.run_action(Action::Manuscript),
                KeyCode::Delete => self.run_action(Action::Remove),
                KeyCode::Char('p') => self.run_action(Action::Download),
                KeyCode::Char('o') => self.run_action(Action::OpenPdf),
                KeyCode::Char('X') => self.run_action(Action::ClearPdf),
                KeyCode::Char('B') => self.run_action(Action::BrowserDl),
                KeyCode::Char('D') | KeyCode::Char('z') => self.run_action(Action::Card),
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
                self.status = format!(
                    "Imported {} for {key}  ({kb} KB)",
                    file.file_name().unwrap_or_default().to_string_lossy()
                );
            }
            None => {
                self.status = format!(
                    "{} does not look like a PDF",
                    file.file_name().unwrap_or_default().to_string_lossy()
                );
            }
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        self.card_buttons.clear();
        self.panel_rows.clear();
        let [main, status] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());
        let mut constraints = vec![Constraint::Min(40)];
        if self.show_detail {
            constraints.push(Constraint::Length(48));
        }
        if self.show_actions {
            constraints.push(Constraint::Length(20));
        }
        let areas = Layout::horizontal(constraints).split(main);
        let mut it = areas.iter();
        let table_area = *it.next().unwrap();
        let detail_area = self.show_detail.then(|| *it.next().unwrap());
        let panel_area = self.show_actions.then(|| *it.next().unwrap());

        self.draw_table(f, table_area);
        if let Some(area) = detail_area {
            self.draw_detail(f, area);
        }
        if let Some(area) = panel_area {
            self.draw_panel(f, area);
        } else {
            self.panel_area = Rect::default();
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
        let remove_label = " remove ";
        let cancel_label = " cancel ";
        self.confirm_btns.push((
            Rect { x: bx, y: by, width: remove_label.len() as u16, height: 1 },
            true,
        ));
        self.confirm_btns.push((
            Rect {
                x: bx + remove_label.len() as u16 + 2,
                y: by,
                width: cancel_label.len() as u16,
                height: 1,
            },
            false,
        ));
        lines.push(Line::from(vec![
            Span::styled(remove_label, Style::default().bg(Color::Red).fg(Color::White)),
            Span::raw("  "),
            Span::styled(cancel_label, Style::default().bg(Color::DarkGray)),
        ]));
        let title = format!(
            " Remove {} paper(s) from the library? ",
            keys.len()
        );
        let p = Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title(title));
        f.render_widget(p, area);
    }

    /// The ctrl+p actions panel: every action, with key, label, and click
    /// target; unavailable actions render dimmed (the Python key panel's
    /// behavior).
    fn draw_panel(&mut self, f: &mut Frame, area: Rect) {
        self.panel_area = area;
        let entries: &[(&str, &str, Action)] = &[
            ("Spc", if self.select_mode { "sel. done (Esc)" } else { "select" }, Action::Select),
            ("s", "star ★", Action::Star),
            ("m", "manuscript ◆", Action::Manuscript),
            ("p", "download PDF", Action::Download),
            ("B", "browser DL", Action::BrowserDl),
            ("", "pick PDF…", Action::PickPdf),
            ("o", "open PDF", Action::OpenPdf),
            (
                "X",
                if self.poll_cancel.is_some() { "cancel DL" } else { "clear PDF" },
                Action::ClearPdf,
            ),
            ("Del", "remove…", Action::Remove),
            ("/", "filter", Action::Filter),
            ("D", "pub card", Action::Card),
            ("q", "quit", Action::Quit),
        ];
        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            "Actions",
            Style::default().add_modifier(Modifier::BOLD),
        ))];
        for (i, (key, label, action)) in entries.iter().enumerate() {
            let y = area.y + 1 + i as u16;
            let avail = self.available(*action);
            if avail && y < area.y + area.height {
                self.panel_rows.push((y, *action));
            }
            let (key_style, label_style) = if avail {
                (
                    Style::default().fg(Color::Cyan),
                    Style::default(),
                )
            } else {
                (
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                )
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{key:>3} "), key_style),
                Span::styled((*label).to_string(), label_style),
            ]));
        }
        let p = Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::LEFT)
                .padding(ratatui::widgets::Padding::horizontal(1)),
        );
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
            let style = if i == *sel {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
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
        self.table_area = area;
        let rows: Vec<Row> = self
            .filtered
            .iter()
            .map(|&i| {
                let e = self.lib.get(&self.order[i]).unwrap();
                let circle = if !self.select_mode {
                    ""
                } else if self.selected.contains(e.key()) {
                    "◉"
                } else {
                    "◯"
                };
                Row::new(vec![
                    circle.to_string(),
                    if has_cached_pdf(e.key()) { "↓" } else { "" }.to_string(),
                    if self.lib.in_manuscript(e.key()) { "●" } else { "" }.to_string(),
                    if e.starred() { "★" } else { "" }.to_string(),
                    e.year(),
                    e.first_author_last().trim_start_matches('{').to_string(),
                    e.title().trim_matches(['{', '}']).to_string(),
                    e.short_key.clone(),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(6),
                Constraint::Length(18),
                Constraint::Min(20),
                Constraint::Length(20),
            ],
        )
        .header(
            Row::new(vec!["", "↓", "●", "★", "Year", "Author", "Title", "Key"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::NONE));
        f.render_stateful_widget(table, area, &mut self.table);
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
        let Some(key) = self.selected_key().map(str::to_string) else {
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

        // ── body ─────────────────────────────────────────────────────
        for l in wrap_text(e.title().trim_matches(['{', '}']), w) {
            line_at(f, y, Line::from(Span::styled(l, Style::default().add_modifier(Modifier::BOLD))));
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
        let rest = 3 + 1 + 1 + kw_lines + 2;
        let abs = e.abstract_();
        if !abs.is_empty() && y + rest < bottom {
            y += 1;
            let shown: String = abs.chars().take(1000).collect();
            let avail = (bottom - y).saturating_sub(rest);
            for l in wrap_text(&shown, w).into_iter().take(avail as usize) {
                line_at(f, y, Line::from(l));
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
            spans.push(Span::styled(label, cyan));
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
        y += 1;

        // ── PDF buttons (Python labels, colors, and visibility rules) ─
        let cached = pdf::is_cached(&key);
        let muted = Style::default().fg(Color::Gray);
        let raised = Style::default().bg(Color::DarkGray);
        let mut buttons: Vec<(&str, CardBtn, Style)> = vec![];
        if !cached && !eprint.is_empty() {
            buttons.push((" arXiv ↓ ", CardBtn::Arxiv, raised.fg(Color::Cyan)));
        }
        if !cached && !adsurl.is_empty() {
            buttons.push((" ADS OA ↓ ", CardBtn::Oa, raised.fg(Color::Cyan)));
        }
        if !cached && (!doi.is_empty() || !adsurl.is_empty()) {
            buttons.push((" browser ↓ ", CardBtn::Browser, raised.fg(Color::Yellow)));
        }
        if !cached {
            buttons.push((" pick … ", CardBtn::Pick, raised.fg(Color::Magenta)));
        }
        if cached {
            buttons.push((" Open ↗ ", CardBtn::Open, raised.fg(Color::Green)));
            buttons.push((" Clear ✕ ", CardBtn::Clear, raised.patch(muted)));
        }
        let mut spans: Vec<Span> = vec![];
        let mut bx = x0;
        for (label, btn, style) in buttons {
            let wl = label.chars().count() as u16;
            if y < bottom {
                self.card_buttons.push((Rect { x: bx, y, width: wl, height: 1 }, btn));
            }
            spans.push(Span::styled(label.to_string(), style));
            spans.push(Span::raw(" "));
            bx += wl + 1;
        }
        line_at(f, y, Line::from(spans));
        y += 1;

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
        line_at(f, y, Line::from(spans));
    }


    fn draw_status(&self, f: &mut Frame, area: Rect) {
        let line = match self.mode {
            Mode::Filter => Line::from(vec![
                Span::styled("/", Style::default().fg(Color::Cyan)),
                Span::raw(self.filter.clone()),
                Span::styled("▏", Style::default().fg(Color::Cyan)),
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
                Line::from(Span::styled(
                    format!("{n}/{total}  ·  {}{filt}  ·  ctrl+p actions", self.status),
                    Style::default().fg(Color::DarkGray),
                ))
            }
        };
        f.render_widget(line, area);
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
