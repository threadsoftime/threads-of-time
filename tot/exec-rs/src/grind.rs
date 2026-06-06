//! The `Grind` goal disposer — a flat state machine (NOT a behavior tree).
//!
//! Plan 1 wires the nav/idle states fully and leaves combat/loot as tested no-ops.
//! Plan 2 replaces the no-op states with real implementations over the new
//! `bot.attack` / `obs.get_lootable_corpses` / `bot.loot` primitives.

use serde::Deserialize;
use thiserror::Error;
use tot_goal_contract::GrindGoal;
use tot_harness_client::{HarnessClient, HarnessError};

/// A selected hostile to engage (world-space position + distance from the bot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub guid: u64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub distance: f64,
}

/// Errors internal to grind execution (mapped to `GoalStatus` by the caller).
#[derive(Debug, Error)]
pub enum GrindError {
    #[error("harness: {0}")]
    Harness(#[from] HarnessError),
    #[error("unexpected response shape: {0}")]
    Shape(String),
}

#[derive(Debug, Deserialize)]
struct Hostile {
    guid: u64,
    level: u32,
    distance: f64,
    is_alive: bool,
    // World-space position so the executor can navigate to the target (design §6.2).
    // `default` keeps offline mocks that omit coords parseable (→ 0,0,0).
    #[serde(default)] x: f64,
    #[serde(default)] y: f64,
    #[serde(default)] z: f64,
}
#[derive(Debug, Deserialize)]
struct NearbyHostiles { hostiles: Vec<Hostile> }

/// Scan for the nearest alive hostile within the goal's level band.
/// Calls `obs.get_nearby_hostiles` (design §6.2).
pub async fn scan_for_target(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &GrindGoal,
) -> Result<Option<Target>, GrindError> {
    let raw = client
        .call("obs.get_nearby_hostiles", serde_json::json!({
            "bot_guid": bot_guid as i64,
            "radius": goal.max_search_radius as f64,
        }))
        .await?;
    let parsed: NearbyHostiles =
        serde_json::from_value(raw).map_err(|e| GrindError::Shape(format!("nearby_hostiles: {e}")))?;

    let mut best: Option<Target> = None;
    for h in parsed.hostiles {
        if !h.is_alive { continue; }
        if h.level < goal.mob_filter.min_level || h.level > goal.mob_filter.max_level { continue; }
        let cand = Target { guid: h.guid, x: h.x, y: h.y, z: h.z, distance: h.distance };
        match best {
            Some(b) if b.distance <= cand.distance => {}
            _ => best = Some(cand),
        }
    }
    Ok(best)
}

/// The grind state machine. Plan 1 implements `Scanning`/`Reacting`/`Approaching`/
/// `Wandering`/`Idle` for real; `Fighting`/`PostKillPause`/`Looting`/`HealthCheck`/
/// `Resting` are tested no-ops until Plan 2.
#[derive(Debug, Clone, PartialEq)]
pub enum GrindState {
    Scanning,
    Reacting { target: Target },
    Approaching { target: Target },
    Fighting { target: Target },
    PostKillPause { target: Target },
    Looting { target: Target },
    HealthCheck,
    Resting,
    Wandering,
    Idle,
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path as AxumPath, http::StatusCode, routing::post, Json, Router};
    use serde_json::json;
    use std::time::Duration;
    use tot_goal_contract::{GrindGoal, MobFilter, WorldPos};
    use tot_harness_client::HarnessClient;

    async fn spawn_mock<F>(handler: F) -> String
    where F: Fn(String, serde_json::Value) -> serde_json::Value + Send + Sync + 'static {
        let h = std::sync::Arc::new(handler);
        let route = move |AxumPath(name): AxumPath<String>, Json(args): Json<serde_json::Value>| {
            let h = h.clone();
            async move { (StatusCode::OK, Json(json!({ "ok": true, "result": h(name, args) }))) }
        };
        let app = Router::new().route("/v1/tools/:name", post(route));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        format!("http://{addr}")
    }
    fn client(base: &str) -> HarnessClient { HarnessClient::new(base, "tok", Duration::from_secs(5)) }

    pub(super) fn test_goal() -> GrindGoal {
        GrindGoal {
            anchor_point: WorldPos { map_id: 0, x: 0.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: None },
            to_level: 6, kill_count: Some(1), rest_threshold: 0.35,
        }
    }

    #[tokio::test]
    async fn scanning_returns_nearest_in_band_target() {
        let base = spawn_mock(|name, _a| match name.as_str() {
            "obs.get_nearby_hostiles" => json!({"hostiles": [
                {"guid": 111u64, "name": "Kobold Laborer", "level": 5, "hp_pct": 100, "distance": 8.0, "is_alive": true, "x": 100.0, "y": 200.0, "z": 5.0},
                {"guid": 222u64, "name": "Murloc", "level": 12, "hp_pct": 100, "distance": 4.0, "is_alive": true, "x": 50.0, "y": 60.0, "z": 1.0}
            ]}),
            other => panic!("unexpected tool {other}"),
        }).await;
        let g = test_goal();
        let t = scan_for_target(&client(&base), 1003, &g).await.unwrap();
        // Murloc is closer but out of the 4-7 level band; Kobold (L5) is the pick.
        assert_eq!(t, Some(Target { guid: 111, x: 100.0, y: 200.0, z: 5.0, distance: 8.0 }));
    }

    #[tokio::test]
    async fn scanning_returns_none_when_no_in_band_target() {
        let base = spawn_mock(|_n, _a| json!({"hostiles": [
            {"guid": 222u64, "name": "Murloc", "level": 12, "hp_pct": 100, "distance": 4.0, "is_alive": true}
        ]})).await;
        let t = scan_for_target(&client(&base), 1003, &test_goal()).await.unwrap();
        assert_eq!(t, None);
    }
}
