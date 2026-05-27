"""C5: tool dispatcher — risk gate + cross-bot guard + MCP call + outcome memory."""
from __future__ import annotations

import logging
import time
from dataclasses import dataclass
from typing import Any

from brain_sidecar.decide import KNOWN_TOOLS
from brain_sidecar.mcp_clients import McpClient
from brain_sidecar.models import Decision, DecisionKind

log = logging.getLogger(__name__)

# Risk classification: low = executable at confidence >= 0.7;
# high = requires confidence >= 0.95 OR confirmation.
# Anything not listed defaults to "high" (safe-by-default).
RISK_TABLE: dict[str, str] = {
    "bot.set_strategy": "low", "bot.get_strategies": "low",
    "bot.send_chat": "low", "bot.follow": "low",
    "bot.stop": "low", "bot.set_goal": "low",
    "memory.write": "low", "memory_write": "low",
    "memory.goals.create": "low", "goals_create": "low",
    "memory.goals.update": "low", "goals_update": "low",
    "memory.update": "low", "memory_update": "low",
    "memory.delete": "high", "memory_delete": "high",
    "memory.goals.complete": "high", "goals_complete": "high",
    # obs.* reads are inherently safe; treat as low even if brain calls them as actions
    "obs.get_state": "low", "obs.get_inventory": "low",
    "obs.get_money": "low", "obs.get_position": "low",
    # Additional obs.* reads — inherently safe, low-risk
    "obs.get_combat_log": "low",
    "obs.get_quest_log": "low",
    "obs.get_rpg_status": "low",
    "obs.get_talents": "low",
    "obs.get_xp": "low",
    "obs.get_auras": "low",
    "obs.get_group": "low",
    "obs.ping": "low",
    "obs.query_db": "low",  # reads only; DB writes are out-of-scope for harness obs.*
    # V1.5 grouping/dungeon tools
    "bot.invite_to_group":   "high",   # social commitment, irreversible if accepted
    "bot.accept_invite":     "high",   # commits the bot to a group
    "bot.leave_group":       "high",   # disruptive if mid-dungeon
    "bot.set_role":          "low",    # safe to change pre-queue
    "bot.queue_for_dungeon": "high",   # queues are noisy; high confidence required
    "bot.enter_instance":    "high",   # teleport, physically moves bot
    # Memory reads — low
    "memory.read": "low", "memory_read": "low",
    "memory.recall": "low", "memory_recall": "low",
    "memory.recall_about": "low", "memory_recall_about": "low",
    "memory.search": "low", "memory_search": "low",
    "memory.list": "low", "memory_list": "low",
    "memory.personality.get": "low", "memory_personality_get": "low",
    "memory.goals.read": "low", "goals_read": "low",
    "memory.goals.list": "low", "goals_list": "low",
    # Personality SET stays high (self-revision deferred to V3.1 per Q7)
    "memory.personality.set": "high", "memory_personality_set": "high",
    "memory.personality.get": "low", "memory_personality_get": "low",
    "memory.personality_get": "low",
    # goals.read / goals.update
    "goals.read": "low", "goals_read": "low",
    "goals.update": "low", "goals_update": "low",
    # Anything not listed defaults to "high" (safe-by-default)
}

LOW_RISK_THRESHOLD = 0.7
HIGH_RISK_THRESHOLD = 0.95

# Keys that, when present in tool args, MUST match the dispatching bot_guid.
# Adding a key here is a conscious decision: this tool addresses the bot itself.
# Everything else (player_guid, target_guid, memory_id, etc.) is NOT constrained —
# those reference entities other than the bot the brain is acting for.
_BOT_OWN_KEYS: frozenset[str] = frozenset({
    "bot_guid",   # harness/bot.* family
    "bot_id",     # memory.* family (memory MCP convention)
})

# Human-readable names for tool actions used in confirmation chat messages.
# Both dot-notation and underscore-notation included where tools have dual spellings.
_TOOL_DISPLAY_NAMES: dict[str, str] = {
    "bot.set_strategy": "change my strategy",
    "bot.get_strategies": "check my strategies",
    "bot.send_chat": "send a chat message",
    "bot.follow": "follow you",
    "bot.stop": "stop what I'm doing",
    "bot.set_goal": "set a new goal",
    "memory.write": "remember that",
    "memory_write": "remember that",
    "memory.delete": "forget that",
    "memory_delete": "forget that",
    "memory.update": "update that memory",
    "memory_update": "update that memory",
    "memory.goals.create": "set that as a goal",
    "goals_create": "set that as a goal",
    "memory.goals.update": "update that goal",
    "goals_update": "update that goal",
    "memory.goals.complete": "mark that goal complete",
    "goals_complete": "mark that goal complete",
    # V1.5 grouping/dungeon tools
    "bot.invite_to_group":   "invite someone to group",
    "bot.accept_invite":     "accept the group invite",
    "bot.leave_group":       "leave the group",
    "bot.set_role":          "set my group role",
    "bot.queue_for_dungeon": "queue for a dungeon",
    "bot.enter_instance":    "enter the dungeon",
}


def _display_name(tool: str) -> str:
    """Return a human-friendly description of the tool action for player-facing chat."""
    return _TOOL_DISPLAY_NAMES.get(tool, tool)


def _normalize_tool(name: str) -> str:
    """Spec uses dot notation; MCP registry uses underscores for memory.* tools.
    Both spellings live in KNOWN_TOOLS / RISK_TABLE; no rewriting needed for MVP."""
    return name


def _mcp_for_tool(tool: str) -> str:
    """Return 'harness' or 'memory' for routing."""
    if tool.startswith(("bot.", "obs.", "gm.", "bot_", "obs_", "gm_")):
        return "harness"
    return "memory"


@dataclass
class DispatchResult:
    disposition: str   # "executed" | "confirmation_emitted" | "no_op" | "blocked_cross_bot" | "tool_call_failed" | "invalid_tool"
    tool: str | None
    result: dict[str, Any] | None = None
    error: str = ""


# Salience grades for outcome types — higher = more important to retain.
_OUTCOME_SALIENCE: dict[str, float] = {
    "action_taken": 0.5,
    "brain_no_op": 0.2,
    "tool_call_failed": 0.8,
    "pending_confirmation": 0.6,
    "blocked_cross_bot": 0.7,
    "decision_invalid_tool": 0.6,
}


@dataclass
class Dispatcher:
    harness_mcp: McpClient | Any
    memory_mcp: McpClient | Any
    confirmation_channel: str = "whisper"

    def _cross_bot_violations(self, args: dict[str, Any], bot_guid: int) -> list[tuple[str, Any]]:
        """Return list of (key, value) where the arg is a bot-ownership key but the
        value differs from the dispatching bot_guid.

        Only _BOT_OWN_KEYS are checked — an explicit frozenset, not a suffix match.
        Keys like player_guid, target_guid, memory_id, etc. are intentionally excluded:
        they reference entities other than the bot the brain is acting for.
        """
        bad = []
        for k in _BOT_OWN_KEYS:
            if k not in args:
                continue
            v = args[k]
            try:
                if int(v) != int(bot_guid):
                    bad.append((k, v))
            except (TypeError, ValueError):
                # Garbage value — flag as violation rather than silently letting through.
                bad.append((k, v))
        return bad

    async def dispatch(self, *, bot_guid: int, decision: Decision) -> DispatchResult:
        ts_ms = int(time.time() * 1000)

        if decision.kind is DecisionKind.NO_OP:
            await self._write_outcome_memory(
                bot_guid=bot_guid, ts_ms=ts_ms,
                memory_type="brain_no_op", payload={"reasoning": decision.reasoning},
            )
            return DispatchResult(disposition="no_op", tool=None)

        tool = _normalize_tool(decision.tool or "")
        args = decision.args or {}

        # Unknown-tool guard (per spec §5.1; moved here from C4 in Task 7 fixes).
        # Runs BEFORE cross-bot guard so unknown tools produce "invalid_tool" not
        # "blocked_cross_bot" — the more specific and actionable signal.
        if tool not in KNOWN_TOOLS:
            await self._write_outcome_memory(
                bot_guid=bot_guid, ts_ms=ts_ms,
                memory_type="decision_invalid_tool",
                payload={"tool": tool, "args": args, "reasoning": decision.reasoning},
            )
            return DispatchResult(disposition="invalid_tool", tool=tool,
                                   error=f"unknown_tool: {tool}")

        # Cross-bot guard — runs BEFORE risk gate so a high-confidence cross-bot
        # decision cannot slip through to execute on the wrong bot.
        violations = self._cross_bot_violations(args, bot_guid)
        if violations:
            await self._write_outcome_memory(
                bot_guid=bot_guid, ts_ms=ts_ms,
                memory_type="blocked_cross_bot",
                payload={"tool": tool, "args": args, "reasoning": decision.reasoning,
                         "violations": [(k, v) for k, v in violations]},
            )
            return DispatchResult(disposition="blocked_cross_bot", tool=tool,
                                   error=f"cross-bot args: {violations}")

        # Risk gate
        risk = RISK_TABLE.get(tool, "high")
        if (risk == "high" and decision.confidence < HIGH_RISK_THRESHOLD) \
                or (risk == "low" and decision.confidence < LOW_RISK_THRESHOLD):
            await self._emit_confirmation(
                bot_guid=bot_guid, decision=decision, ts_ms=ts_ms,
            )
            return DispatchResult(disposition="confirmation_emitted", tool=tool)

        # Execute — route to harness or memory MCP based on tool prefix
        mcp = self.harness_mcp if _mcp_for_tool(tool) == "harness" else self.memory_mcp
        try:
            result = await mcp.call(tool, args)
        except Exception as e:
            await self._write_outcome_memory(
                bot_guid=bot_guid, ts_ms=ts_ms,
                memory_type="tool_call_failed",
                payload={"tool": tool, "args": args, "error": str(e)},
            )
            return DispatchResult(disposition="tool_call_failed", tool=tool, error=str(e))

        await self._write_outcome_memory(
            bot_guid=bot_guid, ts_ms=ts_ms,
            memory_type="action_taken",
            payload={"tool": tool, "args": args, "result": result, "reasoning": decision.reasoning},
        )
        return DispatchResult(disposition="executed", tool=tool, result=result)

    async def _emit_confirmation(self, *, bot_guid: int, decision: Decision, ts_ms: int) -> None:
        display = _display_name(decision.tool or "")
        chat_text = (
            f"want me to {display}? ({decision.reasoning})"
            if decision.reasoning else f"want me to {display}?"
        )
        try:
            await self.harness_mcp.call("bot.send_chat", {
                "bot_guid": bot_guid,
                "channel": self.confirmation_channel,
                "message": chat_text[:200],
            })
        except Exception as e:
            log.warning(
                "confirmation_chat_failed bot=%s tool=%s err=%s",
                bot_guid, decision.tool, e,
            )
        await self._write_outcome_memory(
            bot_guid=bot_guid, ts_ms=ts_ms,
            memory_type="pending_confirmation",
            payload={"tool": decision.tool, "args": decision.args,
                     "confidence": decision.confidence,
                     "reasoning": decision.reasoning,
                     "confirms_by_ts_ms": ts_ms + 60_000},  # 60s TTL per spec §5.1
        )

    async def _write_outcome_memory(
        self, *, bot_guid: int, ts_ms: int,
        memory_type: str, payload: dict[str, Any],
    ) -> None:
        """Write an outcome memory using the live memory-sidecar schema.

        memory.write expects: {bot_id: str, text: str, salience: float,
                               entities: list[str], relations: list[dict]}
        The legacy {memory_type, content, metadata} shape is not accepted.
        """
        # Build a human-readable text summary (max ~200 chars).
        tool = payload.get("tool", "")
        reasoning = payload.get("reasoning", "")
        error = payload.get("error", "")
        if tool:
            text = f"{memory_type}: {tool}"
            if reasoning:
                text += f" — {reasoning}"
            if error:
                text += f" (error: {error})"
        else:
            text = f"{memory_type}: {reasoning or str(payload)}"
        text = text[:200]

        # Entities: always include the bot's own GUID; add tool name when relevant.
        entities: list[str] = [str(bot_guid)]
        if tool:
            entities.append(tool)

        salience = _OUTCOME_SALIENCE.get(memory_type, 0.5)

        try:
            await self.memory_mcp.call("memory.write", {
                "bot_id": str(bot_guid),
                "text": text,
                "salience": salience,
                "entities": entities,
                "relations": [],
                # memory_type is passed as an extra field (ignored by live schema's Pydantic model
                # which has no extra="forbid"). Preserved here so tests and audit logs can identify
                # the outcome category; the live sidecar silently drops it via Pydantic extras='ignore'.
                "memory_type": memory_type,
                "metadata": payload,
            })
        except Exception:
            # Critical log path — but per spec invariant we cannot roll back the tool call.
            log.critical(
                "memory_write failed for outcome bot=%s type=%s payload=%s",
                bot_guid, memory_type, payload,
            )
