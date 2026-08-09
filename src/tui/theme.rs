//! Which way the terminal's background runs, and the surface colours
//! that follow from it.
//!
//! Every colour astrobib draws as RGB rather than as an ANSI name is a
//! *surface*: a panel tint, a row fill, a divider, or the body text on
//! one. Those cannot be left to the terminal's palette, because the
//! whole point of them is to sit a few points away from its background —
//! but that is also why they cannot be constants. "A few points away"
//! has a direction, and on a light terminal the tints that read as
//! barely-raised panels become near-black slabs against white.
//!
//! So the background is asked for once, at startup, and every surface is
//! a function of the answer. This is not a theme system: there is no
//! configuration, no runtime switch and no third option. It is one bit,
//! read from the terminal, that says which way *away from the
//! background* points.

use ratatui::style::Color;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static LIGHT: AtomicBool = AtomicBool::new(false);

fn light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

fn pick(dark: Color, light_: Color) -> Color {
    if light() {
        light_
    } else {
        dark
    }
}

/// Settle the background question, once, before the alternate screen is
/// entered. Dark is the fallback: it is what a terminal that will not
/// answer has historically been, and it is what astrobib looked like
/// before it asked.
///
/// `ASTROBIB_THEME=light|dark` skips the probe. The pty harness sets it
/// so scenarios never depend on a reply that a screen emulator has no
/// way to send.
pub(super) fn detect() {
    let forced = std::env::var("ASTROBIB_THEME").ok();
    let answer = match forced.as_deref() {
        Some("light") => Some(true),
        Some("dark") => Some(false),
        _ => probe_osc11().or_else(from_colorfgbg),
    };
    LIGHT.store(answer.unwrap_or(false), Ordering::Relaxed);
}

/// OSC 11: "what is your background?", answered as `rgb:RRRR/GGGG/BBBB`.
/// Widely supported and the only reliable way to know — `$COLORFGBG` is
/// set by a minority of terminals and never updated when the user
/// switches themes mid-session.
fn probe_osc11() -> Option<bool> {
    use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return None;
    }
    // the reply arrives on stdin as bytes, so it can only be read
    // unbuffered; raw mode is restored either way before returning
    let raw = enable_raw_mode().is_ok();
    let asked = {
        let mut out = std::io::stdout();
        out.write_all(b"\x1b]11;?\x07").is_ok() && out.flush().is_ok()
    };
    let reply = if asked { read_reply(60) } else { None };
    if raw {
        let _ = disable_raw_mode();
    }
    let (r, g, b) = parse_rgb(&reply?)?;
    // relative luminance; the midpoint is all this has to decide
    Some((0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0 > 0.5)
}

/// Read until the reply terminates or the deadline passes. Terminals
/// that ignore OSC 11 send nothing, so this must never block on one.
fn read_reply(ms: u64) -> Option<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() || buf.len() > 256 {
            break;
        }
        let mut pfd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut pfd, 1, left.as_millis() as libc::c_int) } <= 0 {
            break;
        }
        let mut chunk = [0u8; 64];
        let n = unsafe {
            libc::read(0, chunk.as_mut_ptr() as *mut libc::c_void, chunk.len())
        };
        if n <= 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n as usize]);
        // BEL, or the ST that terminals which dislike BEL use instead
        if buf.contains(&0x07) || buf.windows(2).any(|w| w == b"\x1b\\") {
            break;
        }
    }
    (!buf.is_empty()).then(|| String::from_utf8_lossy(&buf).into_owned())
}

/// `rgb:RRRR/GGGG/BBBB`, and the 1-, 2- and 3-digit widths the spec also
/// allows. Each component is scaled to 8 bits by its own width, not by
/// assuming four digits — `rgb:f/f/f` is white, not near-black.
fn parse_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let rest = &s[s.find("rgb:")? + 4..];
    let mut it = rest.split('/');
    let mut comp = || -> Option<u8> {
        let f = it.next()?;
        let hex: String = f.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if hex.is_empty() || hex.len() > 4 {
            return None;
        }
        let v = u32::from_str_radix(&hex, 16).ok()?;
        let max = (1u32 << (4 * hex.len())) - 1;
        Some((v * 255 / max) as u8)
    };
    Some((comp()?, comp()?, comp()?))
}

/// `COLORFGBG=15;0` — foreground, then background, as palette indices.
/// Only a minority of terminals set it, hence the fallback position.
fn from_colorfgbg() -> Option<bool> {
    let v = std::env::var("COLORFGBG").ok()?;
    let bg: u8 = v.rsplit(';').next()?.trim().parse().ok()?;
    Some(bg == 7 || bg == 15)
}

// -- surfaces ------------------------------------------------------------
//
// Each secondary view gets a barely-there tint of its own, so which
// surface the eye is on is legible without a border doing the work. The
// table keeps the terminal's own colour — it is the primary surface, and
// tinting it would fight every theme.

pub(super) fn panel_bg() -> Color {
    pick(Color::Rgb(24, 26, 32), Color::Rgb(234, 237, 243))
}
pub(super) fn card_bg() -> Color {
    pick(Color::Rgb(28, 25, 31), Color::Rgb(242, 236, 243))
}
pub(super) fn log_bg() -> Color {
    pick(Color::Rgb(23, 28, 29), Color::Rgb(233, 241, 240))
}
pub(super) fn help_bg() -> Color {
    pick(Color::Rgb(30, 28, 23), Color::Rgb(245, 240, 230))
}
pub(super) fn footer_bg() -> Color {
    pick(Color::Rgb(26, 27, 31), Color::Rgb(237, 238, 243))
}

/// The cursor row's fill — "standing on a surface". Shared by all three
/// scopes, and the one fill that is allowed to be more than a hint,
/// since it is what tells you where you are.
pub(super) fn cursor_fill() -> Color {
    pick(Color::Rgb(34, 40, 52), Color::Rgb(213, 224, 244))
}

/// Row-level hover fill in the panels that are lists of clickable rows.
pub(super) fn row_hover_bg() -> Color {
    pick(Color::Rgb(50, 54, 62), Color::Rgb(224, 228, 236))
}

/// The fill marking a hovered copy region in the pub card.
pub(super) fn copy_region_bg() -> Color {
    pick(Color::Rgb(44, 48, 56), Color::Rgb(227, 231, 239))
}

/// Rules and dividers drawn *within* a panel.
pub(super) fn divider_fg() -> Color {
    pick(Color::Rgb(62, 66, 74), Color::Rgb(196, 201, 210))
}

// -- text ----------------------------------------------------------------

/// Unhovered table text (author, title) and the card's abstract body —
/// modestly softer than the terminal foreground, in whichever direction
/// softer means here.
pub(super) fn table_text() -> Color {
    pick(Color::Rgb(150, 155, 163), Color::Rgb(88, 93, 102))
}
pub(super) fn abstract_text() -> Color {
    pick(Color::Rgb(170, 174, 182), Color::Rgb(60, 64, 72))
}

/// The foreground one level *above* body text: hovered rows, the cursor
/// entry's title, emphatic labels. `Color::White` was this until a light
/// terminal made it the background.
pub(super) fn text_strong() -> Color {
    pick(Color::White, Color::Rgb(18, 20, 26))
}

// -- chips ---------------------------------------------------------------

pub(super) fn chip_bg() -> Color {
    pick(Color::Rgb(40, 44, 52), Color::Rgb(222, 226, 234))
}
pub(super) fn chip_bg_hover() -> Color {
    pick(Color::Rgb(58, 63, 72), Color::Rgb(202, 208, 219))
}

/// The filter capsule's own warmer pair: it is a mode rather than a
/// scope, so it reads yellow instead of joining the chip family.
pub(super) fn filter_chip_bg() -> Color {
    pick(Color::Rgb(52, 47, 26), Color::Rgb(250, 241, 208))
}
pub(super) fn filter_chip_bg_hover() -> Color {
    pick(Color::Rgb(70, 62, 30), Color::Rgb(244, 230, 176))
}
/// ANSI yellow on a pale yellow chip is unreadable, so the light form
/// names its own amber rather than trusting the palette.
pub(super) fn filter_chip_fg() -> Color {
    pick(Color::Yellow, Color::Rgb(122, 92, 12))
}

/// Chip labels, in the three weights the capsules use.
pub(super) fn chip_fg_strong() -> Color {
    pick(Color::White, Color::Rgb(24, 26, 32))
}
pub(super) fn chip_fg() -> Color {
    pick(Color::Gray, Color::Rgb(72, 76, 84))
}
pub(super) fn chip_fg_dim() -> Color {
    pick(Color::DarkGray, Color::Rgb(126, 130, 138))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_widths_osc11_can_answer_in() {
        assert_eq!(parse_rgb("\x1b]11;rgb:0000/0000/0000\x07"), Some((0, 0, 0)));
        assert_eq!(parse_rgb("\x1b]11;rgb:ffff/ffff/ffff\x1b\\"), Some((255, 255, 255)));
        // scaled by its own width: one digit of f is white, not 0x0f
        assert_eq!(parse_rgb("rgb:f/f/f"), Some((255, 255, 255)));
        assert_eq!(parse_rgb("rgb:ff/80/00"), Some((255, 128, 0)));
        assert_eq!(parse_rgb("no reply at all"), None);
    }

    #[test]
    fn a_dark_background_is_the_fallback_everywhere() {
        // nothing to parse, nothing to conclude — and the caller then
        // falls back to dark, which is what astrobib always was
        assert_eq!(parse_rgb(""), None);
        assert_eq!(parse_rgb("rgb://"), None);
        assert_eq!(parse_rgb("rgb:12345/0/0"), None); // wider than 4 digits
    }
}
