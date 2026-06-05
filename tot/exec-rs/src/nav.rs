//! Navmesh client — thin wrapper over the `nav.find_path` harness primitive.
//!
//! Request:  `{ "bot_guid": <i64>, "dest_x": <f64>, "dest_y": <f64>, "dest_z": <f64> }`
//! Response: `{ "path_type": <i64 bitmask>, "points": [{"x":f64,"y":f64,"z":f64},...] }`

use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use tot_harness_client::{HarnessClient, HarnessError};

/// AC `PathType` bitmask values from `PathGenerator.h`.
pub mod flags {
    pub const PATHFIND_NORMAL: i64         = 0x01;
    pub const PATHFIND_INCOMPLETE: i64     = 0x04;
    pub const PATHFIND_NOPATH: i64         = 0x08;
    pub const PATHFIND_NOT_USING_PATH: i64 = 0x10;
}

#[derive(Debug, Clone, Deserialize)]
pub struct PathPoint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Typed result of a `nav.find_path` call.
#[derive(Debug, Clone)]
pub struct Path {
    /// Raw AC `PathType` bitmask. Test with [`flags`] constants.
    pub path_type: i64,
    /// Ordered waypoints from the bot's current position to the destination.
    pub points: Vec<PathPoint>,
}

impl Path {
    pub fn is_normal(&self) -> bool { self.path_type & flags::PATHFIND_NORMAL != 0 }
    pub fn is_incomplete(&self) -> bool { self.path_type & flags::PATHFIND_INCOMPLETE != 0 }
    pub fn is_no_path(&self) -> bool { self.path_type & flags::PATHFIND_NOPATH != 0 }
    pub fn is_not_using_path(&self) -> bool { self.path_type & flags::PATHFIND_NOT_USING_PATH != 0 }
}

#[derive(Debug, Deserialize)]
struct NavFindPathResult {
    path_type: i64,
    points: Vec<PathPoint>,
}

#[derive(Debug, Error)]
pub enum NavError {
    #[error("harness: {0}")]
    Harness(#[from] HarnessError),
    #[error("nav.find_path: unexpected response shape: {0}")]
    Shape(String),
    #[error("no navigable path to destination")]
    NoPath,
    #[error("bot did not arrive within the time budget")]
    Timeout,
    #[error("bot made no progress toward destination after {0} re-path attempts")]
    Stuck(u32),
    #[error("re-path budget exhausted after {0} attempts without reaching destination")]
    RepathBudgetExceeded(u32),
}

/// Destination coordinates in WoW world-space.
#[derive(Debug, Clone, Copy)]
pub struct Dest {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Call `nav.find_path` for `bot_guid` to `dest` and return the typed path.
pub async fn find_path(
    client: &HarnessClient,
    bot_guid: u64,
    dest: Dest,
) -> Result<Path, NavError> {
    let result = client
        .call(
            "nav.find_path",
            json!({
                "bot_guid": bot_guid,
                "dest_x": dest.x,
                "dest_y": dest.y,
                "dest_z": dest.z,
            }),
        )
        .await?;

    let nav_result: NavFindPathResult =
        serde_json::from_value(result).map_err(|e| NavError::Shape(format!("serde: {e}")))?;

    Ok(Path { path_type: nav_result.path_type, points: nav_result.points })
}

/// Typed result of a `bot.move_path` call.
#[derive(Debug, Clone)]
pub struct MoveResult {
    pub launched: bool,
    pub duration_ms: i64,
    /// The last point the worldserver launched the spline toward.
    pub final_point: PathPoint,
}

#[derive(Debug, Deserialize)]
struct BotMovePathResult {
    launched: bool,
    duration_ms: i64,
    #[serde(rename = "final")]
    final_point: PathPoint,
}

/// Call `bot.move_path` with the given waypoints and return the typed result.
pub async fn move_path(
    client: &HarnessClient,
    bot_guid: u64,
    points: &[PathPoint],
) -> Result<MoveResult, NavError> {
    let points_json: Vec<serde_json::Value> = points
        .iter()
        .map(|p| serde_json::json!({"x": p.x, "y": p.y, "z": p.z}))
        .collect();
    let result = client
        .call("bot.move_path", serde_json::json!({ "bot_guid": bot_guid as i64, "points": points_json }))
        .await?;
    let mr: BotMovePathResult =
        serde_json::from_value(result).map_err(|e| NavError::Shape(format!("bot.move_path serde: {e}")))?;
    Ok(MoveResult { launched: mr.launched, duration_ms: mr.duration_ms, final_point: mr.final_point })
}

#[derive(Debug, Clone, Deserialize)]
struct RawPosition { x: f64, y: f64, z: f64 }

/// Call `obs.get_position` (uses `target_guid` per the adapter contract) → world-space coords.
async fn get_position(client: &HarnessClient, target_guid: u64) -> Result<PathPoint, NavError> {
    let result = client
        .call("obs.get_position", serde_json::json!({"target_guid": target_guid as i64}))
        .await?;
    let pos: RawPosition =
        serde_json::from_value(result).map_err(|e| NavError::Shape(format!("obs.get_position serde: {e}")))?;
    Ok(PathPoint { x: pos.x, y: pos.y, z: pos.z })
}

const MAX_WAIT_MS: i64 = 30_000;
const ARRIVAL_TOLERANCE: f64 = 2.0;
const MAX_ARRIVAL_CHECKS: u32 = 5;
const RECHECK_INTERVAL_MS: u64 = 500;
const MAX_REPATH: u32 = 5;
const STUCK_THRESHOLD: f64 = 1.0;

fn dist(a: &PathPoint, b: &PathPoint) -> f64 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Drive `bot_guid` to `dest` using the server navmesh: find_path → move_path → await arrival,
/// re-querying on INCOMPLETE paths. The worldserver anchors each `find_path` from the bot's
/// live position, so re-paths need no explicit new-origin arg (same as M0).
///
/// Stuck detection: after each INCOMPLETE segment completes, we measure the bot's distance
/// to the FINAL destination. If it hasn't closed the gap by at least `STUCK_THRESHOLD` yards
/// since the previous iteration, we return `NavError::Stuck`. This cannot false-trigger on a
/// legitimately-advancing walk (which by definition reduces the distance to dest by more than
/// `STUCK_THRESHOLD` each segment). The separate `NavError::RepathBudgetExceeded` is returned
/// when the raw re-path count cap (`MAX_REPATH`) is hit regardless of progress.
pub async fn walk_to(client: &HarnessClient, bot_guid: u64, dest: Dest) -> Result<(), NavError> {
    let mut repath_count = 0u32;
    // Tracks the bot's distance to `dest` at the end of each INCOMPLETE segment, so that the
    // next iteration can verify we got meaningfully closer.
    let mut last_dist_to_dest: Option<f64> = None;

    // Synthetic PathPoint for the final destination — used only for distance measurement.
    let dest_pt = PathPoint { x: dest.x, y: dest.y, z: dest.z };

    loop {
        let path = find_path(client, bot_guid, dest).await?;
        if path.is_no_path() {
            return Err(NavError::NoPath);
        }
        let final_wp = match path.points.last() {
            Some(p) => p.clone(),
            None => return Err(NavError::Shape("path has no waypoints".to_string())),
        };
        // Already there (find_path returned a degenerate ≤1-point path) → done.
        if path.points.len() < 2 {
            return Ok(());
        }

        let move_res = move_path(client, bot_guid, &path.points).await?;

        let wait_ms = move_res.duration_ms.clamp(0, MAX_WAIT_MS);
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms as u64)).await;

        // Poll for arrival at the segment's final waypoint.
        let mut arrival_pos: Option<PathPoint> = None;
        for _ in 0..MAX_ARRIVAL_CHECKS {
            let pos = get_position(client, bot_guid).await?;
            if dist(&pos, &final_wp) <= ARRIVAL_TOLERANCE {
                arrival_pos = Some(pos);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(RECHECK_INTERVAL_MS)).await;
        }
        let arrival_pos = match arrival_pos {
            Some(p) => p,
            None => return Err(NavError::Timeout),
        };

        if path.is_incomplete() {
            repath_count += 1;
            if repath_count > MAX_REPATH {
                return Err(NavError::RepathBudgetExceeded(repath_count));
            }

            // Progress guard: did we get meaningfully closer to the FINAL destination?
            // We reuse `arrival_pos` (already fetched) to avoid an extra `obs.get_position` call.
            let current_dist = dist(&arrival_pos, &dest_pt);
            if let Some(prev_dist) = last_dist_to_dest {
                if prev_dist - current_dist < STUCK_THRESHOLD {
                    return Err(NavError::Stuck(repath_count));
                }
            }
            last_dist_to_dest = Some(current_dist);

            continue;
        }
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path as AxumPath, http::StatusCode, routing::post, Json, Router};
    use serde_json::json;
    use std::time::Duration;
    use tot_harness_client::HarnessClient;

    /// Spawn a local axum mock that serves `/v1/tools/:name`.
    /// `tool_handler(name, args)` returns the PAYLOAD; the wrapper adds `{"ok":true,"result":...}`.
    async fn spawn_mock<F>(tool_handler: F) -> String
    where
        F: Fn(String, serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
    {
        let handler_arc = std::sync::Arc::new(tool_handler);
        let handler = move |AxumPath(name): AxumPath<String>,
                            Json(args): Json<serde_json::Value>| {
            let handler_arc = handler_arc.clone();
            async move {
                let payload = handler_arc(name, args);
                (StatusCode::OK, Json(json!({ "ok": true, "result": payload })))
            }
        };
        let app = Router::new().route("/v1/tools/:name", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn client(base: &str) -> HarnessClient {
        HarnessClient::new(base, "tok", Duration::from_secs(5))
    }

    #[tokio::test]
    async fn find_path_sends_correct_request() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(serde_json::Value::Null));
        let recv_clone = received.clone();
        let base = spawn_mock(move |_name, args| {
            *recv_clone.lock().unwrap() = args;
            json!({ "path_type": 1i64, "points": [ { "x": 10.0, "y": 20.0, "z": 30.0 } ] })
        }).await;

        let _ = find_path(&client(&base), 42, Dest { x: 10.0, y: 20.0, z: 30.0 }).await.unwrap();

        let body = received.lock().unwrap().clone();
        assert_eq!(body["bot_guid"], 42i64, "bot_guid must be sent as i64");
        assert_eq!(body["dest_x"], 10.0f64);
        assert_eq!(body["dest_y"], 20.0f64);
        assert_eq!(body["dest_z"], 30.0f64);
    }

    #[tokio::test]
    async fn find_path_parses_normal_path() {
        let base = spawn_mock(|_n, _a| json!({
            "path_type": 1i64,
            "points": [ {"x":0.0,"y":0.0,"z":0.0}, {"x":5.0,"y":0.0,"z":0.0}, {"x":10.0,"y":0.0,"z":0.0} ]
        })).await;
        let path = find_path(&client(&base), 1, Dest { x: 10.0, y: 0.0, z: 0.0 }).await.unwrap();
        assert_eq!(path.path_type, 1);
        assert!(path.is_normal());
        assert!(!path.is_incomplete());
        assert!(!path.is_no_path());
        assert_eq!(path.points.len(), 3);
        assert_eq!(path.points[0].x, 0.0);
        assert_eq!(path.points[2].x, 10.0);
    }

    #[tokio::test]
    async fn find_path_parses_incomplete_path() {
        let base = spawn_mock(|_n, _a| json!({
            "path_type": 4i64, "points": [ {"x":0.0,"y":0.0,"z":0.0}, {"x":3.0,"y":0.0,"z":0.0} ]
        })).await;
        let path = find_path(&client(&base), 2, Dest { x: 50.0, y: 0.0, z: 0.0 }).await.unwrap();
        assert_eq!(path.path_type, 4);
        assert!(!path.is_normal());
        assert!(path.is_incomplete());
        assert!(!path.is_no_path());
        assert_eq!(path.points.len(), 2);
    }

    #[tokio::test]
    async fn find_path_parses_nopath() {
        let base = spawn_mock(|_n, _a| json!({ "path_type": 8i64, "points": [] })).await;
        let path = find_path(&client(&base), 3, Dest { x: 9999.0, y: 9999.0, z: 9999.0 }).await.unwrap();
        assert_eq!(path.path_type, 8);
        assert!(path.is_no_path());
        assert!(!path.is_normal());
        assert!(!path.is_incomplete());
        assert!(path.points.is_empty());
    }

    #[tokio::test]
    async fn find_path_parses_not_using_path() {
        let base = spawn_mock(|_n, _a| json!({
            "path_type": 16i64, "points": [ {"x":0.0,"y":0.0,"z":0.0}, {"x":20.0,"y":5.0,"z":0.0} ]
        })).await;
        let path = find_path(&client(&base), 4, Dest { x: 20.0, y: 5.0, z: 0.0 }).await.unwrap();
        assert_eq!(path.path_type, 16);
        assert!(path.is_not_using_path());
        assert!(!path.is_normal());
        assert_eq!(path.points.len(), 2);
    }

    #[tokio::test]
    async fn find_path_wraps_harness_error() {
        let c = HarnessClient::new("http://127.0.0.1:1", "tok", Duration::from_secs(1));
        let err = find_path(&c, 5, Dest { x: 0.0, y: 0.0, z: 0.0 }).await.unwrap_err();
        match err { NavError::Harness(_) => {}, other => panic!("expected Harness, got {other:?}") }
    }

    #[tokio::test]
    async fn find_path_returns_shape_error_on_bad_result() {
        let base = spawn_mock(|_n, _a| json!({ "unexpected_key": "not a nav result" })).await;
        let err = find_path(&client(&base), 6, Dest { x: 0.0, y: 0.0, z: 0.0 }).await.unwrap_err();
        match err { NavError::Shape(_) => {}, other => panic!("expected Shape, got {other:?}") }
    }

    #[tokio::test]
    async fn find_path_handles_combined_flags() {
        let base = spawn_mock(|_n, _a| json!({ "path_type": 5i64, "points": [ {"x":1.0,"y":2.0,"z":3.0} ] })).await;
        let path = find_path(&client(&base), 7, Dest { x: 1.0, y: 2.0, z: 3.0 }).await.unwrap();
        assert!(path.is_normal());
        assert!(path.is_incomplete());
        assert_eq!(path.path_type, 5);
    }

    // ── move_path + walk_to helpers ───────────────────────────────────────────

    fn make_move_response(launched: bool, duration_ms: i64, x: f64, y: f64, z: f64) -> serde_json::Value {
        serde_json::json!({ "launched": launched, "duration_ms": duration_ms, "final": { "x": x, "y": y, "z": z } })
    }

    #[tokio::test]
    async fn move_path_sends_correct_request_shape() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(serde_json::Value::Null));
        let recv_clone = received.clone();
        let base = spawn_mock(move |name, args| {
            assert_eq!(name, "bot.move_path");
            *recv_clone.lock().unwrap() = args.clone();
            make_move_response(true, 0, 10.0, 20.0, 30.0)
        }).await;

        let pts = vec![PathPoint { x: 0.0, y: 0.0, z: 0.0 }, PathPoint { x: 10.0, y: 20.0, z: 30.0 }];
        let res = move_path(&client(&base), 99, &pts).await.unwrap();

        let body = received.lock().unwrap().clone();
        assert_eq!(body["bot_guid"], 99i64, "bot_guid must be i64");
        let pts_arr = body["points"].as_array().unwrap();
        assert_eq!(pts_arr.len(), 2);
        assert_eq!(pts_arr[0]["x"], 0.0f64);
        assert_eq!(pts_arr[1]["x"], 10.0f64);
        assert_eq!(pts_arr[1]["y"], 20.0f64);
        assert_eq!(pts_arr[1]["z"], 30.0f64);
        assert!(res.launched);
        assert_eq!(res.duration_ms, 0);
        assert_eq!(res.final_point.x, 10.0);
    }

    #[tokio::test]
    async fn move_path_parses_response() {
        let base = spawn_mock(|_n, _a| make_move_response(true, 1500, 5.0, 6.0, 7.0)).await;
        let pts = vec![PathPoint { x: 5.0, y: 6.0, z: 7.0 }];
        let res = move_path(&client(&base), 1, &pts).await.unwrap();
        assert!(res.launched);
        assert_eq!(res.duration_ms, 1500);
        assert_eq!(res.final_point.x, 5.0);
        assert_eq!(res.final_point.y, 6.0);
        assert_eq!(res.final_point.z, 7.0);
    }

    #[tokio::test]
    async fn walk_to_happy_path_normal() {
        let base = spawn_mock(|name, _args| match name.as_str() {
            "nav.find_path" => serde_json::json!({ "path_type": 1i64,
                "points": [{"x":0.0,"y":0.0,"z":0.0},{"x":10.0,"y":0.0,"z":0.0}] }),
            "bot.move_path" => make_move_response(true, 0, 10.0, 0.0, 0.0),
            "obs.get_position" => serde_json::json!({ "x":10.0,"y":0.0,"z":0.0,
                "map_id":0,"zone_id":1,"area_id":1,"orientation":0.0 }),
            other => panic!("unexpected tool: {other}"),
        }).await;
        let result = walk_to(&client(&base), 42, Dest { x: 10.0, y: 0.0, z: 0.0 }).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[tokio::test]
    async fn walk_to_no_path_returns_error() {
        let base = spawn_mock(|name, _args| match name.as_str() {
            "nav.find_path" => serde_json::json!({ "path_type": 8i64, "points": [] }),
            other => panic!("unexpected tool in no-path test: {other}"),
        }).await;
        let err = walk_to(&client(&base), 42, Dest { x: 9999.0, y: 9999.0, z: 9999.0 }).await.unwrap_err();
        match err { NavError::NoPath => {}, other => panic!("expected NoPath, got: {other:?}") }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn walk_to_timeout_when_position_never_arrives() {
        // Mock always returns the start position — bot never reaches final_wp (10,0,0).
        // walk_to polls MAX_ARRIVAL_CHECKS(5) times × RECHECK_INTERVAL_MS(500ms) = 2.5s real wait.
        // Multi-thread flavor keeps the mock server and the walk_to futures on separate threads
        // so the sleep futures don't block the server responses.
        let base = spawn_mock(|name, _args| match name.as_str() {
            "nav.find_path" => serde_json::json!({ "path_type": 1i64,
                "points": [{"x":0.0,"y":0.0,"z":0.0},{"x":10.0,"y":0.0,"z":0.0}] }),
            "bot.move_path" => make_move_response(true, 0, 10.0, 0.0, 0.0),
            "obs.get_position" => serde_json::json!({ "x":0.0,"y":0.0,"z":0.0,
                "map_id":0,"zone_id":1,"area_id":1,"orientation":0.0 }), // never moves
            other => panic!("unexpected tool in timeout test: {other}"),
        }).await;
        let err = walk_to(&client(&base), 42, Dest { x: 10.0, y: 0.0, z: 0.0 }).await.unwrap_err();
        match err { NavError::Timeout => {}, other => panic!("expected Timeout, got: {other:?}") }
    }

    #[tokio::test]
    async fn walk_to_incomplete_repath_succeeds() {
        // Iteration 1: find_path → INCOMPLETE [0→5]; move_path; arrival poll → (5,0,0) ✓.
        //              dist_to_dest=5.0 (dest=(10,0,0)); last_dist_to_dest=Some(5.0). Repath.
        // Iteration 2: find_path → NORMAL [5→10]; move_path; arrival poll → (10,0,0) ✓.
        //              path.is_incomplete()==false → Ok(()).
        //              No stuck check on NORMAL segment — progress guard is inside `if path.is_incomplete()`.
        //
        // Two separate monotonic counters: `fp` counts find_path calls (switches at 2nd),
        // `mp` counts move_path calls (determines final point).
        let fp = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)); // find_path call count
        let mp = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)); // move_path call count
        let fp2 = fp.clone();
        let mp2 = mp.clone();
        let base = spawn_mock(move |name, _args| {
            use std::sync::atomic::Ordering::SeqCst;
            match name.as_str() {
                "nav.find_path" => {
                    let n = fp2.fetch_add(1, SeqCst); // 0 on first call, 1 on second
                    if n == 0 {
                        // First call: INCOMPLETE, segment 0→5
                        serde_json::json!({ "path_type": 4i64, "points": [{"x":0.0,"y":0.0,"z":0.0},{"x":5.0,"y":0.0,"z":0.0}] })
                    } else {
                        // Second call: NORMAL, segment 5→10
                        serde_json::json!({ "path_type": 1i64, "points": [{"x":5.0,"y":0.0,"z":0.0},{"x":10.0,"y":0.0,"z":0.0}] })
                    }
                }
                "bot.move_path" => {
                    let n = mp2.fetch_add(1, SeqCst); // 0 on first call, 1 on second
                    if n == 0 { make_move_response(true, 0, 5.0, 0.0, 0.0) }
                    else { make_move_response(true, 0, 10.0, 0.0, 0.0) }
                }
                "obs.get_position" => {
                    // After iter-1 move_path (mp=1), position is at (5,0,0).
                    // After iter-2 move_path (mp=2), position is at (10,0,0).
                    let m = mp.load(SeqCst);
                    if m <= 1 {
                        serde_json::json!({"x":5.0,"y":0.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0})
                    } else {
                        serde_json::json!({"x":10.0,"y":0.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0})
                    }
                }
                other => panic!("unexpected tool: {other}"),
            }
        }).await;
        let result = walk_to(&client(&base), 42, Dest { x: 10.0, y: 0.0, z: 0.0 }).await;
        assert!(result.is_ok(), "expected Ok on INCOMPLETE→NORMAL path: {result:?}");
    }

    #[tokio::test]
    async fn walk_to_stuck_when_no_progress() {
        // Always INCOMPLETE; bot stays at ~(0.5,0,0); dest=(100,0,0).
        // Iter 1: arrival at (0.5,0,0); dist_to_dest≈99.5; last_dist_to_dest=Some(99.5). Repath.
        // Iter 2: arrival at (0.5,0,0); dist_to_dest≈99.5;
        //         prev_dist(99.5) - current_dist(99.5) = 0 < STUCK_THRESHOLD(1.0) → Stuck.
        let base = spawn_mock(|name, _args| match name.as_str() {
            "nav.find_path" => serde_json::json!({ "path_type": 4i64,
                "points": [{"x":0.0,"y":0.0,"z":0.0},{"x":1.0,"y":0.0,"z":0.0}] }),
            "bot.move_path" => make_move_response(true, 0, 1.0, 0.0, 0.0),
            "obs.get_position" => serde_json::json!({ "x":0.5,"y":0.0,"z":0.0,
                "map_id":0,"zone_id":1,"area_id":1,"orientation":0.0 }),
            other => panic!("unexpected tool: {other}"),
        }).await;
        let err = walk_to(&client(&base), 42, Dest { x: 100.0, y: 0.0, z: 0.0 }).await.unwrap_err();
        match err { NavError::Stuck(_) => {}, other => panic!("expected Stuck, got: {other:?}") }
    }
}
