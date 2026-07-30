"""M cycles the metric swatch column (off → age → citations → off);
the strip appears only while a metric is active, and . touches the
cursor entry."""

from driver import require

DESCRIPTION = "metric column cycles with M; . touches"


def run(t):
    base = t.text()
    require("metric" not in base, "metric note should not show yet", t)
    t.send("M")
    t.wait_for(lambda: "metric column: age (viridis)" in t.text(), what="age metric note")
    t.send("M")
    t.wait_for(
        lambda: "metric column: citations (magma)" in t.text(), what="citations metric note"
    )
    t.send("M")
    t.wait_for(lambda: "metric column: off" in t.text(), what="metric off note")
    # touch the cursor entry
    t.send(".")
    t.wait_for(lambda: "touched " in t.text(), what="touch confirmation")
