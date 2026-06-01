//! `slice-host` — single binary that runs all Rust strangler-fig slices in one container.
//!
//! # Environment variables
//!
//! | Variable           | Required | Default           | Description                                   |
//! |--------------------|----------|-------------------|-----------------------------------------------|
//! | `HARNESS_BASE_URL` | yes      | —                 | e.g. `http://192.168.1.3:8099`                |
//! | `HARNESS_BEARER`   | yes      | —                 | Raw bearer token (no `Bearer ` prefix)        |
//! | `GES_TICK_SECS`    | no       | `15`              | Scheduler tick interval                       |
//! | `GES_DRIVE`        | no       | `false`           | Enable scheduler drive mode                   |
//! | `LFG_TICK_SECS`    | no       | `2`               | LFG matchmaker tick interval                  |
//! | `LFG_ENABLED`      | no       | `false`           | Enable LFG mutating matchmaking actions       |
//! | `SLICE_HOST_LISTEN`| no       | `0.0.0.0:8092`    | Bind address for the host HTTP server         |
//!
//! # Route layout
//!
//! - `GET  /healthz`                    — host liveness probe (always `"ok"`)
//! - `GET  /game-events/report`         — latest scheduler [`ShadowReport`]
//! - `GET  /game-events/report/history` — scheduler tick history
//! - `GET  /lfg/queue`                  — list current LFG queue entries
//! - `POST /lfg/queue`                  — enqueue a bot/player intent
//! - `DELETE /lfg/queue/:guid`          — dequeue by guid
//!
//! # Safety — LFG inert by default
//!
//! The LFG slice is always spawned (its HTTP routes are always live) but the
//! mutating matchmaking actions (`lfg.form_group`, `bot.invite_to_group`,
//! `bot.enter_instance`, etc.) are gated behind `LFG_ENABLED` (default `false`).
//! The deployed service will NOT start performing live matchmaking just because
//! the code is compiled in.  Set `LFG_ENABLED=true` to activate.

use std::sync::Arc;
use std::time::Duration;

use axum::{Router, routing::get};
use tokio::net::TcpListener;

use game_event_scheduler::api::AppState as GesAppState;
use game_event_scheduler::config::Config as GesConfig;
use game_event_scheduler::harness::Harness;
use game_event_scheduler::{api as ges_api, tick as ges_tick};

use lfg_matchmaker::api::{AppState as LfgAppState, PendingPlacements as LfgPendingPlacements};
use lfg_matchmaker::config::Config as LfgConfig;
use lfg_matchmaker::queue::Queue as LfgQueue;
use lfg_matchmaker::{api as lfg_api, tick as lfg_tick};

// ── Configuration ─────────────────────────────────────────────────────────────

/// Read `SLICE_HOST_LISTEN` (default `0.0.0.0:8092`).
fn listen_addr() -> String {
    std::env::var("SLICE_HOST_LISTEN").unwrap_or_else(|_| "0.0.0.0:8092".to_string())
}

/// Read required harness env vars; exit(2) on any missing.
fn require_env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| {
        eprintln!("[slice-host] required env var {key} is not set");
        std::process::exit(2);
    })
}

// ── Supervised task spawner ────────────────────────────────────────────────────

/// Spawn a long-running async task under a supervisor loop.
///
/// The supervisor catches both panics and unexpected returns.  On either, it logs
/// and waits `backoff` before calling `factory` again to get a fresh future.  A
/// panic in one slice's loop does NOT take down the host or other slices.
///
/// Implementation: each iteration spawns the slice as its own [`tokio::task`] and
/// awaits its [`JoinHandle`].  Tokio's `JoinError` distinguishes panics from
/// cancellations.  Using a nested `tokio::spawn` (rather than `catch_unwind`) is
/// the idiomatic approach for async panic isolation in Rust — discussed in
/// Palmieri "Zero to Production" Ch 11 and Gjengset "Rust for Rustaceans" Ch 8.
///
/// `name`    — human-readable label used in log messages.
/// `factory` — `FnMut() -> F` called on each (re)start to produce a new Future.
/// `backoff` — how long to wait before restarting after a failure.
fn spawn_supervised<F, Fut>(
    name: &'static str,
    mut factory: F,
    backoff: Duration,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            eprintln!("[slice-host] starting slice '{name}'");
            // Spawn the slice as a child task; await its JoinHandle.
            // Tokio captures panics as JoinError::is_panic() == true.
            let handle = tokio::spawn(factory());
            match handle.await {
                Ok(()) => {
                    // Task returned normally — unexpected for an infinite loop.
                    eprintln!(
                        "[slice-host] slice '{name}' returned unexpectedly; \
                         restarting after {backoff:?}"
                    );
                }
                Err(join_err) if join_err.is_panic() => {
                    eprintln!(
                        "[slice-host] slice '{name}' panicked; \
                         restarting after {backoff:?}"
                    );
                }
                Err(_cancelled) => {
                    // Task was aborted externally (e.g. during shutdown) — stop the
                    // supervisor loop rather than restart.
                    eprintln!("[slice-host] slice '{name}' was cancelled — stopping supervisor");
                    return;
                }
            }
            tokio::time::sleep(backoff).await;
        }
    })
}

// ── App router ────────────────────────────────────────────────────────────────

/// Build the combined axum [`Router`] for all hosted slices.
///
/// Route layout:
/// - `GET /healthz`                        — host liveness probe
/// - `GET /game-events/report`             — scheduler latest report
/// - `GET /game-events/report/history`     — scheduler tick history
/// - `GET /lfg/queue`                      — list LFG queue
/// - `POST /lfg/queue`                     — enqueue intent
/// - `DELETE /lfg/queue/:guid`             — dequeue by guid
fn build_app(ges_state: Arc<GesAppState>, lfg_state: Arc<LfgAppState>) -> Router {
    // The scheduler's routes() returns /report and /report/history (no /healthz).
    let ges_routes = ges_api::routes(ges_state);
    // The LFG matchmaker's routes() returns /queue routes (no /healthz).
    let lfg_routes = lfg_api::routes(lfg_state);

    Router::new()
        // Host-level liveness probe.
        .route("/healthz", get(|| async { "ok" }))
        // Scheduler slice, mounted under /game-events.
        .nest("/game-events", ges_routes)
        // LFG matchmaker slice, mounted under /lfg.
        .nest("/lfg", lfg_routes)
}

// ── Shutdown signal ────────────────────────────────────────────────────────────

/// Resolve when SIGTERM or Ctrl-C is received.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("[slice-host] received SIGINT — shutting down");
            }
            _ = sigterm.recv() => {
                eprintln!("[slice-host] received SIGTERM — shutting down");
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("Ctrl-C handler");
        eprintln!("[slice-host] received Ctrl-C — shutting down");
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let listen = listen_addr();
    let harness_base_url = require_env("HARNESS_BASE_URL");
    let harness_bearer = require_env("HARNESS_BEARER");

    eprintln!("[slice-host] starting — listen={listen} harness={harness_base_url}");

    // Each slice has its own Harness type (independent reqwest client pools).
    // Both point at the same harness base URL + bearer.

    // ── Game-event-scheduler slice ─────────────────────────────────────────────
    let ges_cfg = match GesConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[slice-host] game-event-scheduler config error: {e}");
            std::process::exit(2);
        }
    };
    let ges_state = Arc::new(GesAppState::new());

    // Capture clones for the supervisor factory (called on each restart).
    let ges_state_for_task = ges_state.clone();
    let ges_harness = Harness::new(harness_base_url.clone(), harness_bearer.clone());
    let ges_cfg_for_task = ges_cfg.clone();

    // Spawn the scheduler tick loop under a supervisor.
    // A panic or unexpected return is caught; the supervisor logs and restarts
    // after a 5-second backoff.  This must NOT take down the host or other slices.
    spawn_supervised(
        "game-event-scheduler",
        move || {
            // Factory: clone Arc/values for each restart.
            let state = ges_state_for_task.clone();
            let h = ges_harness.clone();
            let cfg = ges_cfg_for_task.clone();
            ges_tick::run(state, h, cfg)
        },
        Duration::from_secs(5),
    );

    // ── LFG matchmaker slice ────────────────────────────────────────────────────
    //
    // SAFETY: The LFG slice is always spawned so the /lfg HTTP routes are
    // available, but mutating matchmaking actions are gated behind LFG_ENABLED
    // (default false).  The deployed service will NOT perform live matchmaking
    // until LFG_ENABLED=true is explicitly set.
    let lfg_cfg = match LfgConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[slice-host] lfg-matchmaker config error: {e}");
            std::process::exit(2);
        }
    };
    let lfg_state = Arc::new(LfgAppState { queue: LfgQueue::new(), pending_placements: LfgPendingPlacements::new() });

    let lfg_state_for_task = lfg_state.clone();
    let lfg_harness = lfg_matchmaker::harness::Harness::new(harness_base_url.clone(), harness_bearer.clone());
    let lfg_cfg_for_task = lfg_cfg.clone();

    spawn_supervised(
        "lfg-matchmaker",
        move || {
            let state = lfg_state_for_task.clone();
            let h = lfg_harness.clone();
            let cfg = lfg_cfg_for_task.clone();
            lfg_tick::run(state, h, cfg)
        },
        Duration::from_secs(5),
    );

    // ── HTTP server ────────────────────────────────────────────────────────────
    let app = build_app(ges_state, lfg_state);

    let listener = match TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[slice-host] failed to bind {listen}: {e}");
            std::process::exit(2);
        }
    };

    eprintln!("[slice-host] listening on {listen}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap_or_else(|e| eprintln!("[slice-host] server error: {e}"));

    eprintln!("[slice-host] shut down cleanly");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`

    /// Build fresh state for both slices and return the composed router.
    fn make_app() -> Router {
        let ges_state = Arc::new(GesAppState::new());
        let lfg_state = Arc::new(LfgAppState { queue: LfgQueue::new(), pending_placements: LfgPendingPlacements::new() });
        build_app(ges_state, lfg_state)
    }

    /// Build a test router with fresh state and call /healthz.
    #[tokio::test]
    async fn healthz_returns_ok() {
        let app = make_app();
        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"ok");
    }

    /// /game-events/report returns 404 before any tick has run.
    #[tokio::test]
    async fn game_events_report_404_before_tick() {
        let app = make_app();
        let req = Request::builder()
            .uri("/game-events/report")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// /game-events/report/history returns 200 with empty list.
    #[tokio::test]
    async fn game_events_history_200_empty() {
        let app = make_app();
        let req = Request::builder()
            .uri("/game-events/report/history")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// /lfg/queue returns 200 with an empty array before any entries are added.
    #[tokio::test]
    async fn lfg_queue_get_returns_200_empty() {
        let app = make_app();
        let req = Request::builder()
            .uri("/lfg/queue")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            parsed.as_array().unwrap().is_empty(),
            "/lfg/queue must return empty array before any enqueue"
        );
    }

    /// POST /lfg/queue enqueues a valid entry; GET confirms it is present.
    #[tokio::test]
    async fn lfg_queue_enqueue_and_list() {
        let ges_state = Arc::new(GesAppState::new());
        let lfg_state = Arc::new(LfgAppState { queue: LfgQueue::new(), pending_placements: LfgPendingPlacements::new() });
        let app = build_app(ges_state, lfg_state.clone());

        let req = Request::builder()
            .method("POST")
            .uri("/lfg/queue")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "guid": 42,
                    "role": "tank",
                    "dungeon_id": 36,
                    "faction": "alliance"
                })
                .to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Build a fresh router bound to the same state to query.
        let app2 = build_app(Arc::new(GesAppState::new()), lfg_state.clone());
        let req2 = Request::builder()
            .uri("/lfg/queue")
            .body(Body::empty())
            .unwrap();
        let resp2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp2.into_body(), 4096).await.unwrap();
        let entries: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = entries.as_array().unwrap();
        assert_eq!(arr.len(), 1, "queue must contain the enqueued entry");
        assert_eq!(arr[0]["guid"], 42);
    }

    /// POST /lfg/queue with a bad role returns 400.
    #[tokio::test]
    async fn lfg_queue_bad_role_returns_400() {
        let app = make_app();
        let req = Request::builder()
            .method("POST")
            .uri("/lfg/queue")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "guid": 1,
                    "role": "wizard",
                    "dungeon_id": 4,
                    "faction": "alliance"
                })
                .to_string(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// The supervisor loop restarts after a task that returns immediately.
    /// We verify that the factory function is called more than once.
    #[tokio::test]
    async fn supervisor_restarts_after_immediate_return() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let call_count = Arc::new(AtomicU32::new(0));
        let count_clone = call_count.clone();

        let handle = spawn_supervised(
            "test-slice",
            move || {
                let c = count_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    // Return immediately — the supervisor should restart us.
                }
            },
            Duration::from_millis(20), // short backoff for the test
        );

        // Give the supervisor a chance to restart a few times.
        tokio::time::sleep(Duration::from_millis(120)).await;
        handle.abort();
        let _ = handle.await;

        let n = call_count.load(Ordering::SeqCst);
        assert!(n >= 2, "supervisor must restart the slice after it returns; got {n} calls");
    }

    /// A panicking task is caught and restarted; the supervisor JoinHandle stays alive.
    #[tokio::test]
    async fn supervisor_restarts_after_panic() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let call_count = Arc::new(AtomicU32::new(0));
        let count_clone = call_count.clone();

        let handle = spawn_supervised(
            "panic-slice",
            move || {
                let c = count_clone.clone();
                async move {
                    let n = c.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        panic!("intentional test panic on first call");
                    }
                    // Second call: just return so the test can finish.
                }
            },
            Duration::from_millis(20),
        );

        tokio::time::sleep(Duration::from_millis(120)).await;
        handle.abort();
        let _ = handle.await;

        let n = call_count.load(Ordering::SeqCst);
        assert!(n >= 2, "supervisor must restart after panic; got {n} calls");
    }
}
