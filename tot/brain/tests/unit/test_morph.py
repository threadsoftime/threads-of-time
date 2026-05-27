"""V3.6: morph_personality — random-seed + LLM-morph + failure fallback."""
from __future__ import annotations

import random
from unittest.mock import AsyncMock

import pytest

from brain_sidecar.models import PersonalityCard
from brain_sidecar.morph import morph_personality, _needs_morph, _seed_values


def _v1_card(**overrides) -> PersonalityCard:
    """A V1-shaped card with all v2 fields = None."""
    base = dict(
        name="Caedon", race="Dwarf", backstory="A scout.",
        talkativeness=0.5, courage=0.6, greed=0.3, attitude_to_master=0.4,
    )
    base.update(overrides)
    return PersonalityCard.model_validate({**base, "class": "Hunter"})


def _full_card() -> PersonalityCard:
    """A fully-populated v2 card."""
    return PersonalityCard.model_validate({
        "name": "Casmina", "race": "Human", "class": "Paladin",
        "backstory": "A tank.", "talkativeness": 0.7, "courage": 0.9,
        "greed": 0.3, "attitude_to_master": 0.6,
        "party_invite_policy": "accept_always",
        "pvp_appetite": 0.4, "raid_appetite": 0.9,
        "completionist_streak": 0.5, "gold_motivation": 0.4,
        "profession_appetite": 0.2,
    })


def test_needs_morph_true_when_any_v2_field_none():
    card = _v1_card()
    assert _needs_morph(card) is True


def test_needs_morph_false_when_all_v2_fields_set():
    card = _full_card()
    assert _needs_morph(card) is False


def test_needs_morph_true_on_partial_fill():
    """If 3 of 5 v2 fields are set, morph still triggers (overwrites all 5)."""
    full = _full_card()
    full.gold_motivation = None
    full.profession_appetite = None
    assert _needs_morph(full) is True


def test_seed_values_in_band():
    """Random seed values stay in [0.3, 0.8] per spec §6.2."""
    rng = random.Random(42)
    seed = _seed_values(rng)
    assert set(seed.keys()) == {
        "pvp_appetite", "raid_appetite", "completionist_streak",
        "gold_motivation", "profession_appetite",
    }
    for k, v in seed.items():
        assert 0.3 <= v <= 0.8, f"{k}={v} out of [0.3, 0.8]"


def test_seed_values_deterministic_with_seeded_rng():
    """Same rng seed → same values (lets tests inject deterministic RNG)."""
    seed_a = _seed_values(random.Random(42))
    seed_b = _seed_values(random.Random(42))
    assert seed_a == seed_b


@pytest.mark.asyncio
async def test_morph_noop_on_full_card():
    """A fully-populated v2 card returns unchanged; LLM not called."""
    llm = AsyncMock()
    card = _full_card()
    result = await morph_personality(card, llm)
    assert result.pvp_appetite == 0.4
    assert result.raid_appetite == 0.9
    llm.chat_completion_json.assert_not_awaited()


@pytest.mark.asyncio
async def test_morph_calls_llm_with_backstory_and_seed():
    """LLM is called with system+user prompts; user contains backstory and seed JSON."""
    llm = AsyncMock()
    llm.chat_completion_json.return_value = (
        {
            "pvp_appetite": 0.95, "raid_appetite": 0.1,
            "completionist_streak": 0.5, "gold_motivation": 0.2,
            "profession_appetite": 0.3,
        },
        "raw_text", 42.0,
    )
    card = _v1_card(backstory="A hardened PvP veteran.")
    rng = random.Random(42)
    result = await morph_personality(card, llm, rng=rng)
    # LLM was called once with json_schema
    llm.chat_completion_json.assert_awaited_once()
    call_kwargs = llm.chat_completion_json.await_args.kwargs
    assert "A hardened PvP veteran." in call_kwargs["user"]
    assert "pvp_appetite" in call_kwargs["user"]  # seed values appear in prompt
    assert call_kwargs["json_schema"] is not None
    # LLM-returned values applied to card
    assert result.pvp_appetite == 0.95
    assert result.raid_appetite == 0.1
    assert result.profession_appetite == 0.3


@pytest.mark.asyncio
async def test_morph_llm_failure_returns_seed_values():
    """LLM returns parsed=None (timeout, unparseable, schema-invalid) → seed values kept."""
    llm = AsyncMock()
    llm.chat_completion_json.return_value = (None, "<error>", 100.0)
    card = _v1_card()
    rng = random.Random(42)
    result = await morph_personality(card, llm, rng=rng)
    expected_seed = _seed_values(random.Random(42))
    assert result.pvp_appetite == expected_seed["pvp_appetite"]
    assert result.profession_appetite == expected_seed["profession_appetite"]


@pytest.mark.asyncio
async def test_morph_llm_exception_returns_seed_values():
    """LLM raising any exception → seed values kept (still no raise to caller)."""
    llm = AsyncMock()
    llm.chat_completion_json.side_effect = RuntimeError("connection refused")
    card = _v1_card()
    rng = random.Random(42)
    result = await morph_personality(card, llm, rng=rng)
    expected_seed = _seed_values(random.Random(42))
    assert result.pvp_appetite == expected_seed["pvp_appetite"]


@pytest.mark.asyncio
async def test_morph_partial_fill_triggers_full_overwrite():
    """If 3 of 5 v2 fields are set, all 5 are overwritten by morph LLM output."""
    llm = AsyncMock()
    llm.chat_completion_json.return_value = (
        {
            "pvp_appetite": 0.11, "raid_appetite": 0.22,
            "completionist_streak": 0.33, "gold_motivation": 0.44,
            "profession_appetite": 0.55,
        },
        "raw_text", 42.0,
    )
    full = _full_card()
    full.gold_motivation = None
    full.profession_appetite = None
    # pvp_appetite was 0.4, raid_appetite was 0.9, completionist 0.5 — all should be overwritten
    result = await morph_personality(full, llm, rng=random.Random(0))
    assert result.pvp_appetite == 0.11
    assert result.raid_appetite == 0.22
    assert result.completionist_streak == 0.33
    assert result.gold_motivation == 0.44
    assert result.profession_appetite == 0.55
