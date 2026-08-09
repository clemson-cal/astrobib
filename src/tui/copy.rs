//! The copy chord, and everything it can put on the clipboard.

use super::*;

/// What the copy chord / Copy tab can put on the clipboard. Cite keys
/// and bibcodes join with ", " under multi-selection (comma lists paste
/// straight into \cite{...}); URLs, paths, and titles join with newlines.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyItem {
    /// Not a datum of a paper but of the *scope*: the whole query
    /// configuration, as an ADS search URL.
    QueryConfig,
    Key,
    FullKey,
    Bibcode,
    AdsUrl,
    ArxivUrl,
    DoiUrl,
    PdfPath,
    Title,
    Abstract,
}

/// The copy chord: key, menu label, and what it copies. One table, so
/// the keys the chord accepts and the options the footer offers are the
/// same list by construction — they were a `match` and a hand-written
/// string, and drifted.
/// `(key, label, short label, item)`. The short labels exist because the
/// menu shares its line with the view badges and, with everything
/// available, the full one no longer fits at 140 columns.
pub(super) const COPY_CHORD: [(char, &str, &str, CopyItem); 10] = [
    ('y', "key", "key", CopyItem::Key),
    ('Y', "full key", "full", CopyItem::FullKey),
    ('b', "bibcode", "bib", CopyItem::Bibcode),
    ('a', "ADS", "ADS", CopyItem::AdsUrl),
    ('x', "arXiv", "arXiv", CopyItem::ArxivUrl),
    ('d', "DOI", "DOI", CopyItem::DoiUrl),
    ('p', "PDF path", "PDF", CopyItem::PdfPath),
    ('t', "title", "title", CopyItem::Title),
    ('A', "abstract", "abs", CopyItem::Abstract),
    ('q', "this query", "query", CopyItem::QueryConfig),
];

/// Footer hint for a ⧉ copy row: what is copied, and the y-chord.
pub(super) fn copy_hint(item: CopyItem) -> &'static str {
    match item {
        CopyItem::Key => "⧉ copy the cite key  ·  y y",
        CopyItem::FullKey => "⧉ copy the full key  ·  y Y",
        CopyItem::Bibcode => "⧉ copy the bibcode  ·  y b",
        CopyItem::AdsUrl => "⧉ copy the ADS URL  ·  y a",
        CopyItem::ArxivUrl => "⧉ copy the arXiv URL  ·  y x",
        CopyItem::DoiUrl => "⧉ copy the DOI URL  ·  y d",
        CopyItem::PdfPath => "⧉ copy the cached PDF's path  ·  y p",
        CopyItem::Title => "⧉ copy the title  ·  y t",
        CopyItem::Abstract => "⧉ copy the abstract  ·  y A",
        CopyItem::QueryConfig => "⧉ copy this query's configuration  ·  y q",
    }
}

/// System clipboard: pbcopy on macOS (reliable in any terminal), else
/// the OSC 52 escape (terminal-dependent, but works over SSH).
fn copy_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    if cfg!(target_os = "macos") {
        use std::process::{Command, Stdio};
        if let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
            let wrote = child
                .stdin
                .take()
                .map(|mut s| s.write_all(text.as_bytes()).is_ok())
                .unwrap_or(false);
            if wrote && child.wait().is_ok_and(|s| s.success()) {
                return true;
            }
        }
    }
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes())).is_ok() && out.flush().is_ok()
}

/// Read the system clipboard, or None where there is no way to.
///
/// There is no OSC 52 counterpart: terminals overwhelmingly refuse to
/// *answer* a clipboard read, since that would let any program running
/// in them exfiltrate whatever the user last copied. So this is the
/// platform tool or nothing, and "nothing" is reported rather than
/// guessed at.
pub(super) fn read_clipboard() -> Option<String> {
    use std::process::Command;
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ]
    };
    for (bin, args) in candidates {
        if let Ok(out) = Command::new(bin).args(*args).output() {
            if out.status.success() {
                return String::from_utf8(out.stdout).ok();
            }
        }
    }
    None
}

/// Minimal RFC 4648 base64 for the OSC 52 payload (not worth a crate).
pub(super) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

impl App {
    /// y — await a target key; the panel force-shows the Copy tab as a
    /// which-key menu (restored by exit_copy_mode).
    pub(super) fn enter_copy_mode(&mut self) {
        let on_article = matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. }));
        if !on_article && self.action_keys().is_empty() {
            self.note(MsgCat::Warn, "nothing to copy".to_string());
            return;
        }
        self.mode = Mode::Copy;
    }

    /// The selected articles of the active ADS scope, in row order.
    pub(super) fn selected_articles(&self) -> Vec<&crate::ads::Article> {
        let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) else {
            return vec![];
        };
        if !self.select_mode || self.selected.is_empty() {
            return vec![];
        }
        articles.iter().filter(|a| self.selected.contains(&a.bibcode)).collect()
    }

    /// A copy value spanning several selected query results: list-like
    /// items join with commas (keys, bibcodes) or newlines (URLs,
    /// paths); prose (title, abstract) has no sensible multi form.
    fn articles_copy_value(&self, items: &[&crate::ads::Article], item: CopyItem) -> Option<String> {
        let vals: Vec<String> = items
            .iter()
            .filter_map(|a| match item {
                // a scope's property, not a paper's: copy_text
                // answers it before any of these are reached
                CopyItem::QueryConfig => None,

                CopyItem::Key | CopyItem::FullKey => Some(self.hypothetical_key(a)),
                CopyItem::Bibcode => Some(a.bibcode.clone()),
                CopyItem::AdsUrl => Some(format!(
                    "https://ui.adsabs.harvard.edu/abs/{}/abstract",
                    a.bibcode
                )),
                CopyItem::ArxivUrl => {
                    crate::ads::arxiv_id(a).map(|id| format!("https://arxiv.org/abs/{id}"))
                }
                CopyItem::DoiUrl => a.doi.first().map(|d| format!("https://doi.org/{d}")),
                CopyItem::PdfPath => self
                    .article_entry(a)
                    .map(|e| e.key().to_string())
                    .filter(|k| pdf::is_cached(k))
                    .map(|k| pdf::cache_path(&k).to_string_lossy().into_owned()),
                CopyItem::Title | CopyItem::Abstract => None, // no multi form
            })
            .collect();
        if vals.is_empty() {
            return None;
        }
        let sep = match item {
            CopyItem::Key | CopyItem::FullKey | CopyItem::Bibcode => ", ",
            _ => "\n",
        };
        Some(vals.join(sep))
    }

    /// The chord/click copy value for the shown ADS article — the same
    /// items the card's ⧉ rows offer, from the article itself.
    pub(super) fn article_copy_value(&self, item: CopyItem) -> Option<String> {
        let Some(Scope::Ads { articles, .. }) = self.scopes.get(self.active_scope) else {
            return None;
        };
        let a = self.card_article_pos().and_then(|p| articles.get(p))?;
        self.article_value(a, item)
    }

    /// Every copyable datum of one query-result article.
    fn article_value(&self, a: &crate::ads::Article, item: CopyItem) -> Option<String> {
        match item {
            // a scope's property, not a paper's: copy_text answers
            // it before any of these are reached
            CopyItem::QueryConfig => None,
            CopyItem::Title => Some(a.title.clone()),
            CopyItem::Abstract => {
                (!a.abstract_.is_empty()).then(|| crate::ads::clean_abstract(&a.abstract_))
            }
            CopyItem::Bibcode => Some(a.bibcode.clone()),
            CopyItem::AdsUrl => Some(format!(
                "https://ui.adsabs.harvard.edu/abs/{}/abstract",
                a.bibcode
            )),
            CopyItem::ArxivUrl => {
                crate::ads::arxiv_id(a).map(|id| format!("https://arxiv.org/abs/{id}"))
            }
            CopyItem::DoiUrl => a.doi.first().map(|d| format!("https://doi.org/{d}")),
            CopyItem::PdfPath => self
                .article_entry(a)
                .map(|e| e.key().to_string())
                .filter(|k| pdf::is_cached(k))
                .map(|k| pdf::cache_path(&k).to_string_lossy().into_owned()),
            CopyItem::Key | CopyItem::FullKey => Some(self.hypothetical_key(a)),
        }
    }

    pub(super) fn exit_copy_mode(&mut self) {
        if matches!(self.mode, Mode::Copy) {
            self.mode = Mode::Normal;
        }
    }

    /// The clipboard text an item yields over the current targets, or
    /// None when no target has the field (also the panel's dimming test).
    fn copy_value(&self, item: CopyItem) -> Option<String> {
        self.copy_value_keys(&self.action_keys(), item)
    }

    fn copy_value_keys(&self, keys: &[String], item: CopyItem) -> Option<String> {
        let mut vals: Vec<String> = vec![];
        for k in keys {
            let Some(e) = self.lib.get(k) else { continue };
            let v = match item {
                // a scope's property, not a paper's
                CopyItem::QueryConfig => None,
                CopyItem::Key => Some(if e.short_key.is_empty() {
                    e.key().to_string()
                } else {
                    e.short_key.clone()
                }),
                CopyItem::FullKey => Some(e.key().to_string()),
                CopyItem::Bibcode => e.bibcode().map(str::to_string),
                CopyItem::AdsUrl => (!e.adsurl().is_empty()).then(|| e.adsurl().to_string()),
                CopyItem::ArxivUrl => (!e.eprint().is_empty())
                    .then(|| format!("https://arxiv.org/abs/{}", e.eprint())),
                CopyItem::DoiUrl => {
                    (!e.doi().is_empty()).then(|| format!("https://doi.org/{}", e.doi()))
                }
                CopyItem::PdfPath => pdf::is_cached(k)
                    .then(|| pdf::cache_path(k).to_string_lossy().into_owned()),
                CopyItem::Abstract => {
                    (!e.abstract_().is_empty()).then(|| e.abstract_().to_string())
                }
                CopyItem::Title => {
                    let t = e.title().trim_matches(['{', '}']).to_string();
                    (!t.is_empty()).then_some(t)
                }
            };
            if let Some(v) = v {
                vals.push(v);
            }
        }
        if vals.is_empty() {
            return None;
        }
        let sep = match item {
            CopyItem::Key | CopyItem::FullKey | CopyItem::Bibcode => ", ",
            _ => "\n",
        };
        Some(vals.join(sep))
    }

    /// Copy one entry's datum (the card's copy-regions path).
    pub(super) fn do_copy_single(&mut self, key: String, item: CopyItem) {
        match self.copy_value_keys(&[key], item) {
            Some(text) => self.finish_copy(&text.clone()),
            None => self.note(MsgCat::Warn, "nothing to copy".to_string()),
        }
    }

    /// What `item` would put on the clipboard right now, or why it
    /// would not.
    ///
    /// The menu and the action both go through this, so an option can
    /// only be offered when pressing it would actually copy something —
    /// the two used to be a static string and a separate resolution, and
    /// the menu offered "bibcode" for papers that have none and "this
    /// query" on the library, where there is no query.
    fn copy_text(&self, item: CopyItem) -> Result<String, String> {
        if item == CopyItem::QueryConfig {
            let Some(Scope::Ads { tab, .. }) = self.scopes.get(self.active_scope) else {
                return Err("no query here to copy — this is the library".to_string());
            };
            return Ok(crate::ads::search_url(&tab.query, tab.limit, &tab.ads_sort));
        }
        let multi_prose = matches!(item, CopyItem::Title | CopyItem::Abstract);
        let nothing = || "nothing to copy".to_string();
        if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
            let sel = self.selected_articles();
            if sel.len() > 1 {
                if multi_prose {
                    return Err(format!("no multi-item form for that ({} selected)", sel.len()));
                }
                return self.articles_copy_value(&sel, item).ok_or_else(nothing);
            }
            if sel.len() == 1 {
                return self.article_value(sel[0], item).ok_or_else(nothing);
            }
            return self.article_copy_value(item).ok_or_else(nothing);
        }
        if multi_prose && self.select_mode && self.selected.len() > 1 {
            return Err(format!(
                "no multi-item form for that ({} selected)",
                self.selected.len()
            ));
        }
        self.copy_value(item).ok_or_else(nothing)
    }

    /// Whether the copy menu should offer `item` on the current screen.
    fn copy_offered(&self, item: CopyItem) -> bool {
        self.copy_text(item).is_ok()
    }

    /// The which-key line for the copy chord, listing only what this
    /// screen can actually copy — no "this query" on the library, no
    /// "bibcode" for a paper that has none.
    pub(super) fn copy_menu(&self, width: u16) -> String {
        let offered: Vec<&(char, &str, &str, CopyItem)> =
            COPY_CHORD.iter().filter(|(.., item)| self.copy_offered(*item)).collect();
        if offered.is_empty() {
            return "nothing here to copy · Esc cancel".to_string();
        }
        // shortening beats truncating: a cut-off menu hides options that
        // are available, which is the failure this whole change is about
        let render = |short: bool, tail: bool, sep: &str| {
            let body = offered
                .iter()
                .map(|(k, long, s, _)| format!("{k} {}", if short { s } else { long }))
                .collect::<Vec<_>>()
                .join(sep);
            if tail {
                format!("{body}{sep}Esc cancel")
            } else {
                body
            }
        };
        // last resort, for a terminal too narrow for any of it: the keys
        // alone. They still say what the chord accepts, and the card's
        // copy column carries the meanings — colliding with the badges
        // would say nothing at all.
        let keys = || {
            offered.iter().map(|(k, ..)| k.to_string()).collect::<Vec<_>>().join(" ")
        };
        let fits = |s: &String| s.chars().count() <= width as usize;
        // words before separators before labels: which key does what is
        // the information here, and the dots are only comfort
        [
            render(false, true, " · "),
            render(true, true, " · "),
            render(true, false, " · "),
            render(true, false, "  "),
            keys(),
        ]
            .into_iter()
            .find(fits)
            .unwrap_or_else(|| {
                let mut t: String = keys().chars().take(width.saturating_sub(1) as usize).collect();
                t.push('…');
                t
            })
    }

    pub(super) fn do_copy(&mut self, item: CopyItem) {
        self.exit_copy_mode();
        match self.copy_text(item) {
            Ok(text) => self.finish_copy(&text),
            Err(why) => self.note(MsgCat::Warn, why),
        }
    }

    pub(super) fn finish_copy(&mut self, text: &str) {
        if copy_to_clipboard(&text) {
            let first = text.lines().next().unwrap_or("");
            let mut shown: String = first.chars().take(60).collect();
            if shown.len() < text.len() {
                shown.push('…');
            }
            self.note(MsgCat::Ok, format!("Copied: {shown}"));
        } else {
            self.note(
                MsgCat::Err,
                "clipboard copy failed — terminal may not support OSC 52".to_string(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_rfc4648_vectors() {
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
            ("Quist2019abcde", "UXVpc3QyMDE5YWJjZGU="),
        ] {
            assert_eq!(super::base64(input.as_bytes()), want);
        }
    }
}
