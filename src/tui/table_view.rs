//! Turning a scope into the table model that `table.rs` draws.

use super::*;

/// Responsive column plan for the table: (author width, show Key).
/// Degradation order: the Key column drops first (it is redundant with
/// the card footer) as soon as titles would fall below a comfortable
/// width; then the author column shrinks toward its floor; the title
/// keeps a hard minimum via its Min constraint.
pub(super) fn column_layout(width: u16) -> (u16, bool) {
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

/// The paper columns' resting colours. Shared rather than written out
/// per scope so the library and the manuscript — which now draw the
/// same columns — cannot drift apart in how they draw them. `lit` is
/// the hovered row, which brightens the two columns that dim off it.
pub(super) struct RowPalette {
    pub pdf: Style,
    /// the ● membership column — library only, but part of the set
    pub ms: Style,
    pub year: Style,
    pub author: Style,
    pub key: Style,
}

pub(super) fn row_palette(lit: bool) -> RowPalette {
    RowPalette {
        pdf: Style::default().fg(Color::Green),
        ms: Style::default().fg(Color::Magenta),
        year: if lit {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Green).add_modifier(Modifier::DIM)
        },
        author: if lit {
            Style::default().fg(text_strong())
        } else {
            Style::default().fg(table_text())
        },
        key: if lit {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM)
        },
    }
}

/// "Quist, J. and Blomqvist, A." → "Quist, Blomqvist" (surnames, truncated).
pub(super) fn format_authors(author: &str) -> String {
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

impl App {
    pub(super) fn draw_table(&mut self, f: &mut Frame, area: Rect) {
        self.last_table_area = area;
        self.clamp_cursor();
        let model = self.table_model(area.width);
        // where the metric column landed, for the priority wheel: the
        // solver is the only thing that knows, and the wheel handler
        // wants the column's full height, header rows included
        let widths = table::solve(&model.columns, area.width);
        let mut mx = area.x;
        for (spec, w) in model.columns.iter().zip(widths.iter()) {
            if spec.id == Col::Metric {
                self.hits.add(
                    Rect { x: mx, y: area.y, width: *w, height: area.height },
                    Target::Metric,
                );
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
        for (r, col) in rects {
            self.hits.add(r, Target::SortHeader(col));
        }
        if let Some(hint) = self.empty_hint() {
            draw_empty_hint(f, data_area, &hint);
        }
    }

    fn table_model(&self, width: u16) -> table::TableModel {
        match self.scopes.get(self.active_scope) {
            Some(Scope::Manuscript { rows }) => self.manuscript_model(rows, width),
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
                    // A DOI query is one the user did not compose — it
                    // came from a link, and they are looking at the paper
                    // while the page says nothing found. That is almost
                    // never a wrong DOI: ADS ingests a publisher's record
                    // days after the paper appears, and until it does the
                    // paper is in ADS only as its preprint, whose title
                    // is often not the title it was published under. So
                    // the way through is the authors, not the title.
                    QueryState::Ready if tab.query.starts_with("doi:") => {
                        "ADS has no record with that DOI. A paper published in the last few days is usually not indexed yet — its preprint may be here already, often under an earlier title, so try the authors and year".to_string()
                    }
                    QueryState::Ready if tab.query.starts_with("identifier:\"arXiv:") => {
                        "ADS has no record with that arXiv id — a new posting usually appears within a day or two".to_string()
                    }
                    QueryState::Ready => "no results — r re-runs, +/- changes n".to_string(),
                })
            }
            Some(Scope::Ads { .. }) => None,
            _ => {
                // "the library is empty" is the wrong thing to say to
                // someone standing in a fresh project with a full global
                // library one keystroke away — which is now where a
                // session starts, so the empty table has to distinguish
                // an empty database from a hidden tier
                if self.order.is_empty() && !self.lib.global_active() {
                    Some(
                        "this project's db is empty — t shows your global library, S searches ADS"
                            .to_string(),
                    )
                } else if self.order.is_empty() {
                    Some("library is empty — S searches ADS, or: astrobib add <bibcode>".to_string())
                } else if self.visible.is_empty() {
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
    pub(super) fn gutter(&self, id: Option<&str>, at_cursor: bool) -> (&'static str, Style) {
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
}

#[cfg(test)]
mod tests {
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
}
