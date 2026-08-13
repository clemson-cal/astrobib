//! Keys, pastes, and the line-editing chords the prompts share.

use super::*;

/// option/alt+arrow (and emacs alt+b/f) word motions for text inputs.
/// tui-input's crossterm backend only maps ctrl+arrow and meta+b/f, and
/// macOS terminals report option as ALT — so these arrive unmapped.
fn word_motion(code: KeyCode, mods: KeyModifiers) -> Option<tui_input::InputRequest> {
    if !mods.contains(KeyModifiers::ALT) {
        return None;
    }
    match code {
        KeyCode::Left | KeyCode::Char('b') => Some(tui_input::InputRequest::GoToPrevWord),
        KeyCode::Right | KeyCode::Char('f') => Some(tui_input::InputRequest::GoToNextWord),
        _ => None,
    }
}

impl App {
    /// The text the active mode is composing, if it is composing any.
    fn active_input_mut(&mut self) -> Option<&mut tui_input::Input> {
        match &mut self.mode {
            Mode::Filter => Some(&mut self.filter),
            Mode::AdsPrompt { input, .. }
            | Mode::Setup { input, .. }
            | Mode::Export { input, .. }
            | Mode::Tag { input, .. }
            | Mode::Rename { input } => Some(input),
            _ => None,
        }
    }

    /// Replace the active input's text, keeping the cursor where the
    /// caller puts it (char index, as tui-input counts).
    fn set_input(&mut self, value: String, cursor: usize) {
        if let Some(input) = self.active_input_mut() {
            *input = tui_input::Input::new(value).with_cursor(cursor);
        }
    }

    /// The chords shared by every prompt: emacs kill/yank, and copying
    /// what is being composed. Returns whether the key was claimed.
    ///
    /// `⌃k` used to reach tui-input, which implements it as a plain
    /// delete — so a killed tail was simply gone, and `⌃y` had nothing
    /// to yank because there was no kill ring to yank from.
    fn prompt_chord(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        let ctrl = mods.contains(KeyModifiers::CONTROL);
        let alt = mods.contains(KeyModifiers::ALT);
        let Some(input) = self.active_input_mut() else {
            return false;
        };
        let (value, cursor) = (input.value().to_string(), input.cursor());
        // char indices, since the cursor is one and the text is not ASCII
        let split = |n: usize| {
            let b = value.char_indices().nth(n).map(|(i, _)| i).unwrap_or(value.len());
            (value[..b].to_string(), value[b..].to_string())
        };
        match code {
            KeyCode::Char('k') if ctrl => {
                let (head, tail) = split(cursor);
                if tail.is_empty() {
                    return true; // nothing to kill; do not clobber the ring
                }
                self.kill_ring = tail;
                self.set_input(head, cursor);
                true
            }
            // ⌃u kills to the start of the line and ⌃w the word before
            // the cursor. tui-input performs both, but as deletions —
            // routing them through the ring is what makes ⌃y able to
            // undo any of the three rather than only ⌃k.
            KeyCode::Char('u') if ctrl => {
                let (head, tail) = split(cursor);
                if head.is_empty() {
                    return true;
                }
                self.kill_ring = head;
                self.set_input(tail, 0);
                true
            }
            KeyCode::Char('w') if ctrl => {
                let (head, tail) = split(cursor);
                // back over any run of spaces, then over the word itself
                let kept = head.trim_end();
                let start = kept
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace())
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(0);
                if start == head.len() {
                    return true;
                }
                self.kill_ring = head[start..].to_string();
                let kept = head[..start].to_string();
                let n = kept.chars().count();
                self.set_input(format!("{kept}{tail}"), n);
                true
            }
            KeyCode::Char('y') if ctrl => {
                if self.kill_ring.is_empty() {
                    self.note(MsgCat::Warn, "nothing killed to yank".to_string());
                    return true;
                }
                let (head, tail) = split(cursor);
                let n = self.kill_ring.chars().count();
                let text = format!("{head}{}{tail}", self.kill_ring);
                self.set_input(text, cursor + n);
                true
            }
            // ⌥w, emacs' copy. On an ADS query it copies the search URL,
            // which is the only form that carries the result limit and
            // what ADS returns as well — neither is query syntax, and
            // Solr has no comment to smuggle them into. Pasting one back
            // restores all three.
            KeyCode::Char('w') if alt => {
                let text = match &self.mode {
                    Mode::AdsPrompt { input, limit, sort, .. } => {
                        crate::ads::search_url(input.value(), *limit, sort)
                    }
                    _ => value,
                };
                if text.is_empty() {
                    self.note(MsgCat::Warn, "nothing to copy".to_string());
                } else {
                    self.finish_copy(&text);
                }
                true
            }
            _ => false,
        }
    }

    /// A bracketed paste. In a prompt the text is inserted at the
    /// cursor; an ADS search URL instead *configures* the query, which
    /// is what makes ⌥w reversible — copy a query anywhere, paste it
    /// back, and the text, the limit and the returns mode all come with
    /// it. Pasted onto the table with no prompt up, it opens one.
    pub(super) fn on_paste(&mut self, text: String) {
        let parsed = crate::ads::parse_search_url(&text);
        if let Some((q, rows, so)) = parsed {
            if matches!(self.mode, Mode::Normal) {
                self.open_ads_prompt();
            }
            if let Mode::AdsPrompt { input, limit, sort, .. } = &mut self.mode {
                *input = tui_input::Input::from(q);
                let mut said = String::from("query pasted");
                if let Some(r) = rows {
                    *limit = r.clamp(1, 2000);
                    said.push_str(&format!(" · {} results", *limit));
                }
                // an unknown sort would leave the prompt naming a mode it
                // is not in, since the label is looked up by value
                // ADS silently drops a sort field it does not know, so
                // passing one through would leave the prompt naming an
                // order the query is not actually getting
                if let Some(s) = so {
                    let field = s.split_once(' ').map(|(f, _)| f).unwrap_or(&s);
                    if ADS_SORTS.iter().any(|(f, ..)| *f == field) {
                        *sort = s.clone();
                        said.push_str(&format!(" · {}", ads_sort_name(&s)));
                    } else {
                        said.push_str(" · (unknown sort ignored)");
                    }
                }
                self.note(MsgCat::Ok, said);
                return;
            }
        }
        if self.active_input_mut().is_some() {
            let one_line: String =
                text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
            let input = self.active_input_mut().unwrap();
            let (value, cursor) = (input.value().to_string(), input.cursor());
            let b = value.char_indices().nth(cursor).map(|(i, _)| i).unwrap_or(value.len());
            let n = one_line.chars().count();
            let text = format!("{}{one_line}{}", &value[..b], &value[b..]);
            self.set_input(text, cursor + n);
            if matches!(self.mode, Mode::Filter) {
                self.refilter();
            }
        }
    }

    pub(super) fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.show_about {
            self.show_about = false; // any key dismisses the about modal
            return;
        }
        // while the columns panel holds focus it takes the navigation
        // keys; anything it does not claim falls through to the table,
        // so q, ?, D and the rest keep working with the panel open
        if matches!(self.mode, Mode::Normal)
            && self.show_columns
            && self.focus == Focus::Columns
            && self.columns_panel_key(code)
        {
            return;
        }
        // the ADS-returns menu owns the keyboard while it is open, and
        // is claimed before the editing chords so a field key cannot be
        // mistaken for one
        if self.sort_menu && self.sort_menu_key(code, mods) {
            return;
        }
        // the editing chords every prompt shares, claimed before the
        // per-mode arms so each one does not have to repeat them
        if self.prompt_chord(code, mods) {
            return;
        }
        match &mut self.mode {
            Mode::Filter => match code {
                KeyCode::Esc => {
                    self.filter = tui_input::Input::default();
                    self.mode = Mode::Normal;
                    self.refilter();
                }
                KeyCode::Enter => self.mode = Mode::Normal,
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        self.filter.handle(req);
                        return;
                    }
                    let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                    if self.filter.handle_event(&ev).is_some() {
                        self.refilter();
                    }
                }
            },
            Mode::Pick { key, files, sel } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Char('j') | KeyCode::Down => {
                    *sel = (*sel + 1).min(files.len().saturating_sub(1))
                }
                KeyCode::Char('k') | KeyCode::Up => *sel = sel.saturating_sub(1),
                KeyCode::Enter => {
                    let (key, file) = (key.clone(), files[*sel].clone());
                    self.mode = Mode::Normal;
                    self.import_picked(&key, &file);
                }
                _ => {}
            },
            Mode::Copy => {
                let item = match code {
                    KeyCode::Char(c) => {
                        COPY_CHORD.iter().find(|(k, ..)| *k == c).map(|(.., i)| *i)
                    }
                    _ => None,
                };
                match item {
                    Some(item) => self.do_copy(item),
                    None => {
                        self.exit_copy_mode();
                        self.note(MsgCat::Warn, "copy cancelled".to_string());
                    }
                }
            }
            Mode::Rename { input } => match code {
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.note(MsgCat::Info, "rename cancelled".to_string());
                }
                KeyCode::Enter => {
                    let name = input.value().trim().to_string();
                    // the prompt closes first: it occupies the footer, so
                    // a confirmation raised while it is up would be drawn
                    // over by the prompt itself and never seen
                    self.mode = Mode::Normal;
                    self.rename_query(name);
                }
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        input.handle(req);
                        return;
                    }
                    let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                    input.handle_event(&ev);
                }
            },
            Mode::Tag { input, keys, remove } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let (name, keys, remove) =
                        (input.value().trim().to_string(), keys.clone(), *remove);
                    // the prompt owns the footer; close it first or the
                    // confirmation is drawn over and never seen
                    self.mode = Mode::Normal;
                    if !name.is_empty() {
                        self.do_tag(&name, &keys, remove);
                    }
                }
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        input.handle(req);
                    } else {
                        let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                        input.handle_event(&ev);
                    }
                    // which way ⏎ goes depends on the name typed so far
                    self.retarget_tag();
                }
            },
            Mode::Export { input, keys } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let (path, keys) = (input.value().trim().to_string(), keys.clone());
                    self.mode = Mode::Normal;
                    if !path.is_empty() {
                        self.do_export(&path, &keys);
                    }
                }
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        input.handle(req);
                        return;
                    }
                    let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                    input.handle_event(&ev);
                }
            },
            Mode::Setup { input, email, resume } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let (v, was_email, resume) = (input.value().trim().to_string(), *email, *resume);
                    if !was_email {
                        if v.is_empty() {
                            return; // a token is the point; Esc cancels
                        }
                        if let Err(e) = crate::ads::save_state_field("ads_token", &v) {
                            self.mode = Mode::Normal;
                            self.note(MsgCat::Err, format!("could not save token: {e}"));
                            return;
                        }
                        self.note(MsgCat::Ok, "ADS token saved".to_string());
                        if crate::ads::get_email().is_none() {
                            self.mode = Mode::Setup {
                                input: tui_input::Input::default(),
                                email: true,
                                resume,
                            };
                            return;
                        }
                    } else if !v.is_empty() {
                        if let Err(e) = crate::ads::save_state_field("email", &v) {
                            self.note(MsgCat::Err, format!("could not save email: {e}"));
                        } else {
                            self.note(MsgCat::Ok, "email saved".to_string());
                        }
                    }
                    self.mode = Mode::Normal;
                    if resume {
                        self.open_ads_prompt();
                    }
                }
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        input.handle(req);
                        return;
                    }
                    let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                    input.handle_event(&ev);
                }
            },
            Mode::AdsPrompt { input, limit, sort, edit } => match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    let (q, l, so, ed) =
                        (input.value().to_string(), *limit, sort.clone(), *edit);
                    self.mode = Mode::Normal;
                    self.run_ads_query_limit(q, ed, l, Some(so));
                }
                // ⌃r opens the menu of everything ADS will sort by. A
                // chord, so it cannot be confused with typing, and ⌃s /
                // ⌃q are avoided because terminals still eat those as
                // flow control. It cycled four modes until there were
                // twenty, which is more than a cycle can carry.
                KeyCode::Char('r') if mods.contains(KeyModifiers::CONTROL) => {
                    self.open_sort_menu();
                }
                KeyCode::Up => {
                    const STEPS: [usize; 4] = [20, 50, 100, 200];
                    let i = STEPS.iter().position(|&s| s >= *limit).unwrap_or(0);
                    *limit = STEPS[(i + 1).min(STEPS.len() - 1)];
                }
                KeyCode::Down => {
                    const STEPS: [usize; 4] = [20, 50, 100, 200];
                    let i = STEPS.iter().position(|&s| s >= *limit).unwrap_or(0);
                    *limit = STEPS[i.saturating_sub(1)];
                }
                _ => {
                    use tui_input::backend::crossterm::EventHandler;
                    if let Some(req) = word_motion(code, mods) {
                        input.handle(req);
                        return;
                    }
                    let ev = Event::Key(ratatui::crossterm::event::KeyEvent::new(code, mods));
                    input.handle_event(&ev);
                }
            },
            Mode::Confirm { plan } => match code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let plan = plan.clone();
                    self.mode = Mode::Normal;
                    self.remove_confirmed(&plan);
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.mode = Mode::Normal;
                    self.note(MsgCat::Warn, "removal cancelled".to_string());
                }
                _ => {}
            },
            Mode::Normal => match code {
                // plain-letter bindings must not fire on ctrl/alt chords
                // (ctrl+a once triggered select-all); ctrl+w is the one
                // deliberate chord below
                // ctrl+p was the old control-panel chord; keep it as a
                // cheat-sheet alias for muscle memory
                KeyCode::Char('p') if mods.contains(KeyModifiers::CONTROL) => {
                    self.run_action(Action::Help)
                }
                KeyCode::Char(c)
                    if mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && !(c == 'w' && mods.contains(KeyModifiers::CONTROL)) =>
                {
                    // ignored
                }
                KeyCode::Char('q') => self.run_action(Action::Quit),
                KeyCode::Char('/') => self.run_action(Action::Filter),
                KeyCode::Char('m') => self.run_action(Action::Manuscript),
                KeyCode::Char('T') => self.run_action(Action::Tag),
                KeyCode::Delete | KeyCode::Backspace => self.run_action(Action::Remove),
                KeyCode::Char('p') => self.run_action(Action::Download),
                KeyCode::Char('o') => self.run_action(Action::OpenPdf),
                KeyCode::Char('X') => self.run_action(Action::ClearPdf),
                KeyCode::Char('B') => self.run_action(Action::BrowserDl),
                KeyCode::Char('D') | KeyCode::Char('z') => self.run_action(Action::Card),
                KeyCode::Char('?') => self.run_action(Action::Help),
                KeyCode::Char('|') => self.run_action(Action::Columns),
                KeyCode::Char('N') => self.open_rename_prompt(),
                KeyCode::Char('E') => self.open_edit_query_prompt(),
                KeyCode::Char('H') => self.run_action(Action::QueryHome),
                // the other half of the focus toggle; with no panel open
                // there is only one thing the arrows could drive
                KeyCode::Tab | KeyCode::BackTab if self.show_columns => {
                    self.focus = Focus::Columns
                }
                KeyCode::Char('t') => self.run_action(Action::GlobalTier),
                KeyCode::Char('v') => self.show_bib_source = !self.show_bib_source,
                KeyCode::Char('e') => self.open_export_prompt(),
                KeyCode::Char('M') => {
                    self.metric_col = self.metric_col.next();
                    if self.sort().is_some_and(|(c, _)| c == Col::Metric) {
                        // the sort is by "the metric", so switching which
                        // metric that is has to reorder the rows
                        self.apply_sort();
                    }
                    let res =
                        crate::ads::save_state_field("metric", self.metric_col.state_tag());
                    self.state_write("state.json", res.err().map(|e| e.to_string()));
                    self.note_latest(
                        MsgCat::Info,
                        "metric",
                        format!("metric column: {}", self.metric_col.name()),
                    );
                }
                KeyCode::Char('.') => self.adjust_priority(PriorityOp::Set(1.0)),
                KeyCode::Char('0') => self.adjust_priority(PriorityOp::Set(0.0)),
                KeyCode::Char('>') => self.adjust_priority(PriorityOp::Scale(1.25)),
                KeyCode::Char('<') => self.adjust_priority(PriorityOp::Scale(0.8)),
                KeyCode::Char('@') => self.show_about = true,
                KeyCode::Char('C') => self.spawn_citation_query(false),
                KeyCode::Char('R') => self.spawn_citation_query(true),
                KeyCode::Char('a') => self.select_all(true),
                KeyCode::Char('A') => self.select_all(false),
                KeyCode::Char('S') => self.open_ads_prompt(),
                KeyCode::Char('P') => self.paste_query_config(),
                KeyCode::Char('[') => self.cycle_scope(-1),
                KeyCode::Char(']') => self.cycle_scope(1),
                KeyCode::Char('r') => self.refresh_scope(),
                KeyCode::Char('+') | KeyCode::Char('=') => self.step_limit(1),
                KeyCode::Char('-') => self.step_limit(-1),
                KeyCode::Char('w') if mods.contains(KeyModifiers::CONTROL) => {
                    // through the action rather than straight to
                    // close_scope: on the library or the manuscript it
                    // has nothing to close, and a key that does nothing
                    // silently is the thing the keys panel exists to
                    // stop being a mystery
                    self.run_action(Action::CloseScope)
                }
                KeyCode::Char('i') => self.import_highlighted(false),
                // I — the same import, plus the share the paper would
                // otherwise need a second gesture (s) for
                KeyCode::Char('I') => self.import_highlighted(true),
                KeyCode::Char('s') => self.run_action(Action::Share),
                KeyCode::Char('L') => self.run_action(Action::Log),
                KeyCode::Char('y') => self.run_action(Action::Copy),
                KeyCode::Char('Y') => self.do_copy(CopyItem::FullKey),
                KeyCode::Char(' ') => self.run_action(Action::Select),
                KeyCode::Esc => {
                    if self.show_help {
                        self.show_help = false;
                    } else if self.select_mode {
                        self.exit_select_mode();
                    } else if !self.filter.value().is_empty() {
                        self.filter = tui_input::Input::default();
                        self.refilter();
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
                KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
                KeyCode::Char('g') | KeyCode::Home => {
                    self.table.select((self.row_count() > 0).then_some(0))
                }
                KeyCode::Char('G') | KeyCode::End => {
                    self.table.select(self.row_count().checked_sub(1))
                }
                KeyCode::PageDown => {
                    if self.show_log {
                        let page = self.log.len().min(8) as isize;
                        self.scroll_log(-page);
                    } else {
                        self.move_sel(20);
                    }
                }
                KeyCode::PageUp => {
                    if self.show_log {
                        let page = self.log.len().min(8) as isize;
                        self.scroll_log(page);
                    } else {
                        self.move_sel(-20);
                    }
                }
                _ => {}
            },
        }
    }
}
