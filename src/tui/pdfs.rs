//! Fetching, opening and forgetting PDFs.

use super::*;

pub(super) enum DlMsg {
    Progress(String),
    Done { id: u64, done: usize, failed: Vec<String> },
}

/// Sort fallback for a key the library no longer holds. `self.order`
/// and the library are kept in step by rebuild_order — every mutation
/// path calls it — but nothing in the type system enforces that, and a
/// future path that forgets must not take the running TUI down with it.
/// Orphans sort to the end in either direction (the sort direction is
/// applied to real entries only), keyed by cite key so the order stays
/// total, stable, and identical to before whenever the two agree.
pub(super) fn orphan_order(a_live: bool, b_live: bool, a: &str, b: &str) -> std::cmp::Ordering {
    match (a_live, b_live) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.cmp(b),
    }
}

impl App {
    /// o — open every cached PDF among the targets.
    pub(super) fn open_pdfs(&mut self) {
        let paths: Vec<_> = self
            .action_keys()
            .iter()
            .filter(|k| pdf::is_cached(k))
            .map(|k| pdf::cache_path(k))
            .collect();
        if paths.is_empty() {
            self.note(MsgCat::Warn, "no cached PDFs in selection  (p to download)".to_string());
            return;
        }
        let n = paths.len();
        pdf::open_paths(&paths);
        self.note(MsgCat::Ok, format!("Opened {n} PDF(s)"));
    }

    /// X — cancel a running browser-download watch, else clear cached PDFs.
    pub(super) fn clear_pdfs(&mut self) {
        if self.poll_cancel.is_some() {
            self.cancel_watch();
            return;
        }
        self.pdf_status.clear();
        let mut n = 0;
        for k in self.action_keys() {
            let p = pdf::cache_path(&k);
            if p.exists() && std::fs::remove_file(&p).is_ok() {
                n += 1;
            }
        }
        self.note(MsgCat::Ok, format!("Cleared {n} cached PDF(s)"));
    }

    /// Fetch one entry's PDF from a specific source (pub card buttons),
    /// on the download worker channel.
    /// The cache is keyed by library cite key, full stop. Every path
    /// that writes a PDF passes through here, so no bibcode-named file
    /// can be created even if a caller is careless.
    fn pdf_key(&mut self, key: String) -> Option<String> {
        if self.lib.get(&key).is_some() {
            return Some(key);
        }
        self.note(
            MsgCat::Warn,
            "import the paper first — PDFs are cached under its cite key".to_string(),
        );
        None
    }

    pub(super) fn download_single(&mut self, key: String, source: pdf::Source) {
        if self.dl_rx.is_some() {
            self.note(MsgCat::Warn, "a download is already running".to_string());
            return;
        }
        let Some(e) = self.lib.get(&key) else { return };
        let (eprint, adsurl) = (e.eprint().to_string(), e.adsurl().to_string());
        let src = match source {
            pdf::Source::Arxiv => "arXiv",
            pdf::Source::Oa => "ADS OA",
            pdf::Source::Auto => "auto",
        };
        let id = self.add_task(
            TaskKind::Download,
            format!("↓ {src} PDF — {key}"),
            vec![key.clone()],
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
        self.note(MsgCat::Info, format!("Downloading {key}…"));
        std::thread::spawn(move || {
            let ok = pdf::fetch_source(&key, &eprint, &adsurl, source).is_some();
            let _ = tx.send(DlMsg::Done {
                id,
                done: ok as usize,
                failed: if ok { vec![] } else { vec![key] },
            });
        });
    }

    pub(super) fn browser_download(&mut self) {
        if let Some(k) = self.action_keys().into_iter().next() {
            self.browser_download_for(k);
        }
    }

    /// B — resolve the best manual-download URL, open the browser, and
    /// watch ~/Downloads for the PDF (60s, cancellable with X).
    pub(super) fn browser_download_for(&mut self, key: String) {
        if self.dl_rx.is_some() {
            self.note(MsgCat::Warn, "a download is already running".to_string());
            return;
        }
        let Some(e) = self.lib.get(&key) else {
            return;
        };
        let (key, doi, adsurl, eprint) = (
            e.key().to_string(),
            e.doi().to_string(),
            e.adsurl().to_string(),
            e.eprint().to_string(),
        );
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.poll_cancel = Some(cancel.clone());
        let id = self.add_task(
            TaskKind::Watch,
            format!("👁 watching ~/Downloads — {key}"),
            vec![key.clone()],
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
        self.note(MsgCat::Info, format!("Resolving browser source for {key}…"));
        std::thread::spawn(move || {
            let Some(url) = pdf::browser_resolve_url(&doi, &adsurl, &eprint) else {
                let _ = tx.send(DlMsg::Done { id, done: 0, failed: vec![key] });
                return;
            };
            let before = pdf::downloads_snapshot();
            pdf::browser_open(&url);
            let _ = tx.send(DlMsg::Progress(format!(
                "Browser opened — waiting for {key} in ~/Downloads (60s, X cancels)…"
            )));
            let got = pdf::poll_downloads(&key, &before, 60, &cancel);
            let _ = tx.send(DlMsg::Done {
                id,
                done: got.is_some() as usize,
                failed: if got.is_some() { vec![] } else { vec![key] },
            });
        });
    }

    /// pick … — open the modal ~/Downloads PDF picker for one entry.
    pub(super) fn open_picker_for(&mut self, key: String) {
        let Some(key) = self.pdf_key(key) else { return };
        let files = pdf::downloads_pdfs();
        if files.is_empty() {
            self.note(MsgCat::Info, "no PDFs in ~/Downloads".to_string());
            return;
        }
        self.mode = Mode::Pick { key, files, sel: 0 };
    }

    /// p — download PDFs for targets not yet cached, on a background
    /// thread so the UI stays live; progress arrives over a channel.
    pub(super) fn download_pdfs(&mut self) {
        if self.dl_rx.is_some() {
            self.note(MsgCat::Warn, "a download is already running".to_string());
            return;
        }
        let items: Vec<(String, String, String)> = self
            .action_keys()
            .iter()
            .filter(|k| !pdf::is_cached(k))
            .filter_map(|k| {
                let e = self.lib.get(k)?;
                if e.eprint().is_empty() && e.adsurl().is_empty() {
                    return None;
                }
                Some((k.clone(), e.eprint().to_string(), e.adsurl().to_string()))
            })
            .collect();
        if items.is_empty() {
            // on a query page there are no library keys at all — say the
            // useful thing rather than the generic one
            let msg = if matches!(self.scopes.get(self.active_scope), Some(Scope::Ads { .. })) {
                "import the paper first (i) — PDFs are cached under its cite key"
            } else {
                "nothing to download (cached, or no arXiv ID / ADS URL)"
            };
            self.note(MsgCat::Warn, msg.to_string());
            return;
        }
        let total = items.len();
        let label = if let [(k, _, _)] = items.as_slice() {
            format!("↓ PDF — {k}")
        } else {
            format!("↓ PDFs — {total} papers")
        };
        let id = self.add_task(
            TaskKind::Download,
            label,
            items.iter().map(|(k, _, _)| k.clone()).collect(),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        self.dl_rx = Some(rx);
        std::thread::spawn(move || {
            let mut done = 0;
            let mut failed = vec![];
            for (i, (key, eprint, adsurl)) in items.iter().enumerate() {
                let _ = tx.send(DlMsg::Progress(format!(
                    "Downloading [{}/{total}] {key}…",
                    i + 1
                )));
                if pdf::fetch(key, eprint, adsurl).is_some() {
                    done += 1;
                } else {
                    failed.push(key.clone());
                }
            }
            let _ = tx.send(DlMsg::Done { id, done, failed });
        });
        self.note(MsgCat::Info, format!("Downloading {total} PDF(s)…"));
    }

    pub(super) fn drain_downloads(&mut self) {
        let mut msgs = vec![];
        if let Some(rx) = &self.dl_rx {
            while let Ok(m) = rx.try_recv() {
                msgs.push(m);
            }
        }
        for m in msgs {
            match m {
                DlMsg::Progress(s) => self.status = s,
                DlMsg::Done { id, done, failed } => {
                    if let Some(t) = self.finish_task(id).filter(|t| t.cancelled) {
                        // discard the cancelled task's result: remove
                        // anything it cached so the end state matches the
                        // existing failure/clear paths, and reset the
                        // channel state exactly as the normal arm does
                        for k in &t.keys {
                            let p = pdf::cache_path(k);
                            if p.exists() {
                                let _ = std::fs::remove_file(&p);
                            }
                        }
                        self.note(
                            MsgCat::Info,
                            format!("cancelled — {} (result discarded)", t.label),
                        );
                        self.pdf_status.clear();
                        self.dl_rx = None;
                        self.poll_cancel = None;
                        continue;
                    }
                    // an unavailable PDF is an expected outcome (many
                    // publishers require the browser), not an app failure
                    let cat = if failed.is_empty() { MsgCat::Ok } else { MsgCat::Warn };
                    let msg = if failed.is_empty() {
                        format!("Downloaded {done} PDF(s)")
                    } else {
                        format!(
                            "Downloaded {done} PDF(s) — no auto PDF for {}{} (try browser ↓)",
                            failed[..failed.len().min(3)].join(", "),
                            if failed.len() > 3 { "…" } else { "" }
                        )
                    };
                    self.note(cat, msg);
                    // success needs no card note — the buttons flipping to
                    // Open/Clear already signal it; an unavailable PDF gets
                    // an expected-outcome note, not an error
                    self.pdf_status = if done == 0 && !failed.is_empty() {
                        "⚠ no open-access PDF found".to_string()
                    } else {
                        String::new()
                    };
                    self.dl_rx = None;
                    self.poll_cancel = None;
                }
            }
        }
    }

    /// Clear (or cancel a pending browser watch for) the card entry's PDF.
    pub(super) fn clear_card_pdf(&mut self, key: &str) {
        if self.poll_cancel.is_some() {
            self.cancel_watch();
            return;
        }
        self.pdf_status.clear();
        let p = pdf::cache_path(key);
        if p.exists() && std::fs::remove_file(&p).is_ok() {
            self.note(MsgCat::Ok, format!("Cleared cached PDF for {key}"));
        }
    }

    /// Centered modal list of ~/Downloads PDFs for the pick action.
    pub(super) fn draw_picker(&mut self, f: &mut Frame) {
        let Mode::Pick { files, sel, key } = &self.mode else {
            return;
        };
        let frame = f.area();
        let h = (files.len().min(15) + 2) as u16;
        let w = 64.min(frame.width.saturating_sub(4));
        let area = Rect {
            x: frame.width.saturating_sub(w) / 2,
            y: frame.height.saturating_sub(h) / 2,
            width: w,
            height: h.min(frame.height),
        };
        f.render_widget(ratatui::widgets::Clear, area);
        let mut lines: Vec<Line> = vec![];
        for (i, p) in files.iter().take(15).enumerate() {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            let row_y = area.y + 1 + i as u16;
            self.hits.add(
                Rect { x: area.x, y: row_y, width: area.width, height: 1 },
                Target::PickRow(i),
            );
            let hov = self.hover.1 == row_y && hit(area, self.hover.0, self.hover.1);
            let style = if i == *sel {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else if hov {
                Style::default().bg(row_hover_bg())
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(name, style)));
        }
        let p = Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" pick PDF for {key} — ⏎ import · Esc cancel ")),
        );
        f.render_widget(p, area);
    }
}
