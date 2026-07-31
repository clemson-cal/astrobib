//! Ratatui TUI: library table with live filter, pub card, star toggle.
//!
//! The current cut centers on browsing — instant startup, live filtering
//! with the full query language, manuscript ● and star ★ indicators, a
//! toggleable pub card, star toggling, and instant quit.

use crate::library::{has_cached_pdf, MergedLibrary};
use crate::pdf;
use crate::query::{self, QueryContext};
use crate::text::fit_authors;
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
    /// S — compose an ADS query; ↑/↓ steps the result limit, ⏎ runs it.
    AdsPrompt { input: tui_input::Input, limit: usize },
    // first-run setup: collect the ADS token, then the email, into
    // state.json; resume the query prompt afterwards when asked
    Setup { input: tui_input::Input, email: bool, resume: bool },
    // export the selection (or cursor entry) as a .bib file; the input
    // holds the destination path
    Export { input: tui_input::Input, keys: Vec<String> },
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
    GlobalTier,
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
/// and bibcodes join with ", " under multi-selection (comma lists paste
/// straight into \cite{...}); URLs, paths, and titles join with newlines.
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

/// A data source for the table: the library, or one ADS query's
/// results. One table widget, one interaction path — the scope only
/// decides rows and columns.
enum Scope {
    Library,
    /// Cited keys from the manuscript's .tex files, classified.
    Manuscript { rows: Vec<MsRow> },
    Ads {
        tab: crate::tabs::Tab,
        articles: Vec<crate::ads::Article>,
    },
}

/// One manuscript-view row: a cited string (or an uncited db member).
struct MsRow {
    cited: String,
    state: crate::library::CiteState,
    uncited: bool,
    key: Option<String>,
    title: String,
}

/// Mtime snapshot backing the manuscript auto-rescan: every scanned
/// source file paired with its mtime, plus the bib/ directory's mtime.
type MsWatch = (
    Vec<(std::path::PathBuf, std::time::SystemTime)>,
    Option<std::time::SystemTime>,
);

impl Scope {
    fn label(&self) -> &str {
        match self {
            Scope::Library => "Library",
            Scope::Manuscript { .. } => "Manuscript",
            Scope::Ads { tab, .. } => &tab.label,
        }
    }
}

enum AdsMsg {
    Done {
        id: u64,
        tab: crate::tabs::Tab,
        refresh_of: Option<usize>,
        result: Result<Vec<crate::ads::Article>, String>,
    },
    Imported {
        id: u64,
        bibcode: String,
        result: Result<crate::bib::Data, String>,
    },
}

/// In-flight background work, listed in the T overlay. Worker threads
/// cannot be killed, so cancelling a thread-backed task only marks it;
/// the drain handler discards its result on arrival. The browser
/// watcher is the exception: it cancels for real via poll_cancel.
#[derive(Clone, Copy)]
enum TaskKind {
    Download,
    Query,
    Import,
    Watch,
}

struct Task {
    id: u64,
    label: String,
    kind: TaskKind,
    cancelled: bool,
    /// cache keys the task may write; discarding a cancelled download
    /// removes them again, restoring the failed-download end state
    keys: Vec<String>,
}

/// Sortable table columns; clicking a header toggles direction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortCol {
    Metric,
    Pdf,
    InLib,
    Year,
    Author,
    Title,
    Key,
}

/// Every divider in the app — the card's horizontal rules and the
/// vertical column divider, the table header rule, and the log/footer
/// rules — uses this one quite-dim color.
const DIVIDER_FG: Color = Color::Rgb(62, 66, 74);

fn divider() -> Style {
    Style::default().fg(DIVIDER_FG)
}

/// Unhovered table text (author, title) and the card's abstract body —
/// modestly dimmer than the terminal foreground.
const TABLE_TEXT: Color = Color::Rgb(150, 155, 163);
const ABSTRACT_TEXT: Color = Color::Rgb(170, 174, 182);

/// The one-cell metric swatch column: one scalar per paper, colored
/// by a per-metric colormap so the hue family names the metric.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MetricCol {
    Off,
    Priority,  // viridis — user-curated 0..1 level, decaying over time
    Citations, // magma — ADS citation count
}

impl MetricCol {
    fn next(self) -> Self {
        match self {
            MetricCol::Off => MetricCol::Priority,
            MetricCol::Priority => MetricCol::Citations,
            MetricCol::Citations => MetricCol::Off,
        }
    }
    fn name(self) -> &'static str {
        match self {
            MetricCol::Off => "off",
            MetricCol::Priority => "priority (viridis)",
            MetricCol::Citations => "citations (magma)",
        }
    }
    fn state_tag(self) -> &'static str {
        match self {
            MetricCol::Off => "off",
            MetricCol::Priority => "priority",
            MetricCol::Citations => "citations",
        }
    }
    fn from_tag(s: &str) -> Self {
        match s {
            "priority" => MetricCol::Priority,
            "citations" => MetricCol::Citations,
            _ => MetricCol::Off,
        }
    }
}

/// The clickable "cited by N" card line: tapping refreshes the count
/// from ADS.
#[allow(clippy::too_many_arguments)]
fn draw_cited_line(
    f: &mut Frame,
    x0: u16,
    y: u16,
    w: u16,
    n: Option<i64>,
    hover: (u16, u16),
    card_buttons: &mut Vec<(Rect, CardBtn)>,
    hint: &mut Option<String>,
) {
    let label = match n {
        Some(n) => format!("cited by {n}"),
        None => "cited by ?".to_string(),
    };
    let r = Rect { x: x0, y, width: label.chars().count() as u16 + 2, height: 1 };
    card_buttons.push((r, CardBtn::RefreshCites));
    let hov = hit(r, hover.0, hover.1);
    if hov {
        *hint = Some(card_hint(CardBtn::RefreshCites).to_string());
    }
    let style = if hov {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(label, style),
            Span::styled(" ⟳", Style::default().fg(Color::DarkGray)),
        ])),
        Rect { x: x0, y, width: w, height: 1 },
    );
}

/// A priority edit: set outright or nudge the effective level.
#[derive(Clone, Copy)]
enum PriorityOp {
    Set(f64),
    Scale(f64),
}

/// Map a normalized [0,1] value through the metric's colormap.
fn metric_color(metric: MetricCol, t: f64) -> Color {
    let g = match metric {
        MetricCol::Priority => colorous::VIRIDIS,
        _ => colorous::MAGMA,
    };
    let c = g.eval_continuous(t.clamp(0.0, 1.0));
    Color::Rgb(c.r, c.g, c.b)
}

/// Rank-normalize a row's value against the visible set: robust to
/// outliers, and every colormap stop gets used.
fn rank_norm(vals: &[f64], v: f64) -> f64 {
    if vals.len() < 2 {
        return 0.5;
    }
    let below = vals.iter().filter(|x| **x < v).count();
    below as f64 / (vals.len() - 1) as f64
}

/// Sentinel scope-strip index for the active-filter chip ("+ new" is
/// usize::MAX).
const FILTER_CHIP: usize = usize::MAX - 1;

/// The keys panel's entries and fixed column width.
const HELP_COLW: u16 = 30;
// (shown key, label, availability probe, key a click synthesizes)
const HELP_ENTRIES: &[(&str, &str, Option<Action>, KeyCode)] = &[
    ("␣", "select / toggle row", Some(Action::Select), KeyCode::Char(' ')),
    ("a", "select visible", Some(Action::Select), KeyCode::Char('a')),
    ("A", "select all", Some(Action::Select), KeyCode::Char('A')),
    ("j k", "move cursor", None, KeyCode::Char('j')),
    ("g G", "first / last row", None, KeyCode::Char('g')),
    ("m", "manuscript ± (selection)", Some(Action::Manuscript), KeyCode::Char('m')),
    ("p", "download PDF", Some(Action::Download), KeyCode::Char('p')),
    ("B", "browser download", Some(Action::BrowserDl), KeyCode::Char('B')),
    ("o", "open PDF", Some(Action::OpenPdf), KeyCode::Char('o')),
    ("X", "clear PDF / cancel DL", Some(Action::ClearPdf), KeyCode::Char('X')),
    ("y", "copy…", Some(Action::Copy), KeyCode::Char('y')),
    ("⌫", "remove…", Some(Action::Remove), KeyCode::Delete),
    ("/", "filter", Some(Action::Filter), KeyCode::Char('/')),
    ("D", "pub card", Some(Action::Card), KeyCode::Char('D')),
    ("L", "event log", Some(Action::Log), KeyCode::Char('L')),
    ("t", "global tier", Some(Action::GlobalTier), KeyCode::Char('t')),
    ("v", "pub view", None, KeyCode::Char('v')),
    ("e", "export selection…", None, KeyCode::Char('e')),
    ("M", "metric column", None, KeyCode::Char('M')),
    (".", "priority → 1.0", None, KeyCode::Char('.')),
    ("0", "priority → 0", None, KeyCode::Char('0')),
    ("< >", "priority down / up", None, KeyCode::Char('>')),
    ("@", "about", None, KeyCode::Char('@')),
    ("C", "citations", None, KeyCode::Char('C')),
    ("R", "references", None, KeyCode::Char('R')),
    ("?", "this cheat-sheet", Some(Action::Help), KeyCode::Char('?')),
    ("q", "quit", Some(Action::Quit), KeyCode::Char('q')),
];

/// One row of the card's link stack: ↗ rows open the browser, ⌕ rows
/// act inside astrobib (query scopes).
enum LinkTarget {
    Url(String),
    Query(CardBtn),
    Copy(CopyItem),
}

/// Footer hint for a ⧉ copy row: what is copied, and the y-chord.
fn copy_hint(item: CopyItem) -> &'static str {
    match item {
        CopyItem::Key => "⧉ copy the cite key  ·  y y",
        CopyItem::FullKey => "⧉ copy the full key  ·  y Y",
        CopyItem::Bibcode => "⧉ copy the bibcode  ·  y b",
        CopyItem::AdsUrl => "⧉ copy the ADS URL  ·  y a",
        CopyItem::ArxivUrl => "⧉ copy the arXiv URL  ·  y x",
        CopyItem::DoiUrl => "⧉ copy the DOI URL  ·  y d",
        CopyItem::PdfPath => "⧉ copy the cached PDF's path  ·  y p",
        CopyItem::Title => "⧉ copy the title  ·  y t",
        CopyItem::Abstract => "⧉ copy the abstract  ·  y A",
    }
}

/// Footer hint for a card affordance: what happens, and the key.
fn card_hint(btn: CardBtn) -> &'static str {
    match btn {
        CardBtn::Arxiv => "↓ fetch the PDF from arXiv  ·  p",
        CardBtn::Oa => "↓ fetch the open-access PDF via ADS  ·  p",
        CardBtn::Browser => "↓ download via the browser, watching ~/Downloads  ·  B",
        CardBtn::Pick => "⤷ import a PDF from the filesystem",
        CardBtn::Open => "↗ open the cached PDF  ·  o",
        CardBtn::Clear => "✕ remove the cached PDF  ·  X",
        CardBtn::Cancel => "✕ stop watching for the download",
        CardBtn::MsToggle => "◆ add to / remove from the manuscript db  ·  m",
        CardBtn::Import => "→ import into the library  ·  i",
        CardBtn::BibView => "@ show the .bib entry verbatim  ·  v",
        CardBtn::RefreshCites => "⟳ refresh the citation count from ADS",
        CardBtn::RemoveFromLib => "✕ remove from the library  ·  ⌫",
        CardBtn::Citations => "⌕ new query: papers citing this one  ·  C",
        CardBtn::Refs => "⌕ new query: papers this one cites  ·  R",
    }
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

/// Render the card's action block as two columns — links and query
/// actions on the left, the permanent ⧉ copy menu on the right — split
/// by a dim vertical divider (omitted when either column is empty).
/// Badges name each row's kind: ↗ opens the browser, ⌕ acts inside
/// astrobib, ⧉ copies. Registers whole-row click rects and returns the
/// y below the block.
#[allow(clippy::too_many_arguments)]
fn draw_link_stack(
    f: &mut Frame,
    x0: u16,
    y: u16,
    w: u16,
    bottom: u16,
    hover: (u16, u16),
    left: Vec<(String, LinkTarget, bool)>,
    right: Vec<(String, LinkTarget, bool)>,
    card_links: &mut Vec<(Rect, String)>,
    card_buttons: &mut Vec<(Rect, CardBtn)>,
    hint: &mut Option<String>,
    yanks: &mut Vec<(Rect, CopyItem)>,
) -> u16 {
    let cyan = Style::default().fg(Color::Cyan);
    let dim = Style::default().fg(Color::DarkGray);
    let two_col = !left.is_empty() && !right.is_empty();
    // left column width: its longest row (badge + space + label), capped
    let left_w = if two_col {
        left.iter()
            .map(|(l, _, _)| l.chars().count() as u16 + 2)
            .max()
            .unwrap_or(0)
            .min(w / 2)
    } else {
        0
    };
    let rows = left.len().max(right.len());
    let mut render =
        |f: &mut Frame, item: Option<(String, LinkTarget, bool)>, x: u16, colw: u16, ry: u16| {
            let Some((label, target, enabled)) = item else { return };
            let badge = match &target {
                LinkTarget::Url(_) => "↗",
                LinkTarget::Query(_) => "⌕",
                LinkTarget::Copy(_) => "⧉",
            };
            let shown: String = label.chars().take(colw.saturating_sub(2) as usize).collect();
            let wl = (badge.chars().count() + 1 + shown.chars().count()) as u16;
            let r = Rect { x, y: ry, width: wl, height: 1 };
            let hov = hit(r, hover.0, hover.1);
            if hov {
                let base = match &target {
                    LinkTarget::Url(_) => format!("↗ open {label} in the browser"),
                    LinkTarget::Query(btn) => card_hint(*btn).to_string(),
                    LinkTarget::Copy(item) => copy_hint(*item).to_string(),
                };
                *hint = Some(if enabled {
                    base
                } else {
                    format!("{}  —  not available here", base.split("  ·  ").next().unwrap_or(&base))
                });
            }
            if enabled {
                match target {
                    LinkTarget::Url(url) => card_links.push((r, url)),
                    LinkTarget::Query(btn) => card_buttons.push((r, btn)),
                    LinkTarget::Copy(item) => yanks.push((r, item)),
                }
            }
            let (badge_style, style) = if !enabled {
                // dim cyan stays visible on themes where doubly-dimmed
                // gray sinks into the background, and reads as the
                // inactive form of the active color
                let d = cyan.add_modifier(Modifier::DIM);
                (d, d)
            } else if hov {
                (dim, cyan.add_modifier(Modifier::UNDERLINED))
            } else {
                (dim, cyan)
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(badge, badge_style),
                    Span::raw(" "),
                    Span::styled(shown, style),
                ])),
                Rect { x, y: ry, width: colw, height: 1 },
            );
        };
    let mut left = left.into_iter();
    let mut right = right.into_iter();
    for i in 0..rows {
        let ry = y + i as u16;
        if ry >= bottom {
            break;
        }
        if two_col {
            render(f, left.next(), x0, left_w, ry);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled("│", divider()))),
                Rect { x: x0 + left_w + 1, y: ry, width: 1, height: 1 },
            );
            render(f, right.next(), x0 + left_w + 3, w.saturating_sub(left_w + 3), ry);
        } else {
            render(f, left.next().or_else(|| right.next()), x0, w, ry);
        }
    }
    y + rows as u16
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
    Import,
    // show the .bib file verbatim in the card
    BibView,
    // refresh the shown paper's citation count from ADS
    RefreshCites,
    // query card: remove the imported twin from the library
    RemoveFromLib,
    // citation-graph navigation: spawn citations(...)/references(...)
    // query scopes for the card's bibcode (resolved at dispatch time)
    Citations,
    Refs,
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
    table_area: Rect, // last drawn table region, for mouse hit-testing
    dl_rx: Option<std::sync::mpsc::Receiver<DlMsg>>,
    // ? keyboard cheat-sheet overlay; any key or click dismisses
    show_help: bool,
    // the @ about modal: clickable links, the update-check button
    show_about: bool,
    about_links: Vec<(Rect, String)>,
    about_btn: Rect,
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
    card_area: Rect,
    card_shown: Option<String>,
    metric_area: Rect,
    metrics: crate::metrics::Metrics,
    metric_col: MetricCol,
    cit_rx: Option<std::sync::mpsc::Receiver<Vec<(String, i64)>>>,
    // copy-chord modal region and its clickable rows
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
    scope_rects: Vec<(Rect, usize)>,
    help_rects: Vec<(Rect, KeyCode)>, // keys-panel rows → synthesized key
    ads_rx: Option<std::sync::mpsc::Receiver<AdsMsg>>,
    // table sort (clickable column headers) and their header hit rects
    sort: (SortCol, bool), // (column, ascending)
    sort_headers: Vec<(Rect, SortCol)>,
    // footer view badges: clickable show/hide toggles per app-wide view
    footer_badges: Vec<(Rect, Action)>,
    // pub card button and link rects, rebuilt each draw
    card_buttons: Vec<(Rect, CardBtn)>,
    card_links: Vec<(Rect, String)>,
    card_yanks: Vec<(Rect, CopyItem)>,
    // transient PDF status line shown on the card (waiting/result)
    pdf_status: String,
    // browser-download watcher cancel flag (X / clear cancels the poll)
    poll_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    // pending background tasks (the T overlay); the drain handlers
    // remove each row when its result arrives
    tasks: Vec<Task>,
    next_task_id: u64,
    pick_area: Rect,
    confirm_btns: Vec<(Rect, bool)>, // (rect, is_confirm)
    // plain clicks on the same row within 400ms form a double-click
    last_click: Option<(std::time::Instant, usize, usize)>, // (t, scope, pos)
    // silent manuscript auto-rescan: mtime snapshot of the scanned
    // sources and bib/, compared every ~1.5 s
    ms_watch: MsWatch,
    ms_watch_at: std::time::Instant,
}

/// option/alt+arrow (and emacs alt+b/f) word motions for text inputs.
/// tui-input's crossterm backend only maps ctrl+arrow and meta+b/f, and
/// macOS terminals report option as ALT — so these arrive unmapped.
fn word_motion(code: KeyCode, mods: KeyModifiers) -> Option<tui_input::InputRequest> {
    if !mods.contains(KeyModifiers::ALT) {
        return None;
    }
    match code {
        KeyCode::Left | KeyCode::Char('b') => Some(tui_input::InputRequest::GoToPrevWord),
        KeyCode::Right | KeyCode::Char('f') => Some(tui_input::InputRequest::GoToNextWord),
        _ => None,
    }
}

fn hit(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

/// Centered dim hint for an empty table (no results / empty library).
fn draw_empty_hint(f: &mut Frame, area: Rect, msg: &str) {
    if area.height == 0 {
        return;
    }
    let y = area.y + (area.height / 3).min(area.height - 1);
    let line = Line::from(Span::styled(msg, Style::default().fg(Color::DarkGray)))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(
        Paragraph::new(line),
        Rect { x: area.x, y, width: area.width, height: 1 },
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
    Done { id: u64, done: usize, failed: Vec<String> },
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
            filter: tui_input::Input::default(),
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
            show_about: false,
            about_links: vec![],
            about_btn: Rect::default(),
            upd_rx: None,
            bib_preview: std::collections::HashMap::new(),
            bib_rx: None,
            update_status: None,
            show_bib_source: false,
            card_scroll: 0,
            card_area: Rect::default(),
            card_shown: None,
            metric_area: Rect::default(),
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
            scope_rects: vec![],
            help_rects: vec![],
            ads_rx: None,
            sort: (SortCol::Year, false),
            sort_headers: vec![],
            footer_badges: vec![],
            card_buttons: vec![],
            card_links: vec![],
            card_yanks: vec![],
            pdf_status: String::new(),
            poll_cancel: None,
            tasks: vec![],
            next_task_id: 0,
            pick_area: Rect::default(),
            confirm_btns: vec![],
            last_click: None,
            ms_watch: (vec![], None),
            ms_watch_at: std::time::Instant::now(),
        }
    }

    /// The PDF-cache key for a table row, per scope (cite key when the
    /// paper is in the library, bibcode for unimported ADS rows).
    /// The cache key of a row — always a library cite key. An
    /// un-imported query result has none: PDFs are never cached under
    /// a bibcode, so there is nothing to open or store for it.
    fn row_cache_key(&self, pos: usize) -> Option<String> {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => articles
                .get(pos)
                .and_then(|a| self.lib.get_by_bibcode(&a.bibcode))
                .map(|e| e.key().to_string()),
            Some(Scope::Manuscript { rows }) => rows.get(pos).and_then(|r| r.key.clone()),
            _ => self.filtered.get(pos).map(|&i| self.order[i].clone()),
        }
    }

    fn active_ads(&self) -> Option<&Scope> {
        match self.scopes.get(self.active_scope) {
            Some(s @ Scope::Ads { .. }) => Some(s),
            _ => None,
        }
    }

    fn set_scope(&mut self, idx: usize) {
        self.active_scope = idx.min(self.scopes.len().saturating_sub(1));
        self.table.select(Some(0));
        *self.table.offset_mut() = 0;
        self.pdf_status.clear();
        if self.select_mode {
            self.exit_select_mode();
        }
    }

    fn cycle_scope(&mut self, d: isize) {
        let n = self.scopes.len() as isize;
        let cur = self.active_scope as isize;
        self.set_scope(((cur + d).rem_euclid(n)) as usize);
    }

    fn close_scope(&mut self) {
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

    /// S — compose an ADS query. Pre-filled from the active local filter
    /// via to_ads_query (filter locally, escalate in one keystroke).
    fn open_ads_prompt(&mut self) {
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
        self.mode = Mode::AdsPrompt { input: tui_input::Input::from(input), limit: 20 };
    }

    /// Run a query on a worker thread into a scope. A pasted DOI or ADS
    /// abstract URL short-circuits: DOI becomes a fielded query, an ADS
    /// URL imports the paper directly.
    fn run_ads_query_limit(&mut self, raw: String, refresh_of: Option<usize>, limit: usize) {
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
                t
            }
            _ => crate::tabs::make_tab(&query, limit),
        };
        // replacing the channel orphans any in-flight ADS work: its
        // receiver drops, so those tasks can never report back
        self.tasks
            .retain(|t| !matches!(t.kind, TaskKind::Query | TaskKind::Import));
        let id = self.add_task(TaskKind::Query, format!("⌕ ADS query — '{query}'"), vec![]);
        let (tx, rx) = std::sync::mpsc::channel();
        self.ads_rx = Some(rx);
        self.note(MsgCat::Info, format!("Searching ADS: {query}"));
        std::thread::spawn(move || {
            let result = crate::ads::search(&query, limit).map_err(|e| e.to_string());
            let _ = tx.send(AdsMsg::Done { id, tab, refresh_of, result });
        });
    }

    /// The manuscript view's row set: scan .tex and .md sources and
    /// classify every cited key, then append uncited manuscript-db
    /// members. Markdown wikilinks count only when they resolve (an
    /// unresolved [[link]] is an ordinary note link, not a citation);
    /// pandoc @cites always count and surface as missing.
    fn ms_rows(&self) -> Vec<MsRow> {
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

    fn rescan_manuscript(&mut self) {
        if self.lib.manuscript.is_none() {
            return;
        }
        let rows = self.ms_rows();
        self.sync_refs_bib(&rows);
        match self.scopes.get_mut(1) {
            Some(s @ Scope::Manuscript { .. }) => *s = Scope::Manuscript { rows },
            _ => self.scopes.insert(1, Scope::Manuscript { rows }),
        }
        self.ms_watch = self.ms_watch_snapshot();
    }

    /// Mtime snapshot of everything the manuscript scan depends on:
    /// every .tex/.md source (\input and embed expansion included) plus
    /// the bib/ directory itself. refs.bib lives in the manuscript root
    /// and is never a scanned source, so regenerating it cannot
    /// re-trigger the watcher.
    fn ms_watch_snapshot(&self) -> MsWatch {
        let Some(root) = self.ms_root() else { return (vec![], None) };
        let mut files = crate::export::manuscript_tex_files(&root);
        files.extend(crate::export::manuscript_md_files(&root));
        let srcs = files
            .into_iter()
            .filter_map(|f| {
                let m = std::fs::metadata(&f).and_then(|m| m.modified()).ok()?;
                Some((f, m))
            })
            .collect();
        let bib = std::fs::metadata(root.join("bib"))
            .and_then(|m| m.modified())
            .ok();
        (srcs, bib)
    }

    /// Silent auto-rescan on external changes.
    /// Every ~1.5 s compare the mtime snapshot:
    /// edited sources rescan the manuscript (refs.bib regenerates along
    /// the way); a changed bib/ reloads the library tiers from disk.
    /// Our own writes touch bib/ too, but every mutation path ends in
    /// rescan_manuscript or rebuild_order, which refresh the snapshot —
    /// and a redundant reload would be harmless anyway.
    fn poll_manuscript(&mut self) {
        if self.lib.manuscript.is_none()
            || self.ms_watch_at.elapsed() < Duration::from_millis(1500)
        {
            return;
        }
        self.ms_watch_at = std::time::Instant::now();
        let now = self.ms_watch_snapshot();
        if now == self.ms_watch {
            return;
        }
        if now.1 != self.ms_watch.1 {
            self.reload_library(); // rebuild_order rescans the manuscript too
        } else {
            self.rescan_manuscript();
        }
    }

    /// Reload both tiers from disk after an external change to the
    /// manuscript's bib/ (a coauthor's pull, a hand-dropped .bib, …),
    /// preserving the two-tier switch and UI state; rebuild_order
    /// re-derives everything display-side.
    fn reload_library(&mut self) {
        match MergedLibrary::load(self.ms_root().as_deref()) {
            Ok(mut lib) => {
                lib.global_on = self.lib.global_on;
                self.lib = lib;
                self.rebuild_order();
            }
            Err(e) => {
                // keep the stale library; refresh the snapshot so a
                // persistent error can't warn every poll
                self.ms_watch = self.ms_watch_snapshot();
                self.note(MsgCat::Warn, format!("library reload failed: {e}"));
            }
        }
    }

    /// Keep refs.bib beside the manuscript in step with its citations,
    /// regenerating silently on every rescan. Created only for TeX
    /// manuscripts (astrobib refs opts a markdown one in); once the
    /// file exists it is kept fresh whatever the sources.
    fn sync_refs_bib(&self, rows: &[MsRow]) {
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
        let _ = crate::export::write_refs_bib(&out, &content);
    }

    fn ms_root(&self) -> Option<std::path::PathBuf> {
        self.lib.manuscript.as_ref().map(|m| m.root.clone())
    }

    /// Persist the current ADS scopes to the tabs.json state file,
    /// user-local and keyed per manuscript context.
    fn save_tabs(&self) {
        let tabs: Vec<crate::tabs::Tab> = self
            .scopes
            .iter()
            .filter_map(|s| match s {
                Scope::Ads { tab, .. } => Some(tab.clone()),
                _ => None,
            })
            .collect();
        crate::tabs::save(&tabs, self.ms_root().as_deref());
    }

    /// Restore saved query scopes and refresh them all on one worker.
    /// Saved tabs restore with their cached results — instant and
    /// offline-friendly; nothing re-queries ADS until r asks.
    fn restore_tabs(&mut self) {
        let saved = crate::tabs::load(self.ms_root().as_deref());
        if saved.is_empty() {
            return;
        }
        let mut cached = 0usize;
        for t in &saved {
            let articles = crate::tabs::load_cached_articles(&t.id);
            if !articles.is_empty() {
                cached += 1;
            }
            self.scopes.push(Scope::Ads { tab: t.clone(), articles });
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
    fn step_limit(&mut self, dir: isize) {
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
        self.run_ads_query_limit(q, Some(self.active_scope), l);
    }

    fn refresh_scope(&mut self) {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { tab, .. }) => {
                self.run_ads_query_limit(tab.query.clone(), Some(self.active_scope), tab.limit)
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

    /// i — import the highlighted ADS result into the library (and the
    /// manuscript db when active), via the parity-verified save path.
    fn import_highlighted(&mut self) {
        let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) else {
            return;
        };
        // selection imports in bulk; otherwise the highlighted row
        let bibcodes: Vec<String> = if self.select_mode && !self.selected.is_empty() {
            articles
                .iter()
                .filter(|a| self.selected.contains(&a.bibcode))
                .filter(|a| self.lib.get_by_bibcode(&a.bibcode).is_none())
                .map(|a| a.bibcode.clone())
                .collect()
        } else {
            match self.table.selected().and_then(|p| articles.get(p)) {
                Some(a) if self.lib.get_by_bibcode(&a.bibcode).is_none() => {
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

    fn import_bibcode(&mut self, bibcode: String) {
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

    fn drain_ads(&mut self) {
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
                AdsMsg::Done { id, mut tab, refresh_of, result } => {
                    // a cancelled query's result is discarded on arrival
                    // (the thread cannot be killed, only outwaited)
                    if let Some(t) = self.finish_task(id).filter(|t| t.cancelled) {
                        self.note(
                            MsgCat::Info,
                            format!("cancelled — {} (result discarded)", t.label),
                        );
                        continue;
                    }
                    match result {
                        Ok(articles) => {
                            for a in &articles {
                                if let (Some(e), Some(c)) =
                                    (self.lib.get_by_bibcode(&a.bibcode), a.citation_count)
                                {
                                    let key = e.key().to_string();
                                    self.metrics.set_citations(&key, c);
                                }
                            }
                            self.metrics.save();
                            let n = articles.len();
                            crate::tabs::save_cached_articles(&tab.id, &articles);
                            tab.refreshed = Some(crate::tabs::now_secs());
                            tab.bibcodes = articles.iter().map(|a| a.bibcode.clone()).collect();
                            let scope = Scope::Ads { tab, articles };
                            match refresh_of {
                                Some(i) if i < self.scopes.len() => self.scopes[i] = scope,
                                _ => {
                                    self.scopes.push(scope);
                                    self.set_scope(self.scopes.len() - 1);
                                }
                            }
                            self.save_tabs();
                            self.note(MsgCat::Ok, format!("{n} ADS result(s)"));
                        }
                        Err(e) => self.note(MsgCat::Err, format!("ADS search failed: {e}")),
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

    /// a/A — bulk selection: `a` toggles all *visible* rows (the
    /// filtered set — so with a filter active it is select-all-in-
    /// filter; with the global tier hidden it is tier-2-only), `A`
    /// selects every item in the scope regardless of filter. Selecting
    /// when everything is already selected deselects (a), mirroring the
    /// single-row toggle; an emptied selection exits the mode.
    fn select_all(&mut self, visible_only: bool) {
        let ids: Vec<String> = match self.scopes.get(self.active_scope) {
            Some(Scope::Manuscript { .. }) => return, // not a selection target
            Some(Scope::Ads { articles, .. }) => {
                articles.iter().map(|a| a.bibcode.clone()).collect()
            }
            _ if visible_only => self
                .filtered
                .iter()
                .map(|&i| self.order[i].clone())
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

    /// t — show/hide the global (tier-1) library. Hidden means: global
    /// entries invisible, imports write only the local tier; the rescue
    /// path still protects sole copies by writing to the global tier.
    fn toggle_global(&mut self) {
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

    /// Register an in-flight background task; the returned id travels
    /// with the worker's completion message back to the drain handler.
    fn add_task(&mut self, kind: TaskKind, label: String, keys: Vec<String>) -> u64 {
        self.next_task_id += 1;
        let id = self.next_task_id;
        self.tasks.push(Task {
            id,
            label,
            kind,
            cancelled: false,
            keys,
        });
        id
    }

    /// Remove a task on completion, returning it so the drain handler
    /// can tell whether it was cancelled in the meantime.
    fn finish_task(&mut self, id: u64) -> Option<Task> {
        self.tasks
            .iter()
            .position(|t| t.id == id)
            .map(|i| self.tasks.remove(i))
    }

    /// Cancel a task: the browser watcher stops for real (poll_cancel);
    /// thread-backed work cannot be killed, so the task is only marked
    /// and its result is discarded on arrival.
    fn cancel_task(&mut self, id: u64) {
        let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) else { return };
        if t.cancelled {
            return;
        }
        t.cancelled = true;
        let (kind, label) = (t.kind, t.label.clone());
        if matches!(kind, TaskKind::Watch) {
            if let Some(cancel) = self.poll_cancel.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            self.note(MsgCat::Warn, "browser download cancelled".to_string());
        } else {
            self.note(
                MsgCat::Warn,
                format!("cancelling — {label} (result will be discarded)"),
            );
        }
    }

    /// Cancel the running browser-download watch (X / the card's ✕),
    /// routing through the task registry when its task is present.
    fn cancel_watch(&mut self) {
        if let Some(id) = self
            .tasks
            .iter()
            .find(|t| matches!(t.kind, TaskKind::Watch) && !t.cancelled)
            .map(|t| t.id)
        {
            self.cancel_task(id);
        } else if let Some(cancel) = self.poll_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.note(MsgCat::Warn, "browser download cancelled".to_string());
        }
    }

    /// Emit an event message: color-coded in the log pane and shown in
    /// the footer while it is the newest entry. A new message snaps the
    /// log pane back to the tail; the log keeps at most 500 entries.
    fn note(&mut self, cat: MsgCat, msg: String) {
        self.status = msg.clone();
        self.log.push((cat, self.started.elapsed().as_secs(), msg));
        self.log_scroll = 0;
        if self.log.len() > 500 {
            let cut = self.log.len() - 500;
            self.log.drain(..cut);
        }
    }

    /// PageUp/PageDown while the log pane is open: page through history,
    /// clamped to the stored entries (positive = older).
    fn scroll_log(&mut self, delta: isize) {
        let visible = self.log.len().min(8);
        let max = self.log.len().saturating_sub(visible) as isize;
        self.log_scroll = (self.log_scroll as isize + delta).clamp(0, max) as usize;
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
            Action::GlobalTier => self.lib.manuscript.is_some(),
            Action::Manuscript => {
                self.lib.manuscript.is_some() && self.lib.global_on && !keys.is_empty()
            }
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
    fn unavailable_reason(&self, a: Action) -> String {
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
                "every paper shown is already the manuscript's — t shows the global tier"
                    .to_string()
            }
            Action::Manuscript if unimported => {
                import_first("the manuscript db holds library entries")
            }
            Action::Manuscript => "no paper under the cursor".to_string(),
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
            Action::Remove if unimported => import_first("removal acts on the library entry"),
            Action::Remove => "no paper to remove".to_string(),
            Action::Copy => "nothing to copy".to_string(),
            Action::GlobalTier => {
                "no local db here — the global library is all there is".to_string()
            }
            // always available; unreachable, but the match stays total
            Action::Select | Action::Filter | Action::Card | Action::Log | Action::Help
            | Action::Quit => "not available here".to_string(),
        }
    }

    /// Run an action if available — shared by shortcut keys, panel clicks,
    /// and pub card buttons.
    fn run_action(&mut self, a: Action) {
        if !self.available(a) {
            let why = self.unavailable_reason(a);
            self.note(MsgCat::Warn, why);
            return;
        }
        match a {
            Action::Select => {
                if matches!(self.scopes.get(self.active_scope), Some(Scope::Manuscript { .. })) {
                    // manuscript rows are citations, not selectable papers
                    self.note(
                        MsgCat::Warn,
                        "manuscript rows aren't selectable — [ switches to the library".to_string(),
                    );
                    return;
                }
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
            Action::Filter => {
                if self.active_scope == 0 {
                    self.mode = Mode::Filter;
                } else {
                    self.note(MsgCat::Warn, "the filter applies to the Library scope".to_string());
                }
            }
            Action::Card => self.show_detail = !self.show_detail,
            Action::Log => self.show_log = !self.show_log,
            Action::Help => self.show_help = !self.show_help,
            Action::GlobalTier => self.toggle_global(),
            Action::Quit => self.quit = true,
        }
    }

    /// The entries an action applies to: the selection (in display order)
    /// when selection mode is active and non-empty, else the highlighted
    /// row — one convention for every bulk-capable action.
    /// e — prompt for a destination path and export the selection (or
    /// the cursor entry) as one .bib file.
    fn open_export_prompt(&mut self) {
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
    fn do_export(&mut self, path: &str, keys: &[String]) {
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

    /// Priority targets: the selection, else the cursor entry (in a
    /// query scope, the imported twin).
    fn priority_targets(&mut self) -> Vec<String> {
        let mut keys = self.action_keys();
        if keys.is_empty() {
            if let Some(k) = self.card_entry_key() {
                keys.push(k);
            }
        }
        keys
    }

    /// `.` → 1.0, `0` → 0.0, `<`/`>` scale by ×0.8/×1.25 — multi-select
    /// aware, with the resulting level in the footer and the swatch
    /// recoloring on the next frame.
    fn adjust_priority(&mut self, op: PriorityOp) {
        let keys = self.priority_targets();
        if keys.is_empty() {
            let msg = if self.on_query() {
                "import the paper first (i) — priority is per library entry"
            } else {
                "no paper to prioritize"
            };
            self.note(MsgCat::Warn, msg.to_string());
            return;
        }
        let mut last = 0.0;
        for k in &keys {
            last = match op {
                PriorityOp::Set(v) => self.metrics.set_priority(k, v),
                PriorityOp::Scale(f) => self.metrics.scale_priority(k, f),
            };
        }
        // no disk write here: repeated keys must stay instant — the
        // idle tick (and quit) flushes the dirty store
        let what = if keys.len() == 1 {
            keys[0].clone()
        } else {
            format!("{} papers", keys.len())
        };
        self.note(MsgCat::Ok, format!("priority {last:.2} — {what}"));
    }

    /// r in the library scope: one batched ADS query refreshes the
    /// citation counts of every visible entry.
    fn refresh_citation_counts(&mut self) {
        if self.cit_rx.is_some() {
            return;
        }
        let bibs: Vec<(String, String)> = self
            .filtered
            .iter()
            .filter_map(|&i| self.lib.get(&self.order[i]))
            .filter_map(|e| e.bibcode().map(|b| (b.to_string(), e.key().to_string())))
            .collect();
        if bibs.is_empty() || crate::ads::get_token().is_none() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.cit_rx = Some(rx);
        let id = self.add_task(TaskKind::Query, "⌕ citation counts".to_string(), vec![]);
        let _ = id;
        std::thread::spawn(move || {
            let q = format!(
                "bibcode:({})",
                bibs.iter().map(|(b, _)| b.as_str()).collect::<Vec<_>>().join(" OR ")
            );
            let n = bibs.len();
            let out: Vec<(String, i64)> = match crate::ads::search(&q, n) {
                Ok(arts) => arts
                    .into_iter()
                    .filter_map(|a| {
                        let key = bibs.iter().find(|(b, _)| *b == a.bibcode)?.1.clone();
                        Some((key, a.citation_count?))
                    })
                    .collect(),
                Err(_) => vec![],
            };
            let _ = tx.send(out);
        });
        self.note(MsgCat::Info, "refreshing citation counts…".to_string());
    }

    /// ⟳ on the card — refresh one paper's citation count.
    fn refresh_citation_count_for(&mut self, key: &str) {
        if self.cit_rx.is_some() {
            self.note(MsgCat::Warn, "a citation refresh is already running".to_string());
            return;
        }
        let Some(bc) = self.lib.get(key).and_then(|e| e.bibcode().map(str::to_string)) else {
            self.note(MsgCat::Warn, "no bibcode for that entry".to_string());
            return;
        };
        if crate::ads::get_token().is_none() {
            self.note(MsgCat::Warn, "no ADS token — press S to set one".to_string());
            return;
        }
        let key = key.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.cit_rx = Some(rx);
        self.add_task(TaskKind::Query, "⌕ citation counts".to_string(), vec![]);
        std::thread::spawn(move || {
            let out: Vec<(String, i64)> =
                match crate::ads::search(&format!("identifier:{bc}"), 1) {
                    Ok(arts) => arts
                        .into_iter()
                        .filter_map(|a| Some((key.clone(), a.citation_count?)))
                        .collect(),
                    Err(_) => vec![],
                };
            let _ = tx.send(out);
        });
    }

    fn drain_citations(&mut self) {
        let Some(rx) = &self.cit_rx else { return };
        match rx.try_recv() {
            Ok(counts) => {
                let n = counts.len();
                let bcs: Vec<(String, i64)> = counts
                    .iter()
                    .filter_map(|(k, c)| {
                        self.lib.get(k).and_then(|e| e.bibcode()).map(|b| (b.to_string(), *c))
                    })
                    .collect();
                for (k, c) in counts {
                    self.metrics.set_citations(&k, c);
                }
                for s in &mut self.scopes {
                    if let Scope::Ads { articles, .. } = s {
                        for a in articles.iter_mut() {
                            if let Some((_, c)) = bcs.iter().find(|(b, _)| *b == a.bibcode) {
                                a.citation_count = Some(*c);
                            }
                        }
                    }
                }
                self.metrics.save();
                if let Some(t) = self.tasks.iter().find(|t| t.label == "⌕ citation counts") {
                    let id = t.id;
                    self.finish_task(id);
                }
                self.note(MsgCat::Ok, format!("citation counts refreshed ({n})"));
                self.cit_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.cit_rx = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
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
    fn action_keys(&self) -> Vec<String> {
        if let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) {
            let sel = self.selected_articles();
            let pool: Vec<&crate::ads::Article> = if !sel.is_empty() {
                sel
            } else {
                self.card_article_pos().and_then(|p| articles.get(p)).into_iter().collect()
            };
            return pool
                .iter()
                .filter_map(|a| self.lib.get_by_bibcode(&a.bibcode))
                .map(|e| e.key().to_string())
                .collect();
        }
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

    /// Whether the active scope is a query (ADS results) page.
    fn on_query(&self) -> bool {
        matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. }))
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
        self.restore_tabs();
        debug_layout(&format!("{:>6}ms restore_tabs done", t0.elapsed().as_millis()));
        while !self.quit {
            self.drain_downloads();
            self.drain_ads();
            self.drain_update();
            self.drain_bib_preview();
            self.drain_citations();
            self.poll_manuscript();
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
                self.table_area,
            ));
            let had_events = event::poll(Duration::from_millis(tick))?;
            if !had_events {
                // idle: flush any dirty metrics (a no-op while clean)
                self.metrics.save();
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
                        _ => {}
                    }
                    if self.quit || !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
        }
        self.metrics.save();
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
        (pos < self.row_count()).then_some(pos)
    }

    /// The entry the pub card shows: hovering a scope-specific trigger
    /// column previews that row in the card (full-row hover proved too
    /// twitchy) — the Key column in the library, the Cited column in the
    /// manuscript scope; otherwise the cursor row.
    fn card_key(&self) -> Option<&str> {
        if self.active_scope == 0 {
            let a = self.table_area;
            let (_, show_key) = column_layout(a.width);
            let show_key = show_key || self.show_detail;
            let in_key_col = show_key && self.hover.0 >= a.x + a.width.saturating_sub(20);
            if in_key_col {
                if let Some(pos) = self.hovered_table_pos() {
                    return self.filtered.get(pos).map(|&i| self.order[i].as_str());
                }
            }
        }
        if let Some(Scope::Manuscript { rows }) = self.scopes.get(self.active_scope) {
            // Cited column: after the 2-wide gutter and state columns
            // (spacing 1), x spans [6, 6+26)
            let a = self.table_area;
            if self.hover.0 >= a.x + 6 && self.hover.0 < a.x + 6 + 26 {
                if let Some(k) = self
                    .hovered_table_pos()
                    .and_then(|pos| rows.get(pos))
                    .and_then(|r| r.key.as_deref())
                {
                    return Some(k);
                }
            }
        }
        self.selected_key()
    }

    /// The ADS article position the card shows: hovering the Title
    /// column previews that row; otherwise the cursor row.
    fn card_article_pos(&self) -> Option<usize> {
        let a = self.table_area;
        // Key column (rightmost 20) — the same trigger as the library scope
        if self.hover.0 >= a.x + a.width.saturating_sub(20) {
            if let Some(pos) = self.hovered_table_pos() {
                return Some(pos);
            }
        }
        self.table.selected()
    }

    /// The cite key an article WOULD get on import — computable locally
    /// because keys derive from stable identity (arXiv ID / bibcode,
    /// first-author surname, identity year), all present in the article.
    fn hypothetical_key(&self, a: &crate::ads::Article) -> String {
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

    /// The library entry the pub card's buttons act on: in an ADS scope
    /// the shown article's imported twin (if any), else the selected
    /// entry. Distinct from selected_key because the card can preview a
    /// hovered row.
    /// Open a citations(...) (or references(...)) query scope for the
    /// card's paper — the C / R keys and the ⌕ card rows.
    fn spawn_citation_query(&mut self, refs: bool) {
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
            self.run_ads_query_limit(q, None, crate::tabs::DEFAULT_LIMIT);
        }
    }

    fn card_entry_key(&self) -> Option<String> {
        if let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) {
            let a = self.card_article_pos().and_then(|p| articles.get(p))?;
            return self.lib.get_by_bibcode(&a.bibcode).map(|e| e.key().to_string());
        }
        self.selected_key().map(str::to_string)
    }

    /// The bibcode the card's citation-graph affordances act on: the
    /// shown ADS article's, else the shown library entry's (derived
    /// from its adsurl).
    fn card_bibcode(&self) -> Option<String> {
        if let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) {
            return self
                .card_article_pos()
                .and_then(|p| articles.get(p))
                .map(|a| a.bibcode.clone());
        }
        let key = self.card_key()?;
        self.lib.get(key).and_then(|e| e.bibcode()).map(str::to_string)
    }

    fn selected_key(&self) -> Option<&str> {
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
        Some(self.order[idx].as_str())
    }

    fn refilter(&mut self) {
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
        let ctx = QueryContext {
            in_manuscript: Some(Box::new(move |k: &str| in_ms.iter().any(|x| x == k))),
            has_pdf: Some(Box::new(|k: &str| has_cached_pdf(k))),
            priority: Some(Box::new(move |k: &str| pri.get(k).copied())),
            citations: Some(Box::new(move |k: &str| cit.get(k).copied())),
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

    fn row_count(&self) -> usize {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => articles.len(),
            Some(Scope::Manuscript { rows }) => rows.len(),
            _ => self.filtered.len(),
        }
    }

    fn move_sel(&mut self, delta: isize) {
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
    fn toggle_row_selected(&mut self, pos: usize) {
        let key = match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => {
                let Some(a) = articles.get(pos) else { return };
                a.bibcode.clone()
            }
            _ => {
                let Some(&idx) = self.filtered.get(pos) else {
                    return;
                };
                self.order[idx].clone()
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

    fn exit_select_mode(&mut self) {
        self.select_mode = false;
        self.selected.clear();
        self.status = format!("{} papers", self.order.len());
    }

    /// Rebuild the display order (entries changed or sort changed),
    /// and refresh the manuscript classification when present.
    fn rebuild_order(&mut self) {
        if matches!(self.scopes.get(1), Some(Scope::Manuscript { .. })) {
            let rows = self.ms_rows();
            self.sync_refs_bib(&rows);
            if let Some(s) = self.scopes.get_mut(1) {
                *s = Scope::Manuscript { rows };
            }
        }
        self.order = self.lib.entries().iter().map(|e| e.key().to_string()).collect();
        let lib = &self.lib;
        let (col, asc) = self.sort;
        self.order.sort_by(|a, b| {
            let (ea, eb) = (lib.get(a).unwrap(), lib.get(b).unwrap());
            let ord = match col {
                SortCol::Metric => match self.metric_col {
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
                    MetricCol::Off => std::cmp::Ordering::Equal,
                },
                SortCol::Pdf => has_cached_pdf(ea.key()).cmp(&has_cached_pdf(eb.key())),
                SortCol::InLib => lib
                    .in_manuscript(ea.key())
                    .cmp(&lib.in_manuscript(eb.key())),
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
        if self.lib.manuscript.is_some() {
            // our own writes touch bib/; refresh the watch snapshot so
            // the auto-rescan poll doesn't bounce them back as a reload
            self.ms_watch = self.ms_watch_snapshot();
        }
    }

    /// Header click: same column flips direction, a new column starts
    /// descending for Year (newest first) and ascending otherwise.
    fn sort_by(&mut self, col: SortCol) {
        self.sort = if self.sort.0 == col {
            (col, !self.sort.1)
        } else {
            // bool-ish and recency columns start with the interesting side
            // up: cached/in-library/newest first; text columns start A→Z
            (
                col,
                !matches!(
                    col,
                    SortCol::Year | SortCol::Pdf | SortCol::InLib | SortCol::Metric
                ),
            )
        };
        if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
            self.sort_ads_articles();
        } else {
            self.rebuild_order();
        }
    }

    /// Re-sort the active ADS scope's articles in place (decorate-sort:
    /// cache/library lookups happen before the mutable put-back).
    fn sort_ads_articles(&mut self) {
        let (col, asc) = self.sort;
        let Some(Scope::Ads { articles, .. }) = self.scopes.get_mut(self.active_scope) else {
            return;
        };
        let arts = std::mem::take(articles);
        let mut decorated: Vec<(String, crate::ads::Article)> = arts
            .into_iter()
            .map(|a| {
                let key = match col {
                    SortCol::Key => String::new(), // filled below with lib access
                    SortCol::Metric => format!("{:012}", a.citation_count.unwrap_or(0)),
                    SortCol::Year => a.year.clone(),
                    SortCol::Author => a
                        .author
                        .first()
                        .map(|s| s.split(',').next().unwrap_or("").trim().to_lowercase())
                        .unwrap_or_default(),
                    SortCol::Title => a.title.to_lowercase(),
                    _ => String::new(), // Pdf/InLib filled below with lib access
                };
                (key, a)
            })
            .collect();
        if matches!(col, SortCol::Pdf | SortCol::InLib | SortCol::Key) {
            for (k, a) in decorated.iter_mut() {
                let entry = self.lib.get_by_bibcode(&a.bibcode);
                *k = match col {
                    SortCol::Pdf => {
                        let ck = entry
                            .map(|e| e.key().to_string())
                            .unwrap_or_else(|| a.bibcode.clone());
                        u8::from(pdf::is_cached(&ck)).to_string()
                    }
                    SortCol::Key => self.hypothetical_key(a),
                    _ => u8::from(entry.is_some()).to_string(),
                };
            }
        }
        decorated.sort_by(|x, y| {
            let ord = x.0.cmp(&y.0).then(x.1.bibcode.cmp(&y.1.bibcode));
            if asc { ord } else { ord.reverse() }
        });
        let sorted: Vec<crate::ads::Article> = decorated.into_iter().map(|(_, a)| a).collect();
        if let Some(Scope::Ads { articles, .. }) = self.scopes.get_mut(self.active_scope) {
            *articles = sorted;
        }
    }

    /// m — the library-view manuscript toggle rule: if any target is
    /// missing from the manuscript db, add all missing; else (all
    /// present) remove all.
    fn toggle_manuscript(&mut self) {
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

    /// Delete — ask for confirmation before removing. The targets are
    /// the usual ones (selection, else the shown row — on a query page
    /// their imported twins).
    fn remove_papers(&mut self) {
        let keys = self.action_keys();
        if !keys.is_empty() {
            self.mode = Mode::Confirm { keys };
        }
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

    /// Confirmed removal: both tiers normally; with the global tier
    /// hidden, only the local tier — sole copies rescue to the (hidden)
    /// global tier rather than being destroyed.
    fn remove_confirmed(&mut self, keys: &[String]) {
        let local_only = self.lib.manuscript.is_some() && !self.lib.global_on;
        // query pages: a paper the active manuscript cites keeps its
        // tier-2 copy — only the global (tier-1) copy is removed
        let on_query = matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. }));
        let cited = if on_query { self.cited_keys() } else { Default::default() };
        let mut n = 0;
        let mut kept = 0;
        for k in keys {
            let ok = if on_query && cited.contains(k) {
                kept += 1;
                self.lib.personal.remove_entry(k).is_ok()
            } else if local_only {
                matches!(self.lib.remove_from_manuscript(k), Ok(true))
            } else {
                self.lib.remove_entry(k).is_ok()
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
        if self.poll_cancel.is_some() {
            self.cancel_watch();
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
    /// The cache is keyed by library cite key, full stop. Every path
    /// that writes a PDF passes through here, so no bibcode-named file
    /// can be created even if a caller is careless.
    fn pdf_key(&mut self, key: String) -> Option<String> {
        if self.lib.get(&key).is_some() {
            return Some(key);
        }
        self.note(
            MsgCat::Warn,
            "import the paper first — PDFs are cached under its cite key".to_string(),
        );
        None
    }

    fn download_single(&mut self, key: String, source: pdf::Source) {
        if self.dl_rx.is_some() {
            self.note(MsgCat::Warn, "a download is already running".to_string());
            return;
        }
        let Some(e) = self.lib.get(&key) else { return };
        let (eprint, adsurl) = (e.eprint().to_string(), e.adsurl().to_string());
        let src = match source {
            pdf::Source::Arxiv => "arXiv",
            pdf::Source::Oa => "ADS OA",
            pdf::Source::Auto => "auto",
        };
        let id = self.add_task(
            TaskKind::Download,
            format!("↓ {src} PDF — {key}"),
            vec![key.clone()],
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
        self.note(MsgCat::Info, format!("Downloading {key}…"));
        std::thread::spawn(move || {
            let ok = pdf::fetch_source(&key, &eprint, &adsurl, source).is_some();
            let _ = tx.send(DlMsg::Done {
                id,
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
        let id = self.add_task(
            TaskKind::Watch,
            format!("👁 watching ~/Downloads — {key}"),
            vec![key.clone()],
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
        self.note(MsgCat::Info, format!("Resolving browser source for {key}…"));
        std::thread::spawn(move || {
            let Some(url) = pdf::browser_resolve_url(&doi, &adsurl, &eprint) else {
                let _ = tx.send(DlMsg::Done { id, done: 0, failed: vec![key] });
                return;
            };
            let before = pdf::downloads_snapshot();
            pdf::browser_open(&url);
            let _ = tx.send(DlMsg::Progress(format!(
                "Browser opened — waiting for {key} in ~/Downloads (60s, X cancels)…"
            )));
            let got = pdf::poll_downloads(&key, &before, 60, &cancel);
            let _ = tx.send(DlMsg::Done {
                id,
                done: got.is_some() as usize,
                failed: if got.is_some() { vec![] } else { vec![key] },
            });
        });
    }

    /// pick … — open the modal ~/Downloads PDF picker for one entry.
    fn open_picker_for(&mut self, key: String) {
        let Some(key) = self.pdf_key(key) else { return };
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
            // on a query page there are no library keys at all — say the
            // useful thing rather than the generic one
            let msg = if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
                "import the paper first (i) — PDFs are cached under its cite key"
            } else {
                "nothing to download (cached, or no arXiv ID / ADS URL)"
            };
            self.note(MsgCat::Warn, msg.to_string());
            return;
        }
        let total = items.len();
        let label = if let [(k, _, _)] = items.as_slice() {
            format!("↓ PDF — {k}")
        } else {
            format!("↓ PDFs — {total} papers")
        };
        let id = self.add_task(
            TaskKind::Download,
            label,
            items.iter().map(|(k, _, _)| k.clone()).collect(),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
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
            let _ = tx.send(DlMsg::Done { id, done, failed });
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
                DlMsg::Done { id, done, failed } => {
                    let finished = self.finish_task(id);
                    if finished.as_ref().is_some_and(|t| t.cancelled) {
                        // discard the cancelled task's result: remove
                        // anything it cached so the end state matches the
                        // existing failure/clear paths, and reset the
                        // channel state exactly as the normal arm does
                        let t = finished.unwrap();
                        for k in &t.keys {
                            let p = pdf::cache_path(k);
                            if p.exists() {
                                let _ = std::fs::remove_file(&p);
                            }
                        }
                        self.note(
                            MsgCat::Info,
                            format!("cancelled — {} (result discarded)", t.label),
                        );
                        self.pdf_status.clear();
                        self.dl_rx = None;
                        self.poll_cancel = None;
                        continue;
                    }
                    // an unavailable PDF is an expected outcome (many
                    // publishers require the browser), not an app failure
                    let cat = if failed.is_empty() { MsgCat::Ok } else { MsgCat::Warn };
                    let msg = if failed.is_empty() {
                        format!("Downloaded {done} PDF(s)")
                    } else {
                        format!(
                            "Downloaded {done} PDF(s) — no auto PDF for {}{} (try browser ↓)",
                            failed[..failed.len().min(3)].join(", "),
                            if failed.len() > 3 { "…" } else { "" }
                        )
                    };
                    self.note(cat, msg);
                    // success needs no card note — the buttons flipping to
                    // Open/Clear already signal it; an unavailable PDF gets
                    // an expected-outcome note, not an error
                    self.pdf_status = if done == 0 && !failed.is_empty() {
                        "⚠ no open-access PDF found".to_string()
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
            MouseEventKind::ScrollDown => {
                if self.scroll_swatch(m.column, m.row, 0.8) {
                } else if self.show_detail && hit(self.card_area, m.column, m.row) {
                    self.card_scroll = self.card_scroll.saturating_add(3);
                } else {
                    self.move_sel(3);
                }
            }
            MouseEventKind::ScrollUp => {
                if self.scroll_swatch(m.column, m.row, 1.25) {
                } else if self.show_detail && hit(self.card_area, m.column, m.row) {
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
        if self.metric_col != MetricCol::Priority || !hit(self.metric_area, x, y) {
            return false;
        }
        // the strip's rows start two lines below its top (header + rule)
        if y < self.metric_area.y + 2 {
            return false;
        }
        let pos = self.table.offset() + (y - self.metric_area.y - 2) as usize;
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
                .and_then(|a| self.lib.get_by_bibcode(&a.bibcode))
                .map(|e| e.key().to_string()),
            Some(Scope::Manuscript { rows }) => rows.get(pos).and_then(|r| r.key.clone()),
            _ => self.filtered.get(pos).map(|&i| self.order[i].clone()),
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
        if self.show_about {
            if let Some((_, url)) = self.about_links.iter().find(|(r, _)| hit(*r, x, y)) {
                pdf::browser_open(url);
                self.note(MsgCat::Info, "opened in browser".to_string());
            } else if hit(self.about_btn, x, y) {
                self.check_updates();
            } else {
                self.show_about = false;
            }
            return;
        }
        // clicking away from the query prompt dismisses it, and the click
        // then performs its normal action (e.g. switching scope); the
        // filter likewise leaves entry mode, but stays applied (as ⏎) —
        // clicking a row of the filtered results must not wipe them
        if matches!(
            self.mode,
            Mode::AdsPrompt { .. } | Mode::Filter | Mode::Setup { .. } | Mode::Export { .. }
        ) {
            self.mode = Mode::Normal;
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
        // copy-regions: the card text copies its own entry's datum; in
        // ADS scopes values come from the shown article itself
        if let Some(&(_, item)) = self.card_yanks.iter().find(|(r, _)| hit(*r, x, y)) {
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
        if let Some((_, url)) = self.card_links.iter().find(|(r, _)| hit(*r, x, y)) {
            let url = url.clone();
            pdf::browser_open(&url);
            self.note(MsgCat::Info, "opened in browser".to_string());
            return;
        }
        // keys-panel rows act as their key
        if self.show_help {
            if let Some(&(_, code)) = self.help_rects.iter().find(|(r, _)| hit(*r, x, y)) {
                self.on_key(code, KeyModifiers::NONE);
                return;
            }
        }
        // a click leaves an active y-chord and then acts normally —
        // the card's ⧉ rows are the visible copy menu
        if matches!(self.mode, Mode::Copy) {
            self.exit_copy_mode();
        }
        // pub card buttons (act on the card's entry)
        if let Some(&(_, btn)) = self.card_buttons.iter().find(|(r, _)| hit(*r, x, y)) {
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
        if let Some(&(_, idx)) = self.scope_rects.iter().find(|(r, _)| hit(*r, x, y)) {
            if idx == FILTER_CHIP {
                self.run_action(Action::Filter);
            } else if idx == usize::MAX {
                self.open_ads_prompt();
            } else {
                self.set_scope(idx);
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
        let selectable = !matches!(
            self.scopes.get(self.active_scope),
            Some(Scope::Manuscript { .. })
        );
        if (modified || x < a.x + 2) && selectable {
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

    /// Clear (or cancel a pending browser watch for) the card entry's PDF.
    fn clear_card_pdf(&mut self, key: &str) {
        if self.poll_cancel.is_some() {
            self.cancel_watch();
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
        let on_article = matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. }));
        if !on_article && self.action_keys().is_empty() {
            self.note(MsgCat::Warn, "nothing to copy".to_string());
            return;
        }
        self.mode = Mode::Copy;
    }

    /// The selected articles of the active ADS scope, in row order.
    fn selected_articles(&self) -> Vec<&crate::ads::Article> {
        let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) else {
            return vec![];
        };
        if !self.select_mode || self.selected.is_empty() {
            return vec![];
        }
        articles.iter().filter(|a| self.selected.contains(&a.bibcode)).collect()
    }

    /// A copy value spanning several selected query results: list-like
    /// items join with commas (keys, bibcodes) or newlines (URLs,
    /// paths); prose (title, abstract) has no sensible multi form.
    fn articles_copy_value(&self, items: &[&crate::ads::Article], item: CopyItem) -> Option<String> {
        let vals: Vec<String> = items
            .iter()
            .filter_map(|a| match item {
                CopyItem::Key | CopyItem::FullKey => Some(self.hypothetical_key(a)),
                CopyItem::Bibcode => Some(a.bibcode.clone()),
                CopyItem::AdsUrl => Some(format!(
                    "https://ui.adsabs.harvard.edu/abs/{}/abstract",
                    a.bibcode
                )),
                CopyItem::ArxivUrl => {
                    crate::ads::arxiv_id(a).map(|id| format!("https://arxiv.org/abs/{id}"))
                }
                CopyItem::DoiUrl => a.doi.first().map(|d| format!("https://doi.org/{d}")),
                CopyItem::PdfPath => self
                    .lib
                    .get_by_bibcode(&a.bibcode)
                    .map(|e| e.key().to_string())
                    .filter(|k| pdf::is_cached(k))
                    .map(|k| pdf::cache_path(&k).to_string_lossy().into_owned()),
                CopyItem::Title | CopyItem::Abstract => None, // no multi form
            })
            .collect();
        if vals.is_empty() {
            return None;
        }
        let sep = match item {
            CopyItem::Key | CopyItem::FullKey | CopyItem::Bibcode => ", ",
            _ => "\n",
        };
        Some(vals.join(sep))
    }

    /// The chord/click copy value for the shown ADS article — the same
    /// items the card's ⧉ rows offer, from the article itself.
    fn article_copy_value(&self, item: CopyItem) -> Option<String> {
        let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) else {
            return None;
        };
        let a = self.card_article_pos().and_then(|p| articles.get(p))?;
        self.article_value(a, item)
    }

    /// Every copyable datum of one query-result article.
    fn article_value(&self, a: &crate::ads::Article, item: CopyItem) -> Option<String> {
        match item {
            CopyItem::Title => Some(a.title.clone()),
            CopyItem::Abstract => {
                (!a.abstract_.is_empty()).then(|| crate::ads::clean_abstract(&a.abstract_))
            }
            CopyItem::Bibcode => Some(a.bibcode.clone()),
            CopyItem::AdsUrl => Some(format!(
                "https://ui.adsabs.harvard.edu/abs/{}/abstract",
                a.bibcode
            )),
            CopyItem::ArxivUrl => {
                crate::ads::arxiv_id(a).map(|id| format!("https://arxiv.org/abs/{id}"))
            }
            CopyItem::DoiUrl => a.doi.first().map(|d| format!("https://doi.org/{d}")),
            CopyItem::PdfPath => self
                .lib
                .get_by_bibcode(&a.bibcode)
                .map(|e| e.key().to_string())
                .filter(|k| pdf::is_cached(k))
                .map(|k| pdf::cache_path(&k).to_string_lossy().into_owned()),
            CopyItem::Key | CopyItem::FullKey => Some(self.hypothetical_key(a)),
        }
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
        let multi_prose = matches!(item, CopyItem::Title | CopyItem::Abstract);
        let text = if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
            let sel = self.selected_articles();
            if sel.len() > 1 {
                if multi_prose {
                    self.note(
                        MsgCat::Warn,
                        format!("no multi-item form for that ({} selected)", sel.len()),
                    );
                    return;
                }
                self.articles_copy_value(&sel, item)
            } else if sel.len() == 1 {
                self.article_value(sel[0], item)
            } else {
                self.article_copy_value(item)
            }
        } else {
            if multi_prose && self.select_mode && self.selected.len() > 1 {
                self.note(
                    MsgCat::Warn,
                    format!("no multi-item form for that ({} selected)", self.selected.len()),
                );
                return;
            }
            self.copy_value(item)
        };
        let Some(text) = text else {
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
        if self.show_about {
            self.show_about = false; // any key dismisses the about modal
            return;
        }
        match &mut self.mode {
            Mode::Filter => match code {
                KeyCode::Esc => {
                    self.filter = tui_input::Input::default();
                    self.mode = Mode::Normal;
                    self.refilter();
                }
                KeyCode::Enter => self.mode = Mode::Normal,
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        self.filter.handle(req);
                        return;
                    }
                    let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                    if self.filter.handle_event(&ev).is_some() {
                        self.refilter();
                    }
                }
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
            Mode::Export { input, keys } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let (path, keys) = (input.value().trim().to_string(), keys.clone());
                    self.mode = Mode::Normal;
                    if !path.is_empty() {
                        self.do_export(&path, &keys);
                    }
                }
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        input.handle(req);
                        return;
                    }
                    let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                    input.handle_event(&ev);
                }
            },
            Mode::Setup { input, email, resume } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let (v, was_email, resume) = (input.value().trim().to_string(), *email, *resume);
                    if !was_email {
                        if v.is_empty() {
                            return; // a token is the point; Esc cancels
                        }
                        if let Err(e) = crate::ads::save_state_field("ads_token", &v) {
                            self.mode = Mode::Normal;
                            self.note(MsgCat::Err, format!("could not save token: {e}"));
                            return;
                        }
                        self.note(MsgCat::Ok, "ADS token saved".to_string());
                        if crate::ads::get_email().is_none() {
                            self.mode = Mode::Setup {
                                input: tui_input::Input::default(),
                                email: true,
                                resume,
                            };
                            return;
                        }
                    } else if !v.is_empty() {
                        if let Err(e) = crate::ads::save_state_field("email", &v) {
                            self.note(MsgCat::Err, format!("could not save email: {e}"));
                        } else {
                            self.note(MsgCat::Ok, "email saved".to_string());
                        }
                    }
                    self.mode = Mode::Normal;
                    if resume {
                        self.open_ads_prompt();
                    }
                }
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        input.handle(req);
                        return;
                    }
                    let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                    input.handle_event(&ev);
                }
            },
            Mode::AdsPrompt { input, limit } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let (q, l) = (input.value().to_string(), *limit);
                    self.mode = Mode::Normal;
                    self.run_ads_query_limit(q, None, l);
                }
                KeyCode::Up => {
                    const STEPS: [usize; 4] = [20, 50, 100, 200];
                    let i = STEPS.iter().position(|&s| s >= *limit).unwrap_or(0);
                    *limit = STEPS[(i + 1).min(STEPS.len() - 1)];
                }
                KeyCode::Down => {
                    const STEPS: [usize; 4] = [20, 50, 100, 200];
                    let i = STEPS.iter().position(|&s| s >= *limit).unwrap_or(0);
                    *limit = STEPS[i.saturating_sub(1)];
                }
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        input.handle(req);
                        return;
                    }
                    let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                    input.handle_event(&ev);
                }
            },
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
                // plain-letter bindings must not fire on ctrl/alt chords
                // (ctrl+a once triggered select-all); ctrl+w is the one
                // deliberate chord below
                // ctrl+p was the old control-panel chord; keep it as a
                // cheat-sheet alias for muscle memory
                KeyCode::Char('p') if mods.contains(KeyModifiers::CONTROL) => {
                    self.run_action(Action::Help)
                }
                KeyCode::Char(c)
                    if mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && !(c == 'w' && mods.contains(KeyModifiers::CONTROL)) =>
                {
                    // ignored
                }
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
                KeyCode::Char('t') => self.run_action(Action::GlobalTier),
                KeyCode::Char('v') => self.show_bib_source = !self.show_bib_source,
                KeyCode::Char('e') => self.open_export_prompt(),
                KeyCode::Char('M') => {
                    self.metric_col = self.metric_col.next();
                    if self.metric_col == MetricCol::Off && self.sort.0 == SortCol::Metric {
                        self.sort = (SortCol::Year, false);
                        self.rebuild_order();
                    }
                    let _ = crate::ads::save_state_field("metric", self.metric_col.state_tag());
                    self.note(
                        MsgCat::Info,
                        format!("metric column: {}", self.metric_col.name()),
                    );
                }
                KeyCode::Char('.') => self.adjust_priority(PriorityOp::Set(1.0)),
                KeyCode::Char('0') => self.adjust_priority(PriorityOp::Set(0.0)),
                KeyCode::Char('>') => self.adjust_priority(PriorityOp::Scale(1.25)),
                KeyCode::Char('<') => self.adjust_priority(PriorityOp::Scale(0.8)),
                KeyCode::Char('@') => self.show_about = true,
                KeyCode::Char('C') => self.spawn_citation_query(false),
                KeyCode::Char('R') => self.spawn_citation_query(true),
                KeyCode::Char('a') => self.select_all(true),
                KeyCode::Char('A') => self.select_all(false),
                KeyCode::Char('S') => self.open_ads_prompt(),
                KeyCode::Char('[') => self.cycle_scope(-1),
                KeyCode::Char(']') => self.cycle_scope(1),
                KeyCode::Char('r') => self.refresh_scope(),
                KeyCode::Char('+') | KeyCode::Char('=') => self.step_limit(1),
                KeyCode::Char('-') => self.step_limit(-1),
                KeyCode::Char('w') if mods.contains(KeyModifiers::CONTROL) => {
                    self.close_scope()
                }
                KeyCode::Char('i') => self.import_highlighted(),
                KeyCode::Char('L') => self.run_action(Action::Log),
                KeyCode::Char('y') => self.run_action(Action::Copy),
                KeyCode::Char('Y') => self.do_copy(CopyItem::FullKey),
                KeyCode::Char(' ') => self.run_action(Action::Select),
                KeyCode::Esc => {
                    if self.show_help {
                        self.show_help = false;
                    } else if self.select_mode {
                        self.exit_select_mode();
                    } else if !self.filter.value().is_empty() {
                        self.filter = tui_input::Input::default();
                        self.refilter();
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
                KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
                KeyCode::Char('g') | KeyCode::Home => {
                    self.table.select((self.row_count() > 0).then_some(0))
                }
                KeyCode::Char('G') | KeyCode::End => {
                    self.table.select(self.row_count().checked_sub(1))
                }
                KeyCode::PageDown => {
                    if self.show_log {
                        let page = self.log.len().min(8) as isize;
                        self.scroll_log(-page);
                    } else {
                        self.move_sel(20);
                    }
                }
                KeyCode::PageUp => {
                    if self.show_log {
                        let page = self.log.len().min(8) as isize;
                        self.scroll_log(page);
                    } else {
                        self.move_sel(-20);
                    }
                }
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
        let log_h = if self.show_log {
            (self.log.len().min(8) + 2) as u16
        } else {
            0
        };
        let help_h = if self.show_help {
            Self::help_height(f.area().width)
        } else {
            0
        };
        let [main, help_area, log_area, status] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(help_h),
            Constraint::Length(log_h),
            Constraint::Length(3), // rule + air + the footer line
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

        let (strip_area, table_area) = {
            let h = self.scope_strip_height(table_area.width);
            let [s, t] = Layout::vertical([Constraint::Length(h), Constraint::Min(1)])
                .areas(table_area);
            (s, t)
        };
        self.draw_scope_strip(f, strip_area);
        let table_area = if self.metric_col != MetricCol::Off
            && !matches!(self.scopes.get(self.active_scope), Some(Scope::Manuscript { .. }))
        {
            let [swatch, rest] =
                Layout::horizontal([Constraint::Length(1), Constraint::Min(1)]).areas(table_area);
            self.draw_table(f, rest);
            self.draw_metric_strip(f, swatch);
            rest
        } else {
            self.draw_table(f, table_area);
            table_area
        };
        let _ = table_area;
        if let Some(area) = detail_area {
            self.draw_detail(f, area);
        } else {
            self.card_yanks.clear();
        }
        if self.show_help {
            self.draw_help(f, help_area);
        }
        if self.show_log {
            self.draw_log(f, log_area);
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

    /// The control panel, tabbed: Actions lists every action with key,
    /// label, and click target, unavailable ones dimmed rather than
    /// hidden; Copy lists the clipboard targets of the y-chord the same
    /// way. Tab headers are clickable.
    /// Centered which-key modal for the y chord: clickable rows, items
    /// without a value dimmed; clicking elsewhere or Esc cancels.
    /// Rows the keys panel needs at this width (it flows into as many
    /// columns as fit), plus its border rows.
    fn help_height(width: u16) -> u16 {
        let cols = (width.saturating_sub(2) / HELP_COLW).max(1) as usize;
        HELP_ENTRIES.len().div_ceil(cols) as u16 + 2
    }

    /// The keys cheat-sheet, a non-modal panel above the log: keys keep
    /// working while it shows; ? (or Esc, or the footer badge) closes.
    fn draw_help(&mut self, f: &mut Frame, area: Rect) {
        self.help_rects.clear();
        let cols = (area.width.saturating_sub(2) / HELP_COLW).max(1) as usize;
        let rows = HELP_ENTRIES.len().div_ceil(cols);
        let mut lines: Vec<Line> = vec![];
        for r in 0..rows {
            let mut spans: Vec<Span> = vec![];
            for c in 0..cols {
                if let Some((key, label, action, click)) = HELP_ENTRIES.get(r + c * rows) {
                    let avail = action.map_or(true, |a| self.available(a));
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
                    self.help_rects.push((rect, *click));
                    let (mut ks, mut ls) = if avail {
                        (Style::default().fg(Color::Cyan), Style::default())
                    } else {
                        (
                            Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                            Style::default().fg(Color::DarkGray),
                        )
                    };
                    if hov {
                        ks = ks.bg(Color::Rgb(50, 54, 62));
                        ls = ls.bg(Color::Rgb(50, 54, 62));
                    }
                    let text = format!(" {key:>3}  {label}");
                    let pad = (HELP_COLW as usize).saturating_sub(text.chars().count());
                    spans.push(Span::styled(format!(" {key:>3}  "), ks));
                    spans.push(Span::styled(format!("{label}{}", " ".repeat(pad)), ls));
                }
            }
            lines.push(Line::from(spans));
        }
        let p = Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(divider())
                .title(Span::styled(" keys ", Style::default().fg(Color::DarkGray))),
        );
        f.render_widget(p, area);
    }

    /// Fetch (once) the canonical BibTeX an import of this article
    /// would write — bib-source preview for un-imported query results.
    fn request_bib_preview(&mut self, bibcode: String) {
        if self.bib_rx.is_some() || self.bib_preview.contains_key(&bibcode) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.bib_rx = Some(rx);
        std::thread::spawn(move || {
            let text = match crate::ads::fetch_bibtex(&bibcode) {
                Ok(Some(mut data)) => {
                    let key = crate::keys::generate_key(&data);
                    data.insert("ID".to_string(), key);
                    crate::bib::format_entry(&data)
                }
                Ok(None) => "no BibTeX record for this bibcode".to_string(),
                Err(e) => format!("could not fetch BibTeX: {e}"),
            };
            let _ = tx.send((bibcode, text));
        });
    }

    fn drain_bib_preview(&mut self) {
        let Some(rx) = &self.bib_rx else { return };
        match rx.try_recv() {
            Ok((bc, text)) => {
                self.bib_preview.insert(bc, text);
                self.bib_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.bib_rx = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// ⟳ — ask PyPI for the newest astrobib version, on a worker
    /// thread; the result lands in the about modal and the log.
    fn check_updates(&mut self) {
        if self.upd_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.upd_rx = Some(rx);
        self.update_status = Some("checking PyPI…".to_string());
        std::thread::spawn(move || {
            let current = env!("CARGO_PKG_VERSION");
            let msg = (|| -> Result<String, String> {
                let v: serde_json::Value = ureq::get("https://pypi.org/pypi/astrobib/json")
                    .timeout(std::time::Duration::from_secs(6))
                    .call()
                    .map_err(|e| e.to_string())?
                    .into_json()
                    .map_err(|e| e.to_string())?;
                let latest = v["info"]["version"].as_str().unwrap_or("").to_string();
                Ok(if latest.is_empty() {
                    "could not read the PyPI version".to_string()
                } else if latest == current {
                    format!("astrobib {current} is up to date")
                } else {
                    format!("astrobib {latest} is out — pipx upgrade astrobib")
                })
            })()
            .unwrap_or_else(|e| format!("update check failed: {e}"));
            let _ = tx.send(msg);
        });
    }

    fn drain_update(&mut self) {
        let Some(rx) = &self.upd_rx else { return };
        match rx.try_recv() {
            Ok(msg) => {
                self.update_status = Some(msg.clone());
                self.note(MsgCat::Info, msg);
                self.upd_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.upd_rx = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// The @ about modal.
    fn draw_about(&mut self, f: &mut Frame) {
        self.about_links.clear();
        let dim = Style::default().fg(Color::DarkGray);
        let cyan = Style::default().fg(Color::Cyan);
        // labels are the full URLs so terminals that linkify text (e.g.
        // Warp) pick them up too; our own click handler opens them as well
        let links = [
            "https://jzrake.people.clemson.edu",
            "https://pypi.org/project/astrobib",
        ];
        let frame = f.area();
        // ambiguous-width glyphs (↗ ⟳) can render double on some
        // terminals, shifting rows right — several columns of slack
        // beyond the longest line keep everything inside the borders
        let w = 58.min(frame.width.saturating_sub(4));
        let h = (17 + u16::from(self.update_status.is_some())).min(frame.height);
        let area = Rect {
            x: frame.width.saturating_sub(w) / 2,
            y: frame.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        };
        f.render_widget(ratatui::widgets::Clear, area);
        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(
                format!(" astrobib {}", env!("CARGO_PKG_VERSION")),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(" © 2026 Jonathan Zrake · MIT license", dim)),
            Line::default(),
            Line::from(Span::raw(" Clemson University Physics and Astronomy")),
            Line::from(Span::raw(" Supported by NSF award number 2408034")),
            Line::default(),
        ];
        let link_row = |url: &str, lines: &mut Vec<Line>, about_links: &mut Vec<(Rect, String)>| {
            let y = area.y + 1 + lines.len() as u16;
            let r = Rect {
                x: area.x + 5, // border + the " ↗  " prefix
                y,
                width: url.chars().count() as u16 + 1,
                height: 1,
            };
            about_links.push((r, url.to_string()));
            let hov = hit(r, self.hover.0, self.hover.1);
            let style = if hov { cyan.add_modifier(Modifier::UNDERLINED) } else { cyan };
            lines.push(Line::from(vec![
                Span::styled(" ↗  ", dim),
                Span::styled(url.to_string(), style),
            ]));
        };
        let mut about_links = std::mem::take(&mut self.about_links);
        for url in links {
            link_row(url, &mut lines, &mut about_links);
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(" report a bug / request a feature:", dim)));
        link_row(
            "https://github.com/clemson-cal/astrobib/issues",
            &mut lines,
            &mut about_links,
        );
        self.about_links = about_links;
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            " development assisted by Claude Fable 5",
            dim,
        )));
        lines.push(Line::default());
        {
            let label = "⟳ check for updates";
            let y = area.y + 1 + lines.len() as u16;
            let r = Rect {
                x: area.x + 3,
                y,
                width: label.chars().count() as u16 + 1,
                height: 1,
            };
            self.about_btn = r;
            let hov = hit(r, self.hover.0, self.hover.1);
            let style = if hov {
                Style::default().fg(Color::Green).add_modifier(Modifier::UNDERLINED)
            } else {
                Style::default().fg(Color::Green)
            };
            lines.push(Line::from(vec![Span::raw("  "), Span::styled(label, style)]));
        }
        if let Some(s) = &self.update_status {
            lines.push(Line::from(Span::styled(format!("  {s}"), dim)));
        }
        let p = Paragraph::new(Text::from(lines)).block(
            Block::default().borders(Borders::ALL).title(" about "),
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

    /// The scope capsules plus the trailing "+ new" chip, in draw order.
    fn scope_strip_items(&self) -> Vec<(String, usize, bool)> {
        let mut v: Vec<(String, usize, bool)> = self
            .scopes
            .iter()
            .enumerate()
            .map(|(i, s)| (s.label().to_string(), i, true))
            .collect();
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
    fn scope_strip_height(&self, width: u16) -> u16 {
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
    fn draw_scope_strip(&mut self, f: &mut Frame, area: Rect) {
        // font-height capsules, separated from the table by a blank row
        // above and below (glyph-built "taller" capsules read as
        // corruption across fonts)
        self.scope_rects.clear();
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
            let r = Rect { x, y, width: wl, height: 1 };
            self.scope_rects.push((r, idx));
            let hov = hit(r, self.hover.0, self.hover.1);
            if hov && idx == FILTER_CHIP {
                self.hover_hint =
                    Some("⌕ active filter — click to edit  ·  /  (Esc clears)".to_string());
            }
            let (bg, fg) = if idx == FILTER_CHIP {
                if hov {
                    (Color::Rgb(70, 62, 30), Color::Yellow)
                } else {
                    (Color::Rgb(52, 47, 26), Color::Yellow)
                }
            } else if idx == self.active_scope {
                (Color::Cyan, Color::Black)
            } else if hov {
                (Color::Rgb(58, 63, 72), Color::White)
            } else if idx == usize::MAX {
                (Color::Rgb(40, 44, 52), Color::DarkGray)
            } else {
                (Color::Rgb(40, 44, 52), Color::Gray)
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

    /// The one-cell metric swatch strip beside the table: each row's
    /// scalar rank-normalized over the visible set and mapped through
    /// the active metric's colormap. Its header cell shows the
    /// colormap midpoint and sorts by the metric.
    fn draw_metric_strip(&mut self, f: &mut Frame, area: Rect) {
        self.metric_area = area;
        let metric = self.metric_col;
        // header swatch: legend + sort target
        let hr = Rect { x: area.x, y: area.y, width: 1, height: 1 };
        self.sort_headers.push((hr, SortCol::Metric));
        if hit(hr, self.hover.0, self.hover.1) {
            self.hover_hint = Some(format!("sort by metric: {}", metric.name()));
        }
        let hdr = if self.sort.0 == SortCol::Metric {
            Span::styled(
                if self.sort.1 { "▲" } else { "▼" },
                Style::default().fg(metric_color(metric, 0.7)),
            )
        } else {
            Span::styled(" ", Style::default().bg(metric_color(metric, 0.5)))
        };
        f.render_widget(Paragraph::new(Line::from(hdr)), hr);

        // per-row values over the visible window
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let offset = self.table.offset();
        let rows = area.height.saturating_sub(2) as usize;
        let values: Vec<Option<f64>> = match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => articles
                .iter()
                .map(|a| match metric {
                    MetricCol::Citations => a.citation_count.map(|c| c as f64),
                    MetricCol::Priority => self
                        .lib
                        .get_by_bibcode(&a.bibcode)
                        .and_then(|e| self.metrics.get(e.key()))
                        .and_then(|m| m.effective_priority(now_ts)),
                    MetricCol::Off => None,
                })
                .collect(),
            _ => self
                .filtered
                .iter()
                .filter_map(|&i| self.lib.get(&self.order[i]))
                .map(|e| {
                    let m = self.metrics.get(e.key());
                    match metric {
                        MetricCol::Priority => {
                            m.and_then(|m| m.effective_priority(now_ts))
                        }
                        MetricCol::Citations => m.and_then(|m| m.citations).map(|c| c as f64),
                        MetricCol::Off => None,
                    }
                })
                .collect(),
        };
        let known: Vec<f64> = values.iter().flatten().copied().collect();
        for (row, v) in values.iter().skip(offset).take(rows).enumerate() {
            let y = area.y + 2 + row as u16;
            let cell = match v {
                Some(v) => {
                    // priority IS a 0..1 level — color it absolutely so
                    // edits recolor in place; citations rank-normalize
                    let t = match metric {
                        MetricCol::Priority => *v,
                        _ => rank_norm(&known, *v),
                    };
                    Span::styled(" ", Style::default().bg(metric_color(metric, t)))
                }
                None => Span::styled("·", divider()),
            };
            f.render_widget(Paragraph::new(Line::from(cell)), Rect {
                x: area.x,
                y,
                width: 1,
                height: 1,
            });
        }
    }

    fn draw_table(&mut self, f: &mut Frame, area: Rect) {
        use ratatui::widgets::Cell;
        self.table_area = area;
        self.sort_headers.clear();
        if let Some(Scope::Manuscript { rows }) = self.scopes.get(self.active_scope) {
            // manuscript view: cited string, state, resolved title
            use crate::library::CiteState;
            let cursor = self.table.selected();
            let hov_row = self.hovered_table_pos();
            let trows: Vec<Row> = rows
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
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(Color::Cyan)
                    };
                    let title_style = if lit {
                        Style::default().fg(Color::White).add_modifier(Modifier::ITALIC)
                    } else {
                        Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC)
                    };
                    let row = Row::new(vec![
                        Cell::from(Span::styled(
                            if cursor == Some(pos) { "◉" } else { "" },
                            Style::default().fg(Color::Cyan),
                        )),
                        Cell::from(Span::styled(icon, style)),
                        Cell::from(Span::styled(r.cited.clone(), cite_style)),
                        Cell::from(Span::styled(word, style)),
                        Cell::from(Span::styled(r.title.clone(), title_style)),
                    ]);
                    if cursor == Some(pos) {
                        row.style(Style::default().bg(Color::Rgb(34, 40, 52)))
                    } else {
                        row
                    }
                })
                .collect();
            let header: Vec<Span> = ["", "", "Cited", "State", "Title"]
                .iter()
                .zip([2u16, 2, 26, 10, 0])
                .flat_map(|(l, w)| {
                    let pad = (w as usize).saturating_sub(l.chars().count());
                    [
                        Span::styled(*l, Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw(" ".repeat(pad + 1)),
                    ]
                })
                .collect();
            f.render_widget(
                Paragraph::new(Line::from(header)),
                Rect { x: area.x, y: area.y, width: area.width, height: 1 },
            );
            f.render_widget(
                Paragraph::new(Span::styled(
                    "─".repeat(area.width as usize),
                    divider(),
                )),
                Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 },
            );
            let data_area = Rect {
                x: area.x,
                y: area.y + 2,
                width: area.width,
                height: area.height.saturating_sub(2),
            };
            let table = Table::new(
                trows,
                [
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(26),
                    Constraint::Length(10),
                    Constraint::Min(20),
                ],
            )
            .block(Block::default().borders(Borders::NONE));
            f.render_stateful_widget(table, data_area, &mut self.table);
            return;
        }
        if let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) {
            // ADS results: ↓ ● Year Author Title; ● from the bibcode
            // index, ↓ from the canonical cache key (cite key once
            // imported, bibcode otherwise)
            let (author_w, _) = column_layout(area.width);
            let cursor = self.table.selected();
            let hov_row = self.hovered_table_pos();
            let rows: Vec<Row> = articles
                .iter()
                .enumerate()
                .map(|(pos, a)| {
                    let entry = self.lib.get_by_bibcode(&a.bibcode);
                    let cache_key = entry.map(|e| e.key()).unwrap_or(&a.bibcode);
                    let author = a.author.join(" and ");
                    let lit = hov_row == Some(pos);
                    let (au_style, ti_style, yr_style) = if lit {
                        (
                            Style::default().fg(Color::White),
                            Style::default().fg(Color::White).add_modifier(Modifier::ITALIC),
                            Style::default().fg(Color::Green),
                        )
                    } else {
                        (
                            Style::default().fg(TABLE_TEXT),
                            Style::default().fg(TABLE_TEXT).add_modifier(Modifier::ITALIC),
                            Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
                        )
                    };
                    let circle = if self.select_mode {
                        if self.selected.contains(&a.bibcode) { "◉" } else { "◯" }
                    } else if cursor == Some(pos) {
                        "◉"
                    } else {
                        ""
                    };
                    let circle_style = if circle == "◯" {
                        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)
                    } else if self.select_mode && cursor == Some(pos) {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Cyan)
                    };
                    let row = Row::new(vec![
                        Cell::from(Span::styled(circle, circle_style)),
                        Cell::from(Span::styled(
                            if pdf::is_cached(cache_key) { "↓" } else { "" },
                            Style::default().fg(Color::Green),
                        )),
                        Cell::from(Span::styled(
                            if entry.is_some() { "●" } else { "" },
                            Style::default().fg(Color::Magenta),
                        )),
                        Cell::from(Span::styled(a.year.clone(), yr_style)),
                        Cell::from(Span::styled(
                            fit_authors(&author, author_w as usize),
                            au_style,
                        )),
                        Cell::from(Span::styled(a.title.clone(), ti_style)),
                        Cell::from(Span::styled(
                            self.hypothetical_key(a),
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                        )),
                    ]);
                    if cursor == Some(pos) {
                        row.style(Style::default().bg(Color::Rgb(34, 40, 52)))
                    } else {
                        row
                    }
                })
                .collect();
            let (sort_col, asc) = self.sort;
            let mut hx = area.x;
            let mut header: Vec<Span> = vec![];
            for (ci, base) in ["", "↓", "●", "Year", "Author", "Title", "Key"].iter().enumerate() {
                let w = [2u16, 2, 2, 6, author_w, 0, 20][ci];
                let cw = if ci == 5 {
                    area.width
                        .saturating_sub(hx - area.x)
                        .saturating_sub(21)
                } else {
                    w
                };
                let col = match ci {
                    1 => Some(SortCol::Pdf),
                    2 => Some(SortCol::InLib),
                    3 => Some(SortCol::Year),
                    4 => Some(SortCol::Author),
                    5 => Some(SortCol::Title),
                    6 => Some(SortCol::Key),
                    _ => None,
                };
                let mut label = base.to_string();
                let mut style = Style::default().add_modifier(Modifier::BOLD);
                if let Some(col) = col {
                    let r = Rect { x: hx, y: area.y, width: cw.max(1), height: 1 };
                    self.sort_headers.push((r, col));
                    if sort_col == col {
                        let arrow = if asc { "▲" } else { "▼" };
                        label = if cw <= 2 { format!("{base}{arrow}") } else { format!("{base} {arrow}") };
                    }
                    if hit(r, self.hover.0, self.hover.1) {
                        style = style.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED);
                    }
                }
                let pad = (cw as usize).saturating_sub(label.chars().count());
                header.push(Span::styled(label, style));
                header.push(Span::raw(" ".repeat(pad.min(200) + 1)));
                hx += cw + 1;
            }
            f.render_widget(
                Paragraph::new(Line::from(header)),
                Rect { x: area.x, y: area.y, width: area.width, height: 1 },
            );
            f.render_widget(
                Paragraph::new(Span::styled(
                    "─".repeat(area.width as usize),
                    divider(),
                )),
                Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 },
            );
            let data_area = Rect {
                x: area.x,
                y: area.y + 2,
                width: area.width,
                height: area.height.saturating_sub(2),
            };
            let table = Table::new(
                rows,
                [
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(6),
                    Constraint::Length(author_w),
                    Constraint::Min(20),
                    Constraint::Length(20),
                ],
            )
            .block(Block::default().borders(Borders::NONE));
            f.render_stateful_widget(table, data_area, &mut self.table);
            if self.row_count() == 0 {
                draw_empty_hint(f, data_area, "no results — r re-runs, +/- changes n");
            }
            return;
        }
        // subtle per-column palette; the terminal theme supplies the hues.
        // cursor row: faint cool fill ("standing on a surface") + ◉;
        // hovered row: no fill — the text lifts one level instead
        let cursor_fill = Style::default().bg(Color::Rgb(34, 40, 52));
        let palette = |lit: bool| {
            if lit {
                (
                    Style::default().fg(Color::Cyan),
                    Style::default().fg(Color::Green),
                    Style::default().fg(Color::Magenta),
                    Style::default().fg(Color::Green),
                    Style::default().fg(Color::White),
                    Style::default().fg(Color::Cyan),
                )
            } else {
                (
                    Style::default().fg(Color::Cyan),
                    Style::default().fg(Color::Green),
                    Style::default().fg(Color::Magenta),
                    Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
                    Style::default().fg(TABLE_TEXT),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
                )
            }
        };
        // responsive columns: author scales, Key drops first when tight —
        // but never while the card is shown: the Key column is the
        // hover-preview target, so the title absorbs the squeeze instead
        let (author_w, show_key) = column_layout(area.width);
        let show_key = show_key || self.show_detail;
        let hov_row = self.hovered_table_pos();
        let cursor = self.table.selected();
        let show_membership = self.lib.manuscript.is_some() && self.lib.global_on;
        let rows: Vec<Row> = self
            .filtered
            .iter()
            .enumerate()
            .map(|(pos, &i)| {
                let e = self.lib.get(&self.order[i]).unwrap();
                let at_cursor = cursor == Some(pos);
                let lit = hov_row == Some(pos);
                let (c_ind, c_pdf, c_ms, c_year, c_author, c_key) = palette(lit);
                // the gutter carries the cursor: ◉ marks the cursor row
                // (no row highlight, so cell colors stay visible); in
                // selection mode the cursor row's circle brightens
                let circle = if !self.select_mode {
                    if at_cursor { "◉" } else { "" }
                } else if self.selected.contains(e.key()) {
                    "◉"
                } else {
                    "◯"
                };
                // unselected rings recede almost entirely; selected dots pop
                let circle_style = if circle == "◯" {
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)
                } else if self.select_mode && at_cursor {
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                } else {
                    c_ind
                };
                let mut cells = vec![
                    Cell::from(Span::styled(circle, circle_style)),
                    Cell::from(Span::styled(
                        if has_cached_pdf(e.key()) { "↓" } else { "" },
                        c_pdf,
                    )),
                    Cell::from(Span::styled(
                        if show_membership && self.lib.in_manuscript(e.key()) { "●" } else { "" },
                        c_ms,
                    )),
                    Cell::from(Span::styled(e.year(), c_year)),
                    Cell::from(Span::styled(
                        fit_authors(e.author(), author_w as usize),
                        c_author,
                    )),
                    Cell::from(Span::styled(
                        e.title().trim_matches(['{', '}']).to_string(),
                        if lit {
                            Style::default().fg(Color::White).add_modifier(Modifier::ITALIC)
                        } else {
                            Style::default().fg(TABLE_TEXT).add_modifier(Modifier::ITALIC)
                        },
                    )),
                ];
                if show_key {
                    cells.push(Cell::from(Span::styled(e.short_key.clone(), c_key)));
                }
                let row = Row::new(cells);
                if at_cursor {
                    row.style(cursor_fill)
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
        let ms_header = if show_membership { "●" } else { "" };
        let mut headers: Vec<&str> = vec!["", "↓", ms_header, "Year", "Author", "Title"];
        if show_key {
            headers.push("Key");
        }
        let mut header_spans: Vec<Span> = vec![];
        for (ci, base) in headers.iter().enumerate() {
            let cw = if ci == 5 { title_w } else { widths[ci] };
            let col = match ci {
                1 => Some(SortCol::Pdf),
                2 if show_membership => Some(SortCol::InLib),
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
                divider(),
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
        .block(Block::default().borders(Borders::NONE));
        f.render_stateful_widget(table, data_area, &mut self.table);
        if self.order.is_empty() {
            draw_empty_hint(
                f,
                data_area,
                "library is empty — S searches ADS, or: astrobib add <bibcode>",
            );
        } else if self.filtered.is_empty() {
            draw_empty_hint(f, data_area, "no matches — Esc clears the filter");
        }
    }

    /// The pub card for an ADS result: body, links, citation count, an
    /// import button, and click-to-copy regions like the library card.
    /// The card ⇄ bib-source toggler, pinned to the card's bottom-right
    /// corner: a segmented "▤ card │ @ bib" control — the active side
    /// underlined, the inactive side dimmed and clickable (v toggles).
    fn draw_card_toggle(&mut self, f: &mut Frame, x0: u16, w: u16, bottom: u16, source: bool) {
        let segs: [(&str, bool); 2] = [("▤ card", false), ("@ bib", true)];
        let total: u16 = segs.iter().map(|(l, _)| l.chars().count() as u16).sum::<u16>() + 3;
        let y = bottom.saturating_sub(1);
        let mut x = x0 + w.saturating_sub(total);
        for (i, (label, is_bib)) in segs.iter().enumerate() {
            if i > 0 {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(" │ ", divider()))),
                    Rect { x, y, width: 3, height: 1 },
                );
                x += 3;
            }
            let lw = label.chars().count() as u16;
            let r = Rect { x, y, width: lw, height: 1 };
            let active = *is_bib == source;
            let style = if active {
                Style::default().add_modifier(Modifier::UNDERLINED)
            } else {
                let hov = hit(r, self.hover.0, self.hover.1);
                self.card_buttons.push((r, CardBtn::BibView));
                if hov {
                    self.hover_hint = Some(if *is_bib {
                        card_hint(CardBtn::BibView).to_string()
                    } else {
                        "▤ back to the formatted card  ·  v".to_string()
                    });
                    Style::default().fg(Color::Green).add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::default().fg(Color::DarkGray)
                }
            };
            f.render_widget(Paragraph::new(Line::from(Span::styled(*label, style))), r);
            x += lw;
        }
    }

    /// The verbatim .bib file in place of the formatted card (v / @ bib),
    /// with the permanent ⧉ copy menu pinned above the bottom.
    fn draw_bib_source(&mut self, f: &mut Frame, area: Rect, key: &str) {
        let Some(e) = self.lib.get(key) else { return };
        let path = e.path.clone();
        let copies: Vec<(String, LinkTarget, bool)> = vec![
            ("cite key".into(), LinkTarget::Copy(CopyItem::Key), true),
            ("full key".into(), LinkTarget::Copy(CopyItem::FullKey), true),
            ("bibcode".into(), LinkTarget::Copy(CopyItem::Bibcode), e.bibcode().is_some()),
            ("ADS URL".into(), LinkTarget::Copy(CopyItem::AdsUrl), !e.adsurl().is_empty()),
            ("arXiv URL".into(), LinkTarget::Copy(CopyItem::ArxivUrl), !e.eprint().is_empty()),
            ("DOI URL".into(), LinkTarget::Copy(CopyItem::DoiUrl), !e.doi().is_empty()),
            ("PDF path".into(), LinkTarget::Copy(CopyItem::PdfPath), pdf::is_cached(key)),
        ];
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| "could not read file".to_string());
        self.draw_bib_panel(f, area, &format!("bib/{name}"), &text, copies);
    }

    /// Shared renderer for the verbatim-BibTeX views: header line,
    /// soft-wrapped body, the ⧉ copy stack, the pinned toggler.
    fn draw_bib_panel(
        &mut self,
        f: &mut Frame,
        area: Rect,
        header: &str,
        text: &str,
        copies: Vec<(String, LinkTarget, bool)>,
    ) {
        let x0 = area.x + 3;
        let w = area.width.saturating_sub(5);
        let bottom = area.y + area.height;
        let mut y = area.y + 1;
        let dim = Style::default().fg(Color::DarkGray);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(header.to_string(), dim))),
            Rect { x: x0, y, width: w, height: 1 },
        );
        y += 2;
        // the copy stack sits above the toggler row; content stops there
        let stack_h = copies.len() as u16 + 2; // sep + rows + air
        let content_end = bottom.saturating_sub(stack_h + 1);
        let mut rows: Vec<String> = vec![];
        for line in text.lines() {
            if line.trim().is_empty() {
                rows.push(String::new());
            } else {
                rows.extend(wrap_text(line, w as usize));
            }
        }
        let avail = content_end.saturating_sub(y) as usize;
        let (shown, above, below) = scroll_window(rows, avail, &mut self.card_scroll);
        let (first, last) = (y, y + shown.len().saturating_sub(1) as u16);
        for row in shown {
            f.render_widget(
                Paragraph::new(Line::from(Span::raw(row))),
                Rect { x: x0, y, width: w, height: 1 },
            );
            y += 1;
        }
        if above {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled("↑", divider()))),
                Rect { x: x0.saturating_sub(2), y: first, width: 1, height: 1 },
            );
        }
        if below {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled("↓", divider()))),
                Rect { x: x0.saturating_sub(2), y: last, width: 1, height: 1 },
            );
        }
        // the stack follows the text (short files pull it up; long
        // ones scroll within the reserved window)
        let mut y = y + 1;
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("─".repeat(w as usize), divider()))),
            Rect { x: x0, y, width: w, height: 1 },
        );
        y += 1;
        let mut yanks: Vec<(Rect, CopyItem)> = vec![];
        draw_link_stack(
            f,
            x0,
            y,
            w,
            bottom,
            self.hover,
            copies,
            vec![],
            &mut self.card_links,
            &mut self.card_buttons,
            &mut self.hover_hint,
            &mut yanks,
        );
        self.card_yanks = yanks;
        self.draw_card_toggle(f, x0, w, bottom, true);
    }

    fn draw_article_card(&mut self, f: &mut Frame, area: Rect) {
        self.card_buttons.clear();
        self.card_links.clear();
        let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) else {
            self.card_yanks.clear();
            return;
        };
        let Some(a) = self.card_article_pos().and_then(|p| articles.get(p)) else {
            self.card_yanks.clear();
            return;
        };
        let title = a.title.clone();
        let authors = a.author.join(" and ");
        let year = a.year.clone();
        let abstract_ = crate::ads::clean_abstract(&a.abstract_);
        let bibcode = a.bibcode.clone();
        let doi = a.doi.first().cloned().unwrap_or_default();
        let eprint = crate::ads::arxiv_id(a).map(str::to_string).unwrap_or_default();
        let cites = a.citation_count;
        let hyp_key = self.hypothetical_key(a);
        let lib_key = self.lib.get_by_bibcode(&bibcode).map(|e| e.key().to_string());
        let in_lib = lib_key.is_some();
        // v on a query result: an imported twin shows its real file; an
        // un-imported article previews the exact canonical BibTeX an
        // import would write (fetched once, cached by bibcode)
        if self.show_bib_source {
            if let Some(k) = &lib_key {
                let k = k.clone();
                self.draw_bib_source(f, area, &k);
                return;
            }
            let copies: Vec<(String, LinkTarget, bool)> = vec![
                ("cite key".into(), LinkTarget::Copy(CopyItem::Key), true),
                ("bibcode".into(), LinkTarget::Copy(CopyItem::Bibcode), true),
                ("ADS URL".into(), LinkTarget::Copy(CopyItem::AdsUrl), true),
                ("arXiv URL".into(), LinkTarget::Copy(CopyItem::ArxivUrl), !eprint.is_empty()),
                ("DOI URL".into(), LinkTarget::Copy(CopyItem::DoiUrl), !doi.is_empty()),
            ];
            match self.bib_preview.get(&bibcode).cloned() {
                Some(text) => self.draw_bib_panel(
                    f,
                    area,
                    "canonical BibTeX — what import would write",
                    &text,
                    copies,
                ),
                None => {
                    self.request_bib_preview(bibcode.clone());
                    self.draw_bib_panel(f, area, "fetching BibTeX…", "", copies);
                }
            }
            return;
        }

        let hv = self.hover;
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

        let x0 = area.x + 3;
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
        for l in wrap_text(&title, w) {
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
        let byline = format!("{}   ·   {year}", format_authors(&authors));
        let dim = Style::default().fg(Color::DarkGray);
        for l in wrap_text(&byline, w) {
            line_at(f, y, Line::from(Span::styled(l, dim)));
            y += 1;
        }
        if !a.journal.is_empty() {
            let mut publine = a.journal.clone();
            if !a.volume.is_empty() {
                publine.push_str(&format!(" {}", a.volume));
            }
            if !a.issue.is_empty() {
                publine.push_str(&format!("({})", a.issue));
            }
            if !a.page.is_empty() {
                publine.push_str(&format!(", {}", a.page));
            }
            for l in wrap_text(&publine, w) {
                line_at(f, y, Line::from(Span::styled(l, dim)));
                y += 1;
            }
        }
        // "?" invites a refresh only when there is an imported entry to
        // refresh into; otherwise the line appears only with a count
        if cites.is_some() || lib_key.is_some() {
            if y < bottom {
                draw_cited_line(
                    f, x0, y, w as u16, cites, self.hover,
                    &mut self.card_buttons, &mut self.hover_hint,
                );
            }
            y += 1;
        }
        // the vertical link stack: browser links plus the citation-graph
        // queries (the ⌕ rows act inside astrobib)
        let mut stack: Vec<(String, LinkTarget, bool)> = vec![
            (
                "ADS".into(),
                LinkTarget::Url(format!("https://ui.adsabs.harvard.edu/abs/{bibcode}/abstract")),
                true,
            ),
            (
                if eprint.is_empty() { "arXiv".to_string() } else { format!("arXiv:{eprint}") },
                LinkTarget::Url(format!("https://arxiv.org/abs/{eprint}")),
                !eprint.is_empty(),
            ),
            (
                "DOI".into(),
                LinkTarget::Url(format!("https://doi.org/{doi}")),
                !doi.is_empty(),
            ),
            ("citations".into(), LinkTarget::Query(CardBtn::Citations), true),
            ("references".into(), LinkTarget::Query(CardBtn::Refs), true),
        ];
        // the manuscript ± affordance the library card carries, so m has
        // a visible twin here too; it acts on the imported entry, and
        // dims (with a hint) while there is none
        if self.lib.manuscript.is_some() {
            let in_ms = lib_key.as_deref().is_some_and(|k| self.lib.in_manuscript(k));
            stack.push((
                if in_ms { "in manuscript".into() } else { "add to manuscript".to_string() },
                LinkTarget::Query(CardBtn::MsToggle),
                in_lib && self.lib.global_on,
            ));
        }
        // prose has no multi-item form: those rows dim while several
        // rows are selected (the others copy across the selection)
        let multi = self.select_mode && self.selected.len() > 1;
        let copies: Vec<(String, LinkTarget, bool)> = vec![
            ("cite key".into(), LinkTarget::Copy(CopyItem::Key), true),
            ("bibcode".into(), LinkTarget::Copy(CopyItem::Bibcode), true),
            ("ADS URL".into(), LinkTarget::Copy(CopyItem::AdsUrl), true),
            ("arXiv URL".into(), LinkTarget::Copy(CopyItem::ArxivUrl), !eprint.is_empty()),
            ("DOI URL".into(), LinkTarget::Copy(CopyItem::DoiUrl), !doi.is_empty()),
            (
                "PDF path".into(),
                LinkTarget::Copy(CopyItem::PdfPath),
                lib_key.as_deref().is_some_and(pdf::is_cached),
            ),
            ("title".into(), LinkTarget::Copy(CopyItem::Title), !multi),
            (
                "abstract".into(),
                LinkTarget::Copy(CopyItem::Abstract),
                !multi && !abstract_.is_empty(),
            ),
        ];
        // action block + footer, plus PDF buttons + status once imported
        let rest = 2 + stack.len().max(copies.len()) as u16 + 2 + if in_lib { 2 } else { 0 };
        if !abstract_.is_empty() && y + rest < bottom {
            y += 1;
            let avail = (bottom - y).saturating_sub(rest) as usize;
            let (shown, above, below) =
                scroll_window(wrap_text(&abstract_, w), avail, &mut self.card_scroll);
            let (first, last) = (y, y + shown.len().saturating_sub(1) as u16);
            for l in shown {
                let lw = l.chars().count() as u16;
                yanks.push((
                    Rect { x: x0, y, width: lw.max(1), height: 1 },
                    CopyItem::Abstract,
                ));
                line_at(
                    f,
                    y,
                    Line::from(Span::styled(l, region_style(Style::default().fg(ABSTRACT_TEXT), CopyItem::Abstract))),
                );
                y += 1;
            }
            let mark = |f: &mut Frame, my: u16, s: &str| {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(s, divider()))),
                    Rect { x: x0.saturating_sub(2), y: my, width: 1, height: 1 },
                );
            };
            if above {
                mark(f, first, "↑");
            }
            if below {
                mark(f, last, "↓");
            }
        }
        let sep = "─".repeat(w);
        let dimsep = divider();
        line_at(f, y, Line::from(Span::styled(sep.clone(), dimsep)));
        y += 1;
        y = draw_link_stack(f, x0, y, w as u16, bottom, self.hover, stack, copies, &mut self.card_links, &mut self.card_buttons, &mut self.hover_hint, &mut yanks);
        line_at(f, y, Line::from(Span::styled(sep, dimsep)));
        y += 2;
        // once imported, the library card's PDF buttons appear here too,
        // acting on the imported entry (card_entry_key routes clicks)
        if let Some(kk) = &lib_key {
            let cached = pdf::is_cached(kk);
            let mut buttons: Vec<(&str, CardBtn, Color)> = vec![];
            if !cached {
                if !eprint.is_empty() {
                    buttons.push(("arXiv ↓", CardBtn::Arxiv, Color::Cyan));
                }
                buttons.push(("ADS OA ↓", CardBtn::Oa, Color::Cyan));
                buttons.push(("browser ↓", CardBtn::Browser, Color::Yellow));
                buttons.push(("pick …", CardBtn::Pick, Color::Magenta));
            } else {
                buttons.push(("Open ↗", CardBtn::Open, Color::Green));
                buttons.push(("Clear ✕", CardBtn::Clear, Color::Gray));
            }
            let mut bspans: Vec<Span> = vec![];
            let mut bx = x0;
            for (label, btn, fg) in buttons {
                let wl = pill_width(label);
                let r = Rect { x: bx, y, width: wl, height: 1 };
                if y < bottom {
                    self.card_buttons.push((r, btn));
                }
                let hovb = hit(r, hv.0, hv.1);
                if hovb {
                    self.hover_hint = Some(card_hint(btn).to_string());
                }
                let bg = if hovb { Color::Rgb(58, 63, 72) } else { Color::Rgb(40, 44, 52) };
                push_pill(&mut bspans, label, bg, fg);
                bspans.push(Span::raw(" "));
                bx += wl + 1;
            }
            line_at(f, y, Line::from(bspans));
            y += 1;
            if self.poll_cancel.is_some() {
                let label = "◌ waiting for download…  cancel ✕";
                if y < bottom {
                    self.card_buttons.push((
                        Rect { x: x0, y, width: label.chars().count() as u16, height: 1 },
                        CardBtn::Cancel,
                    ));
                }
                line_at(f, y, Line::from(Span::styled(label, Style::default().fg(Color::Yellow))));
            } else if !self.pdf_status.is_empty() {
                line_at(
                    f,
                    y,
                    Line::from(Span::styled(
                        self.pdf_status.clone(),
                        Style::default().fg(Color::Yellow),
                    )),
                );
            }
            y += 1;
        }
        let cyan = Style::default().fg(Color::Cyan);
        let mut foot: Vec<Span> = vec![Span::styled(
            hyp_key.clone(),
            region_style(cyan, CopyItem::Key),
        )];
        yanks.push((
            Rect { x: x0, y, width: hyp_key.chars().count() as u16, height: 1 },
            CopyItem::Key,
        ));
        if in_lib {
            let label = "● in library";
            let key_w = hyp_key.chars().count() as u16;
            let r = Rect {
                x: x0 + key_w + 2,
                y,
                width: label.chars().count() as u16,
                height: 1,
            };
            self.card_buttons.push((r, CardBtn::RemoveFromLib));
            let hov = hit(r, hv.0, hv.1);
            if hov {
                self.hover_hint = Some(card_hint(CardBtn::RemoveFromLib).to_string());
            }
            let style = if hov {
                Style::default().fg(Color::Red).add_modifier(Modifier::UNDERLINED)
            } else {
                Style::default().fg(Color::Magenta)
            };
            foot.push(Span::raw("  "));
            foot.push(Span::styled(label, style));
        } else {
            // the footer arrow IS the import affordance (i also works)
            let key_w = hyp_key.chars().count() as u16;
            let label = "→ import";
            let r = Rect {
                x: x0 + key_w + 2,
                y,
                width: label.chars().count() as u16,
                height: 1,
            };
            self.card_buttons.push((r, CardBtn::Import));
            let hovb = hit(r, hv.0, hv.1);
            if hovb {
                self.hover_hint = Some(card_hint(CardBtn::Import).to_string());
            }
            let style = if hovb {
                Style::default().fg(Color::Green).add_modifier(Modifier::UNDERLINED)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            foot.push(Span::raw("  "));
            foot.push(Span::styled(label, style));
        }
        line_at(f, y, Line::from(foot));
        self.card_yanks = yanks;
        self.draw_card_toggle(f, x0, w as u16, bottom, false);
    }

    /// The pub card, top to bottom: body (title,
    /// authors · year, abstract), a bordered links row (ADS · arXiv:id ·
    /// DOI, cyan, browser-opening), the PDF buttons (ineligible ones
    /// hidden, not dimmed), a transient PDF status line,
    /// and a footer (keywords, cite key with dim hash suffix, preprint
    /// note). Text is pre-wrapped so every row's click rect is exact.
    fn draw_detail(&mut self, f: &mut Frame, area: Rect) {
        self.card_area = area;
        // a different paper (or view) resets the scroll
        let shown = self.card_key().map(str::to_string);
        if shown != self.card_shown {
            self.card_shown = shown;
            self.card_scroll = 0;
        }
        self.card_links.clear();
        f.render_widget(
            Block::default().borders(Borders::LEFT).border_style(divider()),
            area,
        );
        if self.active_ads().is_some() {
            self.draw_article_card(f, area);
            return;
        }
        let Some(key) = self.card_key().map(str::to_string) else {
            return; // unresolved manuscript rows have nothing to show
        };
        let Some(e) = self.lib.get(&key) else { return };
        if self.show_bib_source {
            let key = key.clone();
            self.draw_bib_source(f, area, &key);
            return;
        }
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
        // journal · volume(issue), pages — under the byline
        let journal = crate::export::journal_name(e.journal());
        if !journal.is_empty() {
            let mut publine = journal.clone();
            if !e.volume().is_empty() {
                publine.push_str(&format!(" {}", e.volume()));
            }
            if !e.number().is_empty() {
                publine.push_str(&format!("({})", e.number()));
            }
            if !e.pages().is_empty() {
                publine.push_str(&format!(", {}", e.pages()));
            }
            for l in wrap_text(&publine, w) {
                line_at(f, y, Line::from(Span::styled(l, Style::default().fg(Color::DarkGray))));
                y += 1;
            }
        }
        let cite_n = self.metrics.get(&key).and_then(|m| m.citations);
        if y < bottom {
            draw_cited_line(
                f, x0, y, w as u16, cite_n, self.hover,
                &mut self.card_buttons, &mut self.hover_hint,
            );
        }
        y += 1;
        let (eprint, adsurl, doi) = (
            e.eprint().to_string(),
            e.adsurl().to_string(),
            e.doi().to_string(),
        );
        let multi_sel = self.select_mode && self.selected.len() > 1;
        // the vertical link stack: browser links plus the citation-graph
        // pair — "citations" / "references" spawn ADS query scopes
        // rather than opening the browser, so they register as card
        // buttons; built here because the layout budget needs its height
        // every operation stays visible; unavailable ones render dimmed
        let links_row: Vec<(String, LinkTarget, bool)> = vec![
            ("ADS".into(), LinkTarget::Url(adsurl.clone()), !adsurl.is_empty()),
            (
                if eprint.is_empty() { "arXiv".to_string() } else { format!("arXiv:{eprint}") },
                LinkTarget::Url(format!("https://arxiv.org/abs/{eprint}")),
                !eprint.is_empty(),
            ),
            (
                "DOI".into(),
                LinkTarget::Url(format!("https://doi.org/{doi}")),
                !doi.is_empty(),
            ),
            ("citations".into(), LinkTarget::Query(CardBtn::Citations), !adsurl.is_empty()),
            ("references".into(), LinkTarget::Query(CardBtn::Refs), !adsurl.is_empty()),
        ];
        // the permanent copy menu (the y-chord's targets), right column
        let copies: Vec<(String, LinkTarget, bool)> = vec![
            ("cite key".into(), LinkTarget::Copy(CopyItem::Key), true),
            ("full key".into(), LinkTarget::Copy(CopyItem::FullKey), true),
            ("bibcode".into(), LinkTarget::Copy(CopyItem::Bibcode), e.bibcode().is_some()),
            ("ADS URL".into(), LinkTarget::Copy(CopyItem::AdsUrl), !adsurl.is_empty()),
            ("arXiv URL".into(), LinkTarget::Copy(CopyItem::ArxivUrl), !eprint.is_empty()),
            ("DOI URL".into(), LinkTarget::Copy(CopyItem::DoiUrl), !doi.is_empty()),
            ("PDF path".into(), LinkTarget::Copy(CopyItem::PdfPath), pdf::is_cached(&key)),
            (
                "title".into(),
                LinkTarget::Copy(CopyItem::Title),
                !multi_sel && !e.title().is_empty(),
            ),
            (
                "abstract".into(),
                LinkTarget::Copy(CopyItem::Abstract),
                !multi_sel && !e.abstract_().is_empty(),
            ),
        ];
        let link_lines = links_row.len().max(copies.len()) as u16;
        // footer + links/buttons need this much room below the abstract
        let kws = e.keywords().join(" · ");
        let kw_lines = if kws.is_empty() { 0 } else { wrap_text(&kws, w).len() as u16 + 1 };
        let has_ms = self.lib.manuscript.is_some() && self.lib.global_on;
        // link stack (sep + rows + sep + air) + buttons + ms chip +
        // status (collapses to a single spacer row when idle) +
        // keywords + key line
        let status_rows: u16 =
            if self.poll_cancel.is_some() || !self.pdf_status.is_empty() { 2 } else { 1 };
        let rest = 3 + link_lines + 1 + u16::from(has_ms) + status_rows + kw_lines + 1;
        let abs = e.abstract_();
        if !abs.is_empty() && y + rest < bottom {
            y += 1;
            // truncation is height-driven only: the full abstract shows
            // whenever the card has room, and a cut ends in an ellipsis
            let avail = (bottom - y).saturating_sub(rest) as usize;
            let (shown, above, below) =
                scroll_window(wrap_text(abs, w), avail, &mut self.card_scroll);
            let (first, last) = (y, y + shown.len().saturating_sub(1) as u16);
            for l in shown {
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
                        region_style(Style::default().fg(ABSTRACT_TEXT), CopyItem::Abstract),
                    )),
                );
                y += 1;
            }
            // margin markers: more text above/below (wheel scrolls)
            let mark = |f: &mut Frame, my: u16, s: &str| {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(s, divider()))),
                    Rect { x: x0.saturating_sub(2), y: my, width: 1, height: 1 },
                );
            };
            if above {
                mark(f, first, "↑");
            }
            if below {
                mark(f, last, "↓");
            }
        }

        // ── link stack (bordered top and bottom, like #detail-links),
        //    right below the abstract ──
        let sep = "─".repeat(w);
        let dimsep = divider();
        line_at(f, y, Line::from(Span::styled(sep.clone(), dimsep)));
        y += 1;
        y = draw_link_stack(f, x0, y, w as u16, bottom, self.hover, links_row, copies, &mut self.card_links, &mut self.card_buttons, &mut self.hover_hint, &mut yanks);
        line_at(f, y, Line::from(Span::styled(sep, dimsep)));
        y += 2; // a little air below the link stack

        // ── PDF buttons, drawn as rounded pills; only the buttons whose
        //    source is available appear ─────────────────────────────────
        let cached = pdf::is_cached(&key);
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
            let hovb = hit(r, hv.0, hv.1);
            if hovb {
                self.hover_hint = Some(card_hint(btn).to_string());
            }
            let bg = if hovb { Color::Rgb(58, 63, 72) } else { Color::Rgb(40, 44, 52) };
            push_pill(&mut spans, label, bg, fg);
            spans.push(Span::raw(" "));
            bx += wl + 1;
        }
        line_at(f, y, Line::from(spans));
        y += 1;


        // ── PDF status (⏳ waiting…, ✓/✗ results) ────────────────────
        if self.poll_cancel.is_some() {
            let label = "◌ waiting for download…  cancel ✕";
            if y < bottom {
                self.card_buttons.push((
                    Rect { x: x0, y, width: label.chars().count() as u16, height: 1 },
                    CardBtn::Cancel,
                ));
            }
            line_at(f, y, Line::from(Span::styled(label, Style::default().fg(Color::Yellow))));
        } else if !self.pdf_status.is_empty() {
            line_at(
                f,
                y,
                Line::from(Span::styled(
                    self.pdf_status.clone(),
                    Style::default().fg(Color::Yellow),
                )),
            );
        }
        // the status row collapses when empty, keeping the keywords
        // close to the buttons
        if self.poll_cancel.is_some() || !self.pdf_status.is_empty() {
            y += 2;
        } else {
            y += 1;
        }

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
        let cyan = Style::default().fg(Color::Cyan);
        let mut spans = vec![
            Span::styled(short.to_string(), cyan),
            // dim cyan, not DarkGray+DIM: stays legible in themes where
            // doubly-dimmed gray vanishes, and sits closer to the
            // leading portion's color
            Span::styled(suffix, Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM)),
        ];
        let used: u16 = spans.iter().map(|s| s.content.chars().count() as u16).sum();
        yanks.push((Rect { x: x0, y, width: used.max(1), height: 1 }, CopyItem::Key));
        if hov_region == Some(CopyItem::Key) {
            for s in &mut spans {
                s.style = s.style.patch(tint);
            }
        }
        // the manuscript chip joins the citekey line when it fits,
        // else drops to the next line
        if has_ms {
            let in_ms = self.lib.in_manuscript(&key);
            let label = if in_ms { "◆ in manuscript" } else { "◇ add to manuscript" };
            let cw = pill_width(label);
            let inline = used + 2 + cw <= w as u16;
            let (cx, cy) = if inline { (x0 + used + 2, y) } else { (x0, y + 1) };
            let r = Rect { x: cx, y: cy, width: cw, height: 1 };
            self.card_buttons.push((r, CardBtn::MsToggle));
            let hovb = hit(r, hv.0, hv.1);
            if hovb {
                self.hover_hint = Some(card_hint(CardBtn::MsToggle).to_string());
            }
            let bg = if hovb { Color::Rgb(58, 63, 72) } else { Color::Rgb(40, 44, 52) };
            let fg = if in_ms { Color::Magenta } else { Color::Gray };
            if inline {
                spans.push(Span::raw("  "));
                push_pill(&mut spans, label, bg, fg);
                line_at(f, y, Line::from(spans));
            } else {
                line_at(f, y, Line::from(std::mem::take(&mut spans)));
                let mut cs: Vec<Span> = vec![];
                push_pill(&mut cs, label, bg, fg);
                line_at(f, cy, Line::from(cs));
            }
        } else {
            line_at(f, y, Line::from(spans));
        }
        self.card_yanks = yanks;
        self.draw_card_toggle(f, x0, w as u16, bottom, false);
    }


    /// Right-aligned clickable show/hide badges for each app-wide view.
    fn draw_badges(&mut self, f: &mut Frame, area: Rect) {
        self.footer_badges.clear();
        let mut badges: Vec<(&str, bool, Action)> = vec![];
        if self.lib.manuscript.is_some() {
            badges.push(("global", self.lib.global_on, Action::GlobalTier));
        }
        badges.extend([
            ("card", self.show_detail, Action::Card),
            ("log", self.show_log, Action::Log),
            ("keys", self.show_help, Action::Help),
        ]);
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
    /// color-coded by category, mm:ss timestamps since launch. PageUp
    /// pages into history (the title shows how far back); any new
    /// message snaps back to the tail.
    fn draw_log(&self, f: &mut Frame, area: Rect) {
        let n = area.height.saturating_sub(2) as usize;
        let scroll = self.log_scroll.min(self.log.len().saturating_sub(n));
        let start = self.log.len().saturating_sub(n + scroll);
        let end = (start + n).min(self.log.len());
        let mut lines: Vec<Line> = vec![];
        for (cat, secs, msg) in &self.log[start..end] {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:02}:{:02}  ", secs / 60, secs % 60),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(msg.clone(), Style::default().fg(cat.color())),
            ]));
        }
        let title = if scroll > 0 {
            format!(" Log ↑{scroll} ")
        } else {
            " Log ".to_string()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(divider())
            .title(Span::styled(title, Style::default().fg(Color::DarkGray)));
        f.render_widget(Paragraph::new(Text::from(lines)).block(block), area);
    }

    fn draw_status(&mut self, f: &mut Frame, area: Rect) {
        // a rule and a breath of space separate the footer from the
        // content above
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(area.width as usize),
                divider(),
            ))),
            Rect { x: area.x, y: area.y, width: area.width, height: 1 },
        );
        let area = Rect { x: area.x, y: area.y + 2, width: area.width, height: 1 };
        let line = match self.mode {
            Mode::Filter => {
                let avail = area.width.saturating_sub(2) as usize;
                let scroll = self.filter.visual_scroll(avail);
                let shown: String = self.filter.value().chars().skip(scroll).collect();
                f.set_cursor_position((
                    area.x + 1 + (self.filter.visual_cursor().saturating_sub(scroll)) as u16,
                    area.y,
                ));
                Line::from(vec![
                    Span::styled("/", Style::default().fg(Color::Cyan)),
                    Span::raw(shown),
                ])
            }
            Mode::Export { ref input, ref keys } => {
                let label = format!("export {} → ", keys.len());
                let prefix = label.chars().count() as u16;
                let avail = area.width.saturating_sub(prefix + 24) as usize;
                let scroll = input.visual_scroll(avail.max(10));
                let shown: String = input.value().chars().skip(scroll).collect();
                f.set_cursor_position((
                    area.x + prefix + (input.visual_cursor().saturating_sub(scroll)) as u16,
                    area.y,
                ));
                Line::from(vec![
                    Span::styled(label, Style::default().fg(Color::Cyan)),
                    Span::raw(shown),
                    Span::styled(
                        "   ⏎ write · Esc cancel",
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            }
            Mode::Setup { ref input, email, .. } => {
                let label = if email { "email (optional, ⏎ skips): " } else { "ADS API token: " };
                let prefix = label.chars().count() as u16;
                let avail = area.width.saturating_sub(prefix + 34) as usize;
                let scroll = input.visual_scroll(avail.max(10));
                let shown: String = input.value().chars().skip(scroll).collect();
                f.set_cursor_position((
                    area.x + prefix + (input.visual_cursor().saturating_sub(scroll)) as u16,
                    area.y,
                ));
                Line::from(vec![
                    Span::styled(label, Style::default().fg(Color::Cyan)),
                    Span::raw(shown),
                    Span::styled(
                        if email {
                            "   ⏎ save · Esc cancel".to_string()
                        } else {
                            "   ⏎ save · Esc cancel · ui.adsabs.harvard.edu/user/settings/token"
                                .to_string()
                        },
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            }
            Mode::AdsPrompt { ref input, limit } => {
                let prefix = 11u16; // "ADS query: "
                let avail = area.width.saturating_sub(prefix + 30) as usize;
                let scroll = input.visual_scroll(avail.max(10));
                let shown: String = input.value().chars().skip(scroll).collect();
                f.set_cursor_position((
                    area.x + prefix + (input.visual_cursor().saturating_sub(scroll)) as u16,
                    area.y,
                ));
                Line::from(vec![
                Span::styled("ADS query: ", Style::default().fg(Color::Cyan)),
                Span::raw(shown),
                Span::styled(
                    format!("   n={limit} ↑↓"),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    "   ⏎ search · Esc cancel · paste DOI/ADS URL to import",
                    Style::default().fg(Color::DarkGray),
                ),
                ])
            }
            Mode::Copy => Line::from(vec![
                Span::styled("copy: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    "y key · Y full key · b bibcode · a ADS · x arXiv · d DOI · p PDF path · t title · A abstract · Esc cancel",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Mode::Normal | Mode::Pick { .. } | Mode::Confirm { .. } if self.select_mode => {
                // fresh notes (an export confirmation, a download
                // result) show through instead of the static hints
                let now = self.started.elapsed().as_secs();
                let fresh = self
                    .log
                    .last()
                    .filter(|(_, t, m)| *m == self.status && now.saturating_sub(*t) < 5);
                let tail = if let Some((cat, _, m)) = fresh {
                    Span::styled(format!("  ·  {m}"), Style::default().fg(cat.color()))
                } else {
                    Span::styled(
                        "  ·  Space/click ◯ toggle · Esc done · ? keys".to_string(),
                        Style::default().fg(Color::DarkGray),
                    )
                };
                Line::from(vec![
                    Span::styled(
                        format!("◉ {} selected", self.selected.len()),
                        Style::default().fg(Color::Cyan),
                    ),
                    tail,
                ])
            }
            Mode::Normal | Mode::Pick { .. } | Mode::Confirm { .. } => {
                let n = self.filtered.len();
                let total = self.order.len();
                let filt = String::new(); // the filter shows as a strip chip
                // logged messages show for ~5s then clear (a fresh one
                // outranks the hover hint); unlogged transient status —
                // download progress, ambient counts — stays visible
                let now = self.started.elapsed().as_secs();
                let last = self.log.last();
                let status_is_logged = last.is_some_and(|(_, _, m)| *m == self.status);
                let fresh = last
                    .filter(|(_, t, m)| *m == self.status && now.saturating_sub(*t) < 5);
                let (msg, msg_color) = if let Some((cat, _, m)) = fresh {
                    (m.clone(), cat.color())
                } else if let Some(hint) = &self.hover_hint {
                    (hint.clone(), Color::Cyan)
                } else if !status_is_logged {
                    (self.status.clone(), Color::Gray)
                } else {
                    (String::new(), Color::Gray)
                };
                let pending = self.tasks.len();
                let mut spans = vec![];
                if pending > 0 {
                    let label = format!("⧗{pending} ");
                    spans.push(Span::styled(label, Style::default().fg(Color::Yellow)));
                }
                spans.extend([
                    Span::styled(
                        format!("{n}/{total}  ·  "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(msg, Style::default().fg(msg_color)),
                    Span::styled(filt, Style::default().fg(Color::DarkGray)),
                ]);
                Line::from(spans)
            }
        };
        f.render_widget(line, area);
        self.draw_badges(f, area);
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

/// System clipboard: pbcopy on macOS (reliable in any terminal), else
/// the OSC 52 escape (terminal-dependent, but works over SSH).
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
