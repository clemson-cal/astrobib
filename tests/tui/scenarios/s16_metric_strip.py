"""The metric swatch is a column like any other: the table panel says
whether it shows, and M only picks which metric it shows.

M used to cycle off → priority → citations → off, which made the strip
the one piece of chrome with its own visibility mode. Now it is off by
default, turned on from the panel, and M toggles between the two
metrics; priority keys (. 0 < >) act with footer feedback either way.
"""

from driver import require

DESCRIPTION = "metric column: panel shows it, M picks the metric"


def run(t):
    # off by default, and M alone does not summon it
    require("⣿" not in t.text(), "the metric strip should start hidden", t)

    t.send("|")
    t.wait_for("Table configuration", what="the table panel")
    # Metric is the first row, so it is already under the cursor
    t.send(" ")
    t.wait_for(
        lambda: "✓ ⣿ metric" in t.text(),
        what="the metric column switched on from the panel",
    )
    # the panel names the active metric, since nothing else on screen does
    t.wait_for(lambda: "prio" in t.text(), what="the panel naming the priority metric")

    t.send("M")
    t.wait_for(lambda: "metric column: citations (magma)" in t.text(), what="citations note")
    t.wait_for(lambda: "cite" in t.text(), what="the panel following M to citations")
    t.send("M")
    t.wait_for(lambda: "metric column: priority (viridis)" in t.text(), what="priority note")

    # hand the arrows back so the priority keys reach the table
    t.send("\x1b")
    t.wait_quiet()

    # . sets the cursor entry's priority to 1.0
    t.send(".")
    t.wait_for(lambda: "priority 1.00" in t.text(), what="set-to-one feedback")
    # < scales the effective level down (×0.8)
    t.send("<")
    t.wait_for(lambda: "priority 0.80" in t.text(), what="scale-down feedback")
    # 0 clears
    t.send("0")
    t.wait_for(lambda: "priority 0.00" in t.text(), what="clear feedback")

    # the wheel over a priority swatch scales that row's level. The
    # swatch is a table column now, sitting after the selection gutter
    # rather than at the screen edge, so find it by its ⣿ header instead
    # of assuming column 0 — which is what this used to do, and which
    # made the assertion pass on a stale footer rather than on the wheel.
    t.send("|")
    t.wait_gone("Table configuration")
    header = next((i for i, l in enumerate(t.lines()) if "Year" in l[:40]), None)
    require(header is not None, "table header row not found (Year in the first 40 cols)", t)
    mx = t.lines()[header].index("⣿")
    t.send(b"\x1b[<64;%d;%dM" % (mx + 1, header + 3))  # wheel-up on the swatch
    t.wait_for(
        lambda: "priority 0.0" in t.lines()[-1],
        what="wheel-up priority feedback in the footer",
    )
