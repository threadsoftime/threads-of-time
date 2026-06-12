//! QuestRunner exec slice (M3 Gate 3).
//!
//! Sequences one quest: walk to giver → accept → objectives (steps) → walk to receiver
//! → turnin.  Returns `GoalStatus`.
//!
//! # Giver GUID resolution (v1)
//!
//! After walking to the giver's position, exec calls `obs.get_nearby_hostiles` to find
//! the live packed-GUID for NPCs.  For GameObjects the entry-search mode of
//! `bot.interact_object` is used (object_entry + search_range); a dedicated
//! `obs.get_nearby_objects` tool is deferred to a future slice.
//!
//! # v1 limitations
//!
//! - Escort / vehicle quests: emit Blocked immediately (tag-based skip).
//! - `UseItem` with a non-zero target: not attempted v1 (adapter returns
//!   fail_code "targeted_phase2"); exec emits NeedsDecision.
//! - Kill / Collect steps: delegate to the grind scan/fight/loot machinery (via
//!   `scan_and_kill_until` below) using `obs.get_quest_log` as the stop predicate.

use std::time::{Duration, Instant};

use serde::Deserialize;
use tot_goal_contract::{
    BlockedReason, EscalationEvent, GiverKind, GoalStatus, QuestGiver, QuestGoal, QuestStep,
    WorldPos,
};
use tot_harness_client::HarnessClient;

use crate::{
    grind::{scan_for_target, GrindError},
    nav::{self, Dest, NavError},
};

// ── constants (exec timing — not goal fields) ────────────────────────────────

/// Walk-arrival threshold (yards): reserved for position-based arrival checks
/// in a future iteration; GoTo steps currently rely on `nav::walk_to` completion.
#[allow(dead_code)]
const ARRIVAL_YARDS: f64 = 8.0;

/// GUID-scan radius when looking for the quest giver/receiver nearby.
const GIVER_SCAN_RADIUS: f64 = 30.0;

/// How long to wait after accept/turnin for the server to ACK before polling
/// the quest log (1 s covers a 200 ms RTT + serverside tick budget).
const VERB_SETTLE_MS: u64 = if cfg!(test) { 10 } else { 1_000 };

/// Poll interval for objective progress checks.
const PROGRESS_POLL_MS: u64 = if cfg!(test) { 10 } else { 2_000 };

/// GrindGoal mob-filter defaults used when Kill/Collect steps delegate.
const GRIND_REST_THRESHOLD: f32 = 0.35;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Thin wrapper around `QuestGoal` fields needed by helpers that don't
/// want a full `GrindGoal`.  Lives only inside this module.
struct StepGrindParams {
    anchor:   WorldPos,
    radius:   f32,
    min_lvl:  u32,
    max_lvl:  u32,
}

/// Parse error type shared within this module.
#[derive(Debug)]
enum QuestError {
    Harness(tot_harness_client::HarnessError),
    Shape(String),
}

impl From<tot_harness_client::HarnessError> for QuestError {
    fn from(e: tot_harness_client::HarnessError) -> Self { QuestError::Harness(e) }
}

impl From<GrindError> for QuestError {
    fn from(e: GrindError) -> Self {
        match e {
            GrindError::Harness(h) => QuestError::Harness(h),
            GrindError::Shape(s)   => QuestError::Shape(s),
        }
    }
}

/// Resolve the live packed-GUID for an NPC giver by scanning nearby hostiles
/// and matching on creature entry.  Returns `None` if the NPC is not in range.
async fn resolve_npc_guid(
    client: &HarnessClient,
    bot_guid: u64,
    entry: u32,
) -> Result<Option<u64>, QuestError> {
    #[derive(Deserialize)]
    struct Hostile { guid: u64, entry: u32, is_alive: bool }
    #[derive(Deserialize)]
    struct NearbyHostiles { hostiles: Vec<Hostile> }

    let raw = client
        .call("obs.get_nearby_hostiles", serde_json::json!({
            "bot_guid": bot_guid as i64,
            "radius": GIVER_SCAN_RADIUS,
        }))
        .await?;
    let parsed: NearbyHostiles = serde_json::from_value(raw)
        .map_err(|e| QuestError::Shape(format!("resolve_npc_guid: {e}")))?;
    Ok(parsed.hostiles.into_iter()
        .find(|h| h.entry == entry && h.is_alive)
        .map(|h| h.guid))
}

/// Walk to `giver.pos`, then return the live GUID for the NPC / GO.
///
/// For NPCs: scans nearby hostiles and matches on entry.
/// For GameObjects: entry-search mode is handled directly by `bot.interact_object`
/// / `bot.accept_quest` — returns `None` to indicate "use entry-search, not guid".
async fn walk_to_and_resolve(
    client: &HarnessClient,
    bot_guid: u64,
    giver: &QuestGiver,
    step_deadline: Instant,
) -> Result<Option<u64>, QuestError> {
    // Walk to the giver position.
    let dest = Dest { x: giver.pos.x, y: giver.pos.y, z: giver.pos.z };
    if step_deadline.elapsed() > Duration::from_secs(0) {
        // Already over budget — skip the walk.
    } else {
        match nav::walk_to(client, bot_guid, dest).await {
            Ok(()) => {}
            Err(NavError::NoPath | NavError::Stuck(_) | NavError::RepathBudgetExceeded(_) | NavError::Timeout) => {
                // Navigation failed but we're somewhere near — still try.
            }
            Err(NavError::Harness(e)) => return Err(QuestError::Harness(e)),
            Err(NavError::Shape(s))   => return Err(QuestError::Shape(s)),
        }
    }

    match giver.kind {
        GiverKind::Npc => resolve_npc_guid(client, bot_guid, giver.entry).await,
        GiverKind::GameObject => Ok(None), // entry-search mode; no GUID needed
    }
}

// ── quest log helpers ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct QuestObjective {
    current: u32,
    required: u32,
}

#[derive(Debug, Deserialize)]
struct QuestLogEntry {
    quest_id: u32,
    #[serde(default)]
    #[allow(dead_code)]
    status: String,
    #[serde(default)]
    objectives: Vec<QuestObjective>,
}

#[derive(Debug, Deserialize)]
struct QuestLog {
    quests: Vec<QuestLogEntry>,
}

/// Fetch the bot's quest log and return the entry for `quest_id`, or `None`.
async fn get_quest_entry(
    client: &HarnessClient,
    bot_guid: u64,
    quest_id: u32,
) -> Result<Option<QuestLogEntry>, QuestError> {
    let raw = client
        .call("obs.get_quest_log", serde_json::json!({ "target_guid": bot_guid as i64 }))
        .await?;
    let log: QuestLog = serde_json::from_value(raw)
        .map_err(|e| QuestError::Shape(format!("get_quest_entry: {e}")))?;
    Ok(log.quests.into_iter().find(|q| q.quest_id == quest_id))
}

/// Returns the current progress of objective `obj_index`, or (0, 1) if absent.
async fn objective_progress(
    client: &HarnessClient,
    bot_guid: u64,
    quest_id: u32,
    obj_index: u8,
) -> Result<(u32, u32), QuestError> {
    if let Some(entry) = get_quest_entry(client, bot_guid, quest_id).await? {
        if let Some(obj) = entry.objectives.get(obj_index as usize) {
            return Ok((obj.current, obj.required));
        }
    }
    Ok((0, 1))
}

/// Returns `true` if `objectives[obj_index].current >= required`.
async fn objective_done(
    client: &HarnessClient,
    bot_guid: u64,
    quest_id: u32,
    obj_index: u8,
) -> Result<bool, QuestError> {
    let (cur, req) = objective_progress(client, bot_guid, quest_id, obj_index).await?;
    Ok(cur >= req)
}

// ── step executors ───────────────────────────────────────────────────────────

/// GoTo step: walk to `pos`.  Completion = arrival within ARRIVAL_YARDS.
async fn run_goto(
    client: &HarnessClient,
    bot_guid: u64,
    pos: WorldPos,
) -> Result<(), QuestError> {
    let dest = Dest { x: pos.x, y: pos.y, z: pos.z };
    match nav::walk_to(client, bot_guid, dest).await {
        Ok(()) => {}
        Err(NavError::NoPath | NavError::Stuck(_) | NavError::RepathBudgetExceeded(_) | NavError::Timeout) => {
            tracing::warn!(bot_guid, "quest goto: navigation warn — proceeding");
        }
        Err(NavError::Harness(e)) => return Err(QuestError::Harness(e)),
        Err(NavError::Shape(s))   => return Err(QuestError::Shape(s)),
    }
    Ok(())
}

/// Kill / Collect step: loop scan→fight→loot until `objectives[obj_index]` is satisfied
/// or the step deadline is exceeded.
///
/// Wraps the existing grind machinery: borrows `scan_for_target` and the fight/loot
/// path from `grind.rs`.  Does NOT inherit the grind state machine — uses the simpler
/// "scan one target, fight, loot, check predicate" loop.
async fn run_kill_or_collect(
    client: &HarnessClient,
    bot_guid: u64,
    quest_id: u32,
    params: StepGrindParams,
    obj_index: u8,
    deadline: Instant,
) -> Result<StepOutcome, QuestError> {
    use tot_goal_contract::{GrindGoal, MobFilter};

    // Construct a temporary GrindGoal that wraps the step parameters.
    let grind_goal = GrindGoal {
        anchor_point: params.anchor,
        wander_radius: params.radius * 2.0,
        max_search_radius: params.radius,
        mob_filter: MobFilter {
            min_level: params.min_lvl,
            max_level: params.max_lvl,
            creature_type: None,
        },
        to_level: 99,
        kill_count: None,
        rest_threshold: GRIND_REST_THRESHOLD,
        rotation_id: None,
        vendor: None,
    };

    // Build rotation (reuse grind machinery).
    let rotation = crate::rotations::build("auto_attack")
        .unwrap_or_else(|| {
            tracing::warn!("quest: no rotation for auto_attack; using default");
            crate::combat::RotationPlugin::melee_m1()
        });

    loop {
        if Instant::now() >= deadline {
            return Ok(StepOutcome::Timeout);
        }

        // Check objective before scanning (may already be done from a prev cycle).
        if objective_done(client, bot_guid, quest_id, obj_index).await? {
            return Ok(StepOutcome::Done);
        }

        match scan_for_target(client, bot_guid, &grind_goal).await? {
            None => {
                tokio::time::sleep(Duration::from_millis(PROGRESS_POLL_MS)).await;
                continue;
            }
            Some(target) => {
                // Navigate to target.
                let dest = Dest { x: target.x, y: target.y, z: target.z };
                match nav::walk_to(client, bot_guid, dest).await {
                    Ok(()) => {}
                    Err(NavError::Harness(e)) => return Err(QuestError::Harness(e)),
                    Err(NavError::Shape(s))   => return Err(QuestError::Shape(s)),
                    Err(_) => { continue; }  // stuck/no-path → try another target
                }
                // Fight.
                match crate::grind::fight(client, bot_guid, target, &grind_goal, &rotation).await {
                    Ok(crate::grind::FightOutcome::TargetDead) => {}
                    Ok(crate::grind::FightOutcome::BotDied) => {
                        return Ok(StepOutcome::BotDied);
                    }
                    Ok(crate::grind::FightOutcome::CastSpin { .. }) => {
                        // Cast-spin during quest combat — log and continue (not a quest abort).
                        tracing::warn!(bot_guid, "quest kill step: cast-spin detected; skipping target");
                        continue;
                    }
                    Err(GrindError::Harness(e)) => return Err(QuestError::Harness(e)),
                    Err(GrindError::Shape(s))   => return Err(QuestError::Shape(s)),
                }
                // Loot (best-effort; ignore loot errors).
                let _ = crate::loot::loot_nearest(client, bot_guid).await;
            }
        }

        // Re-check predicate.
        if objective_done(client, bot_guid, quest_id, obj_index).await? {
            return Ok(StepOutcome::Done);
        }
    }
}

/// UseItem step: call `bot.use_item` and poll objective.
async fn run_use_item(
    client: &HarnessClient,
    bot_guid: u64,
    quest_id: u32,
    item_entry: u32,
    obj_index: u8,
    deadline: Instant,
) -> Result<StepOutcome, QuestError> {
    let args = serde_json::json!({
        "bot_guid": bot_guid as i64,
        "item_entry": item_entry as i64,
    });
    loop {
        if Instant::now() >= deadline {
            return Ok(StepOutcome::Timeout);
        }
        if objective_done(client, bot_guid, quest_id, obj_index).await? {
            return Ok(StepOutcome::Done);
        }
        let result = client.call("bot.use_item", args.clone()).await;
        match result {
            Err(e) => {
                tracing::warn!(bot_guid, error = %e, "quest use_item: harness error");
                // Transient harness error — wait and retry.
            }
            Ok(v) => {
                // Check for v1 targeted-phase2 fail code.
                if let Some(code) = v.get("fail_code").and_then(|c| c.as_str()) {
                    if code == "targeted_phase2" {
                        tracing::warn!(bot_guid, "quest use_item: targeted_phase2 — v1 skip");
                        return Ok(StepOutcome::Unsupported);
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(VERB_SETTLE_MS)).await;
    }
}

/// InteractObject step: call `bot.interact_object` (entry-search mode) and poll objective.
async fn run_interact_object(
    client: &HarnessClient,
    bot_guid: u64,
    quest_id: u32,
    object_entry: u32,
    site: WorldPos,
    search_range: f32,
    obj_index: u8,
    count: u32,
    deadline: Instant,
) -> Result<StepOutcome, QuestError> {
    // Walk to the site first.
    let dest = Dest { x: site.x, y: site.y, z: site.z };
    match nav::walk_to(client, bot_guid, dest).await {
        Ok(()) | Err(NavError::NoPath | NavError::Stuck(_) | NavError::RepathBudgetExceeded(_) | NavError::Timeout) => {}
        Err(NavError::Harness(e)) => return Err(QuestError::Harness(e)),
        Err(NavError::Shape(s))   => return Err(QuestError::Shape(s)),
    }

    let args = serde_json::json!({
        "bot_guid": bot_guid as i64,
        "object_entry": object_entry as i64,
        "search_range": search_range as f64,
    });

    for _interaction_num in 0..count {
        loop {
            if Instant::now() >= deadline {
                return Ok(StepOutcome::Timeout);
            }
            if objective_done(client, bot_guid, quest_id, obj_index).await? {
                return Ok(StepOutcome::Done);
            }
            match client.call("bot.interact_object", args.clone()).await {
                Ok(_) => break,
                Err(e) => {
                    tracing::warn!(bot_guid, error = %e, "quest interact_object: harness error — retry");
                    tokio::time::sleep(Duration::from_millis(VERB_SETTLE_MS)).await;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(VERB_SETTLE_MS)).await;
    }

    // Final predicate check.
    if objective_done(client, bot_guid, quest_id, obj_index).await? {
        return Ok(StepOutcome::Done);
    }
    Ok(StepOutcome::Done)  // best-effort: all interact calls fired
}

// ── step outcome ─────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum StepOutcome {
    Done,
    /// Per-step or total time budget exceeded.
    Timeout,
    /// Bot died mid-step.
    BotDied,
    /// v1 skip (e.g. targeted use_item).
    Unsupported,
}

// ── entry point ───────────────────────────────────────────────────────────────

/// Drive a `Quest` goal to a terminal `GoalStatus`.
///
/// Sequence: walk to giver → accept → steps[0..N] → walk to receiver → turnin.
pub async fn run_quest(client: &HarnessClient, bot_guid: u64, goal: &QuestGoal) -> GoalStatus {
    let total_deadline = Instant::now() + Duration::from_secs(goal.total_timeout_s);

    // ── Phase 1: walk to giver + accept ──────────────────────────────────────
    let step_deadline = Instant::now() + Duration::from_secs(goal.step_timeout_s);
    let giver_guid = match walk_to_and_resolve(client, bot_guid, &goal.giver, step_deadline).await {
        Ok(g) => g,
        Err(QuestError::Harness(e)) =>
            return GoalStatus::Blocked { reason: BlockedReason::Other,
                                         detail: Some(format!("walk_to_giver harness: {e}")) },
        Err(QuestError::Shape(s)) =>
            return GoalStatus::Blocked { reason: BlockedReason::Other,
                                         detail: Some(format!("walk_to_giver shape: {s}")) },
    };

    if giver_guid.is_none() && matches!(goal.giver.kind, GiverKind::Npc) {
        tracing::warn!(bot_guid, quest_id = goal.quest_id,
            "quest accept: giver NPC not found in range; returning Blocked");
        return GoalStatus::Blocked {
            reason: BlockedReason::Other,
            detail: Some(format!("quest {}: giver entry {} not in range", goal.quest_id, goal.giver.entry)),
        };
    }

    let accept_args = {
        let mut a = serde_json::json!({
            "bot_guid": bot_guid as i64,
            "quest_id": goal.quest_id as i64,
        });
        // Provide quest_giver_guid if resolved (NPC); GOs use entry-search.
        if let Some(guid) = giver_guid {
            a["quest_giver_guid"] = serde_json::json!(guid as i64);
        } else {
            a["quest_giver_guid"] = serde_json::json!(goal.giver.entry as i64);
        }
        a
    };

    if let Err(e) = client.call("bot.accept_quest", accept_args).await {
        tracing::warn!(bot_guid, quest_id = goal.quest_id, error = %e, "quest accept failed");
        return GoalStatus::Blocked {
            reason: BlockedReason::Other,
            detail: Some(format!("quest {} accept harness: {e}", goal.quest_id)),
        };
    }
    tokio::time::sleep(Duration::from_millis(VERB_SETTLE_MS)).await;
    tracing::info!(bot_guid, quest_id = goal.quest_id, "quest_accepted");

    // ── Phase 2: steps ────────────────────────────────────────────────────────
    for (idx, step) in goal.steps.iter().enumerate() {
        if Instant::now() >= total_deadline {
            tracing::warn!(bot_guid, quest_id = goal.quest_id, "quest: total timeout at step {idx}");
            return GoalStatus::NeedsDecision {
                event: EscalationEvent::PathStuck { repath_attempts: 0, last_position: None },
            };
        }

        let step_deadline = Instant::now() + Duration::from_secs(goal.step_timeout_s);
        let outcome = execute_step(client, bot_guid, goal, step, step_deadline).await;
        match outcome {
            Ok(StepOutcome::Done) => {
                tracing::info!(bot_guid, quest_id = goal.quest_id, step_idx = idx, "quest_step_done");
            }
            Ok(StepOutcome::Timeout) => {
                tracing::warn!(bot_guid, quest_id = goal.quest_id, step_idx = idx,
                    "quest_step_timeout");
                return GoalStatus::Blocked {
                    reason: BlockedReason::Other,
                    detail: Some(format!(
                        "quest {} step {} timed out after {}s",
                        goal.quest_id, idx, goal.step_timeout_s
                    )),
                };
            }
            Ok(StepOutcome::BotDied) => {
                tracing::warn!(bot_guid, quest_id = goal.quest_id, step_idx = idx,
                    "quest_step_bot_died");
                return GoalStatus::NeedsDecision {
                    event: EscalationEvent::BotDied { position: None },
                };
            }
            Ok(StepOutcome::Unsupported) => {
                tracing::warn!(bot_guid, quest_id = goal.quest_id, step_idx = idx,
                    "quest_step_unsupported_v1");
                return GoalStatus::NeedsDecision {
                    event: EscalationEvent::PathStuck { repath_attempts: 0, last_position: None },
                };
            }
            Err(QuestError::Harness(e)) =>
                return GoalStatus::Blocked { reason: BlockedReason::Other,
                                             detail: Some(format!("step {idx} harness: {e}")) },
            Err(QuestError::Shape(s)) =>
                return GoalStatus::Blocked { reason: BlockedReason::Other,
                                             detail: Some(format!("step {idx} shape: {s}")) },
        }
    }

    // ── Phase 3: walk to receiver + turnin ────────────────────────────────────
    let step_deadline = Instant::now() + Duration::from_secs(goal.step_timeout_s);
    let receiver_guid = match walk_to_and_resolve(client, bot_guid, &goal.receiver, step_deadline).await {
        Ok(g) => g,
        Err(QuestError::Harness(e)) =>
            return GoalStatus::Blocked { reason: BlockedReason::Other,
                                         detail: Some(format!("walk_to_receiver harness: {e}")) },
        Err(QuestError::Shape(s)) =>
            return GoalStatus::Blocked { reason: BlockedReason::Other,
                                         detail: Some(format!("walk_to_receiver shape: {s}")) },
    };

    if receiver_guid.is_none() && matches!(goal.receiver.kind, GiverKind::Npc) {
        tracing::warn!(bot_guid, quest_id = goal.quest_id,
            "quest turnin: receiver NPC not found in range; returning Blocked");
        return GoalStatus::Blocked {
            reason: BlockedReason::Other,
            detail: Some(format!("quest {}: receiver entry {} not in range",
                                 goal.quest_id, goal.receiver.entry)),
        };
    }

    let turnin_args = {
        let mut a = serde_json::json!({
            "bot_guid": bot_guid as i64,
            "quest_id": goal.quest_id as i64,
        });
        if let Some(guid) = receiver_guid {
            a["quest_giver_guid"] = serde_json::json!(guid as i64);
        } else {
            a["quest_giver_guid"] = serde_json::json!(goal.receiver.entry as i64);
        }
        a
    };

    if let Err(e) = client.call("bot.turnin_quest", turnin_args).await {
        tracing::warn!(bot_guid, quest_id = goal.quest_id, error = %e, "quest turnin failed");
        return GoalStatus::Blocked {
            reason: BlockedReason::Other,
            detail: Some(format!("quest {} turnin harness: {e}", goal.quest_id)),
        };
    }
    tokio::time::sleep(Duration::from_millis(VERB_SETTLE_MS)).await;
    tracing::info!(bot_guid, quest_id = goal.quest_id, "quest_completed");

    GoalStatus::Completed {
        summary: format!("quest {} '{}' completed", goal.quest_id, goal.title),
    }
}

// ── step dispatch ─────────────────────────────────────────────────────────────

async fn execute_step(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &QuestGoal,
    step: &QuestStep,
    deadline: Instant,
) -> Result<StepOutcome, QuestError> {
    match step {
        QuestStep::GoTo { pos, .. } => {
            run_goto(client, bot_guid, *pos).await?;
            Ok(StepOutcome::Done)
        }
        QuestStep::Kill {
            site, obj_index, search_radius, mob_level_min, mob_level_max, ..
        } => {
            let params = StepGrindParams {
                anchor:  *site,
                radius:  *search_radius,
                min_lvl: *mob_level_min,
                max_lvl: *mob_level_max,
            };
            run_kill_or_collect(client, bot_guid, goal.quest_id, params, *obj_index, deadline).await
        }
        QuestStep::Collect {
            site, obj_index, search_radius, mob_level_min, mob_level_max, ..
        } => {
            let params = StepGrindParams {
                anchor:  *site,
                radius:  *search_radius,
                min_lvl: *mob_level_min,
                max_lvl: *mob_level_max,
            };
            run_kill_or_collect(client, bot_guid, goal.quest_id, params, *obj_index, deadline).await
        }
        QuestStep::UseItem { item_entry, obj_index } => {
            run_use_item(client, bot_guid, goal.quest_id, *item_entry, *obj_index, deadline).await
        }
        QuestStep::InteractObject { object_entry, site, search_range, obj_index, count } => {
            run_interact_object(
                client, bot_guid, goal.quest_id,
                *object_entry, *site, *search_range, *obj_index, *count,
                deadline,
            ).await
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path as AxumPath, http::StatusCode, routing::post, Json, Router};
    use serde_json::{json, Value};
    use std::{sync::{Arc, Mutex}, time::Duration};
    use tot_goal_contract::{GiverKind, QuestGiver, QuestGoal, QuestStep, WorldPos};
    use tot_harness_client::HarnessClient;

    async fn spawn_mock<F>(handler: F) -> String
    where F: Fn(String, Value) -> Value + Send + Sync + 'static {
        let h = Arc::new(handler);
        let route = move |AxumPath(name): AxumPath<String>, Json(args): Json<Value>| {
            let h = h.clone();
            async move { (StatusCode::OK, Json(json!({ "ok": true, "result": h(name, args) }))) }
        };
        let app = Router::new().route("/v1/tools/:name", post(route));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        format!("http://{addr}")
    }

    fn client(base: &str) -> HarnessClient {
        HarnessClient::new(base, "tok", Duration::from_secs(5))
    }

    fn sample_pos() -> WorldPos {
        WorldPos { map_id: 1, x: 0.0, y: 0.0, z: 0.0 }
    }

    fn npc_giver(entry: u32) -> QuestGiver {
        QuestGiver { kind: GiverKind::Npc, entry, pos: sample_pos() }
    }

    /// A minimal quest with zero steps (accept → turnin immediately).
    fn trivial_quest(quest_id: u32, giver_entry: u32) -> QuestGoal {
        QuestGoal {
            quest_id,
            title: "Test Quest".into(),
            giver: npc_giver(giver_entry),
            receiver: npc_giver(giver_entry),
            steps: vec![],
            step_timeout_s: 10,
            total_timeout_s: 60,
        }
    }

    /// Happy path: accept → no steps → turnin → Completed.
    #[tokio::test(flavor = "multi_thread")]
    async fn trivial_quest_completes_with_no_steps() {
        let accept_calls = Arc::new(Mutex::new(0u32));
        let turnin_calls = Arc::new(Mutex::new(0u32));
        let ac2 = accept_calls.clone();
        let tc2 = turnin_calls.clone();

        let base = spawn_mock(move |name, args| match name.as_str() {
            // giver walk (nav)
            "nav.find_path" => json!({
                "path_type": 1i64, "points": [{"x":0.0,"y":0.0,"z":0.0}]
            }),
            "bot.move_path" => json!({
                "launched": true, "duration_ms": 0, "final": {"x":0.0,"y":0.0,"z":0.0}
            }),
            "obs.get_position" => json!({ "x":0.0,"y":0.0,"z":0.0,"map_id":1,"zone_id":1,"area_id":1,"orientation":0.0 }),
            // GUID resolution
            "obs.get_nearby_hostiles" => json!({
                "hostiles": [{ "guid": 9999u64, "entry": 25816u32, "level": 70, "hp_pct": 100.0, "distance": 5.0, "is_alive": true, "x": 1.0, "y": 0.0, "z": 0.0 }]
            }),
            "bot.accept_quest" => { *ac2.lock().unwrap() += 1; json!({ "accepted": true }) },
            "obs.get_quest_log" => json!({ "quests": [] }),
            "bot.turnin_quest"  => { *tc2.lock().unwrap() += 1; json!({ "turned_in": true }) },
            other => panic!("unexpected: {other} args={args}"),
        }).await;

        let c = client(&base);
        let goal = trivial_quest(11797, 25816);
        let status = run_quest(&c, 1001, &goal).await;

        assert!(
            matches!(status, GoalStatus::Completed { .. }),
            "expected Completed, got {status:?}"
        );
        assert_eq!(*accept_calls.lock().unwrap(), 1, "accept should fire once");
        assert_eq!(*turnin_calls.lock().unwrap(), 1, "turnin should fire once");
    }

    /// Giver NPC not in range → Blocked.
    #[tokio::test(flavor = "multi_thread")]
    async fn giver_not_in_range_returns_blocked() {
        let base = spawn_mock(move |name, _args| match name.as_str() {
            "nav.find_path" => json!({ "path_type": 1i64, "points": [{"x":0.0,"y":0.0,"z":0.0}] }),
            "bot.move_path" => json!({ "launched": true, "duration_ms": 0, "final": {"x":0.0,"y":0.0,"z":0.0} }),
            "obs.get_position" => json!({ "x":0.0,"y":0.0,"z":0.0,"map_id":1,"zone_id":1,"area_id":1,"orientation":0.0 }),
            // Giver NOT in scan results (entry mismatch)
            "obs.get_nearby_hostiles" => json!({ "hostiles": [] }),
            other => panic!("unexpected: {other}"),
        }).await;

        let c = client(&base);
        let goal = trivial_quest(1, 99999);
        let status = run_quest(&c, 1002, &goal).await;

        assert!(
            matches!(status, GoalStatus::Blocked { reason: BlockedReason::Other, .. }),
            "expected Blocked, got {status:?}"
        );
    }

    /// UseItem step: mock returns targeted_phase2 fail_code → NeedsDecision.
    #[tokio::test(flavor = "multi_thread")]
    async fn use_item_targeted_phase2_returns_needs_decision() {
        let base = spawn_mock(move |name, _args| match name.as_str() {
            "nav.find_path" => json!({ "path_type": 1i64, "points": [{"x":0.0,"y":0.0,"z":0.0}] }),
            "bot.move_path" => json!({ "launched": true, "duration_ms": 0, "final": {"x":0.0,"y":0.0,"z":0.0} }),
            "obs.get_position" => json!({ "x":0.0,"y":0.0,"z":0.0,"map_id":1,"zone_id":1,"area_id":1,"orientation":0.0 }),
            "obs.get_nearby_hostiles" => json!({
                "hostiles": [{ "guid": 7777u64, "entry": 100u32, "level": 10, "hp_pct": 100.0, "distance": 3.0, "is_alive": true, "x": 1.0, "y": 0.0, "z": 0.0 }]
            }),
            "bot.accept_quest" => json!({ "accepted": true }),
            "obs.get_quest_log" => json!({ "quests": [{ "quest_id": 555u32, "status": "incomplete", "objectives": [{"current":0,"required":1}] }] }),
            // use_item returns targeted_phase2
            "bot.use_item" => json!({ "fail_code": "targeted_phase2" }),
            other => panic!("unexpected: {other}"),
        }).await;

        let c = client(&base);
        let goal = QuestGoal {
            quest_id: 555,
            title: "Item Quest".into(),
            giver: npc_giver(100),
            receiver: npc_giver(100),
            steps: vec![QuestStep::UseItem { item_entry: 99001, obj_index: 0 }],
            step_timeout_s: 5,
            total_timeout_s: 30,
        };
        let status = run_quest(&c, 1003, &goal).await;
        assert!(
            matches!(status, GoalStatus::NeedsDecision { .. }),
            "expected NeedsDecision for targeted_phase2, got {status:?}"
        );
    }

    /// InteractObject step: accept → interact until objective satisfied → turnin.
    #[tokio::test(flavor = "multi_thread")]
    async fn interact_object_step_completes() {
        let interact_calls = Arc::new(Mutex::new(0u32));
        let ic2 = interact_calls.clone();

        let base = spawn_mock(move |name, _args| match name.as_str() {
            "nav.find_path" => json!({ "path_type": 1i64, "points": [{"x":0.0,"y":0.0,"z":0.0}] }),
            "bot.move_path" => json!({ "launched": true, "duration_ms": 0, "final": {"x":0.0,"y":0.0,"z":0.0} }),
            "obs.get_position" => json!({ "x":0.0,"y":0.0,"z":0.0,"map_id":1,"zone_id":1,"area_id":1,"orientation":0.0 }),
            "obs.get_nearby_hostiles" => json!({
                "hostiles": [{ "guid": 5555u64, "entry": 200u32, "level": 20, "hp_pct": 100.0, "distance": 3.0, "is_alive": true, "x": 1.0, "y": 0.0, "z": 0.0 }]
            }),
            "bot.accept_quest" => json!({ "accepted": true }),
            "obs.get_quest_log" => {
                // After first interact call, return objective complete.
                json!({ "quests": [{ "quest_id": 777u32, "status": "complete", "objectives": [{"current":1,"required":1}] }] })
            },
            "bot.interact_object" => {
                *ic2.lock().unwrap() += 1;
                json!({ "interacted": true })
            },
            "bot.turnin_quest" => json!({ "turned_in": true }),
            other => panic!("unexpected: {other}"),
        }).await;

        let c = client(&base);
        let goal = QuestGoal {
            quest_id: 777,
            title: "Interact Quest".into(),
            giver: npc_giver(200),
            receiver: npc_giver(200),
            steps: vec![QuestStep::InteractObject {
                object_entry: 189188,
                site: sample_pos(),
                search_range: 20.0,
                obj_index: 0,
                count: 1,
            }],
            step_timeout_s: 10,
            total_timeout_s: 60,
        };
        let status = run_quest(&c, 1004, &goal).await;
        assert!(
            matches!(status, GoalStatus::Completed { .. }),
            "expected Completed, got {status:?}"
        );
    }

    /// GoTo step: walks to position and completes.
    #[tokio::test(flavor = "multi_thread")]
    async fn goto_step_completes() {
        let base = spawn_mock(move |name, _args| match name.as_str() {
            "nav.find_path" => json!({ "path_type": 1i64, "points": [{"x":0.0,"y":0.0,"z":0.0}] }),
            "bot.move_path" => json!({ "launched": true, "duration_ms": 0, "final": {"x":100.0,"y":100.0,"z":0.0} }),
            "obs.get_position" => json!({ "x":100.0,"y":100.0,"z":0.0,"map_id":1,"zone_id":1,"area_id":1,"orientation":0.0 }),
            "obs.get_nearby_hostiles" => json!({
                "hostiles": [{ "guid": 3333u64, "entry": 300u32, "level": 30, "hp_pct": 100.0, "distance": 3.0, "is_alive": true, "x": 1.0, "y": 0.0, "z": 0.0 }]
            }),
            "bot.accept_quest" => json!({ "accepted": true }),
            "obs.get_quest_log" => json!({ "quests": [] }),
            "bot.turnin_quest"  => json!({ "turned_in": true }),
            other => panic!("unexpected: {other}"),
        }).await;

        let c = client(&base);
        let goal = QuestGoal {
            quest_id: 888,
            title: "Go Quest".into(),
            giver: npc_giver(300),
            receiver: npc_giver(300),
            steps: vec![QuestStep::GoTo {
                pos: WorldPos { map_id: 1, x: 100.0, y: 100.0, z: 0.0 },
                label: Some("test_dest".into()),
            }],
            step_timeout_s: 10,
            total_timeout_s: 60,
        };
        let status = run_quest(&c, 1005, &goal).await;
        assert!(
            matches!(status, GoalStatus::Completed { .. }),
            "expected Completed for GoTo quest, got {status:?}"
        );
    }
}
