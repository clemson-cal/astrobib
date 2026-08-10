# Plan: collapse the TUI's twenty rect caches into one frame-scoped registry

**Status: complete (2026-08-10).** The TUI now builds a fresh `Hits` registry
per frame, dispatches clicks and wheel targets through it, and keeps only the
persistent `last_table_area` geometry needed by non-draw handlers. The
headless TUI suite, Rust tests, and clippy are green.

*Written 2026-08-09, after the 0.17.1 module split. Implemented 2026-08-10.*

The 0.17.1 refactor broke `impl App` into nineteen modules but left `App`'s 91 fields untouched, and one finding came out of looking at them afterwards: **twenty of those fields are the same thing wearing twenty masks.** This is the plan for fixing that. It is a behaviour-bearing change, unlike the pure movement in 0.17.1, so the verification comes first.

---

## What is wrong

Twenty fields hold `Rect`s that were registered while drawing, so that a later click or hover can be resolved against them:

```
about_btn          card_links         edit_query_rect    scope_rects
about_links        card_tags          footer_badges      sort_headers
card_area          card_yanks         help_rects         sort_menu_rects
card_buttons       col_rects          metric_area        table_area
confirm_btns       pick_area          prompt_sort_rect   sample_rects
```

They share one invariant: **a rect must not outlive the frame that drew it.** A surface that is no longer on screen must stop answering clicks, or a click lands on a control that is not there.

Nothing in the types enforces that. It is maintained by hand, in three overlapping mechanisms:

1. **`draw()` carries an else-branch per surface whose only job is forgetting.** `else { self.col_rects.clear() }`, `else { self.card_yanks.clear() }`, `else { self.confirm_btns.clear() }`, `else { self.pick_area = Rect::default() }`, and the `sort_menu` / `samples` pair that clear each other. Five of them, in `src/tui/mod.rs`.
2. **Each draw function also clears its own on entry.** Seven fields are cleared in two different files: `card_buttons`, `card_tags` and `card_yanks` (`card.rs` *and* `mod.rs`), `col_rects` (`columns.rs` and `mod.rs`), `sample_rects` and `sort_menu_rects` (`search.rs` and `mod.rs`), `confirm_btns` (`overlays.rs` and `mod.rs`).
3. **`on_click` re-checks a visibility flag anyway**, for some of them: `self.show_columns && self.col_rects.iter()...`, `if self.show_help { ... }`, `if let Mode::Confirm { .. } = &self.mode { ... }`.

The three are not interchangeable, and which one a surface relies on is not visible from the field. `help_rects` has no else-branch in `draw()` at all — when the keys panel closes it keeps the previous frame's rects, and what saves it is mechanism 3. Meanwhile **nine surfaces have no guard in `on_click` at all** and rest entirely on the clearing discipline being right: `sample_rects`, `card_yanks`, `card_links`, `card_tags`, `card_buttons`, `scope_rects`, `edit_query_rect`, `footer_badges`, `sort_headers`.

The cost is not the field count. It is that **`on_click` is 322 lines — the largest method in the tree — because it hand-scans twenty lists in priority order**, and that priority order is the z-order, written nowhere but in the sequence of its `if`s.

### One of the twenty is not like the others

`table_area` is **persistent geometry, not a hit target.** `src/tui/columns.rs` reads `self.table_area.width` from `toggle_column`, `nudge_width` and `panel_rows` — event handlers that run outside `draw`, on the width the last frame solved. It must survive between frames and must stay a plain field. Rename it `last_table_area` so the next reader does not have to work that out.

Every other field's only non-mouse mentions are writes (`&mut self.card_buttons` handed to a draw helper, `sort_headers.extend`, `mem::take(&mut self.about_links)`). So the registry takes **19 fields**, not 20.

---

## The target

One value, built fresh each frame, holding `(Rect, Target)` pairs:

```rust
/// What a click or a hover at some cell would reach. Built during
/// draw and installed at the end of it, so a surface that did not
/// draw this frame registers nothing and cannot be clicked.
enum Target {
    AboutLink(String), AboutUpdate, CardButton(CardBtn), CardLink(String),
    CardTag(String), CardYank(CopyItem), Column(PanelHit), ConfirmBtn(bool),
    EditQuery, FooterBadge(Action), HelpRow(KeyCode), PickRow,
    PromptSort, Sample(&'static str), Scope(usize), SortHeader(Col),
    SortMenuRow(usize), /* … */
}

#[derive(Default)]
struct Hits(Vec<(Rect, Target)>);

impl Hits {
    fn at(&mut self, r: Rect, t: Target) { self.0.push((r, t)); }
    /// Last registration wins: draw order is z-order, and modals draw last.
    fn test(&self, x: u16, y: u16) -> Option<&Target> {
        self.0.iter().rev().find(|(r, _)| hit(*r, x, y)).map(|(_, t)| t)
    }
}
```

`draw` builds a local `Hits`, hands `&mut` down through the draw calls, and does `self.hits = hits` at the end. Staleness stops being a discipline and becomes unrepresentable. `on_click` becomes `match self.hits.test(x, y)`, and the nine unguarded surfaces need no guard because there is nothing to guard.

**Last-registration-wins is the whole z-order rule**, and it is already true of the drawing: `draw()` deliberately draws modals last so they paint over everything. Reversing the scan makes the click order agree with the paint order by construction, instead of by a hand-maintained sequence of `if`s that has to be kept parallel to it.

---

## Steps

Each step ends green — `cargo test`, `cargo clippy` (currently zero warnings; keep it there), and `tests/tui/run.py` (42 scenarios, ~12s).

**1. Pin the current behaviour first.** This is the step that must not be skipped. `s26_table_chrome` is a *rendering* oracle and will not catch a click regression. Write click-path scenarios covering, at minimum:
   - each modal swallowing clicks meant for what is behind it (pick, about, confirm);
   - a click landing on the table where a closed panel used to be — the staleness bug this whole change exists to make impossible, one scenario per surface that closes (columns panel, keys sheet, card, sort menu, samples);
   - the card's four click kinds (yank, link, tag, button) which currently resolve in that order;
   - the scope strip, the footer badges, `edit query`, and a sort header.
   Commit these on their own, against the *current* implementation, and watch them pass. They are the contract.

**2. Record the z-order.** The existing sequence, top to bottom, is: pick modal → about → prompt sort control → sort-menu rows → sample rows → confirm modal → card yanks → card links → help rows → card tags → card buttons → scope strip → edit-query → footer badges → columns panel → sort headers → table rows. Under last-wins this becomes registration order, i.e. the reverse. Check it against `draw`'s actual call order before trusting it — where the two disagree, the current click order is what ships, so it wins, and the disagreement is worth a comment.

**3. Introduce `Hits` alongside the existing fields.** Register into both, assert nothing changes, keep `on_click` reading the old fields. A no-op commit that only adds.

**4. Move `on_click` onto `Hits.test()`, one surface at a time**, deleting each old field as its last reader goes. Nineteen small commits or a few grouped ones; the scenarios from step 1 gate every move.

**5. Delete the clearing machinery.** All five else-branches in `draw`, all the duplicate `.clear()` calls, and the now-redundant `show_help` / `show_columns` / mode guards in `on_click`. This is where the win actually lands.

**6. Rename `table_area` to `last_table_area`** and give it the comment explaining why it is the one that persists.

**7. Then reconsider `on_mouse`.** Hover uses the same rects for roll-over styling and footer hints (`hover_hint`, and the `*_hint` family across the modules). It should fall out of the registry nearly free, but it is a second pass, not part of this one.

---

## Traps

- **The modal hover-blinding.** `draw()` sets `self.hover = (u16::MAX, u16::MAX)` while a covering modal is up, draws the surfaces beneath it blind, then restores the real position before drawing the modal itself — so the modal's own links still react while the ones behind it do not. `s13_about_hover` covers this. A registry changes *when* rects are recorded, so re-read that block before touching it; the blinding may become unnecessary (a rect under a modal simply loses the test) but do not assume it — the hover *styling* is decided during draw, not at test time, which is exactly why the trick exists.
- **`card_toggle`** is a same-frame signal, not a rect: the card sets it while drawing to tell the footer whether the ⇄ toggler belongs there, and `draw` clears it at the top of every frame. It is the same class of problem and should move into the same frame-scoped value, but it is not a click target.
- **Double-click** state (`last_click: Option<(Instant, usize, usize)>`) is keyed on scope and row position and must survive across frames. Leave it alone.
- **`scroll_swatch`** hit-tests `metric_area` on the wheel path, not the click path. It needs the registry too, or `metric_area` stays.
- **The table itself is not in the registry.** Row hit-testing is arithmetic on `table_area` plus the scroll offset, not a rect per row, and should stay that way — one rect per visible row would be a regression in both allocation and clarity.

---

## Also worth doing, but separately

Two smaller findings from the same read of the struct, both independent of the above:

- **Five `Option<Receiver<T>>`** — `dl_rx`, `upd_rx`, `bib_rx`, `cit_rx`, `ads_rx` — each with a matching `drain_*` that is the same shape. Real duplication, but the payload types differ, so unifying costs an enum and buys less than it looks like it will. Low priority.
- **Four fields for one overlay**: `sort_menu`, `sort_menu_rects`, `sort_menu_sel`, `sort_menu_primary` are only meaningful together, and `Option<SortMenu>` would make the closed state unrepresentable rather than merely ignored. The picker and the confirm modal have the same shape. Cheap, low risk, and can be done any time — but do it *after* the registry, which removes `sort_menu_rects` from the group anyway.

---

## Done looks like

`App` around 70 fields instead of 91; `on_click` well under half its 322 lines and reading as a `match` on what was clicked; no `.clear()` of a rect cache anywhere; z-order stated once, in `draw`'s call order, instead of twice in two files that have to be kept parallel. Zero clippy warnings, 42+ scenarios green, and `s26`'s golden screens unchanged — the pixels must not move.
