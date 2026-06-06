//! Combat — the pluggable rotation seam (wRobot model). M1: a single melee AutoAttackAction.
use thiserror::Error;
use tot_harness_client::{HarnessClient, HarnessError};

#[derive(Debug, Error)]
pub enum CombatError {
    #[error("harness: {0}")]
    Harness(#[from] HarnessError),
    #[error("combat: unexpected response: {0}")]
    Shape(String),
}

/// Snapshot of combat state for action preconditions (grows in M2).
#[derive(Debug, Clone, Copy)]
pub struct CombatContext {
    pub bot_hp_pct: f32,
    pub target_hp_pct: f32,
    pub target_distance: f64,
}

/// A single combat action. M2 plugs per-spec abilities behind this trait.
pub trait RotationAction: Send + Sync {
    fn name(&self) -> &str;
    fn range(&self) -> f64;
    fn can_execute(&self, ctx: &CombatContext) -> bool;
    /// Fire the action via the harness. Returns Ok(true) if it fired.
    fn execute<'a>(
        &'a self,
        bot_guid: u64,
        target_guid: u64,
        h: &'a HarnessClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, CombatError>> + Send + 'a>>;
}

/// M1 melee: assert the attack each tick (core auto-swings; re-asserting is idempotent and
/// re-acquires after a target switch).
pub struct AutoAttackAction;

impl RotationAction for AutoAttackAction {
    fn name(&self) -> &str { "auto_attack" }
    fn range(&self) -> f64 { 5.0 }
    fn can_execute(&self, _ctx: &CombatContext) -> bool { true }

    fn execute<'a>(
        &'a self,
        bot_guid: u64,
        target_guid: u64,
        h: &'a HarnessClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, CombatError>> + Send + 'a>> {
        Box::pin(async move {
            // bot_guid is a low id (fits i64); target_guid is a PACKED creature uint64
            // (HighGuid bits set → exceeds i64) so it MUST be sent as a bare u64 JSON
            // number — the C++ adapter reads it via is_number_integer()+get<uint64_t>().
            h.call("bot.attack", serde_json::json!({
                "bot_guid": bot_guid as i64,
                "target_guid": target_guid,
            }))
            .await
            .map_err(CombatError::Harness)?;
            Ok(true)
        })
    }
}

/// An ordered rotation. M1 holds one action; M2 holds a per-spec priority list.
pub struct RotationPlugin {
    actions: Vec<Box<dyn RotationAction>>,
}

impl RotationPlugin {
    pub fn melee_m1() -> Self {
        Self { actions: vec![Box::new(AutoAttackAction)] }
    }

    /// Fire the highest-priority ready action.
    pub async fn tick(
        &self,
        bot_guid: u64,
        target_guid: u64,
        h: &HarnessClient,
        ctx: &CombatContext,
    ) -> Result<(), CombatError> {
        for a in &self.actions {
            if a.can_execute(ctx) {
                a.execute(bot_guid, target_guid, h).await?;
                return Ok(());
            }
        }
        Ok(())
    }
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

    fn ctx() -> CombatContext {
        CombatContext { bot_hp_pct: 100.0, target_hp_pct: 80.0, target_distance: 3.0 }
    }

    /// AutoAttackAction sends bot.attack with bot_guid as i64, target_guid as bare u64.
    #[tokio::test]
    async fn auto_attack_sends_correct_request_shape() {
        let captured = Arc::new(Mutex::new(serde_json::Value::Null));
        let cap2 = captured.clone();
        let base = spawn_mock(move |name, args| {
            assert_eq!(name, "bot.attack");
            *cap2.lock().unwrap() = args.clone();
            json!({"attacked": true, "target_guid": 12345678901234u64, "target_name": "Kobold"})
        })
        .await;

        // packed creature GUID: high bits set, exceeds i64::MAX
        let target_guid: u64 = 0xF130000000000001u64;
        let action = AutoAttackAction;
        let result = action.execute(1003, target_guid, &client(&base)).await;
        assert!(result.is_ok(), "execute returned err: {result:?}");
        assert_eq!(result.unwrap(), true);

        let body = captured.lock().unwrap().clone();
        assert_eq!(body["bot_guid"], 1003i64, "bot_guid must be i64");
        // target_guid must be sent as a bare u64 JSON number — serde_json stores large
        // u64 as Number which round-trips as u64.
        let tg = body["target_guid"].as_u64().expect("target_guid must be a u64 number");
        assert_eq!(tg, target_guid, "target_guid round-trip");
    }

    /// RotationPlugin::tick fires the first ready action (AutoAttackAction).
    #[tokio::test]
    async fn rotation_plugin_tick_fires_auto_attack() {
        let called = Arc::new(Mutex::new(false));
        let called2 = called.clone();
        let base = spawn_mock(move |name, _args| {
            assert_eq!(name, "bot.attack");
            *called2.lock().unwrap() = true;
            json!({"attacked": true, "target_guid": 111u64, "target_name": "Kobold"})
        })
        .await;

        let rotation = RotationPlugin::melee_m1();
        rotation.tick(1003, 111, &client(&base), &ctx()).await.unwrap();
        assert!(*called.lock().unwrap(), "bot.attack must have been called");
    }
}
