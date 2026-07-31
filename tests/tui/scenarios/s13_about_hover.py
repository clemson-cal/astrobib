"""Hovering inside the @ about modal must not restyle the UI behind it.

A mouse position inside the modal also sits on hover-styled rects of the
surfaces it covers (pub-card link/copy rows, table rows, scope capsules);
their hover styling used to render around the modal's edges and read as
the modal's underline "extending outside". draw() now blinds the covered
surfaces to the mouse while a modal is up, restoring the real position
only for the modal itself.

At 100 cols the detail card sits behind the modal's right half, which is
the geometry the bug was reported in.
"""

from driver import require

DESCRIPTION = "@ modal suppresses hover on covered surfaces"

URL = "https://github.com/clemson-cal/astrobib/issues"

COLS, ROWS = 100, 40
# draw_about geometry at 100x40: w = min(52, 96) = 52 centered,
# h = 17 centered (no update-status line).
X0, X1 = 24, 76
Y0, Y1 = 11, 28


def cell(t, x, y):
    c = t.screen.buffer[y][x]
    return (c.data, c.fg, c.bg, c.bold, c.italics, c.underscore, c.reverse)


def snapshot_outside(t):
    """Every cell's glyph+style outside the modal rect (footer rows
    excluded: transient messages may legitimately change there)."""
    return {
        (x, y): cell(t, x, y)
        for y in range(t.rows - 3)
        for x in range(t.cols)
        if not (X0 <= x < X1 and Y0 <= y < Y1)
    }


def run(t):
    t.send("@")
    t.wait_for("report a bug")
    pos = t.find(URL)
    require(pos is not None, "issues link not on screen", t)
    lx, ly = pos
    require(
        X0 <= lx < X1 and Y0 <= ly < Y1,
        f"modal not where expected (link at {pos})",
        t,
    )
    base = snapshot_outside(t)

    # the modal's own links must still react to the mouse
    t.hover(lx + 10, ly)
    t.wait_for(
        lambda: t.screen.buffer[ly][lx + 10].underscore,
        what=f"underline on the hovered modal link at ({lx + 10}, {ly})",
    )

    # sweep the modal interior: card link/copy rows run beneath its right
    # half here, and pre-fix their hover styling bled out around the modal.
    #
    # There is no needle to wait for here — a correct app answers most of
    # these hovers with no output at all — so wait_quiet, not settle: it
    # returns only once the stream has actually gone silent, whereas a
    # fixed sleep can snapshot halfway through a repaint and report cells
    # as "leaked" that were about to be restored.
    leaks = []
    for y in range(Y0 + 1, Y1 - 1):
        for x in (X0 + 21, X0 + 33, X0 + 41, X0 + 49):
            t.hover(x, y)
            t.wait_quiet(0.06)
            cur = snapshot_outside(t)
            diff = [k for k in base if cur[k] != base[k]]
            if diff:
                leaks.append((
                    (x, y),
                    [f"{k}: {base[k][0]!r}->{cur[k][0]!r} style {base[k][1:]}->{cur[k][1:]}"
                     for k in diff[:3]],
                ))
    require(
        not leaks,
        "hovering inside the @ modal restyled cells outside it (the modal must "
        "blind the surfaces it covers to the mouse): " + "; ".join(
            f"hover{p} changed {d}" for p, d in leaks[:4]
        ),
        t,
    )
