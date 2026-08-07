"""T tags the selection and untags it again — the same ± reading as m —
writing a plain-text file the filter can then select on with tag:."""

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
