// SPDX-License-Identifier: GPL-2.0-or-later
//! Exponential time-decay for episodes (design subspec §7 / §6.5).
//!
//! w(Δt) = 2^(-age_hours / half_life_hours)
//! age ≤ 0 (future timestamp) → 1.0   (clock-skew clamp)

/// Return the half-life in hours for a given episode type.
///
/// Unknown types fall back to `"chat"` (48 h) — the shortest sensible default
/// so stale unknown-type episodes age out rather than pinning at the top of
/// recall results.
pub fn half_life_for(episode_type: &str) -> f64 {
    match episode_type.to_lowercase().as_str() {
        "chat"        => 48.0,
        "combat"      => 72.0,
        "social"      => 96.0,
        "quest"       => 168.0,
        "discovery"   => 720.0,
        "goal"        => 168.0,
        "reflection"  => 336.0,
        "observation" => 24.0,
        _             => 48.0, // fallback = chat
    }
}

/// Exponential decay multiplier in (0, 1].
///
/// * `episode_time_ms` — event time as unix epoch milliseconds.
/// * `now_ms`          — current wall-clock as unix epoch milliseconds.
/// * `half_life_hours` — must be > 0; panics in debug, clamps to 1.0 in release.
///
/// Returns `2^(-age_hours / half_life_hours)` clamped so that future timestamps
/// (negative age) cannot produce a value > 1.0.
pub fn decay_weight(episode_time_ms: i64, now_ms: i64, half_life_hours: f64) -> f64 {
    if half_life_hours <= 0.0 {
        // Unreachable in production (all half-lives are positive constants).
        // Return 1.0 as a safe clamp rather than panicking in release builds.
        return 1.0;
    }
    let age_ms = now_ms - episode_time_ms;
    if age_ms <= 0 {
        return 1.0;
    }
    let age_hours = age_ms as f64 / 3_600_000.0;
    0.5_f64.powf(age_hours / half_life_hours)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── half_life_for ────────────────────────────────────────────────────────

    #[test]
    fn half_life_chat_is_48() {
        assert_eq!(half_life_for("chat"), 48.0);
    }

    #[test]
    fn half_life_combat_is_72() {
        assert_eq!(half_life_for("combat"), 72.0);
    }

    #[test]
    fn half_life_social_is_96() {
        assert_eq!(half_life_for("social"), 96.0);
    }

    #[test]
    fn half_life_quest_is_168() {
        assert_eq!(half_life_for("quest"), 168.0);
    }

    #[test]
    fn half_life_discovery_is_720() {
        assert_eq!(half_life_for("discovery"), 720.0);
    }

    #[test]
    fn half_life_goal_is_168() {
        assert_eq!(half_life_for("goal"), 168.0);
    }

    #[test]
    fn half_life_reflection_is_336() {
        assert_eq!(half_life_for("reflection"), 336.0);
    }

    #[test]
    fn half_life_observation_is_24() {
        assert_eq!(half_life_for("observation"), 24.0);
    }

    #[test]
    fn half_life_unknown_type_falls_back_to_48() {
        assert_eq!(half_life_for("not-a-real-type"), 48.0);
        assert_eq!(half_life_for(""), 48.0);
    }

    // ── decay_weight ─────────────────────────────────────────────────────────

    /// Fresh episode (age = 0) → weight 1.0 exactly.
    #[test]
    fn decay_weight_is_1_at_zero_age() {
        let now_ms: i64 = 1_748_000_000_000;
        assert_eq!(decay_weight(now_ms, now_ms, 24.0), 1.0);
    }

    /// Δt = one half-life → weight 0.5 (the defining property).
    #[test]
    fn decay_weight_is_half_at_one_half_life() {
        let now_ms: i64 = 1_748_000_000_000;
        let half_life_h = 24.0_f64;
        let ep_ms = now_ms - (half_life_h * 3_600_000.0) as i64;
        let w = decay_weight(ep_ms, now_ms, half_life_h);
        assert!((w - 0.5).abs() < 1e-9, "expected 0.5, got {w}");
    }

    /// Δt = 2 × half-life → weight 0.25.
    #[test]
    fn decay_weight_quarter_at_two_half_lives() {
        let now_ms: i64 = 1_748_000_000_000;
        let half_life_h = 24.0_f64;
        let ep_ms = now_ms - (2.0 * half_life_h * 3_600_000.0) as i64;
        let w = decay_weight(ep_ms, now_ms, half_life_h);
        assert!((w - 0.25).abs() < 1e-9, "expected 0.25, got {w}");
    }

    /// Future episode timestamp (clock skew) → weight clamped to 1.0.
    #[test]
    fn decay_weight_future_timestamp_clamps_to_1() {
        let now_ms: i64 = 1_748_000_000_000;
        let future_ms = now_ms + 3_600_000; // 1 hour in the future
        assert_eq!(decay_weight(future_ms, now_ms, 24.0), 1.0);
    }

    /// Verify each type's half-life round-trips through decay_weight correctly.
    #[test]
    fn decay_weight_per_type_half_life_round_trip() {
        let now_ms: i64 = 1_748_000_000_000;
        for (etype, expected_hl) in &[
            ("chat", 48.0_f64),
            ("combat", 72.0),
            ("social", 96.0),
            ("quest", 168.0),
            ("discovery", 720.0),
            ("goal", 168.0),
            ("reflection", 336.0),
            ("observation", 24.0),
        ] {
            let hl = half_life_for(etype);
            assert_eq!(hl, *expected_hl, "half_life_for({etype})");
            let ep_ms = now_ms - (hl * 3_600_000.0) as i64;
            let w = decay_weight(ep_ms, now_ms, hl);
            assert!(
                (w - 0.5).abs() < 1e-9,
                "type={etype}: expected 0.5 at one half-life, got {w}"
            );
        }
    }

    /// half_life_hours <= 0 → safe clamp (1.0), not a panic.
    #[test]
    fn decay_weight_zero_half_life_returns_one() {
        // Contract: unreachable in production; safe clamp rather than panic.
        let now_ms: i64 = 1_748_000_000_000;
        assert_eq!(decay_weight(now_ms - 1000, now_ms, 0.0), 1.0);
    }
}
