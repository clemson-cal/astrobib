"""T opens the pending-tasks overlay (empty state); Esc closes it.

Real in-flight tasks need the network, so this only exercises the empty
overlay — open on its key, show the empty state, close on Esc.
"""

from driver import require

DESCRIPTION = "pending-tasks overlay opens empty, Esc closes"


def run(t):
    t.send("T")
    t.wait_for("no pending tasks")
    require("pending tasks" in t.text(), "overlay title missing", t)
    require("Esc close" in t.text(), "overlay footer hint missing", t)
    t.key("esc")
    t.wait_gone("no pending tasks")
    require("pending tasks" not in t.text(), "overlay should close on Esc", t)
    # the table is intact behind the dismissed overlay
    require("Cabrera, +1" in t.text(), "table should be back after close", t)
