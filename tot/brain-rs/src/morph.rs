// SPDX-License-Identifier: AGPL-3.0
//! V3.6: morph PersonalityCard v2 fields via LLM, with random-seed fallback.
//!
//! Faithful Rust port of `brain_sidecar/morph.py`.
//!
//! The morph runs whenever PersonalityCard has any None v2 field, in two paths:
//!   (1) /enroll — fresh bot, v2 fields default to None on construction;
//!   (2) PersonalityCache.get() migration — V1 persona JSON loaded from
//!       memory-sidecar deserializes with v2 fields = None.
//!
//! Random seed values in [0.3, 0.8] per scalar are generated FIRST. The LLM is
//! asked to morph these based on the bot's backstory. On any LLM failure
//! (timeout, unparseable, schema-invalid, exception), the seed values are kept
//! and the card is returned without raising — callers must not fail enroll or
//! decide on a stale LLM.

use std::collections::HashMap;

use rand::Rng;
use serde_json::{json, Value};
use tracing::warn;

use crate::llm_client::LlmClient;
use crate::models::PersonalityCard;
use crate::personality::needs_morph;

// ---------------------------------------------------------------------------
// V2 field names (matches morph.py _V2_FIELD_NAMES)
// ---------------------------------------------------------------------------

pub const V2_FIELD_NAMES: &[&str] = &[
    "pvp_appetite",
    "raid_appetite",
    "completionist_streak",
    "gold_motivation",
    "profession_appetite",
];

const SEED_MIN: f64 = 0.3;
const SEED_MAX: f64 = 0.8;

// ---------------------------------------------------------------------------
// Prompts (verbatim from morph.py)
// ---------------------------------------------------------------------------

const MORPH_SYSTEM_PROMPT: &str = "You assign end-game activity preferences for a World of Warcraft bot \
character based on its backstory. Return ONLY a JSON object with \
exactly five floats in [0.0, 1.0]: pvp_appetite, raid_appetite, \
completionist_streak, gold_motivation, profession_appetite. No prose.";

const MORPH_USER_TEMPLATE: &str = "Character backstory:\n{backstory}\n\n\
Random seed values (use as starting point; morph based on backstory):\n\
{seed_json}\n\n\
What are this character's actual end-game preferences? Respond with JSON only.";

// ---------------------------------------------------------------------------
// JSON schema for the LLM response (verbatim from morph.py)
// ---------------------------------------------------------------------------

fn morph_json_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pvp_appetite":         {"type": "number", "minimum": 0.0, "maximum": 1.0},
            "raid_appetite":        {"type": "number", "minimum": 0.0, "maximum": 1.0},
            "completionist_streak": {"type": "number", "minimum": 0.0, "maximum": 1.0},
            "gold_motivation":      {"type": "number", "minimum": 0.0, "maximum": 1.0},
            "profession_appetite":  {"type": "number", "minimum": 0.0, "maximum": 1.0},
        },
        "required": ["pvp_appetite", "raid_appetite", "completionist_streak", "gold_motivation", "profession_appetite"],
        "additionalProperties": false,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate random seed values in [SEED_MIN, SEED_MAX] for each v2 field.
/// Mirrors Python `_seed_values(rng)`.
pub fn seed_values(rng: &mut impl Rng) -> HashMap<String, f64> {
    V2_FIELD_NAMES
        .iter()
        .map(|name| (name.to_string(), rng.gen_range(SEED_MIN..=SEED_MAX)))
        .collect()
}

/// Return a NEW card with v2 fields filled (LLM-morphed from random seed).
///
/// No-op if card has no None v2 fields. On LLM failure (timeout, unparseable,
/// schema-invalid, or exception): seed values are kept (returned card has all
/// v2 fields populated, just with the random seed not the LLM output). Input
/// card is not mutated.
///
/// Mirrors Python `morph_personality(card, llm_client, *, rng=None)`.
pub async fn morph_personality(
    card: &PersonalityCard,
    llm_client: &LlmClient,
    rng: Option<&mut impl Rng>,
) -> PersonalityCard {
    if !needs_morph(card) {
        return card.clone();
    }

    // Build seed values — use caller-supplied rng or create a fresh thread_rng.
    let seed: HashMap<String, f64> = if let Some(r) = rng {
        seed_values(r)
    } else {
        seed_values(&mut rand::thread_rng())
    };

    let seed_json =
        serde_json::to_string_pretty(&seed).unwrap_or_else(|_| "{}".to_string());
    let user_prompt = MORPH_USER_TEMPLATE
        .replace("{backstory}", &card.backstory)
        .replace("{seed_json}", &seed_json);

    let schema = morph_json_schema();
    let result = llm_client
        .chat_completion_json(
            MORPH_SYSTEM_PROMPT,
            &user_prompt,
            200,
            0.7,
            Some(&schema),
        )
        .await;

    match result {
        Err(e) => {
            warn!(
                "morph_personality LLM exception bot={} err={}; keeping seed values",
                card.name, e
            );
            apply_v2_fields(card.clone(), &seed)
        }
        Ok((None, raw, _)) => {
            warn!(
                "morph_personality LLM unparseable bot={} raw={:?}; keeping seed values",
                card.name,
                &raw[..raw.len().min(120)]
            );
            apply_v2_fields(card.clone(), &seed)
        }
        Ok((Some(parsed), _, _)) => {
            // LLM happy path: trust the parsed dict; json_schema enforced strict shape.
            apply_v2_fields_from_json(card.clone(), &parsed)
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Apply v2 field values from a HashMap<String, f64> to a cloned card.
fn apply_v2_fields(mut card: PersonalityCard, values: &HashMap<String, f64>) -> PersonalityCard {
    if let Some(&v) = values.get("pvp_appetite") {
        card.pvp_appetite = Some(v);
    }
    if let Some(&v) = values.get("raid_appetite") {
        card.raid_appetite = Some(v);
    }
    if let Some(&v) = values.get("completionist_streak") {
        card.completionist_streak = Some(v);
    }
    if let Some(&v) = values.get("gold_motivation") {
        card.gold_motivation = Some(v);
    }
    if let Some(&v) = values.get("profession_appetite") {
        card.profession_appetite = Some(v);
    }
    card
}

/// Apply v2 field values from a parsed JSON Value (LLM response) to a cloned card.
fn apply_v2_fields_from_json(mut card: PersonalityCard, parsed: &Value) -> PersonalityCard {
    if let Some(v) = parsed.get("pvp_appetite").and_then(|v| v.as_f64()) {
        card.pvp_appetite = Some(v);
    }
    if let Some(v) = parsed.get("raid_appetite").and_then(|v| v.as_f64()) {
        card.raid_appetite = Some(v);
    }
    if let Some(v) = parsed.get("completionist_streak").and_then(|v| v.as_f64()) {
        card.completionist_streak = Some(v);
    }
    if let Some(v) = parsed.get("gold_motivation").and_then(|v| v.as_f64()) {
        card.gold_motivation = Some(v);
    }
    if let Some(v) = parsed.get("profession_appetite").and_then(|v| v.as_f64()) {
        card.profession_appetite = Some(v);
    }
    card
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router, response::IntoResponse};
    use rand::{rngs::StdRng, SeedableRng};

    fn card_with_all_v2() -> PersonalityCard {
        PersonalityCard {
            name: "Kael".into(),
            race: "Blood Elf".into(),
            class_: "Paladin".into(),
            backstory: "A noble warrior.".into(),
            talkativeness: 0.7,
            courage: 0.8,
            greed: 0.3,
            attitude_to_master: 0.0,
            party_invite_policy: "accept_from_known".into(),
            pvp_appetite: Some(0.5),
            raid_appetite: Some(0.5),
            completionist_streak: Some(0.5),
            gold_motivation: Some(0.5),
            profession_appetite: Some(0.5),
        }
    }

    fn card_with_no_v2() -> PersonalityCard {
        PersonalityCard {
            pvp_appetite: None,
            raid_appetite: None,
            completionist_streak: None,
            gold_motivation: None,
            profession_appetite: None,
            ..card_with_all_v2()
        }
    }

    async fn spawn_mock_llm(response_body: serde_json::Value) -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || async move { Json(response_body.clone()).into_response() }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    async fn spawn_mock_llm_500() -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "error").into_response() }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    #[test]
    fn test_needs_morph_returns_false_when_all_v2_filled() {
        let card = card_with_all_v2();
        assert!(!needs_morph(&card));
    }

    #[test]
    fn test_needs_morph_returns_true_when_any_v2_none() {
        let card = card_with_no_v2();
        assert!(needs_morph(&card));

        // Partial fill also triggers
        let mut partial = card_with_all_v2();
        partial.pvp_appetite = None;
        assert!(needs_morph(&partial));
    }

    #[test]
    fn test_seed_values_in_range() {
        let mut rng = StdRng::seed_from_u64(42);
        let vals = seed_values(&mut rng);
        assert_eq!(vals.len(), 5);
        for (name, v) in &vals {
            assert!(
                *v >= SEED_MIN && *v <= SEED_MAX,
                "field {} value {} out of [0.3, 0.8]",
                name, v
            );
        }
    }

    #[test]
    fn test_seed_values_deterministic_with_seeded_rng() {
        let mut rng1 = StdRng::seed_from_u64(12345);
        let mut rng2 = StdRng::seed_from_u64(12345);
        let vals1 = seed_values(&mut rng1);
        let vals2 = seed_values(&mut rng2);
        // Same seed → same values
        for name in V2_FIELD_NAMES {
            assert_eq!(vals1[*name], vals2[*name]);
        }
    }

    #[tokio::test]
    async fn test_morph_no_op_when_all_v2_filled() {
        let card = card_with_all_v2();
        let llm = LlmClient {
            base_url: "http://unused".into(),
            model: "test".into(),
            timeout_s: 5.0,
        };
        let result = morph_personality(&card, &llm, None::<&mut StdRng>).await;
        // All v2 fields unchanged
        assert_eq!(result.pvp_appetite, card.pvp_appetite);
        assert_eq!(result.raid_appetite, card.raid_appetite);
    }

    #[tokio::test]
    async fn test_morph_fills_all_v2_fields_from_llm() {
        let card = card_with_no_v2();
        let llm_body = serde_json::json!({
            "choices": [{"message": {"content": "{\"pvp_appetite\":0.4,\"raid_appetite\":0.6,\"completionist_streak\":0.7,\"gold_motivation\":0.3,\"profession_appetite\":0.5}"}}]
        });
        let base_url = spawn_mock_llm(llm_body).await;
        let llm = LlmClient { base_url, model: "test".into(), timeout_s: 5.0 };
        let mut rng = StdRng::seed_from_u64(99);
        let result = morph_personality(&card, &llm, Some(&mut rng)).await;
        // LLM values used
        assert_eq!(result.pvp_appetite, Some(0.4));
        assert_eq!(result.raid_appetite, Some(0.6));
        assert_eq!(result.completionist_streak, Some(0.7));
        assert_eq!(result.gold_motivation, Some(0.3));
        assert_eq!(result.profession_appetite, Some(0.5));
    }

    #[tokio::test]
    async fn test_morph_falls_back_to_seed_on_llm_failure() {
        let card = card_with_no_v2();
        let base_url = spawn_mock_llm_500().await;
        let llm = LlmClient { base_url, model: "test".into(), timeout_s: 5.0 };
        let mut rng = StdRng::seed_from_u64(42);
        let expected_seed = seed_values(&mut StdRng::seed_from_u64(42));
        let result = morph_personality(&card, &llm, Some(&mut rng)).await;
        // v2 fields are set (to seed values), no panic
        assert!(result.pvp_appetite.is_some());
        assert!(result.raid_appetite.is_some());
        assert!(result.completionist_streak.is_some());
        assert!(result.gold_motivation.is_some());
        assert!(result.profession_appetite.is_some());
        // Values match expected seed
        assert_eq!(result.pvp_appetite, Some(expected_seed["pvp_appetite"]));
    }

    #[tokio::test]
    async fn test_morph_falls_back_to_seed_on_unparseable_llm() {
        let card = card_with_no_v2();
        let llm_body = serde_json::json!({
            "choices": [{"message": {"content": "not json at all"}}]
        });
        let base_url = spawn_mock_llm(llm_body).await;
        let llm = LlmClient { base_url, model: "test".into(), timeout_s: 5.0 };
        let result = morph_personality(&card, &llm, None::<&mut StdRng>).await;
        // Still gets v2 fields (from seed fallback)
        assert!(result.pvp_appetite.is_some());
        assert!(result.profession_appetite.is_some());
    }
}
