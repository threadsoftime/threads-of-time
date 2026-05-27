"""C4: decider — assemble prompt, call LLM, parse Decision."""
from __future__ import annotations

import json
import logging
import re
from collections.abc import Iterable
from dataclasses import dataclass, field
from typing import Any

from pydantic import ValidationError

from brain_sidecar.llm_client import LlmClient
from brain_sidecar.mcp_clients import McpClient
from brain_sidecar.models import Decision, DecisionKind
from brain_sidecar.personality import PersonalityCache
from brain_sidecar.state import StateStore

log = logging.getLogger(__name__)

# Initial allowlist for V3-MVP. Covers V1.4 bot.* family + memory MCP family.
# Tool names match the live MCP registries exactly (dot-notation as registered).
KNOWN_TOOLS: set[str] = {
    # harness / bot.*
    "bot.set_strategy", "bot.get_strategies", "bot.send_chat",
    "bot.follow", "bot.stop", "bot.set_goal",
    # V1.5 grouping/dungeon tools
    "bot.invite_to_group", "bot.accept_invite", "bot.leave_group",
    "bot.set_role", "bot.queue_for_dungeon", "bot.enter_instance",
    # harness / obs.*  (read-only; brain rarely calls these directly from a Decision,
    # but allow for fact-finding queries)
    "obs.get_state", "obs.get_inventory", "obs.get_combat_log",
    "obs.get_money", "obs.get_position", "obs.get_quest_log",
    "obs.get_rpg_status", "obs.get_talents", "obs.get_xp",
    "obs.get_auras", "obs.get_group", "obs.ping", "obs.query_db",
    # memory MCP (dot-notation as registered in memory-sidecar tool_schemas.py)
    "memory.write", "memory_write",
    "memory.read", "memory_read",
    "memory.update", "memory_update",
    "memory.delete", "memory_delete",
    "memory.search", "memory_search",
    "memory.recall", "memory_recall",
    "memory.recall_about", "memory_recall_about",
    "memory.list", "memory_list",
    "memory.personality_set", "memory_personality_set",
    "memory.personality_get", "memory_personality_get",
    "memory.personality.get", "memory.personality.set",
    # goals (registered in memory-sidecar as goals.* not memory.goals.*)
    "goals.create", "goals_create", "memory.goals.create",
    "goals.list", "goals_list", "memory.goals.list",
    "goals.complete", "goals_complete", "memory.goals.complete",
    "goals.read", "goals_read", "memory.goals.read",
    "goals.update", "goals_update", "memory.goals.update",
}

# Schema literal reused in retry feedback so the LLM sees the same description
# as in the system prompt — no Python internals, no Pydantic reprs.
# V3.7.1: wakeup_at_ms (absolute Unix ms) → wakeup_in_ms (relative delta ms).
# Inline range hint anchors the LLM to acceptable delta values.
_DECISION_SCHEMA_LITERAL = (
    '{"kind": "action"|"no_op", "tool": "<tool_name>"|null, '
    '"args": {<tool-specific>}|null, "confidence": 0.0..1.0, '
    '"reasoning": "<one sentence>", "wakeup_in_ms": <60000..600000>}'
)

# Truncation limits — keep context under the LLM window budget.
_MAX_CONTENT_CHARS = 300
_MAX_REASONING_CHARS = 100

# Regex to extract sender from whisper memory content written by T3 chat brain.
# T3 stores: "received whisper from <name>: <message text>"
# Stop at whitespace or colon so the colon separator is not included in the name.
_WHISPER_SENDER_RE = re.compile(r"received whisper from ([^\s:]+)")

# Class-based default dungeon role — overridable by player chat in fresh_chat goals.
# Keys are lower-case WoW class names; values are human-readable role suggestions
# for the system prompt. LLM overrides via goal extraction if player says "heal tonight."
_CLASS_DEFAULT_ROLE: dict[str, str] = {
    "paladin":      "tank or healer",
    "warrior":      "tank",
    "priest":       "healer",
    "druid":        "healer or tank",
    "shaman":       "healer or dps",
    "hunter":       "dps",
    "rogue":        "dps",
    "mage":         "dps",
    "warlock":      "dps",
    "death knight": "tank or dps",
}


@dataclass
class Decider:
    llm_client: LlmClient | Any
    personality_cache: PersonalityCache | Any
    memory_mcp: McpClient | Any
    state_store: StateStore | Any
    prompt_template: str
    decision_schema: dict[str, Any] | None = None
    tools_summary: str | None = None
    max_retries: int = 1
    known_tools: set[str] = field(default_factory=lambda: KNOWN_TOOLS.copy())
    # V3.6: at-cap derivation needs the max-player-level setting. Optional so
    # existing tests that construct Decider without it still work; production
    # app.py wires it explicitly (Task 11).
    settings: Any | None = None

    async def decide(
        self, *,
        bot_guid: int,
        hot_inputs: dict[str, Any],
        triage_reason: str | None = None,
    ) -> tuple[Decision, float | None, bool]:
        """Assemble context, call LLM, parse and return a Decision.

        Returns a triple of (Decision, llm_latency_ms, at_cap). The latency is the
        duration of the most recent LLM call in milliseconds, or None if no LLM call
        was made (currently unreachable in MVP, but defensively typed for future
        early-exit paths). On retry-exhausted or unparseable paths the latency from
        the last attempt is returned.
        at_cap is True when the bot's level >= settings.max_player_level.
        """
        # V3.6: derive at_cap up front; loop.py needs it in the decisions log
        max_level = 25
        if self.settings is not None and hasattr(self.settings, "max_player_level"):
            max_level = int(self.settings.max_player_level)
        state_summary = hot_inputs.get("state_summary", {}) if isinstance(hot_inputs, dict) else {}
        self_obj = state_summary.get("self") if isinstance(state_summary, dict) else None
        self_level = self_obj.get("level") if isinstance(self_obj, dict) else None
        at_cap = isinstance(self_level, int) and self_level >= max_level

        # Gather context
        card = await self.personality_cache.get(bot_guid)
        recent_decisions = self.state_store.decisions_recent(bot_guid=bot_guid, k=3)
        # goals via memory MCP (best-effort; empty list on failure)
        try:
            goals_raw = await self.memory_mcp.call("goals.list", {"bot_id": str(bot_guid), "status": "active"})
            # Unwrap {"ok": True, "result": {...}} envelope from memory-sidecar
            goals_resp = goals_raw.get("result", goals_raw) if isinstance(goals_raw, dict) else {}
            goals = goals_resp.get("items", [])
        except Exception:
            goals = []
        # recent memories (entities pulled from hot_inputs.fresh_chat etc.)
        memories = await self._gather_memories(bot_guid, hot_inputs)

        prompt = self._assemble_prompt(
            personality=card,
            state=hot_inputs.get("state_summary", {}),
            goals=goals,
            memories=memories,
            recent_decisions=recent_decisions,
            hot_inputs=hot_inputs,
            bot_guid=bot_guid,
            triage_reason=triage_reason,
        )

        # V3.7.1: organic-wakeup ticks get higher temperature for action variety.
        # Reactive ticks (fresh_chat, combat) keep 0.5 — those decisions should be
        # tightly constrained by the incoming context.
        temperature = 0.7 if triage_reason == "organic_wakeup" else 0.5

        # Call LLM (with one retry on parse failure)
        last_raw = ""
        last_latency_ms: float | None = None
        for attempt in range(self.max_retries + 1):
            parsed, raw, latency_ms = await self.llm_client.chat_completion_json(
                system=prompt["system"], user=prompt["user"],
                json_schema=self.decision_schema,
                temperature=temperature,
            )
            last_raw = raw
            last_latency_ms = latency_ms
            if parsed is None:
                # Retry once with a precise restatement of the schema.
                if attempt < self.max_retries:
                    prompt["user"] += (
                        "\n\nYour previous response was not a valid JSON object. "
                        f"Emit ONLY this structure and nothing else: {_DECISION_SCHEMA_LITERAL}"
                    )
                    continue
                return (
                    Decision(
                        kind=DecisionKind.NO_OP, tool=None, args=None,
                        confidence=0.0,
                        reasoning=f"llm_unparseable: {raw[:120]}",
                    ),
                    last_latency_ms,
                    at_cap,
                )
            try:
                decision = Decision.model_validate(parsed)
            except ValidationError:
                if attempt < self.max_retries:
                    prompt["user"] += (
                        "\n\nYour previous JSON did not match the required schema. "
                        f"Emit ONLY this structure: {_DECISION_SCHEMA_LITERAL}"
                    )
                    continue
                return (
                    Decision(
                        kind=DecisionKind.NO_OP, tool=None, args=None,
                        confidence=0.0,
                        reasoning="llm_invalid_schema",
                    ),
                    last_latency_ms,
                    at_cap,
                )
            # Unknown-tool validation belongs in C5 (spec §5.1). Pass through intact.
            return decision, last_latency_ms, at_cap

        # Should not be reached
        return (
            Decision(
                kind=DecisionKind.NO_OP, tool=None, args=None,
                confidence=0.0, reasoning="decide_path_exhausted",
            ),
            last_latency_ms,
            at_cap,
        )

    async def _gather_memories(self, bot_guid: int, hot_inputs: dict[str, Any]) -> list[dict[str, Any]]:
        """Pull entity-relevant memories from the memory MCP.

        Tries field-based sender extraction first (future-proof), then falls back
        to a regex on the content text written by the T3 chat brain:
        "received whisper from <name>: <message>"
        """
        entities: list[str] = []
        for chat in hot_inputs.get("fresh_chat", []):
            sender = chat.get("from") or chat.get("sender")
            # Memory MCP schema uses "text" (not "content"); fall back to "content"
            # for back-compat with older fixtures and any pre-migration rows.
            content_text = chat.get("text") or chat.get("content") or ""
            if not sender and isinstance(content_text, str):
                m = _WHISPER_SENDER_RE.search(content_text)
                if m:
                    sender = m.group(1)
            if sender:
                entities.append(str(sender))
        try:
            if entities:
                # memory.recall_about expects entity: str (single), not entities: list.
                # Loop over each unique sender and merge results; deduplicate by id.
                seen_ids: set[str] = set()
                merged: list[dict[str, Any]] = []
                for ent in dict.fromkeys(entities):  # preserves order, deduplicates
                    resp = await self.memory_mcp.call(
                        "memory.recall_about",
                        {"bot_id": str(bot_guid), "entity": ent, "top_k": 5},
                    )
                    inner = resp.get("result", resp) if isinstance(resp, dict) else {}
                    for item in inner.get("items", []):
                        item_id = item.get("id") or item.get("memory_id")
                        if item_id is not None and item_id not in seen_ids:
                            seen_ids.add(item_id)
                            merged.append(item)
                return merged
            else:
                resp = await self.memory_mcp.call(
                    "memory.recall",
                    {"bot_id": str(bot_guid), "query": "", "top_k": 5},
                )
            inner = resp.get("result", resp) if isinstance(resp, dict) else {}
            return inner.get("items", [])
        except Exception:
            return []

    def _assemble_prompt(
        self,
        *,
        personality,
        state: dict[str, Any],
        goals: Iterable[dict[str, Any]],
        memories: Iterable[dict[str, Any]],
        recent_decisions: Iterable[Decision],
        hot_inputs: dict[str, Any],
        bot_guid: int,
        triage_reason: str | None = None,
    ) -> dict[str, str]:
        # V3.6: derive at_cap from state["self"]["level"] vs settings.max_player_level
        max_level = 25  # safe default if settings not wired (test paths)
        if self.settings is not None and hasattr(self.settings, "max_player_level"):
            max_level = int(self.settings.max_player_level)
        self_obj = state.get("self") if isinstance(state, dict) else None
        self_level = self_obj.get("level") if isinstance(self_obj, dict) else None
        at_cap = isinstance(self_level, int) and self_level >= max_level

        # System carries the persona (immutable identity) + the JSON-only constraint
        # with the schema literal inline. This is the only place the LLM sees the persona.
        _default_role = _CLASS_DEFAULT_ROLE.get(personality.class_.lower(), "dps")
        system = (
            f"You are {personality.name}, a {personality.race} {personality.class_} "
            f"in World of Warcraft. {personality.backstory}\n"
            f"Your bot_guid is {bot_guid}. When a tool's args require bot_guid, "
            f"use exactly this integer ({bot_guid}). Do not use 0 or guess.\n"
            f"Personality traits (0..1 scale unless noted): "
            f"talkativeness={personality.talkativeness}, "
            f"courage={personality.courage}, "
            f"greed={personality.greed}, "
            f"attitude_to_master={personality.attitude_to_master} (range -1..1), "
            f"party_invite_policy={personality.party_invite_policy}.\n"
            f"Your default dungeon role: {_default_role}.\n"
        )

        # V3.6: at-cap end-game framing paragraph (only when at cap)
        if at_cap:
            system += (
                f"\nYou are at max level (L{max_level}). XP from quests no longer matters. "
                f"Your end-game preferences (0..1): "
                f"pvp_appetite={personality.pvp_appetite}, "
                f"raid_appetite={personality.raid_appetite}, "
                f"completionist_streak={personality.completionist_streak}, "
                f"gold_motivation={personality.gold_motivation}, "
                f"profession_appetite={personality.profession_appetite}.\n"
                f"Available end-game activities on this server: Black Fathom Deeps "
                f"raid, battlegrounds (Warsong Gulch, Arathi Basin), dungeon farming, "
                f"profession crafting, gold/AH play, zone completion. Let your "
                f"preferences shape what you talk about wanting to do, even though "
                f"the harness tools to execute these activities are not yet wired.\n"
            )

        # V3.7.1: goal-management paragraph (always rendered; imperative +
        # worked-example trio + ownership anchor). V3.7 used "consider creating"
        # — soak showed 0 goals.create across 7 bots in 10h. Imperative voice +
        # concrete examples + ownership statement raise the salience.
        system += (
            "\nYour goals shape what you do across ticks. If you have no "
            "active goals, create one now via goals.create — pick something "
            'concrete that fits your personality (e.g., "reach level 25", '
            '"earn 5 gold by tomorrow", "run BFD with a group"). Goals you '
            "write here are your own — the brain owns them, they persist "
            "across ticks, and you reference them in future decisions. If "
            "you have an active goal, check whether you're making progress; "
            "use goals.update to record progress or change tack, and "
            "goals.complete when done.\n"
        )

        # V3.7 / V3.7.1: organic-wakeup framing (only when this tick was triggered
        # by the bot's own self-paced timer, not by an external event).
        if triage_reason == "organic_wakeup":
            system += (
                "\nYou woke up on your own — no one is asking you anything, no "
                "combat is happening, no invitations are pending. This is your "
                "own time. Decide what YOU want to do next based on your "
                "personality, your goals, and your current situation. If nothing "
                "important needs doing, that's a valid choice — emit no_op.\n"
                "Set wakeup_in_ms to a number of milliseconds reflecting "
                "your current engagement: shorter (60000-120000) if you're "
                "actively pursuing something or expecting an event soon; "
                "medium (180000-360000) if you're between activities; "
                "longer (480000-600000) if you're settled and content. Vary "
                "it based on your situation — don't pick the same value "
                "every time.\n"
                "Look at your recent decisions in this prompt. If you've done "
                "the same action 3+ times in a row, deliberately pick something "
                "different — a real character varies their activities. Repetition "
                "is fine; identical repetition isn't.\n"
            )

        system += (
            "\nYou make ONE decision per call. Respond with ONLY a single JSON object "
            "matching this exact schema, and nothing else (no prose, no markdown, no preamble):\n"
            + _DECISION_SCHEMA_LITERAL
        )

        # User carries only variable game-state context; no persona duplication.
        user = self.prompt_template.format(
            tools_summary=(self.tools_summary
                           if self.tools_summary is not None
                           else json.dumps(sorted(self.known_tools))),
            state_json=json.dumps(state),
            goals_json=json.dumps(list(goals)),
            memories_json=json.dumps(self._truncate_memory_items(list(memories))),
            recent_decisions_json=json.dumps(self._truncate_recent_decisions(recent_decisions)),
            hot_inputs_json=json.dumps(self._project_hot_inputs(hot_inputs)),
        )
        return {"system": system, "user": user}

    # ------------------------------------------------------------------
    # Truncation helpers — prevent unbounded context from blowing the window
    # ------------------------------------------------------------------

    def _truncate_memory_items(self, items: list[dict[str, Any]]) -> list[dict[str, Any]]:
        """Clip the 'text' (and back-compat 'content') field of each memory item.

        Memory MCP schema uses 'text' as the primary field name.  Older fixtures
        and any pre-migration rows may still carry 'content'; clip both so nothing
        reaches the LLM window unbounded.
        """
        out = []
        for it in items:
            clipped = dict(it)
            for field_name in ("text", "content"):
                val = clipped.get(field_name)
                if isinstance(val, str) and len(val) > _MAX_CONTENT_CHARS:
                    clipped[field_name] = val[:_MAX_CONTENT_CHARS] + "…"
            out.append(clipped)
        return out

    def _truncate_recent_decisions(self, decisions: Iterable[Decision]) -> list[dict[str, Any]]:
        """Clip the 'reasoning' field of each recent decision to _MAX_REASONING_CHARS."""
        out = []
        for d in decisions:
            dd = d.model_dump()
            if isinstance(dd.get("reasoning"), str) and len(dd["reasoning"]) > _MAX_REASONING_CHARS:
                dd["reasoning"] = dd["reasoning"][:_MAX_REASONING_CHARS] + "…"
            out.append(dd)
        return out

    # ------------------------------------------------------------------
    # hot_inputs projection — flatten fresh_chat to avoid duplication
    # with memories_json; other keys pass through unchanged.
    # ------------------------------------------------------------------

    def _project_hot_inputs(self, hot_inputs: dict[str, Any]) -> dict[str, Any]:
        """Flatten fresh_chat to {sender, message} pairs and clip content.

        The memories_json section already contains entity-relevant recall so
        the full raw chat payloads would be redundant. Other hot_inputs keys
        (e.g., combat_events) are forwarded unchanged.
        """
        out: dict[str, Any] = {}
        for key, val in hot_inputs.items():
            if key == "fresh_chat" and isinstance(val, list):
                projected = []
                for c in val:
                    # Memory MCP schema uses "text"; fall back to "content" for back-compat.
                    content_text = c.get("text") or c.get("content") or ""
                    sender = (
                        c.get("from")
                        or c.get("sender")
                        or self._sender_from_content(content_text)
                    )
                    msg = content_text
                    if len(msg) > _MAX_CONTENT_CHARS:
                        msg = msg[:_MAX_CONTENT_CHARS] + "…"
                    projected.append({"sender": sender or "?", "message": msg})
                out["fresh_chat"] = projected
            else:
                out[key] = val
        return out

    def _sender_from_content(self, content: str) -> str | None:
        """Extract sender name from T3 whisper content format."""
        m = _WHISPER_SENDER_RE.search(content) if isinstance(content, str) else None
        return m.group(1) if m else None
