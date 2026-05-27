"""V3.6: _assemble_prompt at_cap branch — system prompt mutation + return value."""
from __future__ import annotations

from dataclasses import dataclass
from unittest.mock import AsyncMock, MagicMock

import pytest

from brain_sidecar.decide import Decider
from brain_sidecar.models import PersonalityCard


@dataclass
class _StubSettings:
    max_player_level: int = 25


def _make_decider(max_level: int = 25) -> Decider:
    """Construct a Decider with stubs sufficient for _assemble_prompt testing."""
    d = Decider(
        llm_client=AsyncMock(),
        personality_cache=AsyncMock(),
        memory_mcp=AsyncMock(),
        state_store=AsyncMock(),
        prompt_template="STATE: {state_json} GOALS: {goals_json} MEMORIES: {memories_json} RECENT: {recent_decisions_json} HOT: {hot_inputs_json} TOOLS: {tools_summary}",
        decision_schema={"type": "object"},
        tools_summary="[]",
    )
    d.settings = _StubSettings(max_player_level=max_level)
    return d


def _card_at_cap() -> PersonalityCard:
    return PersonalityCard.model_validate({
        "name": "Casmina", "race": "Human", "class": "Paladin",
        "backstory": "x", "talkativeness": 0.5, "courage": 0.5,
        "greed": 0.5, "attitude_to_master": 0.0,
        "pvp_appetite": 0.4, "raid_appetite": 0.9,
        "completionist_streak": 0.5, "gold_motivation": 0.3,
        "profession_appetite": 0.2,
    })


def test_at_cap_block_omitted_when_level_below_cap():
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 24}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
    )
    assert "at max level" not in prompt["system"]
    assert "pvp_appetite" not in prompt["system"]


def test_at_cap_block_present_when_level_equals_cap():
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 25}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
    )
    assert "at max level (L25)" in prompt["system"]
    assert "pvp_appetite=0.4" in prompt["system"]
    assert "raid_appetite=0.9" in prompt["system"]
    assert "profession_appetite=0.2" in prompt["system"]
    # talk-only framing conveyed
    assert "talk about wanting" in prompt["system"]


def test_at_cap_block_present_when_level_above_cap():
    """>= semantics: a L30 bot at L25 cap still gets the at-cap block."""
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 30}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
    )
    assert "at max level (L25)" in prompt["system"]


def test_at_cap_block_omitted_when_self_level_missing():
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
    )
    assert "at max level" not in prompt["system"]


def test_at_cap_block_omitted_when_level_non_int():
    """Defensive: malformed state must not crash; falls through to no at-cap block."""
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": "twenty-five"}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
    )
    assert "at max level" not in prompt["system"]


# V3.7 + V3.7.1: goal-management paragraph (always rendered)
def test_v371_goal_management_paragraph_imperative_and_worked_trio():
    """V3.7.1: paragraph uses imperative 'create one now', includes a worked-
    example trio, and anchors ownership with 'your own' + 'persist across ticks'."""
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 10}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
        triage_reason="fresh_chat",
    )
    sys = prompt["system"]
    assert "Your goals shape what you do" in sys
    # V3.7.1 imperative form
    assert "create one now" in sys
    # Worked-example trio (three concrete strings)
    assert "reach level 25" in sys
    assert "earn 5 gold by tomorrow" in sys
    assert "run BFD with a group" in sys
    # Ownership anchoring sentence
    assert "your own" in sys
    assert "persist across ticks" in sys
    # All three tool names still referenced
    assert "goals.create" in sys
    assert "goals.update" in sys
    assert "goals.complete" in sys
    # V3.7 hedging language must be gone
    assert "consider creating" not in sys


def test_v371_goal_management_present_on_organic_wakeup_too():
    """The goal-management paragraph also appears for organic_wakeup ticks."""
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 25}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
        triage_reason="organic_wakeup",
    )
    assert "Your goals shape what you do" in prompt["system"]
    assert "create one now" in prompt["system"]


def test_v373_organic_wakeup_paragraph_uses_ranges_not_anchors():
    """V3.7.3: the wakeup sentence asks for engagement-based ranges, not specific
    anchor values. V3.7.1's '180000 for default pacing' caused 100% default rate
    in soak (LLM dutifully picked the value labeled default). V3.7.3 frames the
    choice in terms of engagement state with overlapping ranges."""
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 25}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
        triage_reason="organic_wakeup",
    )
    sys = prompt["system"]
    # V3.7.3 specifics — engagement-based ranges
    assert "wakeup_in_ms" in sys
    assert "current engagement" in sys
    assert "60000-120000" in sys
    assert "180000-360000" in sys
    assert "480000-600000" in sys
    assert "Vary it based on your situation" in sys
    assert "don't pick the same value every time" in sys
    # V3.7.1 anchor-style wording must be gone
    assert "for default pacing" not in sys
    assert "Pick a specific number" not in sys
    # Intro sentence preserved
    assert "no one is asking you anything" in sys
    assert "your own time" in sys
    assert "no_op" in sys  # "valid choice — emit no_op" retained


def test_v37_organic_wakeup_paragraph_omitted_on_fresh_chat():
    """For reactive ticks (fresh_chat), the organic-wakeup framing is omitted."""
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 25}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
        triage_reason="fresh_chat",
    )
    assert "no one is asking you anything" not in prompt["system"]


def test_v371_diversity_nudge_sentence_on_organic_wakeup():
    """V3.7.1 Gap #3a: the diversity-nudge sentence appears at the end of the
    organic-wakeup paragraph. Reactive ticks (fresh_chat) do NOT get the nudge."""
    d = _make_decider(max_level=25)
    organic_prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 25}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
        triage_reason="organic_wakeup",
    )
    sys = organic_prompt["system"]
    # Diversity nudge specifics
    assert "Look at your recent decisions" in sys
    assert "same action 3+ times" in sys
    assert "deliberately pick something different" in sys
    assert "Repetition is fine; identical repetition isn't" in sys

    # Reactive tick must NOT have the nudge — only organic
    reactive_prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 25}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
        triage_reason="fresh_chat",
    )
    assert "deliberately pick something different" not in reactive_prompt["system"]


def test_v37_assemble_prompt_accepts_triage_reason_none():
    """Back-compat: existing call sites without triage_reason still work.
    Defaults to no organic-wakeup paragraph."""
    d = _make_decider(max_level=25)
    prompt = d._assemble_prompt(
        personality=_card_at_cap(),
        state={"self": {"level": 25}}, goals=[], memories=[],
        recent_decisions=[], hot_inputs={}, bot_guid=1003,
        # triage_reason omitted
    )
    assert "no one is asking you anything" not in prompt["system"]
    # Goal mgmt always renders
    assert "Your goals shape what you do" in prompt["system"]
