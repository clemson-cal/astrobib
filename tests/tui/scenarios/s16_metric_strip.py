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
    # < nudges the effective level down
    t.send("<")
    t.wait_for(lambda: "priority 0.90" in t.text(), what="nudge-down feedback")
    # 0 clears
    t.send("0")
    t.wait_for(lambda: "priority 0.00" in t.text(), what="clear feedback")
