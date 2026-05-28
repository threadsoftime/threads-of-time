# SPDX-License-Identifier: GPL-2.0-or-later
"""Brain-side salience scorer (Plan 2 Task 34).

Decides which ticks produce a written memory episode. The score is a HINT —
passed to memory.write as `salience_hint` per design subspec §10.1 — and the
server-side scorer in tot/memory is authoritative for retrieval-time scoring.
At the brain layer this is just the write-time gate: `score >= threshold`
means "worth remembering."

Tiers and per-type defaults follow design subspec §8 verbatim. Rule
adjustments (combat outcome, quest event, goal completion, chat source)
mirror §8.3.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


# ---------------------------------------------------------------------------
# Salience tiers — design subspec §8.2 (mirrors the server-side constants in
# tot/memory/src/tot_memory/salience/scorer.py so both layers agree)
# ---------------------------------------------------------------------------

SALIENCE_PIVOTAL: float   = 1.0   # First-ever / "remember this" moments
SALIENCE_IMPORTANT: float = 0.8   # Named boss kill, quest complete, goal achieved
SALIENCE_NOTABLE: float   = 0.6   # Named NPC interaction, group formed
SALIENCE_NORMAL: float    = 0.45  # Typical social / non-boss combat
SALIENCE_FILLER: float    = 0.2   # Routine observation, near-dupes
SALIENCE_TRIVIAL: float   = 0.05  # System noise


_TYPE_DEFAULT: dict[str, float] = {
    "chat":        SALIENCE_NORMAL,
    "combat":      SALIENCE_NORMAL,
    "social":      SALIENCE_NOTABLE,
    "quest":       SALIENCE_IMPORTANT,
    "discovery":   SALIENCE_NOTABLE,
    "goal":        SALIENCE_IMPORTANT,
    "reflection":  SALIENCE_IMPORTANT,
    "observation": SALIENCE_FILLER,
}


@dataclass
class SalienceScorer:
    """Rule-based scorer with optional brain-emitted override hint.

    Attributes:
        threshold: episodes with `score(...) >= threshold` are written to memory.
                   0.3 by default — observation (FILLER=0.2) is filtered out,
                   chat (NORMAL=0.45) and everything stronger is kept.
    """

    threshold: float = 0.3

    # Allow callers (tests, future tuning) to override the tier table without
    # subclassing. Defaults to a copy of the module-level table so per-instance
    # mutations don't leak across scorers.
    type_defaults: dict[str, float] = field(
        default_factory=lambda: dict(_TYPE_DEFAULT)
    )

    def score(
        self,
        perception: dict[str, Any],
        decision: dict[str, Any],  # noqa: ARG002 — reserved for future signal (e.g., decision.kind)
        action_result: dict[str, Any],
    ) -> float:
        """Compute the salience score for a single tick.

        Args:
            perception: brain-side perception bundle. Recognized keys:
                - `episode_type` (str): the episode_type to write; selects the
                  per-type default salience (§8.2).
                - `source` (str, optional): for `chat`, "player" promotes the
                  score to NOTABLE per §8.3.
                - `salience_hint` (float, optional): brain override in [0, 1].
                  If provided and in-range, returned verbatim (rounded to 3
                  decimals). Out-of-range hints are ignored.
            decision: the LLM-emitted Decision dict for this tick. Currently
                unused but accepted to keep the signature future-proof for
                signals like "decision asked the player to remember this".
            action_result: tool-dispatch outcome. Recognized keys:
                - `outcome` (str): for `combat`, one of
                  "kill" | "near_death" | "wipe" — applies §8.3 promotions.
                - `event` (str): for `quest` / `goal`, the lifecycle event
                  ("complete" | "fail" | "completed" | "achieved").

        Returns:
            A float in [0.0, 1.0] rounded to 3 decimals.
        """
        # Brain override takes precedence (subspec §8.1, §10.1).
        hint = perception.get("salience_hint")
        if isinstance(hint, (int, float)) and 0.0 <= float(hint) <= 1.0:
            return round(float(hint), 3)

        episode_type = perception.get("episode_type", "")
        base = self.type_defaults.get(episode_type, SALIENCE_NORMAL)
        score = base

        # Combat: outcome-aware promotions (subspec §8.3).
        if episode_type == "combat":
            outcome = action_result.get("outcome", "")
            if outcome == "kill":
                score = max(score, SALIENCE_IMPORTANT)
            elif outcome == "near_death":
                score = max(score, SALIENCE_NOTABLE)
            elif outcome == "wipe":
                score = SALIENCE_IMPORTANT

        # Quest: completion is the most salient event (subspec §8.3).
        if episode_type == "quest":
            event = action_result.get("event", "")
            if event == "complete":
                score = max(score, SALIENCE_IMPORTANT)
            elif event == "fail":
                score = max(score, SALIENCE_NOTABLE)

        # Goal: completion is pivotal (subspec §8.3).
        if episode_type == "goal":
            event = action_result.get("event", "")
            if event in ("completed", "achieved"):
                score = SALIENCE_PIVOTAL

        # Chat: from-a-player is more salient than from-an-NPC (subspec §8.3).
        if episode_type == "chat" and perception.get("source") == "player":
            score = max(score, SALIENCE_NOTABLE)

        return round(min(1.0, max(0.0, score)), 3)
