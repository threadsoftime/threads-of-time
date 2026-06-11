//! Economy interrupt (M2 slice 2.4): bags-full / grey-threshold triggers and the
//! vendor trip (nav → sell greys → repair → nav back). Spec:
//! docs/superpowers/specs/2026-06-10-m2-slice-2.4-economy-exec-flows-design.md
//! (azerothcore-heimdal master). Failures are never terminal (spec §5).

use serde::Deserialize;

use crate::grind::GrindError;
use tot_harness_client::HarnessClient;

/// Trigger thresholds + pacing (exec constants, not goal fields — spec §3).
pub(crate) const FREE_SLOT_TRIGGER: u32 = 3;
pub(crate) const GREY_COUNT_TRIGGER: u32 = 8;
pub(crate) const ECONOMY_CHECK_KILL_STRIDE: u32 = 5;
pub(crate) const VENDOR_TRIP_COOLDOWN_S: u64 = 600;

/// Backpack proper (INVENTORY_SLOT_ITEM_START..END) is 16 slots.
const BACKPACK_SLOTS: u32 = 16;

#[derive(Debug, Deserialize)]
struct InvItem {
    #[serde(default)]
    quality: u32,
}
#[derive(Debug, Deserialize)]
struct NestedBag {
    capacity: u32,
    #[serde(default)]
    contents: Vec<InvItem>,
}
#[derive(Debug, Deserialize)]
struct Inventory {
    #[serde(default)]
    bags: Vec<InvItem>,
    #[serde(default)]
    nested_bags: Vec<NestedBag>,
}

/// What the trigger check needs from `obs.get_inventory` (spec §3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BagSummary {
    pub free_slots: u32,
    pub grey_count: u32,
}

impl BagSummary {
    pub(crate) fn triggered(&self) -> bool {
        self.free_slots <= FREE_SLOT_TRIGGER || self.grey_count >= GREY_COUNT_TRIGGER
    }
}

/// Pure derivation per spec §3: free = (16 − backpack items) + Σ(bag capacity − bag
/// items); greys = quality==0 across backpack + nested bags. Equipped NEVER counts.
pub(crate) fn summarize_inventory(raw: serde_json::Value) -> Result<BagSummary, GrindError> {
    let inv: Inventory = serde_json::from_value(raw)
        .map_err(|e| GrindError::Shape(format!("obs.get_inventory: {e}")))?;
    let mut free = BACKPACK_SLOTS.saturating_sub(inv.bags.len() as u32);
    let mut greys = inv.bags.iter().filter(|i| i.quality == 0).count() as u32;
    for b in &inv.nested_bags {
        free = free.saturating_add(b.capacity.saturating_sub(b.contents.len() as u32));
        greys = greys.saturating_add(b.contents.iter().filter(|i| i.quality == 0).count() as u32);
    }
    Ok(BagSummary { free_slots: free, grey_count: greys })
}

/// One `obs.get_inventory` round-trip → `BagSummary`.
pub(crate) async fn read_bags(client: &HarnessClient, bot_guid: u64) -> Result<BagSummary, GrindError> {
    let raw = client
        .call("obs.get_inventory", serde_json::json!({ "target_guid": bot_guid as i64 }))
        .await?;
    summarize_inventory(raw)
}

/// Entry/recovery-time economy check (spec rounds 4/4b): poll bags and return the
/// trip inputs when a trigger condition holds, the goal has a vendor, and the trip
/// cooldown is clear. Unlike `economy_due` there is no kill stride — bag state
/// carries across goal lives and recoveries. A failed poll warns and returns None.
pub(crate) async fn entry_check(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &tot_goal_contract::GrindGoal,
    last_trip: Option<std::time::Instant>,
    cooldown: std::time::Duration,
) -> Option<(BagSummary, tot_goal_contract::VendorInfo)> {
    let vendor = goal.vendor?;
    if let Some(t) = last_trip {
        if t.elapsed() < cooldown {
            return None;
        }
    }
    match read_bags(client, bot_guid).await {
        Ok(bags) if bags.triggered() => Some((bags, vendor)),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!(bot_guid, error = %e, "entry economy check failed — skipping");
            None
        }
    }
}

/// Gate the economy check (spec §3): vendor present → at most every
/// `ECONOMY_CHECK_KILL_STRIDE` kills → outside the trip cooldown → poll bags →
/// `Some(summary)` when a trigger condition holds. A failed poll logs and skips
/// (never terminal). `cooldown` is injected so tests don't wait 600 s.
pub(crate) async fn economy_due(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &tot_goal_contract::GrindGoal,
    kills: u32,
    kills_at_last_check: &mut u32,
    last_trip: Option<std::time::Instant>,
    cooldown: std::time::Duration,
) -> Option<BagSummary> {
    goal.vendor.as_ref()?;
    if kills < kills_at_last_check.saturating_add(ECONOMY_CHECK_KILL_STRIDE) {
        return None;
    }
    if let Some(t) = last_trip {
        if t.elapsed() < cooldown {
            return None;
        }
    }
    *kills_at_last_check = kills;
    match read_bags(client, bot_guid).await {
        Ok(s) if s.triggered() => Some(s),
        Ok(_) => None,
        Err(e) => {
            tracing::warn!(bot_guid, error = %e, "economy check failed — skipping");
            None
        }
    }
}

/// How a vendor trip ended. Both are non-terminal for the grind goal (spec §5).
#[derive(Debug, PartialEq)]
pub(crate) enum VendorTripOutcome {
    /// Trip ran (possibly with soft fails) — resume Scanning under the cooldown.
    Done,
    /// Bot found dead at a checkpoint — caller enters Recovering.
    BotDead,
}

#[derive(Debug, Deserialize)]
struct SellResult {
    #[serde(default)]
    sold_count: u32,
    #[serde(default)]
    copper_gained: u64,
    #[serde(default)]
    fail_code: Option<String>,
}
#[derive(Debug, Deserialize)]
struct RepairResult {
    #[serde(default)]
    copper_spent: u64,
    #[serde(default)]
    fail_code: Option<String>,
}

/// Dead-check between trip phases. Read errors count as alive — a transient obs
/// failure must not bounce the bot into death recovery.
async fn bot_is_dead(client: &HarnessClient, bot_guid: u64) -> bool {
    matches!(crate::grind::read_self(client, bot_guid).await, Ok(s) if s.hp_pct == 0)
}

/// Hop length for long vendor legs (yd). Two live-verified bounds (2026-06-10):
/// a single nav.find_path NOPATHs beyond ~200 yd, and walk_to's arrival budget
/// (duration_ms clamped to 30 s + 5×500 ms polls) times out hops the bot can't
/// cover in ~32 s — at bot WALK speed (~2.5 yd/s) that caps a hop at ~70 yd.
const WALK_FAR_HOP_YD: f64 = 60.0;
/// Hop budget: 16 hops ≈ 960 yd ceiling — beyond the longest mined vendor leg (712 yd).
const WALK_FAR_MAX_HOPS: u32 = 16;
/// Per-leg fight budget (live-verify round 3, 2026-06-11): an externally-owned bot
/// won't defend itself against an en-route aggro. Capped at 6 fights so a pathological
/// respawn zone can't hold the bot forever.
const WALK_FAR_MAX_FIGHTS: u32 = 6;
/// Aggro scan radius for the fight-through path: only the attacker that stopped the
/// spline should be within ~15 yd; scanning wider risks pulling new packs.
const WALK_FAR_AGGRO_YD: f64 = 15.0;

/// Walk a long leg in ≤WALK_FAR_HOP_YD segments: re-read the bot position each
/// hop (obs.get_position), aim at the straight-line interpolation toward `dest`,
/// and walk_to it. Hop targets snap to the navmesh server-side (FARFROMPOLY is
/// tolerated).
///
/// Fight-through (live-verify round 3): a Timeout or Stuck result on a hop is the
/// signature of combat interrupting the movement spline. An externally-owned bot
/// won't defend itself, so this function scans for the attacker and fights it with
/// the goal's rotation before retrying the hop. The per-leg fight budget is
/// WALK_FAR_MAX_FIGHTS; budget exhaustion or a Timeout/Stuck with no attacker in
/// range returns the original NavError so the caller's soft-failure policy applies.
/// Other NavErrors (NoPath, Harness, Shape, RepathBudgetExceeded) propagate immediately.
///
/// NOTE: vendor legs don't loot after fights — the bags are full (that's why we're
/// traveling), and looting here would add RPC cost and delay. Deaths during fight-
/// through are caught by the trip's dead-check checkpoints upstream.
pub(crate) async fn walk_far(
    client: &HarnessClient,
    bot_guid: u64,
    dest: crate::nav::Dest,
    goal: &tot_goal_contract::GrindGoal,
    rotation: &crate::combat::RotationPlugin,
) -> Result<(), crate::nav::NavError> {
    use crate::nav::{self, NavError};
    let mut hops: u32 = 0;
    let mut fights: u32 = 0;
    while hops < WALK_FAR_MAX_HOPS {
        let (bx, by, bz) = crate::grind::bot_world_pos(client, bot_guid)
            .await
            .map_err(|e| match e {
                crate::grind::GrindError::Harness(h) => NavError::Harness(h),
                crate::grind::GrindError::Shape(s) => NavError::Shape(s),
            })?;
        let (dx, dy, dz) = (dest.x - bx, dest.y - by, dest.z - bz);
        // 2-D horizontal distance (z ignored for hop sizing, consistent with
        // the navmesh server treating Z as terrain-snapped).
        let dist = (dx * dx + dy * dy).sqrt();
        let hop_result = if dist <= WALK_FAR_HOP_YD {
            // Final approach — use the exact dest. A Timeout here may also be
            // combat; fall through to the combat-interruption handler below.
            match nav::walk_to(client, bot_guid, dest).await {
                Ok(()) => return Ok(()),
                Err(e) => Err(e),
            }
        } else {
            let frac = WALK_FAR_HOP_YD / dist;
            nav::walk_to(client, bot_guid, crate::nav::Dest {
                x: bx + dx * frac,
                y: by + dy * frac,
                z: bz + dz * frac,
            }).await
        };
        match hop_result {
            Ok(()) => { hops += 1; }
            Err(e @ (NavError::Timeout | NavError::Stuck(_))) => {
                // Likely combat interruption (live-verified round 3): an attacker stops
                // the spline and an externally-owned bot won't defend itself. Fight back
                // with the goal's rotation, then retry the hop (re-reading position —
                // the fight may have moved the bot).
                fights += 1;
                if fights > WALK_FAR_MAX_FIGHTS {
                    return Err(e);
                }
                match crate::grind::scan_for_target(client, bot_guid, goal).await {
                    Ok(Some(t)) if t.distance <= WALK_FAR_AGGRO_YD => {
                        tracing::info!(bot_guid, target = t.guid,
                            "walk_far: fighting through en-route aggro");
                        // Outcome intentionally ignored: a death is caught by the trip's
                        // dead-check checkpoints; a fight error is soft (leg retries).
                        let _ = crate::grind::fight(client, bot_guid, t, goal, rotation).await;
                    }
                    _ => return Err(e), // no attacker in range — genuine nav failure
                }
            }
            Err(e) => return Err(e),
        }
    }
    // Hop budget exhausted without reaching the final approach — treat as stuck.
    // NavError::Stuck carries the repath count; we pass the hop budget as proxy.
    Err(crate::nav::NavError::Stuck(WALK_FAR_MAX_HOPS))
}

/// The trip (spec §4): walk to the vendor → sell greys → repair (if able) →
/// walk back to the anchor. Every failure is soft (spec §5); death at any
/// checkpoint aborts into the caller's Recovering path.
///
/// `rotation` is forwarded to `walk_far` for the fight-through path (live-verify
/// round 3): if the bot aggros a mob en route, it fights with the goal's rotation.
pub(crate) async fn run_vendor_trip(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &tot_goal_contract::GrindGoal,
    vendor: &tot_goal_contract::VendorInfo,
    bags: BagSummary,
    rotation: &crate::combat::RotationPlugin,
) -> VendorTripOutcome {
    let trigger = if bags.free_slots <= FREE_SLOT_TRIGGER { "free_slots" } else { "grey_count" };
    tracing::info!(bot_guid, vendor_spawn_id = vendor.spawn_id,
        free_slots = bags.free_slots, grey_count = bags.grey_count,
        trigger, "vendor_trip_start");

    let walked = match walk_far(client, bot_guid,
        crate::nav::Dest { x: vendor.pos.x, y: vendor.pos.y, z: vendor.pos.z },
        goal, rotation).await
    {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(bot_guid, error = %e, "vendor_trip: outbound nav failed");
            false
        }
    };
    if bot_is_dead(client, bot_guid).await { return VendorTripOutcome::BotDead; }

    let mut sold_count = 0u32;
    let mut copper_gained = 0u64;
    let mut repair_copper = 0u64;
    if walked {
        match client.call("bot.vendor_sell", serde_json::json!({
            "bot_guid": bot_guid as i64,
            "vendor_spawn_id": vendor.spawn_id,
            "max_quality": 0,
        })).await {
            Ok(raw) => match serde_json::from_value::<SellResult>(raw) {
                Ok(r) => {
                    if let Some(code) = &r.fail_code {
                        tracing::warn!(bot_guid, fail_code = %code, "vendor_trip: sell soft-fail");
                    }
                    sold_count = r.sold_count;
                    copper_gained = r.copper_gained;
                }
                Err(e) => tracing::warn!(bot_guid, error = %e, "vendor_trip: sell shape"),
            },
            Err(e) => tracing::warn!(bot_guid, error = %e, "vendor_trip: sell errored"),
        }
        // Repair is independent of the sell outcome (spec §5) and self-gating (§1).
        if vendor.can_repair {
            match client.call("bot.repair", serde_json::json!({
                "bot_guid": bot_guid as i64,
                "vendor_spawn_id": vendor.spawn_id,
            })).await {
                Ok(raw) => match serde_json::from_value::<RepairResult>(raw) {
                    Ok(r) => {
                        if let Some(code) = &r.fail_code {
                            tracing::warn!(bot_guid, fail_code = %code, "vendor_trip: repair soft-fail");
                        }
                        repair_copper = r.copper_spent;
                    }
                    Err(e) => tracing::warn!(bot_guid, error = %e, "vendor_trip: repair shape"),
                },
                Err(e) => tracing::warn!(bot_guid, error = %e, "vendor_trip: repair errored"),
            }
        }
    }
    // Checkpoint 2: dead after verbs — emit result with what we learned, then abort.
    let mut outcome = VendorTripOutcome::Done;
    let mut free_after: Option<u32> = None;
    if bot_is_dead(client, bot_guid).await {
        outcome = VendorTripOutcome::BotDead;
    } else {
        // free_slots_after: best-effort observability (spec §4 step 6).
        free_after = read_bags(client, bot_guid).await.map(|s| s.free_slots).ok();

        // Return leg — best-effort; Scanning recenters via Wandering on failure (spec §5).
        let a = &goal.anchor_point;
        if let Err(e) = walk_far(client, bot_guid,
            crate::nav::Dest { x: a.x, y: a.y, z: a.z },
            goal, rotation).await
        {
            tracing::warn!(bot_guid, error = %e, "vendor_trip: return nav failed");
        }
        // Checkpoint 3: dead after return leg.
        if bot_is_dead(client, bot_guid).await {
            outcome = VendorTripOutcome::BotDead;
        }
    }

    tracing::info!(bot_guid, sold_count, copper_gained, repair_copper_spent = repair_copper,
        free_slots_after = ?free_after, outcome = ?outcome, "vendor_trip_result");
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use axum::{extract::Path as AxumPath, http::StatusCode, routing::post, Json, Router};
    use std::time::Duration;
    use tot_goal_contract::{GrindGoal, MobFilter, VendorInfo, WorldPos};
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
    fn rotation() -> crate::combat::RotationPlugin { crate::rotations::build("auto_attack").unwrap() }

    fn vendor() -> VendorInfo {
        // Placed 100 yd from origin along +x so both outbound (0→100) and return
        // (100→50) legs are ≤150 yd — a single walk_far hop each. Tests that need
        // a longer leg spawn their own mock with custom coords.
        VendorInfo { spawn_id: 40001,
                     pos: WorldPos { map_id: 1, x: 100.0, y: 0.0, z: 0.0 },
                     can_repair: true }
    }
    fn goal_with_vendor() -> GrindGoal {
        GrindGoal {
            // Anchor 50 yd from origin — return leg is 100→50 = 50 yd, within one hop.
            anchor_point: WorldPos { map_id: 1, x: 50.0, y: 0.0, z: 0.0 },
            wander_radius: 90.0, max_search_radius: 35.0,
            mob_filter: MobFilter { min_level: 19, max_level: 25, creature_type: None },
            to_level: 23, kill_count: None, rest_threshold: 0.35,
            rotation_id: None, vendor: Some(vendor()),
        }
    }

    #[tokio::test]
    async fn economy_due_skips_entirely_without_vendor() {
        // A goal without vendor must make ZERO harness calls (spec §3/§9).
        let base = spawn_mock(|name, _| panic!("unexpected tool {name} — no-vendor goal must not poll")).await;
        let mut goal = goal_with_vendor();
        goal.vendor = None;
        let mut last_check = 0u32;
        let due = economy_due(&client(&base), 1114, &goal, 10, &mut last_check, None,
                              Duration::from_secs(VENDOR_TRIP_COOLDOWN_S)).await;
        assert!(due.is_none());
    }

    #[tokio::test]
    async fn economy_due_respects_kill_stride() {
        let base = spawn_mock(|name, _| panic!("unexpected tool {name} — stride must gate the poll")).await;
        let mut last_check = 0u32;
        // kills 4 < stride 5 → no poll, no trigger.
        let due = economy_due(&client(&base), 1114, &goal_with_vendor(), 4, &mut last_check, None,
                              Duration::from_secs(VENDOR_TRIP_COOLDOWN_S)).await;
        assert!(due.is_none());
        assert_eq!(last_check, 0, "stride miss must not stamp the check counter");
    }

    #[tokio::test]
    async fn economy_due_fires_on_full_bags_at_stride() {
        let base = spawn_mock(move |name, _| {
            assert_eq!(name, "obs.get_inventory");
            // 16 backpack items, 10 grey → free 0, greys 10 → triggered.
            let grey = |slot: u64| json!({"quality": 0, "slot": slot, "item_entry": 750,
                                          "name": "x", "itemset": 0, "count": 1, "guid": slot});
            let white = |slot: u64| json!({"quality": 1, "slot": slot, "item_entry": 2589,
                                           "name": "x", "itemset": 0, "count": 1, "guid": slot});
            let mut bags: Vec<serde_json::Value> = (0..10).map(grey).collect();
            bags.extend((10..16).map(white));
            json!({"equipped": [], "bags": bags, "nested_bags": []})
        }).await;
        let mut last_check = 0u32;
        let due = economy_due(&client(&base), 1114, &goal_with_vendor(), 5, &mut last_check, None,
                              Duration::from_secs(VENDOR_TRIP_COOLDOWN_S)).await;
        assert_eq!(due, Some(BagSummary { free_slots: 0, grey_count: 10 }));
        assert_eq!(last_check, 5, "poll must stamp the check counter");
    }

    #[tokio::test]
    async fn economy_due_suppressed_by_cooldown_then_fires_after() {
        let base = spawn_mock(|_, _| json!({"equipped": [], "bags":
            (0..16).map(|s| json!({"quality": 0, "slot": s, "item_entry": 750, "name": "x",
                                   "itemset": 0, "count": 1, "guid": s})).collect::<Vec<_>>(),
            "nested_bags": []})).await;
        let mut last_check = 0u32;
        let last_trip = Some(std::time::Instant::now());
        let cooldown = Duration::from_millis(50);
        let due = economy_due(&client(&base), 1114, &goal_with_vendor(), 5, &mut last_check,
                              last_trip, cooldown).await;
        assert!(due.is_none(), "inside cooldown → suppressed");
        tokio::time::sleep(Duration::from_millis(60)).await;
        let due = economy_due(&client(&base), 1114, &goal_with_vendor(), 5, &mut last_check,
                              last_trip, cooldown).await;
        assert!(due.is_some(), "cooldown elapsed → fires");
    }

    #[tokio::test]
    async fn economy_due_poll_error_skips_quietly() {
        // Malformed response → Shape error inside read_bags → None (never terminal, spec §5).
        let base = spawn_mock(|_, _| json!({"bags": "not-an-array"})).await;
        let mut last_check = 0u32;
        let due = economy_due(&client(&base), 1114, &goal_with_vendor(), 5, &mut last_check, None,
                              Duration::from_secs(VENDOR_TRIP_COOLDOWN_S)).await;
        assert!(due.is_none());
    }

    /// Backpack with 14 items (12 grey) + one 6-slot bag with 4 items (2 grey):
    /// free = (16-14) + (6-4) = 4; greys = 14. Equipped greys must NOT count.
    fn busy_inventory() -> serde_json::Value {
        let grey = |slot: u64| json!({"slot": slot, "item_entry": 750, "name": "Tough Wolf Meat",
                                      "itemset": 0, "quality": 0, "count": 1, "guid": 9000 + slot});
        let white = |slot: u64| json!({"slot": slot, "item_entry": 2589, "name": "Linen Cloth",
                                       "itemset": 0, "quality": 1, "count": 5, "guid": 9100 + slot});
        let mut bags: Vec<serde_json::Value> = (23..35).map(grey).collect(); // 12 greys
        bags.push(white(35)); bags.push(white(36)); // 14 occupied
        json!({
            "equipped": [ {"slot": 0, "item_entry": 1, "name": "Grey Hat (equipped)",
                           "itemset": 0, "quality": 0, "count": 1, "guid": 8000} ],
            "bags": bags,
            "nested_bags": [ {"bag_slot": 19, "capacity": 6,
                              "contents": [grey(0), grey(1), white(2), white(3)]} ]
        })
    }

    #[test]
    fn summarize_counts_free_slots_and_greys() {
        let s = summarize_inventory(busy_inventory()).unwrap();
        assert_eq!(s.free_slots, 4, "(16-14) backpack + (6-4) nested");
        assert_eq!(s.grey_count, 14, "12 backpack + 2 nested; equipped excluded");
    }

    #[test]
    fn empty_inventory_is_untriggered() {
        let s = summarize_inventory(json!({"equipped": [], "bags": [], "nested_bags": []})).unwrap();
        assert_eq!(s.free_slots, 16);
        assert_eq!(s.grey_count, 0);
        assert!(!s.triggered());
    }

    #[test]
    fn missing_nested_bags_defaults_empty() {
        // Older mocks / minimal responses omit nested_bags — must not be a Shape error.
        let s = summarize_inventory(json!({"bags": []})).unwrap();
        assert_eq!(s.free_slots, 16);
    }

    #[test]
    fn triggered_on_free_slots() {
        assert!(BagSummary { free_slots: 3, grey_count: 0 }.triggered());
        assert!(!BagSummary { free_slots: 4, grey_count: 0 }.triggered());
    }

    #[test]
    fn triggered_on_grey_count() {
        assert!(BagSummary { free_slots: 16, grey_count: 8 }.triggered());
        assert!(!BagSummary { free_slots: 16, grey_count: 7 }.triggered());
    }

    type CallLog = std::sync::Arc<std::sync::Mutex<Vec<String>>>;
    type TrackedPos = std::sync::Arc<std::sync::Mutex<(f64, f64, f64)>>;

    /// Stateful mock for full trips: tracks call order + last move_path endpoint
    /// so arrival polls succeed for BOTH legs (vendor out, anchor back).
    fn trip_mock_state() -> (CallLog, TrackedPos) {
        (std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
         std::sync::Arc::new(std::sync::Mutex::new((0.0, 0.0, 0.0))))
    }

    fn trip_handler(
        calls: CallLog,
        pos: TrackedPos,
        hp_pct: u32,
        sell_result: serde_json::Value,
    ) -> impl Fn(String, serde_json::Value) -> serde_json::Value + Send + Sync + 'static {
        move |name: String, args: serde_json::Value| {
            calls.lock().unwrap().push(name.clone());
            match name.as_str() {
                "nav.find_path" => json!({"path_type": 1i64, "points": [
                    {"x": 0.0, "y": 0.0, "z": 0.0},
                    {"x": args["dest_x"], "y": args["dest_y"], "z": args["dest_z"]}]}),
                "bot.move_path" => {
                    // Track the final (last) waypoint so arrival polls succeed.
                    let pts = args["points"].as_array().unwrap();
                    let p = pts.last().unwrap().clone();
                    *pos.lock().unwrap() = (p["x"].as_f64().unwrap(),
                                            p["y"].as_f64().unwrap(),
                                            p["z"].as_f64().unwrap());
                    json!({"launched": true, "duration_ms": 0,
                           "final": {"x": p["x"], "y": p["y"], "z": p["z"]}})
                }
                "obs.get_position" => {
                    let (x, y, z) = *pos.lock().unwrap();
                    json!({"x": x, "y": y, "z": z, "map_id": 1, "zone_id": 1,
                           "area_id": 1, "orientation": 0.0})
                }
                "obs.get_state" => json!({"self": {"level": 22, "hp_pct": hp_pct}}),
                "bot.vendor_sell" => sell_result.clone(),
                "bot.repair" => json!({"copper_spent": 123u64}),
                "obs.get_inventory" => json!({"equipped": [], "bags": [], "nested_bags": []}),
                other => panic!("unexpected tool {other}"),
            }
        }
    }

    #[tokio::test]
    async fn vendor_trip_happy_path_sells_then_repairs_then_returns() {
        let (calls, pos) = trip_mock_state();
        let base = spawn_mock(trip_handler(calls.clone(), pos, 100,
            json!({"sold_count": 9u32, "copper_gained": 1234u64}))).await;
        let out = run_vendor_trip(&client(&base), 1114, &goal_with_vendor(), &vendor(),
                                  BagSummary { free_slots: 0, grey_count: 10 }, &rotation()).await;
        assert_eq!(out, VendorTripOutcome::Done);
        let seq = calls.lock().unwrap().clone();
        let idx = |t: &str| seq.iter().position(|c| c == t)
            .unwrap_or_else(|| panic!("{t} not called: {seq:?}"));
        assert!(idx("nav.find_path") < idx("bot.vendor_sell"), "walk before sell: {seq:?}");
        assert!(idx("bot.vendor_sell") < idx("bot.repair"), "sell before repair: {seq:?}");
        assert!(idx("bot.repair") < seq.iter().rposition(|c| c == "nav.find_path").unwrap(),
                "repair before the return leg: {seq:?}");
    }

    #[tokio::test]
    async fn vendor_trip_skips_repair_when_vendor_cannot() {
        let (calls, pos) = trip_mock_state();
        let base = spawn_mock(trip_handler(calls.clone(), pos, 100,
            json!({"sold_count": 9u32, "copper_gained": 1234u64}))).await;
        let mut v = vendor();
        v.can_repair = false;
        let out = run_vendor_trip(&client(&base), 1114, &goal_with_vendor(), &v,
                                  BagSummary { free_slots: 0, grey_count: 10 }, &rotation()).await;
        assert_eq!(out, VendorTripOutcome::Done);
        assert!(!calls.lock().unwrap().contains(&"bot.repair".to_string()));
    }

    #[tokio::test]
    async fn vendor_trip_soft_fail_still_repairs_and_returns() {
        // no_vendor on sell: repair is an independent gate (spec §5) — still attempted.
        let (calls, pos) = trip_mock_state();
        let base = spawn_mock(trip_handler(calls.clone(), pos, 100,
            json!({"sold_count": 0u32, "copper_gained": 0u64, "fail_code": "no_vendor"}))).await;
        let out = run_vendor_trip(&client(&base), 1114, &goal_with_vendor(), &vendor(),
                                  BagSummary { free_slots: 0, grey_count: 10 }, &rotation()).await;
        assert_eq!(out, VendorTripOutcome::Done);
        assert!(calls.lock().unwrap().contains(&"bot.repair".to_string()));
    }

    #[tokio::test]
    async fn vendor_trip_outbound_nav_failure_skips_verbs() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let c2 = calls.clone();
        let base = spawn_mock(move |name, _| {
            c2.lock().unwrap().push(name.clone());
            match name.as_str() {
                // walk_far reads obs.get_position before the first hop to get the bot position.
                "obs.get_position" => json!({"x": 0.0, "y": 0.0, "z": 0.0,
                    "map_id": 1, "zone_id": 1, "area_id": 1, "orientation": 0.0}),
                "nav.find_path" => json!({"path_type": 8i64, "points": []}), // NOPATH
                "obs.get_state" => json!({"self": {"level": 22, "hp_pct": 100}}),
                "obs.get_inventory" => json!({"equipped": [], "bags": [], "nested_bags": []}),
                other => panic!("unexpected tool {other} after NOPATH"),
            }
        }).await;
        let out = run_vendor_trip(&client(&base), 1114, &goal_with_vendor(), &vendor(),
                                  BagSummary { free_slots: 0, grey_count: 10 }, &rotation()).await;
        assert_eq!(out, VendorTripOutcome::Done, "nav failure is non-terminal (spec §5)");
        let seq = calls.lock().unwrap().clone();
        assert!(!seq.contains(&"bot.vendor_sell".to_string()), "no sell after NOPATH: {seq:?}");
        assert!(!seq.contains(&"bot.repair".to_string()), "no repair after NOPATH: {seq:?}");
    }

    #[tokio::test]
    async fn vendor_trip_dead_after_outbound_aborts_to_recovery() {
        let (calls, pos) = trip_mock_state();
        let base = spawn_mock(trip_handler(calls.clone(), pos, 0, // hp 0 at every checkpoint
            json!({"sold_count": 0u32, "copper_gained": 0u64}))).await;
        let out = run_vendor_trip(&client(&base), 1114, &goal_with_vendor(), &vendor(),
                                  BagSummary { free_slots: 0, grey_count: 10 }, &rotation()).await;
        assert_eq!(out, VendorTripOutcome::BotDead);
        assert!(!calls.lock().unwrap().contains(&"bot.vendor_sell".to_string()),
                "dead bot must not sell");
    }

    // ── walk_far unit tests ────────────────────────────────────────────────────────────────

    /// walk_far with dest within one hop must issue exactly ONE nav.find_path aimed at
    /// the exact dest (no intermediate hop).
    #[tokio::test]
    async fn walk_far_single_hop_when_close() {
        use std::sync::Arc;
        use std::sync::Mutex;
        // Track every dest_x sent to nav.find_path.
        let fp_dests: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
        let fp2 = fp_dests.clone();
        // Tracked position so walk_to's arrival check sees the bot at the final waypoint.
        let pos: Arc<Mutex<(f64, f64, f64)>> = Arc::new(Mutex::new((0.0, 0.0, 0.0)));
        let pos2 = pos.clone();
        let base = spawn_mock(move |name, args| {
            match name.as_str() {
                // walk_far calls obs.get_position to measure distance before hopping.
                // walk_to's arrival check also calls obs.get_position — it needs to see
                // the bot at the final waypoint so the arrival tolerance (≤2 yd) passes.
                "obs.get_position" => {
                    let (x, y, z) = *pos2.lock().unwrap();
                    json!({"x": x, "y": y, "z": z,
                           "map_id": 0, "zone_id": 1, "area_id": 1, "orientation": 0.0})
                }
                "nav.find_path" => {
                    fp2.lock().unwrap().push(args["dest_x"].as_f64().unwrap());
                    json!({"path_type": 1i64, "points": [
                        {"x": 0.0, "y": 0.0, "z": 0.0},
                        {"x": args["dest_x"], "y": args["dest_y"], "z": args["dest_z"]}
                    ]})
                }
                "bot.move_path" => {
                    let pts = args["points"].as_array().unwrap();
                    let p = pts.last().unwrap();
                    let nx = p["x"].as_f64().unwrap();
                    let ny = p["y"].as_f64().unwrap();
                    let nz = p["z"].as_f64().unwrap();
                    // Advance tracked position so the arrival check passes.
                    *pos2.lock().unwrap() = (nx, ny, nz);
                    json!({"launched": true, "duration_ms": 0,
                           "final": {"x": nx, "y": ny, "z": nz}})
                }
                other => panic!("unexpected tool {other}"),
            }
        }).await;
        // Dest 50 yd away — below the 60 yd hop threshold.
        let dest = crate::nav::Dest { x: 50.0, y: 0.0, z: 0.0 };
        let result = walk_far(&client(&base), 1, dest, &goal_with_vendor(), &rotation()).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        let dests = fp_dests.lock().unwrap().clone();
        assert_eq!(dests.len(), 1, "exactly one nav.find_path call, got {} calls: {dests:?}", dests.len());
        assert!((dests[0] - 50.0).abs() < 0.001,
            "dest_x of the single call must equal the exact dest (50), got {}", dests[0]);
    }

    /// walk_far with a 400 yd leg must segment into 7 nav.find_path calls:
    /// hops at 60..360 yd (6 hops), then the final exact dest at 400 yd.
    #[tokio::test]
    async fn walk_far_segments_long_leg() {
        use std::sync::Arc;
        use std::sync::Mutex;

        // Tracked position — starts at (0,0,0); updated on each bot.move_path.
        let pos: Arc<Mutex<(f64, f64, f64)>> = Arc::new(Mutex::new((0.0, 0.0, 0.0)));
        let pos2 = pos.clone();
        // Record every dest_x sent to nav.find_path.
        let fp_dests: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
        let fp2 = fp_dests.clone();

        let base = spawn_mock(move |name, args| {
            match name.as_str() {
                "obs.get_position" => {
                    let (x, y, z) = *pos2.lock().unwrap();
                    json!({"x": x, "y": y, "z": z,
                           "map_id": 0, "zone_id": 1, "area_id": 1, "orientation": 0.0})
                }
                "nav.find_path" => {
                    fp2.lock().unwrap().push(args["dest_x"].as_f64().unwrap());
                    json!({"path_type": 1i64, "points": [
                        {"x": args["dest_x"].as_f64().unwrap() - 1.0, "y": 0.0, "z": 0.0},
                        {"x": args["dest_x"], "y": args["dest_y"], "z": args["dest_z"]}
                    ]})
                }
                "bot.move_path" => {
                    // Advance tracked position to the last waypoint (final destination of this hop).
                    let pts = args["points"].as_array().unwrap();
                    let p = pts.last().unwrap();
                    let nx = p["x"].as_f64().unwrap();
                    let ny = p["y"].as_f64().unwrap();
                    let nz = p["z"].as_f64().unwrap();
                    *pos2.lock().unwrap() = (nx, ny, nz);
                    json!({"launched": true, "duration_ms": 0,
                           "final": {"x": nx, "y": ny, "z": nz}})
                }
                other => panic!("unexpected tool {other}"),
            }
        }).await;

        // Dest 400 yd along +x.
        // Hop arithmetic at 60 yd hops: 60, 120, 180, 240, 300, 360 (6 hops), then
        // dist=40 ≤ 60 → final walk_to exact dest. Total: 7 find_path calls.
        let dest = crate::nav::Dest { x: 400.0, y: 0.0, z: 0.0 };
        let result = walk_far(&client(&base), 2, dest, &goal_with_vendor(), &rotation()).await;
        assert!(result.is_ok(), "expected Ok for 400yd leg, got: {result:?}");

        let dests = fp_dests.lock().unwrap().clone();
        assert_eq!(dests.len(), 7,
            "expected 7 nav.find_path calls (60..360 + exact 400), got {}: {dests:?}", dests.len());
        // Last call must be exact dest.
        assert!((dests[6] - 400.0).abs() < 0.1,
            "final hop must aim at exact dest (400), got {}", dests[6]);
    }

    /// A hop that returns NOPATH mid-leg must propagate the error; walk_far must issue
    /// ≤2 nav.find_path calls (first OK, second NOPATH).
    #[tokio::test]
    async fn walk_far_propagates_nopath_mid_leg() {
        use std::sync::Arc;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};

        let fp_count = Arc::new(AtomicU32::new(0));
        let fp2 = fp_count.clone();
        let pos: Arc<Mutex<(f64, f64, f64)>> = Arc::new(Mutex::new((0.0, 0.0, 0.0)));
        let pos2 = pos.clone();

        let base = spawn_mock(move |name, args| {
            match name.as_str() {
                "obs.get_position" => {
                    let (x, y, z) = *pos2.lock().unwrap();
                    json!({"x": x, "y": y, "z": z,
                           "map_id": 0, "zone_id": 1, "area_id": 1, "orientation": 0.0})
                }
                "nav.find_path" => {
                    let n = fp2.fetch_add(1, SeqCst);
                    if n == 0 {
                        // First hop: NORMAL → walk succeeds.
                        json!({"path_type": 1i64, "points": [
                            {"x": 0.0, "y": 0.0, "z": 0.0},
                            {"x": args["dest_x"], "y": args["dest_y"], "z": args["dest_z"]}
                        ]})
                    } else {
                        // Second hop: NOPATH → walk_far propagates the error.
                        json!({"path_type": 8i64, "points": []})
                    }
                }
                "bot.move_path" => {
                    let pts = args["points"].as_array().unwrap();
                    let p = pts.last().unwrap();
                    let nx = p["x"].as_f64().unwrap();
                    let ny = p["y"].as_f64().unwrap();
                    let nz = p["z"].as_f64().unwrap();
                    *pos2.lock().unwrap() = (nx, ny, nz);
                    json!({"launched": true, "duration_ms": 0,
                           "final": {"x": nx, "y": ny, "z": nz}})
                }
                other => panic!("unexpected tool {other}"),
            }
        }).await;

        // 400 yd leg: hop 1 OK (60 yd), hop 2 NOPATH (60→120) → error.
        let dest = crate::nav::Dest { x: 400.0, y: 0.0, z: 0.0 };
        let result = walk_far(&client(&base), 3, dest, &goal_with_vendor(), &rotation()).await;
        assert!(result.is_err(), "expected Err on NOPATH mid-leg");
        match result.unwrap_err() {
            crate::nav::NavError::NoPath => {}
            other => panic!("expected NoPath, got: {other:?}"),
        }
        assert!(fp_count.load(SeqCst) <= 2,
            "must have issued ≤2 nav.find_path calls, got {}", fp_count.load(SeqCst));
    }

    /// Bot sells successfully, then dies on the walk home.
    /// Fix 1: the function must still emit `vendor_trip_result` (outcome=BotDead) and
    /// return `BotDead` — not silently drop the copper-movement log.
    #[tokio::test]
    async fn vendor_trip_dead_after_selling_still_reports_bot_dead() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let (calls, pos) = trip_mock_state();
        let sold = std::sync::Arc::new(AtomicBool::new(false));
        let sold2 = sold.clone();

        let base = spawn_mock({
            let calls2 = calls.clone();
            move |name: String, args: serde_json::Value| {
                calls2.lock().unwrap().push(name.clone());
                match name.as_str() {
                    "nav.find_path" => json!({"path_type": 1i64, "points": [
                        {"x": 0.0, "y": 0.0, "z": 0.0},
                        {"x": args["dest_x"], "y": args["dest_y"], "z": args["dest_z"]}]}),
                    "bot.move_path" => {
                        let pts = args["points"].as_array().unwrap();
                        let p = pts.last().unwrap().clone();
                        *pos.lock().unwrap() = (p["x"].as_f64().unwrap(),
                                                p["y"].as_f64().unwrap(),
                                                p["z"].as_f64().unwrap());
                        json!({"launched": true, "duration_ms": 0,
                               "final": {"x": p["x"], "y": p["y"], "z": p["z"]}})
                    }
                    "obs.get_position" => {
                        let (x, y, z) = *pos.lock().unwrap();
                        json!({"x": x, "y": y, "z": z, "map_id": 1, "zone_id": 1,
                               "area_id": 1, "orientation": 0.0})
                    }
                    // hp: alive (100) before sell, dead (0) once sold flag is set.
                    "obs.get_state" => {
                        let hp = if sold2.load(Ordering::SeqCst) { 0 } else { 100 };
                        json!({"self": {"level": 22, "hp_pct": hp}})
                    }
                    "bot.vendor_sell" => {
                        sold2.store(true, Ordering::SeqCst);
                        json!({"sold_count": 5u32, "copper_gained": 500u64})
                    }
                    "bot.repair" => json!({"copper_spent": 50u64}),
                    "obs.get_inventory" => json!({"equipped": [], "bags": [], "nested_bags": []}),
                    other => panic!("unexpected tool {other}"),
                }
            }
        }).await;

        let out = run_vendor_trip(&client(&base), 1114, &goal_with_vendor(), &vendor(),
                                  BagSummary { free_slots: 0, grey_count: 10 }, &rotation()).await;

        // Must return BotDead (checkpoint 2 — dead right after verbs).
        assert_eq!(out, VendorTripOutcome::BotDead, "sold then died → BotDead");
        // Sell verb MUST have been called (items were sold before death).
        assert!(calls.lock().unwrap().contains(&"bot.vendor_sell".to_string()),
                "vendor_sell must be called before death");
    }

    // ── fight-through tests (live-verify round 3, 2026-06-11) ─────────────────

    /// walk_far fights through combat interruption: a Timeout on the first hop triggers
    /// a scan+fight sequence, and the subsequent retry succeeds.
    ///
    /// Mock sequence (single 50 yd hop, auto_attack rotation):
    /// 1. obs.get_position → (0,0,0) — bot at origin.
    /// 2. nav.find_path → NORMAL path to (50,0,0).
    /// 3. bot.move_path → final_pt=(50,0,0).
    /// 4. obs.get_position × 5 → always (0,0,0) — never arrives → walk_to returns Timeout.
    /// 5. obs.get_nearby_hostiles → hostile guid=999 at distance 5 (within WALK_FAR_AGGRO_YD).
    /// 6. bot.attack → success (fight tick 1).
    /// 7. obs.get_nearby_hostiles → empty → TargetDead → fight() returns.
    /// 8. obs.get_state → alive (fight loop).
    /// 9. obs.get_position → (0,0,0) — re-read bot position for retry hop.
    /// 10. nav.find_path → NORMAL (50,0,0).
    /// 11. bot.move_path → final_pt=(50,0,0).
    /// 12. obs.get_position → (50,0,0) — arrival passes → Ok(()).
    ///
    /// Real timing: walk_to Timeout = 5 polls × 500ms = ~2.5s. Accept it; 1 timeout per test.
    #[tokio::test(flavor = "multi_thread")]
    async fn walk_far_fights_through_on_timeout() {
        use std::sync::Arc;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};

        // Counts how many times bot.move_path has been called. After the first call the
        // position mock returns the origin (Timeout); after the second it returns the dest.
        let move_calls = Arc::new(AtomicU32::new(0));
        let mc2 = move_calls.clone();
        // Record each tool called, so assertions can verify bot.attack was used.
        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let log2 = call_log.clone();
        // Number of obs.get_nearby_hostiles calls: 0→hostile alive, 1+→empty (target dead).
        let scan_calls = Arc::new(AtomicU32::new(0));
        let sc2 = scan_calls.clone();

        let base = spawn_mock(move |name, _args| {
            log2.lock().unwrap().push(name.clone());
            match name.as_str() {
                "obs.get_position" => {
                    // Before first move_path completes (or on arrival poll after it):
                    //   return origin so walk_to polls all 5 checks and returns Timeout.
                    // After second move_path: return the dest so arrival passes.
                    let moves = mc2.load(SeqCst);
                    if moves < 2 {
                        json!({"x": 0.0, "y": 0.0, "z": 0.0,
                               "map_id": 0, "zone_id": 1, "area_id": 1, "orientation": 0.0})
                    } else {
                        json!({"x": 50.0, "y": 0.0, "z": 0.0,
                               "map_id": 0, "zone_id": 1, "area_id": 1, "orientation": 0.0})
                    }
                }
                "nav.find_path" => json!({"path_type": 1i64, "points": [
                    {"x": 0.0, "y": 0.0, "z": 0.0},
                    {"x": 50.0, "y": 0.0, "z": 0.0}
                ]}),
                "bot.move_path" => {
                    mc2.fetch_add(1, SeqCst);
                    json!({"launched": true, "duration_ms": 0,
                           "final": {"x": 50.0, "y": 0.0, "z": 0.0}})
                }
                "obs.get_nearby_hostiles" => {
                    // First scan (from fight-through path): hostile at distance 5.
                    // Subsequent scans (fight() combat poll): target gone → TargetDead.
                    let n = sc2.fetch_add(1, SeqCst);
                    if n == 0 {
                        json!({"hostiles": [{"guid": 999u64, "name": "Wildboar",
                            "level": 22, "hp_pct": 100.0, "distance": 5.0, "is_alive": true,
                            "x": 5.0, "y": 0.0, "z": 0.0}]})
                    } else {
                        json!({"hostiles": []})
                    }
                }
                "bot.attack" => json!({"attacked": true, "target_guid": 999u64,
                                       "target_name": "Wildboar"}),
                "obs.get_state" => json!({"self": {"level": 22, "hp_pct": 80}}),
                other => panic!("unexpected tool {other}"),
            }
        }).await;

        let dest = crate::nav::Dest { x: 50.0, y: 0.0, z: 0.0 };
        let result = walk_far(&client(&base), 1114, dest, &goal_with_vendor(), &rotation()).await;
        assert!(result.is_ok(), "expected Ok after fight-through, got: {result:?}");

        let log = call_log.lock().unwrap().clone();
        // Verify bot.attack was called (fight was engaged).
        assert!(log.contains(&"bot.attack".to_string()),
            "bot.attack must be called during fight-through: {log:?}");
        // Verify at least 2 nav.find_path calls (first hop → Timeout, retry after fight).
        let fp_count = log.iter().filter(|c| *c == "nav.find_path").count();
        assert!(fp_count >= 2,
            "expected ≥2 nav.find_path calls (first fails, retry succeeds), got {fp_count}: {log:?}");
    }

    /// walk_far propagates Timeout unchanged when no attacker is in range.
    ///
    /// Same partway-move mock as above but obs.get_nearby_hostiles returns empty.
    /// walk_far must return Err(Timeout) immediately (no fight, no retry).
    #[tokio::test(flavor = "multi_thread")]
    async fn walk_far_no_attacker_propagates_timeout() {
        use std::sync::Arc;
        use std::sync::Mutex;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let log2 = call_log.clone();

        let base = spawn_mock(move |name, _args| {
            log2.lock().unwrap().push(name.clone());
            match name.as_str() {
                "obs.get_position" =>
                    // Always at origin — bot never arrives → walk_to Timeout.
                    json!({"x": 0.0, "y": 0.0, "z": 0.0,
                           "map_id": 0, "zone_id": 1, "area_id": 1, "orientation": 0.0}),
                "nav.find_path" => json!({"path_type": 1i64, "points": [
                    {"x": 0.0, "y": 0.0, "z": 0.0},
                    {"x": 50.0, "y": 0.0, "z": 0.0}
                ]}),
                "bot.move_path" => json!({"launched": true, "duration_ms": 0,
                                         "final": {"x": 50.0, "y": 0.0, "z": 0.0}}),
                // No hostiles in range — genuine nav failure, not combat.
                "obs.get_nearby_hostiles" => json!({"hostiles": []}),
                other => panic!("unexpected tool {other}"),
            }
        }).await;

        let dest = crate::nav::Dest { x: 50.0, y: 0.0, z: 0.0 };
        let result = walk_far(&client(&base), 1114, dest, &goal_with_vendor(), &rotation()).await;
        assert!(result.is_err(), "expected Err when no attacker in range");
        match result.unwrap_err() {
            crate::nav::NavError::Timeout => {}
            other => panic!("expected Timeout, got: {other:?}"),
        }

        let log = call_log.lock().unwrap().clone();
        assert!(!log.contains(&"bot.attack".to_string()),
            "bot.attack must NOT be called when no attacker in range: {log:?}");
    }
}
