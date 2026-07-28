//! Ratatui TUI: library table with live filter, pub card, star toggle.
//!
//! Feature parity with the Textual app comes incrementally; the current cut
//! covers browsing — instant startup, live filtering with the full query
//! language, manuscript ● and star ★ indicators, a toggleable pub card,
//! star toggling, and instant quit.

use crate::library::{has_cached_pdf, MergedLibrary};
use crate::query::{self, QueryContext};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Frame;
use std::time::Duration;

pub fn run(lib: MergedLibrary) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = App::new(lib).run(&mut terminal);
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
        }
    }

    fn run(mut self, terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.on_key(key.code, key.modifiers);
                    }
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

    fn toggle_star(&mut self) {
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
                KeyCode::Esc => {
                    if !self.filter.is_empty() {
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
        let rows: Vec<Row> = self
            .filtered
            .iter()
            .map(|&i| {
                let e = self.lib.get(&self.order[i]).unwrap();
                Row::new(vec![
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
                Constraint::Length(6),
                Constraint::Length(18),
                Constraint::Min(20),
                Constraint::Length(20),
            ],
        )
        .header(
            Row::new(vec!["↓", "●", "★", "Year", "Author", "Title", "Key"])
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
                        "{n}/{total}  ·  {}{filt}  ·  / filter · s star · d card · q quit",
                        self.status
                    ),
                    Style::default().fg(Color::DarkGray),
                ))
            }
        };
        f.render_widget(line, area);
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
