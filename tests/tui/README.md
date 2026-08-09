# TUI integration harness

Drives the real `target/release/astrobib` binary in a pseudo-terminal and asserts against the screen reconstructed with [pyte](https://github.com/selectel/pyte) — never against raw output bytes.

## Run

```bash
tests/tui/run.py            # builds the binary, then runs every scenario
tests/tui/run.py --no-build # reuse the existing target/release/astrobib
tests/tui/run.py -k sort    # only scenarios whose filename matches
tests/tui/run.py --list     # show the scenario roster
tests/tui/run.py -j 1       # serially, for a clean log or a stubborn flake
```

On first use `run.py` creates `tests/tui/.venv` (any python3 works), installs pyte into it, and re-execs itself — no manual setup. Exit status is nonzero if any scenario fails; failures print the reconstructed screen.

Scenarios run concurrently (`-j`, default 8). They are isolated by construction — a `Session` owns its scratch directory, its pty and its process, and nothing in the suite reaches outside that sandbox — so the only contended resource is the machine, and the suite barely uses it: run serially it takes a minute at 8% CPU, because almost all of that minute is *waiting* on a repaint to land or on the app's own timers. Overlapped, the whole roster costs about what its slowest single scenario does.

That makes the slowest scenario the thing to watch: `s32_edit_query` sets the floor at ~10s, because it twice waits out the ageing of a startup note so the footer affordance it is testing can take the line. Results print as they land, so completion order is roughly fastest-first and a hung scenario shows up as the line that never arrives; failures are named again in roster order at the end.

Network scenarios (ADS search) are skipped unless `RUN_ADS_TESTS=1` is set; the ADS scenario additionally needs `ADS_API_TOKEN` in the environment.

## Sandboxing

Each scenario gets a fresh scratch directory. `HOME`, `ASTROBIB_LIBRARY`, and `ASTROBIB_STATE_DIR` all point inside it, so the app can never read or write the real library, `state.json`, `tabs.json`, the PDF cache, or `~/Downloads`. The scratch library is populated from `fixtures/*.bib` — five synthetic entries written by hand in the on-disk format (FIELD_ORDER section first, trailing lowercase fields after), with correct `AuthorYYYY` + hash cite keys computed from their synthetic arXiv IDs / bibcodes. `ASTROBIB_ASCII=1` keeps pill rendering out of the Nerd Font private-use range.

## Writing scenarios

A scenario is `scenarios/sNN_name.py` exposing `DESCRIPTION`, optionally `ALLOW_NETWORK = True` (passes `ADS_API_TOKEN` through), and `run(t)` where `t` is a started, ready `driver.Session`:

- `t.send(str)` / `t.key("esc"|"enter"|"delete"|"space"|…)` — keyboard input
- `t.click(x, y)` — left click at a 0-based cell, injected as SGR mouse reports
- `t.lines()` / `t.text()` / `t.find(needle)` / `t.row_of(needle)` — reconstructed screen
- `t.wait_for(substring_or_predicate, timeout=8)` / `t.wait_gone(needle)` — polling waits
- `t.wait_quiet(idle=0.15)` — block until the pty has been silent that long, i.e. the redraw the last input triggered has finished landing; the condition-based way to assert that something did *not* change
- `t.resize(cols, rows=None)` — resize the pty and the pyte screen together; the app gets a real SIGWINCH and re-lays-out as it would in a dragged terminal
- `driver.require(cond, msg, t)` — assert with a screen dump attached
- raise `driver.Skip("why")` to skip

### Waiting, not sleeping

Every assertion about something that must *become* true belongs inside a `wait_for`, not after one. The recurring flake is a scenario that waits for X and then asserts Y in the same breath, when Y lands a frame later: the pub card, in particular, repaints from the cursor entry and can trail the table by a frame, so `t.wait_for("1 selected")` followed by `require("Baxter2019equxm" in t.text())` is a race, while waiting on the card needle is not.

Useful invariant: within one frame the footer is drawn *after* the table, so a footer needle appearing (or disappearing) means that frame's table is already on screen. Negative assertions about the table are safe immediately after a footer `wait_gone`.

For "prove this key did *not* do something", prefer proof that the key was processed at all — after `?` opens the keys panel, `j` is verified by the card moving to the next entry, and only then is the panel asserted to still be up. Where no such proof exists, `wait_quiet` beats `settle`: it returns once the stream is actually idle, whereas a fixed sleep can hand you half a repaint. `settle` survives in exactly one place, s15, and not for the screen at all — a lone ESC written back-to-back with the next byte arrives in one pty read and crossterm parses it as alt+*key*, so the sleep is there to split two keystrokes. Any new `settle` call needs a comment saying why the other two do not work.

Failure messages print the reconstructed screen, so they should say what was expected, not that something was missing — `what="the 2018 entry (Délacroix) on the first data row, 6"` reads far better next to a screen dump than `"row wrong"`.

Gotchas learned the hard way:

- An assertion on the footer is only as good as what else could have put text there. `s16` passed for a while on a *stale* status line left by an earlier keypress rather than on the wheel it claimed to test; adding a real log message superseded that line and the gap showed at once. Prefer a needle only the thing under test could produce.
- Long titles are truncated in the table (~40 visible chars) and reflowed with different line breaks in the pub card, so a title fragment can match either region or neither. Anchor row positions with table-only needles such as author cells (`"Cabrera, +1"`) or short keys.
- The gutter (two leftmost cells of a table row) is a click target for selection mode; data rows start two rows below the header line (header, rule, rows).
- Never click a copy target in a scenario: clipboard writes go through `pbcopy`, which is the user's real pasteboard — the pty sandbox does not contain it. Open the copy modal, assert, Esc out.
- ratatui diff-renders, so the pty stream alone is meaningless; only the pyte screen state is trustworthy. `wait_for` pumps the stream before every predicate check.
- `wait_quiet` is unsound straight after `resize`: the stream is *already* quiet at that instant, so it can return before SIGWINCH has even been handled and hand the scenario the previous geometry — after which every click lands on stale coordinates. Wait for a positive width-dependent signal instead; s26 waits for the table's header rule to take its new width.

## Golden screens

`s26_table_chrome` is a refactor oracle rather than a feature test. It captures the table region — header, rule, and data rows — in all three scopes (library, manuscript, query) at four terminal widths with the pub card open and closed, plus three sort states, and requires the result to match `baselines/table_chrome.txt` byte for byte. Its purpose is to make large edits to `draw_table` provable: the responsive rules (author width scaling, the Key column dropping when tight) only appear under resize, and inspection alone cannot show that 460 lines of conditional rendering still produce the same pixels.

Re-bless with `ASTROBIB_BLESS=1 tests/tui/run.py -k s26`. That is a deliberate act, not a fixture refresh — a diff here means the rendered screen changed, which is exactly what the file exists to report. Review the diff in the commit.

## What this harness structurally cannot catch: glyph width

ratatui lays a frame out by asking the `unicode-width` crate how wide each character is, and pyte reconstructs the screen with the same table. App and harness therefore always agree — so a glyph that a *real* terminal draws wider than ratatui assumed produces a perfect green run here and a broken screen for the user.

Two shipped bugs came from that blind spot. `⏳` (U+23F3, East Asian Width = Wide) began the "waiting for download… cancel `✕`" line; Warp drew it two cells wide, so every cell after it landed one column right of its registered click rect and the cancel button ignored clicks. `↗` (U+2197, Ambiguous *and* a member of the Unicode emoji set) prefixed the about modal's link rows and pushed them one column right, bleeding the hover underline through the modal border.

`tests/glyphs.rs` (a plain `cargo test`, not part of this harness) is the guard. It scans every string literal in the TUI sources (every module under `src/tui/`, named one by one in its `SOURCES` list because `include_str!` needs literal paths — a companion test reads the directory and fails if the list has fallen behind it), and for each non-ASCII character it finds it requires that the character is **neither Wide/Fullwidth nor in the Unicode emoji set** — emoji-set membership matters because a terminal may pick an emoji font and spend two cells regardless of what the width table says. East-Asian-Ambiguous non-emoji glyphs (box drawing, arrows, geometric shapes, `·`, `…`, `—`) are accepted: every terminal outside a CJK locale draws them at one cell.

Each glyph is listed in an `INVENTORY` table with its expected risk class and where it is drawn, so adding a glyph, removing one, or a Unicode release reclassifying one all fail with a message naming the character and the site. Glyphs that violate the rule and still ship sit in `ACCEPTED_RISK` with a written reason; entries that stop violating anything also fail, so the list can only shrink.

The practical rule when writing UI: anything inside, or to the left of, a click rect or a fixed-width column must be an unambiguous single-width glyph, because every rect under `src/tui/` is sized with `chars().count()` / `pill_width()`.
