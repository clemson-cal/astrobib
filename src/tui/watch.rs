//! Noticing that something outside this session changed the files.

use super::*;

/// Mtime snapshot backing the auto-reload. Three independent parts,
/// because each answers a different question and each triggers a
/// different amount of work.
#[derive(Default, PartialEq)]
pub(super) struct Watch {
    /// Every scanned manuscript source (.tex/.md, expansions included)
    /// with its mtime. An edit here rescans citations, nothing more.
    srcs: Vec<(std::path::PathBuf, std::time::SystemTime)>,
    /// The bib/ directory mtime of each tier, personal first. Directory
    /// granularity: this sees .bib files appear, vanish and get renamed
    /// — a `git pull`, a coauthor's hand-drop, `astrobib add` in another
    /// terminal — but not one edited in place. Per-file mtimes would
    /// catch that too, at a stat per entry per poll, which is a cost
    /// that grows with the library for a case that barely happens.
    bibs: Vec<Option<std::time::SystemTime>>,
    /// Every tag file of every tier, with its own mtime. Per file here,
    /// because appending a key to an existing file *is* the ordinary
    /// way a tag changes and a directory mtime would sleep through it.
    pub(super) tags: Vec<(std::path::PathBuf, std::time::SystemTime)>,
}

impl App {
    /// Mtime snapshot of everything a session reads from disk but does
    /// not own: the manuscript's scanned sources, both tiers' bib/, and
    /// every tag file of both tiers. refs.bib lives in the manuscript
    /// root and is never a scanned source, so regenerating it cannot
    /// re-trigger the watcher.
    pub(super) fn watch_snapshot(&self) -> Watch {
        let mut w = Watch::default();
        let ms_root = self.ms_root();
        if let Some(root) = &ms_root {
            let mut files = crate::export::manuscript_tex_files(root);
            files.extend(crate::export::manuscript_md_files(root));
            w.srcs = files
                .into_iter()
                .filter_map(|f| {
                    let m = std::fs::metadata(&f).and_then(|m| m.modified()).ok()?;
                    Some((f, m))
                })
                .collect();
        }
        let roots = [Some(self.lib.personal.root.clone()), ms_root]
            .into_iter()
            .flatten();
        for root in roots {
            w.bibs.push(
                std::fs::metadata(root.join("bib"))
                    .and_then(|m| m.modified())
                    .ok(),
            );
            w.tags.extend(crate::tags::watch(&root));
        }
        w
    }

    /// Silent auto-reload on external changes, every ~1.5 s.
    ///
    /// A changed bib/ reloads both tiers from disk; a changed tag file
    /// re-reads only tags/, which is far cheaper and all that can have
    /// changed; edited sources rescan the manuscript (refs.bib
    /// regenerates along the way). Unlike the manuscript-only watcher
    /// this replaced, it runs with no manuscript open, because the
    /// personal library is now watched too: a `git pull` in the library
    /// repo, or `astrobib add` in another terminal, used to go unseen
    /// until restart.
    ///
    /// The new snapshot is adopted *before* acting on it. Our own writes
    /// move these mtimes too, and every mutation path ends in
    /// rescan_manuscript or rebuild_order, which refresh the snapshot —
    /// but adopting first is what makes a missed refresh cost one
    /// redundant reload instead of a reload on every poll forever.
    pub(super) fn poll_external(&mut self) {
        if self.watch_at.elapsed() < Duration::from_millis(1500) {
            return;
        }
        self.watch_at = std::time::Instant::now();
        let now = self.watch_snapshot();
        if now == self.watch {
            return;
        }
        let was = std::mem::replace(&mut self.watch, now);
        if self.watch.bibs != was.bibs {
            self.reload_library(); // rebuild_order rescans the manuscript too
            self.report_tags();
        } else if self.watch.tags != was.tags {
            self.lib.reload_tags();
            self.report_tags();
        } else {
            self.rescan_manuscript();
        }
    }

    /// Say what a hand-edited tag file got wrong — and only that.
    ///
    /// A key that resolves to nothing is skipped by design: cite keys
    /// denote papers for life, so a dangling line is far more likely to
    /// be a paper not yet imported than a mistake, and deleting it would
    /// be the destructive behaviour the format exists to avoid. But
    /// skipping it silently means a typo just disappears, so the count
    /// is reported — as information, not an error. Unreadable files are
    /// a real failure and warn.
    ///
    /// Reported only when the wording changes: the watcher re-checks
    /// every 1.5 s and a line per poll is unreadable. Same latch idea as
    /// state_write, one store rather than many.
    pub(super) fn report_tags(&mut self) {
        let mut msgs: Vec<(MsgCat, String)> = vec![];
        let tiers = [Some(&self.lib.personal), self.lib.manuscript.as_ref()];
        for lib in tiers.into_iter().flatten() {
            for (file, why) in lib.tags().errors() {
                msgs.push((MsgCat::Warn, format!("tags/{file} unreadable: {why}")));
            }
        }
        let mut per: Vec<String> = vec![];
        let mut total = 0usize;
        for (name, keys) in self.lib.tags() {
            let n = keys.iter().filter(|k| self.lib.get(k).is_none()).count();
            if n > 0 {
                total += n;
                per.push(format!("{name}: {n}"));
            }
        }
        if total > 0 {
            // the footer shows one line, so name a few tags and count
            // the rest rather than running off the end
            const SHOWN: usize = 3;
            let rest = per.len().saturating_sub(SHOWN);
            per.truncate(SHOWN);
            if rest > 0 {
                per.push(format!("+{rest} more"));
            }
            msgs.push((
                MsgCat::Info,
                format!("tags: {total} key(s) not in the library ({})", per.join(", ")),
            ));
        }
        let said: Vec<String> = msgs.iter().map(|(_, m)| m.clone()).collect();
        if said == self.tags_said {
            return;
        }
        self.tags_said = said;
        for (cat, m) in msgs {
            self.note(cat, m);
        }
    }

    /// Reload both tiers from disk after an external change to either
    /// bib/ (a git pull, a hand-dropped .bib, an add from another
    /// terminal, …), preserving the two-tier switch and UI state;
    /// rebuild_order re-derives everything display-side.
    fn reload_library(&mut self) {
        match MergedLibrary::load(self.ms_root().as_deref()) {
            Ok(mut lib) => {
                lib.global_on = self.lib.global_on;
                self.lib = lib;
                self.rebuild_order();
            }
            Err(e) => {
                // keep the stale library; the snapshot was already
                // adopted, so a persistent error can't warn every poll
                self.note(MsgCat::Warn, format!("library reload failed: {e}"));
            }
        }
    }
}
