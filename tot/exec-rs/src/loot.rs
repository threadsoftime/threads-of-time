//! Loot — find nearest lootable corpse, navigate to it, loot it.
use serde::Deserialize;
use thiserror::Error;
use tot_harness_client::{HarnessClient, HarnessError};

use crate::nav::{self, Dest, NavError};

#[derive(Debug, Error)]
pub enum LootError {
    #[error("harness: {0}")]
    Harness(#[from] HarnessError),
    #[error("loot: unexpected response shape: {0}")]
    Shape(String),
}

/// A single lootable corpse as returned by `obs.get_lootable_corpses`.
#[derive(Debug, Clone, Deserialize)]
struct Corpse {
    guid: u64,
    distance: f64,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    z: f64,
}

#[derive(Debug, Deserialize)]
struct LootableCorpses {
    corpses: Vec<Corpse>,
}

/// The outcome of a loot attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct LootOutcome {
    pub items_taken: i64,
    pub gold_copper: i64,
}

impl LootOutcome {
    /// No corpse found / nav failed — non-fatal, returns zero values.
    pub fn none() -> Self {
        Self { items_taken: 0, gold_copper: 0 }
    }
}

/// Find the nearest lootable corpse, walk to it, and call `bot.loot`.
///
/// Returns `Ok(LootOutcome::none())` when:
/// - No lootable corpses are within range.
/// - Navigation to the corpse fails (non-fatal: skip the corpse).
///
/// Returns `Err(LootError::Harness)` only on network / harness-level failures.
pub async fn loot_nearest(
    client: &HarnessClient,
    bot_guid: u64,
) -> Result<LootOutcome, LootError> {
    // 1. Discover nearby lootable corpses.
    let raw = client
        .call(
            "obs.get_lootable_corpses",
            serde_json::json!({ "bot_guid": bot_guid as i64, "radius": 60.0f64 }),
        )
        .await?;

    let parsed: LootableCorpses =
        serde_json::from_value(raw)
            .map_err(|e| LootError::Shape(format!("lootable_corpses: {e}")))?;

    // 2. Pick the nearest corpse (sorted ascending by distance already, but we
    //    scan in case the server doesn't guarantee ordering).
    let nearest = match parsed
        .corpses
        .into_iter()
        .min_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal))
    {
        Some(c) => c,
        None => return Ok(LootOutcome::none()),
    };

    // 3. Navigate to the corpse position.
    match nav::walk_to(client, bot_guid, Dest { x: nearest.x, y: nearest.y, z: nearest.z })
        .await
    {
        Ok(()) => {}
        Err(NavError::NoPath)
        | Err(NavError::Stuck(_))
        | Err(NavError::RepathBudgetExceeded(_))
        | Err(NavError::Timeout) => {
            // Non-fatal: skip this corpse.
            return Ok(LootOutcome::none());
        }
        Err(NavError::Harness(e)) => return Err(LootError::Harness(e)),
        Err(NavError::Shape(s)) => return Err(LootError::Shape(s)),
    }

    // 4. Loot. target_guid is a packed creature u64 — send as bare u64.
    let loot_raw = client
        .call(
            "bot.loot",
            serde_json::json!({
                "bot_guid": bot_guid as i64,
                "target_guid": nearest.guid,
            }),
        )
        .await?;

    let items_taken = loot_raw["items_taken"].as_i64().unwrap_or(0);
    let gold_copper = loot_raw["gold_copper"].as_i64().unwrap_or(0);
    Ok(LootOutcome { items_taken, gold_copper })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path as AxumPath, http::StatusCode, routing::post, Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tot_harness_client::HarnessClient;

    async fn spawn_mock<F>(handler: F) -> String
    where
        F: Fn(String, serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
    {
        let h = Arc::new(handler);
        let route = move |AxumPath(name): AxumPath<String>, Json(args): Json<serde_json::Value>| {
            let h = h.clone();
            async move { (StatusCode::OK, Json(json!({ "ok": true, "result": h(name, args) }))) }
        };
        let app = Router::new().route("/v1/tools/:name", post(route));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn client(base: &str) -> HarnessClient {
        HarnessClient::new(base, "tok", Duration::from_secs(5))
    }

    /// Happy path: nearest corpse found, nav succeeds, loot succeeds.
    #[tokio::test]
    async fn loot_nearest_happy_path() {
        let loot_args = Arc::new(Mutex::new(serde_json::Value::Null));
        let loot_args2 = loot_args.clone();
        let corpse_guid: u64 = 0xF130000000000042u64; // packed creature GUID

        let base = spawn_mock(move |name, args| {
            match name.as_str() {
                "obs.get_lootable_corpses" => json!({
                    "corpses": [
                        {"guid": corpse_guid, "name": "Kobold Laborer", "distance": 4.0, "x": 10.0, "y": 5.0, "z": 0.0}
                    ]
                }),
                "nav.find_path" => json!({
                    "path_type": 1i64,
                    "points": [{"x":0.0,"y":0.0,"z":0.0},{"x":10.0,"y":5.0,"z":0.0}]
                }),
                "bot.move_path" => json!({
                    "launched": true, "duration_ms": 0, "final": {"x":10.0,"y":5.0,"z":0.0}
                }),
                "obs.get_position" => json!({
                    "x":10.0,"y":5.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0
                }),
                "bot.loot" => {
                    *loot_args2.lock().unwrap() = args.clone();
                    json!({"looted": true, "items_taken": 2, "items_failed": 0, "gold_copper": 150})
                }
                other => panic!("unexpected tool: {other}"),
            }
        })
        .await;

        let outcome = loot_nearest(&client(&base), 1003).await.unwrap();
        assert_eq!(outcome.items_taken, 2);
        assert_eq!(outcome.gold_copper, 150);

        // Verify bot.loot got the right args.
        let body = loot_args.lock().unwrap().clone();
        assert_eq!(body["bot_guid"], 1003i64);
        let tg = body["target_guid"].as_u64().expect("target_guid must be u64");
        assert_eq!(tg, corpse_guid, "target_guid must be the packed corpse guid");
    }

    /// No corpses within range → LootOutcome::none().
    #[tokio::test]
    async fn loot_nearest_no_corpses() {
        let base = spawn_mock(|name, _args| match name.as_str() {
            "obs.get_lootable_corpses" => json!({"corpses": []}),
            other => panic!("unexpected tool: {other}"),
        })
        .await;

        let outcome = loot_nearest(&client(&base), 1003).await.unwrap();
        assert_eq!(outcome, LootOutcome::none());
    }

    /// Nav failure (NoPath) → non-fatal, returns LootOutcome::none().
    #[tokio::test]
    async fn loot_nearest_nav_failure_returns_none() {
        let base = spawn_mock(|name, _args| match name.as_str() {
            "obs.get_lootable_corpses" => json!({
                "corpses": [{"guid": 999u64, "name": "Kobold", "distance": 8.0, "x": 8.0, "y": 0.0, "z": 0.0}]
            }),
            "nav.find_path" => json!({"path_type": 8i64, "points": []}), // PATHFIND_NOPATH
            other => panic!("unexpected tool: {other}"),
        })
        .await;

        let outcome = loot_nearest(&client(&base), 1003).await.unwrap();
        assert_eq!(outcome, LootOutcome::none(), "nav failure must be non-fatal");
    }

    /// Picks the nearest of multiple corpses.
    #[tokio::test]
    async fn loot_nearest_picks_closest_corpse() {
        let picked_guid = Arc::new(Mutex::new(0u64));
        let picked2 = picked_guid.clone();

        let base = spawn_mock(move |name, args| {
            match name.as_str() {
                "obs.get_lootable_corpses" => json!({
                    "corpses": [
                        {"guid": 100u64, "name": "Far", "distance": 20.0, "x": 20.0, "y": 0.0, "z": 0.0},
                        {"guid": 200u64, "name": "Near", "distance": 5.0, "x": 5.0, "y": 0.0, "z": 0.0},
                    ]
                }),
                "nav.find_path" => json!({
                    "path_type": 1i64,
                    "points": [{"x":0.0,"y":0.0,"z":0.0},{"x":5.0,"y":0.0,"z":0.0}]
                }),
                "bot.move_path" => json!({
                    "launched": true, "duration_ms": 0, "final": {"x":5.0,"y":0.0,"z":0.0}
                }),
                "obs.get_position" => json!({
                    "x":5.0,"y":0.0,"z":0.0,"map_id":0,"zone_id":1,"area_id":1,"orientation":0.0
                }),
                "bot.loot" => {
                    *picked2.lock().unwrap() = args["target_guid"].as_u64().unwrap_or(0);
                    json!({"looted": true, "items_taken": 1, "items_failed": 0, "gold_copper": 0})
                }
                other => panic!("unexpected tool: {other}"),
            }
        })
        .await;

        loot_nearest(&client(&base), 1003).await.unwrap();
        assert_eq!(*picked_guid.lock().unwrap(), 200u64, "must pick the nearest corpse (guid 200)");
    }
}
