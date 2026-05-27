"""Tests for shared Pydantic models."""
from __future__ import annotations

import json

import pytest
from pydantic import ValidationError

from brain_sidecar.models import (
    Decision,
    DecisionKind,
    PersonalityCard,
    TickState,
    TriageResult,
)


def test_personality_card_minimal_fields():
    card = PersonalityCard(
        name="Caedon",
        race="Dwarf",
        **{"class": "Hunter"},
        backstory="A gruff old veteran of the Bronzebeard expeditions.",
        talkativeness=0.4,
        courage=0.8,
        greed=0.3,
        attitude_to_master=0.6,
    )
    assert card.name == "Caedon"
    assert card.class_ == "Hunter"  # alias 'class' → field 'class_'


def test_personality_card_trait_bounds():
    with pytest.raises(ValidationError):
        PersonalityCard(
            name="x", race="x", **{"class": "x"}, backstory="x",
            talkativeness=1.5,  # > 1.0
            courage=0.5, greed=0.5, attitude_to_master=0.0,
        )


def test_decision_action_requires_tool_and_args():
    d = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_strategy",
        args={"bot_guid": 12345, "add": ["grind"]},
        confidence=0.85,
        reasoning="Player wants to grind",
    )
    assert d.tool == "bot.set_strategy"


def test_decision_no_op_no_tool():
    d = Decision(
        kind=DecisionKind.NO_OP,
        tool=None, args=None,
        confidence=0.0,
        reasoning="nothing changed",
    )
    assert d.tool is None


def test_decision_action_missing_tool_rejected():
    with pytest.raises(ValidationError):
        Decision(
            kind=DecisionKind.ACTION,
            tool=None, args=None,
            confidence=0.9,
            reasoning="x",
        )


def test_triage_result_no_fire():
    r = TriageResult(should_decide=False, reason="no_change", hot_inputs={})
    assert r.should_decide is False


def test_tick_state_holds_last_tick():
    s = TickState(bot_guid=1, last_tick_ms=1_000_000, last_decision_id=None)
    assert s.bot_guid == 1


# V3.6: PersonalityCard v2 fields


def test_v1_json_deserializes_with_v2_fields_none():
    """A V1 persona JSON (no v2 fields) deserializes cleanly; v2 fields default to None."""
    v1_json = {
        "name": "Caedon", "race": "Dwarf", "class": "Hunter",
        "backstory": "A grizzled scout.", "talkativeness": 0.5,
        "courage": 0.6, "greed": 0.3, "attitude_to_master": 0.4,
        "party_invite_policy": "accept_from_known",
    }
    card = PersonalityCard.model_validate(v1_json)
    assert card.pvp_appetite is None
    assert card.raid_appetite is None
    assert card.completionist_streak is None
    assert card.gold_motivation is None
    assert card.profession_appetite is None


def test_v2_full_card_roundtrip():
    """A fully-populated v2 card serializes and deserializes preserving all 9 fields."""
    full_json = {
        "name": "Casmina", "race": "Human", "class": "Paladin",
        "backstory": "A devoted tank.", "talkativeness": 0.7,
        "courage": 0.9, "greed": 0.3, "attitude_to_master": 0.6,
        "party_invite_policy": "accept_always",
        "pvp_appetite": 0.4, "raid_appetite": 0.9,
        "completionist_streak": 0.5, "gold_motivation": 0.4,
        "profession_appetite": 0.2,
    }
    card = PersonalityCard.model_validate(full_json)
    out = card.model_dump(by_alias=True)
    assert out["pvp_appetite"] == 0.4
    assert out["raid_appetite"] == 0.9
    assert out["completionist_streak"] == 0.5
    assert out["gold_motivation"] == 0.4
    assert out["profession_appetite"] == 0.2
    # round-trip through JSON
    rehydrated = PersonalityCard.model_validate(json.loads(json.dumps(out)))
    assert rehydrated.pvp_appetite == 0.4
    assert rehydrated.profession_appetite == 0.2


def test_v2_field_range_validation_rejects_out_of_range():
    """v2 scalars enforce [0.0, 1.0]."""
    base = {
        "name": "X", "race": "Y", "class": "Z", "backstory": "x",
        "talkativeness": 0.5, "courage": 0.5, "greed": 0.5, "attitude_to_master": 0.5,
    }
    # pvp_appetite below 0
    with pytest.raises(ValidationError):
        PersonalityCard.model_validate({**base, "pvp_appetite": -0.1})
    # raid_appetite above 1
    with pytest.raises(ValidationError):
        PersonalityCard.model_validate({**base, "raid_appetite": 1.5})
