//! MCP ServerHandler with all 47 V1 tools.
//!
//! Port of `harness_daemon/mcp_server.py:build_mcp_server`.
//!
//! Pattern:
//! - `#[tool_router]` on the inherent `impl HarnessMcp` block wires each
//!   `#[tool]` method into a compile-time router.
//! - `#[tool_handler(name="tot-harness", version="0.2.0")]` on
//!   `impl ServerHandler for HarnessMcp` sets `serverInfo`.
//! - Every tool is a direct `#[tool]` fn that forwards to `self.forward(...)`.
//!
//! IMPORTANT: All 47 tool methods must be defined DIRECTLY in the
//! `#[tool_router] impl HarnessMcp` block with `#[tool(...)]` attributes on
//! each function. The `#[tool_router]` proc macro detects `#[tool]` attributes
//! in the token stream BEFORE macro_rules expansion — so `macro_rules!`
//! invocations inside the block would NOT be detected. Every tool method must
//! be written out explicitly.
//!
//! Description strings are VERBATIM from `TOOL_SCHEMAS` in
//! `harness_daemon/tool_schemas.py` (the one-liner per tool) so the brain
//! receives the same hints as from the Python daemon.

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::Extension, wrapper::Parameters},
    model::{CallToolResult, Content, ListToolsResult, PaginatedRequestParams},
    service::RequestContext,
    RoleServer,
    tool, tool_handler, tool_router,
};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::audit::AuditEvent;
use crate::auth::AuthResult;
use crate::config::TokenRecord;
use crate::dispatch::dispatch_tool;
use crate::mcp::schemas;
use crate::rest::SharedState;

// ── Nullable-schema transform ─────────────────────────────────────────────────

/// Convert schemars draft-2020-12 nullable form to pydantic-v2 `anyOf` form.
///
/// schemars 1.x emits `{"type": ["integer", "null"], ...}` for `Option<i64>`.
/// The brain client (`schema_builder.py:_render_arg`) passes `schema.get("type")`
/// directly to `_JSON_TO_PY_TYPE.get(...)` — a list value is unhashable and
/// crashes with `TypeError: unhashable type: 'list'` at startup.
///
/// This function walks the entire schema tree (recursively through `properties`,
/// `$defs`, `anyOf`, `allOf`, `oneOf`, `items`, etc.) and wherever it finds a
/// node whose `"type"` key is a JSON array (possibly containing `"null"`), it
/// rewrites that node as:
///
/// ```json
/// {
///   "anyOf": [{"type": "T1"}, {"type": "T2"}, {"type": "null"}],
///   "description": "...",   // outer-level metadata kept
///   "default": ...,         // outer-level metadata kept
///   "title": "...",         // outer-level metadata kept
/// }
/// ```
///
/// Keys that are structural type descriptors (`"type"`, `"format"`) are moved
/// into the first `anyOf` branch; keys that are documentation/defaults
/// (`"description"`, `"default"`, `"title"`) stay at the outer level,
/// mirroring pydantic v2 output.
///
/// The transform is applied at the SERVING BOUNDARY so the stored schemas are
/// not mutated — only the `list_tools` response is affected.
pub fn transform_nullable_types(v: Value) -> Value {
    match v {
        Value::Object(map) => transform_nullable_object(map),
        Value::Array(arr)  => Value::Array(arr.into_iter().map(transform_nullable_types).collect()),
        other              => other,
    }
}

/// Structural keys that belong INSIDE an `anyOf` branch (not at the outer level).
const STRUCTURAL_KEYS: &[&str] = &["format", "minimum", "maximum", "minLength", "maxLength",
                                   "pattern", "enum", "const", "items", "prefixItems",
                                   "properties", "required", "additionalProperties",
                                   "allOf", "anyOf", "oneOf", "not",
                                   "$ref", "$defs", "$schema"];

fn transform_nullable_object(mut map: Map<String, Value>) -> Value {
    // First recursively transform all nested values.
    for v in map.values_mut() {
        *v = transform_nullable_types(std::mem::replace(v, Value::Null));
    }

    // Now check if "type" is an array.
    let type_is_array = map
        .get("type")
        .map(|t| t.is_array())
        .unwrap_or(false);

    if !type_is_array {
        return Value::Object(map);
    }

    // Extract the type array.
    let type_arr: Vec<Value> = match map.remove("type") {
        Some(Value::Array(a)) => a,
        other => {
            // Shouldn't happen, but restore and return unchanged.
            if let Some(t) = other { map.insert("type".into(), t); }
            return Value::Object(map);
        }
    };

    // Build anyOf branches: one per type string in the array.
    // Each non-null type gets its own branch; structural sibling keys (format,
    // minimum, etc.) are pulled into the FIRST non-null branch only (same as
    // pydantic, which puts format annotations inside the typed branch).
    let non_null_types: Vec<&Value> = type_arr.iter().filter(|t| t != &&Value::String("null".into())).collect();
    let has_null = type_arr.iter().any(|t| t == &Value::String("null".into()));

    // Collect structural sibling keys to move into the first non-null branch.
    let mut first_branch_extra: Map<String, Value> = Map::new();
    for &key in STRUCTURAL_KEYS {
        if key == "anyOf" || key == "oneOf" || key == "allOf" {
            // These are already present in the map (transformed above) — leave them.
            continue;
        }
        if let Some(v) = map.remove(key) {
            first_branch_extra.insert(key.to_string(), v);
        }
    }

    let mut branches: Vec<Value> = Vec::new();
    let mut is_first = true;
    for type_val in &non_null_types {
        let mut branch: Map<String, Value> = Map::new();
        branch.insert("type".into(), (*type_val).clone());
        if is_first {
            branch.extend(first_branch_extra.clone());
            is_first = false;
        }
        branches.push(Value::Object(branch));
    }
    if has_null {
        branches.push(Value::Object({
            let mut m = Map::new();
            m.insert("type".into(), Value::String("null".into()));
            m
        }));
    }

    // If there's only one type (no null case, e.g. a plain non-nullable array type),
    // skip anyOf expansion — just reconstruct.
    if !has_null && non_null_types.len() == 1 {
        // Restore: single-type array → plain type string (shouldn't normally happen
        // with schemars, but handle gracefully).
        map.insert("type".into(), non_null_types[0].clone());
        for (k, v) in first_branch_extra {
            map.insert(k, v);
        }
        return Value::Object(map);
    }

    // Build the outer node: anyOf + outer-level metadata keys stay.
    map.insert("anyOf".into(), Value::Array(branches));
    // type was already removed; structural keys were moved to branch — done.
    Value::Object(map)
}

/// Apply `transform_nullable_types` to every tool in a `ListToolsResult`.
///
/// The `input_schema` on each `Tool` is an `Arc<JsonObject>`; we clone the map,
/// transform it, and re-wrap it.
fn transform_tools_schemas(mut result: ListToolsResult) -> ListToolsResult {
    result.tools = result.tools
        .into_iter()
        .map(|mut tool| {
            // Clone the JsonObject (serde_json::Map<String, Value>) into a Value::Object,
            // transform, then re-wrap as Arc<JsonObject>.
            let schema_val = Value::Object((*tool.input_schema).clone());
            let transformed = transform_nullable_types(schema_val);
            let new_map = match transformed {
                Value::Object(m) => m,
                _                 => Map::new(),
            };
            tool.input_schema = Arc::new(new_map);
            tool
        })
        .collect();
    result
}

// ── HarnessMcp ────────────────────────────────────────────────────────────────

/// MCP ServerHandler backed by the shared `AppState`.
///
/// Holds a compile-time `ToolRouter<Self>` produced by `#[tool_router]`.
#[derive(Clone)]
pub struct HarnessMcp {
    state:       SharedState,
    #[allow(dead_code)] // read by the #[tool_handler]-generated ServerHandler impl
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
        .unwrap_or(0.0)
}

fn gen_mcp_request_id() -> String {
    let id = Uuid::new_v4().simple().to_string();
    format!("mcp_{}", &id[..12])
}

impl HarnessMcp {
    /// Shared dispatch path for all 47 tool methods.
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

// ── 47-tool #[tool_router] impl ───────────────────────────────────────────────
//
// CRITICAL: Each tool method MUST be defined directly with `#[tool(...)]` on
// the function. Do NOT use macro_rules! invocations here — the `#[tool_router]`
// proc macro parses the token stream before macro_rules expansion and will NOT
// detect `#[tool]` attributes inside unexpanded macro invocations.

#[tool_router]
impl HarnessMcp {
    // ── gm.* (7) ──────────────────────────────────────────────────────────────

    #[tool(name = "gm.additem", description = "Add `count` of `item_id` to a player's inventory.")]
    async fn gm_additem(&self, Parameters(w): Parameters<schemas::GmAdditemWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("gm.additem", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "gm.equip_all", description = "Equip the best in-bag gear for every slot.")]
    async fn gm_equip_all(&self, Parameters(w): Parameters<schemas::GmEquipAllWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("gm.equip_all", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "gm.teleport", description = "Teleport the player to (map, x, y, z, orientation).")]
    async fn gm_teleport(&self, Parameters(w): Parameters<schemas::GmTeleportWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("gm.teleport", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "gm.set_level", description = "Set the player's level (1..80 WotLK).")]
    async fn gm_set_level(&self, Parameters(w): Parameters<schemas::GmSetLevelWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("gm.set_level", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "gm.run_console", description = "Run an allowlisted server console command.")]
    async fn gm_run_console(&self, Parameters(w): Parameters<schemas::GmRunConsoleWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("gm.run_console", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "gm.read_console_output", description = "Read buffered console output from the last invocation.")]
    async fn gm_read_console_output(&self, Parameters(w): Parameters<schemas::GmReadConsoleOutputWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("gm.read_console_output", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "gm.strip_gear", description = "Unequip every item from the player to their bags.")]
    async fn gm_strip_gear(&self, Parameters(w): Parameters<schemas::GmStripGearWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("gm.strip_gear", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    // ── bot.* (12) ────────────────────────────────────────────────────────────

    #[tool(name = "bot.set_goal", description = "Set a playerbot's next RPG goal.")]
    async fn bot_set_goal(&self, Parameters(w): Parameters<schemas::BotSetGoalWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.set_goal", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.set_strategy", description = "Add/remove/toggle strategies on a bot's engine buckets.")]
    async fn bot_set_strategy(&self, Parameters(w): Parameters<schemas::BotSetStrategyWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.set_strategy", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.get_strategies", description = "Current active + available strategy names per bucket.")]
    async fn bot_get_strategies(&self, Parameters(w): Parameters<schemas::BotGetStrategiesWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.get_strategies", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.send_chat", description = "Have a bot say/yell/whisper or speak in party/raid/guild.")]
    async fn bot_send_chat(&self, Parameters(w): Parameters<schemas::BotSendChatWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.send_chat", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.follow", description = "Set master + apply +follow strategy (durable).")]
    async fn bot_follow(&self, Parameters(w): Parameters<schemas::BotFollowWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.follow", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.stop", description = "Apply +stay,-follow strategy (optionally also CombatStop).")]
    async fn bot_stop(&self, Parameters(w): Parameters<schemas::BotStopWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.stop", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.invite_to_group", description = "Invite a player into the bot's group (or create one).")]
    async fn bot_invite_to_group(&self, Parameters(w): Parameters<schemas::BotInviteToGroupWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.invite_to_group", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.accept_invite", description = "Accept a pending group invitation.")]
    async fn bot_accept_invite(&self, Parameters(w): Parameters<schemas::BotAcceptInviteWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.accept_invite", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.leave_group", description = "Leave (or disband if leader) the bot's current group.")]
    async fn bot_leave_group(&self, Parameters(w): Parameters<schemas::BotLeaveGroupWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.leave_group", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.set_role", description = "Set LFG role or transfer group leadership.")]
    async fn bot_set_role(&self, Parameters(w): Parameters<schemas::BotSetRoleWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.set_role", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.queue_for_dungeon", description = "Queue the bot for a dungeon via LFG.")]
    async fn bot_queue_for_dungeon(&self, Parameters(w): Parameters<schemas::BotQueueForDungeonWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.queue_for_dungeon", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "bot.enter_instance", description = "Teleport bot into a dungeon (lfg teleport or direct).")]
    async fn bot_enter_instance(&self, Parameters(w): Parameters<schemas::BotEnterInstanceWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("bot.enter_instance", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    // ── obs.* (17) ────────────────────────────────────────────────────────────

    #[tool(name = "obs.ping", description = "Health check — returns {pong:true}.")]
    async fn obs_ping(&self, Parameters(w): Parameters<schemas::ObsPingWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.ping", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_state", description = "Player state: level, hp/mana, class, race, position, combat.")]
    async fn obs_get_state(&self, Parameters(w): Parameters<schemas::ObsGetStateWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_state", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_auras", description = "Active auras (buffs/debuffs) on the player.")]
    async fn obs_get_auras(&self, Parameters(w): Parameters<schemas::ObsGetAurasWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_auras", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_inventory", description = "Full inventory: bags, equipped, bank stub.")]
    async fn obs_get_inventory(&self, Parameters(w): Parameters<schemas::ObsGetInventoryWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_inventory", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_combat_log", description = "Recent combat-log events for the player.")]
    async fn obs_get_combat_log(&self, Parameters(w): Parameters<schemas::ObsGetCombatLogWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_combat_log", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_quest_log", description = "Quest log: open quests + objectives + status.")]
    async fn obs_get_quest_log(&self, Parameters(w): Parameters<schemas::ObsGetQuestLogWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_quest_log", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_xp", description = "Current XP, next-level threshold, rested XP, %.")]
    async fn obs_get_xp(&self, Parameters(w): Parameters<schemas::ObsGetXpWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_xp", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.list_players", description = "List all online players (bots + humans) with GUIDs and basic state.")]
    async fn obs_list_players(&self, Parameters(w): Parameters<schemas::ObsListPlayersWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.list_players", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.list_bot_population", description = "World-wide bot population snapshot: count, level distribution, zone spread.")]
    async fn obs_list_bot_population(&self, Parameters(w): Parameters<schemas::ObsListBotPopulationWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.list_bot_population", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_rpg_status", description = "Playerbot NewRpgInfo: current goal + description.")]
    async fn obs_get_rpg_status(&self, Parameters(w): Parameters<schemas::ObsGetRpgStatusWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_rpg_status", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_money", description = "Gold/silver/copper balance.")]
    async fn obs_get_money(&self, Parameters(w): Parameters<schemas::ObsGetMoneyWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_money", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_position", description = "Map/zone/area IDs + names + (x,y,z,o).")]
    async fn obs_get_position(&self, Parameters(w): Parameters<schemas::ObsGetPositionWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_position", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_group", description = "Party/raid roster, HP/mana%, distances.")]
    async fn obs_get_group(&self, Parameters(w): Parameters<schemas::ObsGetGroupWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_group", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.get_talents", description = "Active-spec talents: flat list with tab/row/col/rank.")]
    async fn obs_get_talents(&self, Parameters(w): Parameters<schemas::ObsGetTalentsWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.get_talents", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.game_events", description = "Full game-event schedule: active set, resolved start/end/next per event, and raw sHolidaysStore dump.")]
    async fn obs_game_events(&self, Parameters(w): Parameters<schemas::ObsGameEventsWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.game_events", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.query_db", description = "Run an allowlisted MySQL template against the auth/char DBs.")]
    async fn obs_query_db(&self, Parameters(w): Parameters<schemas::ObsQueryDbWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.query_db", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "obs.lfg_pending", description = "Drain pending real-player + bot LFG join intents recorded by the veto hook / bot-queue.")]
    async fn obs_lfg_pending(&self, Parameters(w): Parameters<schemas::ObsLfgPendingWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("obs.lfg_pending", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    // ── event.* (2) ───────────────────────────────────────────────────────────

    #[tool(name = "event.start", description = "Start a game event by id (slice-driven scheduler enactment primitive).")]
    async fn event_start(&self, Parameters(w): Parameters<schemas::EventStartWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("event.start", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "event.stop", description = "Stop a game event by id (slice-driven scheduler enactment primitive).")]
    async fn event_stop(&self, Parameters(w): Parameters<schemas::EventStopWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("event.stop", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    // ── memory.* (7) ─────────────────────────────────────────────────────────

    #[tool(name = "memory.write", description = "Write a new episode to a bot's episodic memory store.")]
    async fn memory_write(&self, Parameters(w): Parameters<schemas::MemoryWriteWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("memory.write", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "memory.read", description = "Fetch a single episode by primary-key ID.")]
    async fn memory_read(&self, Parameters(w): Parameters<schemas::MemoryReadWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("memory.read", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "memory.recall", description = "Hybrid-ranked recall: semantic + recency + salience + MMR.")]
    async fn memory_recall(&self, Parameters(w): Parameters<schemas::MemoryRecallWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("memory.recall", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "memory.search", description = "Pure ANN search by text or pre-computed embedding vector.")]
    async fn memory_search(&self, Parameters(w): Parameters<schemas::MemorySearchWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("memory.search", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "memory.list", description = "Paginated episode listing with optional type/entity/time filters.")]
    async fn memory_list(&self, Parameters(w): Parameters<schemas::MemoryListWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("memory.list", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "memory.update", description = "Patch content, salience, or metadata on an existing episode.")]
    async fn memory_update(&self, Parameters(w): Parameters<schemas::MemoryUpdateWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("memory.update", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "memory.delete", description = "Hard-delete an episode (cascades to entities + embeddings).")]
    async fn memory_delete(&self, Parameters(w): Parameters<schemas::MemoryDeleteWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("memory.delete", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    // ── lfg.* (2) ────────────────────────────────────────────────────────────

    #[tool(name = "lfg.form_group", description = "Force-create a server-side LFG group and teleport all members into the dungeon.")]
    async fn lfg_form_group(&self, Parameters(w): Parameters<schemas::LfgFormGroupWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("lfg.form_group", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }

    #[tool(name = "lfg.cancel", description = "Drain pending LFG cancel intents recorded when a player presses Leave; returns {\"cancelled\":[<guid_low>,...]} for the slice to remove from its queue.")]
    async fn lfg_cancel(&self, Parameters(w): Parameters<schemas::LfgCancelWrapper>, Extension(parts): Extension<http::request::Parts>) -> CallToolResult {
        self.forward("lfg.cancel", serde_json::to_value(&w.args).unwrap_or_default(), &parts).await
    }
}

// `#[tool_handler]` sets serverInfo.name and serverInfo.version in the
// `initialize` response (spike finding (e): name="tot-harness", version="0.2.0").
//
// We define `list_tools` MANUALLY here so the macro skips generating the default.
// (rmcp-macros tool_handler only generates `list_tools` when `has_method` returns
// false — see rmcp-macros-1.7.0/src/tool_handler.rs:64.)  Our override calls
// `Self::tool_router().list_all()` (identical to the macro default) and then
// applies `transform_tools_schemas` to rewrite every nullable `"type":["T","null"]`
// as `"anyOf":[{"type":"T"},{"type":"null"}]` — the pydantic-v2 form the brain
// client (`schema_builder.py:_render_arg`) expects.
#[tool_handler(name = "tot-harness", version = "0.2.0")]
impl ServerHandler for HarnessMcp {
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let raw = ListToolsResult {
            tools:       Self::tool_router().list_all(),
            meta:        None,
            next_cursor: None,
        };
        Ok(transform_tools_schemas(raw))
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::strip_top_level_nulls;
    use crate::mcp::schemas;

    // ── MCP {args} envelope regression guard ─────────────────────────────────
    //
    // The brain client (tot/brain/brain_sidecar/schema_builder.py:73,
    // `unwrap_fastmcp_args`) RAISES ValueError unless inputSchema.properties.args
    // exists for EVERY tool.  If any tool method is changed from
    // `Parameters<XxxWrapper>` back to `Parameters<XxxArgs>` (flat), the rmcp
    // proc-macro will emit a schema WITHOUT `properties.args` and the brain will
    // FAIL TO BOOT.
    //
    // This test asserts the representative case (obs.ping).  The full suite
    // covering all 47 wrappers lives in mcp::schemas::tests::all_47_wrappers_have_args_envelope.
    // (A live tools/list assertion previously lived in the Python parity gate,
    // retired along with the Python sidecars.)
    #[test]
    fn obs_ping_wrapper_schema_has_args_key() {
        let schema = schemars::schema_for!(schemas::ObsPingWrapper);
        let v: serde_json::Value = serde_json::to_value(&schema).expect("schema serializes");
        let has_args = v
            .get("properties")
            .and_then(|p| p.get("args"))
            .map(|a| a.is_object())
            .unwrap_or(false);
        assert!(
            has_args,
            "ObsPingWrapper JSON schema must have properties.args (brain contract). \
             Did someone revert Parameters<ObsPingWrapper> to Parameters<ObsPingArgs>?"
        );
    }

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
        let input = json!({
            "a": null,
            "b": 1,
            "c": {
                "d": null,
                "e": 2,
            },
        });
        let out = strip_top_level_nulls(input);
        assert!(out.get("a").is_none());
        assert_eq!(out["b"], 1);
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

    // ── transform_nullable_types ──────────────────────────────────────────────
    //
    // The brain client's _render_arg (schema_builder.py:158) calls
    //   _JSON_TO_PY_TYPE.get(schema.get("type", ""), "any")
    // and crashes with `TypeError: unhashable type: 'list'` when `type` is a JSON
    // array (the schemars 1.x draft-2020-12 form for nullable fields).
    //
    // These tests verify that transform_nullable_types converts every
    // `"type": ["T", "null"]` node into `"anyOf": [{"type":"T"}, {"type":"null"}]`
    // while keeping outer metadata keys (description/default/title) in place.

    use super::transform_nullable_types;

    /// Basic case: `{"type": ["integer", "null"]}` → anyOf form.
    #[test]
    fn transform_nullable_integer() {
        let input = json!({
            "type": ["integer", "null"],
            "description": "A nullable integer",
            "default": null,
        });
        let out = transform_nullable_types(input);
        // "type" key must be gone at the outer level
        assert!(out.get("type").is_none(), "type key must be removed from outer level");
        // anyOf must be present
        let any_of = out.get("anyOf").expect("anyOf must be present");
        let branches = any_of.as_array().expect("anyOf must be an array");
        assert_eq!(branches.len(), 2, "must have 2 branches: integer + null");
        assert_eq!(branches[0], json!({"type": "integer"}));
        assert_eq!(branches[1], json!({"type": "null"}));
        // description stays at outer level
        assert_eq!(out.get("description"), Some(&json!("A nullable integer")));
        // default stays at outer level
        assert!(out.get("default").is_some(), "default must remain at outer level");
    }

    /// String nullable: `{"type": ["string", "null"], "description": "..."}` → anyOf.
    #[test]
    fn transform_nullable_string() {
        let input = json!({
            "type": ["string", "null"],
            "description": "optional text",
        });
        let out = transform_nullable_types(input);
        assert!(out.get("type").is_none());
        let branches = out["anyOf"].as_array().unwrap();
        assert_eq!(branches[0], json!({"type": "string"}));
        assert_eq!(branches[1], json!({"type": "null"}));
        assert_eq!(out.get("description"), Some(&json!("optional text")));
    }

    /// Non-nullable type strings must NOT be transformed.
    #[test]
    fn transform_non_nullable_not_changed() {
        let input = json!({
            "type": "integer",
            "description": "required int",
        });
        let out = transform_nullable_types(input.clone());
        assert_eq!(out, input, "plain string type must be unchanged");
    }

    /// Nested properties are transformed recursively.
    #[test]
    fn transform_recurses_into_properties() {
        let input = json!({
            "type": "object",
            "properties": {
                "since_ts_ms": {
                    "type": ["integer", "null"],
                    "description": "optional timestamp",
                },
                "limit": {
                    "type": ["integer", "null"],
                },
                "required_field": {
                    "type": "integer",
                },
            },
        });
        let out = transform_nullable_types(input);
        // top-level type unchanged (it's a plain string)
        assert_eq!(out.get("type"), Some(&json!("object")));

        let props = out["properties"].as_object().unwrap();
        // since_ts_ms: must be anyOf form
        let since = &props["since_ts_ms"];
        assert!(since.get("type").is_none(), "since_ts_ms type must be moved to anyOf");
        assert!(since.get("anyOf").is_some(), "since_ts_ms must have anyOf");
        // required_field: unchanged
        let req = &props["required_field"];
        assert_eq!(req.get("type"), Some(&json!("integer")));
    }

    /// $defs entries are transformed recursively.
    #[test]
    fn transform_recurses_into_defs() {
        let input = json!({
            "type": "object",
            "$defs": {
                "MyType": {
                    "type": "object",
                    "properties": {
                        "opt_field": {
                            "type": ["boolean", "null"],
                        },
                    },
                },
            },
        });
        let out = transform_nullable_types(input);
        let defs = out["$defs"].as_object().unwrap();
        let my_type_props = &defs["MyType"]["properties"];
        let opt_field = &my_type_props["opt_field"];
        assert!(opt_field.get("type").is_none(), "opt_field type must be moved to anyOf in $defs");
        assert!(opt_field.get("anyOf").is_some(), "opt_field must have anyOf in $defs");
    }

    /// After transform, NO node anywhere in the schema tree should have "type" as
    /// an array. This is the regression guard: if any nullable field escapes
    /// transform, the brain will crash.
    ///
    /// We use the REAL `obs.get_combat_log` wrapper schema (which has Optional
    /// fields `since_ts_ms` and `limit`) as the test input.
    #[test]
    fn transform_produces_no_type_arrays_in_real_schema() {
        use crate::mcp::schemas::ObsGetCombatLogWrapper;

        let raw_schema = schemars::schema_for!(ObsGetCombatLogWrapper);
        let v: serde_json::Value = serde_json::to_value(&raw_schema).unwrap();

        // Confirm the raw schema HAS at least one array-type (otherwise the test
        // isn't proving anything).
        fn has_array_type(v: &serde_json::Value) -> bool {
            match v {
                serde_json::Value::Object(m) => {
                    if let Some(t) = m.get("type") {
                        if t.is_array() { return true; }
                    }
                    m.values().any(has_array_type)
                }
                serde_json::Value::Array(a) => a.iter().any(has_array_type),
                _ => false,
            }
        }
        assert!(
            has_array_type(&v),
            "ObsGetCombatLogWrapper must have at least one nullable (array-type) field \
             in its raw schemars output — otherwise this test proves nothing"
        );

        // Apply transform.
        let transformed = transform_nullable_types(v);

        // Assert: NO node anywhere has type as an array.
        fn has_no_array_type(v: &serde_json::Value) -> bool {
            match v {
                serde_json::Value::Object(m) => {
                    if let Some(t) = m.get("type") {
                        if t.is_array() { return false; }
                    }
                    m.values().all(has_no_array_type)
                }
                serde_json::Value::Array(a) => a.iter().all(has_no_array_type),
                _ => true,
            }
        }
        assert!(
            has_no_array_type(&transformed),
            "After transform, no schema node may have \"type\" as a JSON array. \
             At least one nullable field escaped the transform."
        );

        // Assert: properties.args still present (brain contract preserved).
        assert!(
            transformed.get("properties")
                .and_then(|p| p.get("args"))
                .is_some(),
            "properties.args must survive the nullable transform (brain contract)"
        );
    }
}
