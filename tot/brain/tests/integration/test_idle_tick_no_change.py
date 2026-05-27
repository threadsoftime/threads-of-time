"""5-second tick with no fresh events -> triage returns no-fire, LLM not called (after first tick)."""
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


@pytest.mark.asyncio
async def test_idle_tick_no_llm_call(temp_db_path, fake_harness, fake_memory, fake_llm):
    fake_harness.responder = lambda t, a: {
        "obs.get_state": {"in_combat": False, "level": 5, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
    }.get(t, {})
    import json as _json
    _persona = _json.dumps({"name": "C", "race": "Dwarf", "class": "Hunter", "backstory": "x",
                             "talkativeness": 0.5, "courage": 0.5, "greed": 0.5, "attitude_to_master": 0.5})
    fake_memory.responder = lambda t, a: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
        "memory.personality_get": {"persona": _persona},
        "memory.personality_set": {"ok": True},
        "memory.write": {"ok": True},
        "memory.recall": {"items": []},
        "memory.recall_about": {"items": []},
    }.get(t, {})

    # LLM returns a valid no_op so the first_tick path doesn't fail on dispatch
    fake_llm.response_json = {
        "kind": "no_op", "tool": None, "args": None,
        "confidence": 0.0, "reasoning": "nothing to do",
    }

    store = StateStore(temp_db_path)
    store.migrate()
    store.enroll(bot_guid=1, enrolled_at_ms=1_000_000, personality_seed=_card())

    cache = PersonalityCache(memory_mcp=fake_memory, ttl_s=60, capacity=8)
    triage = TriageGate(harness_mcp=fake_harness, memory_mcp=fake_memory)
    decider = Decider(
        llm_client=fake_llm, personality_cache=cache, memory_mcp=fake_memory,
        state_store=store, prompt_template="tools={tools_summary}",
    )
    dispatcher = Dispatcher(harness_mcp=fake_harness, memory_mcp=fake_memory)

    seen_records: list[dict] = []
    log_writer = type("W", (), {"write": lambda self, r: seen_records.append(r)})()
    sup = LoopSupervisor(
        triage=triage, decider=decider, dispatcher=dispatcher,
        state_store=store, tick_interval_s=0.05,
        decision_log_writer=log_writer,
    )
    sup.start(1)
    # Let it run ~4 ticks: first tick fires (first_tick), subsequent ticks should be no_change.
    await asyncio.sleep(0.30)
    await sup.stop_all()
    store.close()
    # After first_tick fires the LLM exactly once, remaining ticks should be no_change (no LLM call).
    # == 1 (not <= 1) to catch a broken first-tick path where the LLM was never called at all.
    assert fake_llm.call_count == 1
    # At least one tick recorded as no_change (subsequent ticks after first_tick)
    no_change_records = [r for r in seen_records if r.get("triage_reason") == "no_change"]
    assert len(no_change_records) >= 1, (
        f"expected at least one no_change tick; got: {[r.get('triage_reason') for r in seen_records]}"
    )
    # Regression: each tick must produce exactly ONE JSONL record (no duplicate event_ids).
    event_ids = [r["event_id"] for r in seen_records]
    unique_ids = set(event_ids)
    assert len(event_ids) == len(unique_ids), (
        f"Duplicate event_ids in JSONL — double-write bug: total={len(event_ids)} unique={len(unique_ids)}"
    )
