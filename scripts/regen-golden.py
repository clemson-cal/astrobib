#!/usr/bin/env python3
"""Regenerate the golden parity vectors from the retired Python implementation.

The Rust port's parity tests (tests/golden_keys.rs, tests/golden_format.rs)
compare against vectors generated from the Python implementation that lives
at tag v0.4.0 — the last Python release. This script re-derives the expected
outputs the same way the vectors were originally produced:

1. creates a temporary git worktree checked out at v0.4.0,
2. builds a python3.11 venv there and `pip install -e`s the Python package,
3. re-runs astrobib.keys.generate_key and astrobib.library.format_bib_entry
   over the `data` maps of the committed vectors (the data maps — real-library
   records plus hand-written edge cases — are the fixed inputs; only the
   expected outputs are recomputed),
4. serializes with json.dumps(..., indent=1), byte-identical to the committed
   formatting, and diffs against tests/golden_keys.json / golden_format.json.

Exit status 0 when the committed vectors are identical to the regenerated
ones, 1 when they differ. Nothing in the repo is modified unless --write is
given, and even then the change is left uncommitted — inspect the diff and
commit deliberately.

    scripts/regen-golden.py [--write] [--keep]

--keep leaves the temporary worktree and venv behind for inspection.
"""

from __future__ import annotations

import argparse
import difflib
import json
import os
import shutil
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TAG = "v0.4.0"
VECTORS = ["golden_keys.json", "golden_format.json"]

# Runs inside the v0.4.0 venv, with the v0.4.0 worktree on sys.path via the
# editable install. argv: keys_in format_in keys_out format_out
GENERATOR = r"""
import json, sys
from astrobib.keys import generate_key
from astrobib.library import format_bib_entry

keys_in, format_in, keys_out, format_out = sys.argv[1:5]

with open(keys_in) as f:
    vectors = json.load(f)          # dicts preserve field order
out = [{"data": v["data"], "expected_key": generate_key(v["data"])}
       for v in vectors]
with open(keys_out, "w") as f:
    f.write(json.dumps(out, indent=1))

with open(format_in) as f:
    vectors = json.load(f)
out = [{"data": v["data"], "formatted": format_bib_entry(v["data"])}
       for v in vectors]
with open(format_out, "w") as f:
    f.write(json.dumps(out, indent=1))
"""


def sh(cmd, **kw):
    kw.setdefault("check", True)
    return subprocess.run(cmd, **kw)


def find_python():
    for cand in ("/opt/homebrew/bin/python3.11", "python3.11", "python3"):
        path = cand if os.path.isabs(cand) else shutil.which(cand)
        if path and os.path.exists(path):
            return path
    sys.exit("no python3 interpreter found")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--write",
        action="store_true",
        help="overwrite tests/*.json with the regenerated vectors (uncommitted)",
    )
    ap.add_argument(
        "--keep",
        action="store_true",
        help="keep the temporary worktree/venv for inspection",
    )
    args = ap.parse_args()

    tmp = tempfile.mkdtemp(prefix="astrobib-golden-")
    wt = os.path.join(tmp, "v040")
    try:
        print(f"adding worktree at {TAG} → {wt}")
        sh(["git", "-C", REPO, "worktree", "add", "--detach", wt, TAG],
           stdout=subprocess.DEVNULL)

        py = find_python()
        venv = os.path.join(tmp, "venv")
        vpy = os.path.join(venv, "bin", "python")
        print(f"building venv with {py}")
        sh([py, "-m", "venv", venv])
        print("pip install -e (v0.4.0) …")
        sh([vpy, "-m", "pip", "install", "--quiet", "-e", wt])

        ins = [os.path.join(REPO, "tests", n) for n in VECTORS]
        outs = [os.path.join(tmp, n) for n in VECTORS]
        print("regenerating vectors from the Python implementation …")
        sh([vpy, "-c", GENERATOR, ins[0], ins[1], outs[0], outs[1]], cwd=wt)

        differs = False
        for committed, regen, name in zip(ins, outs, VECTORS):
            with open(committed, "rb") as f:
                a = f.read()
            with open(regen, "rb") as f:
                b = f.read()
            if a == b:
                n = len(json.loads(b))
                print(f"  tests/{name}: identical ({n} vectors)")
                continue
            differs = True
            print(f"  tests/{name}: DIFFERS")
            diff = difflib.unified_diff(
                a.decode().splitlines(keepends=True),
                b.decode().splitlines(keepends=True),
                fromfile=f"committed/{name}",
                tofile=f"regenerated/{name}",
            )
            shown = list(diff)[:80]
            sys.stdout.writelines(shown)
            if args.write:
                shutil.copyfile(regen, committed)
                print(f"  wrote regenerated vectors to tests/{name} (not committed)")

        if differs:
            print("\nRegenerated vectors differ from the committed ones.")
            print("The Rust and Python implementations no longer agree on the")
            print("expected outputs — investigate before touching the vectors.")
            return 1
        print("\nCommitted vectors match the v0.4.0 Python implementation.")
        return 0
    finally:
        if args.keep:
            print(f"keeping {tmp}")
        else:
            subprocess.run(
                ["git", "-C", REPO, "worktree", "remove", "--force", wt],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
