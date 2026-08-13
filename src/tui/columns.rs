//! The table panel: which columns show, how wide, sorted by what.

use super::*;

/// Which side of the screen the arrow keys drive.
///
/// The columns panel is a toggle view, not a modal — the table stays
/// visible and live beside it. But it needs ↑↓ to walk its list and ←→
/// to size a column, which are the table's own navigation keys, so
/// something has to say who gets them. Opening the panel moves focus
/// into it; Esc hands focus back without closing it, and clicking either
/// side moves focus there.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Table,
    Columns,
}

/// One line of the columns panel, and what clicking a part of it does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PanelHit {
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
pub(super) enum PanelRow {
    Column(Col),
}

/// One line of the columns sidebar, built before it is placed so the
/// list can be windowed against the pane's height.
struct PanelLine {
    spans: Vec<Span<'static>>,
    /// click targets, as (x offset from the panel's left edge, width, what)
    hits: Vec<(u16, u16, PanelHit)>,
    /// the list row this line is, when it is selectable
    pub(super) row: Option<usize>,
    /// the arrow keys are on this line
    fill: bool,
}

impl PanelLine {
    /// The panel's own name, sitting where the pub card's title sits.
    /// It carries the focus colour, since the panel has no box border
    /// left to tint.
    pub(super) fn title(text: &str, focused: bool) -> Self {
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

/// The table sidebar's width: enough for a label, a width readout with
/// its ‹ › nudges, and the sort marker, inside the same 2-cell padding
/// the pub card keeps on either side of its rule.
pub(super) const COLUMNS_PANEL_W: u16 = 28;

/// The manuscript scope's spine: the columns it always draws before the
/// title — gutter, cite glyph, ↓, Cited, State.
const MS_SPINE_W: u16 = 2 + 2 + 1 + 26 + 10;
/// The title width the manuscript's optional columns have to leave
/// standing before they default themselves on — the same comfort width
/// the library protects when it decides whether to keep Key.
const MS_TITLE_COMFORT: u16 = 32;

/// A column's width as actually laid out, 0 if it is not drawn.
///
/// This has to be the *solved* width, not the declared one: the flex
/// column's size is only known once the fixed ones have taken their
/// share, and the flex role moves when a column is hidden. Reading the
/// declaration instead gave the author cells a width of 0 — every one of
/// them rendered as a bare "…" — the moment Author became the flex
/// column.
pub(super) fn col_width(cols: &[table::ColumnSpec], total: u16, id: Col) -> u16 {
    let solved = table::solve(cols, total);
    cols.iter()
        .position(|c| c.id == id)
        .and_then(|i| solved.get(i).copied())
        .unwrap_or(0)
}

impl App {
    /// Every column a scope can draw, in order, shown or not — the list
    /// the columns panel offers. Widths here are the responsive
    /// defaults and so is visibility; `columns_for` applies the user's
    /// overrides on top.
    fn all_columns(&self, kind: ScopeKind, width: u16) -> Vec<table::ColumnSpec> {
        match kind {
            ScopeKind::Manuscript => {
                // The manuscript's rows are cites, but every cite that
                // resolves names a paper — so the paper columns the
                // library offers are offered here too, in the same
                // order, with the cite's own columns interleaved where
                // they belong. Only ● is left out: what it would say
                // here (in the manuscript) is what the cite glyph
                // beside it already says, in more detail.
                let (author_w, _) = column_layout(width);
                // Cited and State already spend what the library gives
                // Author and Key, so the two widest paper columns have
                // to earn their place: Year appears once the title can
                // keep its comfortable width beside it, Author only once
                // it can do so beside both. (The gap counts are the
                // columns each case draws, minus one.)
                let year_fits = MS_SPINE_W + 6 + MS_TITLE_COMFORT + 6 <= width;
                let author_fits =
                    MS_SPINE_W + 6 + author_w + MS_TITLE_COMFORT + 7 <= width;
                vec![
                    table::fixed(Col::Sel, "", 2, false),
                    metric_column(self.metric_col),
                    // the glyph column has no label, so nothing to click; the
                    // State column beside it sorts by the same fact
                    table::fixed(Col::CiteIcon, "", 2, false),
                    table::fixed(Col::Pdf, "↓", 1, true).fixed_size(),
                    table::fixed(Col::Cited, "Cited", 26, true),
                    table::fixed(Col::State, "State", 10, true),
                    table::fixed(Col::Year, "Year", 6, true).default_when(year_fits),
                    table::fixed(Col::Author, "Author", author_w, true)
                        .default_when(author_fits),
                    table::flex(Col::Title, "Title", true),
                    // off by default alone among the paper columns: the
                    // Cited column already names the paper, and where a
                    // cite resolves the two mostly read the same
                    table::fixed(Col::Key, "Key", 20, true).default_off(),
                ]
            }
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
        let all = self.all_columns(kind, self.last_table_area.width);
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
            ScopeKind::Manuscript => &[Col::Title, Col::Cited, Col::Author, Col::State],
            _ => &[Col::Title, Col::Author, Col::Key],
        }
    }

    /// The columns the table actually draws: `all_columns` with the
    /// user's overrides applied, the metric strip removed (it is drawn
    /// beside the table, not inside it), and the flex role reassigned if
    /// whichever column had it is now hidden.
    pub(super) fn columns_for(&self, kind: ScopeKind, width: u16) -> Vec<table::ColumnSpec> {
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

    pub(super) fn load_column_config() -> std::collections::HashMap<ScopeKind, table::ColumnConfig> {
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
    pub(super) fn columns_panel_key(&mut self, code: KeyCode) -> bool {
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
    pub(super) fn toggle_column(&mut self, id: Col) {
        let kind = self.active_kind();
        let now_shown = self.column_shown(kind, id);
        let default = self
            .all_columns(kind, self.last_table_area.width)
            .iter()
            .find(|c| c.id == id)
            .is_some_and(|c| c.default_visible);
        let shown_after = !now_shown;
        let cfg = self.columns.entry(kind).or_default();
        // storing an override that agrees with the default would pin a
        // responsive rule in place, so put the column back under it
        if shown_after == default {
            cfg.visible.remove(&id);
        } else {
            cfg.visible.insert(id, shown_after);
        }
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
    pub(super) fn nudge_width(&mut self, d: i16) {
        let kind = self.active_kind();
        let rows = self.panel_rows();
        let Some(&PanelRow::Column(id)) = rows.get(self.col_sel) else {
            return;
        };
        let shown = self.columns_for(kind, self.last_table_area.width);
        // hidden, or drawn outside the table (the metric swatch), or
        // locked to a derived or one-cell width
        if !shown.iter().any(|c| c.id == id && c.resizable) {
            return;
        }
        let cur = col_width(&shown, self.last_table_area.width, id) as i16;
        let next = (cur + d).clamp(table::MIN_COL_W as i16, table::MAX_COL_W as i16) as u16;
        self.columns.entry(kind).or_default().widths.insert(id, next);
        self.save_column_config();
        self.note_latest(
            MsgCat::Info,
            "width",
            format!("{} column {next} wide", self.column_hint(id)),
        );
    }

    /// What a column header means, in words, for the footer rollover.
    /// The ● column is the clearest case for having this at all: it says
    /// something different on a query page than in the library.
    pub(super) fn column_hint(&self, col: Col) -> &'static str {
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

    /// What the columns panel lists for the active scope: every column
    /// it can draw — shown or not, which is the point, since sorting by
    /// a hidden column is allowed — followed, in a query scope, by the
    /// ADS sort that decides which records come back at all.
    ///
    /// Structural columns are left out: the selection gutter and the
    /// manuscript's state glyph have no label to click and nothing worth
    /// configuring.
    pub(super) fn panel_rows(&self) -> Vec<PanelRow> {
        let kind = self.active_kind();
        self.all_columns(kind, self.last_table_area.width)
            .into_iter()
            .filter(|c| !c.header.is_empty() || c.sortable)
            .map(|c| PanelRow::Column(c.id))
            .collect()
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
        let shown = self.columns_for(kind, self.last_table_area.width);
        let all = self.all_columns(kind, self.last_table_area.width);
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
                    let w = col_width(&shown, self.last_table_area.width, id);
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
    pub(super) fn draw_columns_panel(&mut self, f: &mut Frame, area: Rect) {
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
            if let Some(i) = line.row {
                self.hits.add(
                    Rect { x: inner.x, y, width: inner.width, height: 1 },
                    Target::Column(PanelHit::Row(i)),
                );
            }
            for &(dx, w, hitkind) in &line.hits {
                self.hits.add(
                    Rect { x: inner.x + dx, y, width: w, height: 1 },
                    Target::Column(hitkind),
                );
            }
            rendered.push(line.to_line());
        }
        // rolling over a control says what it does and which key does it
        if let Some(h) = lines
            .iter()
            .skip(start)
            .take(h)
            .enumerate()
            .flat_map(|(n, line)| line.hits.iter().map(move |&(dx, w, hitkind)| (n, dx, w, hitkind)))
            .find(|(n, dx, w, _)| hit(Rect { x: inner.x + *dx, y: inner.y + *n as u16, width: *w, height: 1 }, self.hover.0, self.hover.1))
            .map(|(_, _, _, hitkind)| hitkind)
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
}
