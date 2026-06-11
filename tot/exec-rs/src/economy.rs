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
#[allow(dead_code)] // used by Task 8 as the default cooldown argument
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
pub(crate) struct BagSummary {
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

/// Gate the economy check (spec §3): vendor present → at most every
/// `ECONOMY_CHECK_KILL_STRIDE` kills → outside the trip cooldown → poll bags →
/// `Some(summary)` when a trigger condition holds. A failed poll logs and skips
/// (never terminal). `cooldown` is injected so tests don't wait 600 s.
#[allow(dead_code)] // wired in Task 8
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
#[allow(dead_code)] // wired in Task 8
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

/// The trip (spec §4): walk to the vendor → sell greys → repair (if able) →
/// walk back to the anchor. Every failure is soft (spec §5); death at any
/// checkpoint aborts into the caller's Recovering path.
#[allow(dead_code)] // wired in Task 8
pub(crate) async fn run_vendor_trip(
    client: &HarnessClient,
    bot_guid: u64,
    goal: &tot_goal_contract::GrindGoal,
    vendor: &tot_goal_contract::VendorInfo,
    bags: BagSummary,
) -> VendorTripOutcome {
    tracing::info!(bot_guid, vendor_spawn_id = vendor.spawn_id,
        free_slots = bags.free_slots, grey_count = bags.grey_count, "vendor_trip_start");

    let walked = match crate::nav::walk_to(client, bot_guid,
        crate::nav::Dest { x: vendor.pos.x, y: vendor.pos.y, z: vendor.pos.z }).await
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
    if bot_is_dead(client, bot_guid).await { return VendorTripOutcome::BotDead; }

    // free_slots_after: best-effort observability (spec §4 step 6).
    let free_after = read_bags(client, bot_guid).await.map(|s| s.free_slots).ok();

    // Return leg — best-effort; Scanning recenters via Wandering on failure (spec §5).
    let a = &goal.anchor_point;
    if let Err(e) = crate::nav::walk_to(client, bot_guid,
        crate::nav::Dest { x: a.x, y: a.y, z: a.z }).await
    {
        tracing::warn!(bot_guid, error = %e, "vendor_trip: return nav failed");
    }
    if bot_is_dead(client, bot_guid).await { return VendorTripOutcome::BotDead; }

    tracing::info!(bot_guid, sold_count, copper_gained, repair_copper_spent = repair_copper,
        free_slots_after = ?free_after, "vendor_trip_result");
    VendorTripOutcome::Done
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

    fn vendor() -> VendorInfo {
        VendorInfo { spawn_id: 40001,
                     pos: WorldPos { map_id: 1, x: 2200.0, y: -300.0, z: 95.0 },
                     can_repair: true }
    }
    fn goal_with_vendor() -> GrindGoal {
        GrindGoal {
            anchor_point: WorldPos { map_id: 1, x: 2100.0, y: -210.0, z: 92.0 },
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

    /// Stateful mock for full trips: tracks call order + last move_path endpoint
    /// so arrival polls succeed for BOTH legs (vendor out, anchor back).
    fn trip_mock_state() -> (std::sync::Arc<std::sync::Mutex<Vec<String>>>,
                             std::sync::Arc<std::sync::Mutex<(f64, f64, f64)>>) {
        (std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
         std::sync::Arc::new(std::sync::Mutex::new((0.0, 0.0, 0.0))))
    }

    fn trip_handler(
        calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        pos: std::sync::Arc<std::sync::Mutex<(f64, f64, f64)>>,
        hp_pct: u32,
        sell_result: serde_json::Value,
    ) -> impl Fn(String, serde_json::Value) -> serde_json::Value + Send + Sync + 'static {
        move |name: String, args: serde_json::Value| {
            calls.lock().unwrap().push(name.clone());
            match name.as_str() {
                "nav.find_path" => json!({"path_type": 1i64, "points": [
                    {"x": args["dest_x"], "y": args["dest_y"], "z": args["dest_z"]}]}),
                "bot.move_path" => {
                    let p = args["points"][0].clone();
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
                                  BagSummary { free_slots: 0, grey_count: 10 }).await;
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
                                  BagSummary { free_slots: 0, grey_count: 10 }).await;
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
                                  BagSummary { free_slots: 0, grey_count: 10 }).await;
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
                "nav.find_path" => json!({"path_type": 8i64, "points": []}), // NOPATH
                "obs.get_state" => json!({"self": {"level": 22, "hp_pct": 100}}),
                "obs.get_inventory" => json!({"equipped": [], "bags": [], "nested_bags": []}),
                other => panic!("unexpected tool {other} after NOPATH"),
            }
        }).await;
        let out = run_vendor_trip(&client(&base), 1114, &goal_with_vendor(), &vendor(),
                                  BagSummary { free_slots: 0, grey_count: 10 }).await;
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
                                  BagSummary { free_slots: 0, grey_count: 10 }).await;
        assert_eq!(out, VendorTripOutcome::BotDead);
        assert!(!calls.lock().unwrap().contains(&"bot.vendor_sell".to_string()),
                "dead bot must not sell");
    }
}
