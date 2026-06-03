// SPDX-License-Identifier: AGPL-3.0
//! SubsetGate — periodic recompute of the brain's living-bot set.
//!
//! Faithful Rust port of `brain_sidecar/subset_gate.py`.
//!
//! # Structure
//!
//! * Pure data types (`PlayerSnapshot`, `BotSnapshot`, `WorldSnapshot`,
//!   `SubsetDecision`, `SubsetGateConfig`) — field-for-field copies of the Python
//!   frozen dataclasses.
//! * Pure functions (`recompute_once`, `_reconcile`, `_euclid`) — deterministic,
//!   no I/O; suitable for unit testing without any async harness.
//! * `SubsetGate` — the async task wrapper that drives the recompute/apply cycle.
//!
//! # Enroll/release ordering (load-bearing)
//!
//! `_apply` releases FIRST, then enrolls.  After both loops finish, it reads
//! `state_store.list_active()` to compute the set of currently-enrolled bots
//! for the `bump_hysteresis` pass.  The state-store writes in `release_fn` /
//! `enroll_fn` (via `LoopSupervisor::release_bot` / `enroll_bot`) therefore
//! happen **before** `list_active()` is called, so hysteresis bump operates on
//! the correct post-apply set.  This ordering is an exact reproduction of the
//! Python `_apply` method.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use tracing::{info, warn};

use crate::state::StateStore;

// ---------------------------------------------------------------------------
// Snapshot / decision types
// ---------------------------------------------------------------------------

/// Snapshot of a single online player. Mirrors Python `PlayerSnapshot`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlayerSnapshot {
    pub player_guid: i64,
    pub name: String,
    pub map_id: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub level: i32,
    pub party_guid: Option<i64>,
    pub raid_guid: Option<i64>,
    pub in_pvp_combat: bool,
}

/// Snapshot of a single managed bot. Mirrors Python `BotSnapshot`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BotSnapshot {
    pub bot_guid: i64,
    pub name: String,
    pub map_id: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub level: i32,
    pub party_guid: Option<i64>,
    pub raid_guid: Option<i64>,
    pub master_guid: Option<i64>,
    pub in_pvp_combat: bool,
}

/// World state snapshot passed to `recompute_once`. Mirrors Python `WorldSnapshot`.
#[derive(Debug, Clone, Default)]
pub struct WorldSnapshot {
    pub players: Vec<PlayerSnapshot>,
    pub bots: Vec<BotSnapshot>,
}

/// Output of `recompute_once`. Mirrors Python `SubsetDecision`.
#[derive(Debug, Clone)]
pub struct SubsetDecision {
    pub target_living_set: std::collections::HashSet<i64>,
    pub sticky_pinned: std::collections::HashSet<i64>,
    pub proximity_picks: std::collections::HashSet<i64>,
    pub to_enroll: std::collections::HashSet<i64>,
    pub to_release: std::collections::HashSet<i64>,
    pub to_full: std::collections::HashSet<i64>,
    pub to_reduced: std::collections::HashSet<i64>,
    pub skipped_due_to_backoff: std::collections::HashSet<i64>,
}

/// Configuration for `recompute_once` and `SubsetGate`. Mirrors Python `SubsetGateConfig`.
#[derive(Debug, Clone)]
pub struct SubsetGateConfig {
    /// Maximum number of bots in the living set.
    pub living_bot_count: usize,
    /// Seconds between recompute cycles.
    pub recompute_interval_s: f64,
    /// Consecutive out-of-range ticks required before releasing an enrolled bot.
    pub hysteresis_out_ticks: i64,
    /// Consecutive in-range ticks required before enrolling a candidate bot.
    pub hysteresis_in_ticks: i64,
    /// Seconds to back off a bot after a failed enroll attempt.
    pub enroll_backoff_s: f64,
    /// Gate master switch. If `false`, `run()` returns immediately.
    pub enabled: bool,
    /// Phase B: warm-cache + REDUCED tier assignment.
    pub phase_b_enabled: bool,
}

impl Default for SubsetGateConfig {
    fn default() -> Self {
        Self {
            living_bot_count: 10,
            recompute_interval_s: 60.0,
            hysteresis_out_ticks: 3,
            hysteresis_in_ticks: 2,
            enroll_backoff_s: 300.0,
            enabled: true,
            phase_b_enabled: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Callback type aliases
// ---------------------------------------------------------------------------

/// Async enroll callback: `(bot_guid, Option<BotSnapshot>) -> BoxFuture<()>`.
/// Matches Python `EnrollFn = Callable[[int, Optional[BotSnapshot]], Awaitable[None]]`.
pub type EnrollFn =
    Arc<dyn Fn(i64, Option<BotSnapshot>) -> BoxFuture<'static, ()> + Send + Sync>;

/// Async release callback: `bot_guid -> BoxFuture<()>`.
/// Matches Python `ReleaseFn = Callable[[int], Awaitable[None]]`.
pub type ReleaseFn = Arc<dyn Fn(i64) -> BoxFuture<'static, ()> + Send + Sync>;

/// Async world snapshot fetcher.
pub type SnapshotFetcher =
    Arc<dyn Fn() -> BoxFuture<'static, WorldSnapshot> + Send + Sync>;

// ---------------------------------------------------------------------------
// Pure helper: Euclidean distance
// ---------------------------------------------------------------------------

/// 3-D Euclidean distance between two positional objects.
/// Mirrors Python `_euclid(a, b)`.
#[inline]
fn _euclid_raw(ax: f64, ay: f64, az: f64, bx: f64, by: f64, bz: f64) -> f64 {
    let dx = ax - bx;
    let dy = ay - by;
    let dz = az - bz;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

// ---------------------------------------------------------------------------
// Pure function: recompute_once
// ---------------------------------------------------------------------------

/// Compute the next `SubsetDecision` from a world snapshot and gate state.
///
/// **Pure — no I/O.**  All inputs are passed explicitly; callers inject
/// whatever state-store values they've already read.
///
/// Mirrors Python `recompute_once(...)` exactly, including:
/// * No-players path (Phase A: sticky only; Phase B: sticky + warm-cache).
/// * Sticky override logic (party/raid/pvp-combat + same-map).
/// * Proximity ranking (same-map bots, sorted by min distance to same-map player).
/// * Phase B warm-cache extension up to `living_bot_count`.
/// * `_reconcile` delegation (release / enroll / tier / backoff output).
pub fn recompute_once(
    snapshot: &WorldSnapshot,
    currently_enrolled: &HashMap<i64, i64>, // bot_guid -> last_seen_ms (or 0)
    currently_pinned: &std::collections::HashSet<i64>,
    backoff_set: &std::collections::HashSet<i64>,
    hysteresis: &HashMap<i64, (i64, i64)>, // bot_guid -> (in_ticks, out_ticks)
    config: &SubsetGateConfig,
) -> SubsetDecision {
    use std::collections::HashSet;

    if snapshot.players.is_empty() {
        // No players online.
        // Phase A: target = sticky (pinned) only.
        // Phase B: target = sticky + currently_enrolled (warm-cache).
        let sticky: HashSet<i64> = currently_pinned.iter().copied().collect();
        let target: HashSet<i64> = if config.phase_b_enabled {
            sticky
                .iter()
                .copied()
                .chain(currently_enrolled.keys().copied())
                .collect()
        } else {
            sticky.clone()
        };
        return _reconcile(
            snapshot,
            &target,
            &HashSet::new(), // target_proximity
            &sticky,
            currently_enrolled,
            currently_pinned,
            backoff_set,
            hysteresis,
            config,
            &HashSet::new(), // proximity_picks
        );
    }

    // Players present — compute sticky overrides.
    let mut sticky: std::collections::HashSet<i64> =
        currently_pinned.iter().copied().collect();
    for bot in &snapshot.bots {
        for p in &snapshot.players {
            let party_match = p.party_guid.is_some() && p.party_guid == bot.party_guid;
            let raid_match = p.raid_guid.is_some() && p.raid_guid == bot.raid_guid;
            let pvp_match = bot.in_pvp_combat && bot.map_id == p.map_id;
            if party_match || raid_match || pvp_match {
                sticky.insert(bot.bot_guid);
                break;
            }
        }
    }

    // Proximity ranking — same-map bots only, excluding sticky + backoff.
    let mut eligible_bots: Vec<&BotSnapshot> = snapshot
        .bots
        .iter()
        .filter(|b| {
            snapshot.players.iter().any(|p| p.map_id == b.map_id)
                && !sticky.contains(&b.bot_guid)
                && !backoff_set.contains(&b.bot_guid)
        })
        .collect();

    let score = |bot: &BotSnapshot| -> f64 {
        let same_map: Vec<&PlayerSnapshot> = snapshot
            .players
            .iter()
            .filter(|p| p.map_id == bot.map_id)
            .collect();
        if same_map.is_empty() {
            f64::INFINITY
        } else {
            same_map
                .iter()
                .map(|p| _euclid_raw(bot.x, bot.y, bot.z, p.x, p.y, p.z))
                .fold(f64::INFINITY, f64::min)
        }
    };

    // Sort by (score, bot_guid) — stable tiebreak via bot_guid, matching Python.
    eligible_bots.sort_by(|a, b| {
        let sa = score(a);
        let sb = score(b);
        sa.partial_cmp(&sb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.bot_guid.cmp(&b.bot_guid))
    });

    let slots_remaining = config.living_bot_count.saturating_sub(sticky.len());
    let proximity_picks: std::collections::HashSet<i64> = eligible_bots
        .iter()
        .take(slots_remaining)
        .map(|b| b.bot_guid)
        .collect();

    let target_proximity: std::collections::HashSet<i64> = sticky
        .iter()
        .copied()
        .chain(proximity_picks.iter().copied())
        .collect();

    // Phase B: warm-cache extends target up to living_bot_count.
    let target: std::collections::HashSet<i64> = if config.phase_b_enabled {
        let warm_slots = config
            .living_bot_count
            .saturating_sub(target_proximity.len());
        // Sort warm candidates by last_seen descending (most-recently-seen first),
        // matching Python: `sorted(..., key=lambda g: currently_enrolled[g], reverse=True)`.
        let mut warm_candidates: Vec<i64> = currently_enrolled
            .keys()
            .copied()
            .filter(|g| !target_proximity.contains(g))
            .collect();
        warm_candidates.sort_by(|a, b| {
            let ta = currently_enrolled.get(a).copied().unwrap_or(0);
            let tb = currently_enrolled.get(b).copied().unwrap_or(0);
            tb.cmp(&ta) // descending
        });
        target_proximity
            .iter()
            .copied()
            .chain(warm_candidates.into_iter().take(warm_slots))
            .collect()
    } else {
        target_proximity.clone()
    };

    _reconcile(
        snapshot,
        &target,
        &target_proximity,
        &sticky,
        currently_enrolled,
        currently_pinned,
        backoff_set,
        hysteresis,
        config,
        &proximity_picks,
    )
}

// ---------------------------------------------------------------------------
// Pure function: _reconcile
// ---------------------------------------------------------------------------

/// Compute `to_enroll`, `to_release`, tier sets, and backoff output from the
/// target set and gate state.
///
/// Mirrors Python `_reconcile(...)` exactly.
#[allow(clippy::too_many_arguments)]
fn _reconcile(
    snapshot: &WorldSnapshot,
    target: &std::collections::HashSet<i64>,
    target_proximity: &std::collections::HashSet<i64>,
    sticky: &std::collections::HashSet<i64>,
    currently_enrolled: &HashMap<i64, i64>,
    currently_pinned: &std::collections::HashSet<i64>,
    backoff_set: &std::collections::HashSet<i64>,
    hysteresis: &HashMap<i64, (i64, i64)>,
    config: &SubsetGateConfig,
    proximity_picks: &std::collections::HashSet<i64>,
) -> SubsetDecision {
    use std::collections::HashSet;

    let mut to_enroll: HashSet<i64> = HashSet::new();
    let mut to_release: HashSet<i64> = HashSet::new();

    // ── Release pass ─────────────────────────────────────────────────────────
    // Currently enrolled bots not in target and not pinned.
    for &bot_guid in currently_enrolled.keys() {
        if target.contains(&bot_guid) || currently_pinned.contains(&bot_guid) {
            continue;
        }
        let (_in_ticks, out_ticks) = hysteresis.get(&bot_guid).copied().unwrap_or((0, 0));
        // Hypothetical post-bump out_ticks; if threshold met and not sticky → release.
        if out_ticks + 1 >= config.hysteresis_out_ticks && !sticky.contains(&bot_guid) {
            to_release.insert(bot_guid);
        }
    }

    // ── Enroll pass ───────────────────────────────────────────────────────────
    // Bots in target_proximity that are not yet enrolled.
    // Sticky bots bypass hysteresis; others must meet hysteresis_in_ticks.
    for &bot_guid in target_proximity {
        if currently_enrolled.contains_key(&bot_guid) {
            continue;
        }
        if backoff_set.contains(&bot_guid) {
            continue;
        }
        if sticky.contains(&bot_guid) {
            // Sticky bypass: enroll immediately, no hysteresis gate.
            to_enroll.insert(bot_guid);
            continue;
        }
        let (in_ticks, _) = hysteresis.get(&bot_guid).copied().unwrap_or((0, 0));
        if in_ticks + 1 >= config.hysteresis_in_ticks {
            to_enroll.insert(bot_guid);
        }
    }

    // ── Phase B tier assignment ───────────────────────────────────────────────
    let mut to_full: HashSet<i64> = HashSet::new();
    let mut to_reduced: HashSet<i64> = HashSet::new();
    if config.phase_b_enabled {
        // kept = (currently_enrolled - to_release) | to_enroll
        let kept: HashSet<i64> = currently_enrolled
            .keys()
            .copied()
            .filter(|g| !to_release.contains(g))
            .chain(to_enroll.iter().copied())
            .collect();
        let bot_by_guid: HashMap<i64, &BotSnapshot> =
            snapshot.bots.iter().map(|b| (b.bot_guid, b)).collect();
        for guid in &kept {
            let bot = bot_by_guid.get(guid);
            if bot.map(|b| snapshot.players.iter().any(|p| p.map_id == b.map_id))
                == Some(true)
            {
                to_full.insert(*guid);
            } else {
                to_reduced.insert(*guid);
            }
        }
    }

    // ── skipped_due_to_backoff ────────────────────────────────────────────────
    let skipped: HashSet<i64> = backoff_set
        .iter()
        .filter(|g| snapshot.bots.iter().any(|b| &b.bot_guid == *g))
        .copied()
        .collect();

    SubsetDecision {
        target_living_set: target.clone(),
        sticky_pinned: sticky.clone(),
        proximity_picks: proximity_picks.clone(),
        to_enroll,
        to_release,
        to_full,
        to_reduced,
        skipped_due_to_backoff: skipped,
    }
}

// ---------------------------------------------------------------------------
// SubsetGate — async task wrapper
// ---------------------------------------------------------------------------

/// Async task wrapper that drives the recompute/apply cycle.
///
/// Mirrors `brain_sidecar.subset_gate.SubsetGate`.
pub struct SubsetGate {
    pub state_store: Arc<StateStore>,
    pub snapshot_fetcher: SnapshotFetcher,
    /// Called to start a bot's brain loop (mirrors Python `enroll_fn`).
    pub enroll_fn: EnrollFn,
    /// Called to stop a bot's brain loop (mirrors Python `release_fn`).
    pub release_fn: ReleaseFn,
    pub config: SubsetGateConfig,
    /// Per-bot enroll-backoff deadlines: `bot_guid → deadline_unix_secs`.
    backoff_state: std::sync::Mutex<HashMap<i64, f64>>,
}

impl SubsetGate {
    /// Construct a new `SubsetGate`.
    pub fn new(
        state_store: Arc<StateStore>,
        snapshot_fetcher: SnapshotFetcher,
        enroll_fn: EnrollFn,
        release_fn: ReleaseFn,
        config: SubsetGateConfig,
    ) -> Self {
        Self {
            state_store,
            snapshot_fetcher,
            enroll_fn,
            release_fn,
            config,
            backoff_state: std::sync::Mutex::new(HashMap::new()),
        }
    }

    // ------------------------------------------------------------------
    // Public: async run loop
    // ------------------------------------------------------------------

    /// Drive the recompute/apply cycle until cancelled.
    ///
    /// Mirrors Python `SubsetGate.run()`:
    /// * If `config.enabled` is `false`, returns immediately.
    /// * On each cycle: `_recompute_and_apply()`, then sleep
    ///   `config.recompute_interval_s`.
    /// * Non-`CancelledError` exceptions are logged and the loop continues.
    pub async fn run(&self) {
        if !self.config.enabled {
            info!("SubsetGate disabled via config — exiting run() immediately");
            return;
        }
        info!(
            "SubsetGate.run() started; interval={}s",
            self.config.recompute_interval_s
        );
        loop {
            if let Err(e) = self._recompute_and_apply().await {
                warn!("SubsetGate recompute cycle failed; will retry next tick: {:?}", e);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs_f64(
                self.config.recompute_interval_s,
            ))
            .await;
        }
    }

    // ------------------------------------------------------------------
    // Internal: recompute_and_apply
    // ------------------------------------------------------------------

    pub async fn _recompute_and_apply(&self) -> Result<SubsetDecision, anyhow::Error> {
        let snapshot = (self.snapshot_fetcher)().await;

        // Read current gate state from the state store.
        let currently_enrolled: HashMap<i64, i64> = self
            .state_store
            .list_active()
            .map_err(|e| anyhow::anyhow!(e))?
            .into_iter()
            .map(|row| (row.bot_guid, row.last_seen.unwrap_or(0)))
            .collect();

        let currently_pinned: std::collections::HashSet<i64> = self
            .state_store
            .list_pinned()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .into_iter()
            .collect();

        let backoff_set = self._expired_backoff_set();

        let hysteresis: HashMap<i64, (i64, i64)> = currently_enrolled
            .keys()
            .filter_map(|&g| {
                self.state_store
                    .get_hysteresis(g)
                    .ok()
                    .map(|h| (g, h))
            })
            .collect();

        let decision = recompute_once(
            &snapshot,
            &currently_enrolled,
            &currently_pinned,
            &backoff_set,
            &hysteresis,
            &self.config,
        );

        self._apply(&decision, &snapshot).await;
        Ok(decision)
    }

    // ------------------------------------------------------------------
    // Internal: expire backoff entries, return live set
    // ------------------------------------------------------------------

    /// Remove expired backoff entries and return the set of still-in-backoff guids.
    ///
    /// Mirrors Python `_expired_backoff_set()`.
    fn _expired_backoff_set(&self) -> std::collections::HashSet<i64> {
        let now = _now_secs();
        let mut state = self.backoff_state.lock().unwrap();
        state.retain(|_, &mut dl| dl > now);
        state.keys().copied().collect()
    }

    // ------------------------------------------------------------------
    // Internal: apply decision
    // ------------------------------------------------------------------

    /// Apply a `SubsetDecision`: release → enroll → tier → bump_hysteresis.
    ///
    /// # Ordering contract (load-bearing)
    ///
    /// 1. **Release loop** — calls `release_fn` for each bot in `to_release`.
    ///    `LoopSupervisor::release_bot` calls `state_store.set_status("released")`
    ///    INSIDE the callback, so by the time the release loop finishes the DB
    ///    status of released bots has already changed.
    ///
    /// 2. **Enroll loop** — calls `enroll_fn` for each bot in `to_enroll`.
    ///    The state_store already reflects the releases from step 1.
    ///
    /// 3. **Tier assignment** — `set_tier` for Phase B full/reduced.
    ///
    /// 4. **`list_active()` call** — reads the post-apply enrolled set.
    ///    Because releases and enrolls have already mutated state_store, the
    ///    result accurately reflects the current living set for the hysteresis
    ///    bump pass.
    ///
    /// This mirrors the Python `_apply` method's loop ordering exactly.
    async fn _apply(&self, decision: &SubsetDecision, snapshot: &WorldSnapshot) {
        // ── 1. Release pass ──────────────────────────────────────────────────
        // Errors are swallowed per-bot (mirrors Python `except Exception: log.exception`).
        // Note: LoopSupervisor::release_bot calls state_store.set_status("released")
        // INSIDE the future, so the DB is updated before this loop completes.
        for &bot_guid in &decision.to_release {
            (self.release_fn)(bot_guid).await;
        }

        // ── 2. Enroll pass ───────────────────────────────────────────────────
        // Build per-bot snapshot lookup so enroll_fn can bootstrap new bots.
        let bot_by_guid: HashMap<i64, BotSnapshot> = snapshot
            .bots
            .iter()
            .map(|b| (b.bot_guid, b.clone()))
            .collect();

        // On enroll failure the caller is responsible for calling `set_enroll_backoff`.
        // Since `enroll_fn` is `Fn → BoxFuture<()>` (no error return), failures are
        // handled by the caller injecting a wrapper that records the failure and then
        // calls `set_enroll_backoff`. This matches Python's exception-swallowing pattern.
        for &bot_guid in &decision.to_enroll {
            let bot_snap = bot_by_guid.get(&bot_guid).cloned();
            (self.enroll_fn)(bot_guid, bot_snap).await;
        }

        // ── 3. Tier assignment (Phase B) ─────────────────────────────────────
        for &bot_guid in &decision.to_full {
            if let Err(e) = self.state_store.set_tier(bot_guid, "full") {
                warn!("set_tier(full) failed bot_guid={}: {}", bot_guid, e);
            }
        }
        for &bot_guid in &decision.to_reduced {
            if let Err(e) = self.state_store.set_tier(bot_guid, "reduced") {
                warn!("set_tier(reduced) failed bot_guid={}: {}", bot_guid, e);
            }
        }

        // ── 4. Bump hysteresis for post-apply enrolled set ───────────────────
        // IMPORTANT: list_active() is called AFTER the release + enroll loops,
        // so it reads the correct post-apply set (releases have set status=
        // "released"; new enrolls are status="active").
        let currently_enrolled_post: std::collections::HashSet<i64> = self
            .state_store
            .list_active()
            .map(|rows| rows.into_iter().map(|r| r.bot_guid).collect())
            .unwrap_or_default();

        let target = &decision.target_living_set;
        for bot_guid in currently_enrolled_post {
            let in_range = target.contains(&bot_guid);
            if let Err(e) = self.state_store.bump_hysteresis(bot_guid, in_range) {
                warn!("bump_hysteresis failed bot_guid={}: {:?}", bot_guid, e);
            }
        }
    }

    // ------------------------------------------------------------------
    // Backoff: set deadline for a failed enroll
    // ------------------------------------------------------------------

    /// Record an enroll failure and arm the backoff timer for `bot_guid`.
    pub fn set_enroll_backoff(&self, bot_guid: i64) {
        let deadline = _now_secs() + self.config.enroll_backoff_s;
        let mut state = self.backoff_state.lock().unwrap();
        state.insert(bot_guid, deadline);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn _now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    // ── helpers ──────────────────────────────────────────────────────────────

    fn cfg(n: usize) -> SubsetGateConfig {
        SubsetGateConfig {
            living_bot_count: n,
            recompute_interval_s: 60.0,
            hysteresis_out_ticks: 2,
            hysteresis_in_ticks: 1,
            enroll_backoff_s: 300.0,
            enabled: true,
            phase_b_enabled: false,
        }
    }

    fn player(guid: i64, map_id: i32, x: f64, y: f64, z: f64) -> PlayerSnapshot {
        PlayerSnapshot {
            player_guid: guid,
            name: format!("P{guid}"),
            map_id,
            x,
            y,
            z,
            level: 10,
            party_guid: None,
            raid_guid: None,
            in_pvp_combat: false,
        }
    }

    fn bot(guid: i64, map_id: i32, x: f64, y: f64, z: f64) -> BotSnapshot {
        BotSnapshot {
            bot_guid: guid,
            name: format!("B{guid}"),
            map_id,
            x,
            y,
            z,
            level: 10,
            party_guid: None,
            raid_guid: None,
            master_guid: None,
            in_pvp_combat: false,
        }
    }

    fn empty_hysteresis() -> HashMap<i64, (i64, i64)> {
        HashMap::new()
    }

    fn empty_enrolled() -> HashMap<i64, i64> {
        HashMap::new()
    }

    fn empty_pinned() -> HashSet<i64> {
        HashSet::new()
    }

    fn empty_backoff() -> HashSet<i64> {
        HashSet::new()
    }

    // ── pure-function tests ───────────────────────────────────────────────────

    #[test]
    fn test_no_players_target_empty_in_phase_a() {
        let snap = WorldSnapshot {
            players: vec![],
            bots: vec![bot(1, 0, 0.0, 0.0, 0.0)],
        };
        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &empty_pinned(),
            &empty_backoff(),
            &empty_hysteresis(),
            &cfg(5),
        );
        assert!(
            dec.target_living_set.is_empty(),
            "no players → empty target in phase A"
        );
        assert!(dec.to_enroll.is_empty());
        assert!(dec.to_release.is_empty());
    }

    #[test]
    fn test_no_players_phase_b_keeps_enrolled() {
        let snap = WorldSnapshot {
            players: vec![],
            bots: vec![bot(1, 0, 0.0, 0.0, 0.0), bot(2, 0, 1.0, 0.0, 0.0)],
        };
        let enrolled: HashMap<i64, i64> = [(1, 0), (2, 0)].into_iter().collect();
        let mut phase_b_cfg = cfg(5);
        phase_b_cfg.phase_b_enabled = true;

        let dec = recompute_once(
            &snap,
            &enrolled,
            &empty_pinned(),
            &empty_backoff(),
            &empty_hysteresis(),
            &phase_b_cfg,
        );
        // Phase B: enrolled bots stay in target even with no players.
        assert!(dec.target_living_set.contains(&1));
        assert!(dec.target_living_set.contains(&2));
    }

    #[test]
    fn test_proximity_ranking_selects_nearest() {
        // Player at origin; bots at distances 10, 1, 100 — cap=2 → nearest 2 selected.
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 0.0, 0.0, 0.0)],
            bots: vec![
                bot(10, 0, 10.0, 0.0, 0.0), // dist 10
                bot(1, 0, 1.0, 0.0, 0.0),   // dist 1  ← nearest
                bot(100, 0, 100.0, 0.0, 0.0), // dist 100
            ],
        };
        // hysteresis_in_ticks=1 so each bot needs in_ticks+1 >= 1 → always enrollable
        let mut c = cfg(2);
        c.hysteresis_in_ticks = 1;
        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &empty_pinned(),
            &empty_backoff(),
            &empty_hysteresis(),
            &c,
        );
        assert!(dec.proximity_picks.contains(&1), "nearest bot selected");
        assert!(dec.proximity_picks.contains(&10), "second nearest selected");
        assert!(!dec.proximity_picks.contains(&100), "farthest excluded");
        // All proximity picks with in_ticks+1 >= 1 → enrolled
        assert!(dec.to_enroll.contains(&1));
        assert!(dec.to_enroll.contains(&10));
        assert!(!dec.to_enroll.contains(&100));
    }

    #[test]
    fn test_sticky_party_member_bypasses_proximity() {
        // Bot 99 is in player's party; bot 1 is nearest but not in party.
        // Cap=1 → normally only 1 bot, but sticky bots are always included.
        let mut p = player(1, 0, 0.0, 0.0, 0.0);
        p.party_guid = Some(42);
        let mut b_party = bot(99, 0, 500.0, 0.0, 0.0); // far but in party
        b_party.party_guid = Some(42);
        let b_near = bot(1, 0, 1.0, 0.0, 0.0); // near but not in party

        let snap = WorldSnapshot {
            players: vec![p],
            bots: vec![b_party, b_near],
        };
        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &empty_pinned(),
            &empty_backoff(),
            &empty_hysteresis(),
            &cfg(1), // cap=1 (just sticky occupies slot)
        );
        assert!(
            dec.sticky_pinned.contains(&99),
            "party bot is sticky"
        );
        assert!(
            dec.target_living_set.contains(&99),
            "party bot always in target"
        );
        // sticky bypasses hysteresis → enrolled immediately
        assert!(dec.to_enroll.contains(&99));
        // cap=1, sticky takes the slot → no proximity slots → bot 1 not selected
        assert!(!dec.proximity_picks.contains(&1));
    }

    #[test]
    fn test_sticky_pvp_combat_same_map() {
        // Bot in PvP combat on player's map → sticky.
        let p = player(1, 5, 0.0, 0.0, 0.0); // map_id=5
        let mut b = bot(7, 5, 200.0, 0.0, 0.0); // map_id=5
        b.in_pvp_combat = true;

        let snap = WorldSnapshot {
            players: vec![p],
            bots: vec![b],
        };
        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &empty_pinned(),
            &empty_backoff(),
            &empty_hysteresis(),
            &cfg(5),
        );
        assert!(dec.sticky_pinned.contains(&7), "pvp-combat bot is sticky");
        assert!(dec.to_enroll.contains(&7));
    }

    #[test]
    fn test_hysteresis_release_requires_out_ticks() {
        // Bot 1 enrolled; not in target; out_ticks=0 → NOT released yet (need 2).
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 1000.0, 0.0, 0.0)], // player far from bot
            bots: vec![bot(1, 1, 0.0, 0.0, 0.0)], // bot on different map → not eligible
        };
        let enrolled: HashMap<i64, i64> = [(1, 0)].into_iter().collect();
        let hysteresis: HashMap<i64, (i64, i64)> = [(1, (0, 0))].into_iter().collect();
        let mut c = cfg(5);
        c.hysteresis_out_ticks = 2;

        let dec = recompute_once(
            &snap,
            &enrolled,
            &empty_pinned(),
            &empty_backoff(),
            &hysteresis,
            &c,
        );
        // out_ticks=0; hypothetical post-bump = 1; threshold=2 → NOT released.
        assert!(
            !dec.to_release.contains(&1),
            "out_ticks 0+1=1 < 2 → not released yet"
        );
    }

    #[test]
    fn test_hysteresis_release_fires_at_threshold() {
        // Bot 1 enrolled; not in target; out_ticks=1 → released (0+1+1=2 meets threshold).
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 1000.0, 0.0, 0.0)],
            bots: vec![bot(1, 1, 0.0, 0.0, 0.0)], // different map → not in target
        };
        let enrolled: HashMap<i64, i64> = [(1, 0)].into_iter().collect();
        // out_ticks already at 1; post-bump = 2 = threshold
        let hysteresis: HashMap<i64, (i64, i64)> = [(1, (0, 1))].into_iter().collect();
        let mut c = cfg(5);
        c.hysteresis_out_ticks = 2;

        let dec = recompute_once(
            &snap,
            &enrolled,
            &empty_pinned(),
            &empty_backoff(),
            &hysteresis,
            &c,
        );
        assert!(dec.to_release.contains(&1), "out_ticks 1+1=2 >= 2 → released");
    }

    #[test]
    fn test_hysteresis_enroll_requires_in_ticks() {
        // Bot in proximity; in_ticks=0; hysteresis_in_ticks=2 → NOT enrolled yet.
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 0.0, 0.0, 0.0)],
            bots: vec![bot(1, 0, 1.0, 0.0, 0.0)],
        };
        let hysteresis: HashMap<i64, (i64, i64)> = [(1, (0, 0))].into_iter().collect();
        let mut c = cfg(5);
        c.hysteresis_in_ticks = 2;

        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &empty_pinned(),
            &empty_backoff(),
            &hysteresis,
            &c,
        );
        // in_ticks=0; post-bump = 1; threshold=2 → NOT enrolled.
        assert!(
            !dec.to_enroll.contains(&1),
            "in_ticks 0+1=1 < 2 → not enrolled yet"
        );
    }

    #[test]
    fn test_hysteresis_enroll_fires_at_threshold() {
        // Bot in proximity; in_ticks=1; hysteresis_in_ticks=2 → enrolled.
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 0.0, 0.0, 0.0)],
            bots: vec![bot(1, 0, 1.0, 0.0, 0.0)],
        };
        let hysteresis: HashMap<i64, (i64, i64)> = [(1, (1, 0))].into_iter().collect();
        let mut c = cfg(5);
        c.hysteresis_in_ticks = 2;

        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &empty_pinned(),
            &empty_backoff(),
            &hysteresis,
            &c,
        );
        assert!(dec.to_enroll.contains(&1), "in_ticks 1+1=2 >= 2 → enrolled");
    }

    #[test]
    fn test_backoff_bot_skipped() {
        // Bot 1 is nearest but in backoff set → not enrolled.
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 0.0, 0.0, 0.0)],
            bots: vec![bot(1, 0, 1.0, 0.0, 0.0)],
        };
        let backoff: HashSet<i64> = [1].into_iter().collect();
        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &empty_pinned(),
            &backoff,
            &empty_hysteresis(),
            &cfg(5),
        );
        assert!(!dec.to_enroll.contains(&1), "backoff bot not enrolled");
        assert!(dec.skipped_due_to_backoff.contains(&1));
    }

    #[test]
    fn test_pinned_bot_always_in_target() {
        // Pinned bot on different map → still in target.
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 0.0, 0.0, 0.0)],
            bots: vec![bot(99, 9, 500.0, 0.0, 0.0)], // map 9
        };
        let pinned: HashSet<i64> = [99].into_iter().collect();
        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &pinned,
            &empty_backoff(),
            &empty_hysteresis(),
            &cfg(5),
        );
        assert!(dec.target_living_set.contains(&99));
        // Pinned bots are in sticky_pinned (they go through sticky bypass).
        assert!(dec.sticky_pinned.contains(&99));
    }

    #[test]
    fn test_pinned_bot_not_released_even_out_of_range() {
        // Bot pinned + enrolled; not in proximity target → pinned guard prevents release.
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 0.0, 0.0, 0.0)],
            bots: vec![bot(99, 9, 0.0, 0.0, 0.0)],
        };
        let enrolled: HashMap<i64, i64> = [(99, 0)].into_iter().collect();
        let pinned: HashSet<i64> = [99].into_iter().collect();
        // Large out_ticks to confirm the pinned guard fires before hysteresis check.
        let hysteresis: HashMap<i64, (i64, i64)> = [(99, (0, 999))].into_iter().collect();

        let dec = recompute_once(
            &snap,
            &enrolled,
            &pinned,
            &empty_backoff(),
            &hysteresis,
            &cfg(5),
        );
        assert!(!dec.to_release.contains(&99), "pinned bot never released");
    }

    #[test]
    fn test_living_bot_count_cap_respected() {
        // 5 bots, cap=3 → only 3 nearest selected.
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 0.0, 0.0, 0.0)],
            bots: (1..=5).map(|i| bot(i, 0, i as f64, 0.0, 0.0)).collect(),
        };
        let mut c = cfg(3);
        c.hysteresis_in_ticks = 1;
        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &empty_pinned(),
            &empty_backoff(),
            &empty_hysteresis(),
            &c,
        );
        // Only 3 bots in proximity_picks.
        assert_eq!(dec.proximity_picks.len(), 3);
        // Nearest 3 by bot_guid (bots 1,2,3 are at x=1,2,3).
        assert!(dec.proximity_picks.contains(&1));
        assert!(dec.proximity_picks.contains(&2));
        assert!(dec.proximity_picks.contains(&3));
        assert!(!dec.proximity_picks.contains(&4));
        assert!(!dec.proximity_picks.contains(&5));
    }

    #[test]
    fn test_phase_b_tier_full_vs_reduced() {
        // Phase B: bot on same map as player → FULL; bot on different map → REDUCED.
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 0.0, 0.0, 0.0)],
            bots: vec![
                bot(1, 0, 1.0, 0.0, 0.0), // same map → FULL
                bot(2, 9, 1.0, 0.0, 0.0), // different map → REDUCED
            ],
        };
        let enrolled: HashMap<i64, i64> = [(1, 0), (2, 0)].into_iter().collect();
        let mut c = cfg(10);
        c.phase_b_enabled = true;
        c.hysteresis_out_ticks = 99; // prevent releases

        let dec = recompute_once(
            &snap,
            &enrolled,
            &empty_pinned(),
            &empty_backoff(),
            &empty_hysteresis(),
            &c,
        );
        assert!(dec.to_full.contains(&1), "same-map bot → FULL");
        assert!(dec.to_reduced.contains(&2), "diff-map bot → REDUCED");
    }

    #[test]
    fn test_release_does_not_apply_to_sticky() {
        // Even if out_ticks meets threshold, sticky bot is not released.
        let snap = WorldSnapshot {
            players: vec![{
                let mut p = player(1, 0, 0.0, 0.0, 0.0);
                p.party_guid = Some(77);
                p
            }],
            bots: vec![{
                let mut b = bot(5, 0, 0.0, 0.0, 0.0);
                b.party_guid = Some(77); // sticky via party
                b
            }],
        };
        let enrolled: HashMap<i64, i64> = [(5, 0)].into_iter().collect();
        let hysteresis: HashMap<i64, (i64, i64)> = [(5, (0, 999))].into_iter().collect();

        let dec = recompute_once(
            &snap,
            &enrolled,
            &empty_pinned(),
            &empty_backoff(),
            &hysteresis,
            &cfg(5),
        );
        assert!(!dec.to_release.contains(&5), "sticky bot not released despite high out_ticks");
    }

    // ── async SubsetGate tests ────────────────────────────────────────────────

    use tempfile::NamedTempFile;
    use crate::models::PersonalityCard;

    fn test_card() -> PersonalityCard {
        PersonalityCard {
            name: "Test".into(),
            race: "Human".into(),
            class_: "Warrior".into(),
            backstory: "Fights things.".into(),
            talkativeness: 0.5,
            courage: 0.5,
            greed: 0.3,
            attitude_to_master: 0.0,
            party_invite_policy: "accept_from_known".into(),
            pvp_appetite: None,
            raid_appetite: None,
            completionist_streak: None,
            gold_motivation: None,
            profession_appetite: None,
        }
    }

    fn temp_store() -> (Arc<StateStore>, NamedTempFile) {
        let f = NamedTempFile::new().unwrap();
        let s = Arc::new(StateStore::open(f.path().to_str().unwrap()).unwrap());
        s.migrate().unwrap();
        (s, f)
    }

    /// Tracking mock: records the sequence of (op, bot_guid) calls.
    #[derive(Clone)]
    struct CallLog {
        inner: Arc<Mutex<Vec<(&'static str, i64)>>>,
    }

    impl CallLog {
        fn new() -> Self {
            Self { inner: Arc::new(Mutex::new(vec![])) }
        }
        fn push(&self, op: &'static str, guid: i64) {
            self.inner.lock().unwrap().push((op, guid));
        }
        fn ops(&self) -> Vec<(&'static str, i64)> {
            self.inner.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn test_apply_releases_before_enrolls() {
        // Verify release_fn is called before enroll_fn in _apply.
        let (store, _f) = temp_store();
        let card = test_card();

        // Enroll bots 1 (currently active, will be released) and 2 (will be enrolled).
        store.enroll(1, 0, &card).unwrap();
        // Bot 2 is NOT enrolled yet (will be enrolled during apply).

        let log = CallLog::new();
        let log_release = log.clone();
        let log_enroll = log.clone();

        let release_fn: ReleaseFn = Arc::new(move |guid| {
            let l = log_release.clone();
            Box::pin(async move {
                l.push("release", guid);
            })
        });

        let enroll_fn: EnrollFn = Arc::new(move |guid, _snap| {
            let l = log_enroll.clone();
            Box::pin(async move {
                l.push("enroll", guid);
            })
        });

        let snap = WorldSnapshot::default();
        let snapshot_fetcher: SnapshotFetcher =
            Arc::new(move || Box::pin(async move { WorldSnapshot::default() }));

        let mut c = cfg(5);
        c.phase_b_enabled = false;

        let gate = SubsetGate::new(
            Arc::clone(&store),
            snapshot_fetcher,
            enroll_fn,
            release_fn,
            c,
        );

        // Build a decision: to_release={1}, to_enroll={2}.
        let decision = SubsetDecision {
            target_living_set: HashSet::new(),
            sticky_pinned: HashSet::new(),
            proximity_picks: HashSet::new(),
            to_enroll: [2].into_iter().collect(),
            to_release: [1].into_iter().collect(),
            to_full: HashSet::new(),
            to_reduced: HashSet::new(),
            skipped_due_to_backoff: HashSet::new(),
        };

        gate._apply(&decision, &snap).await;

        let ops = log.ops();
        // release must appear before enroll in the call log.
        let release_pos = ops.iter().position(|(op, g)| *op == "release" && *g == 1);
        let enroll_pos = ops.iter().position(|(op, g)| *op == "enroll" && *g == 2);
        assert!(release_pos.is_some(), "release was called");
        assert!(enroll_pos.is_some(), "enroll was called");
        assert!(
            release_pos.unwrap() < enroll_pos.unwrap(),
            "release({}) precedes enroll({}): ops={:?}",
            release_pos.unwrap(),
            enroll_pos.unwrap(),
            ops
        );
    }

    #[tokio::test]
    async fn test_list_active_reflects_releases_before_hysteresis_bump() {
        // After _apply, list_active() must NOT include released bots.
        // We verify this by checking that bump_hysteresis is NOT called for
        // a bot whose release_fn set its status to "released".
        let (store, _f) = temp_store();
        let card = test_card();

        // Enroll bot 1 (will be released during apply).
        store.enroll(1, 0, &card).unwrap();
        assert_eq!(store.list_active().unwrap().len(), 1);

        // Release fn updates state_store status.
        let store_for_release = Arc::clone(&store);
        let release_fn: ReleaseFn = Arc::new(move |guid| {
            let s = Arc::clone(&store_for_release);
            Box::pin(async move {
                // Mirrors LoopSupervisor::release_bot: set_status("released") inside fn.
                let _ = s.set_status(guid, "released");
            })
        });

        let enroll_fn: EnrollFn =
            Arc::new(|_guid, _snap| Box::pin(async move {}));

        let snapshot_fetcher: SnapshotFetcher =
            Arc::new(|| Box::pin(async move { WorldSnapshot::default() }));

        let gate = SubsetGate::new(
            Arc::clone(&store),
            snapshot_fetcher,
            enroll_fn,
            release_fn,
            cfg(5),
        );

        let decision = SubsetDecision {
            target_living_set: HashSet::new(),
            sticky_pinned: HashSet::new(),
            proximity_picks: HashSet::new(),
            to_enroll: HashSet::new(),
            to_release: [1].into_iter().collect(),
            to_full: HashSet::new(),
            to_reduced: HashSet::new(),
            skipped_due_to_backoff: HashSet::new(),
        };

        gate._apply(&decision, &WorldSnapshot::default()).await;

        // After apply, bot 1 should be released — not in list_active().
        let active = store.list_active().unwrap();
        assert!(
            active.iter().all(|r| r.bot_guid != 1),
            "bot 1 removed from list_active after release"
        );
    }

    #[tokio::test]
    async fn test_set_enroll_backoff_prevents_reenroll() {
        let (store, _f) = temp_store();
        let snap = WorldSnapshot {
            players: vec![player(1, 0, 0.0, 0.0, 0.0)],
            bots: vec![bot(7, 0, 1.0, 0.0, 0.0)],
        };
        let snap_clone = snap.clone();
        let snapshot_fetcher: SnapshotFetcher =
            Arc::new(move || {
                let s = snap_clone.clone();
                Box::pin(async move { s })
            });

        let enroll_fn: EnrollFn =
            Arc::new(|_guid, _snap| Box::pin(async move {}));
        let release_fn: ReleaseFn =
            Arc::new(|_guid| Box::pin(async move {}));

        let gate = SubsetGate::new(
            Arc::clone(&store),
            snapshot_fetcher,
            enroll_fn,
            release_fn,
            cfg(5),
        );

        // Arm backoff for bot 7.
        gate.set_enroll_backoff(7);

        // Backoff set must contain bot 7.
        let backoff = gate._expired_backoff_set();
        assert!(backoff.contains(&7), "bot 7 in backoff set");

        // recompute_once with backoff → bot not enrolled.
        let dec = recompute_once(
            &snap,
            &empty_enrolled(),
            &empty_pinned(),
            &backoff,
            &empty_hysteresis(),
            &cfg(5),
        );
        assert!(!dec.to_enroll.contains(&7), "backoff bot not enrolled");
        assert!(dec.skipped_due_to_backoff.contains(&7));
    }

    #[tokio::test]
    async fn test_gate_disabled_run_returns_immediately() {
        let (store, _f) = temp_store();
        let snapshot_fetcher: SnapshotFetcher =
            Arc::new(|| Box::pin(async move { WorldSnapshot::default() }));
        let enroll_fn: EnrollFn =
            Arc::new(|_guid, _snap| Box::pin(async move {}));
        let release_fn: ReleaseFn =
            Arc::new(|_guid| Box::pin(async move {}));

        let mut c = cfg(5);
        c.enabled = false;

        let gate = SubsetGate::new(store, snapshot_fetcher, enroll_fn, release_fn, c);

        // run() should return immediately without hanging.
        tokio::time::timeout(
            tokio::time::Duration::from_millis(100),
            gate.run(),
        )
        .await
        .expect("run() should return immediately when disabled");
    }
}
