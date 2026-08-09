//! Background work, and the things it has to say to the log.

use super::*;

/// In-flight background work, listed in the T overlay. Worker threads
/// cannot be killed, so cancelling a thread-backed task only marks it;
/// the drain handler discards its result on arrival. The browser
/// watcher is the exception: it cancels for real via poll_cancel.
#[derive(Clone, Copy)]
pub(super) enum TaskKind {
    Download,
    Query,
    Import,
    Watch,
}

pub(super) struct Task {
    pub(super) id: u64,
    pub(super) label: String,
    pub(super) kind: TaskKind,
    pub(super) cancelled: bool,
    /// cache keys the task may write; discarding a cancelled download
    /// removes them again, restoring the failed-download end state
    pub(super) keys: Vec<String>,
}

/// Message-log categories; each renders in its own color in the log
/// pane and the footer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MsgCat {
    Info,
    Ok,
    Warn,
    Err,
}

impl MsgCat {
    pub(super) fn color(self) -> Color {
        match self {
            MsgCat::Info => Color::Gray,
            MsgCat::Ok => Color::Green,
            MsgCat::Warn => Color::Yellow,
            MsgCat::Err => Color::Red,
        }
    }
}

impl App {
    /// Report a failed user-state write — once. Saved queries, curated
    /// priorities, state.json fields, and refs.bib are written on
    /// changes and on idle ticks, so the choice is not "log it or not"
    /// but "log it once or every tick": the first failure per store is
    /// logged, the rest are latched until that store writes again.
    pub(super) fn state_write(&mut self, what: &'static str, err: Option<String>) {
        match err {
            None => {
                self.write_failed.remove(what);
            }
            Some(e) => {
                if self.write_failed.insert(what) {
                    self.note(MsgCat::Err, format!("could not save {what}: {e}"));
                }
            }
        }
    }

    /// Register an in-flight background task; the returned id travels
    /// with the worker's completion message back to the drain handler.
    pub(super) fn add_task(&mut self, kind: TaskKind, label: String, keys: Vec<String>) -> u64 {
        self.next_task_id += 1;
        let id = self.next_task_id;
        self.tasks.push(Task {
            id,
            label,
            kind,
            cancelled: false,
            keys,
        });
        id
    }

    /// Remove a task on completion, returning it so the drain handler
    /// can tell whether it was cancelled in the meantime.
    pub(super) fn finish_task(&mut self, id: u64) -> Option<Task> {
        self.tasks
            .iter()
            .position(|t| t.id == id)
            .map(|i| self.tasks.remove(i))
    }

    /// Cancel a task: the browser watcher stops for real (poll_cancel);
    /// thread-backed work cannot be killed, so the task is only marked
    /// and its result is discarded on arrival.
    fn cancel_task(&mut self, id: u64) {
        let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) else { return };
        if t.cancelled {
            return;
        }
        t.cancelled = true;
        let (kind, label) = (t.kind, t.label.clone());
        if matches!(kind, TaskKind::Watch) {
            if let Some(cancel) = self.poll_cancel.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            self.note(MsgCat::Warn, "browser download cancelled".to_string());
        } else {
            self.note(
                MsgCat::Warn,
                format!("cancelling — {label} (result will be discarded)"),
            );
        }
    }

    /// Cancel the running browser-download watch (X / the card's ✕),
    /// routing through the task registry when its task is present.
    pub(super) fn cancel_watch(&mut self) {
        if let Some(id) = self
            .tasks
            .iter()
            .find(|t| matches!(t.kind, TaskKind::Watch) && !t.cancelled)
            .map(|t| t.id)
        {
            self.cancel_task(id);
        } else if let Some(cancel) = self.poll_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.note(MsgCat::Warn, "browser download cancelled".to_string());
        }
    }

    /// Emit an event message: color-coded in the log pane and shown in
    /// the footer while it is the newest entry. A new message snaps the
    /// log pane back to the tail; the log keeps at most 500 entries.
    pub(super) fn note(&mut self, cat: MsgCat, msg: String) {
        self.status = msg.clone();
        self.log.push((cat, self.started.elapsed().as_secs(), msg));
        self.log_scroll = 0;
        if self.log.len() > 500 {
            let cut = self.log.len() - 500;
            self.log.drain(..cut);
        }
    }

    /// A note that supersedes its own previous one.
    ///
    /// Holding ← on a column width, or cycling a sort, would otherwise
    /// leave a line per keystroke saying nothing the line before it did
    /// not. The log should record that you resized the column, not how
    /// many times you pressed the key — so a repeat of the same `kind`
    /// within a few seconds replaces its own last entry instead of
    /// appending. Anything the user did not directly cause (a download
    /// finishing, a query landing) always appends, and so uses `note`.
    pub(super) fn note_latest(&mut self, cat: MsgCat, kind: &'static str, msg: String) {
        const WINDOW_SECS: u64 = 6;
        if let Some((k, i)) = self.last_note {
            // only if it is still the newest entry: anything logged since
            // would be silently eaten otherwise
            let fresh = self
                .log
                .get(i)
                .is_some_and(|(_, t, _)| self.started.elapsed().as_secs().saturating_sub(*t) < WINDOW_SECS);
            if k == kind && i + 1 == self.log.len() && fresh {
                self.log.pop();
            }
        }
        self.note(cat, msg);
        self.last_note = Some((kind, self.log.len() - 1));
    }

    /// PageUp/PageDown while the log pane is open: page through history,
    /// clamped to the stored entries (positive = older).
    pub(super) fn scroll_log(&mut self, delta: isize) {
        let visible = self.log.len().min(8);
        let max = self.log.len().saturating_sub(visible) as isize;
        self.log_scroll = (self.log_scroll as isize + delta).clamp(0, max) as usize;
    }

    /// The event-log pane: newest entries at the bottom, one line each,
    /// color-coded by category, mm:ss timestamps since launch. PageUp
    /// pages into history (the title shows how far back); any new
    /// message snaps back to the tail.
    pub(super) fn draw_log(&self, f: &mut Frame, area: Rect) {
        let n = area.height.saturating_sub(2) as usize;
        let scroll = self.log_scroll.min(self.log.len().saturating_sub(n));
        let start = self.log.len().saturating_sub(n + scroll);
        let end = (start + n).min(self.log.len());
        let title = if scroll > 0 {
            format!(" Log ↑{scroll} ")
        } else {
            " Log ".to_string()
        };
        // heading first, on the row the top border used to occupy
        let mut lines: Vec<Line> =
            vec![Line::from(Span::styled(title, Style::default().fg(Color::DarkGray)))];
        for (cat, secs, msg) in &self.log[start..end] {
            lines.push(Line::from(vec![
                Span::styled(
                    // the leading space lines the entries up under the
                    // heading, which the border used to do for both
                    format!(" {:02}:{:02}  ", secs / 60, secs % 60),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(msg.clone(), Style::default().fg(cat.color())),
            ]));
        }
        f.render_widget(Block::default().style(Style::default().bg(log_bg())), area);
        f.render_widget(Paragraph::new(Text::from(lines)), panel_body(area));
    }
}
