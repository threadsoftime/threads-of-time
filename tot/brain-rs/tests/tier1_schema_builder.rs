// Tier-1 byte-parity: schema_builder functions must match Python output exactly.
// Fixture: tools_list_harness.json + tools_list_memory.json (captured from live harness-rs + memory-rs).

use brain_rs::schema_builder::{compose_oneof, render_prompt_summary, resolve_refs, unwrap_fastmcp_args, ToolEntry};
use std::collections::BTreeMap;

#[test]
fn test_unwrap_fastmcp_args_resolves_ref() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tools_list_harness.json")).unwrap();
    let obs_tool = &schema[0];
    let input_schema = obs_tool["inputSchema"].as_object().unwrap().clone();
    let input_schema: std::collections::HashMap<String, serde_json::Value> =
        input_schema.into_iter().collect();
    let unwrapped = unwrap_fastmcp_args(&input_schema).unwrap();
    // After unwrap+resolve: must have properties.target_guid, no $ref remaining
    let props = unwrapped["properties"].as_object().unwrap();
    assert!(
        props.contains_key("target_guid"),
        "expected target_guid after $ref resolve"
    );
    let as_str = serde_json::to_string(&unwrapped).unwrap();
    assert!(!as_str.contains("$ref"), "$ref must be fully resolved");
}

#[test]
fn test_unwrap_missing_args_property_returns_error() {
    let schema = std::collections::HashMap::from([
        (
            "type".to_string(),
            serde_json::Value::String("object".to_string()),
        ),
        (
            "properties".to_string(),
            serde_json::json!({"not_args": {}}),
        ),
    ]);
    assert!(unwrap_fastmcp_args(&schema).is_err());
}

#[test]
fn test_resolve_refs_nested_defs() {
    // Test that nested $ref within a resolved def is also resolved (recursive)
    let defs = {
        let mut m = serde_json::Map::new();
        m.insert(
            "Inner".to_string(),
            serde_json::json!({"type": "object", "properties": {"x": {"type": "integer"}}}),
        );
        m.insert(
            "Outer".to_string(),
            serde_json::json!({"type": "object", "properties": {"inner": {"$ref": "#/$defs/Inner"}}}),
        );
        m
    };
    let node = serde_json::json!({"$ref": "#/$defs/Outer"});
    let resolved = resolve_refs(&node, &defs, &[]).unwrap();
    let as_str = serde_json::to_string(&resolved).unwrap();
    assert!(!as_str.contains("$ref"), "nested $ref must be fully resolved");
    assert!(resolved["properties"]["inner"]["properties"]["x"]["type"] == "integer");
}

#[test]
fn test_resolve_refs_cycle_detection() {
    let defs = {
        let mut m = serde_json::Map::new();
        m.insert("A".to_string(), serde_json::json!({"$ref": "#/$defs/B"}));
        m.insert("B".to_string(), serde_json::json!({"$ref": "#/$defs/A"}));
        m
    };
    let node = serde_json::json!({"$ref": "#/$defs/A"});
    let result = resolve_refs(&node, &defs, &[]);
    assert!(result.is_err(), "cycle should return error");
    let msg = result.unwrap_err();
    assert!(msg.contains("cycle"), "error message should mention cycle: {msg}");
}

#[test]
fn test_resolve_refs_unsupported_ref_form() {
    let defs = serde_json::Map::new();
    let node = serde_json::json!({"$ref": "https://example.com/schema"});
    let result = resolve_refs(&node, &defs, &[]);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("unsupported"),
        "error should mention unsupported: {msg}"
    );
}

#[test]
fn test_resolve_refs_missing_def() {
    let defs = serde_json::Map::new();
    let node = serde_json::json!({"$ref": "#/$defs/Missing"});
    let result = resolve_refs(&node, &defs, &[]);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("Missing") || msg.contains("not present"),
        "error should identify missing def: {msg}"
    );
}

#[test]
fn test_compose_oneof_no_op_first_then_sorted_actions() {
    let mut per_tool: BTreeMap<String, ToolEntry> = BTreeMap::new();
    per_tool.insert(
        "bot.send_chat".to_string(),
        ToolEntry {
            description: "Send chat".to_string(),
            schema: serde_json::json!({"type": "object", "properties": {"bot_guid": {"type": "integer"}}, "required": ["bot_guid"]}),
            source_mcp: "harness".to_string(),
        },
    );
    per_tool.insert(
        "obs.ping".to_string(),
        ToolEntry {
            description: "Ping".to_string(),
            schema: serde_json::json!({"type": "object", "properties": {"target_guid": {"type": "integer"}}, "required": ["target_guid"]}),
            source_mcp: "harness".to_string(),
        },
    );
    let oneof = compose_oneof(&per_tool);
    let branches = oneof["oneOf"].as_array().unwrap();
    // First branch must be no_op
    assert_eq!(branches[0]["title"], "no_op");
    // Remaining branches sorted by tool name
    assert_eq!(branches[1]["title"], "action:bot.send_chat");
    assert_eq!(branches[2]["title"], "action:obs.ping");
    // no_op branch: kind const = "no_op", tool = null, args = null
    let no_op = &branches[0];
    assert_eq!(no_op["properties"]["kind"]["const"], "no_op");
    assert_eq!(no_op["properties"]["tool"]["type"], "null");
    assert_eq!(no_op["additionalProperties"], false);
}

#[test]
fn test_compose_oneof_action_branch_shape() {
    let mut per_tool: BTreeMap<String, ToolEntry> = BTreeMap::new();
    per_tool.insert(
        "bot.follow".to_string(),
        ToolEntry {
            description: "follow".to_string(),
            schema: serde_json::json!({"type": "object", "properties": {"bot_guid": {"type": "integer"}}, "required": ["bot_guid"]}),
            source_mcp: "harness".to_string(),
        },
    );
    let oneof = compose_oneof(&per_tool);
    let action = &oneof["oneOf"][1];
    assert_eq!(action["title"], "action:bot.follow");
    assert_eq!(action["properties"]["kind"]["const"], "action");
    assert_eq!(action["properties"]["tool"]["const"], "bot.follow");
    assert_eq!(action["additionalProperties"], false);
    // required contains kind, tool, args, confidence, reasoning
    let req = action["required"].as_array().unwrap();
    let req_strs: Vec<&str> = req.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(req_strs.contains(&"kind"));
    assert!(req_strs.contains(&"tool"));
    assert!(req_strs.contains(&"args"));
    assert!(req_strs.contains(&"confidence"));
    assert!(req_strs.contains(&"reasoning"));
}

#[test]
fn test_compose_oneof_wakeup_in_ms_anyof_in_metadata() {
    // wakeup_in_ms must be an anyOf[integer, null] in every branch
    let per_tool: BTreeMap<String, ToolEntry> = BTreeMap::new();
    let oneof = compose_oneof(&per_tool);
    let no_op = &oneof["oneOf"][0];
    let wakeup = &no_op["properties"]["wakeup_in_ms"];
    assert!(wakeup["anyOf"].is_array(), "wakeup_in_ms must be anyOf");
    let any_of = wakeup["anyOf"].as_array().unwrap();
    assert_eq!(any_of.len(), 2);
    // First element: integer with min/max
    assert_eq!(any_of[0]["type"], "integer");
    assert_eq!(any_of[0]["minimum"], 60000);
    assert_eq!(any_of[0]["maximum"], 600000);
    // Second element: null
    assert_eq!(any_of[1]["type"], "null");
}

#[test]
fn test_compose_oneof_empty_per_tool_has_only_no_op() {
    let per_tool: BTreeMap<String, ToolEntry> = BTreeMap::new();
    let oneof = compose_oneof(&per_tool);
    let branches = oneof["oneOf"].as_array().unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0]["title"], "no_op");
}

#[test]
fn test_render_prompt_summary_format() {
    let mut per_tool: BTreeMap<String, ToolEntry> = BTreeMap::new();
    per_tool.insert(
        "bot.follow".to_string(),
        ToolEntry {
            description: "follow another player; durable".to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "bot_guid": {"type": "integer"},
                    "leader_guid": {"type": "integer"}
                },
                "required": ["bot_guid", "leader_guid"]
            }),
            source_mcp: "harness".to_string(),
        },
    );
    let summary = render_prompt_summary(&per_tool);
    // Python output: "  bot.follow(bot_guid: int, leader_guid: int)\n    — follow another player; durable"
    assert!(summary.contains("  bot.follow(bot_guid: int, leader_guid: int)"));
    assert!(summary.contains("    \u{2014} follow another player; durable"));
}

#[test]
fn test_render_prompt_summary_empty_description_omits_dash_line() {
    let mut per_tool: BTreeMap<String, ToolEntry> = BTreeMap::new();
    per_tool.insert(
        "obs.ping".to_string(),
        ToolEntry {
            description: "".to_string(),
            schema: serde_json::json!({"type": "object", "properties": {}, "required": []}),
            source_mcp: "harness".to_string(),
        },
    );
    let summary = render_prompt_summary(&per_tool);
    assert!(summary.contains("  obs.ping()"));
    assert!(!summary.contains("\u{2014}"), "empty description must omit the dash line");
}

#[test]
fn test_render_prompt_summary_sorted_by_name() {
    let mut per_tool: BTreeMap<String, ToolEntry> = BTreeMap::new();
    per_tool.insert(
        "zzz.tool".to_string(),
        ToolEntry {
            description: "".to_string(),
            schema: serde_json::json!({"type": "object", "properties": {}, "required": []}),
            source_mcp: "harness".to_string(),
        },
    );
    per_tool.insert(
        "aaa.tool".to_string(),
        ToolEntry {
            description: "".to_string(),
            schema: serde_json::json!({"type": "object", "properties": {}, "required": []}),
            source_mcp: "harness".to_string(),
        },
    );
    let summary = render_prompt_summary(&per_tool);
    let aaa_pos = summary.find("aaa.tool").unwrap();
    let zzz_pos = summary.find("zzz.tool").unwrap();
    assert!(aaa_pos < zzz_pos, "aaa.tool should appear before zzz.tool");
}

#[test]
fn test_render_arg_optional_has_question_mark() {
    use brain_rs::schema_builder::render_arg;
    let schema = serde_json::json!({"type": "string"});
    assert_eq!(render_arg("name", &schema, false), "name?: str");
    assert_eq!(render_arg("name", &schema, true), "name: str");
}

#[test]
fn test_render_arg_type_mapping() {
    use brain_rs::schema_builder::render_arg;
    let cases = [
        ("integer", "int"),
        ("string", "str"),
        ("number", "float"),
        ("boolean", "bool"),
        ("object", "dict"),
        ("array", "list"),
        ("null", "None"),
    ];
    for (json_type, py_type) in cases {
        let schema = serde_json::json!({"type": json_type});
        let result = render_arg("x", &schema, true);
        assert_eq!(result, format!("x: {py_type}"), "type {json_type} should map to {py_type}");
    }
    // Unknown type maps to "any"
    let schema = serde_json::json!({"type": "unknown_type"});
    assert_eq!(render_arg("x", &schema, true), "x: any");
}
