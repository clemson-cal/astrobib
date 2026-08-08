"""The @ panel says where the ADS token came from and what it has spent.

An ADS token is metered daily, and the meter is invisible: nothing in the
protocol tells you how much of the allowance is gone until a call comes
back refusing to work. The rate-limit headers of every response carry it,
so the app keeps the last reading — per endpoint, since ADS meters each
one separately and the numbers must never be added up — and persists it,
so a run that has made no call can still report the day so far.

Seeded here rather than earned: the scenario makes no network call, and
the figures it reads back are the ones an earlier session recorded.
"""

import json
import os
import time

from driver import require

DESCRIPTION = "@ panel reports the ADS token's source and daily use"


def _pre_launch(state_dir):
    now = int(time.time())
    with open(os.path.join(state_dir, "state.json"), "w") as f:
        json.dump(
            {
                "version": 1,
                "ads_token": "not-a-real-token",
                "ads_quota": {
                    "search": {
                        "limit": 5000,
                        "remaining": 4988,
                        "reset": now + 4 * 3600 + 12 * 60,
                    },
                    # a window that has already rolled over: history, not
                    # the state of the allowance now
                    "export": {"limit": 100, "remaining": 97, "reset": now - 60},
                },
            },
            f,
        )


PRE_LAUNCH = _pre_launch


def run(t):
    t.send("@")
    t.wait_for("report a bug", what="the about panel")

    require(
        "ADS token  from state.json" in t.text(),
        f"the panel should name where the token came from:\n{t.text()}",
        t,
    )
    # used, not remaining: the question is what the day has cost
    require(
        "ADS use    search 12/5000 · resets in 4h 12m" in t.text(),
        f"the panel should report the search endpoint's day:\n{t.text()}",
        t,
    )
    # the export window reset since it was recorded, so nothing is known
    # to be spent in the window running now — and no reset is promised
    require(
        "export 0/100" in t.text() and "export 3/100" not in t.text(),
        f"a rolled-over window should read as spent nothing:\n{t.text()}",
        t,
    )
    require(
        "export 0/100 ·" not in t.text(),
        f"a rolled-over window has no reset to count down to:\n{t.text()}",
        t,
    )

    t.key("esc")
    t.wait_gone("report a bug")
