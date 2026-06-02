//! MCP ServerHandler — 15 memory/goals tools.
//!
//! Pattern mirrors harness-rs `mcp/handler.rs`:
//! - `#[tool_router]` on inherent impl wires each `#[tool]` method.
//! - `#[tool_handler(name="memory-sidecar", version="0.2.1")]` sets serverInfo.
//! - Each method takes `Parameters<XxxWrapper>` + `Extension<http::request::Parts>`,
//!   converts `w.args` → core request type, calls the service directly.
//!
//! CRITICAL: all 15 tool methods MUST be written out explicitly inside the
//! `#[tool_router]` block — the proc-macro detects `#[tool]` in the token
//! stream before macro expansion.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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

use crate::auth::TokenRecord;
use crate::core::{
    ForgetReq, GoalCompleteReq, GoalCreateReq, GoalListReq, GoalOutcome, GoalUpdateReq,
    ListReq, MemoryService, RecallAboutReq, RecallReq, SearchReq, UpdateReq, WriteReq,
};
use crate::mcp::schemas;

// ── Schema-transform helpers (copied verbatim from harness-rs handler.rs) ─────
//
// These are MANDATORY at the serving boundary.  The brain client's
// `schema_builder.py:_render_arg` calls `_JSON_TO_PY_TYPE.get(schema.get("type"))`
// and crashes with `TypeError: unhashable type: 'list'` when `type` is a JSON
// array — the schemars 1.x draft-2020-12 form for nullable fields.

/// Convert schemars draft-2020-12 nullable form to pydantic-v2 `anyOf` form.
///
/// Wherever a node's `"type"` key is a JSON array (possibly containing `"null"`),
/// rewrite as `"anyOf": [{"type":"T"}, {"type":"null"}]`.
/// Keys that are structural type descriptors are moved into the first branch;
/// documentation keys (`description`, `default`, `title`) stay at the outer level.
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
    // Recursively transform all nested values first.
    for v in map.values_mut() {
        *v = transform_nullable_types(std::mem::replace(v, Value::Null));
    }

    let type_is_array = map.get("type").map(|t| t.is_array()).unwrap_or(false);
    if !type_is_array {
        return Value::Object(map);
    }

    let type_arr: Vec<Value> = match map.remove("type") {
        Some(Value::Array(a)) => a,
        other => {
            if let Some(t) = other { map.insert("type".into(), t); }
            return Value::Object(map);
        }
    };

    let non_null_types: Vec<&Value> = type_arr.iter().filter(|t| t != &&Value::String("null".into())).collect();
    let has_null = type_arr.iter().any(|t| t == &Value::String("null".into()));

    let mut first_branch_extra: Map<String, Value> = Map::new();
    for &key in STRUCTURAL_KEYS {
        if key == "anyOf" || key == "oneOf" || key == "allOf" {
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

    if !has_null && non_null_types.len() == 1 {
        map.insert("type".into(), non_null_types[0].clone());
        for (k, v) in first_branch_extra {
            map.insert(k, v);
        }
        return Value::Object(map);
    }

    map.insert("anyOf".into(), Value::Array(branches));
    Value::Object(map)
}

/// Apply `transform_nullable_types` to every tool in a `ListToolsResult`.
fn transform_tools_schemas(mut result: ListToolsResult) -> ListToolsResult {
    result.tools = result.tools
        .into_iter()
        .map(|mut tool| {
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

// ── MemoryMcp ─────────────────────────────────────────────────────────────────

/// MCP ServerHandler backed by the shared `MemoryService`.
#[derive(Clone)]
pub struct MemoryMcp {
    service:     Arc<MemoryService>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl MemoryMcp {
    pub fn new(service: Arc<MemoryService>) -> Self {
        Self {
            service,
            tool_router: Self::tool_router(),
        }
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Strips every top-level key whose value is `Value::Null`.
///
/// Mirrors Python `args.model_dump(exclude_none=True)`.
/// Must NOT recurse — pydantic `exclude_none` only strips the model's own level.
pub fn strip_top_level_nulls(obj: Value) -> Value {
    match obj {
        Value::Object(mut map) => {
            map.retain(|_, v| !v.is_null());
            Value::Object(map)
        }
        other => other,
    }
}

#[allow(dead_code)]
fn unix_now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[allow(dead_code)]
fn gen_request_id() -> String {
    let id = Uuid::new_v4().simple().to_string();
    format!("mcp_{}", &id[..12])
}

/// Serialize a value to JSON string; fallback on error.
fn to_json_str(v: &impl serde::Serialize) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| r#"{"ok":false,"error":"serialize_error"}"#.to_owned())
}

/// Return a success `CallToolResult` wrapping a serialized JSON payload.
fn ok_result(v: &impl serde::Serialize) -> CallToolResult {
    CallToolResult::success(vec![Content::text(to_json_str(v))])
}

/// Return a `CallToolResult` with an error text (isError=false for app-level errors,
/// matching Python parity where the tool result body carries `{ok:false,…}`).
fn err_result(msg: &str) -> CallToolResult {
    let body = serde_json::json!({ "ok": false, "error": msg });
    CallToolResult::success(vec![Content::text(to_json_str(&body))])
}

/// Extract the `TokenRecord` from the request `Parts` extensions.
/// Returns `None` if no token was injected (auth layer rejected or bypassed).
fn token_from_parts(parts: &http::request::Parts) -> Option<&TokenRecord> {
    parts.extensions.get::<TokenRecord>()
}

// ── 15-tool #[tool_router] impl ───────────────────────────────────────────────

#[tool_router]
impl MemoryMcp {
    // ── memory.write ──────────────────────────────────────────────────────────

    #[tool(name = "memory.write", description = "Create a memory.")]
    async fn memory_write(
        &self,
        Parameters(w): Parameters<schemas::MemoryWriteWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        // Parse relations: each item should be a JSON object with "src","rel","dst" keys.
        let mut relations = Vec::new();
        for rel_val in a.relations {
            if let Some(obj) = rel_val.as_object() {
                let src = obj.get("src").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let rel = obj.get("rel").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                let dst = obj.get("dst").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                if !src.is_empty() && !rel.is_empty() && !dst.is_empty() {
                    relations.push(crate::core::RelationTriple { src, rel, dst });
                }
            }
        }
        let req = WriteReq {
            bot_id: a.bot_id,
            text: a.text,
            salience: a.salience as f32,
            entities: a.entities,
            relations,
            memory_type: None,
            source: None,
        };
        match self.service.write(req).await {
            Ok(resp) => ok_result(&serde_json::json!({
                "ok": true,
                "memory_id": resp.memory_id,
                "evicted": resp.evicted,
            })),
            Err(e) => err_result(&format!("write_error: {e}")),
        }
    }

    // ── memory.read ───────────────────────────────────────────────────────────

    #[tool(name = "memory.read", description = "Read a memory by id.")]
    async fn memory_read(
        &self,
        Parameters(w): Parameters<schemas::MemoryReadWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        match self.service.read(&a.bot_id, &a.memory_id).await {
            Ok(Some(row)) => ok_result(&serde_json::json!({
                "ok": true,
                "memory": {
                    "id": row.id,
                    "bot_id": row.bot_id,
                    "text": row.text,
                    "salience": row.salience,
                    "memory_type": row.memory_type,
                    "source": row.source,
                    "created_ts": row.created_ts,
                    "last_recalled_ts": row.last_recalled_ts,
                }
            })),
            Ok(None) => err_result("not_found"),
            Err(e)   => err_result(&format!("read_error: {e}")),
        }
    }

    // ── memory.update ─────────────────────────────────────────────────────────

    #[tool(name = "memory.update", description = "Update memory text/metadata; re-embeds on text change.")]
    async fn memory_update(
        &self,
        Parameters(w): Parameters<schemas::MemoryUpdateWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let req = UpdateReq {
            bot_id: a.bot_id,
            memory_id: a.memory_id,
            text: a.text,
            salience: a.salience.map(|s| s as f32),
            memory_type: a.memory_type,
            source: a.source,
        };
        match self.service.update(req).await {
            Ok(resp) => ok_result(&serde_json::json!({
                "ok": true,
                "updated": resp.updated,
                "re_embedded": resp.re_embedded,
            })),
            Err(e) => err_result(&format!("update_error: {e}")),
        }
    }

    // ── memory.delete ─────────────────────────────────────────────────────────

    #[tool(name = "memory.delete", description = "Delete a memory + embeddings + entity links.")]
    async fn memory_delete(
        &self,
        Parameters(w): Parameters<schemas::MemoryDeleteWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let req = ForgetReq {
            bot_id: a.bot_id,
            memory_id: a.memory_id,
        };
        match self.service.forget(req).await {
            Ok(resp) => ok_result(&serde_json::json!({
                "ok": true,
                "forgotten": resp.forgotten,
            })),
            Err(e) => err_result(&format!("delete_error: {e}")),
        }
    }

    // ── memory.recall ─────────────────────────────────────────────────────────

    #[tool(name = "memory.recall", description = "Semantic recall via vec_memories KNN + MMR.")]
    async fn memory_recall(
        &self,
        Parameters(w): Parameters<schemas::MemoryRecallWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let req = RecallReq {
            bot_id: a.bot_id,
            query: a.query,
            top_k: a.top_k as usize,
            since_ts: a.since_ts,
            until_ts: a.until_ts,
            memory_type: a.memory_type,
        };
        match self.service.recall(req).await {
            Ok(resp) => {
                let memories: Vec<_> = resp.memories.iter().map(|m| serde_json::json!({
                    "memory_id": m.memory_id,
                    "text": m.text,
                    "score": m.score,
                    "ts": m.ts,
                })).collect();
                ok_result(&serde_json::json!({ "ok": true, "memories": memories }))
            }
            Err(e) => err_result(&format!("recall_error: {e}")),
        }
    }

    // ── memory.recall_about ───────────────────────────────────────────────────

    #[tool(name = "memory.recall_about", description = "Entity-anchored BFS recall + MMR.")]
    async fn memory_recall_about(
        &self,
        Parameters(w): Parameters<schemas::MemoryRecallAboutWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let req = RecallAboutReq {
            bot_id: a.bot_id,
            entity: a.entity,
            max_hops: a.max_hops as usize,
            top_k: a.top_k as usize,
            since_ts: a.since_ts,
            until_ts: a.until_ts,
            memory_type: a.memory_type,
        };
        match self.service.recall_about(req).await {
            Ok(resp) => ok_result(&serde_json::json!({ "ok": true, "hints": resp.hints })),
            Err(e)   => err_result(&format!("recall_about_error: {e}")),
        }
    }

    // ── memory.search ─────────────────────────────────────────────────────────

    #[tool(name = "memory.search", description = "Hybrid: BM25 + dense + entity, RRF + MMR.")]
    async fn memory_search(
        &self,
        Parameters(w): Parameters<schemas::MemorySearchWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let req = SearchReq {
            bot_id: a.bot_id,
            query: a.query,
            top_k: a.top_k as usize,
            since_ts: a.since_ts,
            until_ts: a.until_ts,
            memory_type: a.memory_type,
        };
        match self.service.search(req).await {
            Ok(resp) => {
                let items: Vec<_> = resp.items.iter().map(|i| serde_json::json!({
                    "memory_id": i.memory_id,
                    "text": i.text,
                    "score": i.score,
                    "ts": i.ts,
                    "signals": {
                        "bm25_rank": i.signals.bm25_rank,
                        "dense_rank": i.signals.dense_rank,
                        "entity_rank": i.signals.entity_rank,
                    }
                })).collect();
                ok_result(&serde_json::json!({
                    "ok": true,
                    "items": items,
                    "total_candidates": resp.total_candidates,
                }))
            }
            Err(e) => err_result(&format!("search_error: {e}")),
        }
    }

    // ── memory.list ───────────────────────────────────────────────────────────

    #[tool(name = "memory.list", description = "List memories filtered by type/source/time.")]
    async fn memory_list(
        &self,
        Parameters(w): Parameters<schemas::MemoryListWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let req = ListReq {
            bot_id: a.bot_id,
            memory_type: a.memory_type,
            source: a.source,
            since_ts: a.since_ts,
            until_ts: a.until_ts,
            limit: a.limit,
            offset: a.offset,
        };
        match self.service.list(req).await {
            Ok(resp) => {
                let items: Vec<_> = resp.items.iter().map(|r| serde_json::json!({
                    "id": r.id,
                    "bot_id": r.bot_id,
                    "text": r.text,
                    "salience": r.salience,
                    "memory_type": r.memory_type,
                    "source": r.source,
                    "created_ts": r.created_ts,
                    "last_recalled_ts": r.last_recalled_ts,
                })).collect();
                ok_result(&serde_json::json!({ "ok": true, "items": items, "total": resp.total }))
            }
            Err(e) => err_result(&format!("list_error: {e}")),
        }
    }

    // ── memory.personality_get ────────────────────────────────────────────────

    #[tool(name = "memory.personality_get", description = "Get a bot's persona.")]
    async fn memory_personality_get(
        &self,
        Parameters(w): Parameters<schemas::PersonalityGetWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        match self.service.personality_get(&a.bot_id).await {
            Ok(persona) => ok_result(&serde_json::json!({ "ok": true, "persona": persona })),
            Err(e)      => err_result(&format!("personality_get_error: {e}")),
        }
    }

    // ── memory.personality_set ────────────────────────────────────────────────

    #[tool(name = "memory.personality_set", description = "Set a bot's persona (<=4000 chars).")]
    async fn memory_personality_set(
        &self,
        Parameters(w): Parameters<schemas::PersonalitySetWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        match self.service.personality_set(&a.bot_id, a.persona).await {
            Ok(()) => ok_result(&serde_json::json!({ "ok": true })),
            Err(e) => err_result(&format!("personality_set_error: {e}")),
        }
    }

    // ── goals.create ──────────────────────────────────────────────────────────

    #[tool(name = "goals.create", description = "Create a goal (status='pending').")]
    async fn goals_create(
        &self,
        Parameters(w): Parameters<schemas::GoalCreateWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let req = GoalCreateReq {
            bot_id: a.bot_id,
            text: a.text,
            source: a.source,
            priority: a.priority,
            origin_memory: a.origin_memory,
        };
        match self.service.goal_create(req).await {
            Ok(resp) => ok_result(&serde_json::json!({
                "ok": true,
                "goal_id": resp.goal_id,
                "status": resp.status,
            })),
            Err(e) => err_result(&format!("goal_create_error: {e}")),
        }
    }

    // ── goals.read ────────────────────────────────────────────────────────────

    #[tool(name = "goals.read", description = "Read a goal by id.")]
    async fn goals_read(
        &self,
        Parameters(w): Parameters<schemas::GoalReadWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        match self.service.goal_read(&a.bot_id, &a.goal_id).await {
            Ok(Some(row)) => ok_result(&serde_json::json!({
                "ok": true,
                "goal": {
                    "id": row.id,
                    "bot_id": row.bot_id,
                    "text": row.text,
                    "status": row.status,
                    "source": row.source,
                    "priority": row.priority,
                    "origin_memory": row.origin_memory,
                    "created_ts": row.created_ts,
                    "updated_ts": row.updated_ts,
                    "completed_ts": row.completed_ts,
                }
            })),
            Ok(None) => err_result("not_found"),
            Err(e)   => err_result(&format!("goal_read_error: {e}")),
        }
    }

    // ── goals.update ──────────────────────────────────────────────────────────

    #[tool(name = "goals.update", description = "Update goal; validates status transitions.")]
    async fn goals_update(
        &self,
        Parameters(w): Parameters<schemas::GoalUpdateWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let req = GoalUpdateReq {
            bot_id: a.bot_id,
            goal_id: a.goal_id,
            text: a.text,
            status: a.status,
            priority: a.priority,
            origin_memory: a.origin_memory,
        };
        match self.service.goal_update(req).await {
            Ok(resp) => ok_result(&serde_json::json!({
                "ok": true,
                "updated": resp.updated,
            })),
            Err(e) => err_result(&format!("goal_update_error: {e}")),
        }
    }

    // ── goals.list ────────────────────────────────────────────────────────────

    #[tool(name = "goals.list", description = "List goals filtered by status.")]
    async fn goals_list(
        &self,
        Parameters(w): Parameters<schemas::GoalListWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let req = GoalListReq {
            bot_id: a.bot_id,
            status: a.status,
            limit: a.limit,
            offset: a.offset,
        };
        match self.service.goal_list(req).await {
            Ok(resp) => {
                let items: Vec<_> = resp.items.iter().map(|r| serde_json::json!({
                    "id": r.id,
                    "bot_id": r.bot_id,
                    "text": r.text,
                    "status": r.status,
                    "source": r.source,
                    "priority": r.priority,
                    "origin_memory": r.origin_memory,
                    "created_ts": r.created_ts,
                    "updated_ts": r.updated_ts,
                    "completed_ts": r.completed_ts,
                })).collect();
                ok_result(&serde_json::json!({ "ok": true, "items": items, "total": resp.total }))
            }
            Err(e) => err_result(&format!("goal_list_error: {e}")),
        }
    }

    // ── goals.complete ────────────────────────────────────────────────────────

    #[tool(name = "goals.complete", description = "Mark completed/abandoned; optionally records a goal_link memory.")]
    async fn goals_complete(
        &self,
        Parameters(w): Parameters<schemas::GoalCompleteWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        if token_from_parts(&parts).is_none() {
            return err_result("unauthorized");
        }
        let a = w.args;
        let outcome = match a.outcome {
            schemas::GoalOutcomeEnum::Completed => GoalOutcome::Completed,
            schemas::GoalOutcomeEnum::Abandoned => GoalOutcome::Abandoned,
        };
        let req = GoalCompleteReq {
            bot_id: a.bot_id,
            goal_id: a.goal_id,
            outcome,
            also_record_memory: a.also_record_memory,
        };
        match self.service.goal_complete(req).await {
            Ok(resp) => ok_result(&serde_json::json!({
                "ok": true,
                "updated": resp.updated,
                "memory_id": resp.memory_id,
            })),
            Err(e) => err_result(&format!("goal_complete_error: {e}")),
        }
    }
}

// `#[tool_handler]` sets serverInfo.name="memory-sidecar", version="0.2.1".
// Manual `list_tools` override applies `transform_tools_schemas` so the brain
// client never sees `"type":["T","null"]` arrays (nullable schemars 1.x form).
#[tool_handler(name = "memory-sidecar", version = "0.2.1")]
impl ServerHandler for MemoryMcp {
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
    use serde_json::{Value, json};

    use super::{MemoryMcp, strip_top_level_nulls, transform_nullable_types};
    use crate::mcp::schemas;

    // ── strip_top_level_nulls ─────────────────────────────────────────────────

    #[test]
    fn removes_top_level_nulls() {
        let input = json!({ "a": null, "b": 1, "c": "hello" });
        let out = strip_top_level_nulls(input);
        assert!(out.get("a").is_none());
        assert_eq!(out["b"], 1);
        assert_eq!(out["c"], "hello");
    }

    #[test]
    fn does_not_recurse_into_nested_objects() {
        let input = json!({ "a": null, "b": 1, "c": { "d": null, "e": 2 } });
        let out = strip_top_level_nulls(input);
        assert!(out.get("a").is_none());
        assert_eq!(out["b"], 1);
        assert_eq!(out["c"]["d"], json!(null));
        assert_eq!(out["c"]["e"], 2);
    }

    #[test]
    fn non_object_value_passed_through() {
        let arr = json!([1, null, 3]);
        assert_eq!(strip_top_level_nulls(arr.clone()), arr);
    }

    // ── transform_nullable_types ──────────────────────────────────────────────

    #[test]
    fn transform_nullable_integer() {
        let input = json!({ "type": ["integer", "null"], "description": "A nullable int", "default": null });
        let out = transform_nullable_types(input);
        assert!(out.get("type").is_none());
        let branches = out["anyOf"].as_array().unwrap();
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0], json!({"type": "integer"}));
        assert_eq!(branches[1], json!({"type": "null"}));
        assert_eq!(out.get("description"), Some(&json!("A nullable int")));
    }

    #[test]
    fn transform_non_nullable_not_changed() {
        let input = json!({ "type": "integer", "description": "required int" });
        let out = transform_nullable_types(input.clone());
        assert_eq!(out, input);
    }

    #[test]
    fn transform_recurses_into_properties() {
        let input = json!({
            "type": "object",
            "properties": {
                "since_ts": { "type": ["integer", "null"] },
                "required_field": { "type": "integer" },
            },
        });
        let out = transform_nullable_types(input);
        assert_eq!(out.get("type"), Some(&json!("object")));
        let props = out["properties"].as_object().unwrap();
        assert!(props["since_ts"].get("type").is_none());
        assert!(props["since_ts"].get("anyOf").is_some());
        assert_eq!(props["required_field"]["type"], json!("integer"));
    }

    /// After transform, NO node anywhere in the schema tree should have "type" as an array.
    /// Use MemoryRecallWrapper (several Option fields) as the test input.
    #[test]
    fn transform_produces_no_type_arrays_in_recall_wrapper_schema() {
        let raw_schema = schemars::schema_for!(schemas::MemoryRecallWrapper);
        let v: serde_json::Value = serde_json::to_value(&raw_schema).unwrap();

        fn has_array_type(v: &serde_json::Value) -> bool {
            match v {
                serde_json::Value::Object(m) => {
                    if let Some(t) = m.get("type") { if t.is_array() { return true; } }
                    m.values().any(has_array_type)
                }
                serde_json::Value::Array(a) => a.iter().any(has_array_type),
                _ => false,
            }
        }
        assert!(has_array_type(&v), "MemoryRecallWrapper must have at least one nullable field");

        let transformed = transform_nullable_types(v);

        fn has_no_array_type(v: &serde_json::Value) -> bool {
            match v {
                serde_json::Value::Object(m) => {
                    if let Some(t) = m.get("type") { if t.is_array() { return false; } }
                    m.values().all(has_no_array_type)
                }
                serde_json::Value::Array(a) => a.iter().all(has_no_array_type),
                _ => true,
            }
        }
        assert!(
            has_no_array_type(&transformed),
            "After transform, no schema node may have type as a JSON array",
        );

        // Brain contract: properties.args must survive.
        assert!(
            transformed.get("properties").and_then(|p| p.get("args")).is_some(),
            "properties.args must survive the nullable transform",
        );
    }

    // ── 15-tool count + names + args-envelope ─────────────────────────────────
    //
    // This lives in handler::tests so Self::tool_router() (private to the impl
    // block) is accessible.
    #[test]
    fn tool_router_has_exactly_15_tools_with_args_envelope() {
        let router = MemoryMcp::tool_router();
        let all_tools = router.list_all();

        assert_eq!(all_tools.len(), 15, "ToolRouter must list exactly 15 tools");

        // Verify all 15 exact names.
        let names: Vec<&str> = all_tools.iter().map(|t| t.name.as_ref()).collect();
        for expected in &[
            "memory.write", "memory.read", "memory.update", "memory.delete",
            "memory.recall", "memory.recall_about", "memory.search", "memory.list",
            "memory.personality_get", "memory.personality_set",
            "goals.create", "goals.read", "goals.update", "goals.list", "goals.complete",
        ] {
            assert!(names.contains(expected), "ToolRouter missing: {expected}");
        }

        // Every tool must have inputSchema.properties.args (brain contract).
        for tool in &all_tools {
            let schema_val = Value::Object((*tool.input_schema).clone());
            let has_args = schema_val
                .get("properties")
                .and_then(|p| p.get("args"))
                .map(|a| a.is_object())
                .unwrap_or(false);
            assert!(
                has_args,
                "tool {}: inputSchema must have properties.args (brain contract)",
                tool.name,
            );
        }
    }
}
