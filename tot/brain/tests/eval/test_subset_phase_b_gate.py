# SPDX-License-Identifier: GPL-2.0-or-later
"""Phase B eval gate (spec §9.2):

- Zero denied tool calls leave the brain when REDUCED tier is in force.
- Only bot.set_strategy / bot.set_goal / bot.set_role are permitted to
  reach the MCP client; all other bot.* tools are blocked with
  disposition='policy_denied'.

Tests the integration of ToolPolicyEnforcer with Dispatcher directly —
not the full LoopSupervisor stack (that is covered by T30's integration test).
"""
from __future__ import annotations

import pytest
from unittest.mock import AsyncMock

from brain_sidecar.dispatch import Dispatcher
from brain_sidecar.models import Decision, DecisionKind
from brain_sidecar.tool_policy import ToolPolicyEnforcer


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _action(tool: str, confidence: float = 0.95) -> Decision:
    """Build a minimal ACTION Decision for the given tool."""
    return Decision(
        kind=DecisionKind.ACTION,
        tool=tool,
        args={"target_guid": 1},
        confidence=confidence,
        reasoning="eval gate",
    )


def _make_dispatcher(harness_mcp: AsyncMock, memory_mcp: AsyncMock) -> Dispatcher:
    """Wire a Dispatcher with ToolPolicyEnforcer and the provided MCP mocks.

    Dispatcher constructor fields confirmed from dispatch.py:
      harness_mcp, memory_mcp, confirmation_channel, tool_policy
    """
    return Dispatcher(
        harness_mcp=harness_mcp,
        memory_mcp=memory_mcp,
        tool_policy=ToolPolicyEnforcer(),
    )


# ---------------------------------------------------------------------------
# Eval gate test
# ---------------------------------------------------------------------------

_DENIED_TOOLS = (
    "bot.send_chat",
    "bot.invite_to_group",
    "bot.queue_for_dungeon",
    "bot.follow",
    "bot.accept_invite",
    "bot.leave_group",
    "bot.enter_instance",
    "bot.stop",
)

_ALLOWED_TOOLS = (
    "bot.set_strategy",
    "bot.set_goal",
    "bot.set_role",
)


@pytest.mark.asyncio
async def test_phase_b_denied_tools_never_reach_mcp():
    """Brain emits denied tools in REDUCED tier; dispatcher must block them all.

    Spec §9.2: zero denied tool calls leave the brain in REDUCED tier.
    """
    harness_mcp = AsyncMock()
    memory_mcp = AsyncMock()
    memory_mcp.call = AsyncMock(return_value={"ok": True})
    dispatcher = _make_dispatcher(harness_mcp, memory_mcp)

    for tool in _DENIED_TOOLS:
        decision = _action(tool)
        result = await dispatcher.dispatch(
            bot_guid=999, decision=decision, tier="reduced"
        )
        assert result.disposition == "policy_denied", (
            f"{tool} should be policy_denied in REDUCED tier, "
            f"got disposition={result.disposition!r}"
        )

    # The harness MCP client must never have been called for any denied tool.
    harness_mcp.call.assert_not_called()


@pytest.mark.asyncio
async def test_phase_b_allowed_tools_reach_mcp():
    """Whitelist tools (set_strategy/set_goal/set_role) pass through in REDUCED tier.

    Disposition must NOT be 'policy_denied' — the successful call reaches the
    harness MCP and returns disposition='executed'.

    The confidence is 0.95 (above both low- and high-risk thresholds) so the
    risk gate does not intercept.  bot.set_role is classified 'low' risk;
    bot.set_strategy and bot.set_goal are also 'low'.
    """
    harness_mcp = AsyncMock()
    memory_mcp = AsyncMock()
    harness_mcp.call = AsyncMock(return_value={"ok": True})
    memory_mcp.call = AsyncMock(return_value={"ok": True})
    dispatcher = _make_dispatcher(harness_mcp, memory_mcp)

    for tool in _ALLOWED_TOOLS:
        result = await dispatcher.dispatch(
            bot_guid=999,
            decision=_action(tool, confidence=0.95),
            tier="reduced",
        )
        assert result.disposition != "policy_denied", (
            f"{tool} should NOT be policy_denied in REDUCED tier, "
            f"got disposition={result.disposition!r}"
        )
        assert result.disposition == "executed", (
            f"{tool}: expected disposition='executed', got {result.disposition!r}"
        )

    # harness MCP called exactly once per allowed tool.
    assert harness_mcp.call.await_count == len(_ALLOWED_TOOLS), (
        f"Expected {len(_ALLOWED_TOOLS)} harness MCP calls, "
        f"got {harness_mcp.call.await_count}"
    )
