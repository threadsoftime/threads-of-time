//! Per-bot task lifecycle: claim ownership, ingest the current goal, dispose it,
//! report status, release on exit.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tot_goal_contract::{Goal, GoalEnvelope, GoalReceiver, GoalStatus, StatusSender, WorldPos};
use tot_harness_client::HarnessClient;

use crate::{grind, own, quest};

/// Idle health-poll cadence. While a bot is parked (no goal in flight), the loop
/// polls its own state at this interval and revives it if it is a dead at-cap bot.
/// Small under cfg(test) so the supervisor tests run fast.
const IDLE_HEALTH_POLL_INTERVAL_MS: u64 = if cfg!(test) { 20 } else { 30_000 };

/// Per-bot idle-recovery configuration. When present, the idle poll revives a
/// dead at-cap bot (level >= cap_level) and re-anchors it to `anchor`. `None`
/// disables the idle poll (used by tests and any non-roster bot).
#[derive(Debug)]
pub struct IdleConfig {
    pub anchor: WorldPos,
    pub cap_level: u32,
}

/// Owns the running per-bot tasks.
///
/// # Shutdown invariant
///
/// Tasks in `handles` MUST be shut down by joining (clean shutdown), never by
/// [`JoinHandle::abort`]. The correct shutdown sequence is:
/// 1. Drop the brain-side goal sink so each `per_bot_loop` observes channel closure.
/// 2. Call [`BotSupervisor::join`] (or [`BotSupervisor::join_all`]) to await the task.
///
/// Aborting a task mid-execution can interrupt the loop between the ownership-claim
/// (`set_ai_owned(.., true)`) and the ownership-release (`set_ai_owned(.., false)`),
/// stranding the bot with `owned = true` permanently.
#[derive(Default)]
pub struct BotSupervisor {
    /// Running per-bot task handles.
    ///
    /// These handles MUST be joined (never aborted) to preserve the ownership-release
    /// invariant — see the struct-level doc comment for the required shutdown sequence.
    handles: std::collections::HashMap<u64, JoinHandle<()>>,
}

impl BotSupervisor {
    pub fn new() -> Self { Self::default() }

    /// Spawn the per-bot loop. The caller holds the matching `GoalSink`/`StatusSource`
    /// (the brain side); dropping that sink closes `goal_rx` and ends the loop.
    pub fn start(
        &mut self,
        bot_guid: u64,
        harness: Arc<HarnessClient>,
        goal_rx: GoalReceiver,
        status_tx: StatusSender,
        idle: Option<IdleConfig>,
    ) {
        let task = tokio::spawn(per_bot_loop(bot_guid, harness, goal_rx, status_tx, idle));
        self.handles.insert(bot_guid, task);
    }

    /// Await a bot's task to finish (after its goal channel is closed).
    pub async fn join(&mut self, bot_guid: u64) {
        if let Some(h) = self.handles.remove(&bot_guid) { let _ = h.await; }
    }

    /// Join every running per-bot task (clean shutdown). Callers should first drop the
    /// brain-side goal sinks so each loop observes shutdown and releases ownership.
    pub async fn join_all(&mut self) {
        let guids: Vec<u64> = self.handles.keys().copied().collect();
        for g in guids { self.join(g).await; }
    }
}

/// Claim ownership, then loop: wait for a goal (or shutdown), dispose it, report status.
/// Releases ownership on every exit path. Shutdown = the goal `watch` sender dropped.
pub async fn per_bot_loop(
    bot_guid: u64,
    harness: Arc<HarnessClient>,
    mut goal_rx: GoalReceiver,
    status_tx: StatusSender,
    idle: Option<IdleConfig>,
) {
    if let Err(e) = own::set_ai_owned(&harness, bot_guid, true).await {
        tracing::error!("exec: claim ownership failed bot={bot_guid}: {e}");
        return;
    }

    loop {
        tokio::select! {
            // Goal path (unchanged): wait for a new goal or shutdown.
            changed = goal_rx.changed() => {
                if changed.is_err() {
                    break; // sender dropped → shutdown
                }
                let envelope: Option<GoalEnvelope> = goal_rx.borrow_and_update().clone();
                let Some(env) = envelope else { continue };
                let status = dispose_goal(&harness, bot_guid, env).await;
                let _ = status_tx.send(status).await;
            }
            // Idle health poll: only meaningful while parked (no goal in flight).
            // changed() is cancel-safe: if the sleep arm wins, cancelling the
            // in-flight changed() future loses no notification.
            _ = tokio::time::sleep(Duration::from_millis(IDLE_HEALTH_POLL_INTERVAL_MS)) => {
                if let Some(cfg) = idle.as_ref() {
                    match grind::read_self(&harness, bot_guid).await {
                        Ok(st) if st.hp_pct == 0 && st.level >= cfg.cap_level => {
                            match grind::revive_and_home(&harness, bot_guid, &cfg.anchor).await {
                                grind::RecoveryOutcome::Recovered =>
                                    tracing::info!(bot_guid, "idle_recovery: at-cap bot revived + re-anchored"),
                                grind::RecoveryOutcome::StillDead =>
                                    tracing::warn!(bot_guid, "idle_recovery: revive failed, will retry next poll"),
                            }
                        }
                        Ok(_) => {} // alive or below cap → no action
                        Err(e) => tracing::warn!(bot_guid, error = %e, "idle_recovery: state read failed"),
                    }
                }
            }
        }
    }

    if let Err(e) = own::set_ai_owned(&harness, bot_guid, false).await {
        tracing::warn!("exec: release ownership failed bot={bot_guid}: {e}");
    }
}

/// Dispatch a goal to its disposer. (M1: only `Grind`.)
async fn dispose_goal(harness: &HarnessClient, bot_guid: u64, env: GoalEnvelope) -> GoalStatus {
    if let Err(e) = env.validate() {
        return GoalStatus::Blocked {
            reason: tot_goal_contract::BlockedReason::VersionTooNew,
            detail: Some(e.to_string()),
        };
    }
    match env.goal {
        Goal::Grind(g) => grind::run_grind(harness, bot_guid, &g).await,
        Goal::Quest(q) => quest::run_quest(harness, bot_guid, &q).await,
        // `Goal` is #[non_exhaustive]; a wildcard is REQUIRED to match it from another
        // crate. Future variants land here until exec implements them.
        _ => GoalStatus::Blocked {
            reason: tot_goal_contract::BlockedReason::UnknownGoalVariant,
            detail: Some("exec-rs does not implement this Goal variant yet".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path as AxumPath, http::StatusCode, routing::post, Json, Router};
    use serde_json::{json, Value};
    use std::time::Duration;
    use tot_goal_contract::{wire, GoalSink, GoalSinkRegistry, GrindGoal, MobFilter, StatusSource, WorldPos};

    async fn spawn_mock<F>(handler: F) -> String
    where F: Fn(String, Value) -> Value + Send + Sync + 'static {
        let h = Arc::new(handler);
        let route = move |AxumPath(name): AxumPath<String>, Json(args): Json<Value>| {
            let h = h.clone();
            async move { (StatusCode::OK, Json(json!({ "ok": true, "result": h(name, args) }))) }
        };
        let app = Router::new().route("/v1/tools/:name", post(route));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        format!("http://{addr}")
    }

    fn grind_goal() -> GrindGoal {
        GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn full_loop_owns_runs_goal_reports_completed_and_releases() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let owned = Arc::new(std::sync::atomic::AtomicI32::new(0)); // +1 claim, -1 release
        let owned2 = owned.clone();
        // Track obs.get_nearby_hostiles calls: first call is the scan (target alive),
        // subsequent calls are combat polls (target dead → TargetDead → PostKillPause).
        let hostile_calls = Arc::new(AtomicU32::new(0));
        let hc2 = hostile_calls.clone();
        let base = spawn_mock(move |name, args| match name.as_str() {
            "bot.set_ai_enabled" => {
                // set_ai_owned(_, true) claims and sends enabled:false; set_ai_owned(_, false)
                // releases and sends enabled:true. Track net ownership: +1 on a claim
                // (enabled:false), -1 on a release (enabled:true).
                let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                if enabled { owned2.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); json!({"owned": false, "reset": true}) }
                else       { owned2.fetch_add(1, std::sync::atomic::Ordering::SeqCst); json!({"owned": true, "reset": false}) }
            }
            "obs.get_nearby_hostiles" => {
                let n = hc2.fetch_add(1, SeqCst);
                if n == 0 {
                    // Scan: return one live in-band target with world-space coords
                    json!({"hostiles": [
                        {"guid": 111u64, "name": "Kobold", "level": 5, "hp_pct": 100.0,
                         "distance": 6.0, "is_alive": true, "x": 6.0, "y": 0.0, "z": 0.0}
                    ]})
                } else {
                    // Combat poll: target dead (absent) → TargetDead → PostKillPause
                    json!({"hostiles": []})
                }
            }
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":6.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0, "final": {"x":6.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":6.0,"y":0.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            // Plan 2: obs.get_state uses hp_pct (not health_pct)
            "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 90}}),
            // Plan 2: bot.attack called by fight()
            "bot.attack" => json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"}),
            // Plan 2: obs.get_lootable_corpses called by Looting state
            "obs.get_lootable_corpses" => json!({"corpses": []}),
            other => panic!("unexpected tool {other}"),
        }).await;

        let harness = Arc::new(HarnessClient::new(base, "tok", Duration::from_secs(5)));
        let (sink, source, goal_rx, status_tx) = wire(1003);
        let mut sup = BotSupervisor::new();
        sup.start(1003, harness, goal_rx, status_tx, None);

        // Push a goal, then wait for the Completed status.
        sink.set_goal(1003, GoalEnvelope { goal_id: "g-1".into(), version: 1,
            goal: Goal::Grind(grind_goal()) }).await.unwrap();

        // Poll status until Completed (bounded).
        let mut completed = false;
        for _ in 0..50 {
            if let Some(GoalStatus::Completed { .. }) = source.poll_status(1003).await.unwrap() {
                completed = true; break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(completed, "expected a Completed status");

        // Drop the sink → shutdown → loop releases ownership and exits.
        drop(sink);
        sup.join(1003).await;

        // Net ownership must be 0 (claimed once, released once).
        assert_eq!(owned.load(std::sync::atomic::Ordering::SeqCst), 0, "ownership not balanced");
    }

    /// When the initial ownership CLAIM fails, `per_bot_loop` must return immediately
    /// without attempting a release and without running any goal.
    ///
    /// The claim is forced to fail by having `bot.set_ai_enabled` return `owned:false`
    /// (i.e. the server did not grant ownership). `own::set_ai_owned(.., true)` requests
    /// `owned=true`; when the response carries `owned:false` instead it returns
    /// `Err(OwnError::Shape)`, which causes `per_bot_loop` to log + early-return.
    ///
    /// Any tool other than `bot.set_ai_enabled` hitting the mock panics — proving that
    /// `run_grind` (and the release path) never execute.
    #[tokio::test(flavor = "multi_thread")]
    async fn claim_failure_returns_without_releasing() {
        let call_count = Arc::new(std::sync::atomic::AtomicI32::new(0));
        let call_count2 = call_count.clone();

        let base = spawn_mock(move |name, _args| match name.as_str() {
            "bot.set_ai_enabled" => {
                // Return owned:false — the claim request asked for owned:true, so
                // own::set_ai_owned will see a mismatch and return Err(OwnError::Shape).
                call_count2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                json!({"owned": false, "reset": false})
            }
            other => panic!("unexpected tool call after claim failure: {other}"),
        }).await;

        let harness = Arc::new(HarnessClient::new(base, "tok", Duration::from_secs(5)));
        let (sink, source, goal_rx, status_tx) = wire(2001);
        let mut sup = BotSupervisor::new();
        sup.start(2001, harness, goal_rx, status_tx, None);

        // Push a goal to mirror the real call shape (the loop exits before reading it).
        sink.set_goal(2001, GoalEnvelope { goal_id: "g-fail".into(), version: 1,
            goal: Goal::Grind(grind_goal()) }).await.unwrap();

        // Drop the sink and join — the loop already returned on claim failure so join
        // resolves promptly.
        drop(sink);
        sup.join(2001).await;

        // Exactly one set_ai_enabled call was made (the failed claim attempt).
        // No release attempt (nothing was claimed), no run_grind call.
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "expected exactly 1 set_ai_enabled call (the failed claim); got more or fewer",
        );

        // No Completed (or any other) status should have been produced.
        assert!(
            source.poll_status(2001).await.unwrap().is_none(),
            "expected no status when claim fails before goal execution",
        );
    }

    /// Two bots, each with its own per_bot_loop, both driven through ONE
    /// GoalSinkRegistry. A goal set for bot B must complete bot B's loop; the
    /// registry routes by guid. Proves the M2 multi-bot wire end-to-end offline.
    #[tokio::test(flavor = "multi_thread")]
    async fn registry_drives_two_bots_independently() {
        // Per-bot call counter for obs.get_nearby_hostiles: the FIRST call per
        // bot is the Scanning probe (live target); later calls are fight()'s
        // liveness polls (empty → target dead). Keyed by bot_guid so the two
        // bots resolve independently through one shared mock.
        let scan_counts = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::HashMap::<i64, u32>::new(),
        ));
        let sc = scan_counts.clone();

        // Happy-path mock: scan returns one in-band target, combat/loot
        // resolve so the kill_count:1 grind completes. Shared by both bots.
        let base = spawn_mock(move |name, args| match name.as_str() {
            "bot.set_ai_enabled" => {
                let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                if enabled { json!({"owned": false, "reset": true}) }
                else       { json!({"owned": true,  "reset": false}) }
            }
            "obs.get_nearby_hostiles" => {
                let bot = args.get("bot_guid").and_then(Value::as_i64).unwrap_or(0);
                let mut counts = sc.lock().unwrap();
                let n = counts.entry(bot).or_insert(0);
                let first = *n == 0;
                *n += 1;
                if first {
                    json!({"hostiles": [
                        {"guid": 222u64, "name": "Kobold", "level": 5, "hp_pct": 100.0,
                         "distance": 6.0, "is_alive": true, "x": 6.0, "y": 0.0, "z": 0.0}
                    ]})
                } else {
                    json!({"hostiles": []})
                }
            }
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":6.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0, "final": {"x":6.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":6.0,"y":0.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            "obs.get_state" => json!({"self": {"level": 5, "hp_pct": 90}}),
            "bot.attack" => json!({"attacked": true, "target_guid": 222u64, "target_name": "Kobold"}),
            "obs.get_lootable_corpses" => json!({"corpses": []}),
            other => panic!("unexpected tool {other}"),
        }).await;

        let harness = Arc::new(HarnessClient::new(base, "tok", Duration::from_secs(5)));

        // Wire two bots; register both sinks in one registry; start both loops.
        let (sink_a, src_a, rx_a, tx_a) = wire(3001);
        let (sink_b, src_b, rx_b, tx_b) = wire(3002);
        let mut reg = GoalSinkRegistry::new();
        reg.register(3001, sink_a);
        reg.register(3002, sink_b);

        let mut sup = BotSupervisor::new();
        sup.start(3001, harness.clone(), rx_a, tx_a, None);
        sup.start(3002, harness.clone(), rx_b, tx_b, None);

        // Route a goal to EACH bot via the registry (by guid).
        reg.set_goal(3001, GoalEnvelope { goal_id: "g-a".into(), version: 1,
            goal: Goal::Grind(grind_goal()) }).await.unwrap();
        reg.set_goal(3002, GoalEnvelope { goal_id: "g-b".into(), version: 1,
            goal: Goal::Grind(grind_goal()) }).await.unwrap();

        // Both bots must reach Completed (bounded poll).
        let mut done_a = false;
        let mut done_b = false;
        for _ in 0..60 {
            if let Some(GoalStatus::Completed { .. }) = src_a.poll_status(3001).await.unwrap() { done_a = true; }
            if let Some(GoalStatus::Completed { .. }) = src_b.poll_status(3002).await.unwrap() { done_b = true; }
            if done_a && done_b { break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(done_a, "bot 3001 did not complete");
        assert!(done_b, "bot 3002 did not complete");

        // Clean shutdown: drop the registry (all sinks) → both loops exit.
        drop(reg);
        sup.join_all().await;
    }

    /// A parked AT-CAP bot that is a corpse (hp==0, level>=cap) is revived by the
    /// idle poll and teleported to its anchor — WITHOUT any goal, and the claim is
    /// held throughout (released only at shutdown).
    #[tokio::test(flavor = "multi_thread")]
    async fn idle_poll_revives_at_cap_corpse_and_keeps_claim() {
        use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::SeqCst};
        let owned = Arc::new(AtomicI32::new(0)); // +1 claim, -1 release
        let state_calls = Arc::new(AtomicU32::new(0));
        let teleports = Arc::new(AtomicU32::new(0));
        let (ow2, st2, tp2) = (owned.clone(), state_calls.clone(), teleports.clone());
        let base = spawn_mock(move |name, args| match name.as_str() {
            "bot.set_ai_enabled" => {
                let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                if enabled { ow2.fetch_sub(1, SeqCst); json!({"owned": false, "reset": true}) }
                else       { ow2.fetch_add(1, SeqCst); json!({"owned": true,  "reset": false}) }
            }
            // First read (idle poll) sees a corpse; after bot.revive, alive.
            "obs.get_state" => {
                let n = st2.fetch_add(1, SeqCst);
                json!({"self": {"level": 25, "hp_pct": if n == 0 { 0 } else { 90 }}})
            }
            "bot.revive" => json!({"revived": true, "was_dead": true}),
            "gm.teleport" => { tp2.fetch_add(1, SeqCst); json!({"teleported": true}) }
            other => panic!("unexpected tool {other}"),
        }).await;

        let harness = Arc::new(HarnessClient::new(base, "tok", Duration::from_secs(5)));
        let (sink, _source, goal_rx, status_tx) = wire(1194);
        let mut sup = BotSupervisor::new();
        let idle = Some(IdleConfig {
            anchor: WorldPos { map_id: 1, x: 2055.0, y: -1030.0, z: 95.0 },
            cap_level: 25,
        });
        sup.start(1194, harness, goal_rx, status_tx, idle);

        // No goal pushed. Wait until the idle poll has revived + teleported.
        let mut recovered = false;
        for _ in 0..100 {
            if teleports.load(SeqCst) >= 1 { recovered = true; break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(recovered, "idle poll did not revive the at-cap corpse");

        // Stop the loop BEFORE asserting final counts so no further poll can run.
        drop(sink);
        sup.join(1194).await;
        assert_eq!(teleports.load(SeqCst), 1, "exactly one idle recovery (no re-revive once alive)");
        assert_eq!(owned.load(SeqCst), 0, "claimed once, released once at shutdown");
    }

    /// A parked BELOW-CAP corpse (hp==0, level<cap) must NOT be revived by the idle
    /// poll — that would defeat run_grind's MAX_DEATHS escalation. bot.revive panics
    /// if called.
    #[tokio::test(flavor = "multi_thread")]
    async fn idle_poll_skips_below_cap_corpse() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let state_calls = Arc::new(AtomicU32::new(0));
        let sc2 = state_calls.clone();
        let base = spawn_mock(move |name, args| match name.as_str() {
            "bot.set_ai_enabled" => {
                let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                if enabled { json!({"owned": false, "reset": true}) }
                else       { json!({"owned": true,  "reset": false}) }
            }
            "obs.get_state" => { sc2.fetch_add(1, SeqCst); json!({"self": {"level": 20, "hp_pct": 0}}) }
            "bot.revive" => panic!("idle poll must not revive a below-cap bot"),
            "gm.teleport" => panic!("idle poll must not teleport a below-cap bot"),
            other => panic!("unexpected tool {other}"),
        }).await;

        let harness = Arc::new(HarnessClient::new(base, "tok", Duration::from_secs(5)));
        let (sink, _source, goal_rx, status_tx) = wire(1195);
        let mut sup = BotSupervisor::new();
        let idle = Some(IdleConfig {
            anchor: WorldPos { map_id: 1, x: 0.0, y: 0.0, z: 0.0 },
            cap_level: 25,
        });
        sup.start(1195, harness, goal_rx, status_tx, idle);

        // Let several idle polls fire (each reads obs.get_state), then shut down.
        let mut polled = false;
        for _ in 0..50 {
            if state_calls.load(SeqCst) >= 3 { polled = true; break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(polled, "idle poll did not run for the below-cap bot");

        drop(sink);
        sup.join(1195).await;
        // Reaching here without a panic proves bot.revive/gm.teleport were never called.
    }
}
