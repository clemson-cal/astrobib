//! Tagging a selection, and following a tag back to its papers.

use super::*;

/// What a hovered tag on the pub card says it will do. It names the
/// library, because the filter is the library's and a click from a query
/// scope goes there; and it says "replaces", because it does — what it
/// lands on is the whole filter, not a term added to what was there.
pub(super) fn tag_hint(name: &str) -> String {
    format!("⌕ filter the library to {}  ·  replaces the current filter", App::tag_term(name))
}

impl App {
    /// T — name a tag for the selection (or the cursor entry).
    pub(super) fn open_tag_prompt(&mut self) {
        let keys = self.action_keys();
        if keys.is_empty() {
            self.note(MsgCat::Warn, self.unavailable_reason(Action::Tag));
            return;
        }
        self.mode = Mode::Tag { input: tui_input::Input::default(), keys, remove: false };
        self.retarget_tag();
    }

    /// Decide which way ⏎ will go, from the name as it stands: an untag
    /// only when every target already carries the tag. Same ± reading
    /// as `m`, and recomputed on each keystroke because the answer is a
    /// property of the name being typed.
    pub(super) fn retarget_tag(&mut self) {
        let Mode::Tag { input, keys, .. } = &self.mode else { return };
        let name = input.value().trim().to_string();
        let all = !name.is_empty() && keys.iter().all(|k| self.lib.has_tag(&name, k));
        if let Mode::Tag { remove, .. } = &mut self.mode {
            *remove = all;
        }
    }

    /// Apply the tag prompt. Adding writes to the tier astrobib is
    /// pointed at; removing takes the key out of every active tier that
    /// lists it, because leaving one behind would look like a no-op.
    pub(super) fn do_tag(&mut self, name: &str, keys: &[String], remove: bool) {
        if let Some(why) = crate::tags::bad_name(name) {
            self.note(MsgCat::Warn, why.to_string());
            return;
        }
        let n = keys.len();
        let papers = if n == 1 { "paper" } else { "papers" };
        if remove {
            match self.lib.untag(name, keys) {
                Ok(tiers) if tiers.is_empty() => {
                    self.note(MsgCat::Warn, format!("no {papers} here carry {name}"))
                }
                Ok(tiers) => self.note(
                    MsgCat::Ok,
                    format!("untagged {n} {papers} — {name} ({})", tiers.join(", ")),
                ),
                Err(e) => self.note(MsgCat::Err, format!("could not write tags/{name}: {e}")),
            }
        } else {
            match self.lib.tag(name, keys) {
                Ok(tier) => self.note(
                    MsgCat::Ok,
                    format!("tagged {n} {papers} — {name} ({tier})"),
                ),
                Err(e) => self.note(MsgCat::Err, format!("could not write tags/{name}: {e}")),
            }
        }
        // the tag files just moved under the watcher's feet; adopt the
        // new snapshot so the poll does not report our own write back
        self.watch = self.watch_snapshot();
        self.tags_said.clear();
        self.report_tags();
        self.refilter(); // a tag: filter in force must follow the change
    }

    /// The tags already in the database, for the band under the prompt.
    /// Typing a fresh name is how a tag is born, so the list is an aid
    /// rather than a constraint — but a tag mistyped into existence is
    /// the failure worth designing against.
    pub(super) fn known_tags(&self) -> Vec<String> {
        self.lib.tags().into_keys().collect()
    }

    /// The filter term that selects one tag — quoted when the name has
    /// whitespace in it, which a tag name may: it is only a filename. A
    /// name containing a double quote cannot be written as one term and
    /// is left unquoted rather than escaped, the filter language having
    /// no escape to use.
    pub(super) fn tag_term(name: &str) -> String {
        if name.contains(char::is_whitespace) && !name.contains('"') {
            format!("tag:\"{name}\"")
        } else {
            format!("tag:{name}")
        }
    }

    /// Click a tag on the pub card: filter the library to it.
    ///
    /// The library, whichever scope the card was read from. The filter
    /// cuts the library's rows and nothing else — a manuscript and a
    /// query carry their own row lists — so applying it in place from a
    /// query scope would change a count nothing on screen was showing.
    /// The hover hint names the destination for that reason.
    ///
    /// Replace, not narrow. A tag is a coarse cut through the library and
    /// the thing you do after following one is follow another, so anding
    /// each onto what was already typed would mostly land on nothing —
    /// and what it replaced is one keystroke away, since the strip chip
    /// reopens the prompt with the text still in it.
    pub(super) fn filter_by_tag(&mut self, name: &str) {
        let text = Self::tag_term(name);
        self.filter = tui_input::Input::from(text.clone());
        if self.active_scope != 0 {
            self.set_scope(0);
        }
        self.refilter();
        let n = self.row_count();
        let total = self.row_total();
        self.note(MsgCat::Ok, format!("filtered to {text} — {n} of {total}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tag name is only a filename, so it may hold the characters the
    /// filter language reads as structure. What the card's tag links
    /// build has to survive a round trip through `tokenize`, or clicking
    /// a two-word tag would filter by its first word and a bare term.
    #[test]
    fn a_tag_term_round_trips_through_the_filter_language() {
        for name in ["section-3", "to read", "intro lit review 2019"] {
            let groups = query::tokenize(&App::tag_term(name));
            assert_eq!(groups.len(), 1, "{name:?} is one group");
            assert_eq!(groups[0].len(), 1, "{name:?} is one term: {:?}", groups[0]);
            let term = &groups[0][0];
            assert_eq!(term.field, Some(query::Field::Tag), "{name:?}");
            assert_eq!(term.value, name, "{name:?}");
            assert!(!term.neg, "{name:?}");
        }
        // and the hint quotes what it will do, so the two cannot drift
        assert!(tag_hint("to read").contains("tag:\"to read\""));
    }
}
