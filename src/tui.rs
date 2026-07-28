//! Ratatui TUI: library table with live filter, pub card, star toggle.
//!
//! Feature parity with the Textual app comes incrementally; the current cut
//! covers browsing — instant startup, live filtering with the full query
//! language, manuscript ● and star ★ indicators, a toggleable pub card,
//! star toggling, and instant quit.

use crate::library::{has_cached_pdf, MergedLibrary};
use crate::query::{self, QueryContext};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, TableState, Wrap};
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
        }
    }

    fn run(mut self, terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
        let t0 = std::time::Instant::now();
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            debug_layout(&format!(
                "{:>6}ms draw frame={:?} table={:?}",
                t0.elapsed().as_millis(),
                terminal.get_frame().area(),
                self.table_area,
            ));
            if event::poll(Duration::from_millis(250))? {
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

    fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollDown => self.move_sel(3),
            MouseEventKind::ScrollUp => self.move_sel(-3),
            MouseEventKind::Down(MouseButton::Left) => self.on_click(m.column, m.row),
            _ => {}
        }
    }

    fn on_click(&mut self, x: u16, y: u16) {
        let a = self.table_area;
        // y == a.y is the header row; data rows start one line below
        if x < a.x || x >= a.x + a.width || y <= a.y || y >= a.y + a.height {
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
        match self.mode {
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
            Mode::Normal => match code {
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Char('/') => self.mode = Mode::Filter,
                KeyCode::Char('s') => self.toggle_star(),
                KeyCode::Char('d') | KeyCode::Char('z') => self.show_detail = !self.show_detail,
                KeyCode::Char(' ') => {
                    // first Space enters selection mode and selects the row
                    self.select_mode = true;
                    if let Some(pos) = self.table.selected() {
                        self.toggle_row_selected(pos);
                    }
                }
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

    fn draw(&mut self, f: &mut Frame) {
        let [main, status] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());
        let (table_area, detail_area) = if self.show_detail {
            let [t, d] =
                Layout::horizontal([Constraint::Min(40), Constraint::Length(48)]).areas(main);
            (t, Some(d))
        } else {
            (main, None)
        };

        self.draw_table(f, table_area);
        if let Some(area) = detail_area {
            self.draw_detail(f, area);
        }
        self.draw_status(f, status);
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

    fn draw_detail(&self, f: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::LEFT);
        let Some(key) = self.selected_key() else {
            f.render_widget(block, area);
            return;
        };
        let e = self.lib.get(key).unwrap();
        let mut lines: Vec<Line> = vec![];
        lines.push(Line::from(Span::styled(
            e.title().trim_matches(['{', '}']).to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled(
                format_authors(e.author()),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("   ·   "),
            Span::styled(e.year(), Style::default().fg(Color::Green)),
        ]));
        let abs = e.abstract_();
        if !abs.is_empty() {
            lines.push(Line::default());
            let shown: String = abs.chars().take(1000).collect();
            lines.push(Line::from(shown));
        }
        let kws = e.keywords();
        if !kws.is_empty() {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                kws.join(" · "),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled(
                if e.short_key.is_empty() { e.key() } else { &e.short_key }.to_string(),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("  ({})", e.key()),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        let p = Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .block(block.padding(ratatui::widgets::Padding::horizontal(1)));
        f.render_widget(p, area);
    }

    fn draw_status(&self, f: &mut Frame, area: Rect) {
        let line = match self.mode {
            Mode::Filter => Line::from(vec![
                Span::styled("/", Style::default().fg(Color::Cyan)),
                Span::raw(self.filter.clone()),
                Span::styled("▏", Style::default().fg(Color::Cyan)),
            ]),
            Mode::Normal if self.select_mode => Line::from(vec![
                Span::styled(
                    format!("◉ {} selected", self.selected.len()),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    "  ·  Space/click ◯ toggle · s star · Esc done".to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            Mode::Normal => {
                let n = self.filtered.len();
                let total = self.order.len();
                let filt = if self.filter.is_empty() {
                    String::new()
                } else {
                    format!("  ·  /{}", self.filter)
                };
                Line::from(Span::styled(
                    format!(
                        "{n}/{total}  ·  {}{filt}  ·  / filter · Space select · s star · d card · q quit",
                        self.status
                    ),
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
