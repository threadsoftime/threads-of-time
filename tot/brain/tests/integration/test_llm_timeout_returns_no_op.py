"""LLM returns None (unparseable) -> decider emits no_op -> dispatch records no_op."""
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
async def test_llm_timeout_returns_no_op(temp_db_path, fake_harness, fake_memory, fake_llm):
    fake_harness.responder = lambda t, a: {
        "obs.get_state": {"in_combat": False, "level": 5, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
    }.get(t, {"ok": True})
    import json as _json
    _persona = _json.dumps({"name": "C", "race": "Dwarf", "class": "Hunter", "backstory": "x",
                             "talkativeness": 0.5, "courage": 0.5, "greed": 0.5, "attitude_to_master": 0.5})
    fake_memory.responder = lambda t, a: {
        # Real memory-sidecar schema: "text" field (not "content"). ts field (not ts_ms).
        "memory.search": {"items": [{"id": "m1",
                                     "text": "received whisper from player1: hey",
                                     "ts": 1_004}]},
        "goals.list": {"items": []},
        "memory.personality_get": {"persona": _persona},
        "memory.personality_set": {"ok": True},
        "memory.recall_about": {"items": []},
        "memory.recall": {"items": []},
        "memory.write": {"ok": True},
    }.get(t, {})

    # LLM returns None (parseable=False) on every call.
    # Decider retries once (max_retries=1); both attempts return None.
    # After retry exhausted -> Decision{no_op, "llm_unparseable"}.
    fake_llm.response_json = None

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
    seen: list[dict] = []
    sup = LoopSupervisor(
        triage=triage, decider=decider, dispatcher=dispatcher,
        state_store=store, tick_interval_s=0.05,
        decision_log_writer=type("W", (), {"write": lambda self, r: seen.append(r)})(),
    )
    sup.start(1)
    await asyncio.sleep(0.30)
    await sup.stop_all()
    store.close()

    # At least one record with dispatch_result=no_op AND decision_kind=no_op (from the LLM path)
    no_ops = [r for r in seen
              if r.get("dispatch_result") == "no_op" and r.get("decision_kind") == "no_op"]
    assert len(no_ops) >= 1, f"expected no_op via LLM path; records: {seen}"
    # No tool-execution calls (no bot.set_strategy, no memory.delete, etc.)
    executed = [r for r in seen if r.get("dispatch_result") == "executed"]
    assert executed == []
    # The dispatcher must write a memory.write carrying "llm_unparseable" in its payload.
    # New schema: text contains the summary; metadata contains the raw payload dict.
    unparseable_writes = [
        c for c in fake_memory.calls
        if c[0] == "memory.write"
        and "llm_unparseable" in str(c[1].get("text", "")) + str(c[1].get("metadata", {}))
    ]
    assert len(unparseable_writes) >= 1, (
        "expected at least one memory.write with llm_unparseable in payload"
    )
