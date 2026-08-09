"""The @ panel says which databases the session is pointed at.

The panel already reported the ADS token's source and what the day had
cost it — everything about the environment except the part the reader is
standing in. The global tier's path is settable with --library and
appeared nowhere at all; the local tier could only be inferred from the
Manuscript capsule existing, which says that there is one and never
which one. Two directories with the same paper in a different state is
exactly the situation that produces the question.

The `t` state belongs on the global row rather than beside the badge it
duplicates: a hidden tier is a fact about what the numbers above it mean.
"""

from driver import require

DESCRIPTION = "@ panel names the global and local databases"

MANUSCRIPT = {
    "main.md": "Cited here: @Zrake2019yzabc\n",
}


def run(t):
    t.send("@")
    t.wait_for("report a bug", what="the about panel")

    # the local tier is the manuscript the session walked up into, named
    # relative to $HOME rather than spelled out from /
    require(
        "Local      ~/ms" in t.text(),
        f"the panel should name the local database:\n{t.text()}",
        t,
    )
    # the global tier sits outside $HOME in the sandbox, so only its tail
    # is predictable — the point is that a path is there at all
    glob = next((l for l in t.lines() if "Global" in l), "")
    require(
        "/library" in glob,
        f"the panel should name the global library: {glob!r}",
        t,
    )
    require(
        "hidden" not in glob,
        f"the global tier is on, so nothing should say hidden: {glob!r}",
        t,
    )

    t.key("esc")
    t.wait_gone("report a bug")

    # hiding the global tier changes what every count above it means, so
    # the panel has to say so
    t.send("t")
    t.wait_for("global tier hidden", what="the tier toggle taking effect")
    t.send("@")
    t.wait_for("report a bug", what="the about panel, reopened")
    glob = next((l for l in t.lines() if "Global" in l), "")
    require(
        "hidden (t)" in glob,
        f"a hidden global tier should say so on its own row: {glob!r}",
        t,
    )
    require(
        "Local      ~/ms" in t.text(),
        "hiding the global tier should not disturb the local row",
        t,
    )
    t.key("esc")
    t.wait_gone("report a bug")
