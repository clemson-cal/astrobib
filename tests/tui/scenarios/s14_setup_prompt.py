"""With no ADS token saved, S prompts for the token (then email) right
in the footer, persists both to state.json, and resumes the query
prompt."""

import json
import os

from driver import require

DESCRIPTION = "S with no token: in-app token/email setup"


def run(t):
    t.send("S")
    t.wait_for(lambda: "ADS API token:" in t.text(), what="token prompt")
    t.send("FAKETOKEN123")
    t.key("enter")
    t.wait_for(lambda: "email (optional" in t.text(), what="email prompt")
    t.send("jane@example.edu")
    t.key("enter")
    t.wait_for(lambda: "ADS query:" in t.text(), what="query prompt resumes")
    state = json.load(open(os.path.join(t.state_dir, "state.json")))
    require(state.get("ads_token") == "FAKETOKEN123", f"token not saved: {state}", t)
    require(state.get("email") == "jane@example.edu", f"email not saved: {state}", t)
    t.key("esc")
