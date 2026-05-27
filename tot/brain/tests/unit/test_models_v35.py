"""V3.5 model tests — party_invite_policy field on PersonalityCard."""
from __future__ import annotations

import pytest
from pydantic import ValidationError

from brain_sidecar.models import PersonalityCard


def _base_card_kwargs() -> dict:
    return dict(
        name="Casmina",
        race="Human",
        **{"class": "Paladin"},
        backstory="A steadfast anchor of the group.",
        talkativeness=0.6,
        courage=0.7,
        greed=0.2,
        attitude_to_master=0.5,
    )


def test_party_invite_policy_default_accept_from_known():
    """Default value must be 'accept_from_known' (spec §5.2)."""
    card = PersonalityCard(**_base_card_kwargs())
    assert card.party_invite_policy == "accept_from_known"


def test_party_invite_policy_accept_always_valid():
    """'accept_always' must pass validation."""
    card = PersonalityCard(**_base_card_kwargs(), party_invite_policy="accept_always")
    assert card.party_invite_policy == "accept_always"


def test_party_invite_policy_decline_always_valid():
    """'decline_always' must pass validation."""
    card = PersonalityCard(**_base_card_kwargs(), party_invite_policy="decline_always")
    assert card.party_invite_policy == "decline_always"


def test_party_invite_policy_invalid_value_rejected():
    """Arbitrary strings like 'maybe' must raise ValidationError."""
    with pytest.raises(ValidationError):
        PersonalityCard(**_base_card_kwargs(), party_invite_policy="maybe")


def test_personality_card_existing_fields_unchanged():
    """Regression guard: existing fields still validate correctly post-V3.5 addition."""
    card = PersonalityCard(**_base_card_kwargs())
    assert card.name == "Casmina"
    assert card.race == "Human"
    assert card.class_ == "Paladin"
    assert card.talkativeness == pytest.approx(0.6)
    assert card.courage == pytest.approx(0.7)
    assert card.greed == pytest.approx(0.2)
    assert card.attitude_to_master == pytest.approx(0.5)
