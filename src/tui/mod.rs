//! Ratatui TUI: library table with live filter and pub card.
//!
//! The current cut centers on browsing — instant startup, live filtering
//! with the full query language, manuscript ● indicators, a toggleable
//! pub card, ADS search and import, and instant quit.
//!
//! # Where things live
//!
//! There is one `App`, and its state has to be one struct: nearly every
//! keystroke reads across concerns, and splitting the state would only
//! turn field access into plumbing. What was splittable is the
//! behaviour, so `impl App` is spread over the modules below, each
//! holding one topic together with the types that exist only to serve
//! it. A method's home is what it is *about*, not which surface it
//! happens to draw on: the samples strip and the ADS-returns menu are in
//! `search` rather than `footer`, because they configure a query and
//! merely borrow the bottom line to do it.
//!
//! What stays here is what has no topic: the struct, the event loop, the
//! frame-level layout in `draw`, and the small chrome helpers (`hit`,
//! `pill_width`, `wrap_text`, `elide_left`) that every module uses.
//!
//! Cross-module calls are marked `pub(super)`, so each file's public
//! surface is the list of things other topics genuinely reach for; a
//! method used only by its own module stays private. `card`, `table` and
//! `theme` are the older, App-free widget modules and keep their own
//! interfaces.
//!
//! Adding a module means adding it to `SOURCES` in `tests/glyphs.rs`,
//! which is enforced there rather than remembered.

use crate::library::{has_cached_pdf, MergedLibrary};
use crate::pdf;
use crate::query::{self, QueryContext};
use crate::text::fit_authors;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Row, TableState};
use ratatui::Frame;
use std::collections::HashSet;
use std::time::Duration;

mod actions;
mod card;
mod card_view;
mod columns;
mod copy;
mod entries;
mod footer;
mod input;
mod manuscript;
mod metric;
mod mouse;
mod overlays;
mod pdfs;
mod rows;
mod scopes;
mod search;
mod table;
mod table_view;
mod tagging;
mod tasks;
mod theme;
mod watch;

use actions::{Action, HELP_ENTRIES, Press};
use card_view::{CardBtn, LinkTarget, TOGGLE_RESERVE, card_hint, draw_cited_line, draw_link_stack};
use columns::{COLUMNS_PANEL_W, Focus, PanelHit, col_width};
use copy::{COPY_CHORD, CopyItem, copy_hint, read_clipboard};
use entries::RemovalKind;
use manuscript::{MsRow, ms_state_rank};
use metric::{MetricCol, PriorityOp, metric_cell, metric_column};
use pdfs::{DlMsg, orphan_order};
use rows::load_sort;
use scopes::{FILTER_CHIP, QueryState, Scope, ScopeKind};
use search::{ADS_SORTS, AdsMsg, ads_sort_name};
use table::Col;
use table_view::{column_layout, format_authors, row_palette};
use tagging::tag_hint;
use tasks::{MsgCat, Task, TaskKind};
use theme::*;
use watch::Watch;

pub fn run(lib: MergedLibrary) -> anyhow::Result<()> {
    // before the alternate screen, while the terminal will still answer:
    // every surface colour is an offset from its background, and an
    // offset has to know which way it points
    theme::detect();
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    // without this a pasted query arrives as a burst of key events, and
    // an ADS search URL cannot be recognised as one thing
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let result = App::new(lib).run(&mut terminal);
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
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
    /// Confirm modal for removing papers (Delete key). It carries the
    /// decided plan, not just the keys: the modal states that plan in
    /// plain words and remove_confirmed executes exactly it.
    Confirm { plan: Vec<(String, RemovalKind)> },
    /// S — compose an ADS query; ↑/↓ steps the result limit, ⏎ runs it.
    /// S — compose an ADS query. `limit` and `sort` are properties of
    /// the query that are not query *syntax*: they ride alongside `q` as
    /// the `rows` and `sort` API parameters. The prompt shows them and
    /// steps them, and the text cursor never enters them, so it stays
    /// clear that typing into that region would not reach ADS.
    /// `edit` is the scope this replaces, or None to open a new query.
    /// Editing keeps the tab's identity — its id, its name, its display
    /// sort — and swaps everything the prompt owns: text, limit, and
    /// what ADS returns.
    AdsPrompt {
        input: tui_input::Input,
        limit: usize,
        sort: String,
        edit: Option<usize>,
    },
    // first-run setup: collect the ADS token, then the email, into
    // state.json; resume the query prompt afterwards when asked
    Setup { input: tui_input::Input, email: bool, resume: bool },
    // export the selection (or cursor entry) as a .bib file; the input
    // holds the destination path
    Export { input: tui_input::Input, keys: Vec<String> },
    /// N — rename the active query scope. The name replaces the one
    /// derived from the query text and outlives edits to it: a name you
    /// typed is a decision, and re-deriving would quietly discard it.
    Rename { input: tui_input::Input },
    /// T — name a tag to add to, or remove from, the papers in `keys`.
    /// `remove` is decided when the prompt opens, not when it closes:
    /// the ± reading is "every one of these already has it, so this is
    /// an untag", and the prompt says which way it will go before ⏎.
    /// It flips as you type, because the answer depends on the name.
    Tag { input: tui_input::Input, keys: Vec<String>, remove: bool },
    /// y pressed — the next key picks what to copy (the Copy panel tab
    /// shows the menu, which-key style); Esc cancels.
    Copy,
}

/// The sort marker for a direction. One definition, so the panel's
/// preview and the table header cannot disagree about which way is which.
fn arrow(asc: bool) -> &'static str {
    if asc {
        "▲"
    } else {
        "▼"
    }
}

/// The surface palette lives in `theme`, which reads the terminal's own
/// background once at startup: every one of these colours is defined as
/// an offset *from* that background, and an offset has a direction.
fn divider() -> Style {
    Style::default().fg(divider_fg())
}

/// Clamp-and-window text lines by the card scroll offset (stored back
/// clamped): the visible slice plus whether more exists above/below.
fn scroll_window(
    lines: Vec<String>,
    avail: usize,
    scroll: &mut usize,
) -> (Vec<String>, bool, bool) {
    let max = lines.len().saturating_sub(avail);
    *scroll = (*scroll).min(max);
    let above = *scroll > 0;
    let below = *scroll + avail < lines.len();
    (lines.iter().skip(*scroll).take(avail).cloned().collect(), above, below)
}

struct App {
    lib: MergedLibrary,
    order: Vec<String>,   // entry keys, year-descending
    filtered: Vec<usize>, // positions into `order` that pass the filter
    filter: tui_input::Input,
    mode: Mode,
    table: TableState,
    show_detail: bool,
    status: String,
    quit: bool,
    // iOS-style selection mode: circles appear, Space/click toggles rows,
    // Esc exits and clears; bulk actions apply to the selection
    select_mode: bool,
    selected: HashSet<String>,
    /// Click and wheel targets registered by the current frame only.
    hits: Hits,
    last_table_area: Rect, // persistent geometry used by table event handlers
    dl_rx: Option<std::sync::mpsc::Receiver<DlMsg>>,
    // ? keyboard cheat-sheet overlay; any key or click dismisses
    show_help: bool,
    /// the columns sidebar, a toggle view like the log and the keys sheet
    show_columns: bool,
    /// which side the arrow keys drive while the columns panel is open
    focus: Focus,
    col_sel: usize,
    /// the kind of the last coalescing note and where it landed in the
    /// log, so a repeat of the same control can replace it
    last_note: Option<(&'static str, usize)>,
    /// per scope kind: which columns are hidden and how wide the user
    /// pinned them. Absent means auto — see table::ColumnConfig.
    columns: std::collections::HashMap<ScopeKind, table::ColumnConfig>,
    // the @ about modal
    show_about: bool,
    upd_rx: Option<std::sync::mpsc::Receiver<String>>,
    // canonical-BibTeX previews for un-imported ADS articles, by bibcode
    bib_preview: std::collections::HashMap<String, String>,
    bib_rx: Option<std::sync::mpsc::Receiver<(String, String)>>,
    update_status: Option<String>,
    // pub card shows the raw .bib file instead of the formatted view
    show_bib_source: bool,
    // scroll offset for the card's abstract / bib text (wheel over the
    // card); reset when the shown paper changes
    card_scroll: usize,
    card_shown: Option<String>,
    metrics: crate::metrics::Metrics,
    metric_col: MetricCol,
    cit_rx: Option<std::sync::mpsc::Receiver<Vec<(String, i64)>>>,
    // copy-chord modal state
    // last known mouse position, for roll-over styling of clickables
    hover: (u16, u16),
    // transient footer hint while hovering a copy-region (never logged)
    hover_hint: Option<String>,
    // event log: (category, seconds-since-start, message); L toggles the
    // pane, the footer always shows the newest entry color-coded
    log: Vec<(MsgCat, u64, String)>,
    show_log: bool,
    // scrollback offset from the tail (0 = newest); PageUp/PageDown move
    // it while the pane is open, any new message snaps back to the tail
    log_scroll: usize,
    started: std::time::Instant,
    // table scopes: index 0 is always Library; ADS query results follow
    scopes: Vec<Scope>,
    active_scope: usize,
    /// Next sequence number for a query scope. Grouping the strip means
    /// sorting the scopes, and a sort needs a tiebreak that outlives the
    /// grouping — otherwise a query moved to the other home and back
    /// would come to rest at the end of its group rather than where it
    /// started, and H twice would not be the no-op it reads as.
    scope_seq: usize,
    ads_rx: Option<std::sync::mpsc::Receiver<AdsMsg>>,
    // table sort (clickable column headers)
    // Sort is per scope, not per app: each query tab keeps its own
    // (stored on the Tab, in tabs.json), and the library and manuscript
    // keep theirs in state.json. Switching scopes therefore leaves every
    // other scope's order alone — and, less obviously, rebuild_order can
    // sort the library by the *library's* column even when a query scope
    // is in front, which a single shared field could not.
    library_sort: (Col, bool), // (column, ascending)
    /// None while the manuscript is in scan order — the order the cites
    /// appear in the source, which is meaningful and is the default.
    ms_sort: Option<(Col, bool)>,
    // footer view badges: show/hide toggles per app-wide view
    /// Set by the card while it draws, read by the footer just after:
    /// `Some(showing the .bib source)` when a pub card is on screen and
    /// the card ⇄ bib toggler therefore belongs in the footer, None when
    /// there is no card to toggle. Cleared at the top of every frame.
    card_toggle: Option<bool>,
    // transient PDF status line shown on the card (waiting/result)
    pdf_status: String,
    // browser-download watcher cancel flag (X / clear cancels the poll)
    poll_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    // pending background tasks (the T overlay); the drain handlers
    // remove each row when its result arrives
    tasks: Vec<Task>,
    next_task_id: u64,
    // plain clicks on the same row within 400ms form a double-click
    last_click: Option<(std::time::Instant, usize, usize)>, // (t, scope, pos)
    // silent auto-reload: mtime snapshot of the manuscript sources, of
    // both tiers' bib/, and of every tag file, compared every ~1.5 s
    watch: Watch,
    watch_at: std::time::Instant,
    // the last tag report, so a check that runs every 1.5 s says the
    // same thing once rather than every poll
    tags_said: Vec<String>,
    // user-state stores whose last write failed: the latch behind
    // state_write, so an unwritable state dir reports once per store
    write_failed: HashSet<&'static str>,
    // what ^k killed, for ^y to yank back. One slot, not a ring: the
    // prompts are one line and there is nothing to cycle through.
    kill_ring: String,
    // ^r's menu of what ADS should return, open over the prompt. Not
    // part of the mode: it is a view of the mode's `sort`, and closing
    // it must never close the prompt underneath.
    sort_menu: bool,
    // the highlighted field, and whether the whole list is showing its
    // primary direction (newest / most / A→Z) or its reverse. Direction
    // is one axis for the list rather than a property of each row: it is
    // the same question whichever field you are on.
    sort_menu_sel: usize,
    sort_menu_primary: bool,
}

fn hit(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

#[derive(Clone, PartialEq, Eq)]
enum Target {
    AboutLink(String),
    AboutUpdate,
    CardButton(CardBtn),
    CardLink(String),
    CardTag(String),
    CardYank(CopyItem),
    Column(PanelHit),
    Confirm(bool),
    EditQuery,
    Footer(Action),
    Help(Press),
    Metric,
    PickRow(usize),
    PromptSort,
    Sample(&'static str),
    Scope(usize),
    ScopeClose(usize),
    SortHeader(Col),
    SortMenu(String),
    Card,
}

#[derive(Default)]
struct Hits(Vec<(Rect, Target)>);

impl Hits {
    fn add(&mut self, rect: Rect, target: Target) {
        self.0.push((rect, target));
    }

    fn at(&self, x: u16, y: u16) -> Option<&Target> {
        self.0
            .iter()
            .rev()
            .find(|(rect, _)| hit(*rect, x, y))
            .map(|(_, target)| target)
    }
}

/// The text rect of a tinted panel. Panels carry no borders — their tint
/// is the boundary — so the shape is: heading on the first row, content
/// under it, and the panel's last row left blank as the bottom inset.
/// A column of inset each side keeps text off the tint's edge, which is
/// where the border used to hold it.
fn panel_body(r: Rect) -> Rect {
    Rect { x: r.x + 1, y: r.y, width: r.width.saturating_sub(2), height: r.height }
}

/// Centered dim hint for an empty table (no results / empty library).
fn draw_empty_hint(f: &mut Frame, area: Rect, msg: &str) {
    if area.height == 0 {
        return;
    }
    // wrapped, not clipped: an ADS error carries the server's own words
    // and runs past the width, and the tail is where the reason and what
    // to do about it are — the part worth reading
    let w = area.width.saturating_sub(8).max(20) as usize;
    let lines = wrap_text(msg, w);
    let h = (lines.len() as u16).min(area.height);
    let y = area.y + (area.height / 3).min(area.height.saturating_sub(h));
    let text: Vec<Line> = lines
        .into_iter()
        .take(h as usize)
        .map(|l| {
            Line::from(Span::styled(l, Style::default().fg(Color::DarkGray)))
                .alignment(ratatui::layout::Alignment::Center)
        })
        .collect();
    f.render_widget(
        Paragraph::new(text),
        Rect { x: area.x, y, width: area.width, height: h },
    );
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

/// A chip with a ✕ before its closing cap, styled apart from the label
/// so it can light under the pointer: the same shape as `push_pill`, two
/// cells wider (the space and the mark). Both modes agree on width, so
/// the ✕ is at the same offset either way and its click rect holds.
fn push_closable_pill<'a>(
    spans: &mut Vec<Span<'a>>,
    label: &str,
    bg: Color,
    fg: Color,
    mark: Style,
) {
    let body = Style::default().bg(bg).fg(fg);
    if ascii_chips() {
        spans.push(Span::styled(" ".to_string(), body));
    } else {
        spans.push(Span::styled("\u{e0b6}".to_string(), Style::default().fg(bg)));
    }
    spans.push(Span::styled(format!("{label} "), body));
    spans.push(Span::styled("✕".to_string(), mark.bg(bg)));
    if ascii_chips() {
        spans.push(Span::styled(" ".to_string(), body));
    } else {
        spans.push(Span::styled("\u{e0b4}".to_string(), Style::default().fg(bg)));
    }
}

/// Greedy word wrap producing the exact lines we render — placement math
/// (links/buttons following variable-height text) depends on the count.
/// Shorten a path to fit, taking the cut off the *front*.
///
/// The usual `…` at the end is wrong for a path: every path in a
/// two-tier setup shares a long prefix with the others, so truncating
/// the tail is the one choice that reliably discards the part that says
/// which database this is.
fn elide_left(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(n + 1 - max).collect();
    format!("…{tail}")
}

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

impl App {
    fn new(lib: MergedLibrary) -> Self {
        let mut order: Vec<String> = lib.entries().iter().map(|e| e.key().to_string()).collect();
        order.sort_by(|a, b| match (lib.get(a), lib.get(b)) {
            (Some(ea), Some(eb)) => eb.year().cmp(&ea.year()).then(a.cmp(b)),
            // orphans cannot exist here (the order was just derived from
            // the library), but comparators never panic on principle —
            // see orphan_order
            (x, y) => orphan_order(x.is_some(), y.is_some(), a, b),
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
            filter: tui_input::Input::default(),
            mode: Mode::Normal,
            table,
            show_detail: true,
            status,
            quit: false,
            select_mode: false,
            selected: HashSet::new(),
            hits: Hits::default(),
            last_table_area: Rect::default(),
            dl_rx: None,
            show_help: false,
            show_columns: false,
            focus: Focus::Table,
            col_sel: 0,
            last_note: None,
            columns: App::load_column_config(),
            show_about: false,
            upd_rx: None,
            bib_preview: std::collections::HashMap::new(),
            bib_rx: None,
            update_status: None,
            show_bib_source: false,
            card_scroll: 0,
            card_shown: None,
            metrics: crate::metrics::Metrics::load(),
            metric_col: MetricCol::from_tag(
                &crate::ads::get_state_field("metric").unwrap_or_default(),
            ),
            cit_rx: None,
            hover: (u16::MAX, u16::MAX),
            hover_hint: None,
            log: vec![],
            show_log: false,
            log_scroll: 0,
            started: std::time::Instant::now(),
            scopes: vec![Scope::Library],
            active_scope: 0,
            scope_seq: 0,
            ads_rx: None,
            library_sort: load_sort("library_sort").unwrap_or((Col::Year, false)),
            ms_sort: load_sort("manuscript_sort"),
            card_toggle: None,
            pdf_status: String::new(),
            poll_cancel: None,
            tasks: vec![],
            next_task_id: 0,
            last_click: None,
            watch: Watch::default(),
            watch_at: std::time::Instant::now(),
            tags_said: vec![],
            write_failed: HashSet::new(),
            kill_ring: String::new(),
            sort_menu: false,
            sort_menu_sel: 0,
            sort_menu_primary: true,
        }
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
        // paint the library before any potentially blocking work: a
        // manuscript scan (cloud-evicted files can stall on read) or a
        // big query cache must never hold up the first frame
        terminal.draw(|f| self.draw(f))?;
        debug_layout(&format!("{:>6}ms first paint", t0.elapsed().as_millis()));
        self.rescan_manuscript();
        debug_layout(&format!("{:>6}ms rescan_manuscript done", t0.elapsed().as_millis()));
        // rescan_manuscript refreshes the snapshot, but only when there
        // is a manuscript; without this the first poll would find every
        // watched path "changed" and reload a library nothing touched
        self.watch = self.watch_snapshot();
        self.report_tags();
        self.restore_tabs();
        debug_layout(&format!("{:>6}ms restore_tabs done", t0.elapsed().as_millis()));
        while !self.quit {
            self.drain_downloads();
            self.drain_ads();
            self.drain_update();
            self.drain_bib_preview();
            self.drain_citations();
            self.poll_external();
            terminal.draw(|f| self.draw(f))?;
            if let Some(ev) = pending.take() {
                debug_layout(&format!("{:>6}ms pending {ev:?}", t0.elapsed().as_millis()));
                match ev {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.on_key(key.code, key.modifiers)
                    }
                    Event::Mouse(m) => self.on_mouse(m),
                    Event::Paste(s) => self.on_paste(s),
                    _ => {}
                }
                continue;
            }
            // fast cadence for the first second (late terminal resizes that
            // slip past the settle window) and while downloads report progress
            let tick = if t0.elapsed() < Duration::from_secs(1)
                || self.dl_rx.is_some()
                || self.ads_rx.is_some()
            {
                25
            } else {
                250
            };
            debug_layout(&format!(
                "{:>6}ms draw frame={:?} table={:?}",
                t0.elapsed().as_millis(),
                terminal.get_frame().area(),
                self.last_table_area,
            ));
            let had_events = event::poll(Duration::from_millis(tick))?;
            if !had_events {
                // idle: flush any dirty metrics (a no-op while clean)
                self.save_metrics();
            }
            if had_events {
                // Coalesce: handle every already-pending event before the
                // next draw. Mouse motion arrives faster than frames render
                // (each Moved event otherwise costs a full redraw), so
                // without this the hover highlight lags the pointer on
                // large scopes. Capped so a saturating stream cannot
                // starve drawing entirely.
                for _ in 0..64 {
                    let ev = event::read()?;
                    debug_layout(&format!("{:>6}ms event {ev:?}", t0.elapsed().as_millis()));
                    match ev {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            self.on_key(key.code, key.modifiers)
                        }
                        Event::Mouse(m) => self.on_mouse(m),
                        Event::Paste(s) => self.on_paste(s),
                        _ => {}
                    }
                    if self.quit || !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
        }
        self.save_metrics();
        Ok(())
    }

    fn draw(&mut self, f: &mut Frame) {
        self.hits = Hits::default();
        self.hover_hint = None;
        // the card claims the footer's toggler while it draws, below
        self.card_toggle = None;
        // the menu belongs to the query prompt: whichever way the prompt
        // went away, it goes with it, rather than each exit having to
        // remember to close it
        self.sort_menu &= matches!(self.mode, Mode::AdsPrompt { .. });
        // While a covering modal is up (about / pick / confirm — not the
        // non-modal keys panel or log), blind the surfaces beneath it to
        // the mouse: a position inside the modal also sits on rects behind
        // it, and their hover styling would show around the modal's edges.
        // The real position is restored before the modal itself draws so
        // its own links and buttons still react.
        let modal_up =
            self.show_about || matches!(self.mode, Mode::Pick { .. } | Mode::Confirm { .. });
        let real_hover = self.hover;
        if modal_up {
            self.hover = (u16::MAX, u16::MAX);
        }
        // The columns panel and the pub card run the full height; the
        // keys sheet and the event log belong to the *table*, so they
        // stack inside its column rather than across the whole frame.
        // Spanning everything meant opening the log shortened the card,
        // which has nothing to do with the log.
        let [body, status] = Layout::vertical([
            Constraint::Min(1),
            // the footer line — or several, when a query has outgrown one
            // and the prompt wraps to stay legible. No rule above it: the
            // footer is its own tinted surface and the tint separates it
            Constraint::Length(self.prompt_height(f.area().width)),
        ])
        .areas(f.area());
        let mut constraints = vec![];
        if self.show_columns {
            constraints.push(Constraint::Length(COLUMNS_PANEL_W));
        }
        constraints.push(Constraint::Min(40));
        if self.show_detail {
            constraints.push(Constraint::Length(48));
        }
        let areas = Layout::horizontal(constraints).split(body);
        let mut it = areas.iter();
        let columns_area = self.show_columns.then(|| *it.next().unwrap());
        let centre = *it.next().unwrap();
        let detail_area = self.show_detail.then(|| *it.next().unwrap());

        let log_h = if self.show_log {
            (self.log.len().min(8) + 2) as u16
        } else {
            0
        };
        // the sheet wraps into as many columns as the table's width
        // allows, so its height follows that width, not the frame's
        let help_h = if self.show_help {
            Self::help_height(centre.width)
        } else {
            0
        };
        // last in the stack, so the samples sit against the footer they
        // are helping you fill; they take only what the table can spare
        let strip_h = self.scope_strip_height(centre.width);
        let spare = centre.height.saturating_sub(strip_h + help_h + log_h);
        // the menu and the samples want the same slot; the menu is what
        // you just asked for, so it wins while it is open
        let samples_h = if self.sort_menu {
            self.sort_menu_height(spare, centre.width)
        } else {
            self.samples_height(spare, centre.width)
        };
        let [strip_area, table_area, help_area, log_area, samples_area] = Layout::vertical([
            Constraint::Length(strip_h),
            Constraint::Min(1),
            Constraint::Length(help_h),
            Constraint::Length(log_h),
            Constraint::Length(samples_h),
        ])
        .areas(centre);
        self.draw_scope_strip(f, strip_area);
        self.draw_table(f, table_area);
        if let Some(area) = columns_area {
            // after the table: the panel reads the solved column widths
            self.draw_columns_panel(f, area);
        }
        if let Some(area) = detail_area {
            self.draw_detail(f, area);
        }
        if self.show_help {
            self.draw_help(f, help_area);
        }
        if self.show_log {
            self.draw_log(f, log_area);
        }
        if self.sort_menu {
            self.draw_sort_menu(f, samples_area);
        } else {
            self.draw_samples(f, samples_area);
        }
        self.draw_status(f, status);
        // Modals draw last (topmost) with the real mouse position back in
        // place so their own hover styling works.
        self.hover = real_hover;
        if self.show_about {
            self.draw_about(f);
        }
        if let Mode::Pick { .. } = self.mode {
            self.draw_picker(f);
        }
        if let Mode::Confirm { .. } = self.mode {
            self.draw_confirm(f);
        }
    }

}

/// Append a line to $ASTROBIB_DEBUG_LAYOUT (a file path) when set —
/// temporary instrumentation for layout/resize investigations.
/// Startup-phase timing, same sink as the layout log.
pub fn debug_startup(line: &str) {
    debug_layout(line);
}

fn debug_layout(line: &str) {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    // The file opens once and stays open: a create+append+close per line
    // costs ~5-30ms on some macOS setups, which swamps the very timings
    // this instrumentation exists to capture.
    static LOG: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    let log = LOG.get_or_init(|| {
        let path = std::env::var("ASTROBIB_DEBUG_LAYOUT").ok()?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new)
    });
    if let Some(f) = log {
        let _ = writeln!(f.lock().unwrap(), "{line}");
    }
}
