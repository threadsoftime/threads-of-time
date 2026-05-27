"""V3.6: morph PersonalityCard v2 fields via LLM, with random-seed fallback.

The morph runs whenever PersonalityCard has any None v2 field, in two paths:
  (1) /enroll — fresh bot, v2 fields default to None on construction;
  (2) PersonalityCache.get() migration — V1 persona JSON loaded from
      memory-sidecar deserializes with v2 fields = None.

Random seed values in [0.3, 0.8] per scalar are generated FIRST. The LLM is
asked to morph these based on the bot's backstory. On any LLM failure
(timeout, unparseable, schema-invalid, exception), the seed values are kept
and the card is returned without raising — callers must not fail enroll or
decide on a stale LLM.
"""
from __future__ import annotations

import logging
import random
from typing import Any

from brain_sidecar.llm_client import LlmClient
from brain_sidecar.models import PersonalityCard

log = logging.getLogger(__name__)

_V2_FIELD_NAMES: tuple[str, ...] = (
    "pvp_appetite",
    "raid_appetite",
    "completionist_streak",
    "gold_motivation",
    "profession_appetite",
)

_SEED_MIN = 0.3
_SEED_MAX = 0.8

_MORPH_SYSTEM_PROMPT = (
    "You assign end-game activity preferences for a World of Warcraft bot "
    "character based on its backstory. Return ONLY a JSON object with "
    "exactly five floats in [0.0, 1.0]: pvp_appetite, raid_appetite, "
    "completionist_streak, gold_motivation, profession_appetite. No prose."
)

_MORPH_USER_TEMPLATE = (
    "Character backstory:\n{backstory}\n\n"
    "Random seed values (use as starting point; morph based on backstory):\n"
    "{seed_json}\n\n"
    "What are this character's actual end-game preferences? Respond with JSON only."
)

_MORPH_JSON_SCHEMA: dict[str, Any] = {
    "type": "object",
    "properties": {
        "pvp_appetite":         {"type": "number", "minimum": 0.0, "maximum": 1.0},
        "raid_appetite":        {"type": "number", "minimum": 0.0, "maximum": 1.0},
        "completionist_streak": {"type": "number", "minimum": 0.0, "maximum": 1.0},
        "gold_motivation":      {"type": "number", "minimum": 0.0, "maximum": 1.0},
        "profession_appetite":  {"type": "number", "minimum": 0.0, "maximum": 1.0},
    },
    "required": list(_V2_FIELD_NAMES),
    "additionalProperties": False,
}


def _needs_morph(card: PersonalityCard) -> bool:
    """True if any v2 field is None (full or partial-fill triggers morph)."""
    return any(getattr(card, name) is None for name in _V2_FIELD_NAMES)


def _seed_values(rng: random.Random) -> dict[str, float]:
    """Generate random seed values in [_SEED_MIN, _SEED_MAX] for each v2 field."""
    return {name: rng.uniform(_SEED_MIN, _SEED_MAX) for name in _V2_FIELD_NAMES}


async def morph_personality(
    card: PersonalityCard,
    llm_client: LlmClient,
    *,
    rng: random.Random | None = None,
) -> PersonalityCard:
    """Return a NEW card with v2 fields filled (LLM-morphed from random seed).

    No-op if card has no None v2 fields. On LLM failure (timeout, unparseable,
    schema-invalid, or exception): seed values are kept (returned card has all
    v2 fields populated, just with the random seed not the LLM output). Input
    card is not mutated.
    """
    if not _needs_morph(card):
        return card
    if rng is None:
        rng = random.Random()
    seed = _seed_values(rng)
    # Prepare the user prompt; seed JSON is pretty-printed for LLM legibility.
    import json
    user_prompt = _MORPH_USER_TEMPLATE.format(
        backstory=card.backstory,
        seed_json=json.dumps(seed, indent=2),
    )
    try:
        parsed, raw, latency_ms = await llm_client.chat_completion_json(
            system=_MORPH_SYSTEM_PROMPT,
            user=user_prompt,
            max_tokens=200,
            temperature=0.7,
            json_schema=_MORPH_JSON_SCHEMA,
        )
    except Exception as exc:
        log.warning(
            "morph_personality LLM exception bot=%s err=%s; keeping seed values",
            card.name, exc,
        )
        return card.model_copy(update=seed)
    if parsed is None:
        log.warning(
            "morph_personality LLM unparseable bot=%s raw=%r; keeping seed values",
            card.name, raw[:120] if raw else "",
        )
        return card.model_copy(update=seed)
    # LLM happy path: trust the parsed dict; json_schema enforced strict shape.
    return card.model_copy(update=parsed)
