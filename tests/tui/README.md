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
