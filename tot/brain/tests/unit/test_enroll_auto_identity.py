"""TDD tests for Item C — /enroll auto-identity from obs.get_state.

The /enroll handler must call obs.get_state after state_store.enroll() and
override name/race/class_ on the personality_seed before calling
personality_cache.seed().  If obs.get_state fails (exception or missing
fields), enrollment must still succeed using the original seed values.
"""
from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path
from unittest.mock import AsyncMock

import pytest

from brain_sidecar.api import make_router
from brain_sidecar.models import PersonalityCard
from brain_sidecar.personality import PersonalityCache
from brain_sidecar.state import StateStore


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _seed_card(
    name: str = "Placeholder",
    race: str = "gnome",
    class_: str = "warrior",
) -> PersonalityCard:
    """Return a PersonalityCard with configurable identity fields."""
    return PersonalityCard(
        name=name,
        race=race,
        **{"class": class_},
        backstory="test backstory",
        talkativeness=0.5,
        courage=0.5,
        greed=0.5,
        attitude_to_master=0.0,
    )


def _obs_ok_response(name: str, race: str, class_: str) -> dict:
    """Return a harness-wrapped obs.get_state response matching the actual Tier0_StateDigest
    shape (kb_6c4b36a9 / Tier0_StateDigest.cpp BuildDigestJson): identity is nested under
    `self`. Project envelope: {"ok": True, "result": {"self": {...}, "location": {...}, ...}}."""
    return {
        "ok": True,
        "result": {
            "self": {
                "name": name,
                "race": race,
                "class": class_,
                "spec": "",
                "level": 10,
                "hp_pct": 100,
                "is_in_combat": False,
            },
            "location": {"map": 0, "zone": 0, "subzone": "", "position": [0, 0, 0]},
        },
    }


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture
def store(tmp_path):
    db_path = str(tmp_path / "state.sqlite")
    s = StateStore(db_path)
    s.migrate()
    yield s
    s.close()


@pytest.fixture
def memory_mcp():
    """AsyncMock for the memory MCP (needed by PersonalityCache.seed)."""
    m = AsyncMock()
    m.call.return_value = {"ok": True}
    return m


@pytest.fixture
def harness_mcp():
    """AsyncMock for the harness MCP (provides obs.get_state)."""
    m = AsyncMock()
    m.call.return_value = _obs_ok_response("Casmina", "human", "paladin")
    return m


@pytest.fixture
def supervisor():
    """Minimal stub supervisor — start() is synchronous no-op in unit tests."""
    class _Stub:
        def start(self, bot_guid: int) -> None:
            pass

        def list_active(self):
            return []

    return _Stub()


def _make_router_under_test(store, memory_mcp, harness_mcp, supervisor):
    """Build the APIRouter with harness_mcp wired in."""
    cache = PersonalityCache(memory_mcp=memory_mcp, ttl_s=60, capacity=8)
    return make_router(
        state_store=store,
        personality_cache=cache,
        supervisor=supervisor,
        brain_bearer="",          # auth disabled in tests
        harness_mcp=harness_mcp,  # Item C: new parameter
    ), cache


# ---------------------------------------------------------------------------
# Test 1: happy path — obs.get_state overrides seed identity
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_enroll_auto_populates_identity_from_obs_get_state(
    store, memory_mcp, harness_mcp, supervisor
):
    """POST /enroll with placeholder seed → obs.get_state returns real identity →
    personality_cache.seed receives the corrected card (name=Casmina, race=human, class_=paladin).
    """
    from fastapi import FastAPI
    from httpx import AsyncClient, ASGITransport

    router, cache = _make_router_under_test(store, memory_mcp, harness_mcp, supervisor)
    app = FastAPI()
    app.include_router(router)

    payload = {
        "bot_guid": 1001,
        "personality_seed": {
            "name": "Placeholder",
            "race": "gnome",
            "class": "warrior",
            "backstory": "test backstory",
            "talkativeness": 0.5,
            "courage": 0.5,
            "greed": 0.5,
            "attitude_to_master": 0.0,
        },
    }

    async with AsyncClient(transport=ASGITransport(app=app), base_url="http://test") as client:
        resp = await client.post("/enroll", json=payload)

    assert resp.status_code == 201, resp.text

    # Verify harness_mcp.call was invoked with obs.get_state for this bot_guid
    obs_calls = [
        c for c in harness_mcp.call.await_args_list
        if c.args[0] == "obs.get_state"
    ]
    assert len(obs_calls) == 1
    assert obs_calls[0].args[1] == {"target_guid": 1001}

    # Verify personality_cache.seed received the OVERRIDDEN card, not the placeholder seed.
    # memory_mcp.call is called by PersonalityCache.seed with memory.personality_set.
    seed_calls = [
        c for c in memory_mcp.call.await_args_list
        if c.args[0] == "memory.personality_set"
    ]
    assert len(seed_calls) == 1
    stored_card = PersonalityCard.model_validate(
        json.loads(seed_calls[0].args[1]["persona"])
    )
    assert stored_card.name == "Casmina", (
        f"Expected name='Casmina' (from obs.get_state) but got {stored_card.name!r}"
    )
    assert stored_card.race == "human", (
        f"Expected race='human' (from obs.get_state) but got {stored_card.race!r}"
    )
    assert stored_card.class_ == "paladin", (
        f"Expected class_='paladin' (from obs.get_state) but got {stored_card.class_!r}"
    )


# ---------------------------------------------------------------------------
# Test 2: fallback — obs.get_state raises an exception
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_enroll_falls_back_to_seed_on_obs_get_state_failure(
    store, memory_mcp, harness_mcp, supervisor
):
    """If obs.get_state raises, enrollment must still succeed (HTTP 201) and
    personality_cache.seed receives the original seed values unchanged.
    """
    from fastapi import FastAPI
    from httpx import AsyncClient, ASGITransport

    # Make harness raise on the obs.get_state call
    harness_mcp.call.side_effect = RuntimeError("bot offline")

    router, cache = _make_router_under_test(store, memory_mcp, harness_mcp, supervisor)
    app = FastAPI()
    app.include_router(router)

    payload = {
        "bot_guid": 1002,
        "personality_seed": {
            "name": "Placeholder",
            "race": "gnome",
            "class": "warrior",
            "backstory": "test backstory",
            "talkativeness": 0.5,
            "courage": 0.5,
            "greed": 0.5,
            "attitude_to_master": 0.0,
        },
    }

    async with AsyncClient(transport=ASGITransport(app=app), base_url="http://test") as client:
        resp = await client.post("/enroll", json=payload)

    # Enrollment must succeed even when harness is down
    assert resp.status_code == 201, (
        f"Enrollment must succeed on harness failure but got {resp.status_code}: {resp.text}"
    )

    # Verify seed received the ORIGINAL placeholder values (not overridden)
    seed_calls = [
        c for c in memory_mcp.call.await_args_list
        if c.args[0] == "memory.personality_set"
    ]
    assert len(seed_calls) == 1
    stored_card = PersonalityCard.model_validate(
        json.loads(seed_calls[0].args[1]["persona"])
    )
    assert stored_card.name == "Placeholder", (
        f"Expected seed fallback name='Placeholder' but got {stored_card.name!r}"
    )
    assert stored_card.race == "gnome", (
        f"Expected seed fallback race='gnome' but got {stored_card.race!r}"
    )
    assert stored_card.class_ == "warrior", (
        f"Expected seed fallback class_='warrior' but got {stored_card.class_!r}"
    )


# ---------------------------------------------------------------------------
# Test 3: fallback — obs.get_state returns partial/missing fields
#
# ALL-OR-NOTHING strategy: if any of name/race/class is missing from the
# obs.get_state result, keep ALL seed values (don't partially override).
# This avoids a mixed-source card that has, e.g., a real race but a
# placeholder name.
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_enroll_falls_back_to_seed_on_obs_get_state_missing_fields(
    store, memory_mcp, harness_mcp, supervisor
):
    """If obs.get_state response is missing the 'name' field, enrollment must
    succeed and ALL seed fields (name, race, class_) must be preserved
    (ALL-OR-NOTHING fallback — no partial override).
    """
    from fastapi import FastAPI
    from httpx import AsyncClient, ASGITransport

    # Partial response: 'name' is absent from self, race and class are present
    harness_mcp.call.return_value = {
        "ok": True,
        "result": {
            "self": {
                "race": "human",
                "class": "paladin",
                # 'name' deliberately missing
                "level": 10,
                "is_in_combat": False,
            },
            "location": {"map": 0, "zone": 0, "subzone": "", "position": [0, 0, 0]},
        },
    }

    router, cache = _make_router_under_test(store, memory_mcp, harness_mcp, supervisor)
    app = FastAPI()
    app.include_router(router)

    payload = {
        "bot_guid": 1003,
        "personality_seed": {
            "name": "Placeholder",
            "race": "gnome",
            "class": "warrior",
            "backstory": "test backstory",
            "talkativeness": 0.5,
            "courage": 0.5,
            "greed": 0.5,
            "attitude_to_master": 0.0,
        },
    }

    async with AsyncClient(transport=ASGITransport(app=app), base_url="http://test") as client:
        resp = await client.post("/enroll", json=payload)

    assert resp.status_code == 201, (
        f"Enrollment must succeed on missing fields but got {resp.status_code}: {resp.text}"
    )

    seed_calls = [
        c for c in memory_mcp.call.await_args_list
        if c.args[0] == "memory.personality_set"
    ]
    assert len(seed_calls) == 1
    stored_card = PersonalityCard.model_validate(
        json.loads(seed_calls[0].args[1]["persona"])
    )
    # ALL-OR-NOTHING: since 'name' was missing, all three fields stay as seed
    assert stored_card.name == "Placeholder", (
        f"ALL-OR-NOTHING: missing 'name' should keep seed name='Placeholder', got {stored_card.name!r}"
    )
    assert stored_card.race == "gnome", (
        f"ALL-OR-NOTHING: missing 'name' should keep seed race='gnome', got {stored_card.race!r}"
    )
    assert stored_card.class_ == "warrior", (
        f"ALL-OR-NOTHING: missing 'name' should keep seed class_='warrior', got {stored_card.class_!r}"
    )


# ---------------------------------------------------------------------------
# Test 4: fallback — obs.get_state returns tool-level failure (ok: False)
#
# The harness does NOT raise an exception on tool-level errors; instead it
# returns {"ok": False, "error": "..."} as a plain dict.  The envelope-unwrap
# path in the handler (`raw.get("result", raw)`) falls back to the full dict,
# which has no name/race/class keys, so the ALL-OR-NOTHING guard kicks in and
# the original seed values are preserved.  This test documents that the
# existing code already handles this path correctly — no exception needed.
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_enroll_falls_back_to_seed_on_obs_get_state_tool_error(
    store, memory_mcp, harness_mcp, supervisor, caplog
):
    """obs.get_state returns {"ok": False, "error": "bot not found"} (no exception).
    Enrollment must succeed (HTTP 201) and personality_cache.seed must receive
    the ORIGINAL seed values (Placeholder/gnome/warrior), not live values.
    A warning must be logged.
    """
    import logging
    from fastapi import FastAPI
    from httpx import AsyncClient, ASGITransport

    # Silent tool-level failure — no exception, just an error envelope
    harness_mcp.call.return_value = {"ok": False, "error": "bot not found"}

    router, cache = _make_router_under_test(store, memory_mcp, harness_mcp, supervisor)
    app = FastAPI()
    app.include_router(router)

    payload = {
        "bot_guid": 1004,
        "personality_seed": {
            "name": "Placeholder",
            "race": "gnome",
            "class": "warrior",
            "backstory": "test backstory",
            "talkativeness": 0.5,
            "courage": 0.5,
            "greed": 0.5,
            "attitude_to_master": 0.0,
        },
    }

    with caplog.at_level(logging.WARNING, logger="brain_sidecar.api"):
        async with AsyncClient(transport=ASGITransport(app=app), base_url="http://test") as client:
            resp = await client.post("/enroll", json=payload)

    # Enrollment must succeed despite tool-level failure
    assert resp.status_code == 201, (
        f"Enrollment must succeed on ok:False tool error but got {resp.status_code}: {resp.text}"
    )

    # Verify seed received the ORIGINAL placeholder values (not overridden)
    seed_calls = [
        c for c in memory_mcp.call.await_args_list
        if c.args[0] == "memory.personality_set"
    ]
    assert len(seed_calls) == 1
    stored_card = PersonalityCard.model_validate(
        json.loads(seed_calls[0].args[1]["persona"])
    )
    assert stored_card.name == "Placeholder", (
        f"Expected seed fallback name='Placeholder' but got {stored_card.name!r}"
    )
    assert stored_card.race == "gnome", (
        f"Expected seed fallback race='gnome' but got {stored_card.race!r}"
    )
    assert stored_card.class_ == "warrior", (
        f"Expected seed fallback class_='warrior' but got {stored_card.class_!r}"
    )

    # A warning must be logged (missing-fields branch fires, not exception branch)
    assert any("missing identity fields" in r.message for r in caplog.records), (
        f"Expected a warning about missing identity fields; got records: {[r.message for r in caplog.records]}"
    )


# ---------------------------------------------------------------------------
# Test 5: regression — top-level identity fields (no `self` wrapper) must NOT
# be picked up.  This was the V3.3 ship bug (kb_ef9ed459 deploy-caught #2):
# triage assumed top-level identity, but Tier0_StateDigest BuildDigestJson
# nests identity under `self`.  Fix in V3.3.1 reads from `obs["self"]`.
# This test pins the contract so a future refactor cannot regress to the
# top-level shape silently.
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_enroll_falls_back_when_identity_at_top_level_no_self_wrapper(
    store, memory_mcp, harness_mcp, supervisor, caplog
):
    """V3.3.1 regression test: if obs.get_state returned identity at the TOP level
    (without a `self` wrapper), the handler MUST fall back to seed values — not
    accidentally read them as live identity. This codifies the Tier0 nested-shape
    contract.
    """
    import logging
    from fastapi import FastAPI
    from httpx import AsyncClient, ASGITransport

    caplog.set_level(logging.WARNING)

    # Legacy buggy shape: identity at top level, no `self` wrapper.
    harness_mcp.call.return_value = {
        "ok": True,
        "result": {
            "name": "Casmina",
            "race": "human",
            "class": "paladin",
            "level": 25,
        },
    }

    router, cache = _make_router_under_test(store, memory_mcp, harness_mcp, supervisor)
    app = FastAPI()
    app.include_router(router)

    payload = {
        "bot_guid": 1099,
        "personality_seed": {
            "name": "Placeholder",
            "race": "gnome",
            "class": "warrior",
            "backstory": "test backstory",
            "talkativeness": 0.5,
            "courage": 0.5,
            "greed": 0.5,
            "attitude_to_master": 0.0,
        },
    }

    async with AsyncClient(transport=ASGITransport(app=app), base_url="http://test") as client:
        resp = await client.post("/enroll", json=payload)

    assert resp.status_code == 201, (
        f"Enrollment must succeed even with legacy shape but got {resp.status_code}: {resp.text}"
    )

    seed_calls = [c for c in memory_mcp.call.await_args_list if c.args[0] == "memory.personality_set"]
    assert len(seed_calls) == 1
    stored_card = PersonalityCard.model_validate(json.loads(seed_calls[0].args[1]["persona"]))
    # MUST fall back to seed — identity at top level (no self wrapper) is NOT
    # the documented Tier0 shape and must be ignored.
    assert stored_card.name == "Placeholder", (
        f"Top-level name without 'self' wrapper MUST be ignored — got {stored_card.name!r}"
    )
    assert stored_card.race == "gnome", (
        f"Top-level race without 'self' wrapper MUST be ignored — got {stored_card.race!r}"
    )
    assert stored_card.class_ == "warrior", (
        f"Top-level class without 'self' wrapper MUST be ignored — got {stored_card.class_!r}"
    )


# ---------------------------------------------------------------------------
# V3.6: /enroll calls morph_personality before personality_cache.seed
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_enroll_calls_morph_and_persists_v2_fields():
    """POST /enroll on a fresh bot triggers morph; persisted card has v2 fields."""
    from fastapi import FastAPI
    from fastapi.testclient import TestClient
    from unittest.mock import AsyncMock, MagicMock
    from brain_sidecar.api import make_router
    from brain_sidecar.models import PersonalityCard

    # Mock the LLM to return predictable v2 values
    llm = AsyncMock()
    llm.chat_completion_json.return_value = (
        {
            "pvp_appetite": 0.77, "raid_appetite": 0.66,
            "completionist_streak": 0.55, "gold_motivation": 0.44,
            "profession_appetite": 0.33,
        },
        "raw", 50.0,
    )

    # Mock personality_cache.seed; capture the card it receives
    seeded_cards: list[PersonalityCard] = []

    async def seed_side(bot_guid, card):
        seeded_cards.append(card)

    pc = AsyncMock()
    pc.seed.side_effect = seed_side

    # Mock state_store / supervisor / harness as the existing test does
    ss = MagicMock()
    ss.get_bot.return_value = None  # not already enrolled
    sup = MagicMock()
    sup.start = MagicMock()

    app = FastAPI()
    app.include_router(make_router(
        state_store=ss, personality_cache=pc, supervisor=sup,
        brain_bearer="", harness_mcp=None, llm_client=llm,
    ))

    client = TestClient(app)
    response = client.post("/enroll", json={
        "bot_guid": 7777,
        "personality_seed": {
            "name": "TestBot", "race": "Human", "class": "Mage",
            "backstory": "A scholar with mercantile leanings.",
            "talkativeness": 0.5, "courage": 0.5, "greed": 0.5,
            "attitude_to_master": 0.0,
        },
    })
    assert response.status_code == 201
    assert len(seeded_cards) == 1
    seeded = seeded_cards[0]
    assert seeded.pvp_appetite == 0.77
    assert seeded.profession_appetite == 0.33


@pytest.mark.asyncio
async def test_enroll_continues_when_morph_llm_down():
    """/enroll succeeds even if LLM is down (morph keeps random seed values)."""
    from fastapi import FastAPI
    from fastapi.testclient import TestClient
    from unittest.mock import AsyncMock, MagicMock
    from brain_sidecar.api import make_router

    llm = AsyncMock()
    llm.chat_completion_json.side_effect = RuntimeError("connection refused")

    seeded_cards = []

    async def seed_side(bot_guid, card):
        seeded_cards.append(card)

    pc = AsyncMock()
    pc.seed.side_effect = seed_side
    ss = MagicMock(); ss.get_bot.return_value = None
    sup = MagicMock(); sup.start = MagicMock()

    app = FastAPI()
    app.include_router(make_router(
        state_store=ss, personality_cache=pc, supervisor=sup,
        brain_bearer="", harness_mcp=None, llm_client=llm,
    ))
    client = TestClient(app)
    response = client.post("/enroll", json={
        "bot_guid": 7778,
        "personality_seed": {
            "name": "TestBot2", "race": "Human", "class": "Mage",
            "backstory": "x", "talkativeness": 0.5, "courage": 0.5,
            "greed": 0.5, "attitude_to_master": 0.0,
        },
    })
    assert response.status_code == 201
    assert len(seeded_cards) == 1
    # Seed values populated (within [0.3, 0.8])
    seeded = seeded_cards[0]
    assert seeded.pvp_appetite is not None
    assert 0.3 <= seeded.pvp_appetite <= 0.8
