//! Periodic tick loop — fetches ground truth + raw events from the harness,
//! runs the three-check shadow diff, stores the latest report, and logs a
//! one-line summary per tick.
//!
//! The loop is fault-tolerant: any harness fetch error is logged and the
//! iteration is skipped (the previously-stored report is unchanged).
//! The loop itself does NOT panic or exit on transient harness errors.
//!
//! # Cancel safety
//!
//! The `run` future holds no locks across `.await` points.  It is safe to
//! abort the spawned task via [`tokio::task::JoinHandle::abort`].

use std::sync::Arc;
use std::time::Duration;

use crate::api::AppState;
use crate::config::Config;
use crate::harness::Harness;
use crate::holiday::holiday_rules;
use crate::shadow::compute_report;

/// Run the perpetual tick loop.
///
/// Each tick:
/// 1. Calls `harness.game_events()` and `harness.query_game_events()`.
/// 2. On any fetch error: logs `[shadow] fetch failed: {e}` and continues.
/// 3. On success: calls `compute_report`, stores the report in `state`, and logs a
///    one-line summary.
///
/// The interval is `cfg.tick_secs.max(1)` seconds (minimum 1 to avoid a tight loop
/// if the config value is somehow 0).
pub async fn run(state: Arc<AppState>, harness: Harness, cfg: Config) {
    let interval_secs = cfg.tick_secs.max(1);
    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    // The first tick fires immediately (MissedTickBehavior default is Burst).
    // We use the default (Burst), which is fine — on startup the first tick
    // runs immediately, giving us a report as soon as possible.

    loop {
        interval.tick().await;

        // Fetch ground truth.
        let gt = match harness.game_events().await {
            Ok(gt) => gt,
            Err(e) => {
                eprintln!("[shadow] fetch failed: {e}");
                continue;
            }
        };

        // Fetch raw events.
        let raw = match harness.query_game_events().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[shadow] fetch failed: {e}");
                continue;
            }
        };

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw, rules);

        eprintln!(
            "[shadow] gametime={} active={}/{} resolution={}/{} date_math={}/{} mismatches={}",
            report.server_gametime,
            report.active_set.matched,
            report.active_set.checked,
            report.resolution.matched,
            report.resolution.checked,
            report.date_math.matched,
            report.date_math.checked,
            report.mismatches.len(),
        );

        state.push_report(report);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::post, Json};
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    // ── Mock harness shapes ───────────────────────────────────────────────────
    //
    // Reuse the same shapes as harness.rs tests (real live payload structure).

    /// Single-event `obs.game_events` response (mirrors live 2026-05-31 shape).
    const GAME_EVENTS_RESULT: &str = r#"{
        "events":[
            {"entry":1,"start":1782000000,"end":1843316906,"occurence":525600,"length":20160,
             "holiday":341,"holiday_stage":1,"is_active":false,"next_start":0,"state":0}
        ],
        "active_event_list":[],
        "holidays":[
            {"holiday_id":62,"calendar_filter_type":-1,"looping":0,"region":1,
             "date":[425781248,442560512,459325440,476106752,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
             "duration":[18,0,0,0,0,0,0,0,0,0]}
        ],
        "server_gametime":1780245388,
        "resolve_reference_unixtime":1780244906,
        "server_tz_offset_secs":0
    }"#;

    /// Single-row `obs.query_db` response — real live envelope shape (2026-05-31 verified).
    /// result is an OBJECT `{ "row_count": <int>, "rows": [...] }`, NOT a bare array.
    /// Row has BOTH start_time: null AND end_time: null (holiday row, eventEntry:1).
    const QUERY_DB_RESULT: &str = r#"{
        "row_count": 1,
        "rows": [
            {"eventEntry":1,"start_time":null,"end_time":null,"occurence":525600,"length":20160,
             "holiday":341,"holidayStage":1,"description":"d","world_event":0,"announce":2}
        ]
    }"#;

    /// Spawn a minimal mock harness returning fixed payloads.
    async fn spawn_mock_harness() -> String {
        let app = Router::new()
            .route(
                "/v1/tools/obs.game_events",
                post(|| async {
                    let result: Value = serde_json::from_str(GAME_EVENTS_RESULT).unwrap();
                    Json(json!({"ok": true, "result": result}))
                }),
            )
            .route(
                "/v1/tools/obs.query_db",
                post(|| async {
                    let result: Value = serde_json::from_str(QUERY_DB_RESULT).unwrap();
                    Json(json!({"ok": true, "result": result}))
                }),
            );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }

    // ── Tick stores a report ──────────────────────────────────────────────────

    /// Run the tick loop for ~150ms, then abort it, and assert a report was stored.
    #[tokio::test]
    async fn tick_run_stores_report_in_state() {
        let base_url = spawn_mock_harness().await;
        let state = Arc::new(AppState::new());
        let harness = Harness::new(base_url, "test-token");
        let cfg = Config {
            listen_addr: "127.0.0.1:8091".to_string(),
            harness_base_url: "unused".to_string(),
            harness_bearer: "unused".to_string(),
            tick_secs: 1, // fire once, then again after 1s; we abort quickly
        };

        let handle = tokio::spawn(run(state.clone(), harness, cfg));

        // Give the first tick time to fire and complete.
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.abort();
        let _ = handle.await; // ignore JoinError from abort

        let latest = state.latest.lock().unwrap();
        assert!(
            latest.is_some(),
            "state.latest must be Some after the first tick"
        );
        let report = latest.as_ref().unwrap();
        // The mock harness returns 1 event (holiday 341, entry 1, is_active=false).
        // The active_set check runs for that one event (both vs_field and vs_list).
        // entry 1 is not in active_event_list (empty) AND is_active=false in the gt.
        // Rust computes is_active = start(1782000000) > server_gametime(1780245388) → false.
        // Both comparisons: Rust=false, cpp=false → match.
        assert_eq!(
            report.server_gametime, 1780245388,
            "server_gametime must come from the mock harness"
        );
        assert_eq!(
            report.active_set.checked, 2,
            "one event × two checks (vs_field + vs_list) = 2 checked"
        );
    }

    // ── Tick history is populated ─────────────────────────────────────────────

    #[tokio::test]
    async fn tick_run_pushes_summary_to_history() {
        let base_url = spawn_mock_harness().await;
        let state = Arc::new(AppState::new());
        let harness = Harness::new(base_url, "test-token");
        let cfg = Config {
            listen_addr: "127.0.0.1:8091".to_string(),
            harness_base_url: "unused".to_string(),
            harness_bearer: "unused".to_string(),
            tick_secs: 1,
        };

        let handle = tokio::spawn(run(state.clone(), harness, cfg));
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.abort();
        let _ = handle.await;

        let history = state.history.lock().unwrap();
        assert!(!history.is_empty(), "history must have at least one entry");
        assert_eq!(history[0].server_gametime, 1780245388);
    }

    // ── Fetch error → loop continues, no panic ────────────────────────────────

    /// Spawn the tick loop pointing at a dead URL; it should not panic and the
    /// state should remain empty (no report stored).
    #[tokio::test]
    async fn tick_run_continues_on_fetch_error() {
        let state = Arc::new(AppState::new());
        let harness = Harness::new("http://127.0.0.1:1", "test-token"); // refused
        let cfg = Config {
            listen_addr: "127.0.0.1:8091".to_string(),
            harness_base_url: "unused".to_string(),
            harness_bearer: "unused".to_string(),
            tick_secs: 1,
        };

        let handle = tokio::spawn(run(state.clone(), harness, cfg));
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.abort();
        let _ = handle.await;

        // No panic means the test reached here.
        // State remains empty because no tick succeeded.
        let latest = state.latest.lock().unwrap();
        assert!(
            latest.is_none(),
            "state must remain empty when all fetches fail"
        );
    }
}
