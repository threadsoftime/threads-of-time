//! Combat — the pluggable rotation seam (wRobot model). M2: typed cast outcomes + CastSpellAction.
use thiserror::Error;
use tot_harness_client::{HarnessClient, HarnessError};

#[derive(Debug, Error)]
pub enum CombatError {
    #[error("harness: {0}")]
    Harness(#[from] HarnessError),
    #[error("combat: unexpected response: {0}")]
    Shape(String),
}

/// Snapshot of combat state for action preconditions, refreshed each rotation tick (M2).
#[derive(Debug, Clone, Copy)]
pub struct CombatContext {
    pub bot_hp_pct: f32,
    /// Primary power (rage/energy/mana) as 0–100.
    pub bot_power_pct: f32,
    /// Mana as 0–100 for mana classes; None otherwise.
    pub bot_mana_pct: Option<f32>,
    pub combo_points: u8,
    pub target_hp_pct: f32,
    pub target_distance: f64,
}

/// Typed combat-state failure from bot.cast_spell (spec §1.1). Never parse messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCode {
    NotKnown, OnCooldown, NoPower, OutOfRange, NoLos, InvalidTarget, TargetDead, CastFailed,
}

impl FailCode {
    fn from_wire(s: &str) -> Self {
        match s {
            "not_known"      => Self::NotKnown,
            "on_cooldown"    => Self::OnCooldown,
            "no_power"       => Self::NoPower,
            "out_of_range"   => Self::OutOfRange,
            "no_los"         => Self::NoLos,
            "invalid_target" => Self::InvalidTarget,
            "target_dead"    => Self::TargetDead,
            _                => Self::CastFailed,
        }
    }
}

#[derive(Debug)]
pub enum ActionOutcome {
    /// Cast initiated; wait cast_time_ms before the next rotation tick.
    Casting { cast_time_ms: u64 },
    /// Melee auto-attack asserted (idempotent).
    Engaged,
    /// The verb ran but the cast did not start; engine falls through.
    Failed(FailCode),
}

pub trait RotationAction: Send + Sync {
    fn name(&self) -> &str;
    fn range(&self) -> f64;
    /// Rank-1 spell id for memo keying; None for non-spell actions.
    fn spell_id(&self) -> Option<u32> { None }
    fn can_execute(&self, ctx: &CombatContext) -> bool;
    fn execute<'a>(
        &'a self,
        bot_guid: u64,
        target_guid: u64,
        h: &'a HarnessClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ActionOutcome, CombatError>> + Send + 'a>>;
}

/// Per-fight scratch: abilities the server reported not_known (rank not yet learned).
/// Reset per fight — bots learn ranks as they level.
#[derive(Debug, Default)]
pub struct FightMemo {
    pub dropped: std::collections::HashSet<u32>,
}

#[derive(Debug)]
pub enum TickOutcome {
    /// An action fired (or melee asserted); wait_ms is the cast time (0 for instants/melee).
    Acted { wait_ms: u64 },
    /// The server says the target is gone/dead — end the fight.
    TargetGone,
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ActionOutcome, CombatError>> + Send + 'a>> {
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
            Ok(ActionOutcome::Engaged)
        })
    }
}

/// A castable ability: rank-1 spell id + a precondition over the combat context.
pub struct CastSpellAction {
    pub name: &'static str,
    pub spell_id: u32,
    pub range: f64,
    pub precondition: fn(&CombatContext) -> bool,
    /// Self-buffs omit target_guid on the wire.
    pub self_cast: bool,
}

#[derive(Debug, serde::Deserialize)]
struct CastResult {
    #[serde(default)] casting: bool,
    #[serde(default)] fail_code: Option<String>,
    #[serde(default)] cast_time_ms: Option<u64>,
}

impl RotationAction for CastSpellAction {
    fn name(&self) -> &str { self.name }
    fn range(&self) -> f64 { self.range }
    fn spell_id(&self) -> Option<u32> { Some(self.spell_id) }
    fn can_execute(&self, ctx: &CombatContext) -> bool { (self.precondition)(ctx) }

    fn execute<'a>(
        &'a self,
        bot_guid: u64,
        target_guid: u64,
        h: &'a HarnessClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ActionOutcome, CombatError>> + Send + 'a>> {
        Box::pin(async move {
            // bot_guid low id fits i64; target_guid is a PACKED uint64 (bare u64 on the
            // wire — see AutoAttackAction). Self-casts omit target_guid entirely.
            let args = if self.self_cast {
                serde_json::json!({ "bot_guid": bot_guid as i64, "spell_id": self.spell_id })
            } else {
                serde_json::json!({
                    "bot_guid": bot_guid as i64,
                    "spell_id": self.spell_id,
                    "target_guid": target_guid,
                })
            };
            let raw = h.call("bot.cast_spell", args).await.map_err(CombatError::Harness)?;
            let parsed: CastResult = serde_json::from_value(raw)
                .map_err(|e| CombatError::Shape(format!("bot.cast_spell: {e}")))?;
            if parsed.casting {
                Ok(ActionOutcome::Casting { cast_time_ms: parsed.cast_time_ms.unwrap_or(0) })
            } else {
                let code = parsed.fail_code.as_deref().map(FailCode::from_wire)
                    .unwrap_or(FailCode::CastFailed);
                Ok(ActionOutcome::Failed(code))
            }
        })
    }
}

/// An ordered rotation: priority actions + out-of-combat self-buffs + engage range.
pub struct RotationPlugin {
    actions: Vec<Box<dyn RotationAction>>,
    buffs: Vec<CastSpellAction>,
    engage_range: f64,
}

impl RotationPlugin {
    pub fn new(actions: Vec<Box<dyn RotationAction>>, buffs: Vec<CastSpellAction>, engage_range: f64) -> Self {
        Self { actions, buffs, engage_range }
    }

    pub fn melee_m1() -> Self {
        Self::new(vec![Box::new(AutoAttackAction)], vec![], 5.0)
    }

    pub fn engage_range(&self) -> f64 { self.engage_range }
    pub fn buffs(&self) -> &[CastSpellAction] { &self.buffs }

    /// Server-truth fallthrough: first action whose precondition passes AND whose
    /// cast starts wins the tick. Typed failures fall through; not_known memo-drops.
    pub async fn tick(
        &self,
        bot_guid: u64,
        target_guid: u64,
        h: &HarnessClient,
        ctx: &CombatContext,
        memo: &mut FightMemo,
    ) -> Result<TickOutcome, CombatError> {
        for a in &self.actions {
            if let Some(id) = a.spell_id() {
                if memo.dropped.contains(&id) { continue; }
            }
            if !a.can_execute(ctx) { continue; }
            match a.execute(bot_guid, target_guid, h).await? {
                ActionOutcome::Casting { cast_time_ms } => return Ok(TickOutcome::Acted { wait_ms: cast_time_ms }),
                ActionOutcome::Engaged => return Ok(TickOutcome::Acted { wait_ms: 0 }),
                ActionOutcome::Failed(FailCode::NotKnown) => {
                    if let Some(id) = a.spell_id() { memo.dropped.insert(id); }
                }
                ActionOutcome::Failed(FailCode::TargetDead) | ActionOutcome::Failed(FailCode::InvalidTarget) => {
                    return Ok(TickOutcome::TargetGone);
                }
                ActionOutcome::Failed(_) => {} // on_cooldown / no_power / out_of_range / no_los → next
            }
        }
        Ok(TickOutcome::Acted { wait_ms: 0 }) // nothing fired this tick (melee fallback makes this rare)
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
        CombatContext {
            bot_hp_pct: 100.0, bot_power_pct: 100.0, bot_mana_pct: None,
            combo_points: 0, target_hp_pct: 80.0, target_distance: 3.0,
        }
    }

    fn ctx2() -> CombatContext {
        CombatContext {
            bot_hp_pct: 100.0, bot_power_pct: 100.0, bot_mana_pct: Some(80.0),
            combo_points: 0, target_hp_pct: 95.0, target_distance: 3.0,
        }
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
        assert!(matches!(result.unwrap(), ActionOutcome::Engaged));

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
        let mut memo = FightMemo::default();
        rotation.tick(1003, 111, &client(&base), &ctx(), &mut memo).await.unwrap();
        assert!(*called.lock().unwrap(), "bot.attack must have been called");
    }

    /// CastSpellAction sends rank-1 spell_id + omits target_guid on self-cast.
    #[tokio::test]
    async fn cast_spell_self_cast_omits_target() {
        let captured = Arc::new(Mutex::new(serde_json::Value::Null));
        let cap2 = captured.clone();
        let base = spawn_mock(move |name, args| {
            assert_eq!(name, "bot.cast_spell");
            *cap2.lock().unwrap() = args.clone();
            json!({"casting": true, "spell_id": 6673u32, "cast_time_ms": 0})
        }).await;
        let a = CastSpellAction { name: "battle_shout", spell_id: 6673, range: 0.0,
                                  precondition: |_| true, self_cast: true };
        let out = a.execute(1003, 0xF130000000000001u64, &client(&base)).await.unwrap();
        assert!(matches!(out, ActionOutcome::Casting { cast_time_ms: 0 }));
        let body = captured.lock().unwrap().clone();
        assert_eq!(body["bot_guid"], 1003i64);
        assert_eq!(body["spell_id"], 6673u32);
        assert!(body.get("target_guid").is_none() || body["target_guid"].is_null(),
                "self-cast must omit target_guid");
    }

    /// fail_code maps to a typed FailCode (no message-string parsing).
    #[tokio::test]
    async fn cast_spell_fail_code_is_typed() {
        let base = spawn_mock(move |_n, _a| json!({"casting": false, "fail_code": "on_cooldown"})).await;
        let a = CastSpellAction { name: "fire_blast", spell_id: 2136, range: 20.0,
                                  precondition: |_| true, self_cast: false };
        let out = a.execute(1003, 0xF130000000000001u64, &client(&base)).await.unwrap();
        assert!(matches!(out, ActionOutcome::Failed(FailCode::OnCooldown)));
    }

    /// tick: on_cooldown/no_power fall through to the next action; not_known memo-drops.
    #[tokio::test]
    async fn tick_falls_through_and_memoizes_not_known() {
        let calls = Arc::new(Mutex::new(Vec::<(String, serde_json::Value)>::new()));
        let c2 = calls.clone();
        let base = spawn_mock(move |name, args| {
            c2.lock().unwrap().push((name.clone(), args.clone()));
            if name == "bot.cast_spell" {
                match args["spell_id"].as_u64().unwrap() {
                    772 => json!({"casting": false, "fail_code": "not_known"}),
                    78  => json!({"casting": false, "fail_code": "no_power"}),
                    _   => json!({"casting": true, "cast_time_ms": 0}),
                }
            } else {
                json!({"attacked": true, "target_guid": 1u64, "target_name": "Kobold"})
            }
        }).await;

        let plugin = RotationPlugin::new(vec![
            Box::new(CastSpellAction { name: "rend", spell_id: 772, range: 5.0,
                                       precondition: |_| true, self_cast: false }),
            Box::new(CastSpellAction { name: "heroic_strike", spell_id: 78, range: 5.0,
                                       precondition: |_| true, self_cast: false }),
            Box::new(AutoAttackAction),
        ], vec![], 5.0);
        let mut memo = FightMemo::default();

        // Tick 1: rend not_known (memo-drop) → heroic_strike no_power → auto_attack engages.
        let out = plugin.tick(1003, 0xF130000000000001u64, &client(&base), &ctx2(), &mut memo).await.unwrap();
        assert!(matches!(out, TickOutcome::Acted { .. }));
        assert!(memo.dropped.contains(&772));

        // Tick 2: rend must NOT be attempted again.
        calls.lock().unwrap().clear();
        plugin.tick(1003, 0xF130000000000001u64, &client(&base), &ctx2(), &mut memo).await.unwrap();
        let seen: Vec<u64> = calls.lock().unwrap().iter()
            .filter(|(n, _)| n == "bot.cast_spell")
            .map(|(_, a)| a["spell_id"].as_u64().unwrap()).collect();
        assert!(!seen.contains(&772), "not_known spell must be memo-dropped for the fight");
    }

    /// target_dead from the adapter ends the fight (TickOutcome::TargetGone).
    #[tokio::test]
    async fn tick_target_dead_ends_fight() {
        let base = spawn_mock(move |_n, _a| json!({"casting": false, "fail_code": "target_dead"})).await;
        let plugin = RotationPlugin::new(vec![
            Box::new(CastSpellAction { name: "smite", spell_id: 585, range: 30.0,
                                       precondition: |_| true, self_cast: false }),
        ], vec![], 25.0);
        let mut memo = FightMemo::default();
        let out = plugin.tick(1003, 1, &client(&base), &ctx2(), &mut memo).await.unwrap();
        assert!(matches!(out, TickOutcome::TargetGone));
    }
}
