"""V1 tool registry.

Each ToolEntry declares: name, required scope, subject-GUID arg name
(for self-binding), forwards-to-AC flag, and a pydantic args schema.
Spec §6 lists the 12 V1 tools.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional


class ToolNotFound(KeyError):
    pass


class SelfBindingViolation(ValueError):
    pass


@dataclass(frozen=True)
class ToolEntry:
    name:               str
    required_scope:     str
    subject_guid_arg:   Optional[str]   # None for tools without a subject
    forwards_to_ac:     bool            # False = daemon-direct (obs.query_db)
    tool_version:       int = 1

    def check_self_binding(self, args: dict[str, Any], bound_to_guid: int) -> None:
        """Enforce that the request targets the token's bound bot/player.

        Called by the auth path ONLY when the matched scope pattern is
        `<ns>.self.*`. For non-self scopes this is skipped.
        """
        if self.subject_guid_arg is None:
            # Tool has no subject GUID — self-scope tokens can't call it.
            raise SelfBindingViolation("not_bound: tool has no subject GUID")
        if self.subject_guid_arg not in args:
            raise SelfBindingViolation(f"missing subject arg '{self.subject_guid_arg}'")
        subject = args[self.subject_guid_arg]
        if subject != bound_to_guid:
            raise SelfBindingViolation(
                f"not_bound: subject {subject} != token bound_to_guid {bound_to_guid}"
            )


class Registry:
    def __init__(self, entries: list[ToolEntry]) -> None:
        self._by_name = {e.name: e for e in entries}

    def find(self, name: str) -> ToolEntry:
        try:
            return self._by_name[name]
        except KeyError:
            raise ToolNotFound(name)

    def names(self) -> list[str]:
        return sorted(self._by_name)


def build_v1_registry() -> Registry:
    """Build the V1 registry per spec §6.

    Tasks 14-19 are still in progress; tools whose AC adapters are not
    yet implemented are listed here anyway so the daemon-side wire is
    stable, and the AC bridge returns `unknown_tool` until the adapter
    lands.
    """
    return Registry([
        # GM-tier
        ToolEntry("gm.additem",                "gm.additem",                "target_guid", True),
        ToolEntry("gm.equip_all",              "gm.equip_all",              "target_guid", True),
        ToolEntry("gm.teleport",               "gm.teleport",               "target_guid", True),
        ToolEntry("gm.set_level",              "gm.set_level",              "target_guid", True),
        ToolEntry("gm.run_console",            "gm.run_console",            None,          True),
        ToolEntry("gm.read_console_output",    "gm.read_console_output",    None,          True),
        ToolEntry("gm.strip_gear",             "gm.strip_gear",             "target_guid", True),
        # Bot-tier
        ToolEntry("bot.set_goal",              "bot.set_goal",              "bot_guid",    True),
        # Bot-tier (V1.4)
        ToolEntry("bot.set_strategy",          "bot.set_strategy",          "bot_guid",    True),
        ToolEntry("bot.get_strategies",        "bot.get_strategies",        "bot_guid",    True),
        ToolEntry("bot.send_chat",             "bot.send_chat",             "bot_guid",    True),
        ToolEntry("bot.follow",                "bot.follow",                "bot_guid",    True),
        ToolEntry("bot.stop",                  "bot.stop",                  "bot_guid",    True),
        # Bot-tier (V1.5 — grouping primitives)
        ToolEntry("bot.invite_to_group",      "bot.invite_to_group",        "bot_guid",    True),
        ToolEntry("bot.accept_invite",        "bot.accept_invite",          "bot_guid",    True),
        ToolEntry("bot.leave_group",          "bot.leave_group",            "bot_guid",    True),
        ToolEntry("bot.set_role",             "bot.set_role",               "bot_guid",    True),
        ToolEntry("bot.queue_for_dungeon",    "bot.queue_for_dungeon",      "bot_guid",    True),
        ToolEntry("bot.enter_instance",       "bot.enter_instance",         "bot_guid",    True),
        # Observation
        ToolEntry("obs.ping",                  "obs.ping",                  None,          True),
        ToolEntry("obs.get_state",             "obs.get_state",             "target_guid", True),
        ToolEntry("obs.get_auras",             "obs.get_auras",             "target_guid", True),
        ToolEntry("obs.get_inventory",         "obs.get_inventory",         "target_guid", True),
        ToolEntry("obs.get_combat_log",        "obs.get_combat_log",        "target_guid", True),
        # V1.3 additions
        ToolEntry("obs.get_quest_log",         "obs.get_quest_log",         "target_guid", True),
        ToolEntry("obs.get_xp",                "obs.get_xp",                "target_guid", True),
        ToolEntry("obs.get_rpg_status",        "obs.get_rpg_status",        "target_guid", True),
        ToolEntry("obs.get_money",             "obs.get_money",             "target_guid", True),
        ToolEntry("obs.get_position",          "obs.get_position",          "target_guid", True),
        ToolEntry("obs.get_group",             "obs.get_group",             "target_guid", True),
        # Observation (V1.4)
        ToolEntry("obs.get_talents",           "obs.get_talents",           "target_guid", True),
        # Daemon-direct
        ToolEntry("obs.query_db",              "obs.query_db",              None,          False),
    ])
