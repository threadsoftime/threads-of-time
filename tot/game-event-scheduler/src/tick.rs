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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::api::AppState;
use crate::config::Config;
use crate::harness::Harness;
use crate::holiday::holiday_rules;
use crate::resolve::{effective_end, effective_start, resolve_holiday_event, resolve_periodic_event};
use crate::schedule::is_active;
use crate::shadow::{active_set_exclusion_reason, compute_report};

/// Run the perpetual tick loop.
///
/// Each tick:
/// 1. Calls `harness.game_events()` and `harness.query_game_events()`.
/// 2. On any fetch error: logs `[shadow] fetch failed: {e}` and continues.
/// 3. On success: calls `compute_report`, stores the report in `state`, and logs a
///    one-line summary.
/// 4. When `cfg.drive` is `true`: reconciles the live world-event active set against
///    Rust's schedule computation.  For every **in-scope** event (same scoping as
///    the active-set shadow check — NORMAL state, NOT ManualStart, NOT non-Normal),
///    if `rust_active != live_active`, calls `harness.event_start` or
///    `harness.event_stop` as appropriate.  Excluded events (Internal / ManualStart /
///    non-Normal) are NEVER driven.
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
        let report = compute_report(&gt, &raw, rules, cfg.live_shadow_resolution);

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

        // ── Drive step ────────────────────────────────────────────────────────
        // Only execute when cfg.drive is true.
        // Scope: EXACTLY the same in-scope set as the active-set shadow check.
        // NEVER drive excluded events (NonNormalState / ManualStart).
        if cfg.drive {
            // Build a lookup map: entry → &GroundTruthEvent for the drive loop.
            let gt_event_by_entry: HashMap<u16, &crate::events::GroundTruthEvent> =
                gt.events.iter().map(|e| (e.entry, e)).collect();

            for row in &raw {
                // Resolve the event exactly as compute_report does (anti-circularity preserved:
                // Rust is_active is computed from raw inputs + server_gametime, NOT from
                // gt.events[].is_active).
                let row_start = effective_start(row.start_time);
                let row_end = effective_end(row.end_time, gt.resolve_reference_unixtime);

                let rust_resolved = if row.holiday != 0 {
                    if let Some(hentry) =
                        gt.holidays.iter().find(|h| h.holiday_id == row.holiday)
                    {
                        resolve_holiday_event(
                            row,
                            row_start,
                            row_end,
                            hentry,
                            gt.resolve_reference_unixtime,
                            gt.server_tz_offset_secs,
                        )
                    } else {
                        resolve_periodic_event(row, row_start, row_end)
                    }
                } else {
                    resolve_periodic_event(row, row_start, row_end)
                };

                // Get the matching GroundTruthEvent for exclusion check + live state.
                let gt_ev = match gt_event_by_entry.get(&row.entry) {
                    Some(ev) => ev,
                    None => continue, // no gt entry → skip
                };

                // Apply the SAME exclusion logic as the active-set shadow check.
                // NEVER drive excluded events.
                if active_set_exclusion_reason(row, gt_ev, gt.resolve_reference_unixtime)
                    .is_some()
                {
                    continue;
                }

                // Compare Rust prediction vs live C++ state.
                let rust_active = is_active(&rust_resolved, gt.server_gametime, |_| None);
                // live_active: check both the is_active field and active_event_list membership.
                // Use is_active field as the authoritative live state (same source as shadow check).
                let live_active = gt_ev.is_active;

                if rust_active && !live_active {
                    // Rust says the event should be active but the server has it inactive.
                    eprintln!("[drive] event={} start (rust_active=true live_active=false)", row.entry);
                    if let Err(e) = harness.event_start(row.entry).await {
                        eprintln!("[drive] event_start({}) failed: {e}", row.entry);
                    }
                } else if !rust_active && live_active {
                    // Rust says the event should be inactive but the server has it active.
                    eprintln!("[drive] event={} stop (rust_active=false live_active=true)", row.entry);
                    if let Err(e) = harness.event_stop(row.entry).await {
                        eprintln!("[drive] event_stop({}) failed: {e}", row.entry);
                    }
                }
                // If rust_active == live_active: already converged, no action.
            }
        }

        state.push_report(report);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::State, routing::post, Json};
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
            drive: false,
            live_shadow_resolution: false,
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
            drive: false,
            live_shadow_resolution: false,
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
            drive: false,
            live_shadow_resolution: false,
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

    // ── Drive mode tests ──────────────────────────────────────────────────────
    //
    // These tests verify the reconcile-drive step in tick::run when cfg.drive=true.
    //
    // Design: we craft the obs.game_events payload to introduce a controlled
    // divergence between Rust's schedule computation and the live C++ active state,
    // then assert that event.start / event.stop is called (or NOT called for excluded
    // events).  Call counts are tracked via Arc<AtomicU32> shared with the axum handler.
    //
    // Server gametime used throughout: 1_780_300_000 (Unix seconds).
    // In-scope event 20 (Normal, periodic, no holiday):
    //   raw_start = server_gametime - 3600 (1h ago), raw_end = server_gametime + 86400 (tomorrow).
    //   occ = 525600 min (1 year), len = 20160 min (14 days).
    //   elapsed = 3600s; len_secs = 20160*60 = 1209600s; 3600 < 1209600 → rust_active = TRUE.
    //
    // Test 1 (start): gt.is_active=false, rust_active=true → event.start(20) expected.
    // Test 2 (stop):  gt.is_active=true,  rust_active=false → event.stop(21) expected.
    //   For event 21 to have rust_active=false: start far in the past, end past.
    //   Use start = server_gametime - 30*86400, end = server_gametime + 86400, occ=YEAR, len=20160.
    //   elapsed = 30*86400 = 2592000s > 1209600s → rust_active = FALSE.
    //
    // Test 3 (excluded): event 50, state=Internal (5), live=true, rust would say active,
    //   but excluded by active_set_exclusion_reason → event.start/stop must NOT be called.

    use std::sync::atomic::{AtomicU32, Ordering};

    /// Shared state for the drive mock harness.
    #[derive(Clone)]
    struct DriveMockState {
        game_events_json: Arc<str>,
        query_db_json: Arc<str>,
        start_call_count: Arc<AtomicU32>,
        stop_call_count: Arc<AtomicU32>,
    }

    /// Spawn a mock harness that tracks event.start / event.stop call counts.
    /// Returns (base_url, start_call_count, stop_call_count).
    async fn spawn_drive_mock(
        game_events_json: &str,
        query_db_json: &str,
    ) -> (String, Arc<AtomicU32>, Arc<AtomicU32>) {
        let start_count = Arc::new(AtomicU32::new(0));
        let stop_count = Arc::new(AtomicU32::new(0));

        let mock_state = DriveMockState {
            game_events_json: game_events_json.into(),
            query_db_json: query_db_json.into(),
            start_call_count: start_count.clone(),
            stop_call_count: stop_count.clone(),
        };

        let app = Router::new()
            .route(
                "/v1/tools/obs.game_events",
                post(|State(s): State<DriveMockState>| async move {
                    let result: Value = serde_json::from_str(&s.game_events_json).unwrap();
                    Json(json!({"ok": true, "result": result}))
                }),
            )
            .route(
                "/v1/tools/obs.query_db",
                post(|State(s): State<DriveMockState>| async move {
                    let result: Value = serde_json::from_str(&s.query_db_json).unwrap();
                    Json(json!({"ok": true, "result": result}))
                }),
            )
            .route(
                "/v1/tools/event.start",
                post(|State(s): State<DriveMockState>, Json(body): Json<Value>| async move {
                    s.start_call_count.fetch_add(1, Ordering::SeqCst);
                    let event_id = body.get("event_id").and_then(Value::as_u64).unwrap_or(0) as u16;
                    Json(json!({"ok": true, "result": {"started": true, "event_id": event_id, "is_active_now": true}}))
                }),
            )
            .route(
                "/v1/tools/event.stop",
                post(|State(s): State<DriveMockState>, Json(body): Json<Value>| async move {
                    s.stop_call_count.fetch_add(1, Ordering::SeqCst);
                    let event_id = body.get("event_id").and_then(Value::as_u64).unwrap_or(0) as u16;
                    Json(json!({"ok": true, "result": {"stopped": true, "event_id": event_id, "is_active_now": false}}))
                }),
            )
            .with_state(mock_state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{}", addr), start_count, stop_count)
    }

    // Server gametime for drive tests.
    const DRIVE_SERVER_GAMETIME: i64 = 1_780_300_000;

    /// Test 1: in-scope event where rust_active=true but live is_active=false.
    /// Asserts event.start is called exactly once.
    #[tokio::test]
    async fn drive_mode_calls_event_start_when_rust_active_live_inactive() {
        // Event 20: Normal, periodic, no holiday.
        // raw: start = DRIVE_SERVER_GAMETIME - 3600, end = DRIVE_SERVER_GAMETIME + 86400,
        //      occ=525600 min, len=20160 min.
        // Rust computes: elapsed=3600s < len_secs=1209600s → rust_active=TRUE.
        // gt: is_active=false (divergence — drive should fix it via event.start).
        let start = DRIVE_SERVER_GAMETIME - 3600;
        let end = DRIVE_SERVER_GAMETIME + 86400;

        let game_events_json = format!(r#"{{
            "events":[
                {{"entry":20,"start":{start},"end":{end},"occurence":525600,"length":20160,
                  "holiday":0,"holiday_stage":0,"is_active":false,"next_start":0,"state":0}}
            ],
            "active_event_list":[],
            "holidays":[],
            "server_gametime":{DRIVE_SERVER_GAMETIME},
            "resolve_reference_unixtime":{DRIVE_SERVER_GAMETIME},
            "server_tz_offset_secs":0
        }}"#);

        let query_db_json = format!(r#"{{
            "row_count":1,
            "rows":[
                {{"eventEntry":20,"start_time":{start},"end_time":{end},
                  "occurence":525600,"length":20160,"holiday":0,"holidayStage":0,
                  "description":"test","world_event":0,"announce":0}}
            ]
        }}"#);

        let (base_url, start_count, stop_count) =
            spawn_drive_mock(&game_events_json, &query_db_json).await;

        let state = Arc::new(AppState::new());
        let harness = Harness::new(base_url, "test-token");
        let cfg = Config {
            listen_addr: "127.0.0.1:8091".to_string(),
            harness_base_url: "unused".to_string(),
            harness_bearer: "unused".to_string(),
            tick_secs: 1,
            drive: true,
            live_shadow_resolution: false,
        };

        let handle = tokio::spawn(run(state.clone(), harness, cfg));
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();
        let _ = handle.await;

        let starts = start_count.load(Ordering::SeqCst);
        let stops = stop_count.load(Ordering::SeqCst);

        assert!(
            starts >= 1,
            "event.start must be called at least once when rust_active=true live=false; got start_count={starts}"
        );
        assert_eq!(
            stops, 0,
            "event.stop must NOT be called; got stop_count={stops}"
        );
    }

    /// Test 2: in-scope event where rust_active=false but live is_active=true.
    /// Asserts event.stop is called exactly once.
    #[tokio::test]
    async fn drive_mode_calls_event_stop_when_rust_inactive_live_active() {
        // Event 21: Normal, periodic, no holiday.
        // raw: start = DRIVE_SERVER_GAMETIME - 30*86400 (30 days ago),
        //      end = DRIVE_SERVER_GAMETIME + 86400, occ=525600 min, len=20160 min.
        // Rust computes: elapsed=30*86400=2592000s > len_secs=1209600s → rust_active=FALSE.
        // gt: is_active=true (divergence — drive should fix it via event.stop).
        let start = DRIVE_SERVER_GAMETIME - 30 * 86400;
        let end = DRIVE_SERVER_GAMETIME + 86400;

        let game_events_json = format!(r#"{{
            "events":[
                {{"entry":21,"start":{start},"end":{end},"occurence":525600,"length":20160,
                  "holiday":0,"holiday_stage":0,"is_active":true,"next_start":0,"state":0}}
            ],
            "active_event_list":[21],
            "holidays":[],
            "server_gametime":{DRIVE_SERVER_GAMETIME},
            "resolve_reference_unixtime":{DRIVE_SERVER_GAMETIME},
            "server_tz_offset_secs":0
        }}"#);

        let query_db_json = format!(r#"{{
            "row_count":1,
            "rows":[
                {{"eventEntry":21,"start_time":{start},"end_time":{end},
                  "occurence":525600,"length":20160,"holiday":0,"holidayStage":0,
                  "description":"test","world_event":0,"announce":0}}
            ]
        }}"#);

        let (base_url, start_count, stop_count) =
            spawn_drive_mock(&game_events_json, &query_db_json).await;

        let state = Arc::new(AppState::new());
        let harness = Harness::new(base_url, "test-token");
        let cfg = Config {
            listen_addr: "127.0.0.1:8091".to_string(),
            harness_base_url: "unused".to_string(),
            harness_bearer: "unused".to_string(),
            tick_secs: 1,
            drive: true,
            live_shadow_resolution: false,
        };

        let handle = tokio::spawn(run(state.clone(), harness, cfg));
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();
        let _ = handle.await;

        let starts = start_count.load(Ordering::SeqCst);
        let stops = stop_count.load(Ordering::SeqCst);

        assert!(
            stops >= 1,
            "event.stop must be called at least once when rust_active=false live=true; got stop_count={stops}"
        );
        assert_eq!(
            starts, 0,
            "event.start must NOT be called; got start_count={starts}"
        );
    }

    /// Test 3: excluded events (Internal state and ManualStart) are NEVER driven,
    /// even if there is a divergence between rust and live active state.
    #[tokio::test]
    async fn drive_mode_never_drives_excluded_events() {
        // Event 50: state=Internal (5), live=true, raw start=recent end=future.
        // Rust would compute rust_active=true, so there is no divergence... but
        // to make it more interesting (and test the exclusion strictly), use a
        // case where there WOULD be a divergence if not excluded:
        // Event 50: state=Internal (5), live=false (gt says inactive),
        //   raw start=recent, len=long → rust_active=true. Without exclusion,
        //   event.start would fire. With exclusion: NEVER fires.
        //
        // Event 60: ManualStart — state=Normal, holiday=0, raw start=Some(far_past),
        //   end=Some(far_past) (equal), live=false, rust would compute inactive too
        //   (start < end = same timestamp → zero-length window).
        //   Actually rust_active depends on the math: start=end means zero-length
        //   window but start=0+something... Let's use a case where even if rust and
        //   live differ, the exclusion fires first.
        //   ManualStart has raw_start==raw_end << RESOLVE_REF.
        //   rust_resolved = periodic with start=far_past, end=far_past; is_active = false.
        //   live is_active = true. → divergence. But excluded → no event.stop call.
        let far_past: i64 = 946_735_200; // year 2000 — well before DRIVE_SERVER_GAMETIME
        let internal_start = DRIVE_SERVER_GAMETIME - 3600;
        let internal_end = DRIVE_SERVER_GAMETIME + 86400;

        let game_events_json = format!(r#"{{
            "events":[
                {{"entry":50,"start":{internal_start},"end":{internal_end},"occurence":525600,"length":20160,
                  "holiday":0,"holiday_stage":0,"is_active":false,"next_start":0,"state":5}},
                {{"entry":60,"start":{far_past},"end":{far_past},"occurence":525600,"length":20160,
                  "holiday":0,"holiday_stage":0,"is_active":true,"next_start":0,"state":0}}
            ],
            "active_event_list":[60],
            "holidays":[],
            "server_gametime":{DRIVE_SERVER_GAMETIME},
            "resolve_reference_unixtime":{DRIVE_SERVER_GAMETIME},
            "server_tz_offset_secs":0
        }}"#);

        let query_db_json = format!(r#"{{
            "row_count":2,
            "rows":[
                {{"eventEntry":50,"start_time":{internal_start},"end_time":{internal_end},
                  "occurence":525600,"length":20160,"holiday":0,"holidayStage":0,
                  "description":"internal","world_event":0,"announce":0}},
                {{"eventEntry":60,"start_time":{far_past},"end_time":{far_past},
                  "occurence":525600,"length":20160,"holiday":0,"holidayStage":0,
                  "description":"manual","world_event":0,"announce":0}}
            ]
        }}"#);

        let (base_url, start_count, stop_count) =
            spawn_drive_mock(&game_events_json, &query_db_json).await;

        let state = Arc::new(AppState::new());
        let harness = Harness::new(base_url, "test-token");
        let cfg = Config {
            listen_addr: "127.0.0.1:8091".to_string(),
            harness_base_url: "unused".to_string(),
            harness_bearer: "unused".to_string(),
            tick_secs: 1,
            drive: true,
            live_shadow_resolution: false,
        };

        let handle = tokio::spawn(run(state.clone(), harness, cfg));
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();
        let _ = handle.await;

        let starts = start_count.load(Ordering::SeqCst);
        let stops = stop_count.load(Ordering::SeqCst);

        assert_eq!(
            starts, 0,
            "event.start must NEVER be called for excluded events (Internal); got start_count={starts}"
        );
        assert_eq!(
            stops, 0,
            "event.stop must NEVER be called for excluded events (ManualStart); got stop_count={stops}"
        );
    }

    /// Test 4 (shadow-only): drive=false → event.start/stop are never called even with divergence.
    #[tokio::test]
    async fn shadow_only_mode_never_calls_event_start_stop() {
        // Same divergence as Test 1 (rust_active=true, live=false), but drive=false.
        let start = DRIVE_SERVER_GAMETIME - 3600;
        let end = DRIVE_SERVER_GAMETIME + 86400;

        let game_events_json = format!(r#"{{
            "events":[
                {{"entry":20,"start":{start},"end":{end},"occurence":525600,"length":20160,
                  "holiday":0,"holiday_stage":0,"is_active":false,"next_start":0,"state":0}}
            ],
            "active_event_list":[],
            "holidays":[],
            "server_gametime":{DRIVE_SERVER_GAMETIME},
            "resolve_reference_unixtime":{DRIVE_SERVER_GAMETIME},
            "server_tz_offset_secs":0
        }}"#);

        let query_db_json = format!(r#"{{
            "row_count":1,
            "rows":[
                {{"eventEntry":20,"start_time":{start},"end_time":{end},
                  "occurence":525600,"length":20160,"holiday":0,"holidayStage":0,
                  "description":"test","world_event":0,"announce":0}}
            ]
        }}"#);

        let (base_url, start_count, stop_count) =
            spawn_drive_mock(&game_events_json, &query_db_json).await;

        let state = Arc::new(AppState::new());
        let harness = Harness::new(base_url, "test-token");
        let cfg = Config {
            listen_addr: "127.0.0.1:8091".to_string(),
            harness_base_url: "unused".to_string(),
            harness_bearer: "unused".to_string(),
            tick_secs: 1,
            drive: false,  // shadow-only — must NOT drive
            live_shadow_resolution: false,
        };

        let handle = tokio::spawn(run(state.clone(), harness, cfg));
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();
        let _ = handle.await;

        assert_eq!(
            start_count.load(Ordering::SeqCst), 0,
            "shadow-only mode must never call event.start"
        );
        assert_eq!(
            stop_count.load(Ordering::SeqCst), 0,
            "shadow-only mode must never call event.stop"
        );
    }
}
