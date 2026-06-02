//! Per-tool argument schemas for the MCP adapter.
//!
//! Port of `memory_sidecar/tool_schemas.py` — one `XxxArgs` struct + one
//! `XxxWrapper { pub args: XxxArgs }` per tool.
//!
//! All 15 tools from `TOOL_SCHEMAS` are represented here.
//!
//! MANDATORY derives: `Debug, Clone, Serialize, Deserialize, JsonSchema`.
//! `Serialize` is required so `serde_json::to_value(&w.args)` works for
//! `strip_top_level_nulls` in the handler.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Shared enum ───────────────────────────────────────────────────────────────

/// Goal completion outcome — mirrors Python `Literal["completed", "abandoned"]`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GoalOutcomeEnum {
    Completed,
    Abandoned,
}

// ── memory.write ──────────────────────────────────────────────────────────────

/// Args for `memory.write`.
///
/// Exact parity with Python `MemoryWriteArgs` in `tool_schemas.py`:
/// `bot_id`, `text`, `salience`, `entities`, `relations` — NO `memory_type` or `source`.
/// The handler hardcodes `memory_type: None, source: None` in the internal `WriteReq`.
///
/// `relations` is a list of JSON objects with `src`, `rel`, `dst` keys.
/// Typed as `Vec<Value>` to accept arbitrary shapes (Python uses `list[dict[str,str]]`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryWriteArgs {
    /// Bot identifier.
    pub bot_id: String,
    /// Episode text to store.
    pub text: String,
    /// Importance weight in [0.0, 1.0].
    pub salience: f64,
    /// Named entities to link (e.g. ["Ragnaros", "Molten Core"]).
    #[serde(default)]
    pub entities: Vec<String>,
    /// Relation triples: each `{"src":"A","rel":"killed","dst":"B"}`.
    #[serde(default)]
    pub relations: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryWriteWrapper {
    pub args: MemoryWriteArgs,
}

// ── memory.read ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryReadArgs {
    pub bot_id: String,
    pub memory_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryReadWrapper {
    pub args: MemoryReadArgs,
}

// ── memory.update ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryUpdateArgs {
    pub bot_id: String,
    pub memory_id: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub salience: Option<f64>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryUpdateWrapper {
    pub args: MemoryUpdateArgs,
}

// ── memory.delete ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryDeleteArgs {
    pub bot_id: String,
    pub memory_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryDeleteWrapper {
    pub args: MemoryDeleteArgs,
}

// ── memory.recall ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallArgs {
    pub bot_id: String,
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: i64,
    #[serde(default)]
    pub since_ts: Option<i64>,
    #[serde(default)]
    pub until_ts: Option<i64>,
    #[serde(default)]
    pub memory_type: Option<String>,
}

fn default_top_k() -> i64 { 5 }
fn default_top_k_3() -> i64 { 3 }
fn default_limit() -> i64 { 50 }
fn default_offset() -> i64 { 0 }
fn default_max_hops() -> i64 { 2 }
fn default_priority() -> i64 { 0 }
fn default_also_record_memory() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallWrapper {
    pub args: MemoryRecallArgs,
}

// ── memory.recall_about ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallAboutArgs {
    pub bot_id: String,
    pub entity: String,
    #[serde(default = "default_max_hops")]
    pub max_hops: i64,
    #[serde(default = "default_top_k_3")]
    pub top_k: i64,
    #[serde(default)]
    pub since_ts: Option<i64>,
    #[serde(default)]
    pub until_ts: Option<i64>,
    #[serde(default)]
    pub memory_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallAboutWrapper {
    pub args: MemoryRecallAboutArgs,
}

// ── memory.search ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemorySearchArgs {
    pub bot_id: String,
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: i64,
    #[serde(default)]
    pub since_ts: Option<i64>,
    #[serde(default)]
    pub until_ts: Option<i64>,
    #[serde(default)]
    pub memory_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemorySearchWrapper {
    pub args: MemorySearchArgs,
}

// ── memory.list ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryListArgs {
    pub bot_id: String,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub since_ts: Option<i64>,
    #[serde(default)]
    pub until_ts: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default = "default_offset")]
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryListWrapper {
    pub args: MemoryListArgs,
}

// ── memory.personality_get ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PersonalityGetArgs {
    pub bot_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PersonalityGetWrapper {
    pub args: PersonalityGetArgs,
}

// ── memory.personality_set ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PersonalitySetArgs {
    pub bot_id: String,
    /// Persona text (max 4000 chars enforced by the core service).
    pub persona: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PersonalitySetWrapper {
    pub args: PersonalitySetArgs,
}

// ── goals.create ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalCreateArgs {
    pub bot_id: String,
    pub text: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: i64,
    #[serde(default)]
    pub origin_memory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalCreateWrapper {
    pub args: GoalCreateArgs,
}

// ── goals.read ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalReadArgs {
    pub bot_id: String,
    pub goal_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalReadWrapper {
    pub args: GoalReadArgs,
}

// ── goals.update ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalUpdateArgs {
    pub bot_id: String,
    pub goal_id: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub origin_memory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalUpdateWrapper {
    pub args: GoalUpdateArgs,
}

// ── goals.list ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalListArgs {
    pub bot_id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default = "default_offset")]
    pub offset: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalListWrapper {
    pub args: GoalListArgs,
}

// ── goals.complete ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalCompleteArgs {
    pub bot_id: String,
    pub goal_id: String,
    /// Must be "completed" or "abandoned".
    pub outcome: GoalOutcomeEnum,
    #[serde(default = "default_also_record_memory")]
    pub also_record_memory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoalCompleteWrapper {
    pub args: GoalCompleteArgs,
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::Value;

    // Helper: check that `schema_for!(T)["properties"]["args"].is_object()`.
    macro_rules! assert_wrapper_has_args_envelope {
        ($wrapper:ty) => {{
            let schema = schemars::schema_for!($wrapper);
            let v: Value = serde_json::to_value(&schema).expect("schema serializes");
            let has_args = v
                .get("properties")
                .and_then(|p| p.get("args"))
                .map(|a| a.is_object())
                .unwrap_or(false);
            assert!(
                has_args,
                "{} JSON schema must have properties.args (brain contract)",
                stringify!($wrapper),
            );
        }};
    }

    /// For all 15 wrappers: `schema_for!(Wrapper)["properties"]["args"].is_object()`.
    #[test]
    fn all_15_wrappers_have_args_envelope() {
        use super::*;
        assert_wrapper_has_args_envelope!(MemoryWriteWrapper);
        assert_wrapper_has_args_envelope!(MemoryReadWrapper);
        assert_wrapper_has_args_envelope!(MemoryUpdateWrapper);
        assert_wrapper_has_args_envelope!(MemoryDeleteWrapper);
        assert_wrapper_has_args_envelope!(MemoryRecallWrapper);
        assert_wrapper_has_args_envelope!(MemoryRecallAboutWrapper);
        assert_wrapper_has_args_envelope!(MemorySearchWrapper);
        assert_wrapper_has_args_envelope!(MemoryListWrapper);
        assert_wrapper_has_args_envelope!(PersonalityGetWrapper);
        assert_wrapper_has_args_envelope!(PersonalitySetWrapper);
        assert_wrapper_has_args_envelope!(GoalCreateWrapper);
        assert_wrapper_has_args_envelope!(GoalReadWrapper);
        assert_wrapper_has_args_envelope!(GoalUpdateWrapper);
        assert_wrapper_has_args_envelope!(GoalListWrapper);
        assert_wrapper_has_args_envelope!(GoalCompleteWrapper);
    }

    /// After `transform_nullable_types`, no node in `MemoryRecallWrapper`'s schema
    /// should have `"type"` as a JSON array.
    #[test]
    fn recall_wrapper_has_no_type_arrays_after_transform() {
        use super::MemoryRecallWrapper;
        use crate::mcp::handler::transform_nullable_types;

        let raw_schema = schemars::schema_for!(MemoryRecallWrapper);
        let v: Value = serde_json::to_value(&raw_schema).unwrap();

        // Confirm raw schema HAS at least one nullable type array.
        fn has_array_type(v: &Value) -> bool {
            match v {
                Value::Object(m) => {
                    if let Some(t) = m.get("type") {
                        if t.is_array() { return true; }
                    }
                    m.values().any(has_array_type)
                }
                Value::Array(a) => a.iter().any(has_array_type),
                _ => false,
            }
        }
        assert!(
            has_array_type(&v),
            "MemoryRecallWrapper must have at least one nullable field in raw schema — \
             otherwise this test proves nothing",
        );

        // Apply the transform.
        let transformed = transform_nullable_types(v);

        // After transform: no type arrays anywhere (including $defs).
        fn has_no_array_type(v: &Value) -> bool {
            match v {
                Value::Object(m) => {
                    if let Some(t) = m.get("type") { if t.is_array() { return false; } }
                    m.values().all(has_no_array_type)
                }
                Value::Array(a) => a.iter().all(has_no_array_type),
                _ => true,
            }
        }
        assert!(
            has_no_array_type(&transformed),
            "After transform_nullable_types, no schema node may have type as a JSON array",
        );
    }
}
