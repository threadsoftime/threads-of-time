# SPDX-License-Identifier: GPL-2.0-or-later
from brain_sidecar.tool_policy import ToolPolicyEnforcer


def test_full_tier_allows_all_bot_tools():
    enforcer = ToolPolicyEnforcer()
    for tool in (
        "bot.send_chat", "bot.set_strategy", "bot.set_goal", "bot.set_role",
        "bot.invite_to_group", "bot.accept_invite", "bot.leave_group",
        "bot.follow", "bot.queue_for_dungeon", "bot.enter_instance", "bot.stop",
    ):
        assert enforcer.is_allowed(tool, "full"), f"{tool} should be allowed in FULL"


def test_reduced_tier_allows_only_high_level_intent():
    enforcer = ToolPolicyEnforcer()
    for tool in ("bot.set_strategy", "bot.set_goal", "bot.set_role"):
        assert enforcer.is_allowed(tool, "reduced")
    for tool in ("bot.send_chat", "bot.invite_to_group", "bot.accept_invite",
                 "bot.leave_group", "bot.follow", "bot.queue_for_dungeon",
                 "bot.enter_instance", "bot.stop"):
        assert not enforcer.is_allowed(tool, "reduced"), f"{tool} should be denied in REDUCED"


def test_unknown_tier_defaults_to_full():
    enforcer = ToolPolicyEnforcer()
    assert enforcer.is_allowed("bot.send_chat", "unknown_tier") is True
    assert enforcer.is_allowed("bot.set_strategy", "unknown_tier") is True
