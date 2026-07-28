"""ADS search round-trip (network). Skipped unless RUN_ADS_TESTS=1."""

import os

from driver import Skip, require

DESCRIPTION = "ADS search opens a query scope (network)"
ALLOW_NETWORK = True


def run(t):
    if os.environ.get("RUN_ADS_TESTS") != "1":
        raise Skip("set RUN_ADS_TESTS=1 to run network scenarios")
    if not os.environ.get("ADS_API_TOKEN"):
        raise Skip("RUN_ADS_TESTS=1 but ADS_API_TOKEN is not set")

    t.send("S")
    t.wait_for("ADS query:")
    t.send('author:"einstein, a" year:1916')
    t.key("enter")
    t.wait_for("Searching ADS", timeout=10)
    # results land on a worker thread; the new scope's rows carry the year
    t.wait_for(
        lambda: "1916" in t.text() and "Searching ADS" not in t.lines()[-1],
        timeout=45,
        what="ADS results in the query scope",
    )
    require("einstein" in t.text(), "query scope tab should carry the query", t)
    # back to the Library scope
    t.send("[")
    t.wait_for("Turbulent magnetic amplification")
