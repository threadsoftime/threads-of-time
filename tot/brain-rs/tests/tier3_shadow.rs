//! Tier-3 shadow-compare harness.
//!
//! Pre-cutover gate for the LLM-nondeterminism wrinkle (spec §6 Tier-3). Replays a
//! fixture recorded from the LIVE Python brain (Task 15.2) and asserts, on real
//! recorded ticks, that brain-rs is byte-faithful on the two deterministic seams
//! that ride live inputs:
//!   1. SYSTEM prompt — `Decider::assemble_prompt_test(...).system` must equal the
//!      Python-produced system prompt. (USER-prompt parity is already gated by the
//!      Tier-1 suite `tier1_prompt_assembly.rs` with controlled fixtures; the system
//!      prompt depends only on card/state/triage/bot_guid, so we reconstruct it from
//!      the record with empty goal/memory/decision slices — those feed only the user
//!      prompt.)
//!   2. DECISION parse — parsing the recorded raw LLM response must yield the same
//!      Decision the Python brain produced (happy path = `serde_json::from_str`;
//!      parse failure = the terminal no_op fallback from `decide.py`).
//!
//! `#[ignore]` in normal CI; run manually pre-cutover:
//!   cargo test -p brain-rs --test tier3_shadow -- --ignored --nocapture
//! Fixture (Task 15.2): tests/fixtures/shadow_replay.jsonl, one ShadowRecord per line.
//! Gate: zero mismatches across all records.

use brain_rs::decide::Decider;
use brain_rs::models::{Decision, DecisionKind, PersonalityCard};
use serde::Deserialize;
use std::collections::HashMap;

/// One recorded live tick from the Python brain (emitted by Task 15.2's recorder).
#[derive(Debug, Deserialize)]
struct ShadowRecord {
    bot_guid: i64,
    card: PersonalityCard,
    /// The `state_summary` dict fed to `_assemble_prompt` (drives at-cap framing).
    state: serde_json::Value,
    /// e.g. "fresh_chat" | "organic_wakeup" | "combat" | null.
    triage_reason: Option<String>,
    /// BRAIN_MAX_PLAYER_LEVEL in effect when recorded (drives at_cap).
    max_player_level: u32,
    /// Raw `choices[0].message.content` string the LLM returned.
    llm_response_raw: String,
    /// The system prompt the Python brain produced for this tick.
    expected_system_prompt: String,
    /// The Decision the Python brain ended up with after its parse/retry ladder.
    expected_decision: Decision,
}

/// Reproduce `decide.py`'s terminal parse outcome for a recorded raw response.
/// Recorded successful ticks carry valid JSON; the retry-with-restatement step is
/// not reproducible against a fixed recording (it re-queries the LLM), so a hard
/// parse failure maps straight to the terminal no_op the ladder would emit.
fn parse_decision_like_python(raw: &str) -> Decision {
    match serde_json::from_str::<Decision>(raw) {
        Ok(d) if d.validate().is_ok() => d,
        Ok(_) => Decision {
            kind: DecisionKind::NoOp, tool: None, args: None,
            confidence: 0.0, reasoning: "llm_invalid_schema".to_string(),
            wakeup_in_ms: None,
        },
        Err(_) => {
            let snippet: String = raw.chars().take(120).collect();
            Decision {
                kind: DecisionKind::NoOp, tool: None, args: None,
                confidence: 0.0, reasoning: format!("llm_unparseable: {snippet}"),
                wakeup_in_ms: None,
            }
        }
    }
}

#[ignore]
#[test]
fn test_tier3_shadow_compare() {
    let fixture_path = std::path::Path::new("tests/fixtures/shadow_replay.jsonl");
    if !fixture_path.exists() {
        eprintln!("SKIP: shadow fixture absent — generate it with Task 15.2's recorder");
        return;
    }
    let raw = std::fs::read_to_string(fixture_path).expect("read fixture");
    let mut total = 0usize;
    let mut mismatches: Vec<String> = Vec::new();

    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue; // skip the recorder's header/comment lines
        }
        let rec: ShadowRecord = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("record {i} parse: {e}\nline: {line}"));
        total += 1;

        // (1) System-prompt parity. goals/memories/recent_decisions/hot_inputs do
        // not affect the SYSTEM prompt, so empty slices are faithful here (the
        // element types are inferred from assemble_prompt_test's signature).
        let decider = Decider::new_test_with_max_level(
            rec.bot_guid, rec.card.clone(), "", rec.max_player_level,
        );
        let prompt = decider.assemble_prompt_test(
            &rec.card, &rec.state, &[], &[], &[],
            &HashMap::new(), rec.bot_guid, rec.triage_reason.as_deref(),
        );
        if prompt.system != rec.expected_system_prompt {
            mismatches.push(format!(
                "record {i} (bot {}): SYSTEM prompt differs\n  expected: {:?}\n  got:      {:?}",
                rec.bot_guid, rec.expected_system_prompt, prompt.system,
            ));
        }

        // (2) Decision-parse parity on the recorded raw response.
        let got = parse_decision_like_python(&rec.llm_response_raw);
        let exp = &rec.expected_decision;
        if got.kind != exp.kind || got.tool != exp.tool || got.args != exp.args
            || (got.confidence - exp.confidence).abs() > 1e-9
            || got.reasoning != exp.reasoning || got.wakeup_in_ms != exp.wakeup_in_ms
        {
            mismatches.push(format!(
                "record {i} (bot {}): DECISION differs\n  expected: {exp:?}\n  got:      {got:?}",
                rec.bot_guid,
            ));
        }
    }

    eprintln!("tier3 shadow: {total} records compared, {} mismatches", mismatches.len());
    assert!(mismatches.is_empty(), "Tier-3 shadow mismatches:\n{}", mismatches.join("\n"));
}
