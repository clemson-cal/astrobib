#!/usr/bin/env python3
"""Run the astrobib TUI integration scenarios.

    tests/tui/run.py [-k SUBSTR] [--no-build] [--list] [-j N]

Bootstraps a local venv (tests/tui/.venv) with pyte on first use, builds
target/release/astrobib, then drives it through every scenario in
tests/tui/scenarios/. Exits nonzero if any scenario fails.

Scenarios run concurrently. They are mutually isolated by construction —
each Session owns a scratch directory, a pty and a process, and nothing
in the suite reaches outside its own sandbox — so the only shared
resource is the machine. The suite is almost entirely *waiting*: on a
repaint to land, on the app's 1.5s external-change throttle to expire.
Run serially that idle time adds up to a minute at ~8% CPU; overlapped
it collapses to roughly the slowest single scenario.

Network scenarios are skipped unless RUN_ADS_TESTS=1 (and, for ADS,
ADS_API_TOKEN) is set in the environment.
"""

from __future__ import annotations

import argparse
import concurrent.futures
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


def run_one(driver, name, mod):
    """Drive one scenario to completion. Never raises; returns a result row.

    Called from a worker thread, so it must own everything it touches:
    the Session is created and torn down here, and the only value that
    escapes is the tuple.
    """
    desc = getattr(mod, "DESCRIPTION", "")
    label = f"{name}  ({desc})" if desc else name
    t0 = time.monotonic()
    sess = None
    try:
        sess = driver.Session(
            BINARY,
            cols=getattr(mod, "COLS", 140),
            rows=getattr(mod, "ROWS", 40),
            allow_network=getattr(mod, "ALLOW_NETWORK", False),
            manuscript=getattr(mod, "MANUSCRIPT", None),
            pre_launch=getattr(mod, "PRE_LAUNCH", None),
            env_override=getattr(mod, "ENV", None),
        )
        sess.wait_ready()
        mod.run(sess)
    except driver.Skip as e:
        return ("SKIP", name, label, str(e))
    except AssertionError as e:
        return ("FAIL", name, label, str(e))
    except Exception:
        return ("FAIL", name, label, traceback.format_exc())
    else:
        return ("PASS", name, label, f"{time.monotonic() - t0:.1f}s")
    finally:
        if sess is not None:
            sess.quit()


def report(status, label, note):
    if status == "PASS":
        print(f"  PASS  {label}  [{note}]", flush=True)
    elif status == "SKIP":
        print(f"  SKIP  {label}  — {note}", flush=True)
    else:
        print(f"  FAIL  {label}\n{note}", flush=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("-k", metavar="SUBSTR", help="only scenarios whose name matches")
    ap.add_argument("--no-build", action="store_true", help="skip cargo build")
    ap.add_argument("--list", action="store_true", help="list scenarios and exit")
    ap.add_argument(
        "-j",
        "--jobs",
        type=int,
        default=min(8, os.cpu_count() or 4),
        metavar="N",
        help="scenarios to run concurrently (default: %(default)s; 1 is serial)",
    )
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

    jobs = max(1, min(args.jobs, len(scenarios)))
    wall = time.monotonic()
    results = []
    if jobs == 1:
        for name, mod in scenarios:
            results.append(run_one(driver, name, mod))
            report(results[-1][0], results[-1][2], results[-1][3])
    else:
        # Reporting happens here, in the main thread, as each future
        # lands: output stays interleaving-free without a lock, and a
        # hung scenario is visible as the gap where its line never came.
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
            futures = [pool.submit(run_one, driver, n, m) for n, m in scenarios]
            for fut in concurrent.futures.as_completed(futures):
                row = fut.result()
                results.append(row)
                report(row[0], row[2], row[3])

    counts = {s: sum(1 for r in results if r[0] == s) for s in ("PASS", "FAIL", "SKIP")}
    dt = time.monotonic() - wall
    print(
        f"\n{counts['PASS']} passed, {counts['FAIL']} failed, "
        f"{counts['SKIP']} skipped  in {dt:.1f}s across {jobs} job(s)"
    )
    if counts["FAIL"] and jobs > 1:
        # Completion order scatters the failures through the log; name
        # them again, in roster order, so the tail of the run is the
        # list of what to go and look at. Serial runs need no recap —
        # there, completion order already is roster order.
        print("\nfailed:")
        for name in sorted(r[1] for r in results if r[0] == "FAIL"):
            print(f"  {name}")
    return 1 if counts["FAIL"] else 0


if __name__ == "__main__":
    sys.exit(main())
