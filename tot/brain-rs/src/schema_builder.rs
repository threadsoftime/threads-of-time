//! Build the LLM grammar + prompt summary from live MCP tools/list.
//!
//! Faithful Rust port of `brain_sidecar/schema_builder.py` (V3.1).
//! Parity is byte-critical: the composed `oneOf` and rendered summary feed
//! llama-server's json_schema grammar and the LLM prompt respectively.
//!
//! `serde_json` is compiled with `preserve_order` (IndexMap backend) so that
//! JSON serialization of [`compose_oneof`] output preserves insertion order,
//! matching Python's dict insertion order (guaranteed since Python 3.7).

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value};

// ---------------------------------------------------------------------------
// Bare-boolean subschema sanitizer
// ---------------------------------------------------------------------------

/// Convert a bare JSON Schema boolean shorthand into its object equivalent.
///
/// JSON Schema allows `true` (any value is valid) and `false` (no value is
/// valid) wherever a subschema is expected.  llama.cpp's json-schema→GBNF
/// compiler does **not** support this form and returns HTTP 400
/// `"Unrecognized schema: true"`.
///
/// - `true`  → `{}` (empty schema = allow anything)
/// - `false` → `{"not": {}}` (matches nothing)
fn bool_to_schema(b: bool) -> Value {
    if b {
        Value::Object(Map::new())
    } else {
        let mut m = Map::new();
        m.insert("not".to_string(), Value::Object(Map::new()));
        Value::Object(m)
    }
}

/// Recursively sanitize bare-boolean subschemas in a JSON Schema value.
///
/// JSON Schema permits `true`/`false` in positions where a subschema is
/// expected (RFC draft-07 §4.3.2).  llama.cpp's GBNF converter rejects
/// them with HTTP 400.  This function replaces them with their object
/// equivalents (`true → {}`, `false → {"not": {}}`) but **only** at
/// recognised subschema positions:
///
/// | Position kind | Keys |
/// |---|---|
/// | Single subschema | `items`, `additionalItems`, `contains`, `not`, `propertyNames`, `if`, `then`, `else` |
/// | Map of subschemas | `properties`, `patternProperties`, `$defs`, `definitions`, `dependentSchemas` |
/// | Array of subschemas | `allOf`, `anyOf`, `oneOf`, `prefixItems` |
///
/// Booleans at **non-subschema positions** are left untouched:
/// `additionalProperties` (llama.cpp handles it), `default`, `const`,
/// enum elements, `required` entries, `strict`, `readOnly`, etc.
pub fn sanitize_bool_schemas(node: &Value) -> Value {
    match node {
        Value::Object(obj) => {
            let mut out = Map::with_capacity(obj.len());
            for (k, v) in obj.iter() {
                let sanitized = match k.as_str() {
                    // ── Single-subschema positions ────────────────────────
                    "items"
                    | "additionalItems"
                    | "contains"
                    | "not"
                    | "propertyNames"
                    | "if"
                    | "then"
                    | "else" => match v {
                        Value::Bool(b) => bool_to_schema(*b),
                        other => sanitize_bool_schemas(other),
                    },
                    // ── Map-of-subschemas positions ───────────────────────
                    "properties"
                    | "patternProperties"
                    | "$defs"
                    | "definitions"
                    | "dependentSchemas" => {
                        if let Value::Object(map) = v {
                            let mut new_map = Map::with_capacity(map.len());
                            for (prop_k, prop_v) in map.iter() {
                                let sanitized_prop = match prop_v {
                                    Value::Bool(b) => bool_to_schema(*b),
                                    other => sanitize_bool_schemas(other),
                                };
                                new_map.insert(prop_k.clone(), sanitized_prop);
                            }
                            Value::Object(new_map)
                        } else {
                            sanitize_bool_schemas(v)
                        }
                    }
                    // ── Array-of-subschemas positions ─────────────────────
                    "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                        if let Value::Array(arr) = v {
                            let new_arr: Vec<Value> = arr
                                .iter()
                                .map(|elem| match elem {
                                    Value::Bool(b) => bool_to_schema(*b),
                                    other => sanitize_bool_schemas(other),
                                })
                                .collect();
                            Value::Array(new_arr)
                        } else {
                            sanitize_bool_schemas(v)
                        }
                    }
                    // ── Everything else: recurse structurally, no bool fix ─
                    _ => sanitize_bool_schemas(v),
                };
                out.insert(k.clone(), sanitized);
            }
            Value::Object(out)
        }
        Value::Array(arr) => {
            Value::Array(arr.iter().map(sanitize_bool_schemas).collect())
        }
        // Scalars pass through unchanged (includes booleans at non-schema positions)
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// $ref resolution
// ---------------------------------------------------------------------------

/// Walk a JSON schema tree, replacing every `{"$ref": "#/$defs/X"}` with
/// `defs["X"]`.
///
/// Cycle-safe via `stack` of currently-resolving def names.  Returns a new
/// value; does not mutate `node` in place.  All `$ref` pointers must use the
/// `#/$defs/` prefix — any other form is an error.
pub fn resolve_refs(
    node: &Value,
    defs: &Map<String, Value>,
    stack: &[&str],
) -> Result<Value, String> {
    match node {
        Value::Object(obj) => {
            if let Some(ref_val) = obj.get("$ref") {
                let ref_str = ref_val
                    .as_str()
                    .ok_or_else(|| format!("$ref value is not a string: {ref_val:?}"))?;
                let prefix = "#/$defs/";
                if !ref_str.starts_with(prefix) {
                    return Err(format!(
                        "cannot resolve unsupported $ref form: {ref_str:?} (expected #/$defs/...)"
                    ));
                }
                let def_name = &ref_str[prefix.len()..];
                if stack.contains(&def_name) {
                    return Err(format!(
                        "cannot resolve $ref {ref_str:?}: cycle detected (stack={stack:?})"
                    ));
                }
                let resolved = defs.get(def_name).ok_or_else(|| {
                    let available: Vec<&str> = defs.keys().map(|s| s.as_str()).collect();
                    format!(
                        "cannot resolve $ref {ref_str:?}: {def_name} not present in $defs \
                         (available: {available:?})"
                    )
                })?;
                // Extend the stack and recurse into the resolved def in case
                // it has nested $refs.
                let mut new_stack: Vec<&str> = stack.to_vec();
                new_stack.push(def_name);
                resolve_refs(resolved, defs, &new_stack)
            } else {
                // Walk dict values
                let mut out = Map::with_capacity(obj.len());
                for (k, v) in obj.iter() {
                    out.insert(k.clone(), resolve_refs(v, defs, stack)?);
                }
                Ok(Value::Object(out))
            }
        }
        Value::Array(arr) => {
            let out: Result<Vec<Value>, String> =
                arr.iter().map(|item| resolve_refs(item, defs, stack)).collect();
            Ok(Value::Array(out?))
        }
        // Scalars: return as-is (clone is cheap for bool/number/string/null)
        other => Ok(other.clone()),
    }
}

// ---------------------------------------------------------------------------
// FastMCP args unwrapping
// ---------------------------------------------------------------------------

/// Strip the FastMCP `{args: ...}` envelope and recursively resolve all `$ref`s.
///
/// FastMCP exposes tool handler signatures as `inputSchema` with `args` as the
/// sole top-level property; the real per-tool args live under `properties.args`
/// and may contain `$ref` pointers into `inputSchema["$defs"]`.
///
/// Steps:
/// 1. Pull `properties.args` (raises error if missing).
/// 2. Recursively walk the result, replacing every `{"$ref": "#/$defs/X"}`
///    with the actual `inputSchema["$defs"]["X"]` content.
///
/// The returned schema is fully self-contained — no `$ref` pointers remain.
pub fn unwrap_fastmcp_args(
    input_schema: &HashMap<String, Value>,
) -> Result<Value, String> {
    let props = match input_schema.get("properties") {
        Some(Value::Object(m)) => m,
        Some(other) => {
            return Err(format!(
                "inputSchema 'properties' is not an object: {other:?}"
            ))
        }
        None => {
            return Err(format!(
                "inputSchema missing required 'args' property: keys={:?}",
                input_schema.keys().collect::<Vec<_>>()
            ))
        }
    };

    let args_schema = props.get("args").ok_or_else(|| {
        let keys: Vec<&str> = props.keys().map(|s| s.as_str()).collect();
        format!("inputSchema missing required 'args' property: keys={keys:?}")
    })?;

    // Extract $defs (empty map if absent — some tools have no nested types)
    let empty_defs = Map::new();
    let defs = match input_schema.get("$defs") {
        Some(Value::Object(m)) => m,
        _ => &empty_defs,
    };

    let resolved = resolve_refs(args_schema, defs, &[])?;
    // Sanitize bare-boolean subschemas (`items: true`, property values of
    // `true`/`false`, etc.) that schemars emits for `Vec<Value>` and similar
    // types.  llama.cpp's json-schema→GBNF compiler rejects them with HTTP 400.
    Ok(sanitize_bool_schemas(&resolved))
}

// ---------------------------------------------------------------------------
// ToolEntry — one row in the per_tool map
// ---------------------------------------------------------------------------

/// One entry in the `per_tool` map: description + unwrapped schema + source MCP.
pub struct ToolEntry {
    pub description: String,
    /// Fully-resolved (no `$ref`) JSON schema for the tool's arguments.
    pub schema: Value,
    /// `"harness"` or `"memory"`.
    pub source_mcp: String,
}

// ---------------------------------------------------------------------------
// Decision metadata properties (matches Python _DECISION_METADATA_PROPS)
// ---------------------------------------------------------------------------

/// Build the three decision-metadata properties as a `serde_json::Map`,
/// preserving Python insertion order: `confidence`, `reasoning`, `wakeup_in_ms`.
fn decision_metadata_props() -> Map<String, Value> {
    let mut m = Map::new();
    // confidence: number in [0, 1]
    m.insert(
        "confidence".to_string(),
        serde_json::json!({"type": "number", "minimum": 0, "maximum": 1}),
    );
    // reasoning: string
    m.insert("reasoning".to_string(), serde_json::json!({"type": "string"}));
    // wakeup_in_ms: anyOf[integer 60000..600000, null] — optional, not in `required`
    m.insert(
        "wakeup_in_ms".to_string(),
        serde_json::json!({
            "anyOf": [
                {"type": "integer", "minimum": 60000, "maximum": 600000},
                {"type": "null"}
            ]
        }),
    );
    m
}

/// Build a `required` array: `["kind", "tool", "args", "confidence", "reasoning"]`.
fn required_fields() -> Value {
    Value::Array(vec![
        Value::String("kind".to_string()),
        Value::String("tool".to_string()),
        Value::String("args".to_string()),
        Value::String("confidence".to_string()),
        Value::String("reasoning".to_string()),
    ])
}

/// Build one discriminated-union branch object.
///
/// Key insertion order (matching Python):
/// `title` → `type` → `properties` → `required` → `additionalProperties`
fn make_branch(title: &str, properties: Map<String, Value>) -> Value {
    let mut branch = Map::new();
    branch.insert("title".to_string(), Value::String(title.to_string()));
    branch.insert("type".to_string(), Value::String("object".to_string()));
    branch.insert("properties".to_string(), Value::Object(properties));
    branch.insert("required".to_string(), required_fields());
    branch.insert("additionalProperties".to_string(), Value::Bool(false));
    Value::Object(branch)
}

// ---------------------------------------------------------------------------
// compose_oneof
// ---------------------------------------------------------------------------

/// Build the discriminated-union JSON schema covering the `Decision` shape.
///
/// Branch order (matching Python):
/// 1. `no_op` — kind=const "no_op", tool=null, args=null.
/// 2. One `action:<tool>` branch per entry in `per_tool`, **sorted by tool name**
///    (`BTreeMap` iteration is already sorted).
///
/// Every branch includes `_DECISION_METADATA_PROPS`:
/// `confidence`, `reasoning`, `wakeup_in_ms`.
pub fn compose_oneof(per_tool: &BTreeMap<String, ToolEntry>) -> Value {
    // no_op branch — properties in Python insertion order:
    // kind, tool, args, confidence, reasoning, wakeup_in_ms
    let mut no_op_props = Map::new();
    no_op_props.insert(
        "kind".to_string(),
        serde_json::json!({"const": "no_op"}),
    );
    no_op_props.insert("tool".to_string(), serde_json::json!({"type": "null"}));
    no_op_props.insert("args".to_string(), serde_json::json!({"type": "null"}));
    for (k, v) in decision_metadata_props() {
        no_op_props.insert(k, v);
    }

    let mut branches = vec![make_branch("no_op", no_op_props)];

    // Action branches — BTreeMap gives alphabetical order, matching Python's sorted()
    for (tool_name, entry) in per_tool.iter() {
        let mut action_props = Map::new();
        action_props.insert(
            "kind".to_string(),
            serde_json::json!({"const": "action"}),
        );
        action_props.insert(
            "tool".to_string(),
            serde_json::json!({"const": tool_name}),
        );
        action_props.insert("args".to_string(), entry.schema.clone());
        for (k, v) in decision_metadata_props() {
            action_props.insert(k, v);
        }
        let title = format!("action:{tool_name}");
        branches.push(make_branch(&title, action_props));
    }

    serde_json::json!({"oneOf": branches})
}

// ---------------------------------------------------------------------------
// render_prompt_summary
// ---------------------------------------------------------------------------

/// Map JSON Schema type strings to Python type names.
/// Matches Python's `_JSON_TO_PY_TYPE`.
fn json_to_py_type(json_type: &str) -> &'static str {
    match json_type {
        "integer" => "int",
        "string" => "str",
        "number" => "float",
        "boolean" => "bool",
        "object" => "dict",
        "array" => "list",
        "null" => "None",
        _ => "any",
    }
}

/// Render one argument as `name: type` or `name?: type` if optional.
///
/// Matches Python's `_render_arg`.
pub fn render_arg(name: &str, schema: &Value, required: bool) -> String {
    let py_type = schema
        .get("type")
        .and_then(|t| t.as_str())
        .map(json_to_py_type)
        .unwrap_or("any");
    let qmark = if required { "" } else { "?" };
    format!("{name}{qmark}: {py_type}")
}

/// Render the B-style human-readable tool list for the LLM prompt.
///
/// Output format per tool (matching Python):
/// ```text
///   tool.name(arg: int, opt_arg?: str)
///     — Tool description here
/// ```
/// Tools are sorted alphabetically by name (`BTreeMap` iteration).
/// If a tool has an empty description, the `— ...` line is omitted.
pub fn render_prompt_summary(per_tool: &BTreeMap<String, ToolEntry>) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (tool_name, entry) in per_tool.iter() {
        let props = entry.schema.get("properties").and_then(|p| p.as_object());
        let required: std::collections::HashSet<&str> = entry
            .schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let arg_strs: Vec<String> = props
            .map(|m| {
                m.iter()
                    .map(|(n, s)| render_arg(n, s, required.contains(n.as_str())))
                    .collect()
            })
            .unwrap_or_default();

        let sig = format!("  {tool_name}({})", arg_strs.join(", "));
        lines.push(sig);
        if !entry.description.is_empty() {
            lines.push(format!("    \u{2014} {}", entry.description));
        }
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// fetch_schemas — pull live tools/list from both MCPs
// ---------------------------------------------------------------------------

/// Pull tools/list from both MCPs, unwrap, and merge into one BTreeMap.
///
/// Mirrors Python `schema_builder.fetch_schemas(harness_mcp, memory_mcp)`.
///
/// Returns a `BTreeMap<tool_name, ToolEntry>` suitable for `compose_oneof` and
/// `render_prompt_summary`. Tools from `memory` override tools from `harness`
/// if names collide (matching Python dict update order).
///
/// Errors: propagates `anyhow::Error` from `list_tools()` on either MCP.
/// The caller (app lifespan) is responsible for `sys.exit(1)` on failure.
pub async fn fetch_schemas<H, M>(
    harness_mcp: &H,
    memory_mcp: &M,
) -> Result<BTreeMap<String, ToolEntry>, anyhow::Error>
where
    H: ListTools,
    M: ListTools,
{
    let harness_tools = harness_mcp.list_tools().await?;
    let memory_tools = memory_mcp.list_tools().await?;

    let mut per_tool: BTreeMap<String, ToolEntry> = BTreeMap::new();

    for tool in harness_tools {
        // input_schema is Arc<JsonObject> = Arc<serde_json::Map<String, Value>>
        let schema_map: HashMap<String, serde_json::Value> =
            tool.input_schema.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        match unwrap_fastmcp_args(&schema_map) {
            Ok(schema) => {
                per_tool.insert(
                    tool.name.to_string(),
                    ToolEntry {
                        description: tool.description.as_deref().unwrap_or("").to_string(),
                        schema,
                        source_mcp: "harness".to_string(),
                    },
                );
            }
            Err(e) => {
                tracing::warn!(
                    "fetch_schemas: skipping harness tool {} — unwrap_fastmcp_args failed: {}",
                    tool.name, e
                );
            }
        }
    }

    for tool in memory_tools {
        let schema_map: HashMap<String, serde_json::Value> =
            tool.input_schema.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        match unwrap_fastmcp_args(&schema_map) {
            Ok(schema) => {
                per_tool.insert(
                    tool.name.to_string(),
                    ToolEntry {
                        description: tool.description.as_deref().unwrap_or("").to_string(),
                        schema,
                        source_mcp: "memory".to_string(),
                    },
                );
            }
            Err(e) => {
                tracing::warn!(
                    "fetch_schemas: skipping memory tool {} — unwrap_fastmcp_args failed: {}",
                    tool.name, e
                );
            }
        }
    }

    Ok(per_tool)
}

/// Abstraction over MCP clients that can list tools.
/// Allows `fetch_schemas` to be called in tests with a mock.
///
/// Uses `BoxFuture` for object-safety (stable async-fn-in-traits are not
/// dyn-safe without this indirection on Rust stable as of 1.92).
pub trait ListTools: Send + Sync {
    fn list_tools(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<Vec<rmcp::model::Tool>>> + Send + '_>,
    >;
}

// Blanket impl for McpClient (which already has list_tools as an async fn)
impl ListTools for crate::mcp_client::McpClient {
    fn list_tools(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<Vec<rmcp::model::Tool>>> + Send + '_>,
    > {
        Box::pin(async move { self.list_tools().await })
    }
}

// ---------------------------------------------------------------------------
// Unit tests — sanitize_bool_schemas
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_items_true_to_empty_object() {
        let s = serde_json::json!({"type": "array", "items": true});
        let out = sanitize_bool_schemas(&s);
        assert_eq!(out["items"], serde_json::json!({}), "items:true -> {{}}");
    }

    #[test]
    fn sanitizes_property_value_bool() {
        let s = serde_json::json!({
            "type": "object",
            "properties": {"foo": true, "bar": {"type": "string"}}
        });
        let out = sanitize_bool_schemas(&s);
        assert_eq!(out["properties"]["foo"], serde_json::json!({}));
        assert_eq!(
            out["properties"]["bar"],
            serde_json::json!({"type": "string"})
        );
    }

    #[test]
    fn sanitizes_false_to_not_empty() {
        let s = serde_json::json!({"items": false});
        let out = sanitize_bool_schemas(&s);
        assert_eq!(out["items"], serde_json::json!({"not": {}}));
    }

    #[test]
    fn sanitizes_oneof_anyof_allof_elements() {
        let s = serde_json::json!({"oneOf": [true, {"type": "string"}]});
        let out = sanitize_bool_schemas(&s);
        assert_eq!(out["oneOf"][0], serde_json::json!({}));
        assert_eq!(out["oneOf"][1], serde_json::json!({"type": "string"}));
    }

    #[test]
    fn leaves_additional_properties_bool_untouched() {
        let s = serde_json::json!({"type": "object", "additionalProperties": false});
        let out = sanitize_bool_schemas(&s);
        assert_eq!(
            out["additionalProperties"],
            serde_json::json!(false),
            "additionalProperties bool is valid -> untouched"
        );
    }

    #[test]
    fn leaves_non_schema_booleans_untouched() {
        // strict/default/required/enum booleans are NOT subschemas.
        let s = serde_json::json!({
            "type": "object",
            "properties": {"x": {"type": "boolean", "default": true}},
            "required": ["x"]
        });
        let out = sanitize_bool_schemas(&s);
        assert_eq!(
            out["properties"]["x"]["default"],
            serde_json::json!(true),
            "a boolean default is not a subschema"
        );
        assert_eq!(out["properties"]["x"]["type"], serde_json::json!("boolean"));
    }

    #[test]
    fn nested_sanitization_recurses() {
        let s = serde_json::json!({"properties": {"a": {"type": "array", "items": true}}});
        let out = sanitize_bool_schemas(&s);
        assert_eq!(out["properties"]["a"]["items"], serde_json::json!({}));
    }

    /// Verify the exact `memory.write` `relations` arg shape that caused the
    /// production 400.  schemars emits `{"type":"array","items":true}` for
    /// `Vec<Value>`.  After `unwrap_fastmcp_args` the `items:true` must become
    /// `items:{}` so llama.cpp's GBNF compiler accepts the schema.
    #[test]
    fn pipeline_items_true_sanitized_via_unwrap_fastmcp_args() {
        // Simulate the schemars-generated inputSchema for a tool whose `args`
        // object has a `relations` field typed `Vec<Value>`.
        let input_schema: std::collections::HashMap<String, Value> =
            serde_json::from_value(serde_json::json!({
                "type": "object",
                "properties": {
                    "args": {
                        "type": "object",
                        "properties": {
                            "key": {"type": "string"},
                            "relations": {"type": "array", "items": true}
                        },
                        "required": ["key"]
                    }
                }
            }))
            .unwrap();

        let out = unwrap_fastmcp_args(&input_schema).unwrap();
        assert_eq!(
            out["properties"]["relations"]["items"],
            serde_json::json!({}),
            "items:true from schemars Vec<Value> must become {{}} after unwrap pipeline"
        );
        // Ensure non-schema booleans survive
        let schema_str = serde_json::to_string(&out).unwrap();
        assert!(
            !schema_str.contains(":true}") && !schema_str.contains(":false}"),
            "no bare-boolean subschemas must remain after sanitization: {schema_str}"
        );
    }
}
