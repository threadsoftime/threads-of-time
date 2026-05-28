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


class ObsListPlayersArgs(BaseModel):
    pass


class ObsListBotPopulationArgs(BaseModel):
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


# --- memory.* (V1 memory subsystem — Phase 6B adapters) ---

class _EpisodeType(str, Enum):
    chat        = "chat"
    combat      = "combat"
    social      = "social"
    quest       = "quest"
    discovery   = "discovery"
    goal        = "goal"
    reflection  = "reflection"
    observation = "observation"


class MemoryWriteArgs(BaseModel):
    bot_guid:      int              = Field(..., description="Low-32 GUID of the bot whose memory is written")
    content_text:  str              = Field(..., description="Raw text of the episode to store")
    episode_type:  _EpisodeType     = Field(..., description=(
        "Episode category: chat|combat|social|quest|discovery|goal|reflection|observation"
    ))
    timestamp:     int              = Field(..., description="Unix epoch ms of the in-game event")
    salience_hint: Optional[float]  = Field(None, description=(
        "Caller-supplied salience in [0.0, 1.0]. Overrides the automatic salience scorer."
    ))
    entities:      Optional[list]   = Field(None, description=(
        "List of entity strings (player names, NPC names, item names) referenced in this episode."
    ))
    metadata:      Optional[dict]   = Field(None, description=(
        "Arbitrary key-value metadata stored alongside the episode (not embedded)."
    ))
    source:        Optional[str]    = Field(None, description=(
        "Originating subsystem label, e.g. 'chat', 'combat_log', 'planner'."
    ))


class MemoryReadArgs(BaseModel):
    bot_guid:   int = Field(..., description="Low-32 GUID of the bot")
    episode_id: int = Field(..., description="Primary-key ID of the episode to retrieve")


class MemoryRecallArgs(BaseModel):
    bot_guid:      int             = Field(..., description="Low-32 GUID of the bot")
    query_text:    str             = Field(..., description=(
        "Natural-language query. The sidecar embeds this and ranks episodes by "
        "hybrid relevance (semantic + recency + salience + MMR)."
    ))
    top_k:         Optional[int]   = Field(None, ge=1, description="Max episodes to return (default 10 in sidecar)")
    entity_names:  Optional[list]  = Field(None, description="Filter to episodes referencing these entity strings")
    episode_types: Optional[list]  = Field(None, description=(
        "Filter to these episode_type values. Each element must be a valid _EpisodeType string."
    ))
    time_filter:   Optional[dict]  = Field(None, description=(
        "Time-window filter: {\"after\": <epoch_ms>, \"before\": <epoch_ms>}. Either key is optional."
    ))
    alpha:         Optional[float] = Field(None, description="Semantic similarity weight (0–1; sidecar default 0.5)")
    beta:          Optional[float] = Field(None, description="Recency weight (0–1; sidecar default 0.3)")
    gamma:         Optional[float] = Field(None, description="Salience weight (0–1; sidecar default 0.15)")
    delta:         Optional[float] = Field(None, description="Recall-count (familiarity) weight (0–1; sidecar default 0.05)")
    mmr_lambda:    Optional[float] = Field(None, description=(
        "MMR lambda: 1.0 = pure relevance, 0.0 = pure diversity (sidecar default 0.7)."
    ))


class MemorySearchArgs(BaseModel):
    """Pure nearest-neighbour search — no side effects on recall counters.

    At least one of query_text or query_vec must be provided. The adapter
    validates this at runtime and returns BadArgs when both are absent.
    """
    bot_guid:   int             = Field(..., description="Low-32 GUID of the bot")
    query_text: Optional[str]   = Field(None, description=(
        "Natural-language query; sidecar embeds it for ANN search. "
        "Provide either this or query_vec, not both."
    ))
    query_vec:  Optional[list]  = Field(None, description=(
        "Pre-computed embedding vector (list of floats). "
        "Provide either this or query_text, not both."
    ))
    top_k:      Optional[int]   = Field(None, ge=1, description="Max episodes to return (default 10)")


class MemoryListArgs(BaseModel):
    bot_guid:     int            = Field(..., description="Low-32 GUID of the bot")
    episode_type: Optional[str]  = Field(None, description=(
        "Filter by episode type: chat|combat|social|quest|discovery|goal|reflection|observation. "
        "Omit to return all types."
    ))
    entity_name:  Optional[str]  = Field(None, description=(
        "Filter to episodes referencing this entity string (exact match)."
    ))
    after:        Optional[int]  = Field(None, description="Unix epoch ms lower bound (inclusive)")
    before:       Optional[int]  = Field(None, description="Unix epoch ms upper bound (inclusive)")
    limit:        Optional[int]  = Field(None, ge=1, le=1000, description="Page size (default 50)")
    offset:       Optional[int]  = Field(None, ge=0, description="Pagination offset (default 0)")


class MemoryUpdateArgs(BaseModel):
    """Update mutable fields of an existing episode.

    At least one of content_text, salience_score, or metadata must be
    provided. The adapter returns BadArgs when all three are absent/null.
    """
    bot_guid:      int            = Field(..., description="Low-32 GUID of the bot")
    episode_id:    int            = Field(..., description="Primary-key ID of the episode to update")
    content_text:  Optional[str]  = Field(None, description="New text; triggers re-embedding in the sidecar")
    salience_score: Optional[float] = Field(None, ge=0.0, le=1.0, description=(
        "Override salience score in [0.0, 1.0]."
    ))
    metadata:      Optional[dict] = Field(None, description="Replace the episode's metadata dict")


class MemoryDeleteArgs(BaseModel):
    bot_guid:   int = Field(..., description="Low-32 GUID of the bot")
    episode_id: int = Field(..., description="Primary-key ID of the episode to hard-delete")


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
    "obs.list_players":        (ObsListPlayersArgs,         "List all online players (bots + humans) with GUIDs and basic state."),
    "obs.list_bot_population": (ObsListBotPopulationArgs,  "World-wide bot population snapshot: count, level distribution, zone spread."),
    "obs.get_rpg_status":      (ObsGetRpgStatusArgs,        "Playerbot NewRpgInfo: current goal + description."),
    "obs.get_money":           (ObsGetMoneyArgs,            "Gold/silver/copper balance."),
    "obs.get_position":        (ObsGetPositionArgs,         "Map/zone/area IDs + names + (x,y,z,o)."),
    "obs.get_group":           (ObsGetGroupArgs,            "Party/raid roster, HP/mana%, distances."),
    "obs.get_talents":         (ObsGetTalentsArgs,          "Active-spec talents: flat list with tab/row/col/rank."),
    "obs.query_db":            (ObsQueryDbArgs,             "Run an allowlisted MySQL template against the auth/char DBs."),
    # Memory subsystem (Phase 6B — V1 memory.* tools)
    "memory.write":            (MemoryWriteArgs,            "Write a new episode to a bot's episodic memory store."),
    "memory.read":             (MemoryReadArgs,             "Fetch a single episode by primary-key ID."),
    "memory.recall":           (MemoryRecallArgs,           "Hybrid-ranked recall: semantic + recency + salience + MMR."),
    "memory.search":           (MemorySearchArgs,           "Pure ANN search by text or pre-computed embedding vector."),
    "memory.list":             (MemoryListArgs,             "Paginated episode listing with optional type/entity/time filters."),
    "memory.update":           (MemoryUpdateArgs,           "Patch content, salience, or metadata on an existing episode."),
    "memory.delete":           (MemoryDeleteArgs,           "Hard-delete an episode (cascades to entities + embeddings)."),
}
