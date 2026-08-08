"""T tags the selection and untags it again — the same ± reading as m —
writing a plain-text file the filter can then select on with tag:, or
with is:tagged for the coarser question of whether a paper carries any."""

import os

from driver import require

DESCRIPTION = "T tags and untags; tag: filters on the result"


def _tag_file(t, name):
    return os.path.join(os.path.dirname(t.state_dir), "library", "tags", name)


def run(t):
    # the prompt offers what exists; with no tags yet it offers to create
    t.send("T")
    t.wait_for(lambda: "tag 1 +" in t.text(), what="the tag prompt, in + mode")
    require("create" in t.text(), "an empty database should offer to create the tag", t)

    t.send("section-3")
    t.key("enter")
    t.wait_for(lambda: "tagged 1 paper" in t.text(), what="the tag confirmation")
    path = _tag_file(t, "section-3")
    t.wait_for(lambda: os.path.exists(path), what="tags/section-3 on disk")
    first = open(path).read().strip()
    require(first.endswith("bz") or first, f"one cite key per line, got {first!r}", t)
    require(len(first.splitlines()) == 1, f"expected one line, got {first!r}", t)
    # the pub card names the tags this paper carries
    t.wait_for(lambda: "tags  section-3" in t.text(), what="the tag on the pub card")

    # …and each name is a link to its own filter: it underlines under the
    # pointer, the way every other clickable thing on the card does, and
    # clicking it replaces the filter rather than narrowing it
    row = next(i for i, ln in enumerate(t.lines()) if ln.strip().startswith("tags  "))
    col = t.lines()[row].index("tags  ") + len("tags  ")
    t.hover(col, row)
    t.wait_for(
        lambda: t.screen.buffer[row][col].underscore,
        what="the tag to underline under the pointer",
    )
    t.click(col, row)
    t.wait_for(
        lambda: "filtered to tag:section-3" in t.text(),
        what="the click to apply the tag as the filter",
    )
    require("1/5" in t.text(), f"one of five papers is tagged: {t.lines()[-1]!r}", t)
    t.key("esc")  # back to the unfiltered library for the rest of this run
    t.wait_for(lambda: "5/5" in t.text(), what="the filter to clear")

    # a second paper joins the same tag, and the file stays sorted
    t.send("j")
    t.send("T")
    t.wait_for(lambda: "tag 1 +" in t.text(), what="the tag prompt for the next paper")
    require("section-3" in t.text(), "the existing tag should be offered", t)
    t.send("section-3")
    t.key("enter")
    t.wait_for(lambda: "tagged 1 paper" in t.text(), what="the second tag confirmation")
    t.wait_for(
        lambda: len(open(path).read().split()) == 2,
        what="two keys in tags/section-3",
    )
    keys = open(path).read().split()
    require(keys == sorted(keys), f"tag file is not sorted: {keys}", t)

    # tag: selects exactly those two, and negation the rest. The count
    # lives in the footer, which the prompt occupies while it is open,
    # so each filter is committed with ⏎ before it is read.
    t.send("/")
    t.send("tag:section-3")
    t.key("enter")
    t.wait_for(lambda: "2/5" in t.text(), what="the filter to show 2 of 5 rows")
    t.send("/")
    for _ in range(len("tag:section-3")):
        t.key("backspace")
    t.send("-tag:section-3")
    t.key("enter")
    t.wait_for(lambda: "3/5" in t.text(), what="negation to show the other 3")
    # is:tagged asks the question tag: cannot put — carries any tag at all
    t.send("/")
    for _ in range(len("-tag:section-3")):
        t.key("backspace")
    t.send("is:tagged")
    t.key("enter")
    t.wait_for(lambda: "2/5" in t.text(), what="is:tagged to show the tagged two")
    t.send("/")
    for _ in range(len("is:tagged")):
        t.key("backspace")
    t.send("-is:tagged")
    t.key("enter")
    t.wait_for(lambda: "3/5" in t.text(), what="-is:tagged to show the untagged three")

    # ± : the cursor paper already carries it, so T offers to untag
    t.send("/")
    t.key("esc")  # Esc clears the filter, unlike ⏎
    t.wait_for(lambda: "5/5" in t.text(), what="the filter to clear")
    t.send("g")  # back to the first row, which is one of the tagged two
    t.send("T")
    t.wait_for(lambda: "tag 1 +" in t.text(), what="the tag prompt")
    # the ± is a property of the name, so it flips while it is typed —
    # not when the prompt opens with nothing in it
    t.send("section-3")
    t.wait_for(lambda: "tag 1 -" in t.text(), what="the prompt to flip to - mode")
    require("untag" in t.text(), "the prompt should say ⏎ untags", t)
    t.key("enter")
    t.wait_for(lambda: "untagged 1 paper" in t.text(), what="the untag confirmation")
    t.wait_for(
        lambda: len(open(path).read().split()) == 1,
        what="one key left in tags/section-3",
    )
    # …and the card stops naming it for the paper it was taken from
    t.wait_for(
        lambda: "tags  section-3" not in t.text(),
        what="the tag leaving the pub card",
    )
