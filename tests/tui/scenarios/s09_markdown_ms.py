"""Markdown manuscripts: pandoc @cites and resolving wikilinks classify
in the Manuscript scope; unresolved wikilinks are note links, not cites."""

from driver import require

DESCRIPTION = "markdown manuscript: @cites + wikilinks classify"

MANUSCRIPT = {
    "main.md": (
        "# Review\n\n"
        "Built on @Andersson2021 and the census [[Baxter2019]],\n"
        "with [@Nowhere2020] still unread. See [[note to self]].\n\n"
        "![[extra]]\n"
    ),
    "extra.md": "Embedded sections cite too: [[Delacroix2018|seminal]].\n",
}


def run(t):
    # the manuscript scan runs just after the first paint
    t.wait_for("Manuscript", what="manuscript capsule")
    t.send("]")  # Library is scope 0; the manuscript scope sits at 1
    t.wait_for(lambda: "missing" in t.text(), what="manuscript rows")
    txt = t.text()
    # pandoc cites resolve by prefix into the library tier
    require("Andersson2021" in txt, "pandoc cite Andersson2021 missing", t)
    # wikilinks that resolve are citations — including embedded files'
    require("Baxter2019" in txt, "wikilink cite Baxter2019 missing", t)
    require("Delacroix2018" in txt, "embedded-file wikilink cite missing", t)
    # a pandoc cite that resolves nowhere surfaces as missing
    require("Nowhere2020" in txt, "unresolved pandoc cite not shown", t)
    # an unresolved wikilink is an ordinary note link, not a citation
    require("note to self" not in txt, "note link wrongly shown as citation", t)
