"""Manuscript rows that resolve to a paper are selectable; a cite that
resolves to nothing is not, and says so."""

from driver import require

DESCRIPTION = "manuscript rows select when they resolve"

MANUSCRIPT = {
    "main.tex": (
        "\\documentclass{article}\n\\begin{document}\n"
        "Citing \\citep{Andersson2021} and \\citet{Baxter2019} "
        "and \\citep{Nowhere2020}.\n\\end{document}\n"
    ),
}


def _row(t, needle):
    for ln in t.lines():
        if needle in ln:
            return ln
    return ""


def run(t):
    t.wait_for("Manuscript", what="manuscript capsule")
    t.send("]")
    t.wait_for(lambda: "missing" in t.text(), what="manuscript rows")

    # a selects every row that resolves — the missing cite is skipped
    t.send("a")
    t.wait_for(lambda: "2 selected" in t.text(), what="two rows selected")
    require("◉" in _row(t, "Andersson2021"), "resolvable row not marked", t)
    require("◉" in _row(t, "Baxter2019"), "resolvable row not marked", t)
    missing = _row(t, "Nowhere2020")
    require("◉" not in missing and "◯" not in missing,
            f"unresolvable row should carry no mark: {missing!r}", t)

    # and the bulk action reaches those papers
    t.key("esc")
    t.wait_gone("selected")
    t.send("G")  # onto the missing row (last)
    t.send(" ")
    t.wait_for(lambda: "resolves to no paper" in t.text(), what="explanation")
    require("selected" not in t.lines()[-1],
            "an unselectable row must not enter selection mode", t)
