"""Tests for personality LRU cache (C7)."""
from __future__ import annotations

from unittest.mock import AsyncMock

import pytest

from brain_sidecar.models import PersonalityCard
from brain_sidecar.personality import PersonalityCache


def _make_card(name: str) -> PersonalityCard:
    return PersonalityCard(
        name=name, race="Dwarf", **{"class": "Hunter"}, backstory="x",
        talkativeness=0.5, courage=0.5, greed=0.5, attitude_to_master=0.5,
    )


import json


def _persona_payload(name: str = "Caedon") -> dict:
    """Return a mock memory.personality_get response: {"persona": "<json-string>"}."""
    card_dict = {
        "name": name, "race": "Dwarf", "class": "Hunter",
        "backstory": "x", "talkativeness": 0.5, "courage": 0.5,
        "greed": 0.5, "attitude_to_master": 0.5,
    }
    return {"persona": json.dumps(card_dict)}


@pytest.mark.asyncio
async def test_get_fetches_from_memory_mcp_on_miss():
    """memory.personality_get uses dot-underscore notation and bot_id as str."""
    mcp = AsyncMock()
    mcp.call.return_value = _persona_payload()
    cache = PersonalityCache(memory_mcp=mcp, ttl_s=60, capacity=8, now_fn=lambda: 1000.0)
    card = await cache.get(42)
    assert card.name == "Caedon"
    mcp.call.assert_awaited_once_with("memory.personality_get", {"bot_id": "42"})


@pytest.mark.asyncio
async def test_get_cached_avoids_second_fetch():
    mcp = AsyncMock()
    mcp.call.return_value = _persona_payload()
    cache = PersonalityCache(memory_mcp=mcp, ttl_s=60, capacity=8, now_fn=lambda: 1000.0)
    await cache.get(42)
    await cache.get(42)
    assert mcp.call.await_count == 1


@pytest.mark.asyncio
async def test_get_after_ttl_refetches():
    mcp = AsyncMock()
    mcp.call.return_value = _persona_payload()
    now = [1000.0]
    cache = PersonalityCache(memory_mcp=mcp, ttl_s=60, capacity=8, now_fn=lambda: now[0])
    await cache.get(42)
    now[0] += 61.0
    await cache.get(42)
    assert mcp.call.await_count == 2


@pytest.mark.asyncio
async def test_seed_writes_through():
    """seed calls memory.personality_set with bot_id as str and persona as JSON string."""
    mcp = AsyncMock()
    cache = PersonalityCache(memory_mcp=mcp, ttl_s=60, capacity=8, now_fn=lambda: 1000.0)
    await cache.seed(42, _make_card("Caedon"))
    mcp.call.assert_awaited_once()
    name, args = mcp.call.await_args.args
    assert name == "memory.personality_set"
    assert args["bot_id"] == "42"
    # persona is a JSON-encoded string; decode and verify the card name
    card_dict = json.loads(args["persona"])
    assert card_dict["name"] == "Caedon"


@pytest.mark.asyncio
async def test_lru_eviction_at_capacity():
    mcp = AsyncMock()
    mcp.call.return_value = _persona_payload("x")
    cache = PersonalityCache(memory_mcp=mcp, ttl_s=60, capacity=2, now_fn=lambda: 1000.0)
    await cache.get(1)
    await cache.get(2)
    await cache.get(3)  # evicts 1
    await cache.get(1)  # re-fetch (1 was evicted)
    # 1 fetched twice + 2 once + 3 once = 4
    assert mcp.call.await_count == 4


# V3.6: lock infrastructure tests
import asyncio


def test_personality_cache_accepts_llm_client():
    """V3.6: PersonalityCache constructor takes llm_client kwarg."""
    mcp = AsyncMock()
    llm = AsyncMock()
    # Should not raise
    cache = PersonalityCache(memory_mcp=mcp, ttl_s=60, capacity=8, llm_client=llm)
    assert cache is not None


@pytest.mark.asyncio
async def test_lock_for_returns_same_instance_per_bot():
    """Per-bot locks are stable across calls (so concurrent get() serializes)."""
    cache = PersonalityCache(
        memory_mcp=AsyncMock(), ttl_s=60, capacity=8, llm_client=AsyncMock()
    )
    lock_a = cache._lock_for(42)
    lock_b = cache._lock_for(42)
    assert lock_a is lock_b
    # Different bot → different lock
    lock_other = cache._lock_for(99)
    assert lock_other is not lock_a
    # Both are asyncio.Lock instances
    assert isinstance(lock_a, asyncio.Lock)


# V3.6: migration branch tests
from brain_sidecar.morph import morph_personality  # noqa: F401 (kept for symmetry/clarity)


def _v1_persona_payload() -> dict:
    """memory.personality_get response shape: a V1 persona (no v2 fields)."""
    return {"persona": json.dumps({
        "name": "Casmina", "race": "Human", "class": "Paladin",
        "backstory": "A devoted tank seeking glory in BFD.",
        "talkativeness": 0.7, "courage": 0.9, "greed": 0.3,
        "attitude_to_master": 0.6,
    })}


def _v2_persona_payload() -> dict:
    """memory.personality_get response shape: a full v2 persona."""
    return {"persona": json.dumps({
        "name": "Casmina", "race": "Human", "class": "Paladin",
        "backstory": "x", "talkativeness": 0.7, "courage": 0.9,
        "greed": 0.3, "attitude_to_master": 0.6,
        "pvp_appetite": 0.4, "raid_appetite": 0.9,
        "completionist_streak": 0.5, "gold_motivation": 0.4,
        "profession_appetite": 0.2,
    })}


@pytest.mark.asyncio
async def test_get_v1_card_triggers_migration():
    """Loading a V1 card calls morph_personality + memory.personality_set."""
    mcp = AsyncMock()
    # First call (personality_get) → V1; second call (personality_set) → ack.
    mcp.call.side_effect = [_v1_persona_payload(), {"ok": True}]
    llm = AsyncMock()
    llm.chat_completion_json.return_value = (
        {
            "pvp_appetite": 0.55, "raid_appetite": 0.88,
            "completionist_streak": 0.4, "gold_motivation": 0.3,
            "profession_appetite": 0.2,
        },
        "raw", 50.0,
    )
    cache = PersonalityCache(
        memory_mcp=mcp, ttl_s=60, capacity=8, llm_client=llm,
        now_fn=lambda: 1000.0,
    )
    card = await cache.get(1003)
    # v2 fields populated from LLM
    assert card.pvp_appetite == 0.55
    assert card.raid_appetite == 0.88
    # memory.personality_set called with the morphed card
    set_call = [c for c in mcp.call.await_args_list
                if c.args[0] == "memory.personality_set"]
    assert len(set_call) == 1
    args = set_call[0].args[1]
    assert args["bot_id"] == "1003"
    persisted = json.loads(args["persona"])
    assert persisted["pvp_appetite"] == 0.55


@pytest.mark.asyncio
async def test_get_v2_card_skips_migration():
    """A persisted full v2 card does NOT trigger morph or personality_set."""
    mcp = AsyncMock()
    mcp.call.return_value = _v2_persona_payload()
    llm = AsyncMock()
    cache = PersonalityCache(
        memory_mcp=mcp, ttl_s=60, capacity=8, llm_client=llm,
        now_fn=lambda: 1000.0,
    )
    card = await cache.get(1003)
    assert card.pvp_appetite == 0.4
    # LLM not called
    llm.chat_completion_json.assert_not_awaited()
    # Only personality_get called (no personality_set)
    assert mcp.call.await_count == 1
    assert mcp.call.await_args.args[0] == "memory.personality_get"


@pytest.mark.asyncio
async def test_get_llm_down_migration_persists_seed_values():
    """LLM down during migration → card persisted with seed values, no exception."""
    mcp = AsyncMock()
    mcp.call.side_effect = [_v1_persona_payload(), {"ok": True}]
    llm = AsyncMock()
    llm.chat_completion_json.return_value = (None, "", 100.0)
    cache = PersonalityCache(
        memory_mcp=mcp, ttl_s=60, capacity=8, llm_client=llm,
        now_fn=lambda: 1000.0,
    )
    card = await cache.get(1003)
    # v2 fields populated (from seed) — not None
    assert card.pvp_appetite is not None
    assert 0.3 <= card.pvp_appetite <= 0.8
    # personality_set was still called (we persist the seed values)
    set_call = [c for c in mcp.call.await_args_list
                if c.args[0] == "memory.personality_set"]
    assert len(set_call) == 1


@pytest.mark.asyncio
async def test_concurrent_get_same_bot_single_morph():
    """Two concurrent get() for same bot → only one morph + one personality_set."""
    mcp = AsyncMock()
    # Track call sequence: get may be called once (after lock acquire) or twice
    # (if the lock leaks). personality_set should be called exactly once.
    call_log: list[str] = []

    async def mcp_side(name, args):
        call_log.append(name)
        if name == "memory.personality_get":
            return _v1_persona_payload()
        return {"ok": True}

    mcp.call.side_effect = mcp_side

    llm_call_count = 0

    async def llm_side(**kwargs):
        nonlocal llm_call_count
        llm_call_count += 1
        # Simulate latency so concurrent callers actually contend
        await asyncio.sleep(0.05)
        return (
            {
                "pvp_appetite": 0.5, "raid_appetite": 0.5,
                "completionist_streak": 0.5, "gold_motivation": 0.5,
                "profession_appetite": 0.5,
            },
            "raw", 50.0,
        )

    llm = AsyncMock()
    llm.chat_completion_json.side_effect = llm_side

    cache = PersonalityCache(
        memory_mcp=mcp, ttl_s=60, capacity=8, llm_client=llm,
        now_fn=lambda: 1000.0,
    )
    # Fire two concurrent get() for the same bot
    results = await asyncio.gather(cache.get(1003), cache.get(1003))
    assert results[0].pvp_appetite == 0.5
    assert results[1].pvp_appetite == 0.5
    # LLM called exactly once (lock serialized; second caller saw populated cache)
    assert llm_call_count == 1
    # personality_set called exactly once
    assert call_log.count("memory.personality_set") == 1
    # personality_get may be called once or twice depending on timing of the
    # double-check; both are acceptable (the contract is "single morph + set").
    assert call_log.count("memory.personality_get") in (1, 2)
