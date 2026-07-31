"""M cycles the metric swatch column (off → priority → citations → off);
priority keys (. 0 < >) act with footer feedback."""

from driver import require

DESCRIPTION = "metric column cycles; priority keys act"


def run(t):
    t.send("M")
    t.wait_for(lambda: "metric column: priority (viridis)" in t.text(), what="priority note")
    t.send("M")
    t.wait_for(
        lambda: "metric column: citations (magma)" in t.text(), what="citations metric note"
    )
    t.send("M")
    t.wait_for(lambda: "metric column: off" in t.text(), what="metric off note")
    # . sets the cursor entry's priority to 1.0
    t.send(".")
    t.wait_for(lambda: "priority 1.00" in t.text(), what="set-to-one feedback")
    # < scales the effective level down (×0.8)
    t.send("<")
    t.wait_for(lambda: "priority 0.80" in t.text(), what="scale-down feedback")
    # 0 clears
    t.send("0")
    t.wait_for(lambda: "priority 0.00" in t.text(), what="clear feedback")
    # the wheel over a priority swatch scales that row's level
    t.send("M")
    t.wait_for(lambda: "metric column: priority" in t.text(), what="priority back on")
    header = next((i for i, l in enumerate(t.lines()) if "Year" in l[:40]), None)
    require(header is not None, "table header row not found (Year in the first 40 cols)", t)
    t.send(b"\x1b[<64;1;%dM" % (header + 3))  # wheel-up, column 0, first row
    t.wait_for(
        lambda: "priority 0.0" in t.lines()[-1],
        what="wheel-up priority feedback in the footer",
    )
