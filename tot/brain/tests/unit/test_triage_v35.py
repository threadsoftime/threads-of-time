"""V3.5 triage gate tests — PARTY_INVITE_RECEIVED signal.

IMPORTANT: test_triage_fires_on_party_invite_received uses a fixture that
contains the verbatim JSON from probe P2.5 (obs.get_state on bot 1004 with
a pending invite between P2 and P3, spec §4.3, lines 558-676).
Field path: result.self.pending_group_invite (object with from_name + group_type).
"""
from __future__ import annotations

from unittest.mock import AsyncMock

import pytest

from brain_sidecar.models import TickState
from brain_sidecar.triage import TriageGate


@pytest.fixture
def harness_mcp():
    m = AsyncMock()
    m.call.return_value = {}
    return m


@pytest.fixture
def memory_mcp():
    m = AsyncMock()
    m.call.return_value = {"items": []}
    return m


# Verbatim inner result from spec §4.3 P2.5 (post-Stage-1.5 fix, 2026-05-22).
# This is what _safe_call returns after unwrapping {"ok": true, "result": {...}}.
# Source: docs/superpowers/specs/2026-05-22-v1.5-v3.5-grouping-design.md lines 563-676.
PENDING_INVITE_OBS_STATE = {
    "event_log": [],
    "goal": {
        "current": "Idle",
        "elapsed_minutes": 0,
        "params": {},
        "progress_pct": 0,
        "ttl_minutes": 0,
    },
    "inventory_highlights": {
        "bag_used": "0/72",
        "consumables": [],
        "gear_vs_level_score": 0.0,
        "junk_value_copper": 0,
    },
    "location": {
        "map": "Eastern Kingdoms",
        "near_npcs": [],
        "position": [
            -9208.78125,
            -2118.41064453125,
            67.9093017578125,
        ],
        "subzone": "Lakeshire",
        "zone": "Redridge Mountains",
    },
    "memory_hints": [
        "killed Bellygrub in Redridge Mountains",
        "killed Murloc Minor Tidecaller in Redridge Mountains",
        "killed Great Goretusk in Redridge Mountains",
    ],
    "quest_log": [
        {"id": 20, "progress": "in progress", "title": "Blackrock Menace"},
        {"id": 91, "progress": "in progress", "title": "Solomon's Law"},
        {"id": 92, "progress": "in progress", "title": "Redridge Goulash"},
        {"id": 118, "progress": "in progress", "title": "The Price of Shoes"},
        {"id": 125, "progress": "in progress", "title": "The Lost Tools"},
        {"id": 127, "progress": "in progress", "title": "Selling Fish"},
        {"id": 135, "progress": "in progress", "title": "The Defias Brotherhood"},
        {"id": 150, "progress": "in progress", "title": "Murloc Poachers"},
        {"id": 255, "progress": "in progress", "title": "Mercenaries"},
        {"id": 467, "progress": "complete, turn in", "title": "Stonegear's Search"},
    ],
    "self": {
        "class": "hunter",
        "gold_copper": 7471398,
        "hp_pct": 100,
        "is_dead": False,
        "is_in_combat": False,
        "is_resting": False,
        "level": 21,
        "mana_pct": 100,
        "name": "Ellian",
        "pending_group_invite": {
            "from_name": "Casmina",
            "group_type": "party",
        },
        "race": "draenei",
        "spec": "",
    },
    "social": {
        "group_members": [],
        "guild": None,
        "in_group": False,
        "nearby_humans": [],
        "recent_whispers": [],
    },
}

# Verbatim inner result from spec §4.3 P3 follow-up (Ellian post-accept, grouped).
# pending_group_invite is null after acceptance — used as the no-invite baseline.
# Source: docs/superpowers/specs/2026-05-22-v1.5-v3.5-grouping-design.md lines 877-994.
NO_INVITE_OBS_STATE = {
    "event_log": [],
    "goal": {
        "current": "Idle",
        "elapsed_minutes": 0,
        "params": {},
        "progress_pct": 0,
        "ttl_minutes": 0,
    },
    "inventory_highlights": {
        "bag_used": "0/72",
        "consumables": [],
        "gear_vs_level_score": 0.0,
        "junk_value_copper": 0,
    },
    "location": {
        "map": "Eastern Kingdoms",
        "near_npcs": [],
        "position": [
            -9208.78125,
            -2118.41064453125,
            67.9093017578125,
        ],
        "subzone": "Lakeshire",
        "zone": "Redridge Mountains",
    },
    "memory_hints": [
        "killed Bellygrub in Redridge Mountains",
        "killed Murloc Minor Tidecaller in Redridge Mountains",
        "killed Great Goretusk in Redridge Mountains",
    ],
    "quest_log": [
        {"id": 20, "progress": "in progress", "title": "Blackrock Menace"},
        {"id": 91, "progress": "in progress", "title": "Solomon's Law"},
        {"id": 92, "progress": "in progress", "title": "Redridge Goulash"},
        {"id": 118, "progress": "in progress", "title": "The Price of Shoes"},
        {"id": 125, "progress": "in progress", "title": "The Lost Tools"},
        {"id": 127, "progress": "in progress", "title": "Selling Fish"},
        {"id": 135, "progress": "in progress", "title": "The Defias Brotherhood"},
        {"id": 150, "progress": "in progress", "title": "Murloc Poachers"},
        {"id": 255, "progress": "in progress", "title": "Mercenaries"},
        {"id": 467, "progress": "complete, turn in", "title": "Stonegear's Search"},
    ],
    "self": {
        "class": "hunter",
        "gold_copper": 7471398,
        "hp_pct": 100,
        "is_dead": False,
        "is_in_combat": False,
        "is_resting": False,
        "level": 21,
        "mana_pct": 100,
        "name": "Ellian",
        "pending_group_invite": None,
        "race": "draenei",
        "spec": "",
    },
    "social": {
        "group_members": ["Casmina"],
        "guild": None,
        "in_group": True,
        "nearby_humans": [],
        "recent_whispers": [],
    },
}


@pytest.mark.asyncio
async def test_triage_fires_on_party_invite_received(harness_mcp, memory_mcp):
    """obs_state with a pending invite must cause triage to fire with reason='party_invite_received'.

    Fixture uses verbatim P2.5 envelope (spec §4.3, post-Stage-1.5).
    Field path: obs_state["self"]["pending_group_invite"] — object when pending.
    """
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": PENDING_INVITE_OBS_STATE,
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1004, last_tick_ms=1_000_000)
    result = await gate.evaluate(bot_guid=1004, last_state=last_state, now_ms=1_005_000)
    assert result.should_decide is True
    assert result.reason == "party_invite_received"
    assert "pending_invite" in result.hot_inputs
    assert result.hot_inputs["pending_invite"] is not None


@pytest.mark.asyncio
async def test_triage_no_fire_when_no_invite(harness_mcp, memory_mcp):
    """obs_state without the invite field must not fire party_invite_received."""
    # Use a minimal state that is clearly not the PENDING_INVITE_OBS_STATE shape.
    # This test does NOT require P2.5 — it only needs "no invite" state.
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": {"self": {"level": 20, "in_group": False}},
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1004, last_tick_ms=1_000_000)
    result = await gate.evaluate(bot_guid=1004, last_state=last_state, now_ms=1_005_000)
    assert result.reason != "party_invite_received"


@pytest.mark.asyncio
async def test_triage_invite_hot_inputs_forwarded(harness_mcp, memory_mcp):
    """hot_inputs['pending_invite'] must carry the raw invite value from obs_state.

    Fixture uses verbatim P2.5 envelope (spec §4.3, post-Stage-1.5).
    """
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": PENDING_INVITE_OBS_STATE,
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1004, last_tick_ms=1_000_000)
    result = await gate.evaluate(bot_guid=1004, last_state=last_state, now_ms=1_005_000)
    assert result.hot_inputs.get("pending_invite") is not None
