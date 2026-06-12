//! Per-tool argument schemas for the MCP adapter.
//!
//! Port of `harness_daemon/tool_schemas.py` — one `XxxArgs` inner struct plus
//! one `XxxWrapper { pub args: XxxArgs }` per tool. Both carry
//! `#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]`.
//!
//! `Serialize` is required by the MCP handler to call `serde_json::to_value(&w.args)`
//! (the exclude_none path in `forward()`).
//!
//! All 54 tools from `TOOL_SCHEMAS` are represented here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Shared enums ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotState {
    Combat,
    NonCombat,
    Dead,
    All,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannel {
    Say,
    Yell,
    Party,
    Raid,
    Guild,
    Whisper,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotRole {
    Tank,
    Healer,
    Dps,
    Leader,
    None,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotMode {
    Lfg,
    Direct,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeType {
    Chat,
    Combat,
    Social,
    Quest,
    Discovery,
    Goal,
    Reflection,
    Observation,
}

// ── Sub-model: LfgFormGroupMember ─────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LfgFormGroupMember {
    /// Player low GUID to force-add to the group.
    pub guid: i64,
    /// LFG role bitmask (TANK=2, HEALER=4, DAMAGE=8).
    pub roles: i64,
}

// ── gm.additem ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmAdditemArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
    /// WoW item template ID (e.g. 39492).
    pub item_entry: i64,
    /// Stack size; defaults to 1.
    #[serde(default = "default_count")]
    #[schemars(range(min = 1))]
    pub count: i64,
}

fn default_count() -> i64 { 1 }

fn default_false_opt() -> Option<bool> { Some(false) }

fn default_empty_object() -> serde_json::Value { serde_json::json!({}) }

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmAdditemWrapper {
    pub args: GmAdditemArgs,
}

// ── gm.equip_all ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmEquipAllArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmEquipAllWrapper {
    pub args: GmEquipAllArgs,
}

// ── gm.teleport ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmTeleportArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
    /// WoW map ID (0=Eastern Kingdoms).
    pub map: i64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Facing, radians.
    #[serde(default)]
    pub orientation: f64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmTeleportWrapper {
    pub args: GmTeleportArgs,
}

// ── gm.set_level ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmSetLevelArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
    /// Target level (1..80 for WotLK).
    #[schemars(range(min = 1, max = 80))]
    pub level: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmSetLevelWrapper {
    pub args: GmSetLevelArgs,
}

// ── gm.run_console ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmRunConsoleArgs {
    /// Console command. Allowlisted prefixes only.
    pub command: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmRunConsoleWrapper {
    pub args: GmRunConsoleArgs,
}

// ── gm.read_console_output ───────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmReadConsoleOutputArgs {
    /// Request ID returned by the preceding gm.run_console call.
    pub request_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmReadConsoleOutputWrapper {
    pub args: GmReadConsoleOutputArgs,
}

// ── gm.strip_gear ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmStripGearArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
    /// If true, destroy equipped items the bag can't hold.
    #[serde(default = "default_false_opt")]
    pub destroy_if_full: Option<bool>,
    /// If true, AFTER stripping also DestroyItem every backpack slot.
    #[serde(default = "default_false_opt")]
    pub clear_bag: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GmStripGearWrapper {
    pub args: GmStripGearArgs,
}

// ── bot.set_goal ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSetGoalArgs {
    /// Low-32 GUID of the bot to retask.
    pub bot_guid: i64,
    /// Nested goal object.
    pub goal: Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSetGoalWrapper {
    pub args: BotSetGoalArgs,
}

// ── bot.set_strategy ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSetStrategyArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// ChangeStrategy DSL string.
    pub strategy: String,
    /// Which engine bucket to modify.
    #[serde(default = "default_bot_state")]
    pub bot_state: Option<BotState>,
}

fn default_bot_state() -> Option<BotState> { Some(BotState::All) }

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSetStrategyWrapper {
    pub args: BotSetStrategyArgs,
}

// ── bot.get_strategies ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotGetStrategiesArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// Which bucket(s) to return.
    #[serde(default = "default_bot_state")]
    pub bot_state: Option<BotState>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotGetStrategiesWrapper {
    pub args: BotGetStrategiesArgs,
}

// ── bot.send_chat ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSendChatArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// Chat channel: say|yell|party|raid|guild|whisper.
    pub channel: ChatChannel,
    /// Message text.
    #[schemars(length(min = 1))]
    pub message: String,
    /// Recipient character name (REQUIRED when channel==whisper).
    #[serde(default)]
    pub recipient_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSendChatWrapper {
    pub args: BotSendChatArgs,
}

// ── bot.follow ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotFollowArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// Low-32 GUID of the player to follow.
    pub leader_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotFollowWrapper {
    pub args: BotFollowArgs,
}

// ── bot.stop ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotStopArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// If true, also call CombatStop(true).
    #[serde(default = "default_false_opt")]
    pub clear_combat: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotStopWrapper {
    pub args: BotStopArgs,
}

// ── bot.invite_to_group ──────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotInviteToGroupArgs {
    /// Low-32 GUID of the inviting bot.
    pub bot_guid: i64,
    /// Low-32 GUID of the player to invite.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotInviteToGroupWrapper {
    pub args: BotInviteToGroupArgs,
}

// ── bot.accept_invite ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotAcceptInviteArgs {
    /// Low-32 GUID of the bot accepting the invite.
    pub bot_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotAcceptInviteWrapper {
    pub args: BotAcceptInviteArgs,
}

// ── bot.leave_group ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotLeaveGroupArgs {
    /// Low-32 GUID of the bot leaving the group.
    pub bot_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotLeaveGroupWrapper {
    pub args: BotLeaveGroupArgs,
}

// ── bot.set_role ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSetRoleArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// LFG role: tank|healer|dps|none or leader.
    pub role: BotRole,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSetRoleWrapper {
    pub args: BotSetRoleArgs,
}

// ── bot.queue_for_dungeon ────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotQueueForDungeonArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// LFGDungeonEntry ID. 0 = random level-appropriate dungeon.
    pub dungeon_id: i64,
    /// LFG role bitmask.
    #[serde(default)]
    pub roles_mask: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotQueueForDungeonWrapper {
    pub args: BotQueueForDungeonArgs,
}

// ── bot.enter_instance ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotEnterInstanceArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// 'lfg' or 'direct'.
    pub mode: BotMode,
    /// Required when mode='direct'.
    #[serde(default)]
    pub map_id: Option<i64>,
    #[serde(default = "default_zero_f64")]
    pub x: Option<f64>,
    #[serde(default = "default_zero_f64")]
    pub y: Option<f64>,
    #[serde(default = "default_zero_f64")]
    pub z: Option<f64>,
    #[serde(default = "default_zero_f64")]
    pub orientation: Option<f64>,
}

fn default_zero_f64() -> Option<f64> { Some(0.0) }

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotEnterInstanceWrapper {
    pub args: BotEnterInstanceArgs,
}

// ── obs.ping ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsPingArgs {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsPingWrapper {
    pub args: ObsPingArgs,
}

// ── obs.get_state ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetStateArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetStateWrapper {
    pub args: ObsGetStateArgs,
}

// ── obs.get_auras ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetAurasArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetAurasWrapper {
    pub args: ObsGetAurasArgs,
}

// ── obs.get_inventory ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetInventoryArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetInventoryWrapper {
    pub args: ObsGetInventoryArgs,
}

// ── obs.get_combat_log ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetCombatLogArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
    /// Milliseconds since epoch; events older than this are excluded.
    #[serde(default)]
    pub since_ts_ms: Option<i64>,
    /// Max events to return (default 200, hard cap 500).
    #[serde(default)]
    #[schemars(range(min = 1, max = 500))]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetCombatLogWrapper {
    pub args: ObsGetCombatLogArgs,
}

// ── obs.get_quest_log ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetQuestLogArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetQuestLogWrapper {
    pub args: ObsGetQuestLogArgs,
}

// ── obs.get_xp ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetXpArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetXpWrapper {
    pub args: ObsGetXpArgs,
}

// ── obs.list_players ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsListPlayersArgs {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsListPlayersWrapper {
    pub args: ObsListPlayersArgs,
}

// ── obs.list_bot_population ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsListBotPopulationArgs {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsListBotPopulationWrapper {
    pub args: ObsListBotPopulationArgs,
}

// ── obs.get_rpg_status ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetRpgStatusArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetRpgStatusWrapper {
    pub args: ObsGetRpgStatusArgs,
}

// ── obs.get_money ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetMoneyArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetMoneyWrapper {
    pub args: ObsGetMoneyArgs,
}

// ── obs.get_position ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetPositionArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetPositionWrapper {
    pub args: ObsGetPositionArgs,
}

// ── obs.get_group ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetGroupArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetGroupWrapper {
    pub args: ObsGetGroupArgs,
}

// ── obs.get_talents ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetTalentsArgs {
    /// Low-32 GUID of the online player or bot.
    pub target_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetTalentsWrapper {
    pub args: ObsGetTalentsArgs,
}

// ── obs.game_events ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGameEventsArgs {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGameEventsWrapper {
    pub args: ObsGameEventsArgs,
}

// ── obs.query_db ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsQueryDbArgs {
    /// Allowlisted query template name.
    pub template_name: String,
    /// Template params.
    #[serde(default = "default_empty_object")]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsQueryDbWrapper {
    pub args: ObsQueryDbArgs,
}

// ── obs.lfg_pending ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsLfgPendingArgs {
    /// Max pending LFG intents to drain in this call.
    #[serde(default = "default_max")]
    pub max: i64,
}

fn default_max() -> i64 { 64 }

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsLfgPendingWrapper {
    pub args: ObsLfgPendingArgs,
}

// ── lfg.cancel ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LfgCancelArgs {
    /// Max cancel intents to drain in this call.
    #[serde(default = "default_max")]
    pub max: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LfgCancelWrapper {
    pub args: LfgCancelArgs,
}

// ── event.start ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EventStartArgs {
    /// Game event ID (1..N).
    #[schemars(range(min = 1))]
    pub event_id: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EventStartWrapper {
    pub args: EventStartArgs,
}

// ── event.stop ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EventStopArgs {
    /// Game event ID (1..N).
    #[schemars(range(min = 1))]
    pub event_id: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EventStopWrapper {
    pub args: EventStopArgs,
}

// ── memory.write ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryWriteArgs {
    /// Low-32 GUID of the bot whose memory is written.
    pub bot_guid: i64,
    /// Raw text of the episode to store.
    pub content_text: String,
    /// Episode category.
    pub episode_type: EpisodeType,
    /// Unix epoch ms of the in-game event.
    pub timestamp: i64,
    /// Caller-supplied salience in [0.0, 1.0].
    #[serde(default)]
    pub salience_hint: Option<f64>,
    /// Entity strings referenced in this episode.
    #[serde(default)]
    pub entities: Option<Vec<Value>>,
    /// Arbitrary key-value metadata.
    #[serde(default)]
    pub metadata: Option<Value>,
    /// Originating subsystem label.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryWriteWrapper {
    pub args: MemoryWriteArgs,
}

// ── memory.read ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryReadArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// Primary-key ID of the episode to retrieve.
    pub episode_id: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryReadWrapper {
    pub args: MemoryReadArgs,
}

// ── memory.recall ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// Natural-language query.
    pub query_text: String,
    /// Max episodes to return.
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub top_k: Option<i64>,
    /// Filter to episodes referencing these entity strings.
    #[serde(default)]
    pub entity_names: Option<Vec<Value>>,
    /// Filter to these episode_type values.
    #[serde(default)]
    pub episode_types: Option<Vec<Value>>,
    /// Time-window filter.
    #[serde(default)]
    pub time_filter: Option<Value>,
    /// Semantic similarity weight (0-1).
    #[serde(default)]
    pub alpha: Option<f64>,
    /// Recency weight (0-1).
    #[serde(default)]
    pub beta: Option<f64>,
    /// Salience weight (0-1).
    #[serde(default)]
    pub gamma: Option<f64>,
    /// Recall-count weight (0-1).
    #[serde(default)]
    pub delta: Option<f64>,
    /// MMR lambda (1.0 = pure relevance, 0.0 = pure diversity).
    #[serde(default)]
    pub mmr_lambda: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallWrapper {
    pub args: MemoryRecallArgs,
}

// ── memory.search ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemorySearchArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// Natural-language query; provide either this or query_vec.
    #[serde(default)]
    pub query_text: Option<String>,
    /// Pre-computed embedding vector; provide either this or query_text.
    #[serde(default)]
    pub query_vec: Option<Vec<Value>>,
    /// Max episodes to return.
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub top_k: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemorySearchWrapper {
    pub args: MemorySearchArgs,
}

// ── memory.list ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryListArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// Filter by episode type.
    #[serde(default)]
    pub episode_type: Option<String>,
    /// Filter to episodes referencing this entity string.
    #[serde(default)]
    pub entity_name: Option<String>,
    /// Unix epoch ms lower bound (inclusive).
    #[serde(default)]
    pub after: Option<i64>,
    /// Unix epoch ms upper bound (inclusive).
    #[serde(default)]
    pub before: Option<i64>,
    /// Page size (default 50).
    #[serde(default)]
    #[schemars(range(min = 1, max = 1000))]
    pub limit: Option<i64>,
    /// Pagination offset (default 0).
    #[serde(default)]
    #[schemars(range(min = 0))]
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryListWrapper {
    pub args: MemoryListArgs,
}

// ── memory.update ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryUpdateArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// Primary-key ID of the episode to update.
    pub episode_id: i64,
    /// New text; triggers re-embedding in the sidecar.
    #[serde(default)]
    pub content_text: Option<String>,
    /// Override salience score in [0.0, 1.0].
    #[serde(default)]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub salience_score: Option<f64>,
    /// Replace the episode's metadata dict.
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryUpdateWrapper {
    pub args: MemoryUpdateArgs,
}

// ── memory.delete ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryDeleteArgs {
    /// Low-32 GUID of the bot.
    pub bot_guid: i64,
    /// Primary-key ID of the episode to hard-delete.
    pub episode_id: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MemoryDeleteWrapper {
    pub args: MemoryDeleteArgs,
}

// ── nav.find_path ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct NavFindPathArgs {
    /// Low-32 GUID of the bot whose position anchors the path query.
    pub bot_guid: i64,
    /// Destination X coordinate (WoW world-space).
    pub dest_x: f64,
    /// Destination Y coordinate (WoW world-space).
    pub dest_y: f64,
    /// Destination Z coordinate (WoW world-space).
    pub dest_z: f64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct NavFindPathWrapper {
    pub args: NavFindPathArgs,
}

// ── bot.move_path ─────────────────────────────────────────────────────────────

/// One waypoint in a navmesh path. Mirrors `exec_rs::nav::PathPoint`.
/// First `Vec<struct>` arg besides `Vec<LfgFormGroupMember>` — the precedent
/// confirming schemars emits a $defs ref for nested structs.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PathPointArg {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotMovePathArgs {
    /// Low-32 GUID of the bot to move.
    pub bot_guid: i64,
    /// Ordered waypoints (≥2); typically the `points` from a preceding `nav.find_path`.
    pub points: Vec<PathPointArg>,
    /// If true, use FORCED_MOVEMENT_RUN (run speed).
    /// If false or omitted, use FORCED_MOVEMENT_WALK (default).
    /// Omit for backward-compat default (walk).
    #[serde(default)]
    pub run: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotMovePathWrapper {
    pub args: BotMovePathArgs,
}

// ── bot.dismount ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotDismountArgs {
    /// Low-32 GUID of the bot to dismount.
    /// Idempotent: if the bot is not mounted, returns dismounted:true immediately.
    /// Requires the bot to be claimed (externally owned).
    pub bot_guid: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotDismountWrapper {
    pub args: BotDismountArgs,
}

// ── bot.accept_quest ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotAcceptQuestArgs {
    /// Low-32 GUID of the bot accepting the quest.
    pub bot_guid: i64,
    /// Quest template ID (quest_template.ID in world DB).
    pub quest_id: i64,
    /// Packed raw uint64 ObjectGuid of the NPC or GO quest giver.
    /// Obtained from obs.get_nearby_hostiles/obs.get_lootable_corpses guid field
    /// or from a DB probe (creature/gameobject table spawn → GetRawValue()).
    /// Same format as bot.loot target_guid and bot.attack target_guid.
    /// MUST be u64: a packed NPC GUID (HighGuid bits set) exceeds i64::MAX.
    pub quest_giver_guid: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotAcceptQuestWrapper {
    pub args: BotAcceptQuestArgs,
}

// ── bot.turnin_quest ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotTurninQuestArgs {
    /// Low-32 GUID of the bot turning in the quest.
    pub bot_guid: i64,
    /// Quest template ID.
    pub quest_id: i64,
    /// Packed raw uint64 ObjectGuid of the NPC or GO quest ender.
    /// Same format as quest_giver_guid in bot.accept_quest.
    /// MUST be u64: packed GUIDs exceed i64::MAX.
    pub quest_giver_guid: u64,
    /// Zero-based index into the quest's reward_choice_items list.
    /// Omit or pass 0 for quests with no choice or to pick index 0.
    #[serde(default)]
    pub reward_choice: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotTurninQuestWrapper {
    pub args: BotTurninQuestArgs,
}

// ── bot.use_item ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotUseItemArgs {
    /// Low-32 GUID of the bot using the item.
    pub bot_guid: i64,
    /// Item template ID. Bot must have at least one in bags or equipped.
    pub item_entry: i64,
    /// Packed raw uint64 ObjectGuid of the target. Omit to use on self (0).
    /// Phase-1: targeted path returns fail_code:'targeted_phase2' — implement in phase-2.
    /// MUST be u64: packed GUIDs exceed i64::MAX.
    #[serde(default)]
    pub target_guid: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotUseItemWrapper {
    pub args: BotUseItemArgs,
}

// ── bot.interact_object ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotInteractObjectArgs {
    /// Low-32 GUID of the bot interacting with the object.
    pub bot_guid: i64,
    /// Packed raw uint64 ObjectGuid of the specific gameobject spawn.
    /// Preferred when available — unambiguous.
    /// Mutually exclusive with object_entry (object_guid wins if both given).
    /// MUST be u64: packed GUIDs exceed i64::MAX.
    #[serde(default)]
    pub object_guid: Option<u64>,
    /// Gameobject template entry ID.
    /// Adapter finds the nearest spawn of this type within search_range yards.
    /// Use when the specific spawn GUID is not known.
    #[serde(default)]
    pub object_entry: Option<i64>,
    /// Radius (yards) for object_entry nearest-search. Default 25yd.
    /// Ignored when object_guid is provided. Hard cap 100yd enforced by adapter.
    #[serde(default)]
    pub search_range: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotInteractObjectWrapper {
    pub args: BotInteractObjectArgs,
}

// ── bot.set_ai_enabled ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSetAiEnabledArgs {
    /// Low-32 GUID of the bot to own/release.
    pub bot_guid: i64,
    /// true = mod-playerbots AI active; false = AI suppressed (new system owns the bot).
    pub enabled: bool,
    /// When releasing (enabled=true), have the C++ side call PlayerbotAI::Reset(true).
    /// Absent → C++ adapter default (true).
    #[serde(default)]
    pub reset_on_release: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotSetAiEnabledWrapper { pub args: BotSetAiEnabledArgs }

// ── bot.attack ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotAttackArgs {
    /// Low-32 GUID of the externally-owned bot.
    pub bot_guid: i64,
    /// Packed creature uint64 (raw ObjectGuid) — from obs.get_nearby_hostiles `guid` field.
    /// MUST be u64: a packed creature GUID (HighGuid bits set) exceeds i64::MAX and would
    /// fail i64 deserialization, rejecting the call before it forwards to AC.
    pub target_guid: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotAttackWrapper { pub args: BotAttackArgs }

// ── bot.cast_spell ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotCastSpellArgs {
    /// Low-32 GUID of the externally-owned bot.
    pub bot_guid: i64,
    /// Rank-1/base spell id. The adapter upranks to the highest rank in the
    /// bot's spellbook via the sSpellMgr spell chain.
    pub spell_id: u32,
    /// Packed target uint64 (raw ObjectGuid). Omit (or 0) for self-cast.
    /// MUST be u64 (see BotAttackArgs.target_guid): packed GUIDs exceed i64::MAX.
    #[serde(default)]
    pub target_guid: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotCastSpellWrapper { pub args: BotCastSpellArgs }

// ── bot.vendor_sell ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotVendorSellArgs {
    /// Low-32 GUID of the externally-owned bot.
    pub bot_guid: i64,
    /// DB creature spawn id (`creature.guid` column) of the vendor. Resolved server-side via the map's spawn-id store; NOT a packed runtime ObjectGuid (callers cannot obtain those for friendly NPCs).
    pub vendor_spawn_id: u64,
    /// Sell items of quality <= this (0=grey only). Defaults to 0.
    #[serde(default)]
    pub max_quality: u8,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotVendorSellWrapper { pub args: BotVendorSellArgs }

// ── bot.repair ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotRepairArgs {
    /// Low-32 GUID of the externally-owned bot.
    pub bot_guid: i64,
    /// DB creature spawn id (`creature.guid` column) of the vendor. Resolved server-side via the map's spawn-id store; NOT a packed runtime ObjectGuid (callers cannot obtain those for friendly NPCs).
    pub vendor_spawn_id: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotRepairWrapper { pub args: BotRepairArgs }

// ── bot.mail ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotMailArgs {
    /// Low-32 GUID of the externally-owned bot.
    pub bot_guid: i64,
    /// Recipient character name (resolved server-side via the character cache).
    pub recipient: String,
    /// Packed item uint64 guids from the bot's bags. MUST be u64 elements.
    #[serde(default)]
    pub item_guids: Vec<u64>,
    /// Copper to attach. The 30c postage is charged on top, server-side.
    #[serde(default)]
    pub copper: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotMailWrapper { pub args: BotMailArgs }

// ── obs.get_nearby_hostiles ───────────────────────────────────────────────────

fn default_hostiles_radius() -> f64 { 40.0 }

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetNearbyHostilesArgs {
    /// Low-32 GUID of the bot used as the search origin.
    pub bot_guid: i64,
    /// Search radius in yards (cap 100). Defaults to 40.
    #[serde(default = "default_hostiles_radius")]
    pub radius: f64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetNearbyHostilesWrapper { pub args: ObsGetNearbyHostilesArgs }

// ── obs.get_lootable_corpses ──────────────────────────────────────────────────

fn default_corpses_radius() -> f64 { 60.0 }

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetLootableCorpsesArgs {
    /// Low-32 GUID of the bot used as the search origin.
    pub bot_guid: i64,
    /// Search radius in yards (cap 150). Defaults to 60.
    #[serde(default = "default_corpses_radius")]
    pub radius: f64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObsGetLootableCorpsesWrapper { pub args: ObsGetLootableCorpsesArgs }

// ── bot.loot ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotLootArgs {
    /// Low-32 GUID of the externally-owned bot.
    pub bot_guid: i64,
    /// Packed creature uint64 (raw ObjectGuid) — from obs.get_lootable_corpses `guid` field.
    /// MUST be u64 (see BotAttackArgs.target_guid): exceeds i64::MAX, would fail to deserialize.
    pub target_guid: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BotLootWrapper { pub args: BotLootArgs }

// ── lfg.form_group ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LfgFormGroupArgs {
    /// Low GUID of the group leader; must also appear in members.
    pub leader_guid: i64,
    /// All members incl. leader, 1-5 entries.
    pub members: Vec<LfgFormGroupMember>,
    /// LFGDungeons.dbc id of the target dungeon.
    pub dungeon_id: i64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LfgFormGroupWrapper {
    pub args: LfgFormGroupArgs,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use schemars::schema_for;
    use serde_json::Value;

    use super::*;

    /// Helper: get the schema as a JSON Value and check that
    /// `properties.args` exists as an object.
    fn has_args_envelope(schema: &schemars::Schema) -> bool {
        let v: Value = serde_json::to_value(schema).expect("schema serializes");
        v.get("properties")
            .and_then(|p| p.get("args"))
            .map(|a| a.is_object())
            .unwrap_or(false)
    }

    #[test]
    fn all_63_wrappers_have_args_envelope() {
        // gm (7)
        assert!(has_args_envelope(&schema_for!(GmAdditemWrapper)),     "gm.additem");
        assert!(has_args_envelope(&schema_for!(GmEquipAllWrapper)),    "gm.equip_all");
        assert!(has_args_envelope(&schema_for!(GmTeleportWrapper)),    "gm.teleport");
        assert!(has_args_envelope(&schema_for!(GmSetLevelWrapper)),    "gm.set_level");
        assert!(has_args_envelope(&schema_for!(GmRunConsoleWrapper)),  "gm.run_console");
        assert!(has_args_envelope(&schema_for!(GmReadConsoleOutputWrapper)), "gm.read_console_output");
        assert!(has_args_envelope(&schema_for!(GmStripGearWrapper)),   "gm.strip_gear");
        // bot (17)
        assert!(has_args_envelope(&schema_for!(BotSetGoalWrapper)),      "bot.set_goal");
        assert!(has_args_envelope(&schema_for!(BotSetStrategyWrapper)),  "bot.set_strategy");
        assert!(has_args_envelope(&schema_for!(BotGetStrategiesWrapper)),"bot.get_strategies");
        assert!(has_args_envelope(&schema_for!(BotSendChatWrapper)),     "bot.send_chat");
        assert!(has_args_envelope(&schema_for!(BotFollowWrapper)),       "bot.follow");
        assert!(has_args_envelope(&schema_for!(BotStopWrapper)),         "bot.stop");
        assert!(has_args_envelope(&schema_for!(BotInviteToGroupWrapper)),"bot.invite_to_group");
        assert!(has_args_envelope(&schema_for!(BotAcceptInviteWrapper)), "bot.accept_invite");
        assert!(has_args_envelope(&schema_for!(BotLeaveGroupWrapper)),   "bot.leave_group");
        assert!(has_args_envelope(&schema_for!(BotSetRoleWrapper)),      "bot.set_role");
        assert!(has_args_envelope(&schema_for!(BotQueueForDungeonWrapper)),"bot.queue_for_dungeon");
        assert!(has_args_envelope(&schema_for!(BotEnterInstanceWrapper)),"bot.enter_instance");
        // M3 #9 riders — quest verbs + move (5)
        assert!(has_args_envelope(&schema_for!(BotDismountWrapper)),       "bot.dismount");
        assert!(has_args_envelope(&schema_for!(BotAcceptQuestWrapper)),    "bot.accept_quest");
        assert!(has_args_envelope(&schema_for!(BotTurninQuestWrapper)),    "bot.turnin_quest");
        assert!(has_args_envelope(&schema_for!(BotUseItemWrapper)),        "bot.use_item");
        assert!(has_args_envelope(&schema_for!(BotInteractObjectWrapper)), "bot.interact_object");
        // obs (17)
        assert!(has_args_envelope(&schema_for!(ObsPingWrapper)),           "obs.ping");
        assert!(has_args_envelope(&schema_for!(ObsGetStateWrapper)),       "obs.get_state");
        assert!(has_args_envelope(&schema_for!(ObsGetAurasWrapper)),       "obs.get_auras");
        assert!(has_args_envelope(&schema_for!(ObsGetInventoryWrapper)),   "obs.get_inventory");
        assert!(has_args_envelope(&schema_for!(ObsGetCombatLogWrapper)),   "obs.get_combat_log");
        assert!(has_args_envelope(&schema_for!(ObsGetQuestLogWrapper)),    "obs.get_quest_log");
        assert!(has_args_envelope(&schema_for!(ObsGetXpWrapper)),          "obs.get_xp");
        assert!(has_args_envelope(&schema_for!(ObsListPlayersWrapper)),    "obs.list_players");
        assert!(has_args_envelope(&schema_for!(ObsListBotPopulationWrapper)),"obs.list_bot_population");
        assert!(has_args_envelope(&schema_for!(ObsGetRpgStatusWrapper)),   "obs.get_rpg_status");
        assert!(has_args_envelope(&schema_for!(ObsGetMoneyWrapper)),       "obs.get_money");
        assert!(has_args_envelope(&schema_for!(ObsGetPositionWrapper)),    "obs.get_position");
        assert!(has_args_envelope(&schema_for!(ObsGetGroupWrapper)),       "obs.get_group");
        assert!(has_args_envelope(&schema_for!(ObsGetTalentsWrapper)),     "obs.get_talents");
        assert!(has_args_envelope(&schema_for!(ObsGameEventsWrapper)),     "obs.game_events");
        assert!(has_args_envelope(&schema_for!(ObsQueryDbWrapper)),        "obs.query_db");
        assert!(has_args_envelope(&schema_for!(ObsLfgPendingWrapper)),     "obs.lfg_pending");
        // event (2)
        assert!(has_args_envelope(&schema_for!(EventStartWrapper)), "event.start");
        assert!(has_args_envelope(&schema_for!(EventStopWrapper)),  "event.stop");
        // memory (7)
        assert!(has_args_envelope(&schema_for!(MemoryWriteWrapper)),  "memory.write");
        assert!(has_args_envelope(&schema_for!(MemoryReadWrapper)),   "memory.read");
        assert!(has_args_envelope(&schema_for!(MemoryRecallWrapper)), "memory.recall");
        assert!(has_args_envelope(&schema_for!(MemorySearchWrapper)), "memory.search");
        assert!(has_args_envelope(&schema_for!(MemoryListWrapper)),   "memory.list");
        assert!(has_args_envelope(&schema_for!(MemoryUpdateWrapper)), "memory.update");
        assert!(has_args_envelope(&schema_for!(MemoryDeleteWrapper)), "memory.delete");
        // nav (1)
        assert!(has_args_envelope(&schema_for!(NavFindPathWrapper)), "nav.find_path");
        // bot.move_path (1) — nested Vec<struct>; Vec<PathPointArg> precedent; + optional run
        assert!(has_args_envelope(&schema_for!(BotMovePathWrapper)), "bot.move_path");
        // bot.set_ai_enabled (1) — first bare #[serde(default)] Option<bool> (absent → None)
        assert!(has_args_envelope(&schema_for!(BotSetAiEnabledWrapper)), "bot.set_ai_enabled");
        // lfg (2)
        assert!(has_args_envelope(&schema_for!(LfgFormGroupWrapper)), "lfg.form_group");
        assert!(has_args_envelope(&schema_for!(LfgCancelWrapper)),    "lfg.cancel");
        // M1-combat-loot (4)
        assert!(has_args_envelope(&schema_for!(BotAttackWrapper)),               "bot.attack");
        assert!(has_args_envelope(&schema_for!(ObsGetNearbyHostilesWrapper)),    "obs.get_nearby_hostiles");
        assert!(has_args_envelope(&schema_for!(ObsGetLootableCorpsesWrapper)),   "obs.get_lootable_corpses");
        assert!(has_args_envelope(&schema_for!(BotLootWrapper)),                 "bot.loot");
        // M2 slice 2.2 — cast + economy verb batch (4)
        assert!(has_args_envelope(&schema_for!(BotCastSpellWrapper)),  "bot.cast_spell");
        assert!(has_args_envelope(&schema_for!(BotVendorSellWrapper)), "bot.vendor_sell");
        assert!(has_args_envelope(&schema_for!(BotRepairWrapper)),     "bot.repair");
        assert!(has_args_envelope(&schema_for!(BotMailWrapper)),       "bot.mail");
    }

    // ── MCP default-fill parity tests ─────────────────────────────────────────

    /// Python: `Optional[bool] = Field(False)` — absent field → `Some(false)`.
    #[test]
    fn gm_strip_gear_defaults_destroy_if_full_and_clear_bag_to_false() {
        let w: GmStripGearWrapper =
            serde_json::from_str(r#"{"args":{"target_guid":1}}"#).unwrap();
        assert_eq!(w.args.destroy_if_full, Some(false), "destroy_if_full must default to Some(false)");
        assert_eq!(w.args.clear_bag, Some(false), "clear_bag must default to Some(false)");
    }

    /// Python: `Optional[bool] = Field(False)` — absent field → `Some(false)`.
    #[test]
    fn bot_stop_defaults_clear_combat_to_false() {
        let w: BotStopWrapper =
            serde_json::from_str(r#"{"args":{"bot_guid":1}}"#).unwrap();
        assert_eq!(w.args.clear_combat, Some(false), "clear_combat must default to Some(false)");
    }

    /// Python: `dict = Field(default_factory=dict)` — absent field → `{}`.
    #[test]
    fn obs_query_db_defaults_params_to_empty_object() {
        let w: ObsQueryDbWrapper =
            serde_json::from_str(r#"{"args":{"template_name":"x"}}"#).unwrap();
        assert_eq!(
            w.args.params,
            serde_json::json!({}),
            "params must default to empty object"
        );
    }

    /// bot.vendor_sell: absent max_quality → 0 (grey-only). Load-bearing — selling
    /// is destructive, so the default quality ceiling must stay pinned at poor/grey.
    #[test]
    fn bot_vendor_sell_defaults_max_quality_to_grey_only() {
        let w: BotVendorSellWrapper =
            serde_json::from_str(r#"{"args":{"bot_guid":1,"vendor_spawn_id":2}}"#).unwrap();
        assert_eq!(w.args.max_quality, 0, "max_quality must default to 0 (grey only)");
    }

    // ── M3 #9 riders — schema default/parity tests ───────────────────────────

    /// bot.move_path: run field is optional; absent → None (walk).
    #[test]
    fn bot_move_path_run_defaults_to_none() {
        let w: BotMovePathWrapper = serde_json::from_str(
            r#"{"args":{"bot_guid":1173,"points":[{"x":1.0,"y":2.0,"z":3.0},{"x":4.0,"y":5.0,"z":6.0}]}}"#,
        ).unwrap();
        assert_eq!(w.args.run, None, "absent run must be None (walk)");
        // explicit false also works
        let w2: BotMovePathWrapper = serde_json::from_str(
            r#"{"args":{"bot_guid":1173,"points":[{"x":1.0,"y":2.0,"z":3.0},{"x":4.0,"y":5.0,"z":6.0}],"run":false}}"#,
        ).unwrap();
        assert_eq!(w2.args.run, Some(false));
    }

    /// bot.dismount: bot_guid required; no optional fields.
    #[test]
    fn bot_dismount_requires_bot_guid() {
        let w: BotDismountWrapper =
            serde_json::from_str(r#"{"args":{"bot_guid":1194}}"#).unwrap();
        assert_eq!(w.args.bot_guid, 1194);
        let err = serde_json::from_str::<BotDismountWrapper>(r#"{"args":{}}"#);
        assert!(err.is_err(), "bot_guid is required");
    }

    /// bot.accept_quest: quest_giver_guid is u64; packed GUIDs > i64::MAX must work.
    #[test]
    fn bot_accept_quest_giver_guid_accepts_packed_u64() {
        let packed: u64 = 0xF130_0000_0000_0005;
        assert!(packed > i64::MAX as u64, "test value must exceed i64::MAX");
        let body = format!(r#"{{"args":{{"bot_guid":1173,"quest_id":26,"quest_giver_guid":{packed}}}}}"#);
        let w: BotAcceptQuestWrapper = serde_json::from_str(&body)
            .expect("quest_giver_guid must accept u64 > i64::MAX");
        assert_eq!(w.args.quest_giver_guid, packed);
    }

    /// bot.turnin_quest: quest_giver_guid is u64; reward_choice is optional (absent → None).
    #[test]
    fn bot_turnin_quest_optional_reward_choice() {
        let w: BotTurninQuestWrapper = serde_json::from_str(
            r#"{"args":{"bot_guid":1173,"quest_id":26,"quest_giver_guid":12345}}"#,
        ).unwrap();
        assert_eq!(w.args.reward_choice, None, "absent reward_choice must be None");
        let w2: BotTurninQuestWrapper = serde_json::from_str(
            r#"{"args":{"bot_guid":1173,"quest_id":26,"quest_giver_guid":12345,"reward_choice":1}}"#,
        ).unwrap();
        assert_eq!(w2.args.reward_choice, Some(1));
    }

    /// bot.use_item: target_guid is optional and must accept packed u64 > i64::MAX.
    #[test]
    fn bot_use_item_optional_target_guid() {
        // absent → None (self-use)
        let w: BotUseItemWrapper = serde_json::from_str(
            r#"{"args":{"bot_guid":1173,"item_entry":4540}}"#,
        ).unwrap();
        assert_eq!(w.args.target_guid, None, "absent target_guid must be None (self-use)");
        // packed u64 > i64::MAX
        let packed: u64 = 0xF130_0000_0000_0007;
        let body = format!(r#"{{"args":{{"bot_guid":1173,"item_entry":4540,"target_guid":{packed}}}}}"#);
        let w2: BotUseItemWrapper = serde_json::from_str(&body)
            .expect("target_guid must accept u64 > i64::MAX");
        assert_eq!(w2.args.target_guid, Some(packed));
    }

    /// bot.interact_object: all three optional fields absent → None.
    #[test]
    fn bot_interact_object_all_optional_absent() {
        let w: BotInteractObjectWrapper = serde_json::from_str(
            r#"{"args":{"bot_guid":1173}}"#,
        ).unwrap();
        assert_eq!(w.args.object_guid, None);
        assert_eq!(w.args.object_entry, None);
        assert_eq!(w.args.search_range, None);
    }

    /// bot.interact_object: object_guid is u64 and accepts packed GUIDs.
    #[test]
    fn bot_interact_object_guid_accepts_packed_u64() {
        let packed: u64 = 0xF150_0000_0000_0001;
        let body = format!(r#"{{"args":{{"bot_guid":1173,"object_guid":{packed}}}}}"#);
        let w: BotInteractObjectWrapper = serde_json::from_str(&body)
            .expect("object_guid must accept u64 > i64::MAX");
        assert_eq!(w.args.object_guid, Some(packed));
    }

    /// bot.cast_spell: absent target_guid → None (self-cast); bot.mail: absent
    /// item_guids/copper → empty/zero.
    #[test]
    fn cast_spell_and_mail_optional_fields_default() {
        let w: BotCastSpellWrapper =
            serde_json::from_str(r#"{"args":{"bot_guid":1,"spell_id":116}}"#).unwrap();
        assert_eq!(w.args.target_guid, None, "absent target_guid must mean self-cast");
        let m: BotMailWrapper =
            serde_json::from_str(r#"{"args":{"bot_guid":1,"recipient":"Mule"}}"#).unwrap();
        assert!(m.args.item_guids.is_empty(), "item_guids must default to empty");
        assert_eq!(m.args.copper, 0, "copper must default to 0");
    }

    /// Regression: a packed creature GUID (HighGuid bits set) exceeds i64::MAX. target_guid
    /// MUST be u64 — an i64 field would fail to deserialize the value and the daemon would
    /// reject every bot.attack / bot.loot for a real creature before forwarding to AC.
    #[test]
    fn combat_loot_target_guid_accepts_packed_uint64_over_i64_max() {
        let packed: u64 = 0xF130_0000_0000_0001; // HighGuid::Unit-style raw GUID
        assert!(packed > i64::MAX as u64, "test value must exceed i64::MAX");
        let body = format!(r#"{{"args":{{"bot_guid":1003,"target_guid":{packed}}}}}"#);
        let a: BotAttackWrapper = serde_json::from_str(&body)
            .expect("bot.attack target_guid must accept u64 > i64::MAX");
        assert_eq!(a.args.target_guid, packed);
        let l: BotLootWrapper = serde_json::from_str(&body)
            .expect("bot.loot target_guid must accept u64 > i64::MAX");
        assert_eq!(l.args.target_guid, packed);
    }

    /// Enum-bearing wrappers must use $defs not $definitions (schemars 1.x).
    #[test]
    fn enum_schemas_use_defs_not_definitions() {
        let wrappers_with_enums: &[(&str, Value)] = &[
            ("bot.set_strategy",  serde_json::to_value(schema_for!(BotSetStrategyWrapper)).unwrap()),
            ("bot.send_chat",     serde_json::to_value(schema_for!(BotSendChatWrapper)).unwrap()),
            ("bot.set_role",      serde_json::to_value(schema_for!(BotSetRoleWrapper)).unwrap()),
            ("bot.enter_instance",serde_json::to_value(schema_for!(BotEnterInstanceWrapper)).unwrap()),
            ("memory.write",      serde_json::to_value(schema_for!(MemoryWriteWrapper)).unwrap()),
            ("lfg.form_group",    serde_json::to_value(schema_for!(LfgFormGroupWrapper)).unwrap()),
        ];
        for (name, schema_val) in wrappers_with_enums {
            let s = serde_json::to_string(schema_val).unwrap();
            assert!(
                !s.contains("#/definitions/"),
                "{name}: schema must not use #/definitions/ — found it in: {s}"
            );
            assert!(
                s.contains("$defs") || !s.contains("#/"),
                "{name}: schema should use $defs for refs, schema was: {s}"
            );
        }
    }
}
