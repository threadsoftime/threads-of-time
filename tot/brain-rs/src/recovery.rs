// SPDX-License-Identifier: AGPL-3.0
//! Production `PersonalityRecovery` impl + shared identity-from-obs helper.
//!
//! When `PersonalityCache::get()` finds an unusable persona (empty / blank /
//! null / unparseable) it calls `recover(bot_guid)`. `LiveRecovery` rebuilds a
//! real, identity-preserving card by mirroring the enroll path, layered by
//! identity fidelity (best → degraded), then morphs v2 fields:
//!
//!   1. state_store seed  — the authoritative enroll seed (real backstory +
//!      traits). Richest source, zero network calls. Used when its name/race/
//!      class are present and not bootstrap sentinels.
//!   2. obs.get_state     — live in-game name/race/class on a default backstory.
//!      Identical to the enroll path. Used when (1) is absent/sentinel.
//!   3. degraded generic  — a clearly-logged generic card. Only when both (1)
//!      and (2) are unavailable; never silent.
//!   4. morph             — fill v2 fields (itself soft-fails to seed values).
//!
//! The caller (`get()`) persists the returned card, so a bot heals permanently.

use std::sync::Arc;

use tracing::{info, warn};

use crate::llm_client::LlmClient;
use crate::mcp_client::McpClient;
use crate::models::PersonalityCard;
use crate::morph::morph_personality;
use crate::personality::PersonalityRecovery;
use crate::state::StateStore;

/// Identity sentinels that mean "not a real WoW identity" — produced by the
/// subset-gate bootstrap seed (app.rs enroll_via_api) before obs fills it in.
fn is_sentinel_identity(card: &PersonalityCard) -> bool {
    let blank = |s: &str| s.trim().is_empty();
    let unknown = |s: &str| {
        let t = s.trim();
        t.eq_ignore_ascii_case("unknown") || t == "?"
    };
    blank(&card.name)
        || unknown(&card.name)
        || blank(&card.race)
        || unknown(&card.race)
        || blank(&card.class_)
        || unknown(&card.class_)
}

/// A degraded, clearly-non-real placeholder card. Last resort only.
fn degraded_generic_card(bot_guid: i64) -> PersonalityCard {
    PersonalityCard {
        name: "Adventurer".to_string(),
        race: "Unknown".to_string(),
        class_: "Warrior".to_string(),
        backstory: "A wandering adventurer whose origins are unclear.".to_string(),
        talkativeness: 0.5,
        courage: 0.5,
        greed: 0.3,
        attitude_to_master: 0.0,
        party_invite_policy: "accept_from_known".to_string(),
        pvp_appetite: None,
        raid_appetite: None,
        completionist_streak: None,
        gold_motivation: None,
        profession_appetite: None,
    }
    .tap_log_degraded(bot_guid)
}

trait TapLog {
    fn tap_log_degraded(self, bot_guid: i64) -> Self;
}
impl TapLog for PersonalityCard {
    fn tap_log_degraded(self, bot_guid: i64) -> Self {
        warn!(
            "personality_recover bot_guid={bot_guid}: DEGRADED — no state_store \
             seed and obs.get_state identity unavailable; using generic placeholder \
             ('{}', '{}', '{}'). Identity will be corrected on next enroll/obs.",
            self.name, self.race, self.class_
        );
        self
    }
}

/// Best-effort fill of `card`'s name/race/class from `obs.get_state`.
///
/// Shared by enroll (app.rs) and `LiveRecovery`. 5 s timeout; on any
/// failure / missing field, leaves `card` untouched and logs a warn. Returns
/// `true` if all three identity fields were filled from obs.
pub async fn fill_identity_from_obs(
    harness_mcp: &McpClient,
    bot_guid: i64,
    card: &mut PersonalityCard,
) -> bool {
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        harness_mcp.call("obs.get_state", &serde_json::json!({"target_guid": bot_guid})),
    )
    .await
    {
        Ok(Ok(raw)) => {
            let obs = raw.get("result").cloned().unwrap_or(raw);
            let self_obj = obs
                .get("self")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let live_name = self_obj.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let live_race = self_obj.get("race").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let live_class = self_obj.get("class").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !live_name.is_empty() && !live_race.is_empty() && !live_class.is_empty() {
                card.name = live_name;
                card.race = live_race;
                card.class_ = live_class;
                true
            } else {
                warn!(
                    "fill_identity_from_obs bot_guid={bot_guid}: obs.get_state missing \
                     identity fields (name={live_name:?} race={live_race:?} class={live_class:?}); \
                     keeping current values"
                );
                false
            }
        }
        Ok(Err(e)) => {
            warn!("fill_identity_from_obs bot_guid={bot_guid}: obs.get_state failed ({e}); keeping current values");
            false
        }
        Err(_timeout) => {
            warn!("fill_identity_from_obs bot_guid={bot_guid}: obs.get_state timed out; keeping current values");
            false
        }
    }
}

/// Production self-heal: state_store seed → obs identity → degraded, then morph.
pub struct LiveRecovery {
    pub state_store: Arc<StateStore>,
    pub harness_mcp: Arc<McpClient>,
    pub llm_client: Arc<LlmClient>,
}

impl LiveRecovery {
    pub fn new(
        state_store: Arc<StateStore>,
        harness_mcp: Arc<McpClient>,
        llm_client: Arc<LlmClient>,
    ) -> Self {
        Self { state_store, harness_mcp, llm_client }
    }

    /// Build the identity-bearing card (pre-morph) using the layered sources.
    async fn build_identity_card(&self, bot_guid: i64) -> PersonalityCard {
        // Layer 1: state_store enroll seed (richest — real backstory + traits).
        if let Ok(Some(row)) = self.state_store.get_bot(bot_guid) {
            if !is_sentinel_identity(&row.personality_seed) {
                info!(
                    "personality_recover bot_guid={bot_guid}: using state_store seed identity \
                     ('{}', '{}', '{}')",
                    row.personality_seed.name, row.personality_seed.race, row.personality_seed.class_
                );
                return row.personality_seed;
            }
        }

        // Layer 2: obs.get_state identity on a default backstory.
        let mut card = degraded_base_card();
        if fill_identity_from_obs(&self.harness_mcp, bot_guid, &mut card).await {
            info!(
                "personality_recover bot_guid={bot_guid}: using obs.get_state identity \
                 ('{}', '{}', '{}')",
                card.name, card.race, card.class_
            );
            return card;
        }

        // Layer 3: degraded generic (logged inside).
        degraded_generic_card(bot_guid)
    }
}

/// A neutral card used as the obs-fill base (distinct from the degraded
/// placeholder so its log line is the obs path, not the degraded warn).
fn degraded_base_card() -> PersonalityCard {
    PersonalityCard {
        name: "Adventurer".to_string(),
        race: "Unknown".to_string(),
        class_: "Warrior".to_string(),
        backstory: "An adventurer encountered in the world.".to_string(),
        talkativeness: 0.5,
        courage: 0.5,
        greed: 0.3,
        attitude_to_master: 0.0,
        party_invite_policy: "accept_from_known".to_string(),
        pvp_appetite: None,
        raid_appetite: None,
        completionist_streak: None,
        gold_motivation: None,
        profession_appetite: None,
    }
}

impl PersonalityRecovery for LiveRecovery {
    fn recover<'a>(
        &'a self,
        bot_guid: i64,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<PersonalityCard, anyhow::Error>> + Send + 'a>,
    > {
        Box::pin(async move {
            let card = self.build_identity_card(bot_guid).await;
            // Always morph v2 fields (soft-fails to random seed on LLM error).
            let morphed = morph_personality(
                &card,
                &self.llm_client,
                None::<&mut rand::rngs::StdRng>,
            )
            .await;
            Ok(morphed)
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PersonalityCard;

    fn real_card(name: &str, race: &str, class_: &str) -> PersonalityCard {
        PersonalityCard {
            name: name.into(),
            race: race.into(),
            class_: class_.into(),
            backstory: "bs".into(),
            talkativeness: 0.3,
            courage: 0.9,
            greed: 0.5,
            attitude_to_master: 0.0,
            party_invite_policy: "accept_from_known".into(),
            pvp_appetite: None,
            raid_appetite: None,
            completionist_streak: None,
            gold_motivation: None,
            profession_appetite: None,
        }
    }

    #[test]
    fn sentinel_identity_detects_unknown_and_blank() {
        assert!(is_sentinel_identity(&real_card("", "Night Elf", "Warrior")));
        assert!(is_sentinel_identity(&real_card("Morenette", "Unknown", "Warrior")));
        assert!(is_sentinel_identity(&real_card("Morenette", "Night Elf", "?")));
        assert!(is_sentinel_identity(&real_card("  ", "Night Elf", "Warrior")));
        assert!(is_sentinel_identity(&real_card("Morenette", "unknown", "Warrior"))); // case-insensitive
    }

    #[test]
    fn real_identity_not_sentinel() {
        assert!(!is_sentinel_identity(&real_card("Morenette", "Night Elf", "Warrior")));
    }

    #[test]
    fn degraded_card_is_valid_and_generic() {
        let c = degraded_generic_card(1083);
        assert_eq!(c.name, "Adventurer");
        assert_eq!(c.race, "Unknown");
        // Serializable + reparseable (a valid PersonalityCard, just generic).
        let s = serde_json::to_string(&c).unwrap();
        let back: PersonalityCard = serde_json::from_str(&s).unwrap();
        assert_eq!(back.name, "Adventurer");
    }
}
