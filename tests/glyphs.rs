//! Guard against ambiguous- and double-width glyphs in the TUI chrome.
//!
//! Why this test exists — and why the pyte harness cannot replace it:
//!
//! ratatui lays a frame out by asking the `unicode-width` crate how many
//! cells each character occupies. The pty harness (`tests/tui/`) rebuilds
//! the screen with pyte, which uses the same width table. Harness and app
//! therefore always agree, so a glyph that a *real* terminal draws wider
//! than ratatui assumed is structurally invisible to the scenarios.
//!
//! Two shipped bugs came from exactly that blind spot:
//!
//!   * `⏳` (U+23F3, East Asian Width = Wide) began the "waiting for
//!     download… cancel ✕" line. Warp drew it two cells wide, so every
//!     cell after it landed one column right of the click rect ratatui had
//!     registered, and the `✕` cancel button ignored clicks.
//!   * `↗` (U+2197, Ambiguous, and in the Unicode emoji set) prefixed the
//!     about modal's link rows. Terminals that pick an emoji font for it
//!     pushed the row one column right, so the hover underline bled
//!     through the modal border.
//!
//! The rule this file enforces on every glyph the TUI renders:
//!
//!   1. never Wide or Fullwidth (`UnicodeWidthChar::width() == 2`);
//!   2. never a member of the Unicode emoji set (`Emoji=YES`), because a
//!      terminal may render those with an emoji font at two cells no
//!      matter what the width table says — this is the class `↗` and `⏳`
//!      both belong to;
//!   3. East-Asian-Ambiguous non-emoji glyphs (box drawing, arrows, the
//!      geometric shapes, `·`, `…`, `—`) are accepted: every terminal
//!      outside a CJK locale draws them at one cell.
//!
//! `INVENTORY` below records every non-ASCII character that appears in a
//! *string literal* in `src/tui.rs`, together with the risk class it is
//! expected to fall in. The test recomputes that class from the Unicode
//! data and fails on any disagreement, so:
//!
//!   * adding a new glyph to the UI fails until it is reviewed and listed;
//!   * removing one fails until the stale row is deleted;
//!   * a listed glyph whose class changes (say a future Unicode release
//!     moves it into the emoji set) fails immediately.
//!
//! Glyphs that violate rule 1 or 2 and are still shipping must be listed
//! in `ACCEPTED_RISK` with a reason. That list is a debt ledger, not a
//! silencer: an entry that no longer violates anything also fails.

use unicode_properties::UnicodeEmoji;
use unicode_width::UnicodeWidthChar;

const SRC: &str = include_str!("../src/tui.rs");

/// Where a glyph sits, which decides how bad a width surprise is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Zone {
    /// In, or to the left of, a registered click rect or a fixed-width
    /// column: a mis-measured glyph shifts a click target or a border.
    Hit,
    /// Free-flowing text (status line messages, hover hints, modal prose).
    /// A width surprise only ragged-edges the rest of that one line.
    Flow,
}

/// Risk class, worst first. A glyph gets the first class that applies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Risk {
    /// East Asian Width = Wide or Fullwidth. Always two cells. Never OK.
    Wide,
    /// `Emoji=YES`: a terminal may substitute an emoji font and draw two
    /// cells regardless of the width table. Never OK.
    Emoji,
    /// East Asian Width = Ambiguous, not emoji. One cell everywhere
    /// outside a CJK locale. Accepted.
    Ambiguous,
    /// Unambiguously one cell.
    Narrow,
}

impl Risk {
    fn of(c: char) -> Risk {
        if c.width() == Some(2) {
            Risk::Wide
        } else if c.is_emoji_char() {
            Risk::Emoji
        } else if c.width() != c.width_cjk() {
            Risk::Ambiguous
        } else {
            Risk::Narrow
        }
    }

    fn acceptable(self) -> bool {
        matches!(self, Risk::Ambiguous | Risk::Narrow)
    }
}

/// Every non-ASCII character in a string literal in `src/tui.rs`.
/// `(glyph, zone, expected risk, where it is drawn)`
const INVENTORY: &[(char, Zone, Risk, &str)] = &[
    // -- structure: borders, rules, separators ---------------------------
    ('─', Zone::Hit, Risk::Ambiguous, "block borders, card rules, the tab strip"),
    ('│', Zone::Hit, Risk::Ambiguous, "the \"▤ card │ @ bib\" segmented toggler"),
    ('·', Zone::Hit, Risk::Ambiguous, "footer \"n/total  ·  \" prefix, hint separators"),
    ('…', Zone::Hit, Risk::Ambiguous, "truncation marker in fixed-width table cells"),
    ('—', Zone::Flow, Risk::Ambiguous, "em dash in status messages and hints"),
    // -- gutter and table columns ----------------------------------------
    ('◉', Zone::Hit, Risk::Narrow, "gutter cursor / selected marker"),
    ('◯', Zone::Hit, Risk::Ambiguous, "gutter unselected marker"),
    ('●', Zone::Hit, Risk::Ambiguous, "manuscript-membership column, \"● in library\""),
    ('○', Zone::Hit, Risk::Ambiguous, "cite-state marker: in library, not cited"),
    ('↓', Zone::Hit, Risk::Ambiguous, "PDF column header, PDF-fetch button labels"),
    ('↑', Zone::Hit, Risk::Ambiguous, "sort/limit indicators"),
    ('▲', Zone::Hit, Risk::Ambiguous, "ascending sort indicator"),
    ('▼', Zone::Hit, Risk::Ambiguous, "descending sort indicator"),
    ('✗', Zone::Hit, Risk::Narrow, "cite-state missing marker"),
    // -- card affordances and badges -------------------------------------
    ('⧉', Zone::Hit, Risk::Narrow, "copy-row badge in the card link stack"),
    ('⌕', Zone::Hit, Risk::Narrow, "query-row badge, filter chip, task labels"),
    ('✕', Zone::Hit, Risk::Narrow, "\"Clear ✕\" button, \"cancel ✕\" click target"),
    ('◌', Zone::Hit, Risk::Narrow, "\"◌ waiting for download…\" prefix (was ⏳)"),
    ('◆', Zone::Hit, Risk::Ambiguous, "manuscript toggle badge"),
    ('◇', Zone::Hit, Risk::Ambiguous, "manuscript toggle badge, off state"),
    ('⟳', Zone::Hit, Risk::Narrow, "refresh affordance in the strip and about modal"),
    ('▤', Zone::Hit, Risk::Ambiguous, "\"▤ card\" segment of the card/bib toggler"),
    ('→', Zone::Hit, Risk::Ambiguous, "import button, help-sheet key legend"),
    ('⧗', Zone::Hit, Risk::Narrow, "pending-task count at the head of the footer"),
    // -- key legend -------------------------------------------------------
    ('⌫', Zone::Hit, Risk::Narrow, "help-sheet key legend: remove"),
    ('␣', Zone::Hit, Risk::Narrow, "help-sheet key legend: select"),
    ('⏎', Zone::Flow, Risk::Narrow, "modal titles and hint prose"),
    ('±', Zone::Flow, Risk::Ambiguous, "help-sheet legend: \"manuscript ± (selection)\""),
    // -- Nerd Font pill caps (non-ASCII mode only; ASTROBIB_ASCII=1 off) --
    ('\u{e0b6}', Zone::Hit, Risk::Ambiguous, "powerline pill cap, left"),
    ('\u{e0b4}', Zone::Hit, Risk::Ambiguous, "powerline pill cap, right"),
    // -- flowing text -----------------------------------------------------
    ('⤷', Zone::Flow, Risk::Narrow, "hover hint for the PDF picker"),
    ('⤓', Zone::Flow, Risk::Narrow, "import task label"),
    // -- shipping offenders (see ACCEPTED_RISK) ---------------------------
    ('↗', Zone::Hit, Risk::Emoji, "link-row badge, about-modal links, \"Open ↗\" button"),
    ('◼', Zone::Hit, Risk::Emoji, "footer badge, on state"),
    ('◻', Zone::Hit, Risk::Emoji, "footer badge, off state"),
    ('⚠', Zone::Flow, Risk::Emoji, "\"no open-access PDF found\" status message"),
    ('👁', Zone::Flow, Risk::Emoji, "\"watching ~/Downloads\" status message"),
    ('©', Zone::Flow, Risk::Emoji, "about modal copyright line"),
];

/// Glyphs that break rule 1 or 2 and are still on screen. Shrink, never
/// grow: each row is a live rendering bug on some terminal.
///
/// All six are grandfathered, not endorsed. Fixing them means editing
/// `src/tui.rs`, which is outside the reach of the change that added this
/// guard; the point of listing them here is that they can never grow to
/// seven without a deliberate act.
const ACCEPTED_RISK: &[(char, &str)] = &[
    (
        '↗',
        "SHIPPING BUG. Caused the about-modal underline bleed; still prefixes \
         the card's link rows and the \"Open ↗\" pill. Click rects are sized \
         with pill_width()/chars().count(), i.e. one cell per char, so on an \
         emoji-font terminal every affordance to its right is off by one.",
    ),
    (
        '◼',
        "SHIPPING BUG. Footer badge on-state, drawn by draw_badges(), whose \
         click rects advance bx by chars().count()+2. Two cells here misplaces \
         the card / log / keys toggles.",
    ),
    ('◻', "SHIPPING BUG. Footer badge off-state; same rects as ◼."),
    (
        '⚠',
        "Status-line message only — nothing is positioned after it, so the \
         damage is cosmetic. Still emoji-set; prefer a plain marker.",
    ),
    ('👁', "Status-line message only; cosmetic, same reasoning as ⚠."),
    (
        '©',
        "About-modal prose. The modal already carries several columns of \
         slack for exactly this reason, so it renders inside its border.",
    ),
];

// ---------------------------------------------------------------------------
// source scan
// ---------------------------------------------------------------------------

/// Every non-ASCII character inside a string or char literal in `src`,
/// paired with the 1-based lines it occurs on. Comments are skipped: a
/// glyph named in prose is not drawn. `\u{...}` escapes are decoded, so
/// the Nerd Font powerline caps count as used.
fn literal_glyphs(src: &str) -> Vec<(char, Vec<usize>)> {
    #[derive(PartialEq)]
    enum S {
        Code,
        Line,
        Block(usize),
        Str,
        Raw(usize),
        Ch,
    }
    let b: Vec<char> = src.chars().collect();
    let mut state = S::Code;
    let mut line = 1usize;
    let mut i = 0usize;
    let mut found: Vec<(char, Vec<usize>)> = Vec::new();
    let note = |c: char, line: usize, found: &mut Vec<(char, Vec<usize>)>| {
        if c.is_ascii() {
            return;
        }
        match found.iter_mut().find(|(g, _)| *g == c) {
            Some((_, lines)) => {
                if !lines.contains(&line) {
                    lines.push(line);
                }
            }
            None => found.push((c, vec![line])),
        }
    };
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied().unwrap_or('\0');
        if c == '\n' {
            line += 1;
            if state == S::Line {
                state = S::Code;
            }
            i += 1;
            continue;
        }
        match state {
            S::Line => i += 1,
            S::Block(depth) => {
                if c == '*' && next == '/' {
                    state = if depth == 1 { S::Code } else { S::Block(depth - 1) };
                    i += 2;
                } else if c == '/' && next == '*' {
                    state = S::Block(depth + 1);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            S::Str | S::Ch => {
                let closer = if state == S::Str { '"' } else { '\'' };
                if c == '\\' {
                    // decode \u{XXXX} so escaped glyphs are not missed
                    if next == 'u' && b.get(i + 2) == Some(&'{') {
                        let mut j = i + 3;
                        let mut hex = String::new();
                        while j < b.len() && b[j] != '}' {
                            hex.push(b[j]);
                            j += 1;
                        }
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            note(ch, line, &mut found);
                        }
                        i = j + 1;
                    } else {
                        i += 2;
                    }
                } else if c == closer {
                    state = S::Code;
                    i += 1;
                } else {
                    note(c, line, &mut found);
                    i += 1;
                }
            }
            S::Raw(hashes) => {
                if c == '"' && b[i + 1..].iter().take(hashes).all(|h| *h == '#') {
                    state = S::Code;
                    i += 1 + hashes;
                } else {
                    note(c, line, &mut found);
                    i += 1;
                }
            }
            S::Code => {
                if c == '/' && next == '/' {
                    state = S::Line;
                    i += 2;
                } else if c == '/' && next == '*' {
                    state = S::Block(1);
                    i += 2;
                } else if c == '"' {
                    state = S::Str;
                    i += 1;
                } else if c == 'r' && (next == '"' || next == '#') {
                    let mut h = 0;
                    while b.get(i + 1 + h) == Some(&'#') {
                        h += 1;
                    }
                    if b.get(i + 1 + h) == Some(&'"') {
                        state = S::Raw(h);
                        i += 2 + h;
                    } else {
                        i += 1;
                    }
                } else if c == '\'' {
                    // a char literal, or a lifetime — only the former has
                    // a closing quote two or three chars along
                    let is_char = next == '\\'
                        || b.get(i + 2) == Some(&'\'')
                        || (next.len_utf8() > 1 && b.get(i + 2) == Some(&'\''));
                    if is_char {
                        state = S::Ch;
                    }
                    i += 1;
                } else {
                    i += 1;
                }
            }
        }
    }
    found
}

fn describe(c: char) -> String {
    format!("{c:?} (U+{:04X})", c as u32)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// The inventory names exactly the glyphs `src/tui.rs` actually draws.
#[test]
fn inventory_matches_the_source() {
    let used = literal_glyphs(SRC);
    let mut problems = Vec::new();

    for (c, lines) in &used {
        if !INVENTORY.iter().any(|(g, ..)| g == c) {
            let at: Vec<String> = lines.iter().map(|l| format!("src/tui.rs:{l}")).collect();
            problems.push(format!(
                "  new glyph {} at {} — add a row to INVENTORY in tests/glyphs.rs\n\
                     (computed risk: {:?}; Wide and Emoji are not allowed on screen)",
                describe(*c),
                at.join(", "),
                Risk::of(*c),
            ));
        }
    }
    for (c, _, _, where_) in INVENTORY {
        if !used.iter().any(|(g, _)| g == c) {
            problems.push(format!(
                "  stale row {} ({where_}) — no longer in any src/tui.rs string \
                 literal; delete it from INVENTORY",
                describe(*c),
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "tests/glyphs.rs INVENTORY is out of date:\n{}\n\n\
         Every glyph the TUI renders is reviewed for terminal width, because \
         the pyte harness shares ratatui's width table and so cannot see this \
         class of bug. See the module docs in tests/glyphs.rs.",
        problems.join("\n"),
    );
}

/// No rendered glyph is Wide or emoji, and each row's declared risk is
/// what the Unicode data actually says.
#[test]
fn rendered_glyphs_are_single_width_everywhere() {
    let mut problems = Vec::new();

    for (c, zone, declared, where_) in INVENTORY {
        let actual = Risk::of(*c);
        if actual != *declared {
            problems.push(format!(
                "  {} ({where_}): INVENTORY says {declared:?}, Unicode says {actual:?} \
                 — update the row",
                describe(*c),
            ));
            continue;
        }
        if actual.acceptable() {
            continue;
        }
        let excused = ACCEPTED_RISK.iter().find(|(g, _)| g == c);
        if excused.is_none() {
            let consequence = match zone {
                Zone::Hit => "it sits in or before a click target / fixed-width column, \
                              so terminals that draw it two cells wide shift every cell \
                              after it and clicks miss",
                Zone::Flow => "terminals that draw it two cells wide ragged-edge the rest \
                               of the line",
            };
            problems.push(format!(
                "  {} is {actual:?} and is drawn at {where_} — {consequence}.\n    \
                 Replace it with an unambiguous single-width glyph, or, if it must ship, \
                 add it to ACCEPTED_RISK in tests/glyphs.rs with a reason.",
                describe(*c),
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "glyphs unsafe for terminal layout are rendered by src/tui.rs:\n{}",
        problems.join("\n"),
    );
}

/// `ACCEPTED_RISK` is a debt ledger: every entry must still be a real
/// violation and still be on screen, or the row has to go.
#[test]
fn accepted_risk_has_no_stale_entries() {
    let used = literal_glyphs(SRC);
    let mut problems = Vec::new();
    for (c, why) in ACCEPTED_RISK {
        if Risk::of(*c).acceptable() {
            problems.push(format!(
                "  {} is safe now ({why}) — delete its ACCEPTED_RISK row",
                describe(*c),
            ));
        }
        if !used.iter().any(|(g, _)| g == c) {
            problems.push(format!(
                "  {} is no longer rendered ({why}) — delete its ACCEPTED_RISK row",
                describe(*c),
            ));
        }
    }
    assert!(problems.is_empty(), "stale ACCEPTED_RISK rows:\n{}", problems.join("\n"));
}

/// The classifier itself, pinned against the two glyphs that caused the
/// shipped bugs and the replacements that fixed them.
#[test]
fn classifier_catches_the_historical_offenders() {
    assert_eq!(Risk::of('⏳'), Risk::Wide, "U+23F3 is East Asian Wide");
    assert_eq!(Risk::of('↗'), Risk::Emoji, "U+2197 is in the Unicode emoji set");
    assert_eq!(Risk::of('◌'), Risk::Narrow, "U+25CC replaced ⏳ in the cancel row");
    assert_eq!(Risk::of('✕'), Risk::Narrow, "U+2715 is the cancel click target");
    assert!(!Risk::of('⏳').acceptable());
    assert!(!Risk::of('↗').acceptable());
    assert!(Risk::of('─').acceptable(), "box drawing is Ambiguous but safe");
}

/// The literal scanner ignores prose and decodes escapes.
#[test]
fn scanner_reads_literals_only() {
    let sample = r#"
        // a comment mentioning ⏳ and ★
        /// doc comment with ⇄
        /* block ✓ /* nested ✗ */ still comment ⏳ */
        let a = "badge ⧉ here";
        let b = 'x';
        let c: &'static str = "lifetime then ✕";
        let d = "\u{e0b6}cap";
        let e = "escaped \" quote ▤";
    "#;
    let got: Vec<char> = literal_glyphs(sample).into_iter().map(|(c, _)| c).collect();
    assert_eq!(got, vec!['⧉', '✕', '\u{e0b6}', '▤'], "scanner picked up: {got:?}");
}
