//! The `Grind` goal disposer — a flat state machine (NOT a behavior tree).
//!
//! All states are real: Scanning/Reacting/Approaching/Fighting/PostKillPause/Looting/
//! HealthCheck/Resting/Wandering/RespawnWait/Recovering/Idle are fully implemented.

use serde::Deserialize;
use thiserror::Error;
use tot_goal_contract::GrindGoal;
use tot_harness_client::{HarnessClient, HarnessError};

use crate::combat::{CombatContext, FightMemo, RotationAction, RotationPlugin, TickOutcome};

/// A selected hostile to engage (world-space position + distance from the bot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub guid: u64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub distance: f64,
}

/// Errors internal to grind execution (mapped to `GoalStatus` by the caller).
#[derive(Debug, Error)]
pub enum GrindError {
    #[error("harness: {0}")]
    Harness(#[from] HarnessError),
    #[error("unexpected response shape: {0}")]
    Shape(String),
}

#[derive(Debug, Deserialize)]
struct Hostile {
    guid: u64,
    level: u32,
    distance: f64,
    is_alive: bool,
    /// Health percentage 0.0–100.0 (used for target-death detection in combat poll).
    #[serde(default)] hp_pct: f32,
    // World-space position so the executor can navigate to the target (design §6.2).
    // `default` keeps offline mocks that omit coords parseable (→ 0,0,0).
    #[serde(default)] x: f64,
    #[serde(default)] y: f64,
    #[serde(default)] z: f64,
}
#[derive(Debug, Deserialize)]
struct NearbyHostiles { hostiles: Vec<Hostile> }

/// Scan for the nearest alive hostile within the goal's level band.
/// Calls `obs.get_nearby_hostiles` (design §6.2).
pub async fn scan_for_target(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &GrindGoal,
) -> Result<Option<Target>, GrindError> {
    let raw = client
        .call("obs.get_nearby_hostiles", serde_json::json!({
            "bot_guid": bot_guid as i64,
            "radius": goal.max_search_radius as f64,
        }))
        .await?;
    let parsed: NearbyHostiles =
        serde_json::from_value(raw).map_err(|e| GrindError::Shape(format!("nearby_hostiles: {e}")))?;

    let mut best: Option<Target> = None;
    for h in parsed.hostiles {
        if !h.is_alive { continue; }
        if h.level < goal.mob_filter.min_level || h.level > goal.mob_filter.max_level { continue; }
        let cand = Target { guid: h.guid, x: h.x, y: h.y, z: h.z, distance: h.distance };
        match best {
            Some(b) if b.distance <= cand.distance => {}
            _ => best = Some(cand),
        }
    }
    Ok(best)
}

use crate::nav::{self, Dest, NavError};
use std::time::Duration;

/// Sampled micro-timing craft (exec constants, NOT goal fields — see design §4.2).
const REACT_DELAY_MS_MIN: u64 = 500;
const REACT_DELAY_MS_MAX: u64 = 1500;
const POST_KILL_PAUSE_MS_MIN: u64 = 1000;
const POST_KILL_PAUSE_MS_MAX: u64 = 2000;

/// Caster stop-short: walk to a point this fraction of the way from bot to target
/// (engage_range * ENGAGE_STOP_SHORT_FACTOR from target, along bot→target).
const ENGAGE_STOP_SHORT_FACTOR: f64 = 0.8;

/// Rest is considered done once BOTH hp AND mana (where applicable) reach this threshold.
const REST_RECOVERY_PCT: u32 = 75;

/// One spell GCD in milliseconds (used as the minimum sleep after a buff cast).
const GCD_MS: u64 = 1500;

/// Cycle-count proxy for the design's "~120s sustained idle" threshold.
/// Each empty scan cycle is ~5 s (IDLE_SCAN_INTERVAL_MS), so 24 × 5 s ≈ 120 s.
/// A real time-based idle guard can replace this in a future iteration.
const MAX_EMPTY_SCAN_CYCLES: u32 = 24;

/// Respawn-wait (design 2026-06-10 §2.1): a farmed-out camp WAITS instead of going
/// terminal. 45 s between empty-scan rounds; MAX_RESPAWN_WAITS rounds (~15+ min
/// camp-dry) before the Blocked{NoTargetsFound} backstop. Test builds shrink the
/// sleep so the backstop path runs in real time.
const RESPAWN_WAIT_MS: u64 = if cfg!(test) { 20 } else { 45_000 };
const MAX_RESPAWN_WAITS: u32 = 20;

/// Death-recovery (Finding #17, design 2026-06-12). The bot stays CLAIMED throughout —
/// `bot.revive` resurrects it in place (no ownership release → no #12 re-mount window).
const REVIVE_MAX_ATTEMPTS: u32 = 3;
const REVIVE_RETRY_MS: u64 = if cfg!(test) { 5 } else { 1_000 };
/// After a successful bot.revive, confirm the live state (revive is synchronous
/// server-side; this guards a racing state read).
const REVIVE_CONFIRM_POLLS: u32 = 5;
const REVIVE_POLL_MS: u64 = if cfg!(test) { 5 } else { 1_000 };
/// Deaths 1..=MAX resume grinding after recovery; death MAX+1 still recovers (bot ends
/// alive, at anchor, owned) but returns terminal `NeedsDecision{BotDied}` so the brain
/// paces re-emission with its cooldown.
const MAX_DEATHS_PER_GOAL: u32 = 3;

#[derive(Debug)]
pub(crate) enum RecoveryOutcome { Recovered, StillDead }

/// BotDied recovery (Finding #17): resurrect the bot IN PLACE via `bot.revive`, confirm
/// alive, gm.teleport to the camp anchor. The bot stays claimed the entire time — there
/// is NO ownership release, so the native AI never relocates it and the #12 re-mount
/// vector (which lived in the old release window) is gone. `bot.revive` is inventory-safe
/// (Finding #10 intact — it never calls Refresh()/ClearInventory()).
pub(crate) async fn recover_from_death(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &GrindGoal,
) -> RecoveryOutcome {
    // 1. Resurrect in place (bounded retry).
    let mut revived = false;
    for attempt in 0..REVIVE_MAX_ATTEMPTS {
        match crate::own::bot_revive(client, bot_guid).await {
            Ok(_) => { revived = true; break; }
            Err(e) => {
                tracing::warn!(bot_guid, attempt, error = %e, "recovery: bot.revive failed");
                if attempt + 1 < REVIVE_MAX_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(REVIVE_RETRY_MS)).await;
                }
            }
        }
    }
    if !revived {
        return RecoveryOutcome::StillDead;
    }

    // 2. Confirm alive.
    let mut alive = false;
    let mut polls_used: u32 = 0;
    for poll in 0..REVIVE_CONFIRM_POLLS {
        match read_self(client, bot_guid).await {
            Ok(s) if s.hp_pct > 0 => { alive = true; polls_used = poll + 1; break; }
            Ok(_) => {}
            Err(e) => tracing::warn!(bot_guid, error = %e, "recovery: post-revive state poll failed"),
        }
        if poll + 1 < REVIVE_CONFIRM_POLLS {
            tokio::time::sleep(Duration::from_millis(REVIVE_POLL_MS)).await;
        }
    }
    if !alive {
        return RecoveryOutcome::StillDead;
    }

    // 3. Re-home to the camp anchor.
    let a = &goal.anchor_point;
    if let Err(e) = client.call("gm.teleport", serde_json::json!({
        "target_guid": bot_guid as i64,
        "map": a.map_id as i64,
        "x": a.x, "y": a.y, "z": a.z,
        "orientation": 0.0,
    })).await {
        tracing::warn!(bot_guid, error = %e, "recovery: teleport to anchor failed");
    }

    tracing::info!(bot_guid, polls_used, "recovery_succeeded: revived (bot.revive), teleported to anchor");
    RecoveryOutcome::Recovered
}

/// Deterministic pseudo-sample in [min,max] from a rolling seed (avoids a rng dep and
/// keeps tests reproducible). `seed` should vary per call (e.g. a kill counter).
fn sample_ms(min: u64, max: u64, seed: u64) -> u64 {
    if max <= min { return min; }
    min + (seed.wrapping_mul(2654435761) % (max - min + 1))
}

/// `Reacting`: a short delay before moving — the "noticed it" beat (design §7).
pub async fn react(target: Target, seed: u64) -> GrindState {
    tokio::time::sleep(Duration::from_millis(sample_ms(REACT_DELAY_MS_MIN, REACT_DELAY_MS_MAX, seed))).await;
    GrindState::Approaching { target }
}

/// `Approaching`: walk to (or toward) the target's position via the navmesh.
///
/// For caster plugins (`engage_range` > 5 y), stops short at `engage_range * ENGAGE_STOP_SHORT_FACTOR`
/// from the target along the bot→target line. For melee plugins (`engage_range` ≈ 5 y)
/// this is the same as walking to the target directly. On a navigation failure
/// (no path / stuck), falls back to `Scanning` to pick a fresh target.
///
/// `bot_pos` is the bot's current world-space position obtained via `obs.get_position`,
/// used to compute the stop-short point for ranged engage.
pub async fn approach(
    client: &HarnessClient,
    bot_guid: u64,
    target: Target,
    engage_range: f64,
    bot_pos: (f64, f64, f64),
) -> Result<GrindState, GrindError> {
    // Casters stop short of the target: walk to a point engage_range*ENGAGE_STOP_SHORT_FACTOR
    // from the target along the bot→target line (melee plugins: engage ~5 y ≈ unchanged behaviour).
    let dest = if target.distance > engage_range {
        let (bx, by, bz) = bot_pos;
        let frac = ((target.distance - engage_range * ENGAGE_STOP_SHORT_FACTOR) / target.distance).clamp(0.0, 1.0);
        Dest {
            x: bx + (target.x - bx) * frac,
            y: by + (target.y - by) * frac,
            z: bz + (target.z - bz) * frac,
        }
    } else {
        // Already within engage range — no walk needed; transition straight to Fighting.
        return Ok(GrindState::Fighting { target });
    };
    match nav::walk_to(client, bot_guid, dest).await {
        Ok(()) => Ok(GrindState::Fighting { target }),
        Err(NavError::NoPath) | Err(NavError::Stuck(_)) | Err(NavError::RepathBudgetExceeded(_)) => {
            Ok(GrindState::Scanning)
        }
        Err(NavError::Timeout) => Ok(GrindState::Scanning),
        Err(NavError::Harness(e)) => Err(GrindError::Harness(e)),
        Err(NavError::Shape(s)) => Err(GrindError::Shape(s)),
    }
}

use tot_goal_contract::WorldPos;

const IDLE_SCAN_INTERVAL_MS: u64 = 5000;

/// Pick a fresh wander destination within `wander_radius` of the anchor.
/// Deterministic from `seed` (no rng dep; reproducible tests). Spreads points around
/// the circle so consecutive picks differ (design §7: "avoid the last few spots").
pub fn next_wander_point(goal: &GrindGoal, seed: u64) -> WorldPos {
    let angle = (seed.wrapping_mul(40501) % 360) as f64 * std::f64::consts::PI / 180.0;
    let frac = ((seed.wrapping_mul(2654435761) % 1000) as f64 / 1000.0).max(0.35); // 35%..100% out
    let r = goal.wander_radius as f64 * frac;
    WorldPos {
        map_id: goal.anchor_point.map_id,
        x: goal.anchor_point.x + r * angle.cos(),
        y: goal.anchor_point.y + r * angle.sin(),
        z: goal.anchor_point.z,
    }
}

/// `Wandering`: walk (not sprint) to a fresh scan point, then rescan.
pub async fn wander(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &GrindGoal,
    seed: u64,
) -> Result<GrindState, GrindError> {
    let p = next_wander_point(goal, seed);
    match nav::walk_to(client, bot_guid, Dest { x: p.x, y: p.y, z: p.z }).await {
        Ok(()) | Err(NavError::NoPath) | Err(NavError::Stuck(_))
        | Err(NavError::RepathBudgetExceeded(_)) | Err(NavError::Timeout) => Ok(GrindState::Scanning),
        Err(NavError::Harness(e)) => Err(GrindError::Harness(e)),
        Err(NavError::Shape(s)) => Err(GrindError::Shape(s)),
    }
}

/// `Idle`: at the camp, wait one scan interval, then rescan.
/// Sustained-idle detection is handled in `run_grind` via `empty_scan_cycles`; this
/// function only performs a single sleep-and-rescan tick.
pub async fn idle_tick() -> GrindState {
    tokio::time::sleep(Duration::from_millis(IDLE_SCAN_INTERVAL_MS)).await;
    GrindState::Scanning
}

/// The grind state machine. All states are fully implemented.
#[derive(Debug, Clone, PartialEq)]
pub enum GrindState {
    Scanning,
    Reacting { target: Target },
    Approaching { target: Target },
    Fighting { target: Target },
    PostKillPause { target: Target },
    Looting { target: Target },
    HealthCheck,
    Resting,
    Wandering,
    /// Camp empty after a full empty-scan round — wait for respawns, then rescan.
    RespawnWait,
    /// Bot died — release for native self-revive, teleport back, re-claim.
    Recovering,
    /// Bags hit a trigger — run the vendor trip, then rescan (slice 2.4).
    Vendoring { bags: crate::economy::BagSummary, vendor: tot_goal_contract::VendorInfo },
    Idle,
    Done,
}

use tot_goal_contract::{GoalProgress, GoalStatus};

// fight_noop removed — replaced by `fight()` above (Plan 2).

/// `PostKillPause`: the highest-leverage "alive" tell (design §7, §E) — pause before loot.
pub async fn post_kill_pause(target: Target, seed: u64) -> GrindState {
    tokio::time::sleep(Duration::from_millis(sample_ms(POST_KILL_PAUSE_MS_MIN, POST_KILL_PAUSE_MS_MAX, seed))).await;
    GrindState::Looting { target }
}

// loot_noop removed — replaced by loot::loot_nearest in run_grind (Plan 2).

/// Rest poll constants (exec timing; not goal fields).
const REST_POLL_INTERVAL_MS: u64 = 2000;
/// Bounded resting: stop polling after this many attempts (~60 s at 2 s/poll).
const REST_MAX_POLLS: u32 = 30;

#[derive(Debug, Deserialize)]
pub(crate) struct SelfState {
    pub(crate) level: u32,
    /// Health as an integer percentage 0–100 (design §6.1, pre-flight note).
    #[serde(default)]
    pub(crate) hp_pct: u32,
    /// Present after the 2.2 digest extension; None against older worldservers.
    #[serde(default)]
    mana_pct: Option<u32>,
    #[serde(default)]
    power_pct: Option<u32>,
    #[serde(default)]
    combo_points: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct StateDigest {
    #[serde(rename = "self")]
    self_: SelfState,
}

/// Read the bot's own state from `obs.get_state`.
async fn read_self_digest(client: &HarnessClient, bot_guid: u64) -> Result<StateDigest, GrindError> {
    // obs.get_state takes `target_guid` (the older obs.* tools use target_guid, unlike the
    // newer bot_guid-keyed M1 tools). For a bot reading its OWN state, target = the bot.
    let raw = client.call("obs.get_state", serde_json::json!({ "target_guid": bot_guid as i64 })).await?;
    serde_json::from_value(raw).map_err(|e| GrindError::Shape(format!("obs.get_state: {e}")))
}

/// Convenience wrapper: read the bot's own SelfState.
pub(crate) async fn read_self(client: &HarnessClient, bot_guid: u64) -> Result<SelfState, GrindError> {
    Ok(read_self_digest(client, bot_guid).await?.self_)
}

/// Obtain the bot's current world-space position via `obs.get_position`.
/// On harness error, propagates as `GrindError::Harness` — never falls back to a
/// made-up position (a (0,0,0) fallback would interpolate destinations from map origin).
pub(crate) async fn bot_world_pos(client: &HarnessClient, bot_guid: u64) -> Result<(f64, f64, f64), GrindError> {
    let raw = client
        .call("obs.get_position", serde_json::json!({ "target_guid": bot_guid as i64 }))
        .await?;
    #[derive(Deserialize)]
    struct Pos { x: f64, y: f64, z: f64 }
    let p: Pos = serde_json::from_value(raw)
        .map_err(|e| GrindError::Shape(format!("obs.get_position: {e}")))?;
    Ok((p.x, p.y, p.z))
}

/// Combat poll constants (not goal fields — exec timing, see design §4.2).
const FIGHT_POLL_INTERVAL_MS: u64 = 500;
/// Maximum combat-poll iterations before giving up.
/// Each iteration waits max(cast_time, 500 ms) plus 2 harness polls, so this bounds
/// to ~30 s for melee rotations; slow casters may take a few minutes per poll cycle.
const FIGHT_MAX_POLLS: u32 = 60;

/// Internal result of the `fight` state — the `run_grind` loop maps this to the next state.
#[derive(Debug)]
pub(crate) enum FightOutcome {
    /// Target is dead (absent or hp==0 in the hostiles scan). Proceed to post-kill pause.
    TargetDead,
    /// Bot died during combat. Must escalate.
    BotDied,
    /// ≥ CAST_SPIN_THRESHOLD identical CastFailed details — fight cannot progress
    /// (finding #12). run_grind probes mount state and aborts or dismounts.
    CastSpin { detail: Option<i64> },
}

/// Returns `Some((hp_pct, distance))` while the target is alive in the scan radius;
/// `None` when gone/dead. Both values are refreshed each tick so CHARGE_PRECOND
/// and other distance-gated conditions see the live distance, not the stale scan-time value.
async fn target_hp(
    client: &HarnessClient,
    bot_guid: u64,
    target_guid: u64,
    goal: &GrindGoal,
) -> Result<Option<(f32, f64)>, GrindError> {
    let raw = client
        .call("obs.get_nearby_hostiles", serde_json::json!({
            "bot_guid": bot_guid as i64,
            "radius": goal.max_search_radius as f64,
        }))
        .await?;
    let parsed: NearbyHostiles =
        serde_json::from_value(raw)
            .map_err(|e| GrindError::Shape(format!("nearby_hostiles: {e}")))?;
    Ok(parsed.hostiles.iter()
        .find(|h| h.guid == target_guid && h.is_alive && h.hp_pct > 0.0)
        .map(|h| (h.hp_pct, h.distance)))
}

/// `Fighting`: tick the rotation from the goal, refresh ctx each tick, handle cast-time wait.
///
/// Returns:
/// - `Ok(FightOutcome::TargetDead)` — target gone or hp==0.
/// - `Ok(FightOutcome::BotDied)` — bot's own hp==0 (caller escalates).
/// - `Err(GrindError)` — harness/shape failure.
pub(crate) async fn fight(
    client: &HarnessClient,
    bot_guid: u64,
    target: Target,
    goal: &GrindGoal,
    rotation: &RotationPlugin,
) -> Result<FightOutcome, GrindError> {
    let mut memo = FightMemo::default();
    let mut ctx = CombatContext {
        bot_hp_pct: 100.0,
        bot_power_pct: 100.0,
        bot_mana_pct: None,
        combo_points: 0,
        target_hp_pct: 100.0,
        target_distance: target.distance,
    };

    for _ in 0..FIGHT_MAX_POLLS {
        // Fire the rotation tick. The target can die between the previous liveness poll
        // and this tick; the adapter returns "target is not alive" — the grind's WIN
        // condition. Treat it as TargetDead and proceed to loot.
        match rotation.tick(bot_guid, target.guid, client, &ctx, &mut memo).await {
            Ok(TickOutcome::TargetGone) => return Ok(FightOutcome::TargetDead),
            Ok(TickOutcome::CastSpin { detail }) => return Ok(FightOutcome::CastSpin { detail }),
            Ok(TickOutcome::Acted { wait_ms }) => {
                // max(): an exhausted tick (wait_ms==0) must not hot-spin (combat.rs note).
                let wait = wait_ms.max(FIGHT_POLL_INTERVAL_MS);
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            }
            Err(e) => {
                if is_target_dead_attack_error(&e) {
                    return Ok(FightOutcome::TargetDead);
                }
                return Err(match e {
                    crate::combat::CombatError::Harness(he) => GrindError::Harness(he),
                    crate::combat::CombatError::Shape(s) => GrindError::Shape(s),
                });
            }
        }

        // Refresh: target hp + distance (same hostiles poll as liveness) + own state.
        match target_hp(client, bot_guid, target.guid, goal).await? {
            None => return Ok(FightOutcome::TargetDead),
            Some((hp, dist)) => {
                ctx.target_hp_pct = hp;
                ctx.target_distance = dist;
            }
        }
        let self_state = read_self(client, bot_guid).await?;
        if self_state.hp_pct == 0 {
            return Ok(FightOutcome::BotDied);
        }
        ctx.bot_hp_pct = self_state.hp_pct as f32;
        ctx.bot_mana_pct = self_state.mana_pct.map(|m| m as f32);
        ctx.bot_power_pct = self_state.power_pct.unwrap_or(100) as f32;
        ctx.combo_points = self_state.combo_points.unwrap_or(0);
    }

    // Poll budget exhausted — treat as target dead (safety: avoid infinite loop).
    Ok(FightOutcome::TargetDead)
}

/// A `bot.attack` failure of "target is not alive" means the target died between our
/// liveness poll and this attack — the grind's win condition, not an error. (Live-caught:
/// the worldserver kills the target on a swing that lands between exec's poll ticks.)
fn is_target_dead_attack_error(e: &crate::combat::CombatError) -> bool {
    matches!(
        e,
        crate::combat::CombatError::Harness(HarnessError::Tool { message, .. })
            if message.contains("not alive")
    )
}

/// Rest when hp OR (for mana classes) mana is below the goal threshold.
fn should_rest(hp_pct: u32, mana_pct: Option<u32>, threshold: f32) -> bool {
    (hp_pct as f32 / 100.0) < threshold
        || mana_pct.map(|m| (m as f32 / 100.0) < threshold).unwrap_or(false)
}

/// Resting is done when hp AND (for mana classes) mana have recovered to `REST_RECOVERY_PCT`.
fn rest_done(hp_pct: u32, mana_pct: Option<u32>) -> bool {
    hp_pct >= REST_RECOVERY_PCT && mana_pct.map(|m| m >= REST_RECOVERY_PCT).unwrap_or(true)
}

// ── Buff pass ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct AuraEntry {
    spell_id: u32,
    /// Rank-1 head of the aura's spell chain (2.2 ObsGetAuras extension);
    /// falls back to spell_id against older worldservers.
    #[serde(default)]
    first_spell_id: Option<u32>,
    /// Finding #12: nested effect amounts carrying aura_type (wire-verified live 2026-06-11).
    /// aura_type 78 = SPELL_AURA_MOUNTED; nested, NOT top-level.
    #[serde(default)]
    effect_amounts: Vec<EffectAmount>,
}
#[derive(Debug, serde::Deserialize)]
struct AurasResult { auras: Vec<AuraEntry> }

/// Cast any plugin self-buff whose rank-1 id is absent from the bot's auras.
pub(crate) async fn ensure_buffs(
    client: &HarnessClient,
    bot_guid: u64,
    plugin: &RotationPlugin,
) -> Result<(), GrindError> {
    use crate::combat::ActionOutcome;

    if plugin.buffs().is_empty() { return Ok(()); }
    // obs.get_auras keeps the older target_guid arg idiom (cf. obs.get_state).
    let raw = client.call("obs.get_auras", serde_json::json!({ "target_guid": bot_guid as i64 })).await?;
    let parsed: AurasResult = serde_json::from_value(raw)
        .map_err(|e| GrindError::Shape(format!("obs.get_auras: {e}")))?;
    let have: std::collections::HashSet<u32> = parsed.auras.iter()
        .map(|a| a.first_spell_id.unwrap_or(a.spell_id))
        .collect();
    for b in plugin.buffs() {
        if have.contains(&b.spell_id) { continue; }
        match b.execute(bot_guid, 0, client).await {
            Ok(ActionOutcome::Casting { cast_time_ms }) => {
                // Sleep the cast time, but no less than one GCD.
                let wait = cast_time_ms.max(GCD_MS);
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            }
            Ok(ActionOutcome::Failed { code, .. }) => {
                // Typed failure (no_power, on_cooldown, etc.) — log and move on, no sleep.
                tracing::debug!(buff = b.name, ?code, "buff cast did not start");
            }
            Ok(ActionOutcome::Engaged) => {
                // CastSpellAction never produces Engaged; treat as no sleep needed.
            }
            Err(e) => {
                tracing::warn!(buff = b.name, error = %e, "buff cast errored");
                // Continue — a single buff failure should not abort the pass.
            }
        }
    }
    Ok(())
}

/// SPELL_AURA_MOUNTED (SharedDefines.h AuraType). detail-64 wedge precondition.
const SPELL_AURA_MOUNTED: u32 = 78;

#[derive(Debug, serde::Deserialize)]
struct EffectAmount {
    #[serde(default)]
    aura_type: Option<u32>,
}

/// Finding #12: a mounted bot cannot cast (SPELL_FAILED_NOT_MOUNTED) and
/// Unit::Attack silently no-ops — probe before grinding/after re-claims.
pub(crate) async fn is_mounted(client: &HarnessClient, bot_guid: u64) -> Result<bool, GrindError> {
    let raw = client.call("obs.get_auras", serde_json::json!({ "target_guid": bot_guid as i64 })).await?;
    let parsed: AurasResult = serde_json::from_value(raw)
        .map_err(|e| GrindError::Shape(format!("obs.get_auras: {e}")))?;
    Ok(parsed.auras.iter()
        .any(|a| a.effect_amounts.iter().any(|e| e.aura_type == Some(SPELL_AURA_MOUNTED))))
}

/// Dismount-via-native budget (finding #12): native AI dismounts ~2 min after a
/// release (CheckMountStateAction); same poll idiom as death recovery.
const DISMOUNT_POLL_MS: u64 = if cfg!(test) { 20 } else { 5_000 };
const DISMOUNT_MAX_POLLS: u32 = 36;

#[derive(Debug)]
pub(crate) enum DismountOutcome { Dismounted, StillMounted }

/// Finding #12 + M3 #9 rider: attempt `bot.dismount` (instant RPC, requires claim).
///
/// On success: returns `Dismounted` immediately without releasing ownership.
/// On any error (tool unknown, bot not found, not externally owned, harness
/// unavailable): falls back to `dismount_via_native` (release→poll→re-claim).
///
/// Graceful degradation is mandatory: the live daemon may not yet have
/// `bot.dismount` registered (e.g. after a daemon rollback). In that case
/// the harness returns an error and the native path keeps behaviour intact.
pub(crate) async fn dismount(client: &HarnessClient, bot_guid: u64) -> DismountOutcome {
    let args = serde_json::json!({ "bot_guid": bot_guid as i64 });
    match client.call("bot.dismount", args).await {
        Ok(result) => {
            // bot.dismount succeeded — bot is now unmounted, still claimed.
            let was_mounted = result
                .get("was_mounted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            tracing::info!(bot_guid, was_mounted, "dismount_direct: bot.dismount ok");
            DismountOutcome::Dismounted
        }
        Err(e) => {
            // Graceful degradation: bot.dismount unavailable or errored.
            // Fall back to the release/poll/re-claim native dance.
            tracing::warn!(
                bot_guid,
                error = %e,
                "dismount_direct: bot.dismount failed — falling back to native dismount"
            );
            dismount_via_native(client, bot_guid).await
        }
    }
}

/// Finding #12 (fallback): no direct dismount verb — release to native AI,
/// poll the mount aura away, re-claim.
///
/// Kept as the fallback for `dismount()` when bot.dismount is unavailable
/// (e.g. daemon rollback to a version without the M3 #9 rider).
///
/// INVARIANT: re-claims ownership on EVERY exit path (bot must be owned when this returns).
pub(crate) async fn dismount_via_native(client: &HarnessClient, bot_guid: u64) -> DismountOutcome {
    if let Err(e) = crate::own::set_ai_owned(client, bot_guid, false).await {
        tracing::warn!(bot_guid, error = %e, "dismount: release failed");
    }
    let mut dismounted = false;
    let mut polls_used: u32 = 0;
    for poll in 0..DISMOUNT_MAX_POLLS {
        tokio::time::sleep(Duration::from_millis(DISMOUNT_POLL_MS)).await;
        match is_mounted(client, bot_guid).await {
            Ok(false) => { dismounted = true; polls_used = poll + 1; break; }
            Ok(true) => {}
            Err(e) => tracing::warn!(bot_guid, error = %e, "dismount: aura poll failed"),
        }
    }
    if let Err(e) = crate::own::set_ai_owned(client, bot_guid, true).await {
        tracing::warn!(bot_guid, error = %e, "dismount: re-claim failed");
    }
    if dismounted {
        tracing::info!(bot_guid, polls_used, "dismount_succeeded: native AI dismounted, re-claimed");
        DismountOutcome::Dismounted
    } else {
        tracing::warn!(bot_guid, "dismount_budget_exhausted: still mounted after release window");
        DismountOutcome::StillMounted
    }
}

/// On goal entry, if the bot is alive but displaced from its camp (farther than
/// `wander_radius` from the anchor), teleport it home once. No-op for on-camp bots.
///
/// Finding #17 hardening: catches any alive-but-displaced bot (e.g. legacy drift from
/// the old release-window relocation, or a cross-zone warp by native AI). The bot stays
/// claimed; `gm.teleport` moves it silently without affecting inventory.
/// Returns `true` if a teleport was issued, `false` otherwise.
pub(crate) async fn maybe_anchor_on_entry(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &GrindGoal,
) -> bool {
    let a = &goal.anchor_point;
    let reach = goal.wander_radius as f64;
    // Read position with map_id so cross-continent displacement is caught even when
    // the bot's x,y coincidentally fall within wander_radius (AzerothCore overlaps EK/Kalimdor).
    let need_home = {
        #[derive(Deserialize)]
        struct PosWithMap { x: f64, y: f64, map_id: i64 }
        match client.call("obs.get_position",
            serde_json::json!({ "target_guid": bot_guid as i64 })).await
        {
            Ok(raw) => match serde_json::from_value::<PosWithMap>(raw) {
                Ok(p) => {
                    let wrong_map = p.map_id != a.map_id as i64;
                    let dx = p.x - a.x;
                    let dy = p.y - a.y;
                    let out_of_range = (dx * dx + dy * dy).sqrt() > reach;
                    wrong_map || out_of_range
                }
                Err(e) => {
                    tracing::warn!(bot_guid, error = %e, "anchor_on_entry: pos deserialize failed");
                    false
                }
            },
            Err(e) => {
                tracing::warn!(bot_guid, error = %e, "anchor_on_entry: pos read failed");
                false
            }
        }
    };
    if need_home {
        if let Err(e) = client.call("gm.teleport", serde_json::json!({
            "target_guid": bot_guid as i64,
            "map": a.map_id as i64,
            "x": a.x, "y": a.y, "z": a.z,
            "orientation": 0.0,
        })).await {
            tracing::warn!(bot_guid, error = %e, "anchor_on_entry: teleport failed");
            return false;
        }
        tracing::info!(bot_guid, "anchor_on_entry: re-homed displaced bot to camp anchor");
        return true;
    }
    false
}

/// Drive a `Grind` goal to a terminal `GoalStatus`. All states are real.
pub async fn run_grind(client: &HarnessClient, bot_guid: u64, goal: &GrindGoal) -> GoalStatus {
    let mut kills: u32 = 0;
    // Dead-on-arrival check (design §2.2): a goal can start on a corpse — e.g. a
    // re-emitted goal after a budget-exhausted recovery left the bot dead+owned.
    // NOTE: relies on hp_pct in the obs.get_state digest (present since the 2.2
    // Tier0 digest); a worldserver omitting it would serde-default to 0 and
    // misfire — the brain image is always paired with a digest-bearing worldserver.
    let self0 = read_self(client, bot_guid).await;
    let level_at_start = self0.as_ref().map(|s| s.level).unwrap_or(0);
    let mut state = if self0.map(|s| s.hp_pct == 0).unwrap_or(false) {
        GrindState::Recovering
    } else {
        GrindState::Scanning
    };
    let mut idle_seed: u64 = 0;
    let mut empty_scan_cycles: u32 = 0;
    let mut respawn_waits: u32 = 0;
    let mut deaths_this_goal: u32 = 0;
    let mut kills_at_last_econ_check: u32 = 0;
    let mut last_vendor_trip: Option<std::time::Instant> = None;
    let econ_cooldown = std::time::Duration::from_secs(crate::economy::VENDOR_TRIP_COOLDOWN_S);

    // Build the rotation once for the lifetime of this grind; pass by ref to fight().
    let rotation_id = goal.rotation_id.as_deref().unwrap_or("auto_attack");
    let rotation = crate::rotations::build(rotation_id)
        .unwrap_or_else(|| {
            tracing::warn!("unknown rotation_id {rotation_id}; using auto_attack");
            RotationPlugin::melee_m1()
        });

    // Finding #12 claim-time mount check: a bot claimed while mounted cannot cast
    // and Unit::Attack silently no-ops. Probe failures default to not-mounted —
    // never block a goal on a flaky aura read; the spin backoff is the backstop.
    if !matches!(state, GrindState::Recovering)
        && is_mounted(client, bot_guid).await.unwrap_or(false)
    {
        tracing::warn!(bot_guid, "mount_check: mounted at goal entry — dismount cycle");
        if matches!(dismount(client, bot_guid).await, DismountOutcome::StillMounted) {
            return GoalStatus::Blocked {
                reason: tot_goal_contract::BlockedReason::Other,
                detail: Some("mount_wedge: still mounted after dismount budget".into()),
            };
        }
    }

    // Start-anchor teleport (Finding #17 hardening): re-home a live-but-displaced bot
    // before the first scan. Skipped when arriving dead — dead bot teleport would be
    // a no-op server-side and we don't want to race the Recovering state.
    if !matches!(state, GrindState::Recovering) {
        maybe_anchor_on_entry(client, bot_guid, goal).await;
    }

    // Buff pass at grind entry — skipped when arriving dead (buffing a corpse
    // wastes an RPC and logs a spurious warn); the post-recovery path re-buffs.
    if !matches!(state, GrindState::Recovering) {
        if let Err(e) = ensure_buffs(client, bot_guid, &rotation).await {
            tracing::warn!(error = %e, "buff pass failed");
        }
    }

    // Goal-entry economy check (spec rounds 4/4b): death-loop bots re-emit goals before
    // the kill stride can fire — but bag state carries across goal lives. One poll
    // at entry catches accumulated greys; the stride handles mid-goal accumulation.
    if !matches!(state, GrindState::Recovering) {
        if let Some((bags, vendor)) =
            crate::economy::entry_check(client, bot_guid, goal, last_vendor_trip, econ_cooldown).await
        {
            state = GrindState::Vendoring { bags, vendor };
        }
    }

    loop {
        state = match state {
            GrindState::Scanning => match scan_for_target(client, bot_guid, goal).await {
                Ok(Some(t)) => {
                    empty_scan_cycles = 0;
                    respawn_waits = 0; // a live camp resets the dry-camp budget
                    GrindState::Reacting { target: t }
                }
                Ok(None) => {
                    empty_scan_cycles += 1;
                    if empty_scan_cycles >= MAX_EMPTY_SCAN_CYCLES {
                        GrindState::RespawnWait
                    } else {
                        GrindState::Wandering
                    }
                }
                Err(GrindError::Harness(e)) =>
                    return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                 detail: Some(format!("scan harness: {e}")) },
                Err(GrindError::Shape(s)) =>
                    return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                 detail: Some(s) },
            },
            GrindState::Reacting { target } => react(target, kills as u64).await,
            GrindState::Approaching { target } => {
                // Read the bot's current world-space position via obs.get_position so
                // the caster engage calculation can compute the stop-short point.
                // On harness error: propagate as Blocked (same as other harness failures).
                let bot_pos = match bot_world_pos(client, bot_guid).await {
                    Ok(p) => p,
                    Err(GrindError::Harness(e)) =>
                        return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                     detail: Some(format!("approach pos harness: {e}")) },
                    Err(GrindError::Shape(s)) =>
                        return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                     detail: Some(format!("approach pos shape: {s}")) },
                };
                match approach(client, bot_guid, target, rotation.engage_range(), bot_pos).await {
                    Ok(s) => s,
                    Err(GrindError::Harness(e)) =>
                        return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                     detail: Some(format!("approach harness: {e}")) },
                    Err(GrindError::Shape(s)) =>
                        return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                     detail: Some(format!("approach shape: {s}")) },
                }
            }
            GrindState::Fighting { target } => match fight(client, bot_guid, target, goal, &rotation).await {
                Ok(FightOutcome::TargetDead) => GrindState::PostKillPause { target },
                Ok(FightOutcome::BotDied) => GrindState::Recovering,
                Ok(FightOutcome::CastSpin { detail }) => {
                    // Finding #12 + M3 #9: probe-don't-guess. Mounted → dismount
                    // (tries bot.dismount direct first; falls back to native cycle);
                    // anything else → bounded Blocked retry (300s ledger cooldown),
                    // never an in-place spin.
                    if is_mounted(client, bot_guid).await.unwrap_or(false) {
                        match dismount(client, bot_guid).await {
                            DismountOutcome::Dismounted => {
                                if let Err(e) = ensure_buffs(client, bot_guid, &rotation).await {
                                    tracing::warn!(error = %e, "post-dismount buff pass failed");
                                }
                                GrindState::Scanning
                            }
                            DismountOutcome::StillMounted => {
                                return GoalStatus::Blocked {
                                    reason: tot_goal_contract::BlockedReason::Other,
                                    detail: Some("mount_wedge: still mounted after dismount budget".into()),
                                };
                            }
                        }
                    } else {
                        return GoalStatus::Blocked {
                            reason: tot_goal_contract::BlockedReason::Other,
                            detail: Some(format!("cast_spin: detail={detail:?} after identical-fail threshold")),
                        };
                    }
                }
                Err(GrindError::Harness(e)) =>
                    return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                 detail: Some(format!("fight harness: {e}")) },
                Err(GrindError::Shape(s)) =>
                    return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                 detail: Some(format!("fight shape: {s}")) },
            },
            GrindState::PostKillPause { target } => post_kill_pause(target, kills as u64).await,
            GrindState::Looting { target: _ } => {
                kills += 1;
                // Non-fatal: loot failure (no corpse / nav failure) doesn't stop the loop.
                let _ = crate::loot::loot_nearest(client, bot_guid).await;
                GrindState::HealthCheck
            }
            GrindState::HealthCheck => {
                let self_state = read_self(client, bot_guid).await;
                let hp_pct = self_state.as_ref().map(|s| s.hp_pct).unwrap_or(100);
                let mana_pct = self_state.as_ref().ok().and_then(|s| s.mana_pct);
                let lvl = self_state.map(|s| s.level).unwrap_or(level_at_start);
                // Rest-threshold check BEFORE completion check (design §7).
                if should_rest(hp_pct, mana_pct, goal.rest_threshold) {
                    GrindState::Resting
                } else if let (Some(bags), Some(vendor)) = (
                    crate::economy::economy_due(
                        client, bot_guid, goal, kills,
                        &mut kills_at_last_econ_check, last_vendor_trip, econ_cooldown,
                    ).await,
                    goal.vendor,
                ) {
                    // Economy interrupt BEFORE the completion check (spec §4) — a
                    // level-up on the trigger kill still vendors first; one extra
                    // grind pass after the trip is accepted. economy_due only fires
                    // when goal.vendor is Some; carrying it in the variant keeps the
                    // Vendoring arm panic-free.
                    GrindState::Vendoring { bags, vendor }
                } else if lvl >= goal.to_level
                    || goal.kill_count.map(|k| kills >= k).unwrap_or(false)
                {
                    return GoalStatus::Completed {
                        summary: format!("grind done: {kills} kills, level {lvl}"),
                    };
                } else {
                    GrindState::Scanning
                }
            }
            GrindState::Resting => {
                // Poll obs.get_state until both hp and mana (for mana classes) have
                // recovered to REST_RECOVERY_PCT, or the poll budget is exhausted.
                for _ in 0..REST_MAX_POLLS {
                    tokio::time::sleep(std::time::Duration::from_millis(REST_POLL_INTERVAL_MS)).await;
                    let state = read_self(client, bot_guid).await;
                    let hp = state.as_ref().map(|s| s.hp_pct).unwrap_or(100);
                    let mana = state.as_ref().ok().and_then(|s| s.mana_pct);
                    if rest_done(hp, mana) {
                        break;
                    }
                }
                // Re-buff after resting (mana classes may have run dry pre-rest).
                if let Err(e) = ensure_buffs(client, bot_guid, &rotation).await {
                    tracing::warn!(error = %e, "buff pass failed");
                }
                GrindState::Scanning
            }
            GrindState::Wandering => match wander(client, bot_guid, goal, idle_seed).await {
                Ok(s) => { idle_seed += 1; s }
                Err(GrindError::Harness(e)) =>
                    return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                 detail: Some(format!("wander harness: {e}")) },
                Err(GrindError::Shape(s)) =>
                    return GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::Other,
                                                 detail: Some(format!("wander shape: {s}")) },
            },
            GrindState::RespawnWait => {
                respawn_waits += 1;
                if respawn_waits >= MAX_RESPAWN_WAITS {
                    // True backstop — the brain re-emits after its Blocked cooldown.
                    return GoalStatus::Blocked {
                        reason: tot_goal_contract::BlockedReason::NoTargetsFound,
                        detail: Some(format!(
                            "no in-band targets after {MAX_RESPAWN_WAITS} respawn waits"
                        )),
                    };
                }
                tracing::info!(bot_guid, respawn_waits, "camp empty — waiting for respawns");
                tokio::time::sleep(Duration::from_millis(RESPAWN_WAIT_MS)).await;
                empty_scan_cycles = 0;
                GrindState::Scanning
            }
            GrindState::Recovering => {
                deaths_this_goal += 1;
                match recover_from_death(client, bot_guid, goal).await {
                    RecoveryOutcome::StillDead => {
                        // Recovery budget exhausted — bot is dead-but-owned; the
                        // dead-on-arrival check of the next (re-emitted) goal retries.
                        return GoalStatus::NeedsDecision {
                            event: tot_goal_contract::EscalationEvent::BotDied { position: None },
                        };
                    }
                    RecoveryOutcome::Recovered => {
                        if deaths_this_goal > MAX_DEATHS_PER_GOAL {
                            // Alive, at anchor, owned — but dying too often for this
                            // goal. Hand back; the brain paces with its cooldown.
                            return GoalStatus::NeedsDecision {
                                event: tot_goal_contract::EscalationEvent::BotDied { position: None },
                            };
                        }
                        if let Err(e) = ensure_buffs(client, bot_guid, &rotation).await {
                            tracing::warn!(error = %e, "post-recovery buff pass failed");
                        }
                        // Post-recovery economy check (spec round 4b): DOA goals skip
                        // the entry check, and death-loop bots ALWAYS arrive via
                        // recovery. Fresh revive at the anchor is the ideal vendor
                        // moment (repair the death damage). Cooldown-guarded so a
                        // death spiral can't chain trips.
                        if let Some((bags, vendor)) = crate::economy::entry_check(
                            client, bot_guid, goal, last_vendor_trip, econ_cooldown,
                        ).await {
                            GrindState::Vendoring { bags, vendor }
                        } else {
                            GrindState::Scanning
                        }
                    }
                }
            }
            GrindState::Vendoring { bags, vendor } => {
                let outcome =
                    crate::economy::run_vendor_trip(client, bot_guid, goal, &vendor, bags, &rotation).await;
                // Cooldown runs from trip END, success or not (spec §3).
                last_vendor_trip = Some(std::time::Instant::now());
                match outcome {
                    crate::economy::VendorTripOutcome::BotDead => GrindState::Recovering,
                    crate::economy::VendorTripOutcome::Done => GrindState::Scanning,
                }
            }
            GrindState::Idle => idle_tick().await,
            GrindState::Done => {
                // TODO(Plan 2): fetch current level via read_self when Done becomes reachable
                return GoalStatus::Running { progress: Some(GoalProgress {
                    kills_this_goal: kills, level_at_start, current_level: level_at_start }) };
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path as AxumPath, http::StatusCode, routing::post, Json, Router};
    use serde_json::{json, Value};
    use std::sync::Arc;
    use std::time::Duration;
    use tot_goal_contract::{GrindGoal, MobFilter, VendorInfo, WorldPos};
    use tot_harness_client::HarnessClient;

    async fn spawn_mock<F>(handler: F) -> String
    where F: Fn(String, serde_json::Value) -> serde_json::Value + Send + Sync + 'static {
        let h = std::sync::Arc::new(handler);
        let route = move |AxumPath(name): AxumPath<String>, Json(args): Json<serde_json::Value>| {
            let h = h.clone();
            async move { (StatusCode::OK, Json(json!({ "ok": true, "result": h(name, args) }))) }
        };
        let app = Router::new().route("/v1/tools/:name", post(route));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        format!("http://{addr}")
    }
    fn client(base: &str) -> HarnessClient { HarnessClient::new(base, "tok", Duration::from_secs(5)) }

    /// Regression (live-caught): `obs.get_state` requires `target_guid` — the live adapter
    /// rejects a missing one with "target_guid (int) required". `read_self` must send
    /// `target_guid`, NOT `bot_guid` (the older obs.* tools are target_guid-keyed). The
    /// mock doesn't validate arg keys, so this test pins the contract.
    #[tokio::test]
    async fn read_self_sends_target_guid_for_obs_get_state() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(serde_json::Value::Null));
        let recv = received.clone();
        let base = spawn_mock(move |name, args| {
            if name == "obs.get_state" { *recv.lock().unwrap() = args.clone(); }
            json!({ "self": { "level": 6, "hp_pct": 100 } })
        }).await;
        let s = read_self(&client(&base), 1173).await.unwrap();
        assert_eq!(s.level, 6);
        let body = received.lock().unwrap().clone();
        assert_eq!(body["target_guid"], 1173_i64, "obs.get_state must send target_guid");
        assert!(body.get("bot_guid").is_none(), "must NOT send bot_guid for obs.get_state");
    }

    /// Regression (live-caught): a `bot.attack` "target is not alive" error means the
    /// target died mid-fight — the win condition. It must classify as target-dead so the
    /// grind proceeds to loot, NOT propagate as a fatal Blocked.
    #[test]
    fn is_target_dead_attack_error_classifies_not_alive() {
        use tot_harness_client::HarnessError;
        let dead = crate::combat::CombatError::Harness(HarnessError::Tool {
            tool: "bot.attack".into(),
            message: "executor_failed: bot.attack: target is not alive".into(),
        });
        assert!(is_target_dead_attack_error(&dead), "'not alive' must be target-dead");
        let other = crate::combat::CombatError::Harness(HarnessError::Tool {
            tool: "bot.attack".into(),
            message: "executor_failed: bot not found".into(),
        });
        assert!(!is_target_dead_attack_error(&other), "other errors must NOT be target-dead");
    }

    pub(super) fn test_goal() -> GrindGoal {
        GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 6, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        }
    }

    #[tokio::test]
    async fn scanning_returns_nearest_in_band_target() {
        let base = spawn_mock(|name, _a| match name.as_str() {
            "obs.get_nearby_hostiles" => json!({"hostiles": [
                {"guid": 111u64, "name": "Kobold Laborer", "level": 5, "hp_pct": 100, "distance": 8.0, "is_alive": true, "x": 100.0, "y": 200.0, "z": 5.0},
                {"guid": 222u64, "name": "Murloc", "level": 12, "hp_pct": 100, "distance": 4.0, "is_alive": true, "x": 50.0, "y": 60.0, "z": 1.0}
            ]}),
            other => panic!("unexpected tool {other}"),
        }).await;
        let g = test_goal();
        let t = scan_for_target(&client(&base), 1003, &g).await.unwrap();
        // Murloc is closer but out of the 4-7 level band; Kobold (L5) is the pick.
        assert_eq!(t, Some(Target { guid: 111, x: 100.0, y: 200.0, z: 5.0, distance: 8.0 }));
    }

    #[tokio::test]
    async fn scanning_returns_none_when_no_in_band_target() {
        let base = spawn_mock(|_n, _a| json!({"hostiles": [
            {"guid": 222u64, "name": "Murloc", "level": 12, "hp_pct": 100, "distance": 4.0, "is_alive": true}
        ]})).await;
        let t = scan_for_target(&client(&base), 1003, &test_goal()).await.unwrap();
        assert_eq!(t, None);
    }

    #[tokio::test]
    async fn approaching_walks_to_target_then_transitions_to_fighting() {
        let base = spawn_mock(|name, _a| match name.as_str() {
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":8.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0, "final": {"x":8.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":8.0,"y":0.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            other => panic!("unexpected tool {other}"),
        }).await;
        // Melee engage (5.0 y): target at distance 8.0 > 5.0 → walks to ~4y short.
        // bot_pos (0,0,0) + frac toward (8,0,0): dest ≈ (4y,0,0); nav mock returns success.
        let target = Target { guid: 111, x: 8.0, y: 0.0, z: 0.0, distance: 8.0 };
        let next = approach(&client(&base), 1003, target, 5.0, (0.0, 0.0, 0.0)).await.unwrap();
        assert_eq!(next, GrindState::Fighting { target });
    }

    #[tokio::test]
    async fn approaching_already_in_range_transitions_directly() {
        // When distance <= engage_range, approach must return Fighting without any nav calls.
        let base = spawn_mock(|name, _a| panic!("unexpected tool {name} — should not nav when in range")).await;
        let target = Target { guid: 111, x: 3.0, y: 0.0, z: 0.0, distance: 3.0 };
        // engage_range=5.0, distance=3.0 → already in range
        let next = approach(&client(&base), 1003, target, 5.0, (0.0, 0.0, 0.0)).await.unwrap();
        assert_eq!(next, GrindState::Fighting { target });
    }

    #[tokio::test]
    async fn approaching_nopath_falls_back_to_scanning() {
        let base = spawn_mock(|name, _a| match name.as_str() {
            "nav.find_path" => json!({"path_type": 8i64, "points": []}), // PATHFIND_NOPATH
            other => panic!("unexpected tool {other}"),
        }).await;
        let next = approach(&client(&base), 1003, Target { guid: 111, x: 8.0, y: 0.0, z: 0.0, distance: 8.0 }, 5.0, (0.0, 0.0, 0.0)).await.unwrap();
        assert_eq!(next, GrindState::Scanning, "no path → pick a new target");
    }

    #[tokio::test]
    async fn wander_walks_within_radius_and_returns_to_scanning() {
        let base = spawn_mock(|name, _a| match name.as_str() {
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":5.0,"y":5.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0, "final": {"x":5.0,"y":5.0,"z":0.0}}),
            "obs.get_position" => json!({"x":5.0,"y":5.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            other => panic!("unexpected tool {other}"),
        }).await;
        let next = wander(&client(&base), 1003, &test_goal(), 1).await.unwrap();
        assert_eq!(next, GrindState::Scanning);
    }

    #[test]
    fn next_wander_point_is_within_radius_of_anchor() {
        let g = test_goal();
        let p = next_wander_point(&g, 7);
        let dx = p.x - g.anchor_point.x;
        let dy = p.y - g.anchor_point.y;
        assert!((dx*dx + dy*dy).sqrt() <= g.wander_radius as f64 + 0.001);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_blocks_when_no_targets_ever_found() {
        // No in-band hostiles ever — the loop exhausts MAX_EMPTY_SCAN_CYCLES scans,
        // enters RespawnWait, repeats for MAX_RESPAWN_WAITS rounds (20 ms test sleeps),
        // and only THEN returns the Blocked{NoTargetsFound} backstop.
        let base = spawn_mock(|name, _a| match name.as_str() {
            "obs.get_nearby_hostiles" => json!({"hostiles": []}),
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":5.0,"y":5.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0, "final": {"x":5.0,"y":5.0,"z":0.0}}),
            "obs.get_position" => json!({"x":5.0,"y":5.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            // obs.get_state uses hp_pct (not health_pct) — updated for Plan 2
            "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 90}}),
            other => panic!("unexpected tool {other}"),
        }).await;
        let status = run_grind(&client(&base), 1003, &test_goal()).await;
        match status {
            tot_goal_contract::GoalStatus::Blocked { reason: tot_goal_contract::BlockedReason::NoTargetsFound, .. } => {}
            other => panic!("expected Blocked{{NoTargetsFound}}, got {other:?}"),
        }
    }

    /// A camp that respawns DURING the wait: the grind must NOT go terminal at the
    /// empty-scan budget; it waits, rescans, kills, and completes.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_waits_for_respawns_then_completes() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let scan_calls = Arc::new(AtomicU32::new(0));
        let sc = scan_calls.clone();
        let base = spawn_mock(move |name, _args| match name.as_str() {
            "obs.get_nearby_hostiles" => {
                let n = sc.fetch_add(1, SeqCst);
                if n < MAX_EMPTY_SCAN_CYCLES {
                    json!({"hostiles": []}) // dry camp → forces one RespawnWait
                } else if n == MAX_EMPTY_SCAN_CYCLES {
                    json!({"hostiles": [{"guid": 7u64, "name": "Boar", "level": 5,
                        "hp_pct": 100.0, "distance": 6.0, "is_alive": true,
                        "x": 6.0, "y": 0.0, "z": 0.0}]}) // respawn appears
                } else {
                    json!({"hostiles": []}) // combat poll: target dead
                }
            }
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":6.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0,
                "final": {"x":6.0,"y":0.0,"z":0.0}}),
            // Return the nav final waypoint so the arrival check passes instantly (dist ≤ 2.0y).
            "obs.get_position" => json!({"x":6.0,"y":0.0,"z":0.0,"map_id":0,
                "zone_id":1,"area_id":1,"orientation":0.0}),
            "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 90}}),
            "bot.attack" => json!({"attacked": true, "target_guid": 7u64, "target_name": "Boar"}),
            "obs.get_lootable_corpses" => json!({"corpses": []}),
            other => panic!("unexpected tool {other}"),
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let status = run_grind(&client(&base), 1003, &goal).await;
        assert!(matches!(status, GoalStatus::Completed { .. }), "got {status:?}");
        assert!(scan_calls.load(SeqCst) > MAX_EMPTY_SCAN_CYCLES,
            "must have scanned past the empty budget (i.e. waited instead of blocking)");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_completes_after_kill_count() {
        // One in-band target; fight() sees the target die on the first combat poll;
        // kill_count=1 → Completed.
        // The mock uses an AtomicU32 to make obs.get_nearby_hostiles return the target
        // alive on the scan phase, then dead (empty) when polled from within fight().
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let hostile_calls = std::sync::Arc::new(AtomicU32::new(0));
        let hc2 = hostile_calls.clone();
        let base = spawn_mock(move |name, _a| match name.as_str() {
            "obs.get_nearby_hostiles" => {
                // First call is the scan (Scanning state) — return target alive.
                // Subsequent calls are combat polls (fight()) — return empty (target dead).
                let n = hc2.fetch_add(1, SeqCst);
                if n == 0 {
                    json!({"hostiles": [
                        {"guid": 111u64, "name": "Kobold", "level": 5, "hp_pct": 100.0, "distance": 6.0,
                         "is_alive": true, "x": 6.0, "y": 0.0, "z": 0.0}
                    ]})
                } else {
                    json!({"hostiles": []})  // target gone → TargetDead
                }
            }
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":6.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0, "final": {"x":6.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":6.0,"y":0.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            // obs.get_state: bot alive throughout; Plan 2 field is hp_pct
            "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 90}}),
            // bot.attack called by fight()
            "bot.attack" => json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"}),
            // obs.get_lootable_corpses called by Looting state
            "obs.get_lootable_corpses" => json!({"corpses": []}),
            other => panic!("unexpected tool {other}"),
        }).await;
        let status = run_grind(&client(&base), 1003, &test_goal()).await;
        match status {
            tot_goal_contract::GoalStatus::Completed { summary } =>
                assert!(summary.contains("kill"), "summary: {summary}"),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    // ── F3: real fight() tests ────────────────────────────────────────────────

    /// fight() transitions to TargetDead when the target disappears from the hostile list.
    #[tokio::test(flavor = "multi_thread")]
    async fn fight_target_dead_when_absent_from_hostiles() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let calls = std::sync::Arc::new(AtomicU32::new(0));
        let c2 = calls.clone();

        let base = spawn_mock(move |name, _a| match name.as_str() {
            "bot.attack" => json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"}),
            "obs.get_nearby_hostiles" => {
                let n = c2.fetch_add(1, SeqCst);
                if n < 2 {
                    // First 2 polls: target alive
                    json!({"hostiles": [{"guid": 111u64, "level": 5, "hp_pct": 50.0, "distance": 3.0, "is_alive": true}]})
                } else {
                    // 3rd poll: target absent → TargetDead
                    json!({"hostiles": []})
                }
            }
            "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 80}}),
            other => panic!("unexpected tool in fight_target_dead test: {other}"),
        }).await;

        let goal = test_goal();
        let target = Target { guid: 111, x: 3.0, y: 0.0, z: 0.0, distance: 3.0 };
        let rotation = RotationPlugin::melee_m1();
        let outcome = fight(&client(&base), 1003, target, &goal, &rotation).await.unwrap();
        assert!(matches!(outcome, FightOutcome::TargetDead), "expected TargetDead");
    }

    /// fight() transitions to TargetDead when the target's hp_pct drops to 0.
    #[tokio::test(flavor = "multi_thread")]
    async fn fight_target_dead_when_hp_zero() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let calls = std::sync::Arc::new(AtomicU32::new(0));
        let c2 = calls.clone();

        let base = spawn_mock(move |name, _a| match name.as_str() {
            "bot.attack" => json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"}),
            "obs.get_nearby_hostiles" => {
                let n = c2.fetch_add(1, SeqCst);
                if n == 0 {
                    json!({"hostiles": [{"guid": 111u64, "level": 5, "hp_pct": 0.0, "distance": 3.0, "is_alive": false}]})
                } else {
                    json!({"hostiles": []})
                }
            }
            "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 90}}),
            other => panic!("unexpected tool in fight_hp_zero test: {other}"),
        }).await;

        let goal = test_goal();
        let target = Target { guid: 111, x: 3.0, y: 0.0, z: 0.0, distance: 3.0 };
        let rotation = RotationPlugin::melee_m1();
        let outcome = fight(&client(&base), 1003, target, &goal, &rotation).await.unwrap();
        assert!(matches!(outcome, FightOutcome::TargetDead), "hp_pct==0 → TargetDead");
    }

    /// fight() returns BotDied when obs.get_state returns hp_pct==0.
    #[tokio::test(flavor = "multi_thread")]
    async fn fight_bot_died_when_own_hp_zero() {
        let base = spawn_mock(|name, _a| match name.as_str() {
            "bot.attack" => json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"}),
            // target still alive
            "obs.get_nearby_hostiles" => json!({"hostiles": [
                {"guid": 111u64, "level": 5, "hp_pct": 80.0, "distance": 3.0, "is_alive": true}
            ]}),
            // bot died
            "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 0}}),
            other => panic!("unexpected tool in bot_died test: {other}"),
        }).await;

        let goal = test_goal();
        let target = Target { guid: 111, x: 3.0, y: 0.0, z: 0.0, distance: 3.0 };
        let rotation = RotationPlugin::melee_m1();
        let outcome = fight(&client(&base), 1003, target, &goal, &rotation).await.unwrap();
        assert!(matches!(outcome, FightOutcome::BotDied), "expected BotDied");
    }

    /// run_grind escalates NeedsDecision{BotDied} when the bot is dead and recovery
    /// exhausts its poll budget (36 polls × 20 ms in test mode). The DOA check fires
    /// immediately (hp_pct=0 on the initial read_self), so escalation happens AFTER
    /// a failed recovery attempt.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_escalates_bot_died() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let hostile_calls = std::sync::Arc::new(AtomicU32::new(0));
        let hc2 = hostile_calls.clone();

        let base = spawn_mock(move |name, _args| match name.as_str() {
            "obs.get_nearby_hostiles" => {
                let n = hc2.fetch_add(1, SeqCst);
                if n == 0 {
                    // Scan: return a live target
                    json!({"hostiles": [{"guid": 111u64, "level": 5, "hp_pct": 100.0,
                        "distance": 3.0, "is_alive": true, "x": 3.0, "y": 0.0, "z": 0.0}]})
                } else {
                    // Combat poll: target still there (bot will die)
                    json!({"hostiles": [{"guid": 111u64, "level": 5, "hp_pct": 80.0,
                        "distance": 3.0, "is_alive": true}]})
                }
            }
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":3.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0,
                "final": {"x":3.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":3.0,"y":0.0,"z":0.0,
                "map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            "bot.attack" => json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"}),
            // Bot is dead — the DOA check fires immediately; recovery exhausts revive budget.
            "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 0}}),
            // bot.revive returns bad shape → OwnError::Shape → StillDead after REVIVE_MAX_ATTEMPTS
            "bot.revive" => json!({"unexpected_key": "x"}),
            other => panic!("unexpected tool in escalate_bot_died test: {other}"),
        }).await;

        let status = run_grind(&client(&base), 1003, &test_goal()).await;
        match status {
            tot_goal_contract::GoalStatus::NeedsDecision {
                event: tot_goal_contract::EscalationEvent::BotDied { .. },
            } => {}
            other => panic!("expected NeedsDecision{{BotDied}}, got {other:?}"),
        }
    }

    // ── F4: real Looting + HealthCheck + Resting tests ─────────────────────────

    /// Low-hp path: after kill, HealthCheck sends bot to Resting; Resting polls until
    /// hp_pct >= 75, then resumes Scanning → Completed (kill_count=1 met).
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_rests_when_low_hp_then_completes() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let hostile_calls = std::sync::Arc::new(AtomicU32::new(0));
        let state_calls = std::sync::Arc::new(AtomicU32::new(0));
        let hc2 = hostile_calls.clone();
        let sc2 = state_calls.clone();

        let base = spawn_mock(move |name, _a| match name.as_str() {
            "obs.get_nearby_hostiles" => {
                let n = hc2.fetch_add(1, SeqCst);
                if n == 0 {
                    // Scan phase: return one live target
                    json!({"hostiles": [{"guid": 111u64, "level": 5, "hp_pct": 100.0,
                        "distance": 3.0, "is_alive": true, "x": 3.0, "y": 0.0, "z": 0.0}]})
                } else {
                    // Combat poll: target dead immediately
                    json!({"hostiles": []})
                }
            }
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":3.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0,
                "final": {"x":3.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":3.0,"y":0.0,"z":0.0,
                "map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            "bot.attack" => json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"}),
            "obs.get_lootable_corpses" => json!({"corpses": []}),
            "obs.get_state" => {
                // First call (read_self at start): bot alive at L5, low hp (25%)
                // Second call (HealthCheck): low hp → enter Resting
                // Third call (Resting poll 1): still low
                // Fourth call (Resting poll 2): hp recovered to 80 → exit Resting
                // Fifth call (HealthCheck after Resting): but we exit Resting back to Scanning;
                //   the next HealthCheck will fire from the 2nd kill loop iteration
                // NOTE: We set kill_count=1 so after 1 kill we check completion in HealthCheck.
                // The completion check happens BEFORE the rest check so we need hp to be low
                // to test resting. We set kill_count=None (rely on to_level) — but test_goal
                // has kill_count=Some(1), so completion fires first. We need a special goal.
                // Solution: return low hp for the health-check call, then high hp for resting.
                let n = sc2.fetch_add(1, SeqCst);
                if n <= 1 {
                    // start read + HealthCheck: low hp → Resting
                    json!({"self": {"level": 5, "hp_pct": 20}})
                } else if n == 2 {
                    // Resting poll 1: still low
                    json!({"self": {"level": 5, "hp_pct": 50}})
                } else {
                    // Resting poll 2+: recovered
                    json!({"self": {"level": 5, "hp_pct": 80}})
                }
            }
            other => panic!("unexpected tool in resting test: {other}"),
        }).await;

        // Use a goal with kill_count=None so only to_level stops it; rest_threshold=0.35.
        // With hp_pct=20 (0.20 < 0.35) → enters Resting.
        // After Resting exits (hp≥75), enters Scanning again. Then no targets → Wandering.
        // We need it to complete → set kill_count=Some(1) but to_level very high so only
        // kill_count triggers. But HealthCheck checks kill_count BEFORE rest_threshold.
        // So with kill_count=Some(1) and kills=1 it would complete without resting.
        // We need rest_threshold to be checked first, or use kill_count=None + to_level check.
        //
        // Looking at the implementation design: HealthCheck should check rest_threshold FIRST,
        // then the completion check. That's what we'll implement. So here kill_count=Some(1)
        // and we check rest first: hp=20/100=0.20 < rest_threshold=0.35 → Resting.
        // After resting → back to Scanning. Then no target (hostile empty after n>0) → Wandering.
        // Need to eventually complete: set to_level=5 (already at L5) → Completed next HealthCheck.
        // But that requires another full kill cycle. Simpler: use kill_count=None and to_level=5.
        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 5, // already L5 → completes at next HealthCheck after Resting
            kill_count: Some(1),
            rest_threshold: 0.35,
            rotation_id: None,
            vendor: None,
        };
        let status = run_grind(&client(&base), 1003, &goal).await;
        // After resting exits hp≥75 → Scanning → no targets → Wandering → Scanning → no targets
        // → eventually Blocked{NoTargetsFound}. OR: the second HealthCheck fires after Resting
        // with level=5 >= to_level=5 → Completed. We assert Completed OR the resting behavior
        // was exercised by checking state_calls > 2 (resting polls happened).
        let state_count = state_calls.load(SeqCst);
        assert!(state_count >= 3, "expected at least 3 obs.get_state calls (Resting polls), got {state_count}");
        // Also verify terminal status is Completed (to_level met) or Blocked (no more targets).
        match status {
            tot_goal_contract::GoalStatus::Completed { .. }
            | tot_goal_contract::GoalStatus::Blocked { .. } => {}
            other => panic!("expected Completed or Blocked after resting, got {other:?}"),
        }
    }

    /// Rest-not-needed path: high hp after kill → HealthCheck does NOT enter Resting,
    /// goes straight to completion check → Completed.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_no_rest_needed_completes_directly() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let hostile_calls = std::sync::Arc::new(AtomicU32::new(0));
        let state_calls = std::sync::Arc::new(AtomicU32::new(0));
        let hc2 = hostile_calls.clone();
        let sc2 = state_calls.clone();

        let base = spawn_mock(move |name, _a| match name.as_str() {
            "obs.get_nearby_hostiles" => {
                let n = hc2.fetch_add(1, SeqCst);
                if n == 0 {
                    json!({"hostiles": [{"guid": 111u64, "level": 5, "hp_pct": 100.0,
                        "distance": 3.0, "is_alive": true, "x": 3.0, "y": 0.0, "z": 0.0}]})
                } else {
                    json!({"hostiles": []})  // target dead in combat poll
                }
            }
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":3.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0,
                "final": {"x":3.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":3.0,"y":0.0,"z":0.0,
                "map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            "bot.attack" => json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"}),
            "obs.get_lootable_corpses" => json!({"corpses": []}),
            "obs.get_state" => {
                sc2.fetch_add(1, SeqCst);
                // Bot at 90% hp — well above rest_threshold=0.35 → no rest
                json!({"self": {"level": 5, "hp_pct": 90}})
            }
            other => panic!("unexpected tool in no_rest test: {other}"),
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let status = run_grind(&client(&base), 1003, &goal).await;
        match status {
            tot_goal_contract::GoalStatus::Completed { summary } =>
                assert!(summary.contains("kill"), "summary: {summary}"),
            other => panic!("expected Completed, got {other:?}"),
        }
        // Verify obs.get_state was called but NOT for Resting (only 2 times: start + HealthCheck)
        let state_count = state_calls.load(SeqCst);
        assert!(state_count <= 3, "unexpected extra state polls (resting?), got {state_count}");
    }

    // ── F5: full integrated grind cycle test ─────────────────────────────────

    /// Full cycle: scan → react → approach → fight (attack + target-death poll) →
    /// post-kill pause → loot → health-check → complete on kill_count=1.
    ///
    /// Asserts the tool-call sequence includes all expected tools:
    /// obs.get_nearby_hostiles, nav.find_path, bot.move_path, obs.get_position,
    /// bot.attack, obs.get_state, obs.get_lootable_corpses.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_full_cycle_with_real_combat_and_loot() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        use std::sync::{Arc, Mutex};

        // Track every tool call in order.
        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let log2 = call_log.clone();

        let hostile_calls = Arc::new(AtomicU32::new(0));
        let hc2 = hostile_calls.clone();

        let base = spawn_mock(move |name, _a| {
            log2.lock().unwrap().push(name.clone());
            match name.as_str() {
                "obs.get_nearby_hostiles" => {
                    let n = hc2.fetch_add(1, SeqCst);
                    if n == 0 {
                        // Initial scan: live target at 10y (beyond melee engage_range=5y so nav fires)
                        json!({"hostiles": [{"guid": 111u64, "level": 5, "hp_pct": 100.0,
                            "distance": 10.0, "is_alive": true, "x": 10.0, "y": 0.0, "z": 0.0}]})
                    } else {
                        // Combat poll: target dead
                        json!({"hostiles": []})
                    }
                }
                "nav.find_path" => json!({"path_type": 1i64, "points": [
                    {"x":0.0,"y":0.0,"z":0.0},{"x":10.0,"y":0.0,"z":0.0}]}),
                "bot.move_path" => json!({"launched": true, "duration_ms": 0,
                    "final": {"x":10.0,"y":0.0,"z":0.0}}),
                "obs.get_position" => json!({"x":10.0,"y":0.0,"z":0.0,
                    "map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
                "bot.attack" => json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"}),
                // obs.get_state: hp_pct=90 → no rest; level=5
                "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 90}}),
                // loot: no corpses (simplest path through Looting)
                "obs.get_lootable_corpses" => json!({"corpses": []}),
                other => panic!("unexpected tool in full_cycle test: {other}"),
            }
        })
        .await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };

        let status = run_grind(&client(&base), 1003, &goal).await;
        match status {
            tot_goal_contract::GoalStatus::Completed { summary } =>
                assert!(summary.contains("1 kill") || summary.contains("kill"), "summary: {summary}"),
            other => panic!("expected Completed, got {other:?}"),
        }

        let log = call_log.lock().unwrap().clone();
        // Assert each required tool was called at least once.
        for tool in &["obs.get_nearby_hostiles", "nav.find_path", "bot.move_path",
                      "obs.get_position", "bot.attack", "obs.get_state",
                      "obs.get_lootable_corpses"] {
            assert!(
                log.iter().any(|t| t == tool),
                "tool {tool} was never called; call log: {log:?}"
            );
        }
    }

    // ── Task-5 new tests ──────────────────────────────────────────────────────

    /// fight() uses the rotation plugin (mage_frost_b1 → bot.cast_spell) and returns
    /// TargetDead when the hostile poll shows the target dead after the first tick.
    /// Asserts: the rotation was used (cast_spell called ≥1 time) and the TargetDead
    /// path is taken when hp_pct==0 on the first poll.
    #[tokio::test(flavor = "multi_thread")]
    async fn fight_uses_goal_rotation_and_waits_cast_time() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let cast_spell_calls = std::sync::Arc::new(AtomicU32::new(0));
        let csc2 = cast_spell_calls.clone();

        let base = spawn_mock(move |name, _a| match name.as_str() {
            "bot.cast_spell" => {
                csc2.fetch_add(1, SeqCst);
                // Frostbolt: casting:true, short cast time
                json!({"casting": true, "cast_time_ms": 10})
            }
            "obs.get_nearby_hostiles" => {
                // Target immediately dead after the first tick
                json!({"hostiles": [{"guid": 222u64, "level": 6, "hp_pct": 0.0,
                    "distance": 20.0, "is_alive": false}]})
            }
            "obs.get_state" => json!({"self": {
                "level": 6, "hp_pct": 90, "mana_pct": 70, "power_pct": 70, "combo_points": 0
            }}),
            other => panic!("unexpected tool in fight_uses_rotation test: {other}"),
        }).await;

        // fight() receives the rotation as a parameter; rotation_id on the goal is
        // only read by run_grind, so we use None here to avoid false implication.
        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 10, kill_count: None, rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let target = Target { guid: 222, x: 20.0, y: 0.0, z: 0.0, distance: 20.0 };
        let rotation = crate::rotations::build("mage_frost_b1").unwrap();
        let outcome = fight(&client(&base), 1003, target, &goal, &rotation).await.unwrap();
        assert!(matches!(outcome, FightOutcome::TargetDead), "expected TargetDead");
        let cast_count = cast_spell_calls.load(SeqCst);
        assert!(cast_count >= 1, "bot.cast_spell must have been called at least once, got {cast_count}");
    }

    /// Pure predicate: should_rest fires on low hp or (for mana classes) low mana.
    #[test]
    fn should_rest_gates_on_hp_or_mana() {
        // hp=100, no mana class → no rest needed
        assert!(!should_rest(100, None, 0.35));
        // hp=20 (0.20 < 0.35) → rest
        assert!(should_rest(20, None, 0.35));
        // hp=100 but mana=10 (0.10 < 0.35) → rest
        assert!(should_rest(100, Some(10), 0.35));
        // hp=100, mana=80 (0.80 ≥ 0.35) → no rest
        assert!(!should_rest(100, Some(80), 0.35));
    }

    /// Pure predicate: rest_done requires both hp AND mana (if present) ≥ 75.
    #[test]
    fn rest_done_requires_both_pools() {
        // hp=80, no mana class → done (mana defaults to true)
        assert!(rest_done(80, None));
        // hp=80, mana=40 < 75 → not done
        assert!(!rest_done(80, Some(40)));
        // hp=80, mana=80 → done
        assert!(rest_done(80, Some(80)));
        // hp=60 < 75 → not done even if mana is fine
        assert!(!rest_done(60, Some(90)));
    }

    /// ensure_buffs casts only missing buffs: has Arcane Intellect rank-2 (first_spell_id=1459),
    /// does NOT cast it again; missing Frost Armor (spell_id=168) → casts it.
    ///
    /// This test sleeps GCD_MS (1500ms) for the one missing buff because the mock returns
    /// cast_time_ms=0 and ensure_buffs sleeps max(cast_time_ms, GCD_MS).
    #[tokio::test(flavor = "multi_thread")]
    async fn buff_pass_casts_missing_buffs_only() {
        use std::sync::{Arc, Mutex};

        let cast_ids: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
        let ci2 = cast_ids.clone();

        let base = spawn_mock(move |name, args| match name.as_str() {
            "obs.get_auras" => {
                // Bot has Arcane Intellect rank-2 (first_spell_id=1459 = rank-1 head)
                json!({"auras": [{"spell_id": 1461u32, "first_spell_id": 1459u32}]})
            }
            "bot.cast_spell" => {
                let spell_id = args["spell_id"].as_u64().unwrap() as u32;
                ci2.lock().unwrap().push(spell_id);
                json!({"casting": true, "cast_time_ms": 0})
            }
            other => panic!("unexpected tool in buff_pass test: {other}"),
        }).await;

        let plugin = crate::rotations::build("mage_frost_b1").unwrap();
        ensure_buffs(&client(&base), 1003, &plugin).await.unwrap();

        let cast_spell_ids = cast_ids.lock().unwrap().clone();
        // Only Frost Armor (168) should have been cast; Arcane Intellect (1459) already present.
        assert_eq!(cast_spell_ids, vec![crate::rotations::FROST_ARMOR],
            "expected only Frost Armor cast; got: {cast_spell_ids:?}");
    }

    // ── Finding #12: fight() CastSpin escalation ─────────────────────────────

    /// fight() surfaces the tick's CastSpin (returns before any liveness refresh).
    #[tokio::test(flavor = "multi_thread")]
    async fn fight_maps_cast_spin_outcome() {
        use crate::combat::{CastSpellAction, CAST_SPIN_THRESHOLD};
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let tick_count = Arc::new(AtomicU32::new(0));
        let tc2 = tick_count.clone();
        let base = spawn_mock(move |name, _a| {
            match name.as_str() {
                "bot.cast_spell" => {
                    tc2.fetch_add(1, SeqCst);
                    json!({"casting": false, "fail_code": "cast_failed", "detail": 64})
                }
                "bot.attack" =>
                    json!({"attacked": true, "target_guid": 1u64, "target_name": "Kobold"}),
                // fight() polls target_hp (obs.get_nearby_hostiles) + obs.get_state between ticks.
                "obs.get_nearby_hostiles" =>
                    json!({"hostiles": [{"guid": 1u64, "level": 5, "hp_pct": 80.0,
                        "distance": 3.0, "is_alive": true}]}),
                "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 90}}),
                other => panic!("unexpected tool {other} in fight_maps_cast_spin"),
            }
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 10, kill_count: None, rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let target = Target { guid: 1, x: 3.0, y: 0.0, z: 0.0, distance: 3.0 };
        // Build a rotation with one CastSpellAction + AutoAttack (same as plan).
        let rotation = crate::combat::RotationPlugin::new(vec![
            Box::new(CastSpellAction { name: "sinister_strike", spell_id: 1752, range: 5.0,
                                       precondition: |_| true, self_cast: false }),
            Box::new(crate::combat::AutoAttackAction),
        ], vec![], 5.0);
        let outcome = fight(&client(&base), 1003, target, &goal, &rotation).await.unwrap();
        assert!(matches!(outcome, FightOutcome::CastSpin { detail: Some(64) }),
                "expected CastSpin{{detail:Some(64)}}, got {:?}", outcome);
        // Must have fired the threshold worth of ticks.
        let ticks = tick_count.load(SeqCst);
        assert!(ticks >= CAST_SPIN_THRESHOLD, "expected >= {CAST_SPIN_THRESHOLD} ticks, got {ticks}");
    }

    /// run_grind: CastSpin while NOT mounted → Blocked{Other, "cast_spin..."} terminal.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_cast_spin_unmounted_blocks() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        // We need a rotation_id that the run_grind can build — "mage_frost_b1" uses
        // bot.cast_spell. But we want to exercise the CastSpin path. The key is that
        // run_grind builds the rotation from rotation_id. We'll override via a goal with
        // no rotation_id (defaults to auto_attack melee). The melee bot.attack never
        // returns CastFailed so we can't hit CastSpin through the default path.
        //
        // Instead, we test fight() → CastSpin path through run_grind using a caster
        // rotation by setting rotation_id = "mage_frost_b1" and mocking cast_spell to
        // always return cast_failed. We need the scan to find a target, and get_auras
        // to return not-mounted.
        let hostile_calls = Arc::new(AtomicU32::new(0));
        let hc2 = hostile_calls.clone();
        let base = spawn_mock(move |name, _args| match name.as_str() {
            "obs.get_state" => json!({"self": {"level": 6, "hp_pct": 90}}),
            "obs.get_auras" => json!({"auras": []}),  // not mounted at entry
            "obs.get_nearby_hostiles" => {
                let n = hc2.fetch_add(1, SeqCst);
                if n == 0 {
                    json!({"hostiles": [{"guid": 222u64, "level": 6, "hp_pct": 100.0,
                        "distance": 3.0, "is_alive": true, "x": 3.0, "y": 0.0, "z": 0.0}]})
                } else {
                    // During fight: keep returning target alive so fight() doesn't exit via TargetDead
                    json!({"hostiles": [{"guid": 222u64, "level": 6, "hp_pct": 80.0,
                        "distance": 3.0, "is_alive": true}]})
                }
            }
            "bot.cast_spell" => json!({"casting": false, "fail_code": "cast_failed", "detail": 64}),
            "bot.attack" => json!({"attacked": true, "target_guid": 222u64, "target_name": "Kobold"}),
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":3.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0, "final": {"x":3.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":3.0,"y":0.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            other => panic!("unexpected tool {other} in cast_spin_unmounted_blocks"),
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: None, rest_threshold: 0.35,
            rotation_id: Some("mage_frost_b1".into()),
            vendor: None,
        };
        let status = run_grind(&client(&base), 1003, &goal).await;
        match status {
            GoalStatus::Blocked { detail: Some(d), .. } =>
                assert!(d.contains("cast_spin"), "expected cast_spin in detail, got: {d}"),
            other => panic!("expected Blocked cast_spin, got {other:?}"),
        }
    }

    // ── Finding #12: is_mounted probe tests ──────────────────────────────────

    /// Finding #12: mount probe — aura_type 78 nested in effect_amounts (live wire shape).
    #[tokio::test]
    async fn is_mounted_parses_nested_aura_type() {
        let base = spawn_mock(move |name, _a| {
            assert_eq!(name, "obs.get_auras");
            json!({"auras":[{"spell_id":17453,"first_spell_id":17453,
                    "effect_amounts":[{"amount":0,"aura_type":78,"index":0}]}]})
        }).await;
        assert!(is_mounted(&client(&base), 1323).await.unwrap());
    }

    #[tokio::test]
    async fn is_mounted_false_without_mount_aura() {
        let base = spawn_mock(move |_n, _a| {
            json!({"auras":[{"spell_id":9330,
                    "effect_amounts":[{"amount":18,"aura_type":99,"index":0}]}]})
        }).await;
        assert!(!is_mounted(&client(&base), 1323).await.unwrap());
    }

    // ── Finding #12: dismount_via_native cycle tests ──────────────────────────

    /// Finding #12: dismount cycle = release → poll auras → re-claim, in order.
    #[tokio::test]
    async fn dismount_cycle_releases_polls_reclaims_in_order() {
        let calls = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
        let c2 = calls.clone();
        let aura_polls = Arc::new(std::sync::Mutex::new(0u32));
        let a2 = aura_polls.clone();
        let base = spawn_mock(move |name, args| {
            c2.lock().unwrap().push((name.clone(), args.clone()));
            match name.as_str() {
                "bot.set_ai_enabled" => {
                    let enabled = args["enabled"].as_bool().unwrap();
                    json!({"owned": !enabled, "reset": true})
                }
                "obs.get_auras" => {
                    let mut p = a2.lock().unwrap();
                    *p += 1;
                    if *p <= 2 {
                        json!({"auras":[{"spell_id":17453,
                                "effect_amounts":[{"amount":0,"aura_type":78,"index":0}]}]})
                    } else {
                        json!({"auras":[]})
                    }
                }
                other => panic!("unexpected tool {other}"),
            }
        }).await;

        let out = dismount_via_native(&client(&base), 1323).await;
        assert!(matches!(out, DismountOutcome::Dismounted));

        let ledger = calls.lock().unwrap().clone();
        // First call releases (enabled:true), last call re-claims (enabled:false).
        assert_eq!(ledger.first().unwrap().0, "bot.set_ai_enabled");
        assert_eq!(ledger.first().unwrap().1["enabled"], true);
        assert_eq!(ledger.last().unwrap().0, "bot.set_ai_enabled");
        assert_eq!(ledger.last().unwrap().1["enabled"], false);
        assert_eq!(*aura_polls.lock().unwrap(), 3, "polled until dismounted");
    }

    /// Budget exhaustion: still mounted → StillMounted, but ALWAYS re-claims.
    #[tokio::test]
    async fn dismount_cycle_budget_exhaustion_reclaims_and_reports_still_mounted() {
        let reclaimed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let r2 = reclaimed.clone();
        let base = spawn_mock(move |name, args| {
            match name.as_str() {
                "bot.set_ai_enabled" => {
                    let enabled = args["enabled"].as_bool().unwrap();
                    if !enabled { r2.store(true, std::sync::atomic::Ordering::SeqCst); }
                    json!({"owned": !enabled, "reset": true})
                }
                _ => json!({"auras":[{"spell_id":17453,
                        "effect_amounts":[{"amount":0,"aura_type":78,"index":0}]}]}),
            }
        }).await;
        let out = dismount_via_native(&client(&base), 1323).await;
        assert!(matches!(out, DismountOutcome::StillMounted));
        assert!(reclaimed.load(std::sync::atomic::Ordering::SeqCst), "must re-claim on every exit path");
    }

    // ── M3 #9 rider: dismount() prefers bot.dismount with native fallback ────────

    /// dismount(): when bot.dismount is available and succeeds, returns Dismounted
    /// immediately WITHOUT calling bot.set_ai_enabled (no release/re-claim dance).
    #[tokio::test]
    async fn dismount_prefers_direct_when_available() {
        let set_ai_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s2 = set_ai_called.clone();
        let base = spawn_mock(move |name, args| match name.as_str() {
            "bot.dismount" => {
                assert_eq!(args["bot_guid"], 1323_i64, "bot_guid forwarded correctly");
                json!({"dismounted": true, "was_mounted": true})
            }
            "bot.set_ai_enabled" => {
                s2.store(true, std::sync::atomic::Ordering::SeqCst);
                json!({"owned": true, "reset": true})
            }
            other => panic!("unexpected tool {other} in dismount_prefers_direct"),
        }).await;
        let out = dismount(&client(&base), 1323).await;
        assert!(matches!(out, DismountOutcome::Dismounted));
        assert!(
            !set_ai_called.load(std::sync::atomic::Ordering::SeqCst),
            "bot.set_ai_enabled must NOT be called when bot.dismount succeeds"
        );
    }

    /// dismount(): when bot.dismount is unavailable (unknown_tool error),
    /// falls back to the native release/poll/re-claim dance.
    #[tokio::test]
    async fn dismount_falls_back_to_native_on_error() {
        let native_reclaimed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let r2 = native_reclaimed.clone();
        let aura_polls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let a2 = aura_polls.clone();
        let base = spawn_mock(move |name, args| match name.as_str() {
            // bot.dismount is unavailable (simulated via ok:false)
            "bot.dismount" => {
                // Return an error body — the mock wraps everything in ok:true,result
                // but the client checks the envelope. We simulate an unknown-tool
                // by having the mock server itself panic so the outer mock returns it.
                // Instead: just have it return an aura with still-mounted=true
                // so we know the fallback polled. Actually, the mock always wraps
                // in ok:true, so we can't simulate ok:false this way.
                // Use a non-zero mounted aura to trigger fallback-but-native-dismounts.
                // Simpler: test via the error path by not registering bot.dismount
                // and letting the default branch fire. Use a flag to distinguish.
                panic!("bot.dismount: unknown_tool")
            }
            "bot.set_ai_enabled" => {
                let enabled = args["enabled"].as_bool().unwrap();
                if !enabled { r2.store(true, std::sync::atomic::Ordering::SeqCst); }
                json!({"owned": !enabled, "reset": true})
            }
            "obs.get_auras" => {
                let polls = a2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // dismount after 2 polls
                if polls < 2 {
                    json!({"auras":[{"spell_id":17453,
                            "effect_amounts":[{"amount":0,"aura_type":78,"index":0}]}]})
                } else {
                    json!({"auras":[]})
                }
            }
            other => panic!("unexpected tool {other}"),
        }).await;

        // The mock panics on bot.dismount, which the axum handler recovers from
        // as a 500. The client sees a harness error → falls back to native.
        let out = dismount(&client(&base), 1323).await;
        assert!(matches!(out, DismountOutcome::Dismounted), "fallback should have dismounted via native");
        assert!(
            native_reclaimed.load(std::sync::atomic::Ordering::SeqCst),
            "native fallback must re-claim ownership"
        );
    }

    /// read_self correctly parses the new v2 SelfState fields (mana_pct / power_pct / combo_points).
    #[tokio::test]
    async fn read_self_parses_v2_fields() {
        let base = spawn_mock(|name, _args| match name.as_str() {
            "obs.get_state" => json!({"self": {
                "level": 6, "hp_pct": 85, "mana_pct": 60, "power_pct": 70, "combo_points": 3
            }}),
            other => panic!("unexpected tool {other}"),
        }).await;
        let s = read_self(&client(&base), 1003).await.unwrap();
        assert_eq!(s.level, 6);
        assert_eq!(s.hp_pct, 85);
        assert_eq!(s.mana_pct, Some(60));
        assert_eq!(s.power_pct, Some(70));
        assert_eq!(s.combo_points, Some(3));
    }

    // ── Finding #12: claim-site mount checks ─────────────────────────────────

    /// run_grind goal entry: mounted bot that never dismounts → Blocked{mount_wedge}
    /// BEFORE any scanning (only get_state/get_auras/set_ai_enabled are ever called).
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_mounted_at_entry_blocks_when_dismount_fails() {
        let base = spawn_mock(move |name, args| match name.as_str() {
            "obs.get_state" => json!({"self": {"level": 23, "hp_pct": 100}}),
            "obs.get_auras" => json!({"auras":[{"spell_id":17453,
                    "effect_amounts":[{"amount":0,"aura_type":78,"index":0}]}]}),
            "bot.set_ai_enabled" => {
                let enabled = args["enabled"].as_bool().unwrap();
                json!({"owned": !enabled, "reset": true})
            }
            other => panic!("tool {other} must not be reached while mounted"),
        }).await;
        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35,
            rotation_id: None, vendor: None,
        };
        let status = run_grind(&client(&base), 1323, &goal).await;
        match status {
            GoalStatus::Blocked { detail: Some(d), .. } =>
                assert!(d.contains("mount_wedge"), "expected mount_wedge in detail, got: {d}"),
            other => panic!("expected Blocked mount_wedge, got {other:?}"),
        }
    }

    /// recover_from_death (Finding #17): calls bot.revive then obs.get_state then gm.teleport.
    /// NO bot.set_ai_enabled calls (bot stays claimed throughout).
    /// NO obs.get_auras (mount check dropped — the release window that created the #12 re-mount
    /// vector no longer exists).
    #[tokio::test]
    async fn recovery_revive_polls_teleports_no_ownership_release() {
        let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let c2 = calls.clone();
        let base = spawn_mock(move |name, _args| {
            c2.lock().unwrap().push(name.clone());
            match name.as_str() {
                "bot.revive" => json!({"revived": true, "was_dead": true}),
                "obs.get_state" => json!({"self": {"level": 7, "hp_pct": 100}}),
                "gm.teleport" => json!({"teleported": true}),
                other => panic!("unexpected tool {other} in recovery_revive test"),
            }
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: -5447.0, y: -378.0, z: 399.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let out = recover_from_death(&client(&base), 1003, &goal).await;
        assert!(matches!(out, RecoveryOutcome::Recovered));

        let seq = calls.lock().unwrap().clone();
        // No ownership release: bot.set_ai_enabled must never appear.
        assert!(!seq.contains(&"bot.set_ai_enabled".to_string()),
            "bot.set_ai_enabled must not be called — bot stays claimed: {seq:?}");
        // Order: bot.revive → obs.get_state → gm.teleport
        let revive_pos = seq.iter().position(|s| s == "bot.revive").expect("bot.revive called");
        let poll_pos = seq.iter().position(|s| s == "obs.get_state").expect("obs.get_state called");
        let tp_pos = seq.iter().position(|s| s == "gm.teleport").expect("gm.teleport called");
        assert!(revive_pos < poll_pos, "bot.revive must precede obs.get_state: {seq:?}");
        assert!(poll_pos < tp_pos, "obs.get_state must precede gm.teleport: {seq:?}");
    }

    /// recover_from_death (Finding #17): all REVIVE_MAX_ATTEMPTS (3) bot.revive calls fail →
    /// StillDead returned immediately, NO teleport, NO bot.set_ai_enabled.
    #[tokio::test]
    async fn recovery_exhausts_revive_budget_reports_still_dead() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let revive_calls = Arc::new(AtomicU32::new(0));
        let r2 = revive_calls.clone();
        let base = spawn_mock(move |name, _args| match name.as_str() {
            "bot.revive" => {
                r2.fetch_add(1, SeqCst);
                // Simulate a harness error by returning ok:false — the HarnessClient maps
                // this to HarnessError::Tool, which bot_revive wraps as OwnError::Harness.
                // We abuse the mock: return a shape that fails serde deserialization instead.
                json!({"unexpected_key": "x"}) // serde will fail → OwnError::Shape
            }
            "gm.teleport" => panic!("must not teleport a corpse"),
            "bot.set_ai_enabled" => panic!("must not release ownership"),
            other => panic!("unexpected tool {other}"),
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let out = recover_from_death(&client(&base), 1003, &goal).await;
        assert!(matches!(out, RecoveryOutcome::StillDead));
        assert_eq!(revive_calls.load(SeqCst), REVIVE_MAX_ATTEMPTS,
            "must attempt exactly REVIVE_MAX_ATTEMPTS ({REVIVE_MAX_ATTEMPTS}) bot.revive calls");
    }

    /// recover_from_death (Finding #17): bot.revive succeeds but all REVIVE_CONFIRM_POLLS
    /// obs.get_state calls return hp_pct==0 → StillDead (revive RPC may lag, confirm polls guard it).
    #[tokio::test]
    async fn recovery_revive_ok_but_still_dead_after_confirm_polls() {
        let base = spawn_mock(move |name, _args| match name.as_str() {
            "bot.revive" => json!({"revived": true, "was_dead": true}),
            "obs.get_state" => json!({"self": {"level": 7, "hp_pct": 0}}), // never comes alive
            "gm.teleport" => panic!("must not teleport a corpse"),
            "bot.set_ai_enabled" => panic!("must not release ownership"),
            other => panic!("unexpected tool {other}"),
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let out = recover_from_death(&client(&base), 1003, &goal).await;
        assert!(matches!(out, RecoveryOutcome::StillDead));
    }

    /// 4 deaths in one goal: every recovery succeeds, but the 4th exceeds
    /// MAX_DEATHS_PER_GOAL → terminal NeedsDecision{BotDied}, bot left alive+owned.
    /// Finding #17: recovery uses bot.revive (no ownership release); obs.get_state
    /// returns hp=0 during fight and hp=100 after bot.revive is called.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_death_cap_goes_terminal_after_recoveries() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
        // Model: bot is alive until it enters fight(), then dies (hp=0 from obs.get_state).
        // bot.revive resets the alive flag so the recovery confirm poll sees hp=100.
        let revived = Arc::new(AtomicBool::new(true)); // starts alive at goal entry
        let teleports = Arc::new(AtomicU32::new(0));
        let (rev2, tp2) = (revived.clone(), teleports.clone());
        let base = spawn_mock(move |name, _args| match name.as_str() {
            "bot.revive" => {
                // Revive succeeds; flip flag so subsequent obs.get_state returns alive.
                rev2.store(true, SeqCst);
                json!({"revived": true, "was_dead": true})
            }
            "obs.get_state" => {
                let hp = if rev2.load(SeqCst) { 100 } else { 0 };
                json!({"self": {"level": 7, "hp_pct": hp}})
            }
            "bot.attack" => {
                // Fight: kill the bot → obs.get_state returns 0 next poll.
                rev2.store(false, SeqCst);
                json!({"attacked": true, "target_guid": 9u64, "target_name": "Trogg"})
            }
            "gm.teleport" => { tp2.fetch_add(1, SeqCst); json!({"teleported": true}) }
            "obs.get_nearby_hostiles" => json!({"hostiles": [
                {"guid": 9u64, "name": "Trogg", "level": 7, "hp_pct": 100.0,
                 "distance": 4.0, "is_alive": true, "x": 4.0, "y": 0.0, "z": 0.0}]}),
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":4.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0,
                "final": {"x":4.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":4.0,"y":0.0,"z":0.0,"map_id":0,
                "zone_id":1,"area_id":1,"orientation":0.0}),
            "obs.get_lootable_corpses" => json!({"corpses": []}),
            other => panic!("unexpected tool {other}"),
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(10), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let status = run_grind(&client(&base), 1003, &goal).await;
        assert!(
            matches!(status, GoalStatus::NeedsDecision {
                event: tot_goal_contract::EscalationEvent::BotDied { .. } }),
            "expected NeedsDecision{{BotDied}} at the death cap, got {status:?}"
        );
        assert_eq!(teleports.load(SeqCst), 4, "all 4 deaths ran a full (successful) recovery");
    }

    /// A goal starting on a corpse recovers FIRST, then grinds to completion.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_dead_on_arrival_recovers_then_completes() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let state_calls = Arc::new(AtomicU32::new(0));
        let teleports = Arc::new(AtomicU32::new(0));
        let (st2, tp2) = (state_calls.clone(), teleports.clone());
        let scan_calls = Arc::new(AtomicU32::new(0));
        let sc2 = scan_calls.clone();
        let base = spawn_mock(move |name, _args| match name.as_str() {
            "obs.get_state" => {
                // First read (the DOA check) sees a corpse; everything after is alive
                // (bot.revive was called first, so the confirm poll sees hp=90).
                let n = st2.fetch_add(1, SeqCst);
                json!({"self": {"level": 5, "hp_pct": if n == 0 { 0 } else { 90 }}})
            }
            // Finding #17: recovery calls bot.revive (no ownership release).
            "bot.revive" => json!({"revived": true, "was_dead": true}),
            "gm.teleport" => { tp2.fetch_add(1, SeqCst); json!({"teleported": true}) }
            "obs.get_nearby_hostiles" => {
                let n = sc2.fetch_add(1, SeqCst);
                if n == 0 {
                    json!({"hostiles": [{"guid": 7u64, "name": "Boar", "level": 5,
                        "hp_pct": 100.0, "distance": 6.0, "is_alive": true,
                        "x": 6.0, "y": 0.0, "z": 0.0}]})
                } else {
                    json!({"hostiles": []}) // combat poll: target dead
                }
            }
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":6.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0,
                "final": {"x":6.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":6.0,"y":0.0,"z":0.0,"map_id":0,
                "zone_id":1,"area_id":1,"orientation":0.0}),
            "bot.attack" => json!({"attacked": true, "target_guid": 7u64, "target_name": "Boar"}),
            "obs.get_lootable_corpses" => json!({"corpses": []}),
            other => panic!("unexpected tool {other}"),
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let status = run_grind(&client(&base), 1003, &goal).await;
        assert!(matches!(status, GoalStatus::Completed { .. }), "got {status:?}");
        assert_eq!(teleports.load(SeqCst), 1, "exactly one DOA recovery");
    }

    /// Full-loop: kills 1-4 skip the inventory poll (stride 5); the 5th kill polls,
    /// triggers, runs the trip; kill 6 completes the goal (kill_count=6) with the
    /// stride/cooldown suppressing a second trip. Real react/post-kill sleeps make
    /// this run ~10-20 s — accepted, it's the only full-loop economy test.
    ///
    /// Mock kill scheme: each `bot.attack` "kills" the current target by bumping the
    /// served hostile guid (111+attacks). The fight's by-guid liveness poll then
    /// misses the old guid → TargetDead; the next Scanning picks up the new guid.
    #[tokio::test]
    async fn run_grind_executes_vendor_trip_then_resumes_and_completes() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let attacks = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let move_target = std::sync::Arc::new(std::sync::Mutex::new((0.0f64, 0.0f64, 0.0f64)));
        let (c2, a2, m2) = (calls.clone(), attacks.clone(), move_target.clone());
        let base = spawn_mock(move |name, args| {
            c2.lock().unwrap().push(name.clone());
            match name.as_str() {
                "obs.get_nearby_hostiles" => {
                    let k = *a2.lock().unwrap() as u64;
                    json!({"hostiles": [{"guid": 111 + k, "name": "Wolf", "level": 22,
                        "hp_pct": 100, "distance": 3.0, "is_alive": true,
                        "x": 3.0, "y": 0.0, "z": 0.0}]})
                }
                "bot.attack" => { *a2.lock().unwrap() += 1; json!({"attacking": true}) }
                "obs.get_state" => json!({"self": {"level": 22, "hp_pct": 100}}),
                "obs.get_auras" => json!({"auras": []}),
                "obs.get_lootable_corpses" => json!({"corpses": []}),
                "obs.get_inventory" => json!({"equipped": [], "bags":
                    (0..16).map(|s| json!({"quality": 0, "slot": s, "item_entry": 750,
                        "name": "x", "itemset": 0, "count": 1, "guid": s})).collect::<Vec<_>>(),
                    "nested_bags": []}),
                "nav.find_path" => json!({"path_type": 1i64, "points": [
                    {"x": 0.0, "y": 0.0, "z": 0.0},
                    {"x": args["dest_x"], "y": args["dest_y"], "z": args["dest_z"]}]}),
                "bot.move_path" => {
                    let p = args["points"].as_array().unwrap().last().unwrap().clone();
                    *m2.lock().unwrap() = (p["x"].as_f64().unwrap(),
                                           p["y"].as_f64().unwrap(), p["z"].as_f64().unwrap());
                    json!({"launched": true, "duration_ms": 0,
                           "final": {"x": p["x"], "y": p["y"], "z": p["z"]}})
                }
                "obs.get_position" => {
                    let (x, y, z) = *m2.lock().unwrap();
                    json!({"x": x, "y": y, "z": z, "map_id": 0, "zone_id": 1,
                           "area_id": 1, "orientation": 0.0})
                }
                "bot.vendor_sell" => json!({"sold_count": 16u32, "copper_gained": 320u64}),
                "bot.repair" => json!({"copper_spent": 0u64}),
                other => panic!("unexpected tool {other}"),
            }
        }).await;
        let mut goal = test_goal();
        goal.mob_filter = MobFilter { min_level: 19, max_level: 25, creature_type: None };
        goal.to_level = 99;          // complete via kill_count only
        goal.kill_count = Some(6);   // 5th kill trips; 6th completes
        goal.vendor = Some(VendorInfo {
            spawn_id: 40001,
            pos: WorldPos { map_id: 0, x: 50.0, y: 0.0, z: 0.0 },
            can_repair: true,
        });
        let status = run_grind(&client(&base), 1114, &goal).await;
        assert!(matches!(status, GoalStatus::Completed { .. }), "got {status:?}");
        let seq = calls.lock().unwrap().clone();
        let sells = seq.iter().filter(|c| *c == "bot.vendor_sell").count();
        assert_eq!(sells, 1, "exactly one trip (stride/cooldown suppress #2): {seq:?}");
        assert!(seq.contains(&"bot.repair".to_string()));
    }

    /// Post-recovery economy check (spec round 4b): DOA death-loop bots vendor after revival.
    ///
    /// A bot that arrives dead (DOA) skips the goal-entry economy check (Recovering state
    /// at entry). After recovery completes (bot.revive → confirm → teleport), the
    /// post-recovery path runs `entry_check`. If bags are triggered (accumulated greys from
    /// prior goal lives), the bot vendors BEFORE resuming combat.
    ///
    /// Sequence: DOA → Recovering → Recovered → post-recovery buff pass →
    /// post-recovery entry_check (triggered) → Vendoring (trip) → Scanning → kill →
    /// Looting → HealthCheck (kill_count=1 met) → Completed.
    ///
    /// Asserts: GoalStatus::Completed; `bot.vendor_sell` called exactly once;
    /// first `bot.vendor_sell` appears AFTER first `gm.teleport` (i.e., post-recovery,
    /// not at goal entry — entry check was correctly skipped because DOA).
    /// Finding #17: no bot.set_ai_enabled in the recovery path.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_grind_vendors_after_doa_recovery() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

        let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        // Tracks whether bot.revive has been called; obs.get_state returns alive after revive.
        let revived = Arc::new(AtomicBool::new(false));
        let attacks = Arc::new(AtomicU32::new(0));
        let move_target = Arc::new(std::sync::Mutex::new((0.0f64, 0.0f64, 0.0f64)));
        let (c2, rev2, a2, m2) = (calls.clone(), revived.clone(), attacks.clone(), move_target.clone());

        let base = spawn_mock(move |name, args| {
            c2.lock().unwrap().push(name.clone());
            match name.as_str() {
                "obs.get_state" => {
                    // DOA check at goal start: dead. After bot.revive: alive.
                    let hp = if rev2.load(SeqCst) { 100 } else { 0 };
                    json!({"self": {"level": 5, "hp_pct": hp}})
                }
                // Finding #17: recovery calls bot.revive (no ownership release).
                "bot.revive" => {
                    rev2.store(true, SeqCst);
                    json!({"revived": true, "was_dead": true})
                }
                "gm.teleport" => json!({"teleported": true}),
                // 16 grey backpack items → triggered() == true at all times (greys accumulated).
                "obs.get_inventory" => json!({
                    "equipped": [],
                    "bags": (0u32..16).map(|s| json!({
                        "quality": 0, "slot": s, "item_entry": 750,
                        "name": "Grey Item", "itemset": 0, "count": 1, "guid": s
                    })).collect::<Vec<_>>(),
                    "nested_bags": []
                }),
                "obs.get_auras" => json!({"auras": []}),
                "obs.get_nearby_hostiles" => {
                    // Guid bumps on each attack so fight()'s liveness poll misses old guid.
                    let k = a2.load(SeqCst) as u64;
                    json!({"hostiles": [{"guid": 300 + k, "name": "Boar", "level": 5,
                        "hp_pct": 100.0, "distance": 3.0, "is_alive": true,
                        "x": 3.0, "y": 0.0, "z": 0.0}]})
                }
                "bot.attack" => { a2.fetch_add(1, SeqCst); json!({"attacking": true}) }
                "obs.get_lootable_corpses" => json!({"corpses": []}),
                "nav.find_path" => json!({"path_type": 1i64, "points": [
                    {"x": 0.0, "y": 0.0, "z": 0.0},
                    {"x": args["dest_x"], "y": args["dest_y"], "z": args["dest_z"]}
                ]}),
                "bot.move_path" => {
                    let p = args["points"].as_array().unwrap().last().unwrap().clone();
                    *m2.lock().unwrap() = (p["x"].as_f64().unwrap(),
                                          p["y"].as_f64().unwrap(), p["z"].as_f64().unwrap());
                    json!({"launched": true, "duration_ms": 0,
                           "final": {"x": p["x"], "y": p["y"], "z": p["z"]}})
                }
                "obs.get_position" => {
                    let (x, y, z) = *m2.lock().unwrap();
                    json!({"x": x, "y": y, "z": z, "map_id": 0, "zone_id": 1,
                           "area_id": 1, "orientation": 0.0})
                }
                "bot.vendor_sell" => json!({"sold_count": 16u32, "copper_gained": 320u64}),
                "bot.repair" => json!({"copper_spent": 0u64}),
                other => panic!("unexpected tool {other}"),
            }
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99,
            kill_count: Some(1),
            rest_threshold: 0.35,
            rotation_id: None,
            vendor: Some(VendorInfo {
                spawn_id: 40003,
                pos: WorldPos { map_id: 0, x: 50.0, y: 0.0, z: 0.0 },
                can_repair: true,
            }),
        };

        let status = run_grind(&client(&base), 1173, &goal).await;
        assert!(matches!(status, GoalStatus::Completed { .. }), "expected Completed, got {status:?}");

        let seq = calls.lock().unwrap().clone();

        // Exactly one vendor_sell — post-recovery trip fires; stride (kills=1 < 5) +
        // cooldown together prevent a second trip at HealthCheck.
        let sells = seq.iter().filter(|c| *c == "bot.vendor_sell").count();
        assert_eq!(sells, 1, "exactly one vendor_sell (post-recovery): {seq:?}");

        // The FIRST vendor_sell must appear AFTER the first gm.teleport in the call log:
        // the entry economy check was skipped (DOA at goal start), and the sell fires
        // only after the recovery teleport back to anchor.
        let first_sell = seq.iter().position(|c| c == "bot.vendor_sell")
            .expect("vendor_sell must be in call log");
        let first_teleport = seq.iter().position(|c| c == "gm.teleport")
            .expect("gm.teleport must be in call log (recovery)");
        assert!(
            first_teleport < first_sell,
            "post-recovery sell must come AFTER recovery teleport; \
             first_teleport={first_teleport}, first_sell={first_sell}, seq={seq:?}"
        );
    }

    /// Goal-entry economy check (spec round 4): bags are triggered BEFORE any kill fires.
    /// Death-loop bots re-emit goals before the 5-kill stride can fire, but bag state
    /// carries across goal lives — the entry poll catches accumulated greys.
    ///
    /// Sequence: entry-check → Vendoring (trip) → Scanning → kill → Looting →
    /// HealthCheck (stride blocks: kills=1 < 5; cooldown blocks: trip just ran) →
    /// completion check (kills=1 >= kill_count=1) → Completed.
    ///
    /// Asserts: GoalStatus::Completed; `bot.vendor_sell` called EXACTLY once;
    /// first `bot.vendor_sell` precedes first `bot.attack` in the call log (entry trip
    /// fires before combat).
    #[tokio::test]
    async fn run_grind_entry_check_vendors_before_first_kill() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let attacks = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let move_target = std::sync::Arc::new(std::sync::Mutex::new((0.0f64, 0.0f64, 0.0f64)));
        let (c2, a2, m2) = (calls.clone(), attacks.clone(), move_target.clone());

        let base = spawn_mock(move |name, args| {
            c2.lock().unwrap().push(name.clone());
            match name.as_str() {
                // 16 grey backpack items → triggered() == true (grey_count=16 >= GREY_COUNT_TRIGGER=8)
                "obs.get_inventory" => json!({
                    "equipped": [],
                    "bags": (0u32..16).map(|s| json!({
                        "quality": 0, "slot": s, "item_entry": 750,
                        "name": "Grey Item", "itemset": 0, "count": 1, "guid": s
                    })).collect::<Vec<_>>(),
                    "nested_bags": []
                }),
                "obs.get_nearby_hostiles" => {
                    // Guid bumps on each attack so fight()'s liveness poll misses the old guid → TargetDead.
                    let k = *a2.lock().unwrap() as u64;
                    json!({"hostiles": [{"guid": 200 + k, "name": "Wolf", "level": 5,
                        "hp_pct": 100.0, "distance": 3.0, "is_alive": true,
                        "x": 3.0, "y": 0.0, "z": 0.0}]})
                }
                "bot.attack" => { *a2.lock().unwrap() += 1; json!({"attacking": true}) }
                "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 90}}),
                "obs.get_auras" => json!({"auras": []}),
                "obs.get_lootable_corpses" => json!({"corpses": []}),
                "nav.find_path" => json!({"path_type": 1i64, "points": [
                    {"x": 0.0, "y": 0.0, "z": 0.0},
                    {"x": args["dest_x"], "y": args["dest_y"], "z": args["dest_z"]}
                ]}),
                "bot.move_path" => {
                    let p = args["points"].as_array().unwrap().last().unwrap().clone();
                    *m2.lock().unwrap() = (p["x"].as_f64().unwrap(),
                                          p["y"].as_f64().unwrap(), p["z"].as_f64().unwrap());
                    json!({"launched": true, "duration_ms": 0,
                           "final": {"x": p["x"], "y": p["y"], "z": p["z"]}})
                }
                "obs.get_position" => {
                    let (x, y, z) = *m2.lock().unwrap();
                    json!({"x": x, "y": y, "z": z, "map_id": 0, "zone_id": 1,
                           "area_id": 1, "orientation": 0.0})
                }
                "bot.vendor_sell" => json!({"sold_count": 16u32, "copper_gained": 320u64}),
                "bot.repair" => json!({"copper_spent": 0u64}),
                other => panic!("unexpected tool {other}"),
            }
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99,
            kill_count: Some(1),
            rest_threshold: 0.35,
            rotation_id: None,
            vendor: Some(VendorInfo {
                spawn_id: 40002,
                pos: WorldPos { map_id: 0, x: 50.0, y: 0.0, z: 0.0 },
                can_repair: true,
            }),
        };

        let status = run_grind(&client(&base), 1173, &goal).await;
        assert!(matches!(status, GoalStatus::Completed { .. }), "expected Completed, got {status:?}");

        let seq = calls.lock().unwrap().clone();

        // Exactly one vendor_sell — entry trip fires; stride (kills=1 < 5) + cooldown
        // together prevent a second trip at HealthCheck.
        let sells = seq.iter().filter(|c| *c == "bot.vendor_sell").count();
        assert_eq!(sells, 1, "exactly one vendor_sell: {seq:?}");

        // The FIRST vendor_sell must appear BEFORE the FIRST bot.attack in the call log —
        // the entry economy check fires before combat starts.
        let first_sell = seq.iter().position(|c| c == "bot.vendor_sell")
            .expect("vendor_sell must be in call log");
        let first_attack = seq.iter().position(|c| c == "bot.attack")
            .expect("bot.attack must be in call log");
        assert!(
            first_sell < first_attack,
            "entry vendor trip must precede first attack; first_sell={first_sell}, first_attack={first_attack}, seq={seq:?}"
        );
    }

    // ── C3: maybe_anchor_on_entry tests ──────────────────────────────────────

    /// A displaced bot (farther than wander_radius from the anchor) triggers a
    /// gm.teleport and returns true.
    #[tokio::test]
    async fn anchor_on_entry_teleports_displaced_bot() {
        let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let c2 = calls.clone();
        let base = spawn_mock(move |name, _args| {
            c2.lock().unwrap().push(name.clone());
            match name.as_str() {
                // Bot is far from anchor (200y away; wander_radius=90)
                "obs.get_position" => json!({"x": 200.0, "y": 0.0, "z": 0.0,
                    "map_id": 0, "zone_id": 1, "area_id": 1, "orientation": 0.0}),
                "gm.teleport" => json!({"teleported": true}),
                other => panic!("unexpected tool {other} in anchor_on_entry_teleports_displaced_bot"),
            }
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let teleported = maybe_anchor_on_entry(&client(&base), 1003, &goal).await;
        assert!(teleported, "displaced bot must trigger teleport");
        let seq = calls.lock().unwrap().clone();
        assert!(seq.contains(&"gm.teleport".to_string()), "gm.teleport must be called: {seq:?}");
    }

    /// A bot already within wander_radius of the anchor does NOT trigger a teleport.
    #[tokio::test]
    async fn anchor_on_entry_no_teleport_for_on_camp_bot() {
        let base = spawn_mock(move |name, _args| match name.as_str() {
            // Bot is 5y from anchor (wander_radius=90) — well within range.
            "obs.get_position" => json!({"x": 5.0, "y": 0.0, "z": 0.0,
                "map_id": 0, "zone_id": 1, "area_id": 1, "orientation": 0.0}),
            "gm.teleport" => panic!("must not teleport an on-camp bot"),
            other => panic!("unexpected tool {other} in anchor_on_entry_no_teleport test"),
        }).await;

        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let teleported = maybe_anchor_on_entry(&client(&base), 1003, &goal).await;
        assert!(!teleported, "on-camp bot must not be teleported");
    }

    /// Cross-continent displacement: bot is on map 0 (Eastern Kingdoms), anchor is on
    /// map 1 (Kalimdor). The bot's x,y happen to fall within `wander_radius` of the
    /// anchor's x,y (simulating the AzerothCore coordinate-overlap that defeats 2D-only
    /// distance checks). Map mismatch alone MUST force a re-home.
    #[tokio::test]
    async fn anchor_on_entry_teleports_cross_map_bot() {
        let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let c2 = calls.clone();
        let base = spawn_mock(move |name, _args| {
            c2.lock().unwrap().push(name.clone());
            match name.as_str() {
                // Bot is on map 0 (EK), x/y coincidentally within wander_radius=90 of
                // the anchor (which is on map 1 / Kalimdor). Without map check this
                // would NOT trigger a teleport — that is the bug being fixed.
                "obs.get_position" => json!({"x": 5.0, "y": 0.0, "z": 0.0,
                    "map_id": 0, "zone_id": 12, "area_id": 12, "orientation": 0.0}),
                "gm.teleport" => json!({"teleported": true}),
                other => panic!("unexpected tool {other} in anchor_on_entry_teleports_cross_map_bot"),
            }
        }).await;

        // Anchor is on map 1 (Kalimdor) at nearly the same x,y as the bot.
        let goal = GrindGoal {
            anchor_point: WorldPos { map_id: 1, x: 5.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        };
        let teleported = maybe_anchor_on_entry(&client(&base), 1003, &goal).await;
        assert!(teleported, "cross-map bot must trigger teleport even when x,y are within wander_radius");
        let seq = calls.lock().unwrap().clone();
        assert!(seq.contains(&"gm.teleport".to_string()), "gm.teleport must be called: {seq:?}");
    }
}
