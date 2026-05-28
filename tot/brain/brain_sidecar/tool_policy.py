# SPDX-License-Identifier: GPL-2.0-or-later
"""Phase B: tier-aware tool whitelist enforcement."""
from __future__ import annotations

from dataclasses import dataclass


_FULL_WHITELIST = frozenset({
    "bot.send_chat", "bot.set_strategy", "bot.set_goal", "bot.set_role",
    "bot.invite_to_group", "bot.accept_invite", "bot.leave_group",
    "bot.follow", "bot.queue_for_dungeon", "bot.enter_instance", "bot.stop",
    "bot.combat_stop",  # if shipped in V1.5+; harmless if not
})

_REDUCED_WHITELIST = frozenset({
    "bot.set_strategy", "bot.set_goal", "bot.set_role",
})

TIER_TOOL_WHITELIST: dict[str, frozenset[str]] = {
    "full": _FULL_WHITELIST,
    "reduced": _REDUCED_WHITELIST,
}


@dataclass(frozen=True)
class ToolPolicyEnforcer:
    def is_allowed(self, tool_name: str, tier: str) -> bool:
        whitelist = TIER_TOOL_WHITELIST.get(tier, _FULL_WHITELIST)
        return tool_name in whitelist
