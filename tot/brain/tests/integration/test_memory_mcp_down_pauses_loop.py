"""Memory MCP raises on every call -> triage returns triage_timeout, no executions."""
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
async def test_memory_mcp_down_no_llm_call(temp_db_path, fake_harness, fake_memory, fake_llm):
    # Memory MCP fails on every call
    fake_memory.fail_with = RuntimeError("memory down")
    fake_harness.responder = lambda t, a: {
        "obs.get_state": {"in_combat": False, "level": 5, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
    }.get(t, {"ok": True})

    # fake_llm.response_json = None (default) — even if LLM were called it would return no_op

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
    # Run long enough for first_tick + at least 2 more ticks to attempt and timeout
    await asyncio.sleep(0.35)
    await sup.stop_all()
    store.close()

    # Several ticks should have triage_timeout (ticks after the first where memory calls fail)
    timeouts = [r for r in seen if r.get("triage_reason") == "triage_timeout"]
    assert len(timeouts) >= 1
    # No successful executions: memory MCP is down so all triage/dispatch paths fail before execute
    executions = [r for r in seen if r.get("dispatch_result") == "executed"]
    assert executions == []
    # Triage swallows the MCP exception and returns no-fire; LLM should never be invoked.
    assert fake_llm.call_count == 0, (
        f"LLM should not be called when memory MCP is down; call_count={fake_llm.call_count}"
    )
