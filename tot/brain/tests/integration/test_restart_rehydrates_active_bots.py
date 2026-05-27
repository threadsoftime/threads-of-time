"""Stopping + restarting the supervisor rehydrates 'active' bots from disk."""
from __future__ import annotations

import asyncio

import pytest

from brain_sidecar.decide import Decider
from brain_sidecar.dispatch import Dispatcher
from brain_sidecar.loop import LoopSupervisor
from brain_sidecar.models import PersonalityCard
from brain_sidecar.personality import PersonalityCache
from brain_sidecar.state import StateStore
from brain_sidecar.triage import TriageGate


def _card() -> PersonalityCard:
    return PersonalityCard(
        name="C", race="Dwarf", **{"class": "Hunter"}, backstory="x",
        talkativeness=0.5, courage=0.5, greed=0.5, attitude_to_master=0.5,
    )


def _make_supervisor(store, fake_harness, fake_memory, fake_llm):
    cache = PersonalityCache(memory_mcp=fake_memory, ttl_s=60, capacity=8)
    triage = TriageGate(harness_mcp=fake_harness, memory_mcp=fake_memory)
    decider = Decider(
        llm_client=fake_llm, personality_cache=cache, memory_mcp=fake_memory,
        state_store=store, prompt_template="tools={tools_summary}",
    )
    dispatcher = Dispatcher(harness_mcp=fake_harness, memory_mcp=fake_memory)
    return LoopSupervisor(
        triage=triage, decider=decider, dispatcher=dispatcher,
        state_store=store, tick_interval_s=0.05,
        decision_log_writer=type("W", (), {"write": lambda self, r: None})(),
    )


@pytest.mark.asyncio
async def test_restart_rehydrates_active_bots(
    temp_db_path, fake_harness, fake_memory, fake_llm,
):
    fake_harness.responder = lambda t, a: {
        "obs.get_state": {"in_combat": False, "level": 5, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
    }.get(t, {"ok": True})
    import json as _json
    _persona = _json.dumps({"name": "C", "race": "Dwarf", "class": "Hunter", "backstory": "x",
                             "talkativeness": 0.5, "courage": 0.5, "greed": 0.5, "attitude_to_master": 0.5})
    fake_memory.responder = lambda t, a: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
        "memory.personality_get": {"persona": _persona},
        "memory.personality_set": {"ok": True},
        "memory.recall": {"items": []},
        "memory.recall_about": {"items": []},
        "memory.write": {"ok": True},
    }.get(t, {})

    # LLM returns no_op so dispatch doesn't fail
    fake_llm.response_json = {
        "kind": "no_op", "tool": None, "args": None,
        "confidence": 0.0, "reasoning": "idle",
    }

    store = StateStore(temp_db_path)
    store.migrate()
    store.enroll(bot_guid=42, enrolled_at_ms=1_000_000, personality_seed=_card())
    store.enroll(bot_guid=43, enrolled_at_ms=1_000_000, personality_seed=_card())
    # Bot 43 was released before the "restart"
    store.set_status(43, "released")

    # First supervisor session
    sup1 = _make_supervisor(store, fake_harness, fake_memory, fake_llm)
    for row in store.list_active():
        sup1.start(row.bot_guid)
    await asyncio.sleep(0.10)
    await sup1.stop_all()

    # Second session (simulating restart): only bot 42 is active in the DB
    sup2 = _make_supervisor(store, fake_harness, fake_memory, fake_llm)
    for row in store.list_active():
        sup2.start(row.bot_guid)
    await asyncio.sleep(0.10)
    # Only bot 42 should be running; bot 43 (released) must NOT be re-spawned
    assert sup2.list_active() == [42]
    await sup2.stop_all()
    store.close()
