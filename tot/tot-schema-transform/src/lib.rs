//! Shared MCP schema post-processing — pure `serde_json::Value` transforms.
//!
//! Extracted verbatim from harness-rs/memory-rs `mcp/handler.rs` (they were
//! byte-identical copies). `transform_nullable_types` rewrites JSON-Schema
//! `"type": ["string","null"]` arrays into `anyOf` branches (FastMCP/pydantic
//! parity); `strip_top_level_nulls` mirrors pydantic `exclude_none` at the
//! model's own level only.

use serde_json::{Map, Value};

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    /// Pydantic-v2 oracle: a field with `None` default is ABSENT (not `null`) after
    /// `model_dump(exclude_none=True)`, which is what `mcp_server.py:119` uses when
    /// serialising tool-call args.
    ///
    /// Oracle command (do NOT delete /tmp/parity-venv — leave in place):
    ///   python3 -m venv /tmp/parity-venv && /tmp/parity-venv/bin/pip install -q 'pydantic>=2,<3'
    ///   /tmp/parity-venv/bin/python -c '
    ///   import json
    ///   from pydantic import BaseModel
    ///   from typing import Optional
    ///   class M(BaseModel):
    ///       a: int = 1
    ///       b: Optional[str] = None
    ///   print(json.dumps(M().model_dump(exclude_none=True), separators=(",",":")))'
    ///   # -> {"a":1}    (key "b" is ABSENT, not present as null)
    #[test]
    fn strip_top_level_nulls_pydantic_oracle() {
        // Input: a JSON object with one non-null key and one null key,
        // mirroring pydantic model_dump() output before exclude_none.
        let input = json!({"a": 1, "b": null});
        let out = strip_top_level_nulls(input);
        // Pydantic oracle: {"a":1} — key "b" is absent, not null.
        assert_eq!(out, json!({"a": 1}), "null key must be absent (not present as null), matching pydantic exclude_none=True");
        assert!(out.get("b").is_none(), "key 'b' must not appear in output");
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
}
