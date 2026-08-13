//! The pub card's chrome. `card.rs` draws its body.

use super::*;

/// One row of the card's link stack: → rows open the browser, ⌕ rows
/// act inside astrobib (query scopes).
pub(super) enum LinkTarget {
    Url(String),
    Query(CardBtn),
    Copy(CopyItem),
}

/// Pub card buttons; they act on the card's (highlighted) entry.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CardBtn {
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

/// Footer hint for a card affordance: what happens, and the key.
pub(super) fn card_hint(btn: CardBtn) -> &'static str {
    match btn {
        CardBtn::Arxiv => "↓ fetch the PDF from arXiv  ·  p",
        CardBtn::Oa => "↓ fetch the open-access PDF via ADS  ·  p",
        CardBtn::Browser => "↓ download via the browser, watching ~/Downloads  ·  B",
        CardBtn::Pick => "⤷ import a PDF from the filesystem",
        CardBtn::Open => "→ open the cached PDF  ·  o",
        CardBtn::Clear => "✕ remove the cached PDF  ·  X",
        CardBtn::Cancel => "✕ stop watching for the download",
        CardBtn::MsToggle => "◆ add to / remove from the manuscript db  ·  m",
        // which library that is depends on where the session stands, so
        // the hint names the gesture and lets the note that follows name
        // the tier: i is local-first, I imports and shares in one press
        CardBtn::Import => "→ import  ·  i    (I imports and shares to the global library)",
        CardBtn::BibView => "@ show the .bib entry verbatim  ·  v",
        CardBtn::RefreshCites => "⟳ refresh the citation count from ADS",
        CardBtn::RemoveFromLib => "✕ remove from the library  ·  ⌫",
        CardBtn::Citations => "⌕ new query: papers citing this one  ·  C",
        CardBtn::Refs => "⌕ new query: papers this one cites  ·  R",
    }
}

/// Render the card's action block as two columns — links and query
/// actions on the left, the permanent ⧉ copy menu on the right — split
/// by a dim vertical divider (omitted when either column is empty).
/// Badges name each row's kind: → opens the browser, ⌕ acts inside
/// astrobib, ⧉ copies. Registers whole-row click rects and returns the
/// y below the block.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_link_stack(
    f: &mut Frame,
    x0: u16,
    y: u16,
    w: u16,
    bottom: u16,
    hover: (u16, u16),
    left: Vec<(String, LinkTarget, bool)>,
    right: Vec<(String, LinkTarget, bool)>,
    hits: &mut Hits,
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
                    LinkTarget::Url(url) => hits.add(r, Target::CardLink(url)),
                    LinkTarget::Query(btn) => hits.add(r, Target::CardButton(btn)),
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

/// The clickable "cited by N" card line: tapping refreshes the count
/// from ADS.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_cited_line(
    f: &mut Frame,
    x0: u16,
    y: u16,
    w: u16,
    n: Option<i64>,
    hover: (u16, u16),
    hits: &mut Hits,
    hint: &mut Option<String>,
) {
    let label = match n {
        Some(n) => format!("cited by {n}"),
        None => "cited by ?".to_string(),
    };
    let r = Rect { x: x0, y, width: label.chars().count() as u16 + 2, height: 1 };
    hits.add(r, Target::CardButton(CardBtn::RefreshCites));
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

/// The two sides of the card ⇄ bib-source toggler, as
/// `(label, is the bib side)`, left to right.
pub(super) const TOGGLE_SEGS: [(&str, bool); 2] = [("▤ card", false), ("@ bib", true)];

/// The toggler's drawn width: both labels and the " │ " between them.
pub(super) const TOGGLE_W: u16 = 6 + 3 + 5;

/// What the toggler takes out of the footer's right edge, so the view
/// badges know where to stop: its width, its cell of air at the edge,
/// and two cells of separation from the badges.
pub(super) const TOGGLE_RESERVE: u16 = TOGGLE_W + 3;

impl App {
    /// The entry the pub card shows: hovering a scope-specific trigger
    /// column previews that row in the card (full-row hover proved too
    /// twitchy) — the Key column in the library, the Cited column in the
    /// manuscript scope; otherwise the cursor row.
    pub(super) fn card_key(&self) -> Option<&str> {
        if self.active_scope == 0 {
            let a = self.last_table_area;
            let (_, show_key) = column_layout(a.width);
            let show_key = show_key || self.show_detail;
            let in_key_col = show_key && self.hover.0 >= a.x + a.width.saturating_sub(20);
            if in_key_col {
                if let Some(pos) = self.hovered_table_pos() {
                    return self
                        .row_index(pos)
                        .and_then(|i| self.order.get(i))
                        .map(String::as_str);
                }
            }
        }
        if matches!(self.scopes.get(self.active_scope), Some(Scope::Manuscript { .. })) {
            // Cited column: after the 2-wide gutter and state columns
            // (spacing 1), x spans [6, 6+26)
            let a = self.last_table_area;
            if self.hover.0 >= a.x + 6 && self.hover.0 < a.x + 6 + 26 {
                if let Some(k) = self
                    .hovered_table_pos()
                    .and_then(|pos| self.ms_row_at(pos))
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
    pub(super) fn card_article_pos(&self) -> Option<usize> {
        let a = self.last_table_area;
        // Key column (rightmost 20) — the same trigger as the library scope
        if self.hover.0 >= a.x + a.width.saturating_sub(20) {
            if let Some(pos) = self.hovered_table_pos() {
                return Some(pos);
            }
        }
        self.table.selected()
    }

    /// The citation count the card is showing, when it is known. None
    /// means unknown, which must never be confused with a known zero.
    pub(super) fn card_citation_count(&self) -> Option<i64> {
        if let Some(a) = self.card_article_pos().and_then(|p| self.article_at(p)) {
            return a.citation_count;
        }
        let key = self.card_entry_key()?;
        self.metrics.get(&key).and_then(|m| m.citations)
    }

    pub(super) fn card_entry_key(&self) -> Option<String> {
        if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
            let a = self.card_article_pos().and_then(|p| self.article_at(p))?;
            return self.article_entry(a).map(|e| e.key().to_string());
        }
        self.selected_key().map(str::to_string)
    }

    /// The bibcode the card's citation-graph affordances act on: the
    /// shown ADS article's, else the shown library entry's (derived
    /// from its adsurl).
    pub(super) fn card_bibcode(&self) -> Option<String> {
        if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
            return self
                .card_article_pos()
                .and_then(|p| self.article_at(p))
                .map(|a| a.bibcode.clone());
        }
        let key = self.card_key()?;
        self.lib.get(key).and_then(|e| e.bibcode()).map(str::to_string)
    }

    /// Fetch (once) the canonical BibTeX an import of this article
    /// would write — bib-source preview for un-imported query results.
    pub(super) fn request_bib_preview(&mut self, bibcode: String) {
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

    pub(super) fn drain_bib_preview(&mut self) {
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

    /// Where the card ⇄ bib-source toggler's segments sit in the footer:
    /// `(rect, label, is the bib side)`, or nothing while no pub card is
    /// on screen. It is the rightmost thing in the footer, so the view
    /// badges start `TOGGLE_RESERVE` cells further left.
    ///
    /// Split out from drawing for the same reason `badge_layout` is: the
    /// hover hint has to be settled before the footer line is built.
    fn toggle_layout(&self, area: Rect) -> Vec<(Rect, &'static str, bool)> {
        if self.card_toggle.is_none() {
            return vec![];
        }
        // one cell of air at the right edge, the way the badges leave one
        let mut x = (area.x + area.width).saturating_sub(TOGGLE_W + 1);
        TOGGLE_SEGS
            .iter()
            .map(|&(label, is_bib)| {
                let lw = label.chars().count() as u16;
                let r = Rect { x, y: area.y, width: lw, height: 1 };
                x += lw + 3; // the " │ " divider between the segments
                (r, label, is_bib)
            })
            .collect()
    }

    /// What a hovered toggler segment says in the footer. Only the
    /// inactive side is clickable, so only it hints.
    pub(super) fn toggle_hint(&self, area: Rect) -> Option<String> {
        let source = self.card_toggle?;
        let (.., is_bib) = self
            .toggle_layout(area)
            .into_iter()
            .find(|(r, ..)| hit(*r, self.hover.0, self.hover.1))?;
        if is_bib == source {
            return None;
        }
        Some(if is_bib {
            card_hint(CardBtn::BibView).to_string()
        } else {
            "▤ back to the formatted card  ·  v".to_string()
        })
    }

    /// The card ⇄ bib-source toggler, pinned to the footer's right edge:
    /// a segmented "▤ card │ @ bib" control — the active side cyan, the
    /// inactive side dimmed and clickable (v toggles). It lives in the
    /// footer rather than in the card because it selects which view the
    /// card shows, and every other view control is already there.
    pub(super) fn draw_card_toggle(&mut self, f: &mut Frame, area: Rect) {
        let Some(source) = self.card_toggle else { return };
        for (i, (r, label, is_bib)) in self.toggle_layout(area).into_iter().enumerate() {
            if i > 0 {
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(" │ ", divider()))),
                    Rect { x: r.x.saturating_sub(3), y: r.y, width: 3, height: 1 },
                );
            }
            let style = if is_bib == source {
                Style::default().fg(Color::Cyan)
            } else {
                self.hits.add(r, Target::CardButton(CardBtn::BibView));
                if hit(r, self.hover.0, self.hover.1) {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::default().fg(Color::DarkGray)
                }
            };
            f.render_widget(Paragraph::new(Line::from(Span::styled(label, style))), r);
        }
    }

    /// The verbatim .bib file in place of the formatted card (v / @ bib),
    /// with the permanent ⧉ copy menu pinned above the bottom.
    pub(super) fn draw_bib_source(&mut self, f: &mut Frame, area: Rect, key: &str) {
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
    /// soft-wrapped body, the ⧉ copy stack.
    pub(super) fn draw_bib_panel(
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
        // the copy stack sits above the pane's blank last row; text stops there
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
            &mut self.hits,
            &mut self.hover_hint,
            &mut yanks,
        );
        for (r, item) in yanks {
            self.hits.add(r, Target::CardYank(item));
        }
        // the footer draws the card ⇄ bib toggler for us, on the bib side
        self.card_toggle = Some(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TOGGLE_W` has to be spelled out (no const `chars().count()`), so
    /// nothing but this test stops the labels and the width the badges
    /// give way to from drifting apart.
    #[test]
    fn toggle_width_matches_its_labels() {
        let labels: u16 = TOGGLE_SEGS.iter().map(|(l, _)| l.chars().count() as u16).sum();
        assert_eq!(TOGGLE_W, labels + 3, "both labels and the \" │ \" between them");
    }
}
