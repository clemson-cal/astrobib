# TUI integration harness

Drives the real `target/release/astrobib` binary in a pseudo-terminal and asserts against the screen reconstructed with [pyte](https://github.com/selectel/pyte) — never against raw output bytes.

## Run

```bash
tests/tui/run.py            # builds the binary, then runs every scenario
tests/tui/run.py --no-build # reuse the existing target/release/astrobib
tests/tui/run.py -k sort    # only scenarios whose filename matches
tests/tui/run.py --list     # show the scenario roster
```

On first use `run.py` creates `tests/tui/.venv` (any python3 works), installs pyte into it, and re-execs itself — no manual setup. Exit status is nonzero if any scenario fails; failures print the reconstructed screen.

Network scenarios (ADS search) are skipped unless `RUN_ADS_TESTS=1` is set; the ADS scenario additionally needs `ADS_API_TOKEN` in the environment.

## Sandboxing

Each scenario gets a fresh scratch directory. `HOME`, `ASTROBIB_LIBRARY`, and `ASTROBIB_STATE_DIR` all point inside it, so the app can never read or write the real library, `state.json`, `tabs.json`, the PDF cache, or `~/Downloads`. The scratch library is populated from `fixtures/*.bib` — five synthetic entries written by hand in the on-disk format (FIELD_ORDER section first, trailing lowercase fields after), with correct `AuthorYYYY` + hash cite keys computed from their synthetic arXiv IDs / bibcodes. `ASTROBIB_ASCII=1` keeps pill rendering out of the Nerd Font private-use range.

## Writing scenarios

A scenario is `scenarios/sNN_name.py` exposing `DESCRIPTION`, optionally `ALLOW_NETWORK = True` (passes `ADS_API_TOKEN` through), and `run(t)` where `t` is a started, ready `driver.Session`:

- `t.send(str)` / `t.key("esc"|"enter"|"delete"|"space"|…)` — keyboard input
- `t.click(x, y)` — left click at a 0-based cell, injected as SGR mouse reports
- `t.lines()` / `t.text()` / `t.find(needle)` / `t.row_of(needle)` — reconstructed screen
- `t.wait_for(substring_or_predicate, timeout=8)` / `t.wait_gone(needle)` — polling waits; there are no sleeps in scenarios
- `driver.require(cond, msg, t)` — assert with a screen dump attached
- raise `driver.Skip("why")` to skip

Gotchas learned the hard way:

- Long titles are truncated in the table (~40 visible chars) and reflowed with different line breaks in the pub card, so a title fragment can match either region or neither. Anchor row positions with table-only needles such as author cells (`"Cabrera, +1"`) or short keys.
- The gutter (two leftmost cells of a table row) is a click target for selection mode; data rows start two rows below the header line (header, rule, rows).
- Never click a copy target in a scenario: clipboard writes go through `pbcopy`, which is the user's real pasteboard — the pty sandbox does not contain it. Open the copy modal, assert, Esc out.
- ratatui diff-renders, so the pty stream alone is meaningless; only the pyte screen state is trustworthy. `wait_for` pumps the stream before every predicate check.

## What this harness structurally cannot catch: glyph width

ratatui lays a frame out by asking the `unicode-width` crate how wide each character is, and pyte reconstructs the screen with the same table. App and harness therefore always agree — so a glyph that a *real* terminal draws wider than ratatui assumed produces a perfect green run here and a broken screen for the user.

Two shipped bugs came from that blind spot. `⏳` (U+23F3, East Asian Width = Wide) began the "waiting for download… cancel `✕`" line; Warp drew it two cells wide, so every cell after it landed one column right of its registered click rect and the cancel button ignored clicks. `↗` (U+2197, Ambiguous *and* a member of the Unicode emoji set) prefixed the about modal's link rows and pushed them one column right, bleeding the hover underline through the modal border.

`tests/glyphs.rs` (a plain `cargo test`, not part of this harness) is the guard. It scans every string literal in `src/tui.rs`, and for each non-ASCII character it finds it requires that the character is **neither Wide/Fullwidth nor in the Unicode emoji set** — emoji-set membership matters because a terminal may pick an emoji font and spend two cells regardless of what the width table says. East-Asian-Ambiguous non-emoji glyphs (box drawing, arrows, geometric shapes, `·`, `…`, `—`) are accepted: every terminal outside a CJK locale draws them at one cell.

Each glyph is listed in an `INVENTORY` table with its expected risk class and where it is drawn, so adding a glyph, removing one, or a Unicode release reclassifying one all fail with a message naming the character and the site. Glyphs that violate the rule and still ship sit in `ACCEPTED_RISK` with a written reason; entries that stop violating anything also fail, so the list can only shrink.

The practical rule when writing UI: anything inside, or to the left of, a click rect or a fixed-width column must be an unambiguous single-width glyph, because every rect in `src/tui.rs` is sized with `chars().count()` / `pill_width()`.
