"""e prompts for a destination path in the footer and writes the
selection (or cursor entry) as one .bib file; bad paths fail with an
error, creating no intermediate directories."""

import os

from driver import require

DESCRIPTION = "e exports selection to a .bib path"


def run(t):
    # select two rows, export to an absolute scratch path
    t.send(" ")
    t.send("j")
    t.send(" ")
    t.send("e")
    t.wait_for(lambda: "export 2 →" in t.text(), what="export prompt")
    dest = os.path.join(t.scratch, "out.bib")
    # the prompt is prefilled with refs.bib — replace it wholesale
    t.send(b"\x15")  # ctrl+u clears the line
    t.send(dest)
    t.key("enter")
    t.wait_for(lambda: "Exported 2 entries" in t.text(), what="export confirmation")
    content = open(dest).read()
    require(content.count("@article{") == 2, "expected two entries in the file", t)
    t.key("esc")  # leave selection mode
    t.settle(0.2)  # lone ESC: fused with the next key it parses as alt+e
    # a path through a nonexistent directory fails, and creates nothing
    t.send("e")
    t.wait_for(lambda: "export 1 →" in t.text(), what="single-entry prompt")
    t.send(b"\x15")
    bad = os.path.join(t.scratch, "no-such-dir", "x.bib")
    t.send(bad)
    t.key("enter")
    t.wait_for(lambda: "could not write" in t.text(), what="write error")
    require(not os.path.exists(os.path.dirname(bad)), "intermediate dir was created", t)
