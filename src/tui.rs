//! Minimal ratatui TUI: library table with live filter.
//!
//! Feature parity with the Textual app comes incrementally; this first cut
//! demonstrates the core interaction — instant startup, live filtering with
//! the full query language, instant quit.

use crate::library::{has_cached_pdf, Library};
use crate::query::{self, QueryContext};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Row, Table, TableState};
use ratatui::Frame;
use std::time::Duration;

pub fn run(lib: Library) -> anyhow::Result<()> {
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
    lib: Library,
    order: Vec<usize>,    // entry indices, year-descending
    filtered: Vec<usize>, // positions into `order` that pass the filter
    filter: String,
    mode: Mode,
    table: TableState,
    quit: bool,
}

impl App {
    fn new(lib: Library) -> Self {
        let mut order: Vec<usize> = (0..lib.entries().len()).collect();
        order.sort_by(|&a, &b| {
            let (ea, eb) = (&lib.entries()[a], &lib.entries()[b]);
            eb.year().cmp(&ea.year()).then(ea.key().cmp(eb.key()))
        });
        let filtered = order.clone();
        let mut table = TableState::default();
        if !filtered.is_empty() {
            table.select(Some(0));
        }
        App {
            lib,
            order,
            filtered,
            filter: String::new(),
            mode: Mode::Normal,
            table,
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

    fn refilter(&mut self) {
        let groups = query::tokenize(&self.filter);
        let ctx = QueryContext {
            in_manuscript: None,
            has_pdf: Some(Box::new(|k: &str| has_cached_pdf(k))),
        };
        self.filtered = self
            .order
            .iter()
            .copied()
            .filter(|&i| query::matches(&groups, &self.lib.entries()[i], &ctx))
            .collect();
        let sel = self.table.selected().unwrap_or(0);
        self.table
            .select(if self.filtered.is_empty() {
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
                KeyCode::Esc => {
                    if !self.filter.is_empty() {
                        self.filter.clear();
                        self.refilter();
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
                KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
                KeyCode::Char('g') | KeyCode::Home => self.table.select(
                    (!self.filtered.is_empty()).then_some(0),
                ),
                KeyCode::Char('G') | KeyCode::End => self.table.select(
                    self.filtered.len().checked_sub(1),
                ),
                KeyCode::PageDown => self.move_sel(20),
                KeyCode::PageUp => self.move_sel(-20),
                _ => {}
            },
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let [main, status] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(f.area());

        let rows: Vec<Row> = self
            .filtered
            .iter()
            .map(|&i| {
                let e = &self.lib.entries()[i];
                let pdf = if has_cached_pdf(e.key()) { "↓" } else { "" };
                let star = if e.starred() { "★" } else { "" };
                Row::new(vec![
                    pdf.to_string(),
                    star.to_string(),
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
                Constraint::Length(6),
                Constraint::Length(18),
                Constraint::Min(20),
                Constraint::Length(20),
            ],
        )
        .header(
            Row::new(vec!["↓", "★", "Year", "Author", "Title", "Key"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::NONE));
        f.render_stateful_widget(table, main, &mut self.table);

        let status_line = match self.mode {
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
                    format!("{n}/{total} papers{filt}  ·  / filter · q quit"),
                    Style::default().fg(Color::DarkGray),
                ))
            }
        };
        f.render_widget(status_line, status);
    }
}
