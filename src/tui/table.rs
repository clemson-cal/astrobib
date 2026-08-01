//! The table chrome: one renderer for all three scopes.
//!
//! The library, the manuscript, and an ADS query all present a table,
//! and their *cells* are legitimately different — papers, cite strings,
//! and search results are not the same thing, so a column × row-kind
//! matrix would be the wrong shape. What they share is everything
//! around the cells: header labels with ▲/▼ sort markers and hover
//! underlines, the click rects those markers answer to, the divider
//! rule, width solving, and the cursor fill.
//!
//! That chrome used to be written out three times in `draw_table`, and
//! drifted: two different formulas computed the same title width, one
//! branch clamped its padding and the others did not, and the
//! manuscript header quietly had no sort rects at all. Here each scope
//! contributes a `Vec<ColumnSpec>` and its own rows, and `draw` owns
//! the rest — so a chrome change lands on all three or none.
//!
//! `tests/tui/scenarios/s26_table_chrome.py` pins the rendered result.

use super::*;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, TableState};
use ratatui::Frame;

/// Every column the app can draw, across all scopes. Also the sort
/// vocabulary: `Metric` names the swatch strip beside the table, which
/// is a sort target without being a table column.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Col {
    Metric,
    /// the selection gutter — cursor ◉ and selection ◯/◉
    Sel,
    Pdf,
    InLib,
    Year,
    Author,
    Title,
    Key,
    /// when ADS indexed the record — the posting clock, not the
    /// publication clock that `Year` shows
    Entered,
    /// manuscript: the cite-state glyph beside the cited string
    CiteIcon,
    Cited,
    State,
}

impl Col {
    /// The stable name a column is stored under, in tabs.json and in
    /// state.json. Kept short and lowercase; never change one without
    /// accepting that everyone's saved sort silently resets.
    pub(super) fn tag(self) -> &'static str {
        match self {
            Col::Metric => "metric",
            Col::Sel => "sel",
            Col::Pdf => "pdf",
            Col::InLib => "inlib",
            Col::Year => "year",
            Col::Author => "author",
            Col::Title => "title",
            Col::Key => "key",
            Col::Entered => "entered",
            Col::CiteIcon => "citeicon",
            Col::Cited => "cited",
            Col::State => "state",
        }
    }

    /// An unrecognized tag falls back to Year rather than failing: a
    /// state file written by a newer build should not break an older one.
    pub(super) fn from_tag(s: &str) -> Self {
        [
            Col::Metric,
            Col::Sel,
            Col::Pdf,
            Col::InLib,
            Col::Year,
            Col::Author,
            Col::Title,
            Col::Key,
            Col::Entered,
            Col::CiteIcon,
            Col::Cited,
            Col::State,
        ]
        .into_iter()
        .find(|c| c.tag() == s)
        .unwrap_or(Col::Year)
    }
}

/// How much horizontal room a column asks for.
#[derive(Clone, Copy)]
pub(super) enum Width {
    Fixed(u16),
    /// Absorbs whatever the fixed columns leave. Exactly one per table.
    Flex,
}

pub(super) struct ColumnSpec {
    pub id: Col,
    pub header: String,
    pub width: Width,
    /// whether clicking the header sorts by this column in this scope —
    /// the library's ● column, for instance, is only meaningful (and
    /// only labelled) when a manuscript is active
    pub sortable: bool,
}

pub(super) fn fixed(id: Col, header: &str, w: u16, sortable: bool) -> ColumnSpec {
    ColumnSpec { id, header: header.to_string(), width: Width::Fixed(w), sortable }
}

pub(super) fn flex(id: Col, header: &str, sortable: bool) -> ColumnSpec {
    ColumnSpec { id, header: header.to_string(), width: Width::Flex, sortable }
}

pub(super) struct TableModel {
    pub columns: Vec<ColumnSpec>,
    pub rows: Vec<Row<'static>>,
    /// The column carrying the ▲/▼ marker, or None where the rows have
    /// an inherent order — a freshly scanned manuscript is in cite order.
    pub sort: Option<(Col, bool)>,
    pub hover: (u16, u16),
}

/// The cursor row's faint cool fill — "standing on a surface". Shared by
/// all three scopes, which is why it lives here.
pub(super) const CURSOR_FILL: Color = Color::Rgb(34, 40, 52);

/// Solve column widths: fixed columns take what they ask for, and the
/// flex column takes the rest.
///
/// The gap count is what the two hand-written formulas disagreed about.
/// ratatui's `Table` puts `column_spacing` *between* columns, so there
/// are n−1 gaps, not n — the query view had this right and the library
/// view did not, which is why the library's `Key` header sat one column
/// left of the cells beneath it.
fn solve(columns: &[ColumnSpec], total: u16) -> Vec<u16> {
    let claimed: u16 = columns
        .iter()
        .map(|c| match c.width {
            Width::Fixed(w) => w,
            Width::Flex => 0,
        })
        .sum();
    let gaps = columns.len().saturating_sub(1) as u16;
    let rest = total.saturating_sub(claimed + gaps);
    columns
        .iter()
        .map(|c| match c.width {
            Width::Fixed(w) => w,
            Width::Flex => rest,
        })
        .collect()
}

/// Draw the header row, the divider rule, and the body. Returns the
/// click rects for the sortable headers and the data area, which the
/// caller needs for its empty-state hint.
pub(super) fn draw(
    f: &mut Frame,
    area: Rect,
    model: TableModel,
    state: &mut TableState,
) -> (Vec<(Rect, Col)>, Rect) {
    let widths = solve(&model.columns, area.width);
    let mut rects: Vec<(Rect, Col)> = vec![];
    let mut spans: Vec<Span> = vec![];
    let mut hx = area.x;
    for (spec, &cw) in model.columns.iter().zip(widths.iter()) {
        let mut label = spec.header.clone();
        let mut style = Style::default().add_modifier(Modifier::BOLD);
        if spec.sortable {
            let r = Rect { x: hx, y: area.y, width: cw.max(1), height: 1 };
            rects.push((r, spec.id));
            if let Some(asc) = model.sort.and_then(|(c, a)| (c == spec.id).then_some(a)) {
                let arrow = if asc { "▲" } else { "▼" };
                // narrow indicator columns fit glyph+arrow only
                label = if cw <= 2 {
                    format!("{label}{arrow}")
                } else {
                    format!("{label} {arrow}")
                };
            }
            if hit(r, model.hover.0, model.hover.1) {
                style = style.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED);
            }
        }
        let pad = (cw as usize).saturating_sub(label.chars().count());
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" ".repeat(pad.min(200) + 1)));
        hx += cw + 1;
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect { x: area.x, y: area.y, width: area.width, height: 1 },
    );
    f.render_widget(
        Paragraph::new(Span::styled("─".repeat(area.width as usize), divider())),
        Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 },
    );
    let data_area = Rect {
        x: area.x,
        y: area.y + 2,
        width: area.width,
        height: area.height.saturating_sub(2),
    };
    let constraints: Vec<Constraint> = model
        .columns
        .iter()
        .zip(widths.iter())
        .map(|(spec, &cw)| match spec.width {
            Width::Fixed(_) => Constraint::Length(cw),
            Width::Flex => Constraint::Min(20),
        })
        .collect();
    let table = Table::new(model.rows, constraints).block(Block::default().borders(Borders::NONE));
    f.render_stateful_widget(table, data_area, state);
    (rects, data_area)
}
