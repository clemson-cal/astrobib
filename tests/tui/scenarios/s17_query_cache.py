"""Saved queries restore their cached results at startup — instantly,
with no network and no ADS re-query."""

import json
import os

from driver import require

DESCRIPTION = "saved queries restore from cache offline"

_ARTICLE = {
    "bibcode": "2020TestA...1..1Z",
    "title": ["A cached result about kilonovae"],
    "author": ["Cachette, Q."],
    "year": "2020",
    "abstract": "From the cache, not the network.",
    "doi": [],
    "identifier": [],
    "citation_count": 7,
    "pub": "TestJ",
    "volume": "1",
    "issue": "",
    "page": ["1"],
}


def _pre_launch(state_dir):
    with open(os.path.join(state_dir, "tabs.json"), "w") as f:
        json.dump({"contexts": {"global": [
            {"id": "tt1", "query": "kilonova", "label": "kilonova", "limit": 20, "created": 0}
        ]}}, f)
    with open(os.path.join(state_dir, "query_cache.json"), "w") as f:
        json.dump({"version": 1, "tabs": {"tt1": [_ARTICLE]}}, f)


PRE_LAUNCH = _pre_launch


def run(t):
    # the restore note is a logged message that clears after ~5s, so read
    # it before anything else — and assert it for real (it used to be
    # written `… or True`, which asserted nothing at all)
    t.wait_for(
        lambda: "restored 1 saved query from cache" in t.text(),
        what="the startup 'restored … from cache' note",
    )
    t.send("]")  # into the restored query scope
    t.wait_for(
        lambda: "A cached result about kilonovae" in t.text(),
        what="the cached result row in the restored query scope",
    )
    # the card renders from cached data too — needle on the abstract,
    # which only the card draws (the author also sits in a table column)
    t.wait_for("From the cache, not the network.", what="cached abstract on the pub card")
