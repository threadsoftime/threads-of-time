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
}
