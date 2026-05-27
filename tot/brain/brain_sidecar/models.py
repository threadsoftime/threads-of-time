"""Shared Pydantic models for brain-sidecar."""
from __future__ import annotations

from enum import Enum
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator


class DecisionKind(str, Enum):
    ACTION = "action"
    NO_OP = "no_op"


class PersonalityCard(BaseModel):
    """Per-bot personality seed. v2 adds end-game scalars; v1 fields unchanged."""
    model_config = ConfigDict(populate_by_name=True)

    # v1 (unchanged)
    name: str
    race: str
    class_: str = Field(alias="class")
    backstory: str = Field(description="2-3 sentences")
    talkativeness: float = Field(ge=0.0, le=1.0)
    courage: float = Field(ge=0.0, le=1.0)
    greed: float = Field(ge=0.0, le=1.0)
    attitude_to_master: float = Field(ge=-1.0, le=1.0)
    party_invite_policy: Literal[
        "accept_from_known", "accept_always", "decline_always"
    ] = "accept_from_known"

    # v2 (new — None signals "needs morph")
    pvp_appetite: float | None = Field(default=None, ge=0.0, le=1.0)
    raid_appetite: float | None = Field(default=None, ge=0.0, le=1.0)
    completionist_streak: float | None = Field(default=None, ge=0.0, le=1.0)
    gold_motivation: float | None = Field(default=None, ge=0.0, le=1.0)
    profession_appetite: float | None = Field(default=None, ge=0.0, le=1.0)


class Decision(BaseModel):
    """One decision emission from the brain's LLM. Either ACTION (tool + args) or NO_OP."""
    kind: DecisionKind
    tool: str | None = None
    args: dict[str, Any] | None = None
    confidence: float = Field(ge=0.0, le=1.0)
    reasoning: str
    # V3.7.1: was wakeup_at_ms (absolute Unix ms). Renamed to wakeup_in_ms
    # (relative delta in ms from now). LLMs handle deltas better than absolute
    # timestamps; V3.7 soak showed 100% defaulting because the LLM never set it.
    wakeup_in_ms: int | None = None

    @model_validator(mode="after")
    def _action_requires_tool(self) -> "Decision":
        if self.kind is DecisionKind.ACTION and (self.tool is None or self.args is None):
            raise ValueError("ACTION decision requires tool and args")
        return self


class TriageResult(BaseModel):
    """Output of the triage gate. should_decide=True means C4 should be invoked."""
    should_decide: bool
    reason: str
    hot_inputs: dict[str, Any] = Field(default_factory=dict)


class TickState(BaseModel):
    """Per-bot loop state carried tick-to-tick."""
    bot_guid: int
    last_tick_ms: int
    last_decision_id: str | None = Field(
        default=None,
        description="V3.1: triage will use this to correlate pending_confirmation memories with the decision that created them. Unused in MVP.",
    )
