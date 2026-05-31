//! HTTP report API for the game-event-scheduler.
//!
//! Exposes three endpoints:
//! - `GET /healthz` → `"ok"` (liveness probe).
//! - `GET /report`  → latest [`ShadowReport`] as JSON (404 if no tick has run yet).
//! - `GET /report/history` → soak summary with vs_field/vs_list mismatch split.
//!
//! The shared state ([`AppState`]) is held in an `Arc` and injected via axum's
//! [`State`] extractor.  The `Mutex`es guard short, non-async critical sections
//! (just a clone or push), so `std::sync::Mutex` is appropriate here — no risk
//! of holding the lock across an `.await` point.

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::get,
    Json,
};
use serde::Serialize;
use std::sync::{Arc, Mutex};

use crate::shadow::ShadowReport;

// ── AppState ──────────────────────────────────────────────────────────────────

/// Shared state between the tick loop and the HTTP API handlers.
pub struct AppState {
    /// The most recently computed [`ShadowReport`].  `None` until the first tick.
    pub latest: Mutex<Option<ShadowReport>>,
    /// Bounded history of per-tick summaries (capped at 500; oldest dropped).
    pub history: Mutex<Vec<ReportSummary>>,
}

impl AppState {
    /// Create a new, empty `AppState`.
    pub fn new() -> Self {
        AppState {
            latest: Mutex::new(None),
            history: Mutex::new(Vec::new()),
        }
    }

    /// Store a new report as the latest and push a summary into history.
    ///
    /// If history has reached the cap (500), the oldest entry is removed.
    pub fn push_report(&self, report: ShadowReport) {
        let summary = ReportSummary::from_report(&report);
        {
            let mut latest = self.latest.lock().expect("latest mutex poisoned");
            *latest = Some(report);
        }
        {
            let mut history = self.history.lock().expect("history mutex poisoned");
            if history.len() >= 500 {
                history.remove(0);
            }
            history.push(summary);
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ── ReportSummary ─────────────────────────────────────────────────────────────

/// Compact per-tick summary stored in history.
#[derive(Debug, Clone, Serialize)]
pub struct ReportSummary {
    pub server_gametime: i64,
    pub active_set_matched: usize,
    pub active_set_checked: usize,
    pub resolution_matched: usize,
    pub resolution_checked: usize,
    pub date_math_matched: usize,
    pub date_math_checked: usize,
    /// Total mismatch count across all checks.
    pub mismatch_count: usize,
    /// Mismatches in `is_active(vs_field)` (Rust vs C++ `is_active` field).
    pub vs_field_mismatches: usize,
    /// Mismatches in `is_active(vs_list)` (Rust vs `active_event_list` membership).
    pub vs_list_mismatches: usize,
}

impl ReportSummary {
    pub fn from_report(report: &ShadowReport) -> Self {
        let vs_field = report
            .mismatches
            .iter()
            .filter(|m| m.field.contains("vs_field"))
            .count();
        let vs_list = report
            .mismatches
            .iter()
            .filter(|m| m.field.contains("vs_list"))
            .count();
        ReportSummary {
            server_gametime: report.server_gametime,
            active_set_matched: report.active_set.matched,
            active_set_checked: report.active_set.checked,
            resolution_matched: report.resolution.matched,
            resolution_checked: report.resolution.checked,
            date_math_matched: report.date_math.matched,
            date_math_checked: report.date_math.checked,
            mismatch_count: report.mismatches.len(),
            vs_field_mismatches: vs_field,
            vs_list_mismatches: vs_list,
        }
    }
}

// ── HistorySummary response ───────────────────────────────────────────────────

/// Response body for `GET /report/history`.
#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub total_ticks: usize,
    /// Number of ticks where `active_set` had zero mismatches (primary exit gate).
    pub ticks_zero_active_set_mismatch: usize,
    /// Server gametime from the most recent tick (0 if no ticks yet).
    pub latest_gametime: i64,
    /// Per-tick summaries that had at least one mismatch of any kind.
    pub any_mismatches: Vec<ReportSummary>,
}

// ── Router ────────────────────────────────────────────────────────────────────

/// Build the axum [`Router`] with the three report endpoints wired to `state`.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(handle_healthz))
        .route("/report", get(handle_report))
        .route("/report/history", get(handle_history))
        .with_state(state)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn handle_healthz() -> &'static str {
    "ok"
}

async fn handle_report(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ShadowReport>, StatusCode> {
    let latest = state.latest.lock().expect("latest mutex poisoned");
    match &*latest {
        Some(report) => Ok(Json(report.clone())),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn handle_history(State(state): State<Arc<AppState>>) -> Json<HistoryResponse> {
    let history = state.history.lock().expect("history mutex poisoned");
    let total = history.len();
    let zero_mismatch = history
        .iter()
        .filter(|s| s.active_set_matched == s.active_set_checked)
        .count();
    let latest_gametime = history.last().map(|s| s.server_gametime).unwrap_or(0);
    let any_mismatches: Vec<ReportSummary> =
        history.iter().filter(|s| s.mismatch_count > 0).cloned().collect();
    Json(HistoryResponse {
        total_ticks: total,
        ticks_zero_active_set_mismatch: zero_mismatch,
        latest_gametime,
        any_mismatches,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow::{CheckResult, CheckKind, Mismatch};
    use tokio::net::TcpListener;

    // ── Fixture helpers ───────────────────────────────────────────────────────

    fn make_check_result(checked: usize, matched: usize) -> CheckResult {
        CheckResult { checked, matched }
    }

    fn make_report(
        server_gametime: i64,
        active_checked: usize,
        active_matched: usize,
        mismatches: Vec<Mismatch>,
    ) -> ShadowReport {
        ShadowReport {
            server_gametime,
            resolve_reference_unixtime: server_gametime,
            date_math: make_check_result(4, 4),
            resolution: make_check_result(8, 8),
            active_set: make_check_result(active_checked, active_matched),
            active_set_exclusions: vec![],
            mismatches,
        }
    }

    fn make_vs_field_mismatch(key: &str) -> Mismatch {
        Mismatch {
            kind: CheckKind::ActiveSet,
            key: key.to_string(),
            field: "is_active(vs_field)".to_string(),
            rust: "true".to_string(),
            cpp: "false".to_string(),
        }
    }

    fn make_vs_list_mismatch(key: &str) -> Mismatch {
        Mismatch {
            kind: CheckKind::ActiveSet,
            key: key.to_string(),
            field: "is_active(vs_list)".to_string(),
            rust: "true".to_string(),
            cpp: "false".to_string(),
        }
    }

    /// Spawn the router on an ephemeral port; return (base_url, Arc<AppState>).
    async fn spawn_router() -> (String, Arc<AppState>) {
        let state = Arc::new(AppState::new());
        let app = router(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{}", addr), state)
    }

    // ── /healthz ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn healthz_returns_200_ok() {
        let (base_url, _state) = spawn_router().await;
        let resp = reqwest::get(format!("{base_url}/healthz")).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.text().await.unwrap();
        assert_eq!(body, "ok");
    }

    // ── /report before any tick ───────────────────────────────────────────────

    #[tokio::test]
    async fn report_returns_404_when_no_tick_yet() {
        let (base_url, _state) = spawn_router().await;
        let resp = reqwest::get(format!("{base_url}/report")).await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    // ── /report after injecting a report ─────────────────────────────────────

    #[tokio::test]
    async fn report_returns_injected_shadow_report() {
        let (base_url, state) = spawn_router().await;
        let report = make_report(1_780_394_400, 6, 6, vec![]);
        state.push_report(report.clone());

        let resp = reqwest::get(format!("{base_url}/report")).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(
            body["server_gametime"].as_i64().unwrap(),
            1_780_394_400,
            "server_gametime must round-trip"
        );
        assert_eq!(
            body["active_set"]["checked"].as_u64().unwrap(),
            6,
            "active_set.checked must round-trip"
        );
        assert_eq!(
            body["active_set"]["matched"].as_u64().unwrap(),
            6,
            "active_set.matched must round-trip"
        );
        assert!(
            body["mismatches"].as_array().unwrap().is_empty(),
            "mismatches must be empty"
        );
    }

    // ── /report/history with 2 summaries ─────────────────────────────────────

    #[tokio::test]
    async fn history_reflects_pushed_summaries() {
        let (base_url, state) = spawn_router().await;

        // Tick 1: clean (no mismatches)
        let report1 = make_report(1_000, 4, 4, vec![]);
        state.push_report(report1);

        // Tick 2: has one vs_field + one vs_list mismatch
        let report2 = make_report(
            2_000,
            4,
            2,
            vec![
                make_vs_field_mismatch("event:10"),
                make_vs_list_mismatch("event:10"),
            ],
        );
        state.push_report(report2);

        let resp = reqwest::get(format!("{base_url}/report/history")).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["total_ticks"].as_u64().unwrap(), 2, "total_ticks");
        assert_eq!(
            body["ticks_zero_active_set_mismatch"].as_u64().unwrap(),
            1,
            "only tick 1 has zero active_set mismatches"
        );
        assert_eq!(body["latest_gametime"].as_i64().unwrap(), 2_000, "latest_gametime");

        // any_mismatches: only tick 2 had mismatches
        let any = body["any_mismatches"].as_array().unwrap();
        assert_eq!(any.len(), 1, "only one tick had mismatches");
        assert_eq!(any[0]["vs_field_mismatches"].as_u64().unwrap(), 1, "vs_field_mismatches");
        assert_eq!(any[0]["vs_list_mismatches"].as_u64().unwrap(), 1, "vs_list_mismatches");
    }

    // ── vs_field / vs_list split is distinguishable ───────────────────────────

    #[tokio::test]
    async fn history_vs_field_vs_list_split_is_correct() {
        let (_base_url, state) = spawn_router().await;

        // 2 vs_field + 1 vs_list
        let report = make_report(
            3_000,
            6,
            3,
            vec![
                make_vs_field_mismatch("event:1"),
                make_vs_field_mismatch("event:2"),
                make_vs_list_mismatch("event:3"),
            ],
        );
        state.push_report(report);

        let history = state.history.lock().unwrap();
        let summary = &history[0];
        assert_eq!(summary.vs_field_mismatches, 2, "vs_field");
        assert_eq!(summary.vs_list_mismatches, 1, "vs_list");
    }

    // ── History cap at 500 ────────────────────────────────────────────────────

    #[tokio::test]
    async fn history_capped_at_500() {
        let state = Arc::new(AppState::new());
        for i in 0..510u64 {
            let report = make_report(i as i64, 2, 2, vec![]);
            state.push_report(report);
        }
        let history = state.history.lock().unwrap();
        assert_eq!(history.len(), 500, "history must be capped at 500");
        // The oldest entries (0..9) were dropped; the latest entry has gametime 509.
        assert_eq!(history.last().unwrap().server_gametime, 509);
        assert_eq!(history.first().unwrap().server_gametime, 10);
    }

    // ── ReportSummary::from_report field mapping ──────────────────────────────

    #[test]
    fn report_summary_fields_are_correct() {
        let report = make_report(
            999,
            4,
            3,
            vec![make_vs_field_mismatch("event:1")],
        );
        let summary = ReportSummary::from_report(&report);
        assert_eq!(summary.server_gametime, 999);
        assert_eq!(summary.active_set_checked, 4);
        assert_eq!(summary.active_set_matched, 3);
        assert_eq!(summary.mismatch_count, 1);
        assert_eq!(summary.vs_field_mismatches, 1);
        assert_eq!(summary.vs_list_mismatches, 0);
    }
}
