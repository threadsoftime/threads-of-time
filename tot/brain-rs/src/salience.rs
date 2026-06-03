/// Salience scorer — faithful Rust port of brain_sidecar/salience.py.
///
/// Decides which ticks produce a written memory episode. The score is a hint
/// passed to memory.write as `salience_hint`. The threshold gates episode
/// writes: `score >= threshold` means "worth remembering."
///
/// Tiers and per-type defaults mirror salience.py exactly (Tier-1 parity).
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Salience tiers — design subspec §8.2
// ---------------------------------------------------------------------------

pub const SALIENCE_PIVOTAL: f64   = 1.0;
pub const SALIENCE_IMPORTANT: f64 = 0.8;
pub const SALIENCE_NOTABLE: f64   = 0.6;
pub const SALIENCE_NORMAL: f64    = 0.45;
pub const SALIENCE_FILLER: f64    = 0.2;
pub const SALIENCE_TRIVIAL: f64   = 0.05;

fn type_defaults() -> HashMap<&'static str, f64> {
    let mut m = HashMap::new();
    m.insert("chat",        SALIENCE_NORMAL);
    m.insert("combat",      SALIENCE_NORMAL);
    m.insert("social",      SALIENCE_NOTABLE);
    m.insert("quest",       SALIENCE_IMPORTANT);
    m.insert("discovery",   SALIENCE_NOTABLE);
    m.insert("goal",        SALIENCE_IMPORTANT);
    m.insert("reflection",  SALIENCE_IMPORTANT);
    m.insert("observation", SALIENCE_FILLER);
    m
}

/// Rule-based scorer with optional brain-emitted override hint.
///
/// `threshold`: episodes with `score(...) >= threshold` are written to memory.
/// Default 0.3 — observation (FILLER=0.2) is filtered out, chat (NORMAL=0.45)
/// and everything stronger is kept.
#[derive(Debug, Clone)]
pub struct SalienceScorer {
    pub threshold: f64,
    /// Per-type default scores. Defaults mirror `_TYPE_DEFAULT` in salience.py.
    pub type_defaults: HashMap<String, f64>,
}

impl Default for SalienceScorer {
    fn default() -> Self {
        let type_defaults = type_defaults()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        Self { threshold: 0.3, type_defaults }
    }
}

impl SalienceScorer {
    /// Compute the salience score for a single tick.
    ///
    /// Matches salience.py `score()` exactly — same order of operations,
    /// same `round(min(1.0, max(0.0, score)), 3)` terminal transform.
    pub fn score(
        &self,
        perception: &serde_json::Value,
        _decision: &serde_json::Value,
        action_result: &serde_json::Value,
    ) -> f64 {
        // Brain override takes precedence (subspec §8.1, §10.1).
        if let Some(hint_val) = perception.get("salience_hint") {
            if let Some(hint) = hint_val.as_f64() {
                if (0.0..=1.0).contains(&hint) {
                    return round3(hint);
                }
            }
        }

        let episode_type = perception
            .get("episode_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let base = self
            .type_defaults
            .get(episode_type)
            .copied()
            .unwrap_or(SALIENCE_NORMAL);
        let mut score = base;

        // Combat: outcome-aware promotions (subspec §8.3).
        if episode_type == "combat" {
            let outcome = action_result
                .get("outcome")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if outcome == "kill" {
                score = f64::max(score, SALIENCE_IMPORTANT);
            } else if outcome == "near_death" {
                score = f64::max(score, SALIENCE_NOTABLE);
            } else if outcome == "wipe" {
                score = SALIENCE_IMPORTANT;
            }
        }

        // Quest: completion is the most salient event (subspec §8.3).
        if episode_type == "quest" {
            let event = action_result
                .get("event")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if event == "complete" {
                score = f64::max(score, SALIENCE_IMPORTANT);
            } else if event == "fail" {
                score = f64::max(score, SALIENCE_NOTABLE);
            }
        }

        // Goal: completion is pivotal (subspec §8.3).
        if episode_type == "goal" {
            let event = action_result
                .get("event")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if event == "completed" || event == "achieved" {
                score = SALIENCE_PIVOTAL;
            }
        }

        // Chat: from-a-player is more salient than from-an-NPC (subspec §8.3).
        if episode_type == "chat" {
            let source = perception
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if source == "player" {
                score = f64::max(score, SALIENCE_NOTABLE);
            }
        }

        round3(score.min(1.0).max(0.0))
    }
}

/// Round to 3 decimal places — mirrors Python's `round(x, 3)`.
fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_observation_is_filler() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "observation"}),
            &serde_json::json!({}),
            &serde_json::json!({}),
        );
        assert_eq!(s, 0.2); // FILLER
    }

    #[test]
    fn test_combat_kill_promotes_to_important() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "combat"}),
            &serde_json::json!({}),
            &serde_json::json!({"outcome": "kill"}),
        );
        assert_eq!(s, 0.8); // IMPORTANT
    }

    #[test]
    fn test_goal_achieved_is_pivotal() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "goal"}),
            &serde_json::json!({}),
            &serde_json::json!({"event": "achieved"}),
        );
        assert_eq!(s, 1.0); // PIVOTAL
    }

    #[test]
    fn test_salience_hint_overrides_all_rules() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "observation", "salience_hint": 0.99}),
            &serde_json::json!({}),
            &serde_json::json!({}),
        );
        assert_eq!(s, 0.99);
    }

    #[test]
    fn test_chat_from_player_is_notable() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "chat", "source": "player"}),
            &serde_json::json!({}),
            &serde_json::json!({}),
        );
        assert_eq!(s, 0.6); // NOTABLE (max(NORMAL=0.45, NOTABLE=0.6))
    }

    #[test]
    fn test_threshold_default_is_0_3() {
        let scorer = SalienceScorer::default();
        assert_eq!(scorer.threshold, 0.3);
    }

    // Additional Tier-1 parity checks
    #[test]
    fn test_combat_near_death_is_notable() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "combat"}),
            &serde_json::json!({}),
            &serde_json::json!({"outcome": "near_death"}),
        );
        assert_eq!(s, 0.6); // NOTABLE
    }

    #[test]
    fn test_combat_wipe_is_important() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "combat"}),
            &serde_json::json!({}),
            &serde_json::json!({"outcome": "wipe"}),
        );
        assert_eq!(s, 0.8); // IMPORTANT
    }

    #[test]
    fn test_quest_complete_is_important() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "quest"}),
            &serde_json::json!({}),
            &serde_json::json!({"event": "complete"}),
        );
        assert_eq!(s, 0.8); // IMPORTANT (max(base=IMPORTANT, IMPORTANT))
    }

    #[test]
    fn test_quest_fail_is_notable() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "quest"}),
            &serde_json::json!({}),
            &serde_json::json!({"event": "fail"}),
        );
        // base=IMPORTANT=0.8, max(0.8, NOTABLE=0.6) = 0.8
        assert_eq!(s, 0.8);
    }

    #[test]
    fn test_goal_completed_is_pivotal() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "goal"}),
            &serde_json::json!({}),
            &serde_json::json!({"event": "completed"}),
        );
        assert_eq!(s, 1.0); // PIVOTAL
    }

    #[test]
    fn test_social_is_notable_by_default() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "social"}),
            &serde_json::json!({}),
            &serde_json::json!({}),
        );
        assert_eq!(s, 0.6); // NOTABLE
    }

    #[test]
    fn test_unknown_type_defaults_to_normal() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "unknown_novel_type"}),
            &serde_json::json!({}),
            &serde_json::json!({}),
        );
        assert_eq!(s, 0.45); // NORMAL
    }

    #[test]
    fn test_hint_out_of_range_ignored() {
        let scorer = SalienceScorer::default();
        // hint=-0.1 is out of range — falls through to type-based scoring
        let s = scorer.score(
            &serde_json::json!({"episode_type": "observation", "salience_hint": -0.1}),
            &serde_json::json!({}),
            &serde_json::json!({}),
        );
        assert_eq!(s, 0.2); // FILLER — hint was ignored
    }

    #[test]
    fn test_hint_zero_is_valid() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"salience_hint": 0.0}),
            &serde_json::json!({}),
            &serde_json::json!({}),
        );
        assert_eq!(s, 0.0);
    }

    #[test]
    fn test_combat_no_outcome_stays_normal() {
        let scorer = SalienceScorer::default();
        let s = scorer.score(
            &serde_json::json!({"episode_type": "combat"}),
            &serde_json::json!({}),
            &serde_json::json!({}),
        );
        assert_eq!(s, 0.45); // NORMAL
    }

    /// Tier-1 exact-value parity: verify Rust round3() matches Python round(x, 3)
    /// for the one fractional tier value (NORMAL=0.45).
    #[test]
    fn test_tier1_normal_round3_exact() {
        // Python: round(0.45, 3) == 0.45. Must be identical in Rust.
        assert_eq!(round3(0.45), 0.45_f64);
        assert_eq!(round3(0.8), 0.8_f64);
        assert_eq!(round3(1.0), 1.0_f64);
        assert_eq!(round3(0.0), 0.0_f64);
    }
}
