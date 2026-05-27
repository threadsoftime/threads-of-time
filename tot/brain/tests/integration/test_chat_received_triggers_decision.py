"""Fresh chat memory -> triage fires -> decider emits action -> dispatcher executes."""
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
async def test_chat_triggers_action_and_dispatch(
    temp_db_path, fake_harness, fake_memory, fake_llm
):
    fake_harness.responder = lambda t, a: {
        "obs.get_state": {"in_combat": False, "level": 5, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
        "bot.set_strategy": {"ok": True},
    }.get(t, {"ok": True})
    import json as _json
    _persona = _json.dumps({"name": "C", "race": "Dwarf", "class": "Hunter", "backstory": "x",
                             "talkativeness": 0.5, "courage": 0.5, "greed": 0.5, "attitude_to_master": 0.5})
    fake_memory.responder = lambda t, a: {
        # Real memory-sidecar schema: "text" field (not "content"). ts field (not ts_ms).
        "memory.search": {"items": [{"id": "m1",
                                     "text": "received whisper from player1: let's grind",
                                     "ts": 1_004,
                                     "from": "player1"}]},
        "goals.list": {"items": []},
        "memory.personality_get": {"persona": _persona},
        "memory.personality_set": {"ok": True},
        "memory.recall_about": {"items": []},
        "memory.recall": {"items": []},
        "memory.write": {"ok": True},
    }.get(t, {})

    fake_llm.response_json = {
        "kind": "action",
        "tool": "bot.set_strategy",
        "args": {"bot_guid": 1, "add": ["grind"]},
        "confidence": 0.85,
        "reasoning": "Player wants to grind",
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
    await asyncio.sleep(0.30)
    await sup.stop_all()
    store.close()

    # We expect at least one record with dispatch_result=executed and tool=bot.set_strategy.
    executed = [r for r in seen_records
                if r.get("dispatch_result") == "executed" and r.get("tool") == "bot.set_strategy"]
    assert len(executed) >= 1
    # bot.set_strategy was called on harness
    strat_calls = [c for c in fake_harness.calls if c[0] == "bot.set_strategy"]
    assert len(strat_calls) >= 1
