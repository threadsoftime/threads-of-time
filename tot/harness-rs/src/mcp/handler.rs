//! MCP ServerHandler with all 46 V1 tools.
//!
//! Port of `harness_daemon/mcp_server.py:build_mcp_server`.
//!
//! Pattern:
//! - `#[tool_router]` on the inherent `impl HarnessMcp` block wires each
//!   `#[tool]` method into a compile-time router.
//! - `#[tool_handler(name="tot-harness", version="0.2.0")]` on
//!   `impl ServerHandler for HarnessMcp` sets `serverInfo`.
//! - Every tool is a 3-line forward to `self.forward(...)`, DRY via the
//!   `forward_tool!` macro.
//!
//! Description strings are VERBATIM from `TOOL_SCHEMAS` in
//! `harness_daemon/tool_schemas.py` (the one-liner per tool) so the brain
//! receives the same hints as from the Python daemon.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::Extension, wrapper::Parameters},
    model::{CallToolResult, Content},
    tool, tool_handler, tool_router,
};
use serde_json::Value;
use uuid::Uuid;

use crate::audit::AuditEvent;
use crate::auth::AuthResult;
use crate::config::TokenRecord;
use crate::dispatch::dispatch_tool;
use crate::mcp::schemas;
use crate::rest::SharedState;

// ── HarnessMcp ────────────────────────────────────────────────────────────────

/// MCP ServerHandler backed by the shared `AppState`.
///
/// Holds a compile-time `ToolRouter<Self>` produced by `#[tool_router]`.
#[derive(Clone)]
pub struct HarnessMcp {
    state:       SharedState,
    tool_router: ToolRouter<Self>,
}

impl HarnessMcp {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

// ── Shared forward helper ─────────────────────────────────────────────────────

/// Strips every top-level key whose value is `Value::Null`.
///
/// Mirrors Python `args.model_dump(exclude_none=True)` (mcp_server.py:119).
///
/// MUST NOT recurse into nested object values — parity: pydantic
/// `exclude_none` removes None fields only at the model's OWN level, not
/// inside opaque `dict`-typed fields like `metadata`/`goal`/`time_filter`/`params`.
pub fn strip_top_level_nulls(obj: Value) -> Value {
    match obj {
        Value::Object(mut map) => {
            map.retain(|_, v| !v.is_null());
            Value::Object(map)
        }
        other => other,
    }
}

fn unix_now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        // system clock before Unix epoch is impossible in production; 0.0 is a safe fallback
        .unwrap_or(0.0)
}

fn gen_mcp_request_id() -> String {
    let id = Uuid::new_v4().simple().to_string();
    format!("mcp_{}", &id[..12])
}

impl HarnessMcp {
    /// Shared dispatch path for all 46 tool methods.
    ///
    /// Steps (mirror mcp_server.py:111-150):
    /// 1. Resolve TokenRecord from Parts extensions → build AuthResult.
    /// 2. Strip top-level null fields (exclude_none parity).
    /// 3. Generate request_id, record t0.
    /// 4. Dispatch.
    /// 5. Audit with transport="mcp".
    /// 6. Return success text wrapping the JSON body.
    async fn forward(
        &self,
        tool_name: &str,
        wrapper_args: Value,
        parts: &http::request::Parts,
    ) -> CallToolResult {
        // ── 1. Auth ───────────────────────────────────────────────────────────
        let record: &TokenRecord = match parts.extensions.get::<TokenRecord>() {
            Some(r) => r,
            None => {
                // Should not happen (auth layer rejected before reaching here),
                // but mirror mcp_server.py:116-117.
                return CallToolResult::success(vec![Content::text(
                    r#"{"ok":false,"error":"unauthorized"}"#,
                )]);
            }
        };
        let auth = AuthResult {
            identity:      record.identity.clone(),
            scope:         record.scope.clone(),
            bound_to_guid: record.bound_to_guid,
            augmented:     record.augmented,
        };

        // ── 2. strip top-level nulls (exclude_none parity) ────────────────────
        let stripped = strip_top_level_nulls(wrapper_args);

        // ── 3. request_id, t0 ────────────────────────────────────────────────
        let request_id = gen_mcp_request_id();
        let t0 = Instant::now();

        // ── 4. dispatch ───────────────────────────────────────────────────────
        let outcome = dispatch_tool(
            tool_name,
            &stripped,
            &auth,
            &request_id,
            &self.state.registry,
            &self.state.ac_client,
            self.state.db_client.as_ref(),
        )
        .await;

        let latency_ms = t0.elapsed().as_millis() as u64;

        // ── 5. audit (transport = "mcp") ──────────────────────────────────────
        // `request_id` and `stripped` are moved here — neither is used after
        // dispatch_tool returned (which took them by reference), so no clone needed.
        let _ = self.state.audit.write(&AuditEvent {
            ts:            unix_now_f64(),
            request_id,
            identity:      auth.identity.clone(),
            tool:          tool_name.to_string(),
            args_body:     stripped,
            outcome:       outcome.audit_outcome.clone(),
            status:        outcome.status,
            latency_ms,
            ac_latency_ms: outcome.ac_latency_ms,
            error_detail:  outcome.error_detail.clone(),
            transport:     "mcp".to_string(),
        });

        // ── 6. return success text (app-level {ok:false} is still isError=false) ─
        let body_str = match serde_json::to_string(&outcome.body) {
            Ok(s)  => s,
            Err(_) => r#"{"ok":false,"error":"serialize_error"}"#.to_string(),
        };
        CallToolResult::success(vec![Content::text(body_str)])
    }
}

// ── Declarative macro: 3-line tool forward ────────────────────────────────────
//
// Expands:
//   forward_tool!(fn_name, "tool.name", "description…", WrapperType)
// into:
//   #[tool(name = "tool.name", description = "description…")]
//   async fn fn_name(&self, Parameters(w): Parameters<WrapperType>,
//                    Extension(parts): Extension<http::request::Parts>) -> CallToolResult
//   { self.forward("tool.name", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await }
//
// The `serde_json::to_value(&w.args)` call fills in all `#[serde(default)]`
// values that are present-but-None into `Value::Null` — then `strip_top_level_nulls`
// removes them, matching Python's `model_dump(exclude_none=True)`.

macro_rules! forward_tool {
    ($fn_name:ident, $tool_name:expr, $desc:expr, $wrapper:ty) => {
        #[tool(name = $tool_name, description = $desc)]
        async fn $fn_name(
            &self,
            Parameters(w): Parameters<$wrapper>,
            Extension(parts): Extension<http::request::Parts>,
        ) -> CallToolResult {
            self.forward(
                $tool_name,
                serde_json::to_value(&w.args).unwrap_or_default(),
                &parts,
            )
            .await
        }
    };
}

// ── 46-tool #[tool_router] impl ───────────────────────────────────────────────

#[tool_router]
impl HarnessMcp {
    // ── gm.* (7) ──────────────────────────────────────────────────────────────
    forward_tool!(gm_additem,            "gm.additem",             "Add `count` of `item_id` to a player's inventory.",                      schemas::GmAdditemWrapper);
    forward_tool!(gm_equip_all,          "gm.equip_all",           "Equip the best in-bag gear for every slot.",                             schemas::GmEquipAllWrapper);
    forward_tool!(gm_teleport,           "gm.teleport",            "Teleport the player to (map, x, y, z, orientation).",                    schemas::GmTeleportWrapper);
    forward_tool!(gm_set_level,          "gm.set_level",           "Set the player's level (1..80 WotLK).",                                  schemas::GmSetLevelWrapper);
    forward_tool!(gm_run_console,        "gm.run_console",         "Run an allowlisted server console command.",                             schemas::GmRunConsoleWrapper);
    forward_tool!(gm_read_console_output,"gm.read_console_output", "Read buffered console output from the last invocation.",                 schemas::GmReadConsoleOutputWrapper);
    forward_tool!(gm_strip_gear,         "gm.strip_gear",          "Unequip every item from the player to their bags.",                      schemas::GmStripGearWrapper);

    // ── bot.* (12) ────────────────────────────────────────────────────────────
    forward_tool!(bot_set_goal,          "bot.set_goal",           "Set a playerbot's next RPG goal.",                                       schemas::BotSetGoalWrapper);
    forward_tool!(bot_set_strategy,      "bot.set_strategy",       "Add/remove/toggle strategies on a bot's engine buckets.",                schemas::BotSetStrategyWrapper);
    forward_tool!(bot_get_strategies,    "bot.get_strategies",     "Current active + available strategy names per bucket.",                  schemas::BotGetStrategiesWrapper);
    forward_tool!(bot_send_chat,         "bot.send_chat",          "Have a bot say/yell/whisper or speak in party/raid/guild.",              schemas::BotSendChatWrapper);
    forward_tool!(bot_follow,            "bot.follow",             "Set master + apply +follow strategy (durable).",                         schemas::BotFollowWrapper);
    forward_tool!(bot_stop,              "bot.stop",               "Apply +stay,-follow strategy (optionally also CombatStop).",             schemas::BotStopWrapper);
    forward_tool!(bot_invite_to_group,   "bot.invite_to_group",    "Invite a player into the bot's group (or create one).",                  schemas::BotInviteToGroupWrapper);
    forward_tool!(bot_accept_invite,     "bot.accept_invite",      "Accept a pending group invitation.",                                     schemas::BotAcceptInviteWrapper);
    forward_tool!(bot_leave_group,       "bot.leave_group",        "Leave (or disband if leader) the bot's current group.",                  schemas::BotLeaveGroupWrapper);
    forward_tool!(bot_set_role,          "bot.set_role",           "Set LFG role or transfer group leadership.",                             schemas::BotSetRoleWrapper);
    forward_tool!(bot_queue_for_dungeon, "bot.queue_for_dungeon",  "Queue the bot for a dungeon via LFG.",                                   schemas::BotQueueForDungeonWrapper);
    forward_tool!(bot_enter_instance,    "bot.enter_instance",     "Teleport bot into a dungeon (lfg teleport or direct).",                  schemas::BotEnterInstanceWrapper);

    // ── obs.* (17) ────────────────────────────────────────────────────────────
    forward_tool!(obs_ping,              "obs.ping",               "Health check — returns {pong:true}.",                                    schemas::ObsPingWrapper);
    forward_tool!(obs_get_state,         "obs.get_state",          "Player state: level, hp/mana, class, race, position, combat.",           schemas::ObsGetStateWrapper);
    forward_tool!(obs_get_auras,         "obs.get_auras",          "Active auras (buffs/debuffs) on the player.",                            schemas::ObsGetAurasWrapper);
    forward_tool!(obs_get_inventory,     "obs.get_inventory",      "Full inventory: bags, equipped, bank stub.",                             schemas::ObsGetInventoryWrapper);
    forward_tool!(obs_get_combat_log,    "obs.get_combat_log",     "Recent combat-log events for the player.",                               schemas::ObsGetCombatLogWrapper);
    forward_tool!(obs_get_quest_log,     "obs.get_quest_log",      "Quest log: open quests + objectives + status.",                          schemas::ObsGetQuestLogWrapper);
    forward_tool!(obs_get_xp,            "obs.get_xp",             "Current XP, next-level threshold, rested XP, %.",                       schemas::ObsGetXpWrapper);
    forward_tool!(obs_list_players,      "obs.list_players",       "List all online players (bots + humans) with GUIDs and basic state.",    schemas::ObsListPlayersWrapper);
    forward_tool!(obs_list_bot_population,"obs.list_bot_population","World-wide bot population snapshot: count, level distribution, zone spread.", schemas::ObsListBotPopulationWrapper);
    forward_tool!(obs_get_rpg_status,    "obs.get_rpg_status",     "Playerbot NewRpgInfo: current goal + description.",                      schemas::ObsGetRpgStatusWrapper);
    forward_tool!(obs_get_money,         "obs.get_money",          "Gold/silver/copper balance.",                                            schemas::ObsGetMoneyWrapper);
    forward_tool!(obs_get_position,      "obs.get_position",       "Map/zone/area IDs + names + (x,y,z,o).",                                schemas::ObsGetPositionWrapper);
    forward_tool!(obs_get_group,         "obs.get_group",          "Party/raid roster, HP/mana%, distances.",                               schemas::ObsGetGroupWrapper);
    forward_tool!(obs_get_talents,       "obs.get_talents",        "Active-spec talents: flat list with tab/row/col/rank.",                  schemas::ObsGetTalentsWrapper);
    forward_tool!(obs_game_events,       "obs.game_events",        "Full game-event schedule: active set, resolved start/end/next per event, and raw sHolidaysStore dump.", schemas::ObsGameEventsWrapper);
    forward_tool!(obs_query_db,          "obs.query_db",           "Run an allowlisted MySQL template against the auth/char DBs.",           schemas::ObsQueryDbWrapper);
    forward_tool!(obs_lfg_pending,       "obs.lfg_pending",        "Drain pending real-player + bot LFG join intents recorded by the veto hook / bot-queue.", schemas::ObsLfgPendingWrapper);

    // ── event.* (2) ───────────────────────────────────────────────────────────
    forward_tool!(event_start,           "event.start",            "Start a game event by id (slice-driven scheduler enactment primitive).", schemas::EventStartWrapper);
    forward_tool!(event_stop,            "event.stop",             "Stop a game event by id (slice-driven scheduler enactment primitive).",  schemas::EventStopWrapper);

    // ── memory.* (7) ─────────────────────────────────────────────────────────
    forward_tool!(memory_write,          "memory.write",           "Write a new episode to a bot's episodic memory store.",                  schemas::MemoryWriteWrapper);
    forward_tool!(memory_read,           "memory.read",            "Fetch a single episode by primary-key ID.",                              schemas::MemoryReadWrapper);
    forward_tool!(memory_recall,         "memory.recall",          "Hybrid-ranked recall: semantic + recency + salience + MMR.",             schemas::MemoryRecallWrapper);
    forward_tool!(memory_search,         "memory.search",          "Pure ANN search by text or pre-computed embedding vector.",              schemas::MemorySearchWrapper);
    forward_tool!(memory_list,           "memory.list",            "Paginated episode listing with optional type/entity/time filters.",      schemas::MemoryListWrapper);
    forward_tool!(memory_update,         "memory.update",          "Patch content, salience, or metadata on an existing episode.",           schemas::MemoryUpdateWrapper);
    forward_tool!(memory_delete,         "memory.delete",          "Hard-delete an episode (cascades to entities + embeddings).",            schemas::MemoryDeleteWrapper);

    // ── lfg.* (1) ────────────────────────────────────────────────────────────
    forward_tool!(lfg_form_group,        "lfg.form_group",         "Force-create a server-side LFG group and teleport all members into the dungeon.", schemas::LfgFormGroupWrapper);
}

// `#[tool_handler]` sets serverInfo.name and serverInfo.version in the
// `initialize` response (spike finding (e): name="tot-harness", version="0.2.0").
#[tool_handler(name = "tot-harness", version = "0.2.0")]
impl ServerHandler for HarnessMcp {}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::strip_top_level_nulls;

    // ── strip_top_level_nulls ─────────────────────────────────────────────────

    #[test]
    fn removes_top_level_nulls() {
        let input = json!({
            "a": null,
            "b": 1,
            "c": "hello",
        });
        let out = strip_top_level_nulls(input);
        assert!(out.get("a").is_none(), "null key 'a' must be removed");
        assert_eq!(out["b"], 1);
        assert_eq!(out["c"], "hello");
    }

    #[test]
    fn does_not_recurse_into_nested_objects() {
        // Python parity: exclude_none does NOT recurse into opaque dict fields.
        let input = json!({
            "a": null,
            "b": 1,
            "c": {
                "d": null,
                "e": 2,
            },
        });
        let out = strip_top_level_nulls(input);
        // top-level null removed
        assert!(out.get("a").is_none());
        // top-level non-null kept
        assert_eq!(out["b"], 1);
        // nested object preserved INTACT — including its own null keys
        let c = &out["c"];
        assert_eq!(c["d"], json!(null), "nested null must NOT be removed");
        assert_eq!(c["e"], 2);
    }

    #[test]
    fn non_object_value_passed_through() {
        let arr = json!([1, null, 3]);
        let out = strip_top_level_nulls(arr.clone());
        assert_eq!(out, arr, "non-object values must be returned unchanged");

        let s = json!("hello");
        assert_eq!(strip_top_level_nulls(s.clone()), s);
    }

    #[test]
    fn empty_object_remains_empty() {
        let out = strip_top_level_nulls(json!({}));
        assert_eq!(out, json!({}));
    }

    #[test]
    fn all_non_null_object_unchanged() {
        let input = json!({ "x": 1, "y": "two", "z": false });
        let out = strip_top_level_nulls(input.clone());
        assert_eq!(out, input);
    }
}
