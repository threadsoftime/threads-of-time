"""C3: triage gate — decide whether C4 (LLM) should run this tick."""
from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Any, Optional

from brain_sidecar.mcp_clients import McpClient
from brain_sidecar.models import TickState, TriageResult

# Substrings that identify a row as originating from a real player interaction.
# Brain-internal outcome rows (brain_no_op, action_taken, tool_call_failed, …)
# must NOT trigger fresh_chat — they are the dispatcher's own write-backs.
# This tuple is also used as CHAT_PREFIXES by SseConsumer — it is the authoritative
# source for which text prefixes indicate player chat.
_PLAYER_CHAT_SIGNALS: tuple[str, ...] = (
    "received whisper",
    "chat_received",
    "received party",
    "received say",
    "received guild",
    "whisper from",
)

# Public alias used by sse_consumer and loop (avoiding internal-name convention).
CHAT_PREFIXES = _PLAYER_CHAT_SIGNALS

# Prefixes written by the dispatcher itself — exclude these even if they somehow
# contain one of the player signals (shouldn't happen, but be defensive).
_BRAIN_OUTCOME_PREFIXES: tuple[str, ...] = (
    "brain_no_op:",
    "action_taken:",
    "tool_call_failed:",
    "pending_confirmation:",
    "blocked_cross_bot:",
    "decision_invalid_tool:",
)


def _is_player_chat(item: dict[str, Any]) -> bool:
    """Return True if the memory item looks like a real player chat message.

    Filters out brain-internal outcome rows (brain_no_op, action_taken, etc.)
    that the dispatcher writes back and that would otherwise cause a dense-
    retrieval match on the "chat OR whisper" query every tick.
    """
    text = item.get("text") or item.get("content") or ""
    if not isinstance(text, str):
        return False
    # Fast reject: known brain-outcome prefixes are never player chat
    lower = text.lower()
    for prefix in _BRAIN_OUTCOME_PREFIXES:
        if lower.startswith(prefix):
            return False
    # Accept: must contain at least one player-chat signal
    for signal in _PLAYER_CHAT_SIGNALS:
        if signal in lower:
            return True
    return False


@dataclass
class TriageGate:
    harness_mcp: McpClient | Any
    memory_mcp: McpClient | Any
    per_call_timeout_s: float = 5.0

    async def _safe_call(self, mcp: Any, tool: str, args: dict[str, Any]) -> dict[str, Any] | None:
        try:
            raw = await asyncio.wait_for(mcp.call(tool, args), timeout=self.per_call_timeout_s)
            # Both harness-daemon and memory-sidecar wrap responses:
            # {"ok": True, "result": {...}}  or  {"ok": False, "error": "..."}
            # Unwrap the inner result so callers see the domain-level payload directly.
            if isinstance(raw, dict) and "result" in raw:
                return raw["result"]
            return raw
        except (asyncio.TimeoutError, Exception):
            return None

    async def evaluate(
        self,
        *,
        bot_guid: int,
        last_state: TickState,
        now_ms: int,
        sse_inputs: Optional[dict[str, Any]] = None,
        next_wakeup_at_ms: int | None = None,
    ) -> TriageResult:
        """Evaluate whether the LLM (C4) should run this tick.

        ``sse_inputs`` carries SSE-delivered events from SseConsumer:
          - ``fresh_chat``: list of memory-row dicts received via SSE push.

        When ``sse_inputs['fresh_chat']`` is non-empty:
          - skip the memory.search chat poll (SSE already delivered the rows)
          - immediately fire with reason="fresh_chat"
          - the obs/combat/goals fan-out is still run for context

        When ``sse_inputs`` is None or empty, behaviour is unchanged from V3.1.
        """
        # First tick after enroll: always fire (let the brain orient).
        if last_state.last_tick_ms == 0:
            return TriageResult(should_decide=True, reason="first_tick", hot_inputs={})

        # SSE fast-path: if SseConsumer already delivered chat rows, use them directly
        # and skip the memory.search chat poll.
        sse_fresh_chat: list[dict] = (sse_inputs or {}).get("fresh_chat") or []
        has_sse_chat = bool(sse_fresh_chat)

        # Parallel-fan the observation queries.
        # Harness obs.* tools use target_guid (not bot_guid) per the _TargetGuid base model
        # in harness-daemon tool_schemas.py. obs.get_combat_log uses since_ts_ms (not since_ms).
        obs_state_task = self._safe_call(self.harness_mcp, "obs.get_state", {"target_guid": bot_guid})
        combat_task = self._safe_call(
            self.harness_mcp, "obs.get_combat_log",
            {"target_guid": bot_guid, "since_ts_ms": last_state.last_tick_ms},
        )
        # memory-sidecar stores created_ts as Unix seconds (int(time.time())).
        # last_tick_ms is in milliseconds — divide by 1000 to convert to seconds.
        since_ts_s = last_state.last_tick_ms // 1000

        if has_sse_chat:
            # Skip chat poll — SSE already delivered the rows.
            chat_task = _noop_task()
        else:
            chat_task = self._safe_call(
                self.memory_mcp, "memory.search",
                {"bot_id": str(bot_guid), "query": "chat OR whisper",
                 "since_ts": since_ts_s, "top_k": 5},
            )
        goals_task = self._safe_call(
            self.memory_mcp, "goals.list",
            {"bot_id": str(bot_guid), "status": "active"},
        )

        obs_state, combat, chat, goals = await asyncio.gather(
            obs_state_task, combat_task, chat_task, goals_task
        )

        # SSE fast-path: return immediately with the SSE-delivered items.
        # V3.6.1: include state_summary so decide.py can derive at_cap from
        # self.level vs settings.max_player_level. Without this, at_cap is
        # always False in production (V3.6 ship bug surfaced 2026-05-22).
        if has_sse_chat:
            return TriageResult(
                should_decide=True,
                reason="fresh_chat",
                hot_inputs={"fresh_chat": sse_fresh_chat, "state_summary": obs_state or {}},
            )

        # PARTY_INVITE_RECEIVED gate — check obs_state for a pending group invite.
        # Field path confirmed by probe P2.5 (spec §4.3, post-Stage-1.5 fix 2026-05-22):
        # obs_state["self"]["pending_group_invite"] is an object {"from_name": str, "group_type": str}
        # when a pending invite exists, and null otherwise.
        _PENDING_INVITE_FIELD = "pending_group_invite"
        pending_invite = (obs_state or {}).get("self", {}).get(_PENDING_INVITE_FIELD)
        if pending_invite:
            return TriageResult(
                should_decide=True,
                reason="party_invite_received",
                hot_inputs={"pending_invite": pending_invite, "state_summary": obs_state or {}},
            )

        if obs_state is None or combat is None or chat is None or goals is None:
            return TriageResult(
                should_decide=False, reason="triage_timeout", hot_inputs={}
            )

        # Fire conditions (in priority order):
        # Client-side freshness + relevance filter.
        #
        # WHY: The dispatcher writes outcome memories (brain_no_op, action_taken, etc.)
        # on every tick. The memory-sidecar's dense retrieval can match these against the
        # "chat OR whisper" query, causing a feedback loop: triage sees fresh items on
        # every tick even when no real player chat arrived.
        #
        # Two guards:
        #   1. since_ts guard — items returned by the server may still slip through if the
        #      memory row was written during the same second as the query. Reapply the
        #      watermark client-side using the item's "ts" field.
        #   2. text content guard — only treat an item as fresh_chat if its text actually
        #      looks like a player message (contains "whisper" or "received" or "chat_received").
        #      Brain-internal outcome rows (brain_no_op, action_taken, …) are excluded.
        raw_chat_items = (chat or {}).get("items", [])
        fresh_chat = [
            it for it in raw_chat_items
            if it.get("ts", 0) > since_ts_s
            and _is_player_chat(it)
        ]
        if fresh_chat:
            return TriageResult(
                should_decide=True, reason="fresh_chat",
                hot_inputs={"fresh_chat": fresh_chat, "state_summary": obs_state or {}},
            )

        combat_events = (combat or {}).get("events", [])
        if combat_events:
            return TriageResult(
                should_decide=True, reason="combat_event",
                hot_inputs={"combat_events": combat_events, "state_summary": obs_state or {}},
            )

        # V3.7: organic-wakeup path — fires when the bot's self-paced timer
        # (LoopSupervisor._next_wakeup_ms[bot_guid]) has expired. Lowest priority
        # above no_change so external events still pre-empt. obs_state was already
        # awaited above; reuse so the brain has state_summary for at_cap derivation.
        if next_wakeup_at_ms is not None and now_ms >= next_wakeup_at_ms:
            return TriageResult(
                should_decide=True,
                reason="organic_wakeup",
                hot_inputs={"state_summary": obs_state or {}},
            )

        return TriageResult(should_decide=False, reason="no_change", hot_inputs={})


async def _noop_task() -> dict[str, Any]:
    """Return an empty result without calling any MCP (used when SSE skips chat poll)."""
    return {"items": []}
