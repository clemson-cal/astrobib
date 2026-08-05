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

mod card;
mod table;
mod theme;

use theme::*;

use table::Col;

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
    /// y pressed — the next key picks what to copy (the Copy panel tab
    /// shows the menu, which-key style); Esc cancels.
    Copy,
}

/// What removing one paper will actually do. Delete means three
/// different things depending on context, each defensible on its own
/// but unpredictable from the keystroke — so the decision is made once,
/// stated by the confirm modal, and executed from the same value.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RemovalKind {
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
    fn sentence(self, n: usize, ms: bool) -> String {
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
    Columns,
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
    /// Not a datum of a paper but of the *scope*: the whole query
    /// configuration, as an ADS search URL.
    QueryConfig,
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
        state: QueryState,
    },
}

/// Where a query scope is in its round trip to ADS.
///
/// A tab exists from the moment its query is sent, not from the moment
/// the results land: a query that takes a minute used to leave nothing
/// on screen at all, which reads as though nothing was asked. The tab
/// is the acknowledgement, and this is what it has to say meanwhile.
#[derive(Clone, PartialEq)]
enum QueryState {
    Pending,
    Ready,
    /// ADS refused or the network did — the text is shown on the page,
    /// where it stays put, rather than only in a log line that scrolls
    /// away while you are reading it.
    Failed(String),
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
    fn kind(&self) -> ScopeKind {
        match self {
            Scope::Library => ScopeKind::Library,
            Scope::Manuscript { .. } => ScopeKind::Manuscript,
            Scope::Ads { .. } => ScopeKind::Query,
        }
    }

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

/// Which side of the screen the arrow keys drive.
///
/// The columns panel is a toggle view, not a modal — the table stays
/// visible and live beside it. But it needs ↑↓ to walk its list and ←→
/// to size a column, which are the table's own navigation keys, so
/// something has to say who gets them. Opening the panel moves focus
/// into it; Esc hands focus back without closing it, and clicking either
/// side moves focus there.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Table,
    Columns,
}

/// One line of the columns panel, and what clicking a part of it does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelHit {
    /// the row itself — select it
    Row(usize),
    /// the ✓ box: show or hide this column
    Toggle(Col),
    /// the label: sort by this column, shown or not
    Sort(Col),
    /// the ‹ › nudges beside the width
    Narrower(Col),
    Wider(Col),
}

/// A line in the columns panel. Columns only — what ADS *returns* is a
/// property of the query, set where the query is composed and cycled
/// with `s`, not a column of the table it fills.
enum PanelRow {
    Column(Col),
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
const ADS_SORTS: [(&str, bool, &str, &str); 10] = [
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
fn ads_sort_value(field: &str, primary: bool) -> String {
    let desc = ADS_SORTS
        .iter()
        .find(|(f, ..)| *f == field)
        .map(|(_, d, ..)| *d)
        .unwrap_or(true);
    format!("{field} {}", if desc == primary { "desc" } else { "asc" })
}

/// The name for a sort parameter, falling back to the default — a state
/// file or a pasted URL could name one we do not know.
fn ads_sort_name(sort: &str) -> &'static str {
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

/// Which kind of table a scope presents. Columns are configured per
/// kind, not per scope: every query tab holds the same kind of record,
/// so they all show the same columns, while each keeps its own sort.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ScopeKind {
    Library,
    Manuscript,
    Query,
}

impl ScopeKind {
    fn tag(self) -> &'static str {
        match self {
            ScopeKind::Library => "library",
            ScopeKind::Manuscript => "manuscript",
            ScopeKind::Query => "query",
        }
    }
}

/// A column's width as actually laid out, 0 if it is not drawn.
///
/// This has to be the *solved* width, not the declared one: the flex
/// column's size is only known once the fixed ones have taken their
/// share, and the flex role moves when a column is hidden. Reading the
/// declaration instead gave the author cells a width of 0 — every one of
/// them rendered as a bare "…" — the moment Author became the flex
/// column.
fn col_width(cols: &[table::ColumnSpec], total: u16, id: Col) -> u16 {
    let solved = table::solve(cols, total);
    cols.iter()
        .position(|c| c.id == id)
        .and_then(|i| solved.get(i).copied())
        .unwrap_or(0)
}

/// Read a stored sort ("column:asc" / "column:desc") from state.json.
/// An absent or unparseable field means "no stored sort", which each
/// caller resolves to its own default.
fn load_sort(field: &str) -> Option<(Col, bool)> {
    let raw = crate::ads::get_state_field(field)?;
    let (col, dir) = raw.split_once(':')?;
    Some((Col::from_tag(col), dir == "asc"))
}

/// Persist one. None writes an empty value, which reads back as absent.
fn store_sort(field: &str, v: Option<(Col, bool)>) -> std::io::Result<()> {
    let raw = match v {
        Some((c, asc)) => format!("{}:{}", c.tag(), if asc { "asc" } else { "desc" }),
        None => String::new(),
    };
    crate::ads::save_state_field(field, &raw)
}

/// Cite states in the order the manuscript view groups them when sorted
/// by state: what needs attention first.
fn ms_state_rank(r: &MsRow) -> u8 {
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

/// The table sidebar's width: enough for a label, a width readout with
/// its ‹ › nudges, and the sort marker, inside the same 2-cell padding
/// the pub card keeps on either side of its rule.
const COLUMNS_PANEL_W: u16 = 28;

/// One line of the columns sidebar, built before it is placed so the
/// list can be windowed against the pane's height.
struct PanelLine {
    spans: Vec<Span<'static>>,
    /// click targets, as (x offset from the panel's left edge, width, what)
    hits: Vec<(u16, u16, PanelHit)>,
    /// the list row this line is, when it is selectable
    row: Option<usize>,
    /// the arrow keys are on this line
    fill: bool,
}

impl PanelLine {
    /// The panel's own name, sitting where the pub card's title sits.
    /// It carries the focus colour, since the panel has no box border
    /// left to tint.
    fn title(text: &str, focused: bool) -> Self {
        PanelLine {
            spans: vec![Span::styled(
                text.to_string(),
                if focused {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::Gray)
                },
            )],
            hits: vec![],
            row: None,
            fill: false,
        }
    }

    fn blank() -> Self {
        PanelLine { spans: vec![], hits: vec![], row: None, fill: false }
    }

    /// Filled like the table's cursor row when the arrow keys are on it.
    /// Without that the selection is invisible on a hidden column, whose
    /// label stays dim either way.
    fn to_line(&self) -> Line<'static> {
        let line = Line::from(self.spans.clone());
        if self.fill {
            line.style(Style::default().bg(cursor_fill()))
        } else {
            line
        }
    }
}

/// The metric swatch: one cell beside the table rather than inside it,
/// but a column like any other — now literally, drawn inside the table
/// rather than as a strip beside it, which is what gives it the same
/// column order, header hover and click handling as the rest. Off until
/// asked for, and never resizable. M chooses which metric it shows.
fn metric_column(metric: MetricCol) -> table::ColumnSpec {
    table::fixed(Col::Metric, "⣿", 2, true)
        .default_off()
        .fixed_size()
        // the legend carries its colormap's hue, the only thing on
        // screen naming which metric is showing
        .styled_header(
            Style::default()
                .fg(metric_color(metric, 0.65))
                .add_modifier(Modifier::BOLD),
        )
}

/// One row's metric swatch. Priority IS a 0..1 level, so it is coloured
/// absolutely and an edit recolours in place; citations rank-normalize
/// over the scope, so the whole ramp gets used.
fn metric_cell(
    metric: MetricCol,
    v: Option<f64>,
    known: &[f64],
) -> ratatui::widgets::Cell<'static> {
    match v {
        Some(v) => {
            let t = match metric {
                MetricCol::Priority => v,
                _ => rank_norm(known, v),
            };
            ratatui::widgets::Cell::from(Span::styled(
                " ",
                Style::default().bg(metric_color(metric, t)),
            ))
        }
        None => ratatui::widgets::Cell::from(Span::styled("·", divider())),
    }
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

/// The one-cell metric swatch column: one scalar per paper, colored
/// by a per-metric colormap so the hue family names the metric.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MetricCol {
    Priority,  // viridis — user-curated 0..1 level, decaying over time
    Citations, // magma — ADS citation count
}

impl MetricCol {
    /// M picks *which* metric the swatch shows. Whether it shows at all
    /// is the columns panel's business, like every other column — so
    /// there is no "off" here to cycle through.
    fn next(self) -> Self {
        match self {
            MetricCol::Priority => MetricCol::Citations,
            MetricCol::Citations => MetricCol::Priority,
        }
    }
    fn name(self) -> &'static str {
        match self {
            MetricCol::Priority => "priority (viridis)",
            MetricCol::Citations => "citations (magma)",
        }
    }
    fn state_tag(self) -> &'static str {
        match self {
            MetricCol::Priority => "priority",
            MetricCol::Citations => "citations",
        }
    }
    fn from_tag(s: &str) -> Self {
        match s {
            "citations" => MetricCol::Citations,
            _ => MetricCol::Priority,
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
const ADS_SAMPLES: [(&str, &str); 4] = [
    ("abs:\"little red dot\" -doctype:abstract", "phrase, minus meeting abstracts"),
    ("author:\"^Andersson, K.\" year:2020-", "first author, from a year on"),
    ("bibstem:ApJL abs:\"magnetar\"", "one journal"),
    ("arxiv_class:astro-ph.HE", "an arXiv subject class"),
];

const FILTER_SAMPLES: [(&str, &str); 4] = [
    ("^andersson year:2019-", "first author, open-ended years"),
    ("abs:\"fast radio burst\"", "phrase in the abstract"),
    ("is:pdf pri:>0.5", "has a PDF, high priority"),
    ("kw:\"compact objects\" -abs:neutrino", "keyword, and a negation"),
];
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
    ("|", "table columns…", Some(Action::Columns), KeyCode::Char('|')),
    ("N", "name this query…", None, KeyCode::Char('N')),
    ("E", "edit this query…", None, KeyCode::Char('E')),
    ("y q", "copy this query", None, KeyCode::Char('y')),
    ("P", "open query on clipboard", None, KeyCode::Char('P')),
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

/// The copy chord: key, menu label, and what it copies. One table, so
/// the keys the chord accepts and the options the footer offers are the
/// same list by construction — they were a `match` and a hand-written
/// string, and drifted.
/// `(key, label, short label, item)`. The short labels exist because the
/// menu shares its line with the view badges and, with everything
/// available, the full one no longer fits at 140 columns.
const COPY_CHORD: [(char, &str, &str, CopyItem); 10] = [
    ('y', "key", "key", CopyItem::Key),
    ('Y', "full key", "full", CopyItem::FullKey),
    ('b', "bibcode", "bib", CopyItem::Bibcode),
    ('a', "ADS", "ADS", CopyItem::AdsUrl),
    ('x', "arXiv", "arXiv", CopyItem::ArxivUrl),
    ('d', "DOI", "DOI", CopyItem::DoiUrl),
    ('p', "PDF path", "PDF", CopyItem::PdfPath),
    ('t', "title", "title", CopyItem::Title),
    ('A', "abstract", "abs", CopyItem::Abstract),
    ('q', "this query", "query", CopyItem::QueryConfig),
];

/// One row of the card's link stack: → rows open the browser, ⌕ rows
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
        CopyItem::QueryConfig => "⧉ copy this query's configuration  ·  y q",
    }
}

/// Footer hint for a card affordance: what happens, and the key.
fn card_hint(btn: CardBtn) -> &'static str {
    match btn {
        CardBtn::Arxiv => "↓ fetch the PDF from arXiv  ·  p",
        CardBtn::Oa => "↓ fetch the open-access PDF via ADS  ·  p",
        CardBtn::Browser => "↓ download via the browser, watching ~/Downloads  ·  B",
        CardBtn::Pick => "⤷ import a PDF from the filesystem",
        CardBtn::Open => "→ open the cached PDF  ·  o",
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
/// Badges name each row's kind: → opens the browser, ⌕ acts inside
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
                LinkTarget::Url(_) => "→",
                LinkTarget::Query(_) => "⌕",
                LinkTarget::Copy(_) => "⧉",
            };
            let shown: String = label.chars().take(colw.saturating_sub(2) as usize).collect();
            let wl = (badge.chars().count() + 1 + shown.chars().count()) as u16;
            let r = Rect { x, y: ry, width: wl, height: 1 };
            let hov = hit(r, hover.0, hover.1);
            if hov {
                let base = match &target {
                    LinkTarget::Url(_) => format!("→ open {label} in the browser"),
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
    /// the columns sidebar, a toggle view like the log and the keys sheet
    show_columns: bool,
    /// which side the arrow keys drive while the columns panel is open
    focus: Focus,
    col_sel: usize,
    col_rects: Vec<(Rect, PanelHit)>,
    /// the ADS-returns control in the query prompt — a click target only
    /// while the prompt is up
    prompt_sort_rect: Rect,
    /// "edit query (E)" in the footer — a click target only on a query
    edit_query_rect: Rect,
    /// the sample-query rows, live only while a prompt is up
    sample_rects: Vec<(Rect, &'static str)>,
    /// the kind of the last coalescing note and where it landed in the
    /// log, so a repeat of the same control can replace it
    last_note: Option<(&'static str, usize)>,
    /// per scope kind: which columns are hidden and how wide the user
    /// pinned them. Absent means auto — see table::ColumnConfig.
    columns: std::collections::HashMap<ScopeKind, table::ColumnConfig>,
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
    sort_headers: Vec<(Rect, Col)>,
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
    sort_menu_rects: Vec<(Rect, String)>,
    // the highlighted field, and whether the whole list is showing its
    // primary direction (newest / most / A→Z) or its reverse. Direction
    // is one axis for the list rather than a property of each row: it is
    // the same question whichever field you are on.
    sort_menu_sel: usize,
    sort_menu_primary: bool,
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

/// Sort fallback for a key the library no longer holds. `self.order`
/// and the library are kept in step by rebuild_order — every mutation
/// path calls it — but nothing in the type system enforces that, and a
/// future path that forgets must not take the running TUI down with it.
/// Orphans sort to the end in either direction (the sort direction is
/// applied to real entries only), keyed by cite key so the order stays
/// total, stable, and identical to before whenever the two agree.
fn orphan_order(a_live: bool, b_live: bool, a: &str, b: &str) -> std::cmp::Ordering {
    match (a_live, b_live) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.cmp(b),
    }
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
            table_area: Rect::default(),
            dl_rx: None,
            show_help: false,
            show_columns: false,
            focus: Focus::Table,
            col_sel: 0,
            col_rects: vec![],
            prompt_sort_rect: Rect::default(),
            edit_query_rect: Rect::default(),
            sample_rects: vec![],
            last_note: None,
            columns: App::load_column_config(),
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
            library_sort: load_sort("library_sort").unwrap_or((Col::Year, false)),
            ms_sort: load_sort("manuscript_sort"),
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
            write_failed: HashSet::new(),
            kill_ring: String::new(),
            sort_menu: false,
            sort_menu_rects: vec![],
            sort_menu_sel: 0,
            sort_menu_primary: true,
        }
    }

    /// Report a failed user-state write — once. Saved queries, curated
    /// priorities, state.json fields, and refs.bib are written on
    /// changes and on idle ticks, so the choice is not "log it or not"
    /// but "log it once or every tick": the first failure per store is
    /// logged, the rest are latched until that store writes again.
    fn state_write(&mut self, what: &'static str, err: Option<String>) {
        match err {
            None => {
                self.write_failed.remove(what);
            }
            Some(e) => {
                if self.write_failed.insert(what) {
                    self.note(MsgCat::Err, format!("could not save {what}: {e}"));
                }
            }
        }
    }

    /// Flush the metrics store — priorities are hand-curated user data,
    /// so a write that quietly stops working gets said out loud.
    fn save_metrics(&mut self) {
        let err = (!self.metrics.save())
            .then(|| self.metrics.error().unwrap_or("write failed").to_string());
        self.state_write("metrics.json", err);
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
                .and_then(|a| self.article_entry(a))
                .map(|e| e.key().to_string()),
            Some(Scope::Manuscript { rows }) => rows.get(pos).and_then(|r| r.key.clone()),
            _ => self.filtered.get(pos).and_then(|&i| self.order.get(i).cloned()),
        }
    }

    /// The library entry at an `order` position — the one place that
    /// resolves a display index, so a position that outlives the entry
    /// it named yields None instead of indexing or unwrapping.
    fn entry_at(&self, i: usize) -> Option<&crate::library::Entry> {
        self.lib.get(self.order.get(i)?)
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
        // each scope keeps its own sort, so entering one re-asserts it;
        // the library and manuscript are already in their sorted order
        self.sort_ads_at(self.active_scope);
    }

    fn cycle_scope(&mut self, d: isize) {
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
    fn run_ads_query_limit(
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
            self.scopes.push(Scope::Ads {
                tab: tab.clone(),
                articles: vec![],
                state: QueryState::Pending,
            });
            self.set_scope(self.scopes.len() - 1);
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

    /// The scope holding a given tab, by the tab's own id.
    ///
    /// Results are routed by identity rather than by the index the query
    /// was launched from: the tab is on screen for the whole round trip
    /// now, so it can be closed, or another opened beside it, while the
    /// query is still in flight — and an index captured a minute ago
    /// would by then address somebody else's scope.
    fn scope_of_tab(&self, id: &str) -> Option<usize> {
        self.scopes
            .iter()
            .position(|s| matches!(s, Scope::Ads { tab, .. } if tab.id == id))
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
    fn sync_refs_bib(&mut self, rows: &[MsRow]) {
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

    fn ms_root(&self) -> Option<std::path::PathBuf> {
        self.lib.manuscript.as_ref().map(|m| m.root.clone())
    }

    /// Persist the current ADS scopes to the tabs.json state file,
    /// user-local and keyed per manuscript context.
    fn save_tabs(&mut self) {
        let tabs: Vec<crate::tabs::Tab> = self
            .scopes
            .iter()
            .filter_map(|s| match s {
                Scope::Ads { tab, .. } => Some(tab.clone()),
                _ => None,
            })
            .collect();
        let res = crate::tabs::save(&tabs, self.ms_root().as_deref());
        self.state_write("saved queries (tabs.json)", res.err().map(|e| e.to_string()));
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
            self.scopes.push(Scope::Ads { tab: t.clone(), articles, state: QueryState::Ready });
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
        self.run_ads_query_limit(q, Some(self.active_scope), l, None);
    }

    fn refresh_scope(&mut self) {
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
                            self.scopes[idx] =
                                Scope::Ads { tab, articles, state: QueryState::Ready };
                            // ADS hands results back in its own order;
                            // the tab's own sort decides what is shown
                            self.sort_ads_at(idx);
                            self.save_tabs();
                            self.note(MsgCat::Ok, format!("{n} ADS result(s)"));
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

    /// a/A — bulk selection: `a` toggles all *visible* rows (the
    /// filtered set — so with a filter active it is select-all-in-
    /// filter; with the global tier hidden it is tier-2-only), `A`
    /// selects every item in the scope regardless of filter. Selecting
    /// when everything is already selected deselects (a), mirroring the
    /// single-row toggle; an emptied selection exits the mode.
    fn select_all(&mut self, visible_only: bool) {
        let ids: Vec<String> = match self.scopes.get(self.active_scope) {
            // every row that resolves to a paper; missing and ambiguous
            // cites are skipped rather than blocking the whole gesture
            Some(Scope::Manuscript { rows }) => {
                rows.iter().filter_map(|r| r.key.clone()).collect()
            }
            Some(Scope::Ads { articles, .. }) => {
                articles.iter().map(|a| a.bibcode.clone()).collect()
            }
            _ if visible_only => self
                .filtered
                .iter()
                .filter_map(|&i| self.order.get(i).cloned())
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

    /// A note that supersedes its own previous one.
    ///
    /// Holding ← on a column width, or cycling a sort, would otherwise
    /// leave a line per keystroke saying nothing the line before it did
    /// not. The log should record that you resized the column, not how
    /// many times you pressed the key — so a repeat of the same `kind`
    /// within a few seconds replaces its own last entry instead of
    /// appending. Anything the user did not directly cause (a download
    /// finishing, a query landing) always appends, and so uses `note`.
    fn note_latest(&mut self, cat: MsgCat, kind: &'static str, msg: String) {
        const WINDOW_SECS: u64 = 6;
        if let Some((k, i)) = self.last_note {
            // only if it is still the newest entry: anything logged since
            // would be silently eaten otherwise
            let fresh = self
                .log
                .get(i)
                .is_some_and(|(_, t, _)| self.started.elapsed().as_secs().saturating_sub(*t) < WINDOW_SECS);
            if k == kind && i + 1 == self.log.len() && fresh {
                self.log.pop();
            }
        }
        self.note(cat, msg);
        self.last_note = Some((kind, self.log.len() - 1));
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
            | Action::Columns | Action::Quit => true,
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
                // with the global tier hidden every resolvable paper is
                // already a manuscript member, so ± would only ever mean
                // "remove" — press t first and say which you meant
                "manuscript ± needs the global tier shown — press t".to_string()
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
            | Action::Columns | Action::Quit => "not available here".to_string(),
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
            Action::Quit => self.quit = true,
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
    fn open_edit_query_prompt(&mut self) {
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
    fn use_sample(&mut self, sample: &'static str) {
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
    fn rename_query(&mut self, name: String) {
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

    fn open_rename_prompt(&mut self) {
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
            .filter_map(|&i| self.entry_at(i))
            .filter_map(|e| e.bibcode().map(|b| (b.to_string(), e.key().to_string())))
            .collect();
        if bibs.is_empty() || crate::ads::get_token().is_none() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.cit_rx = Some(rx);
        self.add_task(TaskKind::Query, "⌕ citation counts".to_string(), vec![]);
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
                self.save_metrics();
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
    /// The selected manuscript rows' keys, in row order.
    fn selected_ms_keys(&self) -> Vec<String> {
        let Some(Scope::Manuscript { rows }) = self.scopes.get(self.active_scope) else {
            return vec![];
        };
        rows.iter()
            .filter_map(|r| r.key.as_ref())
            .filter(|k| self.selected.contains(*k))
            .cloned()
            .collect()
    }

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
                .filter_map(|a| self.article_entry(a))
                .map(|e| e.key().to_string())
                .collect();
        }
        if self.select_mode && !self.selected.is_empty() {
            // in the manuscript scope the visible rows are the truth:
            // self.order is the library's ordering, which omits rows a
            // cite resolves to only in the personal tier
            if matches!(self.scopes.get(self.active_scope), Some(Scope::Manuscript { .. })) {
                return self.selected_ms_keys();
            }
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
                self.table_area,
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
                    return self
                        .filtered
                        .get(pos)
                        .and_then(|&i| self.order.get(i))
                        .map(String::as_str);
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
    /// The library entry a query result refers to, matched by paper
    /// identity rather than by bibcode.
    ///
    /// A paper imported as a preprint carries the arXiv bibcode, while
    /// a later search returns the published one — comparing bibcodes
    /// calls those two different papers, so an imported preprint shows
    /// as un-imported the moment it is published. Cite keys derive from
    /// the stable identifier (arXiv ID before bibcode), so the key is
    /// the same on both sides of that transition.
    fn article_entry(&self, a: &crate::ads::Article) -> Option<&crate::library::Entry> {
        self.lib.get(&self.hypothetical_key(a))
    }

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

    /// The citation count the card is showing, when it is known. None
    /// means unknown, which must never be confused with a known zero.
    fn card_citation_count(&self) -> Option<i64> {
        if let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) {
            if let Some(a) = self.card_article_pos().and_then(|p| articles.get(p)) {
                return a.citation_count;
            }
        }
        let key = self.card_entry_key()?;
        self.metrics.get(&key).and_then(|m| m.citations)
    }

    fn card_entry_key(&self) -> Option<String> {
        if let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) {
            let a = self.card_article_pos().and_then(|p| articles.get(p))?;
            return self.article_entry(a).map(|e| e.key().to_string());
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
        self.order.get(idx).map(String::as_str)
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
        // filter_map, not filter: a key the library no longer holds
        // simply drops out of the visible set — which also keeps every
        // position in `filtered` resolvable for the table and the
        // metric strip, whatever state `order` is in
        self.filtered = self
            .order
            .iter()
            .enumerate()
            .filter_map(|(i, key)| {
                let e = self.lib.get(key)?;
                query::matches(&groups, e, &ctx).then_some(i)
            })
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
            // a manuscript row selects the paper it resolves to; a
            // missing or ambiguous cite names no paper, so it cannot
            // join a selection keyed by cite key
            Some(Scope::Manuscript { rows }) => {
                let Some(key) = rows.get(pos).and_then(|r| r.key.clone()) else {
                    self.note(
                        MsgCat::Warn,
                        "this cite resolves to no paper — nothing to select".to_string(),
                    );
                    return;
                };
                key
            }
            _ => {
                let Some(&idx) = self.filtered.get(pos) else {
                    return;
                };
                let Some(key) = self.order.get(idx) else { return };
                key.clone()
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
            // a rescan hands back scan order; re-assert the manuscript's
            // own sort over it
            self.sort_ms_rows();
        }
        self.order = self.lib.entries().iter().map(|e| e.key().to_string()).collect();
        let lib = &self.lib;
        // the library's own column, never the front scope's: importing a
        // paper while a query is showing must not reorder the library by
        // the query's sort
        let (col, asc) = self.library_sort;
        self.order.sort_by(|a, b| {
            let (ea, eb) = match (lib.get(a), lib.get(b)) {
                (Some(ea), Some(eb)) => (ea, eb),
                (x, y) => return orphan_order(x.is_some(), y.is_some(), a, b),
            };
            let ord = match col {
                Col::Metric => match self.metric_col {
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
                },
                Col::Pdf => has_cached_pdf(ea.key()).cmp(&has_cached_pdf(eb.key())),
                Col::InLib => lib
                    .in_manuscript(ea.key())
                    .cmp(&lib.in_manuscript(eb.key())),
                Col::Year => ea.year().cmp(&eb.year()),
                Col::Author => ea
                    .first_author_last()
                    .to_lowercase()
                    .cmp(&eb.first_author_last().to_lowercase()),
                Col::Title => ea
                    .title()
                    .trim_matches(['{', '}'])
                    .to_lowercase()
                    .cmp(&eb.title().trim_matches(['{', '}']).to_lowercase()),
                Col::Key => ea.key().cmp(eb.key()),
                // the gutter, the manuscript-only columns, and Entered
                // (which only ADS records carry) never sort the library;
                // listed rather than caught by a wildcard so that a new
                // sortable column has to be handled here
                Col::Sel | Col::CiteIcon | Col::Cited | Col::State | Col::Entered => {
                    std::cmp::Ordering::Equal
                }
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

    /// The active scope's sort, or None where its rows have an inherent
    /// order (a manuscript in scan order).
    fn sort(&self) -> Option<(Col, bool)> {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { tab, .. }) => Some((Col::from_tag(&tab.sort_col), tab.sort_asc)),
            Some(Scope::Manuscript { .. }) => self.ms_sort,
            _ => Some(self.library_sort),
        }
    }

    /// Write the active scope's sort back where that scope keeps it and
    /// persist it: a query tab into tabs.json, the library and the
    /// manuscript into state.json.
    fn set_sort(&mut self, v: (Col, bool)) {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { .. }) => {
                if let Some(Scope::Ads { tab, .. }) = self.scopes.get_mut(self.active_scope) {
                    tab.sort_col = v.0.tag().to_string();
                    tab.sort_asc = v.1;
                }
                self.save_tabs();
            }
            Some(Scope::Manuscript { .. }) => {
                self.ms_sort = Some(v);
                let res = store_sort("manuscript_sort", self.ms_sort);
                self.state_write("state.json", res.err().map(|e| e.to_string()));
            }
            _ => {
                self.library_sort = v;
                let res = store_sort("library_sort", Some(v));
                self.state_write("state.json", res.err().map(|e| e.to_string()));
            }
        }
    }

    /// Header click: same column flips direction, a new column starts
    /// descending for Year (newest first) and ascending otherwise.
    fn sort_by(&mut self, col: Col) {
        let next = self.next_sort(col);
        self.set_sort(next);
        self.apply_sort();
        let dir = if next.1 { "ascending" } else { "descending" };
        self.note_latest(
            MsgCat::Info,
            "sort",
            format!("sorted by {} {dir}", self.column_hint(col)),
        );
    }

    /// What clicking a column's sort control would do. Sharing this with
    /// the panel is what lets it preview the result on hover rather than
    /// guess at it.
    fn next_sort(&self, col: Col) -> (Col, bool) {
        match self.sort() {
            Some((c, asc)) if c == col => (col, !asc),
            // bool-ish and recency columns start with the interesting side
            // up: cached/in-library/newest first; text columns start A→Z
            _ => (
                col,
                !matches!(
                    col,
                    Col::Year | Col::Entered | Col::Pdf | Col::InLib | Col::Metric
                ),
            ),
        }
    }

    /// Re-order the active scope's rows to match its sort.
    fn apply_sort(&mut self) {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { .. }) => self.sort_ads_at(self.active_scope),
            Some(Scope::Manuscript { .. }) => self.sort_ms_rows(),
            _ => self.rebuild_order(),
        }
    }

    /// Re-order the manuscript rows by the manuscript's own sort. With
    /// none set they keep scan order — the order the cites appear in the
    /// source, which is meaningful and is the default.
    fn sort_ms_rows(&mut self) {
        let Some((col, asc)) = self.ms_sort else {
            return;
        };
        let Some(Scope::Manuscript { rows }) = self.scopes.get_mut(1) else {
            return;
        };
        rows.sort_by(|a, b| {
            let ord = match col {
                Col::Cited => a.cited.to_lowercase().cmp(&b.cited.to_lowercase()),
                // both glyph and word render the same fact, so both sort
                // by it: attention-first, so missing cites come to the top
                Col::CiteIcon | Col::State => ms_state_rank(a).cmp(&ms_state_rank(b)),
                Col::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                // no other column is drawn in this scope
                _ => std::cmp::Ordering::Equal,
            };
            let ord = if asc { ord } else { ord.reverse() };
            // ties keep a stable, meaningful order rather than an arbitrary one
            ord.then_with(|| a.cited.cmp(&b.cited))
        });
    }

    /// Re-sort one ADS scope's articles in place, by that scope's own
    /// sort (decorate-sort: cache and library lookups happen before the
    /// mutable put-back). A no-op for any other kind of scope.
    fn sort_ads_at(&mut self, idx: usize) {
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
    fn remove_confirmed(&mut self, plan: &[(String, RemovalKind)]) {
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
                    if let Some(t) = self.finish_task(id).filter(|t| t.cancelled) {
                        // discard the cancelled task's result: remove
                        // anything it cached so the end state matches the
                        // existing failure/clear paths, and reset the
                        // channel state exactly as the normal arm does
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
                .and_then(|a| self.article_entry(a))
                .map(|e| e.key().to_string()),
            Some(Scope::Manuscript { rows }) => rows.get(pos).and_then(|r| r.key.clone()),
            _ => self.filtered.get(pos).and_then(|&i| self.order.get(i).cloned()),
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
        // the prompt's ADS-returns glyph, which must be tested before the
        // click-away dismissal below or it would close the prompt instead
        if matches!(self.mode, Mode::AdsPrompt { .. }) && hit(self.prompt_sort_rect, x, y) {
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
            if let Some(i) = self.sort_menu_rects.iter().position(|(r, _)| hit(*r, x, y)) {
                // a click is one gesture, so it chooses and leaves —
                // unlike the arrows, which are a walk through the list
                if let Some((_, value)) = self.sort_menu_rects.get(i).cloned() {
                    self.sort_menu_sel = ADS_SORTS
                        .iter()
                        .position(|(f, ..)| value.starts_with(f))
                        .unwrap_or(self.sort_menu_sel);
                    self.apply_sort_menu();
                }
                self.sort_menu = false;
                return;
            }
        }
        // a sample row, which must be tested before the click-away
        // dismissal below — reaching that would close the very prompt
        // the sample is meant to fill. Consumed either way: a row that
        // cannot act must not fall through and close the prompt instead.
        if let Some(&(_, sample)) = self.sample_rects.iter().find(|(r, _)| hit(*r, x, y)) {
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
        ) {
            self.mode = Mode::Normal;
        }
        // confirm modal: only its two buttons act; other clicks are inert
        if let Mode::Confirm { plan } = &self.mode {
            if let Some(&(_, is_confirm)) = self.confirm_btns.iter().find(|(r, _)| hit(*r, x, y)) {
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
        if hit(self.edit_query_rect, x, y) {
            if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
                self.open_edit_query_prompt();
            } else {
                self.open_ads_prompt();
            }
            return;
        }
        // footer view badges
        if let Some(&(_, action)) = self.footer_badges.iter().find(|(r, _)| hit(*r, x, y)) {
            self.run_action(action);
            return;
        }
        // the columns panel: clicking anywhere in it takes focus and
        // selects the row; a control inside the row also acts. The
        // specific hits are searched first because each shares its row's
        // rect, and Row alone would swallow every one of them.
        if self.show_columns && self.col_rects.iter().any(|(r, _)| hit(*r, x, y)) {
            self.focus = Focus::Columns;
            if let Some(&(_, PanelHit::Row(i))) = self
                .col_rects
                .iter()
                .find(|(r, h)| hit(*r, x, y) && matches!(h, PanelHit::Row(_)))
            {
                self.col_sel = i;
            }
            let action = self
                .col_rects
                .iter()
                .find(|(r, h)| hit(*r, x, y) && !matches!(h, PanelHit::Row(_)))
                .map(|&(_, h)| h);
            match action {
                Some(PanelHit::Toggle(id)) => self.toggle_column(id),
                Some(PanelHit::Sort(id)) => self.sort_by(id),
                Some(PanelHit::Narrower(_)) => self.nudge_width(-1),
                Some(PanelHit::Wider(_)) => self.nudge_width(1),
                _ => {}
            }
            return;
        }
        // a click anywhere else is the table's, and takes focus with it
        self.focus = Focus::Table;
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
                // a scope's property, not a paper's: copy_text
                // answers it before any of these are reached
                CopyItem::QueryConfig => None,

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
                    .article_entry(a)
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
            // a scope's property, not a paper's: copy_text answers
            // it before any of these are reached
            CopyItem::QueryConfig => None,
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
                .article_entry(a)
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

    /// `P` — take a query configuration off the clipboard and open it.
    ///
    /// Deliberately loud when the clipboard holds something else: the
    /// whole value of this is that it is one keystroke, and one
    /// keystroke that silently did nothing would be worse than none.
    fn paste_query_config(&mut self) {
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
                // a scope's property, not a paper's
                CopyItem::QueryConfig => None,
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

    /// What `item` would put on the clipboard right now, or why it
    /// would not.
    ///
    /// The menu and the action both go through this, so an option can
    /// only be offered when pressing it would actually copy something —
    /// the two used to be a static string and a separate resolution, and
    /// the menu offered "bibcode" for papers that have none and "this
    /// query" on the library, where there is no query.
    fn copy_text(&self, item: CopyItem) -> Result<String, String> {
        if item == CopyItem::QueryConfig {
            let Some(Scope::Ads { tab, .. }) = self.scopes.get(self.active_scope) else {
                return Err("no query here to copy — this is the library".to_string());
            };
            return Ok(crate::ads::search_url(&tab.query, tab.limit, &tab.ads_sort));
        }
        let multi_prose = matches!(item, CopyItem::Title | CopyItem::Abstract);
        let nothing = || "nothing to copy".to_string();
        if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
            let sel = self.selected_articles();
            if sel.len() > 1 {
                if multi_prose {
                    return Err(format!("no multi-item form for that ({} selected)", sel.len()));
                }
                return self.articles_copy_value(&sel, item).ok_or_else(nothing);
            }
            if sel.len() == 1 {
                return self.article_value(sel[0], item).ok_or_else(nothing);
            }
            return self.article_copy_value(item).ok_or_else(nothing);
        }
        if multi_prose && self.select_mode && self.selected.len() > 1 {
            return Err(format!(
                "no multi-item form for that ({} selected)",
                self.selected.len()
            ));
        }
        self.copy_value(item).ok_or_else(nothing)
    }

    /// Whether the copy menu should offer `item` on the current screen.
    fn copy_offered(&self, item: CopyItem) -> bool {
        self.copy_text(item).is_ok()
    }

    /// The which-key line for the copy chord, listing only what this
    /// screen can actually copy — no "this query" on the library, no
    /// "bibcode" for a paper that has none.
    fn copy_menu(&self, width: u16) -> String {
        let offered: Vec<&(char, &str, &str, CopyItem)> =
            COPY_CHORD.iter().filter(|(.., item)| self.copy_offered(*item)).collect();
        if offered.is_empty() {
            return "nothing here to copy · Esc cancel".to_string();
        }
        // shortening beats truncating: a cut-off menu hides options that
        // are available, which is the failure this whole change is about
        let render = |short: bool, tail: bool, sep: &str| {
            let body = offered
                .iter()
                .map(|(k, long, s, _)| format!("{k} {}", if short { s } else { long }))
                .collect::<Vec<_>>()
                .join(sep);
            if tail {
                format!("{body}{sep}Esc cancel")
            } else {
                body
            }
        };
        // last resort, for a terminal too narrow for any of it: the keys
        // alone. They still say what the chord accepts, and the card's
        // copy column carries the meanings — colliding with the badges
        // would say nothing at all.
        let keys = || {
            offered.iter().map(|(k, ..)| k.to_string()).collect::<Vec<_>>().join(" ")
        };
        let fits = |s: &String| s.chars().count() <= width as usize;
        // words before separators before labels: which key does what is
        // the information here, and the dots are only comfort
        [
            render(false, true, " · "),
            render(true, true, " · "),
            render(true, false, " · "),
            render(true, false, "  "),
            keys(),
        ]
            .into_iter()
            .find(fits)
            .unwrap_or_else(|| {
                let mut t: String = keys().chars().take(width.saturating_sub(1) as usize).collect();
                t.push('…');
                t
            })
    }

    fn do_copy(&mut self, item: CopyItem) {
        self.exit_copy_mode();
        match self.copy_text(item) {
            Ok(text) => self.finish_copy(&text),
            Err(why) => self.note(MsgCat::Warn, why),
        }
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

    /// The text the active mode is composing, if it is composing any.
    fn active_input_mut(&mut self) -> Option<&mut tui_input::Input> {
        match &mut self.mode {
            Mode::Filter => Some(&mut self.filter),
            Mode::AdsPrompt { input, .. }
            | Mode::Setup { input, .. }
            | Mode::Export { input, .. }
            | Mode::Rename { input } => Some(input),
            _ => None,
        }
    }

    /// Replace the active input's text, keeping the cursor where the
    /// caller puts it (char index, as tui-input counts).
    fn set_input(&mut self, value: String, cursor: usize) {
        if let Some(input) = self.active_input_mut() {
            *input = tui_input::Input::new(value).with_cursor(cursor);
        }
    }

    /// The chords shared by every prompt: emacs kill/yank, and copying
    /// what is being composed. Returns whether the key was claimed.
    ///
    /// `⌃k` used to reach tui-input, which implements it as a plain
    /// delete — so a killed tail was simply gone, and `⌃y` had nothing
    /// to yank because there was no kill ring to yank from.
    fn prompt_chord(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        let ctrl = mods.contains(KeyModifiers::CONTROL);
        let alt = mods.contains(KeyModifiers::ALT);
        let Some(input) = self.active_input_mut() else {
            return false;
        };
        let (value, cursor) = (input.value().to_string(), input.cursor());
        // char indices, since the cursor is one and the text is not ASCII
        let split = |n: usize| {
            let b = value.char_indices().nth(n).map(|(i, _)| i).unwrap_or(value.len());
            (value[..b].to_string(), value[b..].to_string())
        };
        match code {
            KeyCode::Char('k') if ctrl => {
                let (head, tail) = split(cursor);
                if tail.is_empty() {
                    return true; // nothing to kill; do not clobber the ring
                }
                self.kill_ring = tail;
                self.set_input(head, cursor);
                true
            }
            // ⌃u kills to the start of the line and ⌃w the word before
            // the cursor. tui-input performs both, but as deletions —
            // routing them through the ring is what makes ⌃y able to
            // undo any of the three rather than only ⌃k.
            KeyCode::Char('u') if ctrl => {
                let (head, tail) = split(cursor);
                if head.is_empty() {
                    return true;
                }
                self.kill_ring = head;
                self.set_input(tail, 0);
                true
            }
            KeyCode::Char('w') if ctrl => {
                let (head, tail) = split(cursor);
                // back over any run of spaces, then over the word itself
                let kept = head.trim_end();
                let start = kept
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace())
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                if start == head.len() {
                    return true;
                }
                self.kill_ring = head[start..].to_string();
                let kept = head[..start].to_string();
                let n = kept.chars().count();
                self.set_input(format!("{kept}{tail}"), n);
                true
            }
            KeyCode::Char('y') if ctrl => {
                if self.kill_ring.is_empty() {
                    self.note(MsgCat::Warn, "nothing killed to yank".to_string());
                    return true;
                }
                let (head, tail) = split(cursor);
                let n = self.kill_ring.chars().count();
                let text = format!("{head}{}{tail}", self.kill_ring);
                self.set_input(text, cursor + n);
                true
            }
            // ⌥w, emacs' copy. On an ADS query it copies the search URL,
            // which is the only form that carries the result limit and
            // what ADS returns as well — neither is query syntax, and
            // Solr has no comment to smuggle them into. Pasting one back
            // restores all three.
            KeyCode::Char('w') if alt => {
                let text = match &self.mode {
                    Mode::AdsPrompt { input, limit, sort, .. } => {
                        crate::ads::search_url(input.value(), *limit, sort)
                    }
                    _ => value,
                };
                if text.is_empty() {
                    self.note(MsgCat::Warn, "nothing to copy".to_string());
                } else {
                    self.finish_copy(&text);
                }
                true
            }
            _ => false,
        }
    }

    /// A bracketed paste. In a prompt the text is inserted at the
    /// cursor; an ADS search URL instead *configures* the query, which
    /// is what makes ⌥w reversible — copy a query anywhere, paste it
    /// back, and the text, the limit and the returns mode all come with
    /// it. Pasted onto the table with no prompt up, it opens one.
    fn on_paste(&mut self, text: String) {
        let parsed = crate::ads::parse_search_url(&text);
        if let Some((q, rows, so)) = parsed {
            if matches!(self.mode, Mode::Normal) {
                self.open_ads_prompt();
            }
            if let Mode::AdsPrompt { input, limit, sort, .. } = &mut self.mode {
                *input = tui_input::Input::from(q);
                let mut said = String::from("query pasted");
                if let Some(r) = rows {
                    *limit = r.clamp(1, 2000);
                    said.push_str(&format!(" · {} results", *limit));
                }
                // an unknown sort would leave the prompt naming a mode it
                // is not in, since the label is looked up by value
                // ADS silently drops a sort field it does not know, so
                // passing one through would leave the prompt naming an
                // order the query is not actually getting
                if let Some(s) = so {
                    let field = s.split_once(' ').map(|(f, _)| f).unwrap_or(&s);
                    if ADS_SORTS.iter().any(|(f, ..)| *f == field) {
                        *sort = s.clone();
                        said.push_str(&format!(" · {}", ads_sort_name(&s)));
                    } else {
                        said.push_str(" · (unknown sort ignored)");
                    }
                }
                self.note(MsgCat::Ok, said);
                return;
            }
        }
        if self.active_input_mut().is_some() {
            let one_line: String =
                text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
            let input = self.active_input_mut().unwrap();
            let (value, cursor) = (input.value().to_string(), input.cursor());
            let b = value.char_indices().nth(cursor).map(|(i, _)| i).unwrap_or(value.len());
            let n = one_line.chars().count();
            let text = format!("{}{one_line}{}", &value[..b], &value[b..]);
            self.set_input(text, cursor + n);
            if matches!(self.mode, Mode::Filter) {
                self.refilter();
            }
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
        // while the columns panel holds focus it takes the navigation
        // keys; anything it does not claim falls through to the table,
        // so q, ?, D and the rest keep working with the panel open
        if matches!(self.mode, Mode::Normal)
            && self.show_columns
            && self.focus == Focus::Columns
            && self.columns_panel_key(code)
        {
            return;
        }
        // the ADS-returns menu owns the keyboard while it is open, and
        // is claimed before the editing chords so a field key cannot be
        // mistaken for one
        if self.sort_menu && self.sort_menu_key(code, mods) {
            return;
        }
        // the editing chords every prompt shares, claimed before the
        // per-mode arms so each one does not have to repeat them
        if self.prompt_chord(code, mods) {
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
                    KeyCode::Char(c) => {
                        COPY_CHORD.iter().find(|(k, ..)| *k == c).map(|(.., i)| *i)
                    }
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
            Mode::Rename { input } => match code {
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.note(MsgCat::Info, "rename cancelled".to_string());
                }
                KeyCode::Enter => {
                    let name = input.value().trim().to_string();
                    // the prompt closes first: it occupies the footer, so
                    // a confirmation raised while it is up would be drawn
                    // over by the prompt itself and never seen
                    self.mode = Mode::Normal;
                    self.rename_query(name);
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
            Mode::AdsPrompt { input, limit, sort, edit } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let (q, l, so, ed) =
                        (input.value().to_string(), *limit, sort.clone(), *edit);
                    self.mode = Mode::Normal;
                    self.run_ads_query_limit(q, ed, l, Some(so));
                }
                // ⌃r opens the menu of everything ADS will sort by. A
                // chord, so it cannot be confused with typing, and ⌃s /
                // ⌃q are avoided because terminals still eat those as
                // flow control. It cycled four modes until there were
                // twenty, which is more than a cycle can carry.
                KeyCode::Char('r') if mods.contains(KeyModifiers::CONTROL) => {
                    self.open_sort_menu();
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
            Mode::Confirm { plan } => match code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let plan = plan.clone();
                    self.mode = Mode::Normal;
                    self.remove_confirmed(&plan);
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
                KeyCode::Char('|') => self.run_action(Action::Columns),
                KeyCode::Char('N') => self.open_rename_prompt(),
                KeyCode::Char('E') => self.open_edit_query_prompt(),
                // the other half of the focus toggle; with no panel open
                // there is only one thing the arrows could drive
                KeyCode::Tab | KeyCode::BackTab if self.show_columns => {
                    self.focus = Focus::Columns
                }
                KeyCode::Char('t') => self.run_action(Action::GlobalTier),
                KeyCode::Char('v') => self.show_bib_source = !self.show_bib_source,
                KeyCode::Char('e') => self.open_export_prompt(),
                KeyCode::Char('M') => {
                    self.metric_col = self.metric_col.next();
                    if self.sort().is_some_and(|(c, _)| c == Col::Metric) {
                        // the sort is by "the metric", so switching which
                        // metric that is has to reorder the rows
                        self.apply_sort();
                    }
                    let res =
                        crate::ads::save_state_field("metric", self.metric_col.state_tag());
                    self.state_write("state.json", res.err().map(|e| e.to_string()));
                    self.note_latest(
                        MsgCat::Info,
                        "metric",
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
                KeyCode::Char('P') => self.paste_query_config(),
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
        } else {
            self.col_rects.clear();
        }
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
        if self.sort_menu {
            self.sample_rects.clear();
            self.draw_sort_menu(f, samples_area);
        } else {
            self.sort_menu_rects.clear();
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
        } else {
            self.pick_area = Rect::default();
        }
        if let Mode::Confirm { .. } = self.mode {
            self.draw_confirm(f);
        } else {
            self.confirm_btns.clear();
        }
    }

    /// Centered confirm modal for Delete: lists the targets, states in
    /// plain words what confirming will do to them (the decided plan,
    /// which is also what executes), offers clickable remove/cancel
    /// (⏎/y confirms, Esc/n cancels).
    fn draw_confirm(&mut self, f: &mut Frame) {
        self.confirm_btns.clear();
        let Mode::Confirm { plan } = &self.mode else { return };
        let frame = f.area();
        let w = 52.min(frame.width.saturating_sub(4));
        let listed: Vec<&String> = plan.iter().take(6).map(|(k, _)| k).collect();
        let extra = plan.len().saturating_sub(listed.len());
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
        // one sentence per distinct outcome, in the order they occur;
        // counts appear only when the targets do not share an outcome
        let mut kinds: Vec<(RemovalKind, usize)> = vec![];
        for (_, k) in plan.iter() {
            match kinds.iter_mut().find(|(x, _)| x == k) {
                Some((_, n)) => *n += 1,
                None => kinds.push((*k, 1)),
            }
        }
        let mixed = kinds.len() > 1;
        let ms = self.lib.manuscript.is_some();
        for (kind, n) in &kinds {
            let s = kind.sentence(*n, ms);
            let s = if mixed { format!("{n} · {s}") } else { s };
            for l in wrap_text(&s, w.saturating_sub(4) as usize) {
                lines.push(Line::from(Span::styled(l, Style::default().fg(Color::Yellow))));
            }
        }
        lines.push(Line::default());
        let h = lines.len() as u16 + 3; // + the buttons row and both borders
        let area = Rect {
            x: frame.width.saturating_sub(w) / 2,
            y: frame.height.saturating_sub(h) / 2,
            width: w,
            height: h.min(frame.height),
        };
        f.render_widget(ratatui::widgets::Clear, area);
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
            if hov_cancel { chip_bg_hover() } else { chip_bg() },
            chip_fg_strong(),
        );
        lines.push(Line::from(bspans));
        // with no local tier there is only one place to remove from, so
        // the title can name it; otherwise the sentences below do
        let title = if ms {
            format!(" Remove {} paper(s)? ", plan.len())
        } else {
            format!(" Remove {} paper(s) from the library? ", plan.len())
        };
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
    /// The samples for whichever prompt is up, or None when none is.
    fn active_samples(&self) -> Option<&'static [(&'static str, &'static str); 4]> {
        match self.mode {
            Mode::AdsPrompt { .. } => Some(&ADS_SAMPLES),
            Mode::Filter => Some(&FILTER_SAMPLES),
            _ => None,
        }
    }

    /// Whether the prompt being composed is still blank. A sample loads
    /// only into an empty prompt: it replaces the whole query, and doing
    /// that to something half-typed would destroy work on a stray click.
    fn prompt_is_empty(&self) -> bool {
        match &self.mode {
            Mode::AdsPrompt { input, .. } => input.value().trim().is_empty(),
            Mode::Filter => self.filter.value().trim().is_empty(),
            _ => false,
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
    fn samples_height(&self, spare: u16, width: u16) -> u16 {
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
    fn sort_menu_key(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
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
    fn apply_sort_menu(&mut self) {
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
    fn open_sort_menu(&mut self) {
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
    fn sort_menu_height(&self, spare: u16, _width: u16) -> u16 {
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
    fn draw_sort_menu(&mut self, f: &mut Frame, area: Rect) {
        self.sort_menu_rects.clear();
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
            self.sort_menu_rects.push((rect, ads_sort_value(field, primary)));
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
    fn draw_samples(&mut self, f: &mut Frame, area: Rect) {
        self.sample_rects.clear();
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
            self.sample_rects.push((r, query));
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

    fn draw_help(&mut self, f: &mut Frame, area: Rect) {
        self.help_rects.clear();
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
        // emoji-set glyphs (⟳) can render double-width on some
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
                x: area.x + 5, // border + the " →  " prefix
                y,
                width: url.chars().count() as u16 + 1,
                height: 1,
            };
            about_links.push((r, url.to_string()));
            let hov = hit(r, self.hover.0, self.hover.1);
            let style = if hov { cyan.add_modifier(Modifier::UNDERLINED) } else { cyan };
            lines.push(Line::from(vec![
                Span::styled(" →  ", dim),
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
                Style::default().bg(row_hover_bg())
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
            // composing a *new* query puts you on the "+ new" slot, so the
            // highlight moves there and off the scope you came from —
            // editing an existing one leaves it where it is, since that
            // is the query being changed
            let composing_new = matches!(self.mode, Mode::AdsPrompt { edit: None, .. });
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

    /// Draw the active scope's table. Each scope contributes its own
    /// columns and rows; `table::draw` owns the chrome they share.
    /// Keep the cursor on a row that exists.
    ///
    /// A query refresh can hand back fewer records than the tab was
    /// holding — a smaller `+`/`-` limit, or simply a result set that
    /// moved — and nothing else brings the selection back into range, so
    /// the pub card renders blank and every row action has no target.
    /// The library has `refilter` for this; query and manuscript scopes
    /// had nothing, so it lives here, where row_count is authoritative.
    fn clamp_cursor(&mut self) {
        let n = self.row_count();
        let sel = match (self.table.selected(), n) {
            (_, 0) => None,
            (Some(p), n) => Some(p.min(n - 1)),
            (None, _) => Some(0),
        };
        self.table.select(sel);
    }

    fn draw_table(&mut self, f: &mut Frame, area: Rect) {
        self.table_area = area;
        self.sort_headers.clear();
        self.clamp_cursor();
        let model = self.table_model(area.width);
        // where the metric column landed, for the priority wheel: the
        // solver is the only thing that knows, and the wheel handler
        // wants the column's full height, header rows included
        self.metric_area = Rect::default();
        let widths = table::solve(&model.columns, area.width);
        let mut mx = area.x;
        for (spec, w) in model.columns.iter().zip(widths.iter()) {
            if spec.id == Col::Metric {
                self.metric_area = Rect { x: mx, y: area.y, width: *w, height: area.height };
            }
            mx += w + 1;
        }
        let (rects, data_area) = table::draw(f, area, model, &mut self.table);
        // rolling over a header says what the column is and that it sorts
        if let Some(&(_, col)) = rects.iter().find(|(r, _)| hit(*r, self.hover.0, self.hover.1)) {
            let what = self.column_hint(col);
            if !what.is_empty() {
                self.hover_hint = Some(format!("{what}  ·  click to sort"));
            }
        }
        self.sort_headers.extend(rects);
        if let Some(hint) = self.empty_hint() {
            draw_empty_hint(f, data_area, &hint);
        }
    }

    fn table_model(&self, width: u16) -> table::TableModel {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Manuscript { rows }) => self.manuscript_model(rows),
            Some(Scope::Ads { articles, .. }) => self.ads_model(articles, width),
            _ => self.library_model(width),
        }
    }

    /// The line drawn over an empty table, if the active scope has one.
    fn empty_hint(&self) -> Option<String> {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Manuscript { .. }) => None,
            Some(Scope::Ads { tab, state, .. }) if self.row_count() == 0 => {
                // Everything a query page has to say about being empty
                // says it *here*. The tab now exists from the moment the
                // query is sent, so "why is this empty" has a place to
                // live that stays put — a log line would scroll away
                // while the page it explains sat there saying nothing.
                Some(match state {
                    QueryState::Pending => {
                        format!("searching ADS for {} — results will appear here", tab.query)
                    }
                    QueryState::Failed(e) => format!("ADS search failed: {e}  ·  r retries"),
                    // A citation-graph query coming back empty is not a
                    // search that found nothing — it means ADS has no
                    // edge to follow, which is the normal state of a
                    // recent preprint whose reference list is not yet
                    // extracted. Telling the user to re-run invites them
                    // to wait for something that is not going to change.
                    QueryState::Ready if tab.query.starts_with("references(") => {
                        "ADS has not indexed this paper's references — for a new preprint they usually appear within days".to_string()
                    }
                    QueryState::Ready if tab.query.starts_with("citations(") => {
                        "ADS records nothing citing this paper yet".to_string()
                    }
                    QueryState::Ready => "no results — r re-runs, +/- changes n".to_string(),
                })
            }
            Some(Scope::Ads { .. }) => None,
            _ => {
                if self.order.is_empty() {
                    Some("library is empty — S searches ADS, or: astrobib add <bibcode>".to_string())
                } else if self.filtered.is_empty() {
                    Some("no matches — Esc clears the filter".to_string())
                } else {
                    None
                }
            }
        }
    }

    /// The selection gutter's glyph and style. `id` is the row's
    /// selection key, or None for a row that cannot be selected — a
    /// manuscript cite resolving to no paper shows no ring at all, so
    /// "you cannot select this" is visible rather than mysterious.
    ///
    /// The gutter also carries the cursor: ◉ marks the cursor row, and
    /// in selection mode the cursor's own circle brightens.
    fn gutter(&self, id: Option<&str>, at_cursor: bool) -> (&'static str, Style) {
        let circle = if !self.select_mode {
            if at_cursor {
                "◉"
            } else {
                ""
            }
        } else if id.is_some_and(|k| self.selected.contains(k)) {
            "◉"
        } else if id.is_some() {
            "◯"
        } else {
            ""
        };
        // unselected rings recede almost entirely; selected dots pop.
        // The ring is the *inactive form of the dot*, so it dims the
        // dot's own hue rather than doubling gray onto dim — dim gray
        // sinks out of sight altogether on lighter terminal themes,
        // where this still has to read as a thing you can click.
        let style = if circle == "◯" {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM)
        } else if self.select_mode && at_cursor {
            Style::default().fg(text_strong()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        (circle, style)
    }

    /// Manuscript: cite string, resolution state, resolved title. Rows
    /// arrive in scan order — the order the cites appear in the source —
    /// and stay there until a header is clicked; sorting by State is the
    /// useful one, since it gathers the missing cites at the top.
    fn manuscript_model(&self, rows: &[MsRow]) -> table::TableModel {
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

    /// ADS results: ↓ from the canonical cache key (the cite key once
    /// imported, the bibcode otherwise), ● from paper identity.
    fn ads_model(&self, articles: &[crate::ads::Article], width: u16) -> table::TableModel {
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

    /// The library: a subtle per-column palette, with the terminal theme
    /// supplying the hues. The cursor row takes a faint cool fill and a
    /// ◉; a hovered row takes no fill — its text lifts one level instead.
    fn library_model(&self, width: u16) -> table::TableModel {
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

    /// Every column a scope can draw, in order, shown or not — the list
    /// the columns panel offers. Widths here are the responsive
    /// defaults and so is visibility; `columns_for` applies the user's
    /// overrides on top.
    fn all_columns(&self, kind: ScopeKind, width: u16) -> Vec<table::ColumnSpec> {
        match kind {
            ScopeKind::Manuscript => vec![
                table::fixed(Col::Sel, "", 2, false),
                // the glyph column has no label, so nothing to click; the
                // State column beside it sorts by the same fact
                table::fixed(Col::CiteIcon, "", 2, false),
                table::fixed(Col::Cited, "Cited", 26, true),
                table::fixed(Col::State, "State", 10, true),
                table::flex(Col::Title, "Title", true),
            ],
            ScopeKind::Query => {
                let (author_w, _) = column_layout(width);
                vec![
                    table::fixed(Col::Sel, "", 2, false),
                    metric_column(self.metric_col),
                    // indicator columns carry a single glyph and are
                    // locked to one cell: there is nothing to resize
                    table::fixed(Col::Pdf, "↓", 1, true).fixed_size(),
                    table::fixed(Col::InLib, "●", 1, true).fixed_size(),
                    table::fixed(Col::Year, "Year", 6, true),
                    // the two clocks sit side by side on purpose: Year is
                    // when the paper was published, Entered is when ADS
                    // indexed it, and a date-sorted query needs the second
                    table::fixed(Col::Entered, "Entered", 10, true),
                    table::fixed(Col::Author, "Author", author_w, true),
                    table::flex(Col::Title, "Title", true),
                    table::fixed(Col::Key, "Key", 20, true),
                ]
            }
            ScopeKind::Library => {
                // responsive defaults: author scales, Key drops first when
                // tight — but never while the card is shown, since the Key
                // column is the hover-preview target, so the title absorbs
                // the squeeze instead
                let (author_w, show_key) = column_layout(width);
                let show_membership = self.lib.manuscript.is_some() && self.lib.global_on;
                vec![
                    table::fixed(Col::Sel, "", 2, false),
                    metric_column(self.metric_col),
                    table::fixed(Col::Pdf, "↓", 1, true).fixed_size(),
                    // the ● column is only labelled — and only sorts —
                    // when a manuscript is active and the global tier shows
                    table::fixed(
                        Col::InLib,
                        if show_membership { "●" } else { "" },
                        1,
                        show_membership,
                    )
                    .fixed_size(),
                    table::fixed(Col::Year, "Year", 6, true),
                    table::fixed(Col::Author, "Author", author_w, true),
                    table::flex(Col::Title, "Title", true),
                    table::fixed(Col::Key, "Key", 20, true)
                        .default_when(show_key || self.show_detail),
                ]
            }
        }
    }

    /// Whether one column is drawn, honouring the user's override where
    /// there is one and the scope's default where there is not.
    fn column_shown(&self, kind: ScopeKind, id: Col) -> bool {
        let all = self.all_columns(kind, self.table_area.width);
        let Some(spec) = all.iter().find(|c| c.id == id) else {
            return false;
        };
        self.columns
            .get(&kind)
            .and_then(|cfg| cfg.visible.get(&id).copied())
            .unwrap_or(spec.default_visible)
    }

    /// Which column absorbs the leftover width when the natural one is
    /// hidden, best first. Title is the natural choice everywhere — it
    /// is the column that benefits most from room and degrades most
    /// gracefully without it — but nothing about it is special, so
    /// hiding it just moves the job to the next candidate.
    fn flex_preference(kind: ScopeKind) -> &'static [Col] {
        match kind {
            ScopeKind::Manuscript => &[Col::Title, Col::Cited, Col::State],
            _ => &[Col::Title, Col::Author, Col::Key],
        }
    }

    /// The columns the table actually draws: `all_columns` with the
    /// user's overrides applied, the metric strip removed (it is drawn
    /// beside the table, not inside it), and the flex role reassigned if
    /// whichever column had it is now hidden.
    fn columns_for(&self, kind: ScopeKind, width: u16) -> Vec<table::ColumnSpec> {
        let cfg = self.columns.get(&kind);
        let mut cols: Vec<table::ColumnSpec> = self
            .all_columns(kind, width)
            .into_iter()
            .filter(|c| {
                cfg.and_then(|cfg| cfg.visible.get(&c.id).copied())
                    .unwrap_or(c.default_visible)
            })
            .collect();
        if let Some(cfg) = cfg {
            for c in cols.iter_mut() {
                if let (table::Width::Fixed(_), Some(&w)) = (c.width, cfg.widths.get(&c.id)) {
                    c.width = table::Width::Fixed(w);
                }
            }
        }
        if !cols.iter().any(|c| matches!(c.width, table::Width::Flex)) {
            // last resort is the rightmost labelled column: something has
            // to take the slack or the table strands a blank margin, and
            // stretching the last one looks deliberate where a gap does not
            let pick = Self::flex_preference(kind)
                .iter()
                .find(|id| cols.iter().any(|c| c.id == **id))
                .copied()
                .or_else(|| cols.iter().rev().find(|c| !c.header.is_empty()).map(|c| c.id));
            if let Some(id) = pick {
                if let Some(c) = cols.iter_mut().find(|c| c.id == id) {
                    c.width = table::Width::Flex;
                }
            }
        }
        // whatever ended up flexing has a derived width, so ←/→ mean
        // nothing for it — wherever the role landed
        for c in cols.iter_mut() {
            if matches!(c.width, table::Width::Flex) {
                c.resizable = false;
            }
        }
        cols
    }

    fn load_column_config() -> std::collections::HashMap<ScopeKind, table::ColumnConfig> {
        let mut out = std::collections::HashMap::new();
        let Some(v) = crate::ads::get_state_value("columns") else {
            return out;
        };
        for kind in [ScopeKind::Library, ScopeKind::Manuscript, ScopeKind::Query] {
            if let Some(o) = v.get(kind.tag()) {
                let cfg = table::ColumnConfig::from_json(o);
                if !cfg.is_empty() {
                    out.insert(kind, cfg);
                }
            }
        }
        out
    }

    fn save_column_config(&mut self) {
        let mut o = serde_json::Map::new();
        for (kind, cfg) in &self.columns {
            if !cfg.is_empty() {
                o.insert(kind.tag().to_string(), cfg.to_json());
            }
        }
        let res = crate::ads::save_state_value("columns", serde_json::Value::Object(o));
        self.state_write("state.json", res.err().map(|e| e.to_string()));
    }

    /// Keys the columns panel claims while it has focus. Returns false
    /// for anything it does not handle, which then reaches the table.
    fn columns_panel_key(&mut self, code: KeyCode) -> bool {
        let rows = self.panel_rows();
        if rows.is_empty() {
            return false;
        }
        self.col_sel = self.col_sel.min(rows.len() - 1);
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.col_sel = self.col_sel.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.col_sel = (self.col_sel + 1).min(rows.len() - 1);
            }
            KeyCode::Left => self.nudge_width(-1),
            KeyCode::Right => self.nudge_width(1),
            KeyCode::Char(' ') => {
                let PanelRow::Column(id) = rows[self.col_sel];
                self.toggle_column(id);
            }
            KeyCode::Char('s') | KeyCode::Enter => {
                let PanelRow::Column(id) = rows[self.col_sel];
                self.sort_by(id);
            }
            // Tab and Esc both hand the arrows back without closing the
            // panel; Tab is the reversible one, so it reads as a toggle
            // between the two things the arrows can drive
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => self.focus = Focus::Table,
            _ => return false,
        }
        true
    }

    /// Show or hide one column in the active scope kind. Hiding the one
    /// that was absorbing the leftover width is allowed; the job moves
    /// to the next candidate (see `flex_preference`).
    fn toggle_column(&mut self, id: Col) {
        let kind = self.active_kind();
        let now_shown = self.column_shown(kind, id);
        let default = self
            .all_columns(kind, self.table_area.width)
            .iter()
            .find(|c| c.id == id)
            .is_some_and(|c| c.default_visible);
        let cfg = self.columns.entry(kind).or_default();
        // storing an override that agrees with the default would pin a
        // responsive rule in place, so put the column back under it
        if !now_shown == default {
            cfg.visible.remove(&id);
        } else {
            cfg.visible.insert(id, !now_shown);
        }
        let shown_after = !now_shown;
        // the metric strip is also a sort target; hiding it while it is
        // the sort would leave the marker on a column nobody can see
        if now_shown && id == Col::Metric && self.sort().is_some_and(|(c, _)| c == Col::Metric) {
            self.set_sort((Col::Year, false));
            self.apply_sort();
        }
        self.save_column_config();
        self.note_latest(
            MsgCat::Info,
            "column",
            format!(
                "{} column {}",
                self.column_hint(id),
                if shown_after { "shown" } else { "hidden" }
            ),
        );
    }

    /// ←/→ on a column: widen or narrow it, pinning the width. Until
    /// this is used a column keeps its responsive default, which is what
    /// makes an untouched configuration indistinguishable from none.
    /// A row with no settable width simply does not move. It says
    /// nothing about it: the panel already draws those rows without the
    /// ‹ › nudges, so the affordance is absent rather than refused, and
    /// a warning for pressing an arrow at it would be noise.
    fn nudge_width(&mut self, d: i16) {
        let kind = self.active_kind();
        let rows = self.panel_rows();
        let Some(&PanelRow::Column(id)) = rows.get(self.col_sel) else {
            return;
        };
        let shown = self.columns_for(kind, self.table_area.width);
        // hidden, or drawn outside the table (the metric swatch), or
        // locked to a derived or one-cell width
        if !shown.iter().any(|c| c.id == id && c.resizable) {
            return;
        }
        let cur = col_width(&shown, self.table_area.width, id) as i16;
        let next = (cur + d).clamp(table::MIN_COL_W as i16, table::MAX_COL_W as i16) as u16;
        self.columns.entry(kind).or_default().widths.insert(id, next);
        self.save_column_config();
        self.note_latest(
            MsgCat::Info,
            "width",
            format!("{} column {next} wide", self.column_hint(id)),
        );
    }

    /// Every row's metric value in the active scope, in row order, with
    /// the known ones pooled for rank-normalizing. None where a paper
    /// has no value for the metric on show.
    fn metric_values(&self) -> (Vec<Option<f64>>, Vec<f64>) {
        let metric = self.metric_col;
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let values: Vec<Option<f64>> = match self.scopes.get(self.active_scope) {
            Some(Scope::Ads { articles, .. }) => articles
                .iter()
                .map(|a| match metric {
                    MetricCol::Citations => a.citation_count.map(|c| c as f64),
                    MetricCol::Priority => self
                        .article_entry(a)
                        .and_then(|e| self.metrics.get(e.key()))
                        .and_then(|m| m.effective_priority(now_ts)),
                })
                .collect(),
            _ => self
                .filtered
                .iter()
                .filter_map(|&i| self.entry_at(i))
                .map(|e| {
                    let m = self.metrics.get(e.key());
                    match metric {
                        MetricCol::Priority => m.and_then(|m| m.effective_priority(now_ts)),
                        MetricCol::Citations => m.and_then(|m| m.citations).map(|c| c as f64),
                    }
                })
                .collect(),
        };
        let known: Vec<f64> = values.iter().flatten().copied().collect();
        (values, known)
    }

    /// What a column header means, in words, for the footer rollover.
    /// The ● column is the clearest case for having this at all: it says
    /// something different on a query page than in the library.
    fn column_hint(&self, col: Col) -> &'static str {
        let query = self.active_kind() == ScopeKind::Query;
        match col {
            Col::Metric => "metric",
            Col::Pdf => "PDF cached",
            Col::InLib => {
                if query {
                    "already in your library"
                } else {
                    "in the manuscript"
                }
            }
            Col::Year => "year published",
            Col::Entered => "when ADS indexed it",
            Col::Author => "authors",
            Col::Title => "title",
            Col::Key => "cite key",
            Col::Cited => "the cite as written",
            Col::State => "whether the cite resolves",
            Col::Sel | Col::CiteIcon => "",
        }
    }

    fn active_kind(&self) -> ScopeKind {
        self.scopes
            .get(self.active_scope)
            .map(Scope::kind)
            .unwrap_or(ScopeKind::Library)
    }

    /// What the columns panel lists for the active scope: every column
    /// it can draw — shown or not, which is the point, since sorting by
    /// a hidden column is allowed — followed, in a query scope, by the
    /// ADS sort that decides which records come back at all.
    ///
    /// Structural columns are left out: the selection gutter and the
    /// manuscript's state glyph have no label to click and nothing worth
    /// configuring.
    fn panel_rows(&self) -> Vec<PanelRow> {
        let kind = self.active_kind();
        self.all_columns(kind, self.table_area.width)
            .into_iter()
            .filter(|c| !c.header.is_empty() || c.sortable)
            .map(|c| PanelRow::Column(c.id))
            .collect()
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

    /// Where the footer's view badges sit, and what each one is:
    /// `(rect, action, label, currently on)`, right-aligned.
    ///
    /// Separate from drawing them because the hover hint has to be
    /// decided *before* the footer line is built — `draw_badges` runs
    /// after it, so a hint set there would appear a frame late.
    fn badge_layout(&self, area: Rect) -> Vec<(Rect, Action, &'static str, bool)> {
        let mut badges: Vec<(&'static str, bool, Action)> = vec![];
        if self.lib.manuscript.is_some() {
            badges.push(("global", self.lib.global_on, Action::GlobalTier));
        }
        badges.extend([
            ("card", self.show_detail, Action::Card),
            ("table", self.show_columns, Action::Columns),
            ("log", self.show_log, Action::Log),
            ("keys", self.show_help, Action::Help),
        ]);
        let total: u16 = badges.iter().map(|(l, _, _)| l.chars().count() as u16 + 3).sum();
        let mut bx = (area.x + area.width).saturating_sub(total);
        badges
            .into_iter()
            .map(|(label, on, action)| {
                let wl = label.chars().count() as u16 + 2;
                let r = Rect { x: bx, y: area.y, width: wl, height: 1 };
                bx += wl + 1;
                (r, action, label, on)
            })
            .collect()
    }

    /// What a hovered view badge says in the footer: whether clicking it
    /// shows or hides, what, and which key does the same thing. The key
    /// comes from the cheat-sheet table so the two cannot drift apart.
    fn badge_hint(&self, area: Rect) -> Option<String> {
        let (_, action, label, on) = self
            .badge_layout(area)
            .into_iter()
            .find(|(r, ..)| hit(*r, self.hover.0, self.hover.1))?;
        let what = match action {
            Action::GlobalTier => "the global tier",
            Action::Card => "the pub card",
            Action::Columns => "the table panel",
            Action::Log => "the event log",
            Action::Help => "the cheat-sheet",
            _ => label,
        };
        let key = HELP_ENTRIES
            .iter()
            .find(|(.., a, _)| *a == Some(action))
            .map(|(k, ..)| *k)
            .unwrap_or("");
        let verb = if on { "hide" } else { "show" };
        Some(format!("{verb} {what}  ·  {key}"))
    }

    /// Right-aligned clickable show/hide badges for each app-wide view.
    fn draw_badges(&mut self, f: &mut Frame, area: Rect) {
        let layout = self.badge_layout(area);
        self.footer_badges.clear();
        let mut spans: Vec<Span> = vec![];
        let mut total = 0u16;
        for (r, action, label, on) in layout {
            self.footer_badges.push((r, action));
            total += r.width + 1;
            let hov = hit(r, self.hover.0, self.hover.1);
            let style = match (on, hov) {
                (true, true) => Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED),
                (true, false) => Style::default().fg(Color::Cyan),
                (false, true) => Style::default().fg(Color::Gray).add_modifier(Modifier::UNDERLINED),
                (false, false) => Style::default().fg(Color::DarkGray),
            };
            spans.push(Span::styled(
                format!("{} {label}", if on { "■" } else { "□" }),
                style,
            ));
            spans.push(Span::raw(" "));
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
        let title = if scroll > 0 {
            format!(" Log ↑{scroll} ")
        } else {
            " Log ".to_string()
        };
        // heading first, on the row the top border used to occupy
        let mut lines: Vec<Line> =
            vec![Line::from(Span::styled(title, Style::default().fg(Color::DarkGray)))];
        for (cat, secs, msg) in &self.log[start..end] {
            lines.push(Line::from(vec![
                Span::styled(
                    // the leading space lines the entries up under the
                    // heading, which the border used to do for both
                    format!(" {:02}:{:02}  ", secs / 60, secs % 60),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(msg.clone(), Style::default().fg(cat.color())),
            ]));
        }
        f.render_widget(Block::default().style(Style::default().bg(log_bg())), area);
        f.render_widget(Paragraph::new(Text::from(lines)), panel_body(area));
    }

    /// One rendered line of the columns sidebar: its spans, the click
    /// targets it carries as offsets from the panel's left edge, and the
    /// list row it belongs to when it is selectable (headings and rules
    /// are not).
    ///
    /// Lines are built before they are placed so the list can be
    /// windowed: the panel is as tall as the terminal allows, and
    /// without this the cursor walked off the bottom into rows that were
    /// never drawn.
    fn panel_lines(
        &self,
        kind: ScopeKind,
        focused: bool,
        preview: Option<Col>,
    ) -> Vec<PanelLine> {
        let shown = self.columns_for(kind, self.table_area.width);
        let all = self.all_columns(kind, self.table_area.width);
        let cfg = self.columns.get(&kind).cloned().unwrap_or_default();
        let sort = self.sort();
        let dim = Style::default().fg(Color::DarkGray);
        let mut out: Vec<PanelLine> = vec![];
        for (i, row) in self.panel_rows().iter().enumerate() {
            let on_cursor = focused && self.col_sel == i;
            match *row {
                PanelRow::Column(id) => {
                    let spec = all.iter().find(|c| c.id == id);
                    let drawn = shown.iter().find(|c| c.id == id);
                    // an icon column's header is one glyph, which names
                    // nothing on its own: pair it with the column's tag
                    // so the panel row is readable and still visibly the
                    // same column as the header in the table
                    let head = spec.map(|c| c.header.clone()).unwrap_or_default();
                    let label = if head.chars().count() > 1 {
                        head
                    } else if head.is_empty() {
                        id.tag().to_string()
                    } else {
                        format!("{head} {}", id.tag())
                    };
                    // asked of the configuration, not of the drawn list:
                    // the metric swatch is shown beside the table rather
                    // than in it, so it never appears in `shown`
                    let visible = self.column_shown(kind, id);
                    // read from the drawn spec, not the declared one:
                    // the flex role moves when a column is hidden, so
                    // which column has a derived width moves with it
                    let resizable = drawn.is_some_and(|c| c.resizable);
                    // "no width to set" and "width is derived" are not
                    // the same: a one-cell indicator column has a width,
                    // it just is not yours
                    let flex = drawn.is_some_and(|c| matches!(c.width, table::Width::Flex));
                    let sortable = spec.is_some_and(|c| c.sortable);
                    let w = col_width(&shown, self.table_area.width, id);
                    // The row's cells, as offsets from the panel's left
                    // edge — spelled out because the click rects have to
                    // agree with the spans built below, and drifted once:
                    //
                    //   0..2   "✓ "        toggle
                    //   2..13  label       (not a target)
                    //   13..15 "‹ "        narrower
                    //   15..19 width       (not a target)
                    //   19..21 " ›"        wider
                    //   21..23 " ▲"        sort — the marker is the most
                    //                      obvious thing to click for it
                    let mut hits = vec![(0, 2, PanelHit::Toggle(id))];
                    if sortable {
                        hits.push((21, 2, PanelHit::Sort(id)));
                    }
                    if visible && resizable {
                        hits.push((13, 1, PanelHit::Narrower(id)));
                        hits.push((20, 1, PanelHit::Wider(id)));
                    }
                    let (mark, mark_style) = if visible {
                        ("✓", Style::default().fg(Color::Green))
                    } else {
                        ("·", dim)
                    };
                    let lab_style = if !visible {
                        dim
                    } else if on_cursor {
                        Style::default().fg(text_strong()).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(table_text())
                    };
                    // the metric strip says which metric it is showing —
                    // M toggles that, and nothing else on screen names it
                    let (wtext, nudges) = if !visible {
                        ("    ".to_string(), ("  ", "  "))
                    } else if id == Col::Metric {
                        (
                            match self.metric_col {
                                MetricCol::Priority => "prio",
                                MetricCol::Citations => "cite",
                            }
                            .to_string(),
                            ("  ", "  "),
                        )
                    } else if flex {
                        // whatever is absorbing the leftover width: its
                        // size is derived, so there is nothing to set
                        ("   —".to_string(), ("  ", "  "))
                    } else if resizable {
                        (format!("{w:>4}"), ("‹ ", " ›"))
                    } else {
                        // a fixed one-cell column: a real width, locked
                        (format!("{w:>4}"), ("  ", "  "))
                    };
                    // The marker cell is the only sort control, so on a
                    // column that is not the sort column it would be
                    // blank and advertise nothing: hovering it previews,
                    // faintly, the arrow a click would leave behind.
                    //
                    // The sort column itself keeps showing its real
                    // arrow. Previewing the flip there would mean that
                    // the instant after clicking — mouse still on the
                    // cell — the marker showed the opposite of what the
                    // click had just done, which reads as the click
                    // having gone the wrong way.
                    let (marker, marker_style) = if let Some((_, asc)) =
                        sort.filter(|(c, _)| *c == id)
                    {
                        (arrow(asc), Style::default().fg(Color::Cyan))
                    } else if sortable && preview == Some(id) {
                        (arrow(self.next_sort(id).1), Style::default().fg(divider_fg()))
                    } else {
                        (" ", Style::default())
                    };
                    out.push(PanelLine {
                        spans: vec![
                            Span::styled(format!("{mark} "), mark_style),
                            Span::styled(format!("{label:<11}"), lab_style),
                            Span::styled(nudges.0, dim),
                            Span::styled(
                                wtext,
                                // a pinned width is the user's, not the
                                // responsive default — worth saying so
                                if cfg.widths.contains_key(&id) {
                                    Style::default().fg(Color::Yellow)
                                } else {
                                    dim
                                },
                            ),
                            Span::styled(nudges.1, dim),
                            Span::raw(" "),
                            Span::styled(marker, marker_style),
                        ],
                        hits,
                        row: Some(i),
                        fill: on_cursor,
                    });
                }
            }
        }
        out
    }

    /// The columns sidebar: every column the active scope can draw, with
    /// its visibility, its width, and whether it is the sort target.
    ///
    /// Sorting is offered on hidden columns too, which is the point of
    /// listing them: a query can be ordered by entry date without
    /// spending ten columns of screen on the dates themselves.
    ///
    /// The column showing "—" for its width is the one absorbing the
    /// leftover space; hide it and the "—" moves to whichever column
    /// takes over.
    fn draw_columns_panel(&mut self, f: &mut Frame, area: Rect) {
        self.col_rects.clear();
        let kind = self.active_kind();
        let focused = self.focus == Focus::Columns;
        // no edge rule, mirroring the pub card on the other side of the
        // table: each panel's tint is its own boundary
        f.render_widget(Block::default().style(Style::default().bg(panel_bg())), area);
        // laid out as the pub card's mirror image: the card insets its
        // text 3 cells from the table and 2 from the far edge and starts
        // one line down, so this does the same on the other side
        let inner = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(5),
            height: area.height.saturating_sub(1),
        };
        let head = |focused: bool| {
            vec![
                PanelLine::title("Table configuration", focused),
                PanelLine::blank(),
            ]
        };
        let mut lines: Vec<PanelLine> = head(focused);
        lines.extend(self.panel_lines(kind, focused, None));

        // window the list so the cursor is always on screen: scroll only
        // as far as it takes, so the heading stays put until the list
        // genuinely outgrows the pane
        let h = inner.height as usize;
        let cursor = lines.iter().position(|l| l.fill).unwrap_or(0);
        let start = if lines.len() <= h {
            0
        } else {
            cursor
                .saturating_sub(h.saturating_sub(1))
                .min(lines.len() - h)
        };

        // Which sort control is the mouse on? Answering it needs the
        // window, and the window needs the lines — but neither depends
        // on the preview, so building twice is safe and costs a dozen
        // lines. Reading last frame's rects instead would go stale for a
        // frame on every resize and scroll.
        let preview = (self.hover.0 >= inner.x + 21 && self.hover.0 < inner.x + 23)
            .then(|| self.hover.1.checked_sub(inner.y))
            .flatten()
            .filter(|n| (*n as usize) < h)
            .and_then(|n| lines.get(start + n as usize))
            .and_then(|l| l.row)
            .and_then(|i| match self.panel_rows().get(i) {
                Some(&PanelRow::Column(id)) => Some(id),
                _ => None,
            });
        if preview.is_some() {
            lines = head(focused);
            lines.extend(self.panel_lines(kind, focused, preview));
        }

        let mut rendered: Vec<Line> = vec![];
        for (n, line) in lines.iter().skip(start).take(h).enumerate() {
            let y = inner.y + n as u16;
            for &(dx, w, hitkind) in &line.hits {
                self.col_rects.push((
                    Rect { x: inner.x + dx, y, width: w, height: 1 },
                    hitkind,
                ));
            }
            if let Some(i) = line.row {
                self.col_rects.push((
                    Rect { x: inner.x, y, width: inner.width, height: 1 },
                    PanelHit::Row(i),
                ));
            }
            rendered.push(line.to_line());
        }
        // rolling over a control says what it does and which key does it
        if let Some(&(_, h)) = self
            .col_rects
            .iter()
            .find(|(r, h)| !matches!(h, PanelHit::Row(_)) && hit(*r, self.hover.0, self.hover.1))
        {
            self.hover_hint = Some(match h {
                PanelHit::Toggle(id) => format!("show / hide {}  ·  space", id.tag()),
                PanelHit::Sort(id) => format!("sort by {} (shown or not)  ·  s", id.tag()),
                PanelHit::Narrower(id) => format!("narrow {}  ·  ←", id.tag()),
                PanelHit::Wider(id) => format!("widen {}  ·  →", id.tag()),
                PanelHit::Row(_) => String::new(),
            });
        }
        f.render_widget(Paragraph::new(Text::from(rendered)), inner);
    }

    /// The active prompt's label, its text, the cursor offset into that
    /// text, and how much of the first row its trailing parameters need.
    ///
    /// Only the two prompts that hold *queries* are here. A path or a
    /// name is short by nature and keeps the old horizontal scroll; a
    /// query grows until you cannot see what you are editing, which is
    /// the whole reason for wrapping.
    fn prompt_layout(&self) -> Option<(String, String, usize, usize)> {
        match &self.mode {
            Mode::AdsPrompt { input, limit, sort, edit } => {
                let label = if edit.is_some() { "edit query: " } else { "ADS query:  " };
                let suffix = format!(
                    "   ADS returns {limit} (↑↓) {} (⌃r)   ⏎ search · Esc cancel",
                    ads_sort_name(sort)
                );
                Some((
                    label.to_string(),
                    input.value().to_string(),
                    input.visual_cursor(),
                    suffix.chars().count(),
                ))
            }
            Mode::Filter => Some((
                "/".to_string(),
                self.filter.value().to_string(),
                self.filter.visual_cursor(),
                0,
            )),
            _ => None,
        }
    }

    /// Rows the prompt itself needs, not counting the rule above it.
    ///
    /// One, until the text no longer fits beside its parameters. Then
    /// the text takes the full width across as many rows as it needs and
    /// the parameters move to a row of their own — so a short query
    /// looks exactly as it always did, and a long one is legible instead
    /// of scrolled off the end.
    fn prompt_height(&self, width: u16) -> u16 {
        let Some((label, text, cursor, reserve)) = self.prompt_layout() else {
            return 1;
        };
        let lw = label.chars().count();
        let n = text.chars().count();
        if n < (width as usize).saturating_sub(lw + reserve) {
            return 1;
        }
        let body_w = Self::prompt_body_w(width, lw);
        let rows = (n.max(cursor) + 1).div_ceil(body_w).max(1);
        // capped: a prompt may borrow the screen, not take it
        ((rows + 1) as u16).min(8)
    }

    fn prompt_body_w(width: u16, label_w: usize) -> usize {
        (width as usize).saturating_sub(label_w + 1).max(1)
    }

    /// Draw the prompt wrapped over several rows, returning false if it
    /// fits on one and the ordinary single-row path should run.
    ///
    /// The text stays one logical line throughout — there is no newline
    /// in it and none is sent to ADS, which has no use for one. Wrapping
    /// breaks at the column rather than at word boundaries so that the
    /// cursor maps to a row and a column by division: exact, and with no
    /// reflow surprise while you are typing in the middle of it.
    fn draw_wrapped_prompt(&mut self, f: &mut Frame, area: Rect) -> bool {
        if self.prompt_height(area.width) <= 1 {
            return false;
        }
        let Some((label, text, cursor, _)) = self.prompt_layout() else {
            return false;
        };
        let lw = label.chars().count();
        let body_w = Self::prompt_body_w(area.width, lw);
        let chars: Vec<char> = text.chars().collect();
        let mut rows: Vec<String> =
            chars.chunks(body_w).map(|c| c.iter().collect()).collect();
        let crow = cursor / body_w;
        // the cursor sits one past the end when the text ends exactly on
        // a row boundary, so that row has to exist to put it on
        while rows.len() <= crow {
            rows.push(String::new());
        }
        let text_rows = (area.height as usize).saturating_sub(1);
        let mut lines: Vec<Line> = vec![];
        for (i, r) in rows.iter().take(text_rows).enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    if i == 0 { label.clone() } else { " ".repeat(lw) },
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(r.clone()),
            ]));
        }
        // the parameters and hints, on a row of their own now that the
        // text has taken the width
        let last_y = area.y + area.height - 1;
        let mut tail: Vec<Span> = vec![Span::raw(" ".repeat(lw))];
        if let Mode::AdsPrompt { limit, ref sort, .. } = self.mode {
            let head = format!("ADS returns {limit} (↑↓) ");
            let action = format!("{} (⌃r)", ads_sort_name(sort));
            let r = Rect {
                x: area.x + lw as u16 + head.chars().count() as u16,
                y: last_y,
                width: action.chars().count() as u16,
                height: 1,
            };
            self.prompt_sort_rect = r;
            let hov = hit(r, self.hover.0, self.hover.1);
            tail.push(Span::styled(head, Style::default().fg(Color::Gray)));
            tail.push(Span::styled(
                action,
                if hov {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ));
            tail.push(Span::styled(
                "   ⏎ search · Esc cancel",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            tail.push(Span::styled(
                "⏎ apply · Esc clears",
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(tail));
        f.render_widget(Paragraph::new(Text::from(lines)), area);
        f.set_cursor_position((
            area.x + lw as u16 + (cursor % body_w) as u16,
            area.y + crow.min(text_rows.saturating_sub(1)) as u16,
        ));
        self.draw_badges(f, Rect { x: area.x, y: last_y, width: area.width, height: 1 });
        true
    }

    fn draw_status(&mut self, f: &mut Frame, area: Rect) {
        // no rule above: the footer's own tint separates it from the
        // table, the way every other surface here is separated
        f.render_widget(Block::default().style(Style::default().bg(footer_bg())), area);
        if self.draw_wrapped_prompt(f, area) {
            return;
        }
        let area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
        // the badges live on this same line and are drawn after it, so
        // their hover hint has to be settled before the line is built
        if let Some(hint) = self.badge_hint(area) {
            self.hover_hint = Some(hint);
        }
        // room left of the badges, which are drawn after this line and
        // over it: whatever goes here has to fit in front of them
        let free = self
            .badge_layout(area)
            .first()
            .map(|(r, ..)| r.x.saturating_sub(area.x).saturating_sub(2))
            .unwrap_or(area.width);
        // the prompt's control rect, published after the match: the arm
        // borrows self.mode, so it cannot write to self while building
        let mut sort_rect = Rect::default();
        // cleared every frame; the Normal arm re-publishes it when the
        // line has room for the affordance
        self.edit_query_rect = Rect::default();
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
            Mode::Rename { ref input } => {
                let label = "name this query: ";
                let prefix = label.chars().count() as u16;
                let avail = area.width.saturating_sub(prefix + 30) as usize;
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
                        "   ⏎ rename · Esc cancel",
                        Style::default().fg(Color::DarkGray),
                    ),
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
            Mode::AdsPrompt { ref input, limit, ref sort, edit } => {
                let label = if edit.is_some() { "edit query: " } else { "ADS query:  " };
                let prefix = label.chars().count() as u16;
                let avail = area.width.saturating_sub(prefix + 74) as usize;
                let scroll = input.visual_scroll(avail.max(10));
                let shown: String = input.value().chars().skip(scroll).collect();
                f.set_cursor_position((
                    area.x + prefix + (input.visual_cursor().saturating_sub(scroll)) as u16,
                    area.y,
                ));
                // One phrase for what ADS will return: how many, and by
                // what. Only the mode is a control — the limit has ↑↓,
                // which no pointer can press — so only the mode carries
                // the hover and the click rect.
                let head = format!("ADS returns {limit} (↑↓) ");
                let action = format!("{} (⌃r)", ads_sort_name(sort));
                let x0 = area.x + prefix + shown.chars().count() as u16 + 3;
                sort_rect = Rect {
                    x: x0 + head.chars().count() as u16,
                    y: area.y,
                    width: action.chars().count() as u16,
                    height: 1,
                };
                let hov = hit(sort_rect, self.hover.0, self.hover.1);
                Line::from(vec![
                Span::styled(label, Style::default().fg(Color::Cyan)),
                Span::raw(shown),
                Span::raw("   "),
                Span::styled(head, Style::default().fg(Color::Gray)),
                Span::styled(
                    action,
                    if hov {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED)
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                ),
                Span::styled(
                    "   ⏎ search · Esc cancel",
                    Style::default().fg(Color::DarkGray),
                ),
                ])
            }
            Mode::Copy => Line::from(vec![
                Span::styled("copy: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    // "copy: " already spent six of them
                    self.copy_menu(free.saturating_sub(6)),
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
                let mut x = area.x;
                if pending > 0 {
                    let label = format!("⧗{pending} ");
                    x += label.chars().count() as u16;
                    spans.push(Span::styled(label, Style::default().fg(Color::Yellow)));
                }
                let counts = format!("{n}/{total}  ·  ");
                x += counts.chars().count() as u16;
                spans.push(Span::styled(counts, Style::default().fg(Color::DarkGray)));
                // With nothing to report, offer the thing this scope is
                // for. A prompt or a fresh message outranks it: they are
                // here because you would have missed them, and this is
                // only here because the line was going to be blank.
                if msg.is_empty() {
                    let label = match self.scopes.get(self.active_scope) {
                        Some(Scope::Ads { .. }) => "edit query (E)",
                        _ => "new ADS query (S)",
                    };
                    let r = Rect {
                        x,
                        y: area.y,
                        width: label.chars().count() as u16,
                        height: 1,
                    };
                    self.edit_query_rect = r;
                    let hov = hit(r, self.hover.0, self.hover.1);
                    spans.push(Span::styled(
                        label,
                        if hov {
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ));
                } else {
                    spans.push(Span::styled(msg, Style::default().fg(msg_color)));
                }
                spans.push(Span::styled(filt, Style::default().fg(Color::DarkGray)));
                Line::from(spans)
            }
        };
        self.prompt_sort_rect = sort_rect;
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

/// Read the system clipboard, or None where there is no way to.
///
/// There is no OSC 52 counterpart: terminals overwhelmingly refuse to
/// *answer* a clipboard read, since that would let any program running
/// in them exfiltrate whatever the user last copied. So this is the
/// platform tool or nothing, and "nothing" is reported rather than
/// guessed at.
fn read_clipboard() -> Option<String> {
    use std::process::Command;
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ]
    };
    for (bin, args) in candidates {
        if let Ok(out) = Command::new(bin).args(*args).output() {
            if out.status.success() {
                return String::from_utf8(out.stdout).ok();
            }
        }
    }
    None
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
                        matches!(t.value.as_str(), "ms" | "pdf"),
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
        let readme = include_str!("../README.md");
        for (q, _) in super::ADS_SAMPLES.iter().chain(super::FILTER_SAMPLES.iter()) {
            assert!(readme.contains(q), "README does not document the sample `{q}`");
        }
    }

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
            ("Quist2019abcde", "UXVpc3QyMDE5YWJjZGU="),
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

/// "Quist, J. and Blomqvist, A." → "Quist, Blomqvist" (surnames, truncated).
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
