# SPDX-License-Identifier: GPL-2.0-or-later
"""Regression: WorldSnapshot must be parsed from the wrapped harness response.

Harness tool calls return {"ok": True, "result": {<payload>}, ...} per
kb_87a7eade — the snapshot fetcher must unwrap before reading the
tool-specific keys ("players", "bots"). The pre-fix code read
players_raw.get("players") directly, which silently returned [] and made
SubsetGate always see an empty world.

In-game evidence: with the bug, Puun on map 530 produced to_full=[] even
when bot 1003 (Casmina) was on the same map. After the fix, the snapshot
correctly contains the player + bots.
"""
from brain_sidecar.app import parse_world_snapshot


def test_parse_wrapped_response_extracts_players_and_bots():
    players_raw = {
        "ok": True,
        "result": {
            "players": [
                {"player_guid": 2654, "name": "Puun", "map_id": 530,
                 "x": -3977.9, "y": -13672.0, "z": 63.7, "level": 25,
                 "in_pvp_combat": False},
            ],
        },
        "latency_ms": 5,
    }
    bots_raw = {
        "ok": True,
        "result": {
            "bots": [
                {"bot_guid": 1003, "name": "Casmina", "map_id": 530,
                 "x": -3925.0, "y": -11550.0, "z": 0.0, "level": 25,
                 "in_pvp_combat": False},
            ],
        },
    }
    snap = parse_world_snapshot(players_raw, bots_raw)
    assert len(snap.players) == 1
    assert snap.players[0].name == "Puun"
    assert snap.players[0].map_id == 530
    assert len(snap.bots) == 1
    assert snap.bots[0].bot_guid == 1003


def test_parse_unwrapped_response_still_works():
    """Backwards-compat: existing tests that pass {"players": [...]} directly
    (no "result" wrapper) must still parse correctly."""
    players_raw = {"players": [
        {"player_guid": 1, "name": "Alice", "map_id": 0,
         "x": 0.0, "y": 0.0, "z": 0.0, "level": 10, "in_pvp_combat": False},
    ]}
    bots_raw = {"bots": []}
    snap = parse_world_snapshot(players_raw, bots_raw)
    assert len(snap.players) == 1
    assert snap.players[0].name == "Alice"
    assert len(snap.bots) == 0


def test_parse_empty_result_yields_empty_snapshot():
    players_raw = {"ok": True, "result": {"players": []}}
    bots_raw = {"ok": True, "result": {"bots": []}}
    snap = parse_world_snapshot(players_raw, bots_raw)
    assert snap.players == ()
    assert snap.bots == ()


def test_parse_missing_result_falls_through():
    """If harness returns an error or unexpected shape, parsing must not crash."""
    snap = parse_world_snapshot({"ok": False, "error": "x"}, {"ok": False, "error": "x"})
    assert snap.players == ()
    assert snap.bots == ()
