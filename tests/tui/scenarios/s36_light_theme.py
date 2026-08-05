"""On a light terminal the panels are darker than the table, not black.

Every colour astrobib draws as RGB is a surface: a panel tint, a row
fill, a divider, or the body text on one. Those are defined as an offset
*from* the terminal's background — and an offset has a direction. Held as
constants they only ever pointed one way, so on a light terminal the
tints that read as barely-raised panels became near-black slabs against
white, and `Color::White` foregrounds became invisible.

What this pins is the direction, not the values: on a light background
every surface must be *darker* than the background and still close to it,
and on a dark background lighter. The exact tints are a matter of taste
and will drift; a panel on the wrong side of the background is a bug.
"""

from driver import require

DESCRIPTION = "light terminal: panels sit just under the background, not far below"

ENV = {"ASTROBIB_THEME": "light"}


def _bg_at(t, x, y):
    """The background colour of one cell, as (r, g, b), or None.

    pyte reports a default cell background as the string "default"; the
    table is deliberately on the terminal's own colour, so that is the
    answer there and an RGB hex string on every tinted surface.
    """
    v = t.screen.buffer[y][x].bg
    if v == "default":
        return None
    return _hex(v)


def _hex(v):
    return tuple(int(v[i : i + 2], 16) for i in (0, 2, 4))


def _luma(rgb):
    r, g, b = rgb
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0


def run(t):
    t.wait_for("Cabrera, +1", what="the fixture rows")

    # the pub card is up at startup, on the right of the table
    card_x = t.screen.columns - 10
    # a row the cursor is *not* on: the cursor row is deliberately filled,
    # and that fill would answer for the table's own background
    row = t.row_of("Ekwueme, +4")

    # the table is the primary surface: the terminal's own background,
    # never tinted — tinting it would fight every theme
    require(
        _bg_at(t, 2, row) is None,
        "the table should keep the terminal's own background",
        t,
    )

    card = _bg_at(t, card_x, row)
    require(card is not None, "the pub card should carry a tint of its own", t)
    # light means light: nearly white, and *under* the background rather
    # than the near-black it used to be
    require(
        _luma(card) > 0.8,
        f"a light theme's card tint should be light: {card} (luma {_luma(card):.2f})",
        t,
    )
    require(
        _luma(card) < 0.99,
        f"the card must still be distinguishable from the background: {card}",
        t,
    )

    # the footer is a surface too, and separates by tint alone
    footer = _bg_at(t, 2, len(t.lines()) - 1)
    require(footer is not None, "the footer should carry a tint of its own", t)
    require(
        _luma(footer) > 0.8,
        f"a light theme's footer tint should be light: {footer}",
        t,
    )
    require(
        footer != card,
        "adjacent surfaces must differ, or the tint separates nothing",
        t,
    )

    # a hovered row lifts its text one level rather than taking a fill.
    # That lift was `Color::White`, which on a light terminal is the
    # background — the row went blank under the pointer.
    t.hover(20, row)
    t.wait_quiet()
    lifted = {
        t.screen.buffer[row][x].fg
        for x, c in enumerate(t.lines()[row])
        if c not in " ·"
    }
    require(
        "white" not in lifted and "brightwhite" not in lifted,
        f"hovered text must not lift to white on a light terminal: {lifted}",
        t,
    )
    dark = [f for f in lifted if f != "default" and _luma(_hex(f)) < 0.3]
    require(
        dark,
        f"hovering should lift the row's text to something dark here: {lifted}",
        t,
    )
