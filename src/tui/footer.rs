//! The bottom line: status, badges, the query's home, the prompt.

use super::*;

/// Where the query you are on is visible, as one label rather than two
/// sides. A segmented control cannot say whether the lit side is the
/// state or the button, and since a move changed only the colour, the
/// line read as though nothing had happened. One label that changes its
/// words says the state and shows the move in the same stroke.
///
/// "everywhere" and "this paper" rather than global and local: the
/// footer already says "global" for the library tier a few cells right,
/// and these name what the user actually wants to know — where the
/// query will turn up. The ⌕ carries the query sense that a spelled-out
/// "query" was spending five cells on.
pub(super) const HOME_GLOBAL: &str = "⌕ everywhere";

pub(super) const HOME_LOCAL: &str = "⌕ this paper";

/// Both labels are this wide, which is what lets the indicator hold its
/// place when it changes.
pub(super) const HOME_W: u16 = 12;

impl App {
    /// Whether the prompt being composed is still blank. A sample loads
    /// only into an empty prompt: it replaces the whole query, and doing
    /// that to something half-typed would destroy work on a stray click.
    pub(super) fn prompt_is_empty(&self) -> bool {
        match &self.mode {
            Mode::AdsPrompt { input, .. } => input.value().trim().is_empty(),
            Mode::Filter => self.filter.value().trim().is_empty(),
            _ => false,
        }
    }

    /// Where the footer's view badges sit, and what each one is:
    /// `(rect, action, label, currently on)`, right-aligned — inside
    /// whatever the card ⇄ bib toggler has claimed of the right edge.
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
        // the card ⇄ bib toggler is rightmost; the badges stop short of it
        let reserve = if self.card_toggle.is_some() { TOGGLE_RESERVE } else { 0 };
        let mut bx = (area.x + area.width).saturating_sub(total + reserve);
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

    /// Where the query-home indicator sits, or nothing at all when there
    /// is no query to move, no second home to move it to, or no room
    /// left of the badges to say so in.
    fn home_rect(&self, area: Rect) -> Option<Rect> {
        if !self.available(Action::QueryHome) {
            return None;
        }
        let badges_x = self
            .badge_layout(area)
            .first()
            .map(|(r, ..)| r.x)
            .unwrap_or(area.x + area.width);
        // two cells of separation from the badges, and it goes entirely
        // rather than half-drawn when the width is not there
        if badges_x.saturating_sub(area.x) < HOME_W + 2 {
            return None;
        }
        Some(Rect { x: badges_x - HOME_W - 2, y: area.y, width: HOME_W, height: 1 })
    }

    /// What the indicator takes out of the room left of the badges, so
    /// the footer's own line knows where to stop.
    fn home_reserve(&self, area: Rect) -> u16 {
        if self.home_rect(area).is_some() {
            HOME_W + 2
        } else {
            0
        }
    }

    /// What the hovered indicator says: where the query would go, not
    /// where it is — the label already says where it is, and a hint that
    /// repeated it would leave the click meaning nothing in particular.
    fn home_hint(&self, area: Rect) -> Option<String> {
        let now = self.active_query_home()?;
        let r = self.home_rect(area)?;
        if !hit(r, self.hover.0, self.hover.1) {
            return None;
        }
        Some(match now {
            crate::tabs::Home::Global => "⌕ keep this query with the manuscript  ·  H".to_string(),
            crate::tabs::Home::Local => "⌕ keep this query everywhere  ·  H".to_string(),
        })
    }

    /// The query-home indicator, left of the view badges: where the
    /// active query is visible, and clicking it moves the query to the
    /// other home. It sits in the footer because that is where the rest
    /// of the chrome describing the current view already is.
    fn draw_query_home(&mut self, f: &mut Frame, area: Rect) {
        let Some(now) = self.active_query_home() else { return };
        let Some(r) = self.home_rect(area) else { return };
        self.footer_badges.push((r, Action::QueryHome));
        let style = if hit(r, self.hover.0, self.hover.1) {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED)
        } else {
            Style::default().fg(Color::Cyan)
        };
        let label = match now {
            crate::tabs::Home::Global => HOME_GLOBAL,
            crate::tabs::Home::Local => HOME_LOCAL,
        };
        f.render_widget(Paragraph::new(Line::from(Span::styled(label, style))), r);
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
        // where the cluster starts, which is not the right edge once the
        // card ⇄ bib toggler has claimed it
        let Some(x0) = layout.first().map(|(r, ..)| r.x) else { return };
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
        let badge_area = Rect {
            x: x0,
            y: area.y,
            width: total.min((area.x + area.width).saturating_sub(x0)),
            height: 1,
        };
        f.render_widget(Paragraph::new(Line::from(spans)), badge_area);
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
    pub(super) fn prompt_height(&self, width: u16) -> u16 {
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
        let last = Rect { x: area.x, y: last_y, width: area.width, height: 1 };
        self.draw_badges(f, last);
        // after the badges, whose layout it measures against, and which
        // clear the hit-rect list it pushes into
        self.draw_query_home(f, last);
        self.draw_card_toggle(f, last);
        true
    }

    pub(super) fn draw_status(&mut self, f: &mut Frame, area: Rect) {
        // no rule above: the footer's own tint separates it from the
        // table, the way every other surface here is separated
        f.render_widget(Block::default().style(Style::default().bg(footer_bg())), area);
        if self.draw_wrapped_prompt(f, area) {
            return;
        }
        let area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
        // the badges and the card ⇄ bib toggler live on this same line and
        // are drawn after it, so their hover hints have to be settled
        // before the line is built
        if let Some(hint) = self
            .badge_hint(area)
            .or_else(|| self.toggle_hint(area))
            .or_else(|| self.home_hint(area))
        {
            self.hover_hint = Some(hint);
        }
        // room left of the badges, which are drawn after this line and
        // over it: whatever goes here has to fit in front of them — and
        // in front of the query-home control, which sits between
        let free = self
            .badge_layout(area)
            .first()
            .map(|(r, ..)| r.x.saturating_sub(area.x).saturating_sub(2))
            .unwrap_or(area.width)
            .saturating_sub(self.home_reserve(area));
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
            Mode::Tag { ref input, ref keys, remove } => {
                // ASCII +/-, paired: the true minus sign U+2212 would
                // be one more glyph for the width audit to carry, for a
                // character that sits beside an ASCII plus
                let label = format!("tag {} {} ", keys.len(), if remove { "-" } else { "+" });
                let prefix = label.chars().count() as u16;
                // the tail names what pressing ⏎ will do, since ± is
                // decided from the name and can flip mid-word; when the
                // name is still empty it offers the tags that exist,
                // which is what stops one being mistyped into being
                let tail = if input.value().trim().is_empty() {
                    let known = self.known_tags();
                    if known.is_empty() {
                        "   ⏎ create · Esc cancel".to_string()
                    } else {
                        format!("   {}", known.join(" · "))
                    }
                } else if remove {
                    "   ⏎ untag · Esc cancel".to_string()
                } else {
                    "   ⏎ tag · Esc cancel".to_string()
                };
                let avail = area.width.saturating_sub(prefix + tail.chars().count() as u16) as usize;
                let scroll = input.visual_scroll(avail.max(10));
                let shown: String = input.value().chars().skip(scroll).collect();
                f.set_cursor_position((
                    area.x + prefix + (input.visual_cursor().saturating_sub(scroll)) as u16,
                    area.y,
                ));
                Line::from(vec![
                    Span::styled(
                        label,
                        Style::default().fg(if remove { Color::Yellow } else { Color::Cyan }),
                    ),
                    Span::raw(shown),
                    Span::styled(tail, Style::default().fg(Color::DarkGray)),
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
        self.draw_query_home(f, area);
        self.draw_card_toggle(f, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `HOME_W` gates whether the indicator draws at all — too small and
    /// it would overlap the badges, too large and it would vanish at
    /// widths that had room. Both labels must also be the same width, or
    /// the indicator would shift sideways when the query moves and the
    /// eye would read the jump rather than the word.
    #[test]
    fn both_home_labels_are_the_declared_width() {
        for label in [HOME_GLOBAL, HOME_LOCAL] {
            assert_eq!(label.chars().count() as u16, HOME_W, "{label}");
        }
    }
}
