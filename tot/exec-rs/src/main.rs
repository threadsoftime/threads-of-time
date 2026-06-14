//! exec-rs demo runner (M1 Plan 1).
//!
//! Reads HARNESS_URL / HARNESS_BEARER / EXEC_BOT_GUID from the environment, claims the
//! bot, runs a single hardcoded Grind goal, prints status transitions, and releases on
//! Ctrl-C. This is a developer smoke-test harness — the production wiring (brain-sidecar
//! embeds the supervisor + emits goals) is Plan 2.

use std::sync::Arc;
use std::time::Duration;

use exec_rs::supervisor::BotSupervisor;
use tot_goal_contract::{wire, Goal, GoalEnvelope, GoalSink, GrindGoal, MobFilter, StatusSource, WorldPos};
use tot_harness_client::HarnessClient;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info".into())).init();

    let url = std::env::var("HARNESS_URL").unwrap_or_else(|_| "http://127.0.0.1:8099".into());
    let bearer = std::env::var("HARNESS_BEARER").unwrap_or_default();
    let bot_guid: u64 = std::env::var("EXEC_BOT_GUID").ok()
        .and_then(|s| s.parse().ok()).unwrap_or(1003);

    let harness = Arc::new(HarnessClient::new(url, bearer, Duration::from_secs(10)));
    let (sink, source, goal_rx, status_tx) = wire(bot_guid);
    let mut sup = BotSupervisor::new();
    sup.start(bot_guid, harness, goal_rx, status_tx, None);

    // A single demo Grind goal (Elwynn-style anchor; coordinates are illustrative).
    let goal = GoalEnvelope {
        goal_id: "demo-1".into(),
        version: 1,
        goal: Goal::Grind(GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: -9450.0, y: 50.0, z: 60.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 2, max_level: 10, creature_type: Some("humanoid".into()) },
            to_level: 7, kill_count: None, rest_threshold: 0.35, rotation_id: None,
            vendor: None,
        }),
    };
    if let Err(e) = sink.set_goal(bot_guid, goal).await {
        eprintln!("[exec-rs] set_goal failed: {e}");
        return;
    }

    // Print status transitions until Ctrl-C.
    let status_task = tokio::spawn(async move {
        loop {
            if let Ok(Some(s)) = source.poll_status(bot_guid).await {
                println!("[exec-rs] status: {s:?}");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    let _ = tokio::signal::ctrl_c().await;
    println!("[exec-rs] shutting down — releasing bot {bot_guid}");
    drop(sink);                 // closes the goal channel → loop releases ownership
    sup.join(bot_guid).await;
    status_task.abort();
}
