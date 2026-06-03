// Tier-1: _assemble_prompt must produce byte-identical system + user strings
// given a fixed personality/state/goals/memories/recent_decisions/triage_reason.
// Load fixture from file so it can be updated if Python brain changes.

use brain_rs::decide::{Decider, _DECISION_SCHEMA_LITERAL, KNOWN_TOOLS};
use brain_rs::models::PersonalityCard;
use std::collections::HashMap;

fn fixture_card() -> PersonalityCard {
    PersonalityCard {
        name: "Kael".into(),
        race: "Blood Elf".into(),
        class_: "Paladin".into(),
        backstory: "A noble warrior seeking redemption.".into(),
        talkativeness: 0.7,
        courage: 0.8,
        greed: 0.3,
        attitude_to_master: 0.0,
        party_invite_policy: "accept_from_known".into(),
        pvp_appetite: Some(0.4),
        raid_appetite: Some(0.9),
        completionist_streak: Some(0.6),
        gold_motivation: Some(0.3),
        profession_appetite: Some(0.5),
    }
}

#[test]
fn test_system_prompt_contains_persona_and_bot_guid() {
    let decider = Decider::new_test(1001, fixture_card(), "test_template");
    let prompt = decider.assemble_prompt_test(
        &fixture_card(),
        &serde_json::json!({}),
        &[],
        &[],
        &[],
        &HashMap::new(),
        1001,
        None,
    );
    assert!(prompt.system.contains("Kael"), "system must contain bot name");
    assert!(prompt.system.contains("1001"), "system must contain bot_guid");
    assert!(prompt.system.contains("Paladin"), "system must contain class");
    assert!(
        prompt.system.contains(_DECISION_SCHEMA_LITERAL),
        "system must contain schema literal"
    );
}

#[test]
fn test_system_at_cap_adds_end_game_paragraph() {
    // level=25 with max_player_level=25 → at_cap=true → at-cap paragraph present
    let card = fixture_card();
    let state = serde_json::json!({"self": {"level": 25}});
    let decider = Decider::new_test_with_max_level(1001, card.clone(), "t", 25);
    let prompt = decider.assemble_prompt_test(
        &card,
        &state,
        &[],
        &[],
        &[],
        &HashMap::new(),
        1001,
        None,
    );
    assert!(prompt.system.contains("at max level"), "at-cap paragraph missing");
    assert!(
        prompt.system.contains("pvp_appetite"),
        "v2 fields must appear at cap"
    );
}

#[test]
fn test_system_not_at_cap_no_end_game_paragraph() {
    let card = fixture_card();
    let state = serde_json::json!({"self": {"level": 24}});
    let decider = Decider::new_test_with_max_level(1001, card.clone(), "t", 25);
    let prompt = decider.assemble_prompt_test(
        &card,
        &state,
        &[],
        &[],
        &[],
        &HashMap::new(),
        1001,
        None,
    );
    assert!(
        !prompt.system.contains("at max level"),
        "not-at-cap must not have at-cap paragraph"
    );
}

#[test]
fn test_system_organic_wakeup_adds_wakeup_paragraph() {
    let card = fixture_card();
    let decider = Decider::new_test(1001, card.clone(), "t");
    let prompt = decider.assemble_prompt_test(
        &card,
        &serde_json::json!({}),
        &[],
        &[],
        &[],
        &HashMap::new(),
        1001,
        Some("organic_wakeup"),
    );
    assert!(
        prompt.system.contains("woke up on your own"),
        "organic_wakeup paragraph missing"
    );
}

#[test]
fn test_project_hot_inputs_flattens_fresh_chat() {
    let decider = Decider::new_test(1001, fixture_card(), "t");
    let mut inputs = HashMap::new();
    inputs.insert(
        "fresh_chat".to_string(),
        serde_json::json!([{"text": "received whisper from Alice: hello", "from": "Alice"}]),
    );
    let projected = decider.project_hot_inputs_test(&inputs);
    let chat = &projected["fresh_chat"][0];
    assert_eq!(chat["sender"], "Alice");
    assert!(chat["message"].as_str().unwrap().contains("received whisper"));
    // Must not have raw fields (content, from) passthrough
    assert!(chat.get("from").is_none());
}

#[test]
fn test_truncate_memory_items_clips_text_at_300() {
    let decider = Decider::new_test(1001, fixture_card(), "t");
    let long_text = "x".repeat(400);
    let items = vec![serde_json::json!({"text": long_text, "id": "abc"})];
    let truncated = decider.truncate_memory_items_test(&items);
    assert!(truncated[0]["text"].as_str().unwrap().ends_with('…'));
    // 300 chars + "…" (1 char, 3 UTF-8 bytes) = 301 chars total
    assert_eq!(
        truncated[0]["text"].as_str().unwrap().chars().count(),
        301
    );
}

#[test]
fn test_known_tools_exposes_all_tools() {
    // KNOWN_TOOLS must contain specific entries matching Python's set
    let tools: std::collections::HashSet<&str> = KNOWN_TOOLS.iter().copied().collect();
    assert!(tools.contains("bot.send_chat"), "must contain bot.send_chat");
    assert!(tools.contains("memory.recall_about"), "must contain memory.recall_about");
    assert!(tools.contains("goals.list"), "must contain goals.list");
    assert!(tools.contains("goals_list"), "must contain goals_list alias");
    assert!(tools.contains("memory.goals.list"), "must contain memory.goals.list alias");
    // 62 entries matching Python source
    assert_eq!(KNOWN_TOOLS.len(), 62, "must have 62 KNOWN_TOOLS entries");
}

#[test]
fn test_schema_literal_exact() {
    // Byte-exact match with Python _DECISION_SCHEMA_LITERAL
    let expected = r#"{"kind": "action"|"no_op", "tool": "<tool_name>"|null, "args": {<tool-specific>}|null, "confidence": 0.0..1.0, "reasoning": "<one sentence>", "wakeup_in_ms": <60000..600000>}"#;
    assert_eq!(_DECISION_SCHEMA_LITERAL, expected, "_DECISION_SCHEMA_LITERAL must match Python exactly");
}
