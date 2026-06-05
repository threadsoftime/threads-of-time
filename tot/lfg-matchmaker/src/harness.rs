use crate::config::Dungeon;
use crate::types::{Faction, Intent, Role};
use serde_json::{json, Value};
use std::time::Duration;
use tot_harness_client::HarnessClient;
pub use tot_harness_client::HarnessError;

#[derive(Clone)]
pub struct Harness(HarnessClient);

impl Harness {
    pub fn new(base_url: String, bearer: String) -> Self {
        Harness(HarnessClient::new(base_url, bearer, Duration::from_secs(10)))
    }

    pub async fn call(&self, tool: &str, args: Value) -> Result<Value, HarnessError> {
        self.0.call(tool, args).await
    }

    pub async fn invite_to_group(&self, bot: u64, target: u64) -> Result<Value, HarnessError> {
        self.call("bot.invite_to_group", json!({ "bot_guid": bot, "target_guid": target })).await
    }

    pub async fn accept_invite(&self, bot: u64) -> Result<Value, HarnessError> {
        self.call("bot.accept_invite", json!({ "bot_guid": bot })).await
    }

    /// Remove `bot` from its group. The data-plane adapter routes a
    /// CMSG_GROUP_DISBAND, which the server handles as leave-or-disband (the
    /// leader leaving tears down the whole group). Used to roll back a partially
    /// formed group on a mid-formation failure.
    pub async fn leave_group(&self, bot: u64) -> Result<Value, HarnessError> {
        self.call("bot.leave_group", json!({ "bot_guid": bot })).await
    }

    pub async fn enter_instance_direct(&self, bot: u64, dungeon: &Dungeon) -> Result<Value, HarnessError> {
        self.call(
            "bot.enter_instance",
            json!({
                "bot_guid": bot,
                "mode": "direct",
                "map_id": dungeon.map_id,
                "x": dungeon.x,
                "y": dungeon.y,
                "z": dungeon.z,
                "orientation": dungeon.o,
            }),
        )
        .await
    }

    pub async fn get_group(&self, target: u64) -> Result<Value, HarnessError> {
        self.call("obs.get_group", json!({ "target_guid": target })).await
    }

    /// Force-form a server-side LFG 5-man group via `lfg.form_group`.
    /// `members` is a slice of `(guid, roles_bitmask)`. `dungeon_id` is the
    /// LFGDungeons.dbc id (e.g. 4 for RFC). Returns the raw result value
    /// (shaped: `{formed, map_id, placed}`).
    pub async fn form_group(
        &self,
        leader: u64,
        members: &[(u64, u32)],
        dungeon_id: u32,
    ) -> Result<Value, HarnessError> {
        let members_json: Vec<Value> = members
            .iter()
            .map(|(guid, roles)| json!({ "guid": guid, "roles": roles }))
            .collect();
        self.call(
            "lfg.form_group",
            json!({
                "leader_guid": leader,
                "members": members_json,
                "dungeon_id": dungeon_id,
            }),
        )
        .await
    }

    /// Poll `obs.lfg_pending` for up to `max` pending intents. Returns decoded
    /// `Intent` values. The adapter (Stage 3) clears the server-side queue on
    /// read — intents will NOT re-appear on the next call.
    ///
    /// NOTE: The `obs.lfg_pending` adapter ships in Stage 3. This method is
    /// tested against a mock here; it will not be live until Stage 3 deploys.
    pub async fn lfg_pending(&self, max: u32) -> Result<Vec<Intent>, HarnessError> {
        let result = self.call("obs.lfg_pending", json!({ "max": max })).await?;
        let arr = result
            .get("pending")
            .and_then(Value::as_array)
            .ok_or_else(|| HarnessError::Shape("obs.lfg_pending: missing pending array".into()))?;

        let mut intents = Vec::with_capacity(arr.len());
        for item in arr {
            let guid = item
                .get("guid")
                .and_then(Value::as_u64)
                .ok_or_else(|| HarnessError::Shape("lfg_pending item missing guid".into()))?;
            let team_id = item.get("team_id").and_then(Value::as_u64).unwrap_or(1) as u32;
            let roles_bits = item.get("roles").and_then(Value::as_u64).unwrap_or(8) as u32;
            let is_bot = item.get("is_bot").and_then(Value::as_bool).unwrap_or(true);

            let faction = Faction::from_team_id(team_id);
            let role = Role::from_lfg_bitmask(roles_bits).unwrap_or(Role::Dps);

            let dungeon_ids: Vec<u32> = item
                .get("dungeon_ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
                .unwrap_or_default();

            intents.push(Intent {
                guid,
                faction,
                role,
                dungeon_ids,
                is_real_player: !is_bot,
            });
        }
        Ok(intents)
    }

    /// Poll `lfg.cancel` for up to `max` pending cancellation intents. Returns
    /// the guids of players who cancelled their LFG queue entry (pressed Leave)
    /// since the last drain. Drain semantics: guids do NOT re-appear on the next
    /// call (same as `obs.lfg_pending`).
    ///
    /// NOTE: The `lfg.cancel` adapter ships in Inc 2 (C++ lane). This method is
    /// tested against a mock here; it will not be live until Inc 2 deploys.
    pub async fn lfg_cancel(&self, max: u32) -> Result<Vec<u64>, HarnessError> {
        let result = self.call("lfg.cancel", json!({ "max": max })).await?;
        let arr = result
            .get("cancelled")
            .and_then(Value::as_array)
            .ok_or_else(|| HarnessError::Shape("lfg.cancel: missing cancelled array".into()))?;
        let mut guids = Vec::with_capacity(arr.len());
        for item in arr {
            guids.push(
                item.as_u64()
                    .ok_or_else(|| HarnessError::Shape("lfg.cancel item is not a u64".into()))?,
            );
        }
        Ok(guids)
    }

    /// List the current bot population from `obs.list_bot_population`.
    /// Returns the raw result array (each element: `{bot_guid, level, map_id, name, …}`).
    pub async fn list_bot_population(&self) -> Result<Value, HarnessError> {
        self.call("obs.list_bot_population", json!({})).await
    }

    /// Observe a single player/bot state via `obs.get_state`.
    /// Returns the raw result value. Callers parse `result.self.race`,
    /// `result.social.in_group`, and `result.self.is_in_combat` directly.
    pub async fn get_state(&self, target: u64) -> Result<Value, HarnessError> {
        self.call("obs.get_state", json!({ "target_guid": target })).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, http::StatusCode, routing::post, Json, Router};

    // Mock harness:
    //   - "bot.fail422" → HTTP 422 with {ok:false,error,detail} (real-wire shape:
    //     adapter rejections come back as NON-2xx + typed envelope).
    //   - "bot.fail"    → HTTP 200 with {ok:false,error} (legacy 200-body case).
    //   - anything else → HTTP 200 {ok:true, result:{tool, args}} (echo).
    async fn spawn_mock() -> String {
        async fn handler(
            Path(name): Path<String>,
            Json(args): Json<serde_json::Value>,
        ) -> (StatusCode, Json<serde_json::Value>) {
            if name == "bot.fail422" {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({
                        "ok": false,
                        "error": "executor_failed",
                        "detail": "target is already in a group",
                    })),
                );
            }
            if name == "bot.fail" {
                return (StatusCode::OK, Json(serde_json::json!({ "ok": false, "error": "boom" })));
            }
            (StatusCode::OK, Json(serde_json::json!({ "ok": true, "result": { "tool": name, "args": args } })))
        }
        let app = Router::new().route("/v1/tools/:name", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn unwraps_ok_result() {
        let base = spawn_mock().await;
        let h = Harness::new(base, "tok".into());
        let res = h.invite_to_group(1, 2).await.unwrap();
        assert_eq!(res["tool"], "bot.invite_to_group");
        assert_eq!(res["args"]["bot_guid"], 1);
        assert_eq!(res["args"]["target_guid"], 2);
    }

    // T1 (C1): a 422 typed tool-failure MUST surface as HarnessError::Tool with
    // the human-readable `detail` preserved — not collapsed into HarnessError::Http.
    #[tokio::test]
    async fn surfaces_typed_tool_failure_on_422() {
        let base = spawn_mock().await;
        let h = Harness::new(base, "tok".into());
        let err = h.call("bot.fail422", serde_json::json!({})).await.unwrap_err();
        match err {
            HarnessError::Tool { tool, message } => {
                assert_eq!(tool, "bot.fail422");
                assert!(message.contains("already in a group"), "message lost detail: {message}");
            }
            other => panic!("expected Tool error, got {other:?}"),
        }
    }

    // Legacy shape: HTTP 200 with {ok:false} must also surface as a Tool error.
    #[tokio::test]
    async fn surfaces_tool_failure_on_200_ok_false() {
        let base = spawn_mock().await;
        let h = Harness::new(base, "tok".into());
        let err = h.call("bot.fail", serde_json::json!({})).await.unwrap_err();
        match err {
            HarnessError::Tool { tool, message } => {
                assert_eq!(tool, "bot.fail");
                assert_eq!(message, "boom");
            }
            other => panic!("expected Tool error, got {other:?}"),
        }
    }

    // T3: a transport-level failure (closed/unused port) surfaces as HarnessError::Http,
    // NOT a Tool error. Fast — no sleep, the connect refusal is immediate.
    #[tokio::test]
    async fn connection_error_surfaces_as_http() {
        let h = Harness::new("http://127.0.0.1:1".into(), "tok".into());
        let err = h.call("bot.invite_to_group", serde_json::json!({})).await.unwrap_err();
        match err {
            HarnessError::Http(_) => {}
            other => panic!("expected Http error, got {other:?}"),
        }
    }

    // T4: form_group sends the correct wire shape and returns the result on ok.
    #[tokio::test]
    async fn form_group_sends_correct_args() {
        let base = spawn_mock().await;
        let h = Harness::new(base, "tok".into());
        let members = [(10u64, 2u32), (20, 4), (30, 8), (31, 8)];
        let res = h.form_group(10, &members, 4).await.unwrap();
        assert_eq!(res["args"]["leader_guid"], 10);
        assert_eq!(res["args"]["dungeon_id"], 4);
        let m = res["args"]["members"].as_array().unwrap();
        assert_eq!(m.len(), 4);
        assert_eq!(m[0]["guid"], 10);
        assert_eq!(m[0]["roles"], 2);
    }

    // T5: lfg_pending decodes the mock's pending array into Intent values.
    // Covers: team_id→Faction, roles bitmask→Role, is_bot→is_real_player inversion.
    #[tokio::test]
    async fn lfg_pending_decodes_intents() {
        use crate::types::{Faction, Role};

        // Use a dedicated mock that returns a shaped lfg_pending result.
        async fn spawn_lfg_mock() -> String {
            let app = Router::new().route(
                "/v1/tools/:name",
                post(|Path(_): Path<String>, Json(_): Json<serde_json::Value>| async {
                    Json(serde_json::json!({
                        "ok": true,
                        "result": {
                            "pending": [
                                // Real Alliance tank (team_id=0, roles=2, is_bot=false)
                                { "guid": 101, "team_id": 0, "roles": 2, "dungeon_ids": [4], "comment": "", "is_bot": false },
                                // Bot Horde dps (team_id=1, roles=8, is_bot=true)
                                { "guid": 202, "team_id": 1, "roles": 8, "dungeon_ids": [4, 36], "comment": "", "is_bot": true },
                            ]
                        }
                    }))
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            format!("http://{addr}")
        }

        let base = spawn_lfg_mock().await;
        let h = Harness::new(base, "tok".into());
        let intents = h.lfg_pending(10).await.unwrap();
        assert_eq!(intents.len(), 2);

        let real = &intents[0];
        assert_eq!(real.guid, 101);
        assert_eq!(real.faction, Faction::Alliance);
        assert_eq!(real.role, Role::Tank);
        assert_eq!(real.dungeon_ids, vec![4]);
        assert!(real.is_real_player, "is_bot=false → is_real_player=true");

        let bot = &intents[1];
        assert_eq!(bot.guid, 202);
        assert_eq!(bot.faction, Faction::Horde);
        assert_eq!(bot.role, Role::Dps);
        assert_eq!(bot.dungeon_ids, vec![4, 36]);
        assert!(!bot.is_real_player, "is_bot=true → is_real_player=false");
    }

    // T6: lfg_pending returns a Shape error when the pending array is absent.
    #[tokio::test]
    async fn lfg_pending_shape_error_on_missing_array() {
        async fn spawn_bad_mock() -> String {
            let app = Router::new().route(
                "/v1/tools/:name",
                post(|Path(_): Path<String>, Json(_): Json<serde_json::Value>| async {
                    Json(serde_json::json!({ "ok": true, "result": { "not_pending": [] } }))
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            format!("http://{addr}")
        }

        let base = spawn_bad_mock().await;
        let h = Harness::new(base, "tok".into());
        let err = h.lfg_pending(5).await.unwrap_err();
        match err {
            HarnessError::Shape(msg) => assert!(msg.contains("pending"), "msg: {msg}"),
            other => panic!("expected Shape error, got {other:?}"),
        }
    }

    // T7: get_state passes target_guid and returns the result envelope.
    #[tokio::test]
    async fn get_state_passes_target_guid() {
        let base = spawn_mock().await;
        let h = Harness::new(base, "tok".into());
        let res = h.get_state(77).await.unwrap();
        assert_eq!(res["args"]["target_guid"], 77);
        assert_eq!(res["tool"], "obs.get_state");
    }

    // T8: list_bot_population posts obs.list_bot_population with no required args.
    #[tokio::test]
    async fn list_bot_population_calls_correct_tool() {
        let base = spawn_mock().await;
        let h = Harness::new(base, "tok".into());
        let res = h.list_bot_population().await.unwrap();
        assert_eq!(res["tool"], "obs.list_bot_population");
    }

    // T9: lfg_cancel decodes the cancelled guid array correctly.
    #[tokio::test]
    async fn lfg_cancel_decodes_guids() {
        async fn spawn_lfg_cancel_mock() -> String {
            let app = Router::new().route(
                "/v1/tools/:name",
                post(|Path(_): Path<String>, Json(_): Json<serde_json::Value>| async {
                    Json(serde_json::json!({
                        "ok": true,
                        "result": { "cancelled": [101u64, 202u64] }
                    }))
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            format!("http://{addr}")
        }

        let base = spawn_lfg_cancel_mock().await;
        let h = Harness::new(base, "tok".into());
        assert_eq!(h.lfg_cancel(10).await.unwrap(), vec![101u64, 202u64]);
    }

    // T10: lfg_cancel returns an empty Vec when no players have cancelled.
    #[tokio::test]
    async fn lfg_cancel_returns_empty_on_no_cancellations() {
        async fn spawn_lfg_cancel_empty_mock() -> String {
            let app = Router::new().route(
                "/v1/tools/:name",
                post(|Path(_): Path<String>, Json(_): Json<serde_json::Value>| async {
                    Json(serde_json::json!({
                        "ok": true,
                        "result": { "cancelled": [] }
                    }))
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            format!("http://{addr}")
        }

        let base = spawn_lfg_cancel_empty_mock().await;
        let h = Harness::new(base, "tok".into());
        let result = h.lfg_cancel(10).await.unwrap();
        assert!(result.is_empty());
    }

    // T11: lfg_cancel returns Shape error when the cancelled key is absent.
    #[tokio::test]
    async fn lfg_cancel_shape_error_on_missing_key() {
        async fn spawn_lfg_cancel_bad_mock() -> String {
            let app = Router::new().route(
                "/v1/tools/:name",
                post(|Path(_): Path<String>, Json(_): Json<serde_json::Value>| async {
                    Json(serde_json::json!({
                        "ok": true,
                        "result": { "not_cancelled": [] }
                    }))
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            format!("http://{addr}")
        }

        let base = spawn_lfg_cancel_bad_mock().await;
        let h = Harness::new(base, "tok".into());
        let err = h.lfg_cancel(5).await.unwrap_err();
        match err {
            HarnessError::Shape(msg) => assert!(msg.contains("cancelled"), "msg: {msg}"),
            other => panic!("expected Shape error, got {other:?}"),
        }
    }
}
