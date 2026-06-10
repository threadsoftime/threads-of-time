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

/// A second, distinct personality card — different name/race/class/traits — used
/// to prove the SYSTEM prompt does NOT vary by persona (prefix-cache contract).
fn fixture_card_b() -> PersonalityCard {
    PersonalityCard {
        name: "Grimna".into(),
        race: "Orc".into(),
        class_: "Warrior".into(),
        backstory: "A grizzled veteran of countless battles.".into(),
        talkativeness: 0.2,
        courage: 1.0,
        greed: 0.9,
        attitude_to_master: -1.0,
        party_invite_policy: "accept_all".into(),
        pvp_appetite: Some(0.95),
        raid_appetite: Some(0.1),
        completionist_streak: Some(0.0),
        gold_motivation: Some(0.8),
        profession_appetite: Some(0.2),
    }
}

/// PREFIX-CACHE CONTRACT (load-bearing guard for Slice A):
/// `prompt.system` MUST be byte-identical for every bot AND every tick, so that
/// llama.cpp's prefix cache (which reuses the KV of the longest common prefix)
/// hits across the whole fleet. Here we assemble prompts for TWO distinct cards
/// × TWO triage reasons (None, organic_wakeup) with different per-bot/per-tick
/// state, and assert all four `prompt.system` strings are EQUAL.
#[test]
fn system_prompt_is_byte_identical_across_bots_and_ticks() {
    // Card A: bot 1001, no triage reason, one state shape.
    let decider_a = Decider::new_test(1001, fixture_card(), "test_template");
    let prompt_a1 = decider_a.assemble_prompt_test(
        &fixture_card(),
        &serde_json::json!({"self": {"level": 12, "in_group": false}}),
        &[serde_json::json!({"text": "reach level 25"})],
        &[serde_json::json!({"text": "talked to Alice"})],
        &[],
        &HashMap::new(),
        1001,
        None,
    );
    // Card A again, but organic_wakeup tick with different state.
    let prompt_a2 = decider_a.assemble_prompt_test(
        &fixture_card(),
        &serde_json::json!({"self": {"level": 13, "in_group": true}}),
        &[],
        &[],
        &[],
        &HashMap::new(),
        1001,
        Some("organic_wakeup"),
    );

    // Card B: a different bot (1002), distinct persona, distinct state.
    let decider_b = Decider::new_test(1002, fixture_card_b(), "test_template");
    let prompt_b1 = decider_b.assemble_prompt_test(
        &fixture_card_b(),
        &serde_json::json!({"self": {"level": 25, "in_group": false}}),
        &[serde_json::json!({"text": "win 10 battlegrounds"})],
        &[],
        &[],
        &HashMap::new(),
        1002,
        None,
    );
    let prompt_b2 = decider_b.assemble_prompt_test(
        &fixture_card_b(),
        &serde_json::json!({"self": {"level": 25, "in_group": true}}),
        &[],
        &[serde_json::json!({"text": "fought a murloc"})],
        &[],
        &HashMap::new(),
        1002,
        Some("organic_wakeup"),
    );

    // All four SYSTEM prompts must be byte-identical — the shared fleet prefix.
    assert_eq!(
        prompt_a1.system, prompt_a2.system,
        "system must not vary across ticks for the same bot"
    );
    assert_eq!(
        prompt_a1.system, prompt_b1.system,
        "system must not vary across bots (persona must be in USER, not SYSTEM)"
    );
    assert_eq!(
        prompt_a1.system, prompt_b2.system,
        "system must be byte-identical for all bots and all triage reasons"
    );

    // Sanity: the USER prompts SHOULD differ (they carry the per-bot persona).
    assert_ne!(
        prompt_a1.user, prompt_b1.user,
        "user prompts must differ between distinct personas"
    );
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
    // Slice A: persona (name/guid/class) moved to the USER message (per-bot).
    assert!(prompt.user.contains("Kael"), "user must contain bot name");
    assert!(prompt.user.contains("1001"), "user must contain bot_guid");
    assert!(prompt.user.contains("Paladin"), "user must contain class");
    // Persona must NOT leak into the invariant system block.
    assert!(!prompt.system.contains("Kael"), "system must not contain per-bot name");
    // The schema literal stays in the invariant SYSTEM block.
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
    // Slice A: at-cap paragraph moved to the USER message.
    assert!(prompt.user.contains("at max level"), "at-cap paragraph missing");
    assert!(
        prompt.user.contains("pvp_appetite"),
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
    // Slice A: at-cap paragraph lives in USER; absent there when below cap.
    assert!(
        !prompt.user.contains("at max level"),
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
    // Slice A: organic-wakeup paragraph moved to the USER message (per-tick).
    assert!(
        prompt.user.contains("woke up on your own"),
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
fn test_truncate_memory_items_clips_text_at_200() {
    let decider = Decider::new_test(1001, fixture_card(), "t");
    let long_text = "x".repeat(400);
    let items = vec![serde_json::json!({"text": long_text, "id": "abc"})];
    let truncated = decider.truncate_memory_items_test(&items);
    assert!(truncated[0]["text"].as_str().unwrap().ends_with('…'));
    // 200 chars + "…" (1 char, 3 UTF-8 bytes) = 201 chars total
    assert_eq!(
        truncated[0]["text"].as_str().unwrap().chars().count(),
        201
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
    // 68 entries (62 base + 6 V3 gameplay tools promoted 2026-06-09; Python brain retired)
    assert_eq!(KNOWN_TOOLS.len(), 68, "must have 68 KNOWN_TOOLS entries (62 base + 6 V3 gameplay tools promoted 2026-06-09)");
}

#[test]
fn test_schema_literal_exact() {
    // Byte-exact match with Python _DECISION_SCHEMA_LITERAL
    let expected = r#"{"kind": "action"|"no_op", "tool": "<tool_name>"|null, "args": {<tool-specific>}|null, "confidence": 0.0..1.0, "reasoning": "<one sentence>", "wakeup_in_ms": <60000..600000>}"#;
    assert_eq!(_DECISION_SCHEMA_LITERAL, expected, "_DECISION_SCHEMA_LITERAL must match Python exactly");
}

/// Tier-1 (Slice A): the few-shot EXAMPLES (with `{{`/`}}` escaping) moved from the
/// USER template (decide_v1.txt) into the INVARIANT SYSTEM template (decide_system_v2.txt).
///
/// This locks in parity with Python str.format(): the LLM sees clean JSON examples
/// like `{"kind": "action"}`, not corrupted `{{"kind": "action"}}` — and now in the
/// SYSTEM message. The trimmed USER template must contain NO brace artifacts at all.
#[test]
fn test_system_prompt_examples_unescaped_and_user_clean() {
    let card = fixture_card();
    // Use real templates: new_test wires the real decide_system_v2.txt for system,
    // and we pass the real decide_v1.txt for the user template.
    let user_template_path = concat!(env!("CARGO_MANIFEST_DIR"), "/prompts/decide_v1.txt");
    let user_tmpl = match std::fs::read_to_string(user_template_path) {
        Ok(s) => s,
        Err(_) => return, // template not reachable in this build env — skip
    };
    let decider = Decider::new_test(1001, card.clone(), &user_tmpl);
    let prompt = decider.assemble_prompt_test(
        &card,
        &serde_json::json!({}),
        &[],
        &[],
        &[],
        &HashMap::new(),
        1001,
        None,
    );

    // --- SYSTEM: carries the few-shot examples, unescaped by python_format(). ---
    // After python_format(), ALL `{{` escape sequences must be unescaped to `{`.
    assert!(
        !prompt.system.contains("{{"),
        "system prompt must not contain '{{' — decide_system_v2.txt double-open-braces must be unescaped"
    );
    // `}}` (two adjacent `}`) can legitimately remain — the few-shot examples use
    // `}}}}` for nested JSON closes; Python .format() reduces `}}}}` → `}}`.
    // The examples block is unchanged from the old decide_v1.txt, so the count is 4.
    let double_close_count = prompt.system.match_indices("}}").count();
    assert_eq!(
        double_close_count, 4,
        "system prompt must have exactly 4 remaining '}}' sequences matching Python .format() output, got {double_close_count}"
    );
    // The system must contain clean JSON example fragments (single-brace).
    assert!(
        prompt.system.contains(r#"{"party_invite_received""#)
            || prompt.system.contains(r#"{"kind": "action""#),
        "system prompt must contain unescaped JSON example fragments"
    );
    // The {schema_literal} placeholder must be resolved in system.
    assert!(
        !prompt.system.contains("{schema_literal}"),
        "system must not contain unresolved {{schema_literal}} placeholder"
    );
    assert!(
        prompt.system.contains(_DECISION_SCHEMA_LITERAL),
        "system must contain the resolved schema literal"
    );

    // --- USER: trimmed variable block — no brace artifacts, no unresolved vars. ---
    assert!(
        !prompt.user.contains("{{"),
        "user prompt must not contain '{{'"
    );
    assert_eq!(
        prompt.user.match_indices("}}").count(),
        0,
        "trimmed user prompt must have zero '}}' sequences"
    );
    for placeholder in [
        "{state_json}", "{goals_json}",
        "{memories_json}", "{recent_decisions_json}", "{hot_inputs_json}",
    ] {
        assert!(
            !prompt.user.contains(placeholder),
            "user prompt must not contain unresolved placeholder: {placeholder}"
        );
    }
    // tools_summary no longer belongs to the user template.
    assert!(
        !prompt.user.contains("{tools_summary}"),
        "user prompt must not reference tools_summary"
    );
    // Guard: the user header "YOU ARE:" must never drift into the system block.
    assert!(
        !prompt.system.contains("YOU ARE:"),
        "system prompt must not contain user-section header 'YOU ARE:'"
    );
}
