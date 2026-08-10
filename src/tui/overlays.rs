//! Surfaces drawn over the table rather than in it.

use super::*;

impl App {
    /// Centered confirm modal for Delete: lists the targets, states in
    /// plain words what confirming will do to them (the decided plan,
    /// which is also what executes), offers clickable remove/cancel
    /// (⏎/y confirms, Esc/n cancels).
    pub(super) fn draw_confirm(&mut self, f: &mut Frame) {
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
        let remove_rect = Rect { x: bx, y: by, width: rw, height: 1 };
        let cancel_rect = Rect { x: bx + rw + 2, y: by, width: cw, height: 1 };
        self.hits.add(remove_rect, Target::Confirm(true));
        self.hits.add(cancel_rect, Target::Confirm(false));
        let hov_remove = hit(remove_rect, self.hover.0, self.hover.1);
        let hov_cancel = hit(cancel_rect, self.hover.0, self.hover.1);
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

    /// ⟳ — ask PyPI for the newest astrobib version, on a worker
    /// thread; the result lands in the about modal and the log.
    pub(super) fn check_updates(&mut self) {
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

    pub(super) fn drain_update(&mut self) {
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
    pub(super) fn draw_about(&mut self, f: &mut Frame) {
        let dim = Style::default().fg(Color::DarkGray);
        let cyan = Style::default().fg(Color::Cyan);
        // labels are the full URLs so terminals that linkify text (e.g.
        // Warp) pick them up too; our own click handler opens them as well
        let links = [
            "https://jzrake.people.clemson.edu",
            "https://pypi.org/project/astrobib",
        ];
        let frame = f.area();
        let token_src = if std::env::var("ADS_API_TOKEN").is_ok_and(|t| !t.is_empty()) {
            "from $ADS_API_TOKEN".to_string()
        } else if crate::ads::get_token().is_some() {
            "from state.json".to_string()
        } else {
            "none — press S to set one".to_string()
        };
        // one row per endpoint called, or a single row saying none was
        let quotas = crate::ads::quotas();
        let quota_rows: Vec<String> = if quotas.is_empty() {
            vec!["no calls recorded yet".to_string()]
        } else {
            quotas
                .iter()
                .map(|(endpoint, q)| {
                    let used = if q.stale() { 0 } else { q.used() };
                    match q.resets_in() {
                        Some(span) => format!("{endpoint} {used}/{} · resets in {span}", q.limit),
                        None => format!("{endpoint} {used}/{}", q.limit),
                    }
                })
                .collect()
        };
        // emoji-set glyphs (⟳) can render double-width on some
        // terminals, shifting rows right — several columns of slack
        // beyond the longest line keep everything inside the borders
        let w = 58.min(frame.width.saturating_sub(4));
        // Which databases this session is actually pointed at. Nothing
        // else on screen says: the global tier's path is settable with
        // --library and appears nowhere, and the local tier could only
        // be inferred from the Manuscript capsule existing — which says
        // that there is one, never which one.
        let tiers: Vec<(&str, String)> = vec![
            (
                " Global     ",
                match (&self.lib.manuscript, self.lib.global_on) {
                    // the switch reads as off only when there is a local
                    // tier for reads to fall back to; alone, it is moot
                    (Some(_), false) => {
                        format!("{} · hidden (t)", elide_left(&crate::library::contract_home(&self.lib.personal.root), 30))
                    }
                    _ => elide_left(&crate::library::contract_home(&self.lib.personal.root), 44),
                },
            ),
            (
                " Local      ",
                match &self.lib.manuscript {
                    Some(m) => elide_left(&crate::library::contract_home(&m.root), 44),
                    None => "none — the global library only".to_string(),
                },
            ),
        ];
        let h = (22 + quota_rows.len() as u16 + u16::from(self.update_status.is_some()))
            .min(frame.height);
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
            Line::from(vec![
                Span::styled(" ADS token  ", dim),
                Span::raw(token_src),
            ]),
        ];
        // What the day has cost the token. ADS meters each endpoint on
        // its own allowance, so they get a line each and are never
        // added up; the label sits against the first of them.
        for (i, line) in quota_rows.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(if i == 0 { " ADS use    " } else { "            " }, dim),
                Span::raw(line.clone()),
            ]));
        }
        lines.push(Line::default());
        for (label, value) in &tiers {
            lines.push(Line::from(vec![
                Span::styled(*label, dim),
                Span::raw(value.clone()),
            ]));
        }
        lines.push(Line::default());
        let link_row = |url: &str, lines: &mut Vec<Line>, hits: &mut Hits| {
            let y = area.y + 1 + lines.len() as u16;
            let r = Rect {
                x: area.x + 5, // border + the " →  " prefix
                y,
                width: url.chars().count() as u16 + 1,
                height: 1,
            };
            hits.add(r, Target::AboutLink(url.to_string()));
            let hov = hit(r, self.hover.0, self.hover.1);
            let style = if hov { cyan.add_modifier(Modifier::UNDERLINED) } else { cyan };
            lines.push(Line::from(vec![
                Span::styled(" →  ", dim),
                Span::styled(url.to_string(), style),
            ]));
        };
        for url in links {
            link_row(url, &mut lines, &mut self.hits);
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(" report a bug / request a feature:", dim)));
        link_row(
            "https://github.com/clemson-cal/astrobib/issues",
            &mut lines,
            &mut self.hits,
        );
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
            self.hits.add(r, Target::AboutUpdate);
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
}
