//! Per-bot task lifecycle: claim ownership, ingest the current goal, dispose it,
//! report status, release on exit.

use std::sync::Arc;

use tokio::task::JoinHandle;
use tot_goal_contract::{GoalEnvelope, GoalStatus, Goal};
use tot_goal_contract::wire::{GoalReceiver, StatusSender};
use tot_harness_client::HarnessClient;

use crate::{grind, own};

/// Owns the running per-bot tasks.
#[derive(Default)]
pub struct BotSupervisor {
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
    ) {
        let task = tokio::spawn(per_bot_loop(bot_guid, harness, goal_rx, status_tx));
        self.handles.insert(bot_guid, task);
    }

    /// Await a bot's task to finish (after its goal channel is closed).
    pub async fn join(&mut self, bot_guid: u64) {
        if let Some(h) = self.handles.remove(&bot_guid) { let _ = h.await; }
    }
}

/// Claim ownership, then loop: wait for a goal (or shutdown), dispose it, report status.
/// Releases ownership on every exit path. Shutdown = the goal `watch` sender dropped.
pub async fn per_bot_loop(
    bot_guid: u64,
    harness: Arc<HarnessClient>,
    mut goal_rx: GoalReceiver,
    status_tx: StatusSender,
) {
    if let Err(e) = own::set_ai_owned(&harness, bot_guid, true).await {
        tracing::error!("exec: claim ownership failed bot={bot_guid}: {e}");
        return;
    }

    loop {
        // Wait until a (new) goal is published or the channel closes (shutdown).
        if goal_rx.changed().await.is_err() {
            break; // sender dropped → shutdown
        }
        // borrow_and_update (NOT borrow) marks this goal seen, so the next changed()
        // waits for a genuinely new goal instead of re-firing on the same one.
        let envelope: Option<GoalEnvelope> = goal_rx.borrow_and_update().clone();
        let Some(env) = envelope else { continue };

        let status = dispose_goal(&harness, bot_guid, env).await;
        let _ = status_tx.send(status).await;
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
        // `Goal` is #[non_exhaustive]; a wildcard is REQUIRED to match it from another
        // crate. M2 variants land here until exec implements them.
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
    use tot_goal_contract::{wire, GoalSink, GrindGoal, MobFilter, StatusSource, WorldPos};

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
            to_level: 99, kill_count: Some(1), rest_threshold: 0.35,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn full_loop_owns_runs_goal_reports_completed_and_releases() {
        let owned = Arc::new(std::sync::atomic::AtomicI32::new(0)); // +1 claim, -1 release
        let owned2 = owned.clone();
        let base = spawn_mock(move |name, args| match name.as_str() {
            "bot.set_ai_enabled" => {
                // enabled:false = claim (owned), enabled:true = release.
                let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                if enabled { owned2.fetch_sub(1, std::sync::atomic::Ordering::SeqCst); json!({"owned": false, "reset": true}) }
                else       { owned2.fetch_add(1, std::sync::atomic::Ordering::SeqCst); json!({"owned": true, "reset": false}) }
            }
            "obs.get_nearby_hostiles" => json!({"hostiles": [
                {"guid": 111u64, "name": "Kobold", "level": 5, "hp_pct": 100, "distance": 6.0, "is_alive": true}]}),
            "nav.find_path" => json!({"path_type": 1i64, "points": [
                {"x":0.0,"y":0.0,"z":0.0},{"x":6.0,"y":0.0,"z":0.0}]}),
            "bot.move_path" => json!({"launched": true, "duration_ms": 0, "final": {"x":6.0,"y":0.0,"z":0.0}}),
            "obs.get_position" => json!({"x":6.0,"y":0.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0}),
            "obs.get_state" => json!({"self": {"level": 5, "health_pct": 90}}),
            other => panic!("unexpected tool {other}"),
        }).await;

        let harness = Arc::new(HarnessClient::new(base, "tok", Duration::from_secs(5)));
        let (sink, source, goal_rx, status_tx) = wire(1003);
        let mut sup = BotSupervisor::new();
        sup.start(1003, harness, goal_rx, status_tx);

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
}
