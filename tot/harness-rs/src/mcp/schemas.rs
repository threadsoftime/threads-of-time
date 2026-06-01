//! Per-tool argument schemas for the MCP adapter.
//!
//! Port of `harness_daemon/tool_schemas.py` — one `XxxArgs` inner struct plus
//! one `XxxWrapper { pub args: XxxArgs }` per tool. Both carry
//! `#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]`.
//!
//! `Serialize` is required by the MCP handler to call `serde_json::to_value(&w.args)`
//! (the exclude_none path in `forward()`).
//!
//! All 46 tools from `TOOL_SCHEMAS` are represented here.

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
    fn all_46_wrappers_have_args_envelope() {
        // gm (7)
        assert!(has_args_envelope(&schema_for!(GmAdditemWrapper)),     "gm.additem");
        assert!(has_args_envelope(&schema_for!(GmEquipAllWrapper)),    "gm.equip_all");
        assert!(has_args_envelope(&schema_for!(GmTeleportWrapper)),    "gm.teleport");
        assert!(has_args_envelope(&schema_for!(GmSetLevelWrapper)),    "gm.set_level");
        assert!(has_args_envelope(&schema_for!(GmRunConsoleWrapper)),  "gm.run_console");
        assert!(has_args_envelope(&schema_for!(GmReadConsoleOutputWrapper)), "gm.read_console_output");
        assert!(has_args_envelope(&schema_for!(GmStripGearWrapper)),   "gm.strip_gear");
        // bot (12)
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
        // lfg (1)
        assert!(has_args_envelope(&schema_for!(LfgFormGroupWrapper)), "lfg.form_group");
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
