"""High-risk tool (memory.delete) with confidence < 0.95 -> confirmation, no execution."""
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
async def test_high_risk_emits_confirmation(temp_db_path, fake_harness, fake_memory, fake_llm):
    fake_harness.responder = lambda t, a: {
        "obs.get_state": {"in_combat": False, "level": 5, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
        "bot.send_chat": {"ok": True},
    }.get(t, {"ok": True})
    import json as _json
    _persona = _json.dumps({"name": "C", "race": "Dwarf", "class": "Hunter", "backstory": "x",
                             "talkativeness": 0.5, "courage": 0.5, "greed": 0.5, "attitude_to_master": 0.5})
    fake_memory.responder = lambda t, a: {
        # Real memory-sidecar schema: "text" field (not "content"). ts field (not ts_ms).
        "memory.search": {"items": [{"id": "m1",
                                     "text": "received whisper from player1: prune old",
                                     "ts": 1_004}]},
        "goals.list": {"items": []},
        "memory.personality_get": {"persona": _persona},
        "memory.personality_set": {"ok": True},
        "memory.recall": {"items": []},
        "memory.recall_about": {"items": []},
        "memory.write": {"ok": True},
        "memory.delete": {"ok": True},
    }.get(t, {})

    fake_llm.response_json = {
        "kind": "action", "tool": "memory.delete",
        "args": {"bot_id": 1, "memory_id": "abc"},
        "confidence": 0.92,  # below high-risk threshold (0.95)
        "reasoning": "prune duplicate",
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

    # memory.delete NEVER called on memory MCP (confirmation emitted instead, not execution)
    deletes = [c for c in fake_memory.calls if c[0] in ("memory.delete", "memory_delete")]
    assert deletes == []
    # bot.send_chat WAS called (confirmation)
    confirms = [c for c in fake_harness.calls if c[0] == "bot.send_chat"]
    assert len(confirms) >= 1
    # Verify the durable pending_confirmation memory was written.
    # New schema: memory.write; memory_type at top level; tool in metadata.
    pending = [
        c for c in fake_memory.calls
        if c[0] == "memory.write"
        and c[1].get("memory_type") == "pending_confirmation"
        and c[1].get("metadata", {}).get("tool") in ("memory.delete", "memory_delete")
    ]
    assert len(pending) >= 1, "expected pending_confirmation memory write for memory.delete tool"
