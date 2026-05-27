"""Per-tool argument schemas exposed to the MCP client.

These are HINTS for the LLM that drives Claude Code — actual argument
validation happens server-side in the AC adapters
(modules/mod-harness-bridge/src/Adapters/*). Keeping these in pure
pydantic lets FastMCP auto-generate JSON Schemas without forcing the
daemon to validate twice.

The shapes here mirror the AC adapters as of V1.3. When a new V1.4+
adapter ships, add its schema here AND register it in mcp_server.py.
"""

from __future__ import annotations

from enum import Enum
from typing import Any, Optional

from pydantic import BaseModel, Field


# --- shared ---

class _TargetGuid(BaseModel):
    target_guid: int = Field(..., description="Low-32 GUID of the online player or bot")


# --- gm.* (mutating) ---

class GmAdditemArgs(_TargetGuid):
    item_entry: int = Field(..., description="WoW item template ID (e.g. 39492)")
    count:      int = Field(1,    description="Stack size; defaults to 1", ge=1)


class GmEquipAllArgs(_TargetGuid):
    pass


class GmTeleportArgs(_TargetGuid):
    map:         int   = Field(..., description="WoW map ID (0=Eastern Kingdoms)")
    x:           float
    y:           float
    z:           float
    orientation: Optional[float] = Field(0.0, description="Facing, radians")


class GmSetLevelArgs(_TargetGuid):
    level: int = Field(..., ge=1, le=80, description="Target level (1..80 for WotLK)")


class GmRunConsoleArgs(BaseModel):
    command: str = Field(..., description=(
        "Console command. Allowlisted prefixes only: "
        "`.lookup`, `.gobject`, `.npc`, `.bracketsets`. "
        "Other prefixes return 403 scope_denied."
    ))


class GmReadConsoleOutputArgs(BaseModel):
    request_id: str = Field(..., description=(
        "Request ID returned by the preceding gm.run_console call "
        "(the daemon echoes it as `request_id` in the response envelope)."
    ))


class GmStripGearArgs(_TargetGuid):
    destroy_if_full: Optional[bool] = Field(False, description=(
        "If true, destroy equipped items the bag can't hold instead of "
        "rejecting with validator_rejected. Smoke-suite use only."
    ))
    clear_bag: Optional[bool] = Field(False, description=(
        "If true, AFTER stripping equipment also DestroyItem every "
        "backpack slot (23-38). Destructive — note this also destroys "
        "the items just stripped from equipment, which now sit in the "
        "bag. Smoke-suite use only."
    ))


# --- bot.* ---

class BotSetGoalArgs(BaseModel):
    bot_guid: int            = Field(..., description="Low-32 GUID of the bot to retask")
    goal:     dict[str, Any] = Field(..., description=(
        "Nested goal object; serialized and handed to the playerbots "
        "ParseAndValidate(json_string) entry point. Shape is fork-specific "
        "(see mod-playerbots Bot/LlmAgent/Schemas/Goal.h). Adapter "
        "rejects with `validator_rejected` on bad shape."
    ))


class _BotState(str, Enum):
    combat     = "combat"
    non_combat = "non_combat"
    dead       = "dead"
    all        = "all"


class BotSetStrategyArgs(BaseModel):
    bot_guid:  int = Field(..., description="Low-32 GUID of the bot")
    strategy:  str = Field(..., description=(
        "ChangeStrategy DSL string. Comma-separated; each entry prefixed: "
        "'+name' to add, '-name' to remove, '~name' to toggle. "
        "Example: '+follow,-passive,-grind'."
    ))
    bot_state: Optional[_BotState] = Field(_BotState.all, description=(
        "Which engine bucket to modify: combat, non_combat, dead, or all "
        "(fans out to combat+non_combat per the chat-shortcut idiom). "
        "Default 'all'."
    ))


class BotGetStrategiesArgs(BaseModel):
    bot_guid:  int                 = Field(..., description="Low-32 GUID of the bot")
    bot_state: Optional[_BotState] = Field(_BotState.all, description=(
        "Which bucket(s) to return: combat, non_combat, dead, or all "
        "(returns all three). Default 'all'."
    ))


class _ChatChannel(str, Enum):
    say     = "say"
    yell    = "yell"
    party   = "party"
    raid    = "raid"
    guild   = "guild"
    whisper = "whisper"


class BotSendChatArgs(BaseModel):
    bot_guid:       int           = Field(..., description="Low-32 GUID of the bot")
    channel:        _ChatChannel  = Field(..., description=(
        "Chat channel: say|yell|party|raid|guild|whisper."
    ))
    message:        str           = Field(..., min_length=1, description="Message text")
    recipient_name: Optional[str] = Field(None, description=(
        "Recipient character name. REQUIRED when channel=='whisper'; "
        "ignored otherwise."
    ))


class BotFollowArgs(BaseModel):
    bot_guid:    int = Field(..., description="Low-32 GUID of the bot")
    leader_guid: int = Field(..., description=(
        "Low-32 GUID of the player to follow. The bot's master is set "
        "to this player, then the +follow strategy is applied to both "
        "combat and non_combat buckets. Rejects if bot is in a group "
        "whose leader is not this guid (formation resolves to group leader)."
    ))


class BotStopArgs(BaseModel):
    bot_guid:     int            = Field(..., description="Low-32 GUID of the bot")
    clear_combat: Optional[bool] = Field(False, description=(
        "If true, also call bot->CombatStop(true) after the stay-strategy "
        "flip. Default false (movement halt only)."
    ))


# V1.5 additions — grouping primitives
class _BotMode(str, Enum):
    lfg    = "lfg"
    direct = "direct"


class _BotRole(str, Enum):
    tank   = "tank"
    healer = "healer"
    dps    = "dps"
    leader = "leader"
    none   = "none"


class BotInviteToGroupArgs(BaseModel):
    bot_guid:    int = Field(..., description="Low-32 GUID of the inviting bot")
    target_guid: int = Field(..., description=(
        "Low-32 GUID of the player (bot or human) to invite. "
        "Must be online and not already in a group."
    ))


class BotAcceptInviteArgs(BaseModel):
    bot_guid: int = Field(..., description="Low-32 GUID of the bot accepting the invite")


class BotLeaveGroupArgs(BaseModel):
    bot_guid: int = Field(..., description="Low-32 GUID of the bot leaving the group")


class BotSetRoleArgs(BaseModel):
    bot_guid: int       = Field(..., description="Low-32 GUID of the bot")
    role:     _BotRole  = Field(..., description=(
        "LFG role: tank|healer|dps|none (CMSG_LFG_SET_ROLES bitmask), "
        "or leader (transfers group leadership via GroupSetLeaderOperation)."
    ))


class BotQueueForDungeonArgs(BaseModel):
    bot_guid:   int = Field(..., description="Low-32 GUID of the bot")
    dungeon_id: int = Field(..., description=(
        "LFGDungeonEntry ID. 0 = random level-appropriate dungeon."
    ))
    roles_mask: int = Field(..., description=(
        "PLAYER_ROLE_* bitmask. 0 = auto-detect from bot spec. "
        "Actual constant values captured via probe P5."
    ))


class BotEnterInstanceArgs(BaseModel):
    bot_guid:    int             = Field(..., description="Low-32 GUID of the bot")
    mode:        _BotMode        = Field(..., description="'lfg' or 'direct'.")
    map_id:      Optional[int]   = Field(None, description="Required when mode='direct'.")
    x:           Optional[float] = Field(0.0,  description="X coordinate. Used when mode='direct'.")
    y:           Optional[float] = Field(0.0,  description="Y coordinate. Used when mode='direct'.")
    z:           Optional[float] = Field(0.0,  description="Z coordinate. Used when mode='direct'.")
    orientation: Optional[float] = Field(0.0,  description="Facing radians. Used when mode='direct'.")


# --- obs.* (read-only) ---

class ObsPingArgs(BaseModel):
    pass


class ObsGetStateArgs(_TargetGuid):
    pass


class ObsGetAurasArgs(_TargetGuid):
    pass


class ObsGetInventoryArgs(_TargetGuid):
    pass


class ObsGetCombatLogArgs(_TargetGuid):
    since_ts_ms: Optional[int] = Field(None, description=(
        "Milliseconds since epoch; events older than this are excluded. "
        "Omit for the full ring-buffer window."
    ))
    limit: Optional[int] = Field(None, ge=1, le=500, description=(
        "Max events to return (default 200, hard cap 500)."
    ))


# V1.3 additions
class ObsGetQuestLogArgs(_TargetGuid):
    pass


class ObsGetXpArgs(_TargetGuid):
    pass


class ObsGetRpgStatusArgs(_TargetGuid):
    pass


class ObsGetMoneyArgs(_TargetGuid):
    pass


class ObsGetPositionArgs(_TargetGuid):
    pass


class ObsGetGroupArgs(_TargetGuid):
    pass


class ObsGetTalentsArgs(_TargetGuid):
    pass


# --- daemon-direct ---

class ObsQueryDbArgs(BaseModel):
    template_name: str   = Field(..., description="Allowlisted query template name")
    params:        dict  = Field(default_factory=dict, description="Template params")


# --- registry: name -> (schema_class, one-line description) ---

TOOL_SCHEMAS: dict[str, tuple[type[BaseModel], str]] = {
    "gm.additem":              (GmAdditemArgs,              "Add `count` of `item_id` to a player's inventory."),
    "gm.equip_all":            (GmEquipAllArgs,             "Equip the best in-bag gear for every slot."),
    "gm.teleport":             (GmTeleportArgs,             "Teleport the player to (map, x, y, z, orientation)."),
    "gm.set_level":            (GmSetLevelArgs,             "Set the player's level (1..80 WotLK)."),
    "gm.run_console":          (GmRunConsoleArgs,           "Run an allowlisted server console command."),
    "gm.read_console_output": (GmReadConsoleOutputArgs,    "Read buffered console output from the last invocation."),
    "gm.strip_gear":           (GmStripGearArgs,            "Unequip every item from the player to their bags."),
    "bot.set_goal":            (BotSetGoalArgs,             "Set a playerbot's next RPG goal."),
    "bot.set_strategy":        (BotSetStrategyArgs,         "Add/remove/toggle strategies on a bot's engine buckets."),
    "bot.get_strategies":      (BotGetStrategiesArgs,       "Current active + available strategy names per bucket."),
    "bot.send_chat":           (BotSendChatArgs,            "Have a bot say/yell/whisper or speak in party/raid/guild."),
    "bot.follow":              (BotFollowArgs,              "Set master + apply +follow strategy (durable)."),
    "bot.stop":                (BotStopArgs,                "Apply +stay,-follow strategy (optionally also CombatStop)."),
    # V1.5 additions — grouping primitives
    "bot.invite_to_group":     (BotInviteToGroupArgs,       "Invite a player into the bot's group (or create one)."),
    "bot.accept_invite":       (BotAcceptInviteArgs,        "Accept a pending group invitation."),
    "bot.leave_group":         (BotLeaveGroupArgs,          "Leave (or disband if leader) the bot's current group."),
    "bot.set_role":            (BotSetRoleArgs,             "Set LFG role or transfer group leadership."),
    "bot.queue_for_dungeon":   (BotQueueForDungeonArgs,     "Queue the bot for a dungeon via LFG."),
    "bot.enter_instance":      (BotEnterInstanceArgs,       "Teleport bot into a dungeon (lfg teleport or direct)."),
    "obs.ping":                (ObsPingArgs,                "Health check — returns {pong:true}."),
    "obs.get_state":           (ObsGetStateArgs,            "Player state: level, hp/mana, class, race, position, combat."),
    "obs.get_auras":           (ObsGetAurasArgs,            "Active auras (buffs/debuffs) on the player."),
    "obs.get_inventory":       (ObsGetInventoryArgs,        "Full inventory: bags, equipped, bank stub."),
    "obs.get_combat_log":      (ObsGetCombatLogArgs,        "Recent combat-log events for the player."),
    "obs.get_quest_log":       (ObsGetQuestLogArgs,         "Quest log: open quests + objectives + status."),
    "obs.get_xp":              (ObsGetXpArgs,               "Current XP, next-level threshold, rested XP, %."),
    "obs.get_rpg_status":      (ObsGetRpgStatusArgs,        "Playerbot NewRpgInfo: current goal + description."),
    "obs.get_money":           (ObsGetMoneyArgs,            "Gold/silver/copper balance."),
    "obs.get_position":        (ObsGetPositionArgs,         "Map/zone/area IDs + names + (x,y,z,o)."),
    "obs.get_group":           (ObsGetGroupArgs,            "Party/raid roster, HP/mana%, distances."),
    "obs.get_talents":         (ObsGetTalentsArgs,          "Active-spec talents: flat list with tab/row/col/rank."),
    "obs.query_db":            (ObsQueryDbArgs,             "Run an allowlisted MySQL template against the auth/char DBs."),
}
