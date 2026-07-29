#!/usr/bin/env python3
"""Run the astrobib TUI integration scenarios.

    tests/tui/run.py [-k SUBSTR] [--no-build] [--list]

Bootstraps a local venv (tests/tui/.venv) with pyte on first use, builds
target/release/astrobib, then drives it through every scenario in
tests/tui/scenarios/. Exits nonzero if any scenario fails.

Network scenarios are skipped unless RUN_ADS_TESTS=1 (and, for ADS,
ADS_API_TOKEN) is set in the environment.
"""

from __future__ import annotations

import argparse
import importlib.util
import os
import subprocess
import sys
import time
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
VENV = os.path.join(HERE, ".venv")
VENV_PY = os.path.join(VENV, "bin", "python")
BINARY = os.path.join(REPO, "target", "release", "astrobib")


def bootstrap():
    """Ensure pyte is importable, re-execing into tests/tui/.venv if needed."""
    try:
        import pyte  # noqa: F401

        return
    except ImportError:
        pass
    if os.environ.get("ASTROBIB_TUI_BOOTSTRAPPED"):
        sys.exit("bootstrap loop: pyte still missing inside tests/tui/.venv")
    if not os.path.exists(VENV_PY):
        print(f"creating venv at {VENV} …", flush=True)
        subprocess.run([sys.executable, "-m", "venv", VENV], check=True)
    have_pyte = (
        subprocess.run(
            [VENV_PY, "-c", "import pyte"], capture_output=True
        ).returncode
        == 0
    )
    if not have_pyte:
        subprocess.run(
            [VENV_PY, "-m", "pip", "install", "--quiet", "pyte"], check=True
        )
    os.environ["ASTROBIB_TUI_BOOTSTRAPPED"] = "1"
    os.execv(VENV_PY, [VENV_PY, os.path.abspath(__file__)] + sys.argv[1:])


def discover():
    sdir = os.path.join(HERE, "scenarios")
    mods = []
    for name in sorted(os.listdir(sdir)):
        if not (name.startswith("s") and name.endswith(".py")):
            continue
        path = os.path.join(sdir, name)
        spec = importlib.util.spec_from_file_location(name[:-3], path)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        mods.append((name[:-3], mod))
    return mods


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("-k", metavar="SUBSTR", help="only scenarios whose name matches")
    ap.add_argument("--no-build", action="store_true", help="skip cargo build")
    ap.add_argument("--list", action="store_true", help="list scenarios and exit")
    args = ap.parse_args()

    bootstrap()
    sys.path.insert(0, HERE)
    import driver

    scenarios = discover()
    if args.k:
        scenarios = [(n, m) for n, m in scenarios if args.k in n]
    if args.list:
        for name, mod in scenarios:
            print(f"{name:28s} {getattr(mod, 'DESCRIPTION', '')}")
        return 0
    if not scenarios:
        print("no scenarios matched")
        return 1

    if not args.no_build:
        print("cargo build --release …", flush=True)
        r = subprocess.run(["cargo", "build", "--release"], cwd=REPO)
        if r.returncode != 0:
            return r.returncode
    if not os.path.exists(BINARY):
        sys.exit(f"binary not found: {BINARY}")

    failed = 0
    results = []
    for name, mod in scenarios:
        desc = getattr(mod, "DESCRIPTION", "")
        label = f"{name}  ({desc})" if desc else name
        t0 = time.monotonic()
        sess = None
        try:
            sess = driver.Session(
                BINARY,
                allow_network=getattr(mod, "ALLOW_NETWORK", False),
                manuscript=getattr(mod, "MANUSCRIPT", None),
            )
            sess.wait_ready()
            mod.run(sess)
        except driver.Skip as e:
            results.append(("SKIP", label, str(e)))
        except AssertionError as e:
            failed += 1
            results.append(("FAIL", label, str(e)))
        except Exception:
            failed += 1
            results.append(("FAIL", label, traceback.format_exc()))
        else:
            dt = time.monotonic() - t0
            results.append(("PASS", label, f"{dt:.1f}s"))
        finally:
            if sess is not None:
                sess.quit()
        status, _, note = results[-1]
        if status == "PASS":
            print(f"  PASS  {label}  [{note}]", flush=True)
        elif status == "SKIP":
            print(f"  SKIP  {label}  — {note}", flush=True)
        else:
            print(f"  FAIL  {label}\n{note}", flush=True)

    npass = sum(1 for s, _, _ in results if s == "PASS")
    nskip = sum(1 for s, _, _ in results if s == "SKIP")
    print(f"\n{npass} passed, {failed} failed, {nskip} skipped")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
