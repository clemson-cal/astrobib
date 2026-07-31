//! The pub card: one model, one renderer, both sides of the app.
//!
//! A card is drawn for a library entry and for an ADS query result, and
//! the two used to be separate near-identical renderers that drifted
//! apart with every fix. Here `CardModel` says what a card shows,
//! `entry_card` / `article_card` are the only places the two sides
//! differ, and `draw_card` renders the model — so a card change lands on
//! both or neither.
//!
//! Everything else the card leans on (the link stack, the cited-by line,
//! the verbatim-BibTeX views, the pinned toggler, the wrap and pill
//! helpers) lives in the parent module and is reached through `super`.

use super::*;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// What follows the cite key on the card's last line.
enum KeyAffix {
    /// A pill that joins the key line when it fits and drops to its own
    /// line when it does not — the library card's manuscript chip.
    Chip { label: String, btn: CardBtn, fg: Color },
    /// Plain text, always on the key line — the query card's
    /// "● in library" / "→ import" affordance.
    Text { label: &'static str, btn: CardBtn, fg: Color, hover_fg: Color },
}

/// Everything one pub card draws, whichever side it came from.
///
/// The two cards — a library `Entry` and an ADS `Article` (plus its
/// imported twin, if any) — differ only in how this is filled in;
/// `draw_card` renders it, so a card fix can only ever land on both.
/// Fields are in render order.
struct CardModel {
    /// Wrapped as-is: the builder has already stripped BibTeX braces.
    title: String,
    /// "Surname, Surname   ·   YYYY", pre-formatted.
    byline: String,
    /// When set, the byline's trailing year takes the green accent (the
    /// library card only — a query result's year stays dim).
    year_accent: Option<String>,
    /// journal volume(issue), pages; empty draws no publication line.
    publine: String,
    /// Outer `None` omits the cited-by line entirely — a query result
    /// with no imported twin has nowhere to store a refreshed count.
    /// Inner `None` renders "cited by ?".
    cited_by: Option<Option<i64>>,
    abstract_: String,
    /// Action block, left column: → browser links and ⌕ in-app queries.
    links: Vec<(String, LinkTarget, bool)>,
    /// Action block, right column: the permanent ⧉ copy menu. The bool
    /// on both is "enabled"; disabled rows dim and refuse clicks.
    copies: Vec<(String, LinkTarget, bool)>,
    /// The PDF pill row, or `None` when the card has no PDF block at all
    /// (an un-imported query result: PDFs cache under cite keys).
    pdf: Option<Vec<(&'static str, CardBtn, Color)>>,
    /// Whether a non-empty PDF status row is followed by a blank one.
    status_gap: bool,
    /// Footer keywords line (library entries only).
    keywords: String,
    /// The cite key as styled runs: real short key + dim hash suffix, or
    /// the hypothetical key a query result would get on import.
    key_runs: Vec<(String, Style)>,
    key_affix: Option<KeyAffix>,
}

impl App {
    /// Build the card model for a library entry.
    fn entry_card(&self, key: &str) -> Option<CardModel> {
        let e = self.lib.get(key)?;
        let year = e.year();
        // journal · volume(issue), pages — under the byline
        let journal = crate::export::journal_name(e.journal());
        let mut publine = String::new();
        if !journal.is_empty() {
            publine = journal;
            if !e.volume().is_empty() {
                publine.push_str(&format!(" {}", e.volume()));
            }
            if !e.number().is_empty() {
                publine.push_str(&format!("({})", e.number()));
            }
            if !e.pages().is_empty() {
                publine.push_str(&format!(", {}", e.pages()));
            }
        }
        let (eprint, adsurl, doi) = (
            e.eprint().to_string(),
            e.adsurl().to_string(),
            e.doi().to_string(),
        );
        let multi_sel = self.select_mode && self.selected.len() > 1;
        // browser links plus the citation-graph pair — "citations" /
        // "references" spawn ADS query scopes rather than opening the
        // browser, so they register as card buttons. Every operation
        // stays visible; unavailable ones render dimmed.
        let links: Vec<(String, LinkTarget, bool)> = vec![
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
            ("PDF path".into(), LinkTarget::Copy(CopyItem::PdfPath), pdf::is_cached(key)),
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
        // only the buttons whose source is available appear (ineligible
        // ones are hidden, not dimmed)
        let cached = pdf::is_cached(key);
        let mut buttons: Vec<(&'static str, CardBtn, Color)> = vec![];
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
            buttons.push(("Open →", CardBtn::Open, Color::Green));
            buttons.push(("Clear ✕", CardBtn::Clear, Color::Gray));
        }
        let short = if e.short_key.is_empty() { e.key() } else { &e.short_key };
        let suffix: String = e.key().chars().skip(short.chars().count()).collect();
        let key_runs = vec![
            (short.to_string(), Style::default().fg(Color::Cyan)),
            // dim cyan, not DarkGray+DIM: stays legible in themes where
            // doubly-dimmed gray vanishes, and sits closer to the
            // leading portion's color
            (suffix, Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM)),
        ];
        let key_affix = (self.lib.manuscript.is_some() && self.lib.global_on).then(|| {
            let in_ms = self.lib.in_manuscript(key);
            KeyAffix::Chip {
                label: if in_ms { "◆ in manuscript" } else { "◇ add to manuscript" }.to_string(),
                btn: CardBtn::MsToggle,
                fg: if in_ms { Color::Magenta } else { Color::Gray },
            }
        });
        Some(CardModel {
            title: e.title().trim_matches(['{', '}']).to_string(),
            byline: format!("{}   ·   {year}", format_authors(e.author())),
            year_accent: Some(year),
            publine,
            cited_by: Some(self.metrics.get(key).and_then(|m| m.citations)),
            abstract_: e.abstract_().to_string(),
            links,
            copies,
            pdf: Some(buttons),
            status_gap: true,
            keywords: e.keywords().join(" · "),
            key_runs,
            key_affix,
        })
    }

    /// Build the card model for an ADS query result, reconciled with its
    /// imported twin: entry-side affordances (PDFs, the manuscript row)
    /// appear only once there is a library key for them to act on.
    fn article_card(&self, a: &crate::ads::Article) -> CardModel {
        let doi = a.doi.first().cloned().unwrap_or_default();
        let eprint = crate::ads::arxiv_id(a).map(str::to_string).unwrap_or_default();
        let bibcode = a.bibcode.clone();
        let hyp_key = self.hypothetical_key(a);
        let lib_key = self.lib.get_by_bibcode(&bibcode).map(|e| e.key().to_string());
        let in_lib = lib_key.is_some();
        let mut publine = String::new();
        if !a.journal.is_empty() {
            publine = a.journal.clone();
            if !a.volume.is_empty() {
                publine.push_str(&format!(" {}", a.volume));
            }
            if !a.issue.is_empty() {
                publine.push_str(&format!("({})", a.issue));
            }
            if !a.page.is_empty() {
                publine.push_str(&format!(", {}", a.page));
            }
        }
        let mut links: Vec<(String, LinkTarget, bool)> = vec![
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
            links.push((
                if in_ms { "in manuscript".into() } else { "add to manuscript".to_string() },
                LinkTarget::Query(CardBtn::MsToggle),
                in_lib && self.lib.global_on,
            ));
        }
        // prose has no multi-item form: those rows dim while several
        // rows are selected (the others copy across the selection)
        let multi = self.select_mode && self.selected.len() > 1;
        let abstract_ = crate::ads::clean_abstract(&a.abstract_);
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
        // once imported, the library card's PDF buttons appear here too,
        // acting on the imported entry (card_entry_key routes clicks)
        let pdf = lib_key.as_deref().map(|kk| {
            let mut buttons: Vec<(&'static str, CardBtn, Color)> = vec![];
            if !pdf::is_cached(kk) {
                if !eprint.is_empty() {
                    buttons.push(("arXiv ↓", CardBtn::Arxiv, Color::Cyan));
                }
                buttons.push(("ADS OA ↓", CardBtn::Oa, Color::Cyan));
                buttons.push(("browser ↓", CardBtn::Browser, Color::Yellow));
                buttons.push(("pick …", CardBtn::Pick, Color::Magenta));
            } else {
                buttons.push(("Open →", CardBtn::Open, Color::Green));
                buttons.push(("Clear ✕", CardBtn::Clear, Color::Gray));
            }
            buttons
        });
        CardModel {
            title: a.title.clone(),
            byline: format!("{}   ·   {}", format_authors(&a.author.join(" and ")), a.year),
            year_accent: None,
            publine,
            // "?" invites a refresh only when there is an imported entry
            // to refresh into; otherwise the line appears only with a count
            cited_by: (a.citation_count.is_some() || in_lib).then_some(a.citation_count),
            abstract_,
            links,
            copies,
            pdf,
            status_gap: false,
            keywords: String::new(),
            key_runs: vec![(hyp_key, Style::default().fg(Color::Cyan))],
            key_affix: Some(if in_lib {
                KeyAffix::Text {
                    label: "● in library",
                    btn: CardBtn::RemoveFromLib,
                    fg: Color::Magenta,
                    hover_fg: Color::Red,
                }
            } else {
                // the footer arrow IS the import affordance (i also works)
                KeyAffix::Text {
                    label: "→ import",
                    btn: CardBtn::Import,
                    fg: Color::DarkGray,
                    hover_fg: Color::Green,
                }
            }),
        }
    }

    /// The one pub-card renderer, top to bottom: title, byline
    /// (authors · year), publication line, the clickable "cited by N ⟳"
    /// line, the wheel-scrollable abstract, a rule, the two-column action
    /// block (links + ⌕ queries left, ⧉ copies right), a rule, the PDF
    /// button pills, the PDF status row, the footer (keywords, cite key,
    /// manuscript chip / import affordance), and the pinned view toggler.
    /// Text is pre-wrapped so every row's click rect is exact.
    fn draw_card(&mut self, f: &mut Frame, area: Rect, m: CardModel) {
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
        let dim = Style::default().fg(Color::DarkGray);

        // ── body ─────────────────────────────────────────────────────
        for l in wrap_text(&m.title, w) {
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
        let by_lines = wrap_text(&m.byline, w);
        for (i, l) in by_lines.iter().enumerate() {
            let accent = m
                .year_accent
                .as_ref()
                .filter(|year| i == by_lines.len() - 1 && l.chars().count() > year.chars().count());
            let line = match accent {
                Some(year) => {
                    let split = l.chars().count() - year.chars().count();
                    let head: String = l.chars().take(split).collect();
                    Line::from(vec![
                        Span::styled(head, dim),
                        Span::styled(year.clone(), Style::default().fg(Color::Green)),
                    ])
                }
                None => Line::from(Span::styled(l.clone(), dim)),
            };
            line_at(f, y, line);
            y += 1;
        }
        if !m.publine.is_empty() {
            for l in wrap_text(&m.publine, w) {
                line_at(f, y, Line::from(Span::styled(l, dim)));
                y += 1;
            }
        }
        if let Some(n) = m.cited_by {
            if y < bottom {
                draw_cited_line(
                    f, x0, y, w as u16, n, self.hover,
                    &mut self.card_buttons, &mut self.hover_hint,
                );
            }
            y += 1;
        }

        // Everything below the abstract has to fit: the rule, the action
        // block, the closing rule and its air, the PDF pills, the status
        // row (which collapses when idle), the keywords, the key line,
        // and the manuscript chip when it cannot share that line.
        let link_lines = m.links.len().max(m.copies.len()) as u16;
        let kw_lines = if m.keywords.is_empty() {
            0
        } else {
            wrap_text(&m.keywords, w).len() as u16 + 1
        };
        let busy = self.poll_cancel.is_some() || !self.pdf_status.is_empty();
        let pdf_rows = u16::from(m.pdf.is_some());
        let status_rows = if m.pdf.is_some() {
            1 + u16::from(busy && m.status_gap)
        } else {
            0
        };
        let chip_row = u16::from(matches!(m.key_affix, Some(KeyAffix::Chip { .. })));
        let rest = 1 + link_lines + 2 + pdf_rows + status_rows + kw_lines + 1 + chip_row;
        if !m.abstract_.is_empty() && y + rest < bottom {
            y += 1;
            // truncation is height-driven only: the full abstract shows
            // whenever the card has room, and a cut ends in an ellipsis
            let avail = (bottom - y).saturating_sub(rest) as usize;
            let (shown, above, below) =
                scroll_window(wrap_text(&m.abstract_, w), avail, &mut self.card_scroll);
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

        // ── action block (bordered top and bottom), right below the
        //    abstract ───────────────────────────────────────────────────
        let sep = "─".repeat(w);
        let dimsep = divider();
        line_at(f, y, Line::from(Span::styled(sep.clone(), dimsep)));
        y += 1;
        y = draw_link_stack(f, x0, y, w as u16, bottom, self.hover, m.links, m.copies, &mut self.card_links, &mut self.card_buttons, &mut self.hover_hint, &mut yanks);
        line_at(f, y, Line::from(Span::styled(sep, dimsep)));
        y += 2; // a little air below the link stack

        // ── PDF buttons, drawn as rounded pills, then the transient PDF
        //    status (◌ waiting…, ⚠ results) ────────────────────────────
        if let Some(buttons) = m.pdf {
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
            // the status row collapses when idle, keeping the footer
            // close to the buttons
            y += 1 + u16::from(busy && m.status_gap);
        }

        // ── footer ───────────────────────────────────────────────────
        if !m.keywords.is_empty() {
            for l in wrap_text(&m.keywords, w) {
                line_at(f, y, Line::from(Span::styled(l, dim)));
                y += 1;
            }
            y += 1;
        }
        let mut spans: Vec<Span> = m
            .key_runs
            .into_iter()
            .map(|(text, style)| Span::styled(text, style))
            .collect();
        let used: u16 = spans.iter().map(|s| s.content.chars().count() as u16).sum();
        yanks.push((Rect { x: x0, y, width: used.max(1), height: 1 }, CopyItem::Key));
        if hov_region == Some(CopyItem::Key) {
            for s in &mut spans {
                s.style = s.style.patch(tint);
            }
        }
        match m.key_affix {
            // the manuscript chip joins the citekey line when it fits,
            // else drops to the next line
            Some(KeyAffix::Chip { label, btn, fg }) => {
                let cw = pill_width(&label);
                let inline = used + 2 + cw <= w as u16;
                let (cx, cy) = if inline { (x0 + used + 2, y) } else { (x0, y + 1) };
                let r = Rect { x: cx, y: cy, width: cw, height: 1 };
                self.card_buttons.push((r, btn));
                let hovb = hit(r, hv.0, hv.1);
                if hovb {
                    self.hover_hint = Some(card_hint(btn).to_string());
                }
                let bg = if hovb { Color::Rgb(58, 63, 72) } else { Color::Rgb(40, 44, 52) };
                if inline {
                    spans.push(Span::raw("  "));
                    push_pill(&mut spans, &label, bg, fg);
                    line_at(f, y, Line::from(spans));
                } else {
                    line_at(f, y, Line::from(std::mem::take(&mut spans)));
                    let mut cs: Vec<Span> = vec![];
                    push_pill(&mut cs, &label, bg, fg);
                    line_at(f, cy, Line::from(cs));
                }
            }
            Some(KeyAffix::Text { label, btn, fg, hover_fg }) => {
                let r = Rect {
                    x: x0 + used + 2,
                    y,
                    width: label.chars().count() as u16,
                    height: 1,
                };
                self.card_buttons.push((r, btn));
                let hovb = hit(r, hv.0, hv.1);
                if hovb {
                    self.hover_hint = Some(card_hint(btn).to_string());
                }
                let style = if hovb {
                    Style::default().fg(hover_fg).add_modifier(Modifier::UNDERLINED)
                } else {
                    Style::default().fg(fg)
                };
                spans.push(Span::raw("  "));
                spans.push(Span::styled(label, style));
                line_at(f, y, Line::from(spans));
            }
            None => line_at(f, y, Line::from(spans)),
        }
        self.card_yanks = yanks;
        self.draw_card_toggle(f, x0, w as u16, bottom, false);
    }

    /// The card for the highlighted ADS query result. Its `v` view is its
    /// own: an imported twin shows its real .bib file, while an
    /// un-imported article previews the exact canonical BibTeX an import
    /// would write (fetched once, cached by bibcode).
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
        if self.show_bib_source {
            let bibcode = a.bibcode.clone();
            let eprint = crate::ads::arxiv_id(a).map(str::to_string).unwrap_or_default();
            let doi = a.doi.first().cloned().unwrap_or_default();
            if let Some(k) = self.lib.get_by_bibcode(&bibcode).map(|e| e.key().to_string()) {
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
        let model = self.article_card(a);
        self.draw_card(f, area, model);
    }

    /// The pub card for the highlighted library entry (or, in an ADS
    /// scope, the highlighted query result).
    pub(super) fn draw_detail(&mut self, f: &mut Frame, area: Rect) {
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
        if self.show_bib_source {
            self.draw_bib_source(f, area, &key);
            return;
        }
        let Some(model) = self.entry_card(&key) else { return };
        self.draw_card(f, area, model);
    }
}
