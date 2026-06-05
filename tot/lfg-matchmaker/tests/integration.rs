//! Blackbox integration tests for `lfg-matchmaker`.
//!
//! These tests exercise the PUBLIC library API only — the same surface that
//! `slice-host/src/main.rs` composes.  No `#[cfg(test)]`-only private items
//! are touched; if they were, a `cargo test --test integration` invocation
//! from an EXTERNAL crate would catch the regression.
//!
//! # Composition path (mirrors slice-host)
//!
//! ```
//! let state = Arc::new(LfgAppState { queue: LfgQueue::new(), pending_placements: LfgPendingPlacements::new() });
//! let harness = LfgHarness::new(mock_base_url, "tok");
//! let cfg = LfgConfig { enabled: true, tick_secs: 1, dungeon: ..., ... };
//! // Spawn tick loop.
//! tokio::spawn(lfg_tick::run(state.clone(), harness, cfg));
//! // Mount routes.
//! let router = lfg_api::routes(state.clone());
//! ```
//!
//! # Mock-harness tool shapes
//!
//! All responses are wrapped `{"ok": true, "result": <payload>}`.
//!
//! | Tool | Payload |
//! |---|---|
//! | `obs.lfg_pending` | `{"pending":[...]}` — drain on 1st call, empty on 2nd |
//! | `lfg.cancel` | `{"cancelled":[]}` |
//! | `obs.get_state` | `{"self":{"race":"human","is_in_combat":false},"social":{"in_group":false}}` |
//! | `bot.invite_to_group` | `{}` |
//! | `bot.accept_invite` | `{}` |
//! | `obs.get_group` | `{"in_group":true,"members":[{"guid":1},...]}` |
//! | `bot.enter_instance` | `{}` |
//! | `bot.leave_group` | `{}` |

use axum::{
    extract::Path,
    routing::post,
    Json, Router,
};
use lfg_matchmaker::{
    api::{AppState as LfgAppState, PendingPlacements as LfgPendingPlacements},
    config::{Config as LfgConfig, Dungeon},
    harness::Harness as LfgHarness,
    queue::Queue as LfgQueue,
    types::{Faction, QueueEntry, Role},
    {api as lfg_api, tick as lfg_tick},
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn rfc_dungeon() -> Dungeon {
    Dungeon { id: 4, map_id: 389, x: 3.81, y: -14.82, z: -17.84, o: 4.39 }
}

/// Build a `Config` with `enabled=true` and a 1-second tick. Points at `base_url`.
fn test_cfg(base_url: String) -> LfgConfig {
    LfgConfig {
        listen_addr: "127.0.0.1:0".into(),
        harness_base_url: base_url.clone(),
        harness_bearer: "tok".into(),
        tick_secs: 1,
        dungeon: rfc_dungeon(),
        enabled: true,
    }
}

/// Enqueue one entry into the given `AppState` directly (no HTTP round-trip).
fn seed_entry(state: &LfgAppState, guid: u64, role: Role) {
    state.queue.upsert(QueueEntry {
        guid,
        role,
        dungeon_id: 4,
        faction: Faction::Alliance,
        is_real_player: false,
    });
}

// ── Mock builder ─────────────────────────────────────────────────────────────

/// Spawn a local axum mock harness that records every `(tool, id)` call.
///
/// `tool_handler` is a closure `(tool_name, args) -> serde_json::Value` that
/// returns the response PAYLOAD (the `result` field). The harness wrapper adds
/// `{"ok":true,"result":...}` around it automatically.
///
/// Returns the `http://127.0.0.1:<port>` base URL and the shared call recorder.
async fn spawn_mock<F>(tool_handler: F) -> (String, Arc<Mutex<Vec<String>>>)
where
    F: Fn(String, serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
{
    let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let calls_h = calls.clone();
    let handler_arc = Arc::new(tool_handler);

    let handler = move |Path(name): Path<String>, Json(args): Json<serde_json::Value>| {
        let calls = calls_h.clone();
        let handler_arc = handler_arc.clone();
        async move {
            // Record: prefer bot_guid, fall back to target_guid, then 0.
            let bot = args.get("bot_guid").and_then(|v| v.as_u64()).unwrap_or(0);
            let target = args.get("target_guid").and_then(|v| v.as_u64()).unwrap_or(0);
            let id = if bot != 0 { bot } else { target };
            calls.lock().unwrap().push(format!("{name}:{id}"));

            let payload = handler_arc(name.clone(), args);
            Json(serde_json::json!({ "ok": true, "result": payload }))
        }
    };

    let app = Router::new().route("/v1/tools/:name", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), calls)
}

// ── Test 1: full happy-cycle ─────────────────────────────────────────────────

/// Seed a balanced 5-man (1T/1H/3D, all Alliance bots) directly into AppState,
/// spawn `lfg_tick::run` with a live harness pointed at the mock, wait for the
/// tick to fire, and assert that the recorder logged:
///   - `obs.get_state` × 5 (reconciliation pre-form poll, Task 3),
///   - `bot.invite_to_group` × 4 (non-leader members),
///   - `bot.accept_invite` × 4,
///   - `obs.get_group` at least once (confirm-poll, Task 2),
///   - `bot.enter_instance` × 5.
///
/// Also asserts the queue is empty after a successful form.
#[tokio::test]
async fn integration_full_cycle_forms_and_places_a_balanced_5man() {
    // All calls succeed; obs.get_group always reports all 5 guids present.
    let (base, calls) = spawn_mock(|name, _args| match name.as_str() {
        "obs.lfg_pending" => serde_json::json!({ "pending": [] }),
        "lfg.cancel" => serde_json::json!({ "cancelled": [] }),
        "obs.get_state" => serde_json::json!({
            "self": { "race": "human", "is_in_combat": false },
            "social": { "in_group": false }
        }),
        "obs.get_group" => serde_json::json!({
            "in_group": true,
            "group_type": "party",
            "leader_guid": 1u64,
            "members": [
                {"guid": 1u64}, {"guid": 2u64}, {"guid": 3u64},
                {"guid": 4u64}, {"guid": 5u64}
            ]
        }),
        // bot.invite_to_group, bot.accept_invite, bot.enter_instance → {}
        _ => serde_json::json!({}),
    })
    .await;

    let state = Arc::new(LfgAppState {
        queue: LfgQueue::new(),
        pending_placements: LfgPendingPlacements::new(),
    });
    // Seed a balanced Alliance group: 1 tank, 1 healer, 3 dps.
    seed_entry(&state, 1, Role::Tank);
    seed_entry(&state, 2, Role::Healer);
    seed_entry(&state, 3, Role::Dps);
    seed_entry(&state, 4, Role::Dps);
    seed_entry(&state, 5, Role::Dps);

    assert_eq!(state.queue.len(), 5, "pre-condition: 5 entries seeded");

    let cfg = test_cfg(base.clone());
    let harness = LfgHarness::new(base, "tok".into());
    let handle = {
        let s = state.clone();
        tokio::spawn(lfg_tick::run(s, harness, cfg))
    };

    // One tick fires immediately; wait 300 ms — comfortably covers the first tick
    // plus the MAX_INVITE_CONFIRM_ATTEMPTS × INVITE_CONFIRM_DELAY overhead (5 × 200 ms
    // in the worst case — but here the mock answers immediately, so 1 poll per member).
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle.abort();

    let seq = calls.lock().unwrap().clone();

    // 1. Reconciliation: obs.get_state for each of the 5 candidates.
    let get_state_calls: Vec<_> = seq.iter().filter(|c| c.starts_with("obs.get_state")).collect();
    assert_eq!(
        get_state_calls.len(),
        5,
        "reconciliation must probe obs.get_state ×5: {seq:?}"
    );

    // 2. Invites: the leader (tank=1) invites the other 4.
    let invite_calls: Vec<_> = seq.iter().filter(|c| c.starts_with("bot.invite_to_group")).collect();
    assert_eq!(
        invite_calls.len(),
        4,
        "leader must invite 4 non-leader members: {seq:?}"
    );

    // 3. Accepts: one per non-leader member.
    let accept_calls: Vec<_> = seq.iter().filter(|c| c.starts_with("bot.accept_invite")).collect();
    assert_eq!(accept_calls.len(), 4, "4 members must accept: {seq:?}");

    // 4. Confirm-poll: at least one obs.get_group per non-leader (Task 2).
    let get_group_calls: Vec<_> = seq.iter().filter(|c| c.starts_with("obs.get_group")).collect();
    assert!(
        get_group_calls.len() >= 4,
        "confirm-poll must fire at least once per non-leader member: {seq:?}"
    );

    // 5. Placement: bot.enter_instance ×5.
    let enter_calls: Vec<_> = seq.iter().filter(|c| c.starts_with("bot.enter_instance")).collect();
    assert_eq!(
        enter_calls.len(),
        5,
        "all 5 members must be placed via bot.enter_instance: {seq:?}"
    );

    // 6. Queue must be empty — all members consumed by the successful form.
    assert_eq!(
        state.queue.len(),
        0,
        "queue must be empty after a successful form: {:?}",
        state.queue.snapshot()
    );
}

// ── Test 2: reconciliation aborts when one member is stale ───────────────────

/// One of the 5 candidates (guid 3) reports `social.in_group: true` during the
/// reconciliation pre-form poll.  The tick loop must abort this proposal:
///   - zero `bot.invite_to_group` calls (fulfill was never entered),
///   - the 4 eligible members are back in the queue,
///   - guid 3 is NOT in the queue (dropped as stale).
#[tokio::test]
async fn integration_reconciliation_aborts_when_member_stale() {
    // guid 3 is stale (in_group = true); all others are eligible.
    let stale_guid: u64 = 3;

    let (base, calls) = spawn_mock(move |name, args| match name.as_str() {
        "obs.lfg_pending" => serde_json::json!({ "pending": [] }),
        "lfg.cancel" => serde_json::json!({ "cancelled": [] }),
        "obs.get_state" => {
            let tgt = args.get("target_guid").and_then(|v| v.as_u64()).unwrap_or(0);
            let in_group = tgt == stale_guid;
            serde_json::json!({
                "self": { "race": "human", "is_in_combat": false },
                "social": { "in_group": in_group }
            })
        }
        _ => serde_json::json!({}),
    })
    .await;

    let state = Arc::new(LfgAppState {
        queue: LfgQueue::new(),
        pending_placements: LfgPendingPlacements::new(),
    });
    seed_entry(&state, 1, Role::Tank);
    seed_entry(&state, 2, Role::Healer);
    seed_entry(&state, 3, Role::Dps); // stale
    seed_entry(&state, 4, Role::Dps);
    seed_entry(&state, 5, Role::Dps);

    let cfg = test_cfg(base.clone());
    let harness = LfgHarness::new(base, "tok".into());
    let handle = {
        let s = state.clone();
        tokio::spawn(lfg_tick::run(s, harness, cfg))
    };

    tokio::time::sleep(Duration::from_millis(300)).await;
    handle.abort();

    let seq = calls.lock().unwrap().clone();

    // No invites must have been made — reconciliation aborted the proposal.
    let invite_calls: Vec<_> = seq.iter().filter(|c| c.starts_with("bot.invite_to_group")).collect();
    assert!(
        invite_calls.is_empty(),
        "reconciliation abort: zero bot.invite_to_group expected: {seq:?}"
    );

    // The 4 eligible members must be back in the queue.
    let queue_guids: Vec<u64> = {
        let mut v: Vec<u64> = state.queue.snapshot().iter().map(|e| e.guid).collect();
        v.sort();
        v
    };
    assert_eq!(
        queue_guids,
        vec![1, 2, 4, 5],
        "eligible members must be re-queued: {queue_guids:?}"
    );

    // The stale member must be dropped, not re-queued.
    assert!(
        !queue_guids.contains(&stale_guid),
        "stale guid {stale_guid} must be absent from the queue: {queue_guids:?}"
    );
}

// ── Test 3: confirm-timeout triggers rollback ─────────────────────────────────

/// When `obs.get_group` never includes a given member, `confirm_member_joined`
/// exhausts its poll cap and `fulfill` rolls back.  Assert:
///   - `bot.leave_group` was recorded (rollback executed — Task 2),
///   - `bot.enter_instance` was NOT recorded (no placement on failed form),
///   - The queue is NOT empty (failed members re-queued, minus the poison-pill).
///
/// Setup: 5-member group, but `obs.get_group` always returns only the leader.
/// Member 2 (healer) is the first invite target; it will never appear in the
/// group, so the confirm-poll times out on member 2.  After rollback the tick
/// loop evicts member 2 (failed_guid) and re-queues the remaining innocents.
#[tokio::test]
async fn integration_confirm_timeout_rolls_back() {
    // obs.get_group always returns a group containing only the leader (guid 1).
    // This forces confirm_member_joined to time out for the first non-leader.
    let (base, calls) = spawn_mock(|name, _args| match name.as_str() {
        "obs.lfg_pending" => serde_json::json!({ "pending": [] }),
        "lfg.cancel" => serde_json::json!({ "cancelled": [] }),
        "obs.get_state" => serde_json::json!({
            "self": { "race": "human", "is_in_combat": false },
            "social": { "in_group": false }
        }),
        "obs.get_group" => serde_json::json!({
            "in_group": true,
            "group_type": "party",
            "leader_guid": 1u64,
            "members": [{"guid": 1u64}]  // only the leader — member never joins
        }),
        // bot.invite_to_group, bot.accept_invite, bot.leave_group → {}
        _ => serde_json::json!({}),
    })
    .await;

    let state = Arc::new(LfgAppState {
        queue: LfgQueue::new(),
        pending_placements: LfgPendingPlacements::new(),
    });
    seed_entry(&state, 1, Role::Tank);
    seed_entry(&state, 2, Role::Healer);
    seed_entry(&state, 3, Role::Dps);
    seed_entry(&state, 4, Role::Dps);
    seed_entry(&state, 5, Role::Dps);

    let cfg = test_cfg(base.clone());
    let harness = LfgHarness::new(base, "tok".into());
    let handle = {
        let s = state.clone();
        tokio::spawn(lfg_tick::run(s, harness, cfg))
    };

    // The confirm-poll has MAX_INVITE_CONFIRM_ATTEMPTS=5 attempts × 200 ms delay
    // = up to 1000 ms before timing out.  Wait 1.5 s to be safe.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    handle.abort();

    let seq = calls.lock().unwrap().clone();

    // Rollback must have fired: leader (guid 1) is in joined when timeout fires.
    let leave_calls: Vec<_> = seq.iter().filter(|c| c.starts_with("bot.leave_group")).collect();
    assert!(
        !leave_calls.is_empty(),
        "rollback must issue bot.leave_group: {seq:?}"
    );
    // Specifically, the leader must be rolled back.
    assert!(
        leave_calls.iter().any(|c| c.as_str() == "bot.leave_group:1"),
        "leader (guid 1) must receive bot.leave_group during rollback: {seq:?}"
    );

    // No placement must have occurred — form failed before enter_instance.
    let enter_calls: Vec<_> = seq.iter().filter(|c| c.starts_with("bot.enter_instance")).collect();
    assert!(
        enter_calls.is_empty(),
        "no bot.enter_instance on confirm-timeout: {seq:?}"
    );

    // Queue state: failed_guid (the healer, guid 2, is the first non-leader in
    // MatchProposal::members()) is evicted; the other 4 members are re-queued.
    // NOTE: The tick loop may have attempted a second match after eviction if
    // 4 members remain — but since 4 < 5, no new match fires.  We simply assert
    // that guid 2 is absent (evicted) and the queue is not empty.
    let queue_guids: Vec<u64> = {
        let mut v: Vec<u64> = state.queue.snapshot().iter().map(|e| e.guid).collect();
        v.sort();
        v
    };
    assert!(
        !queue_guids.contains(&2u64),
        "confirm-timeout evicts the non-confirming member (guid 2): {queue_guids:?}"
    );
    assert!(
        !queue_guids.is_empty(),
        "innocent members must be re-queued after rollback: {queue_guids:?}"
    );
}

// ── Test 4: routes() mounts correctly (composition-path smoke test) ──────────

/// Verify that `lfg_api::routes(state)` + `axum::serve` exposes `POST /queue`
/// and `GET /queue` as a blackbox consumer would see them.  Uses `reqwest` —
/// no internal axum `oneshot` helper, which might have more liberal routing.
#[tokio::test]
async fn integration_routes_post_and_get_queue() {
    let state = Arc::new(LfgAppState {
        queue: LfgQueue::new(),
        pending_placements: LfgPendingPlacements::new(),
    });

    // `routes()` does NOT include /healthz — mirrors the slice-host mounting path.
    let router = lfg_api::routes(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://{addr}");

    let client = reqwest::Client::new();

    // POST /queue enqueues an entry.
    let r = client
        .post(format!("{base}/queue"))
        .json(&serde_json::json!({
            "guid": 99,
            "role": "tank",
            "dungeon_id": 4,
            "faction": "alliance"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK, "POST /queue must succeed");

    // GET /queue lists the entry.
    let listed: Vec<serde_json::Value> =
        client.get(format!("{base}/queue")).send().await.unwrap().json().await.unwrap();
    assert_eq!(listed.len(), 1, "GET /queue must return the enqueued entry");
    assert_eq!(listed[0]["guid"], 99u64);

    // AppState is shared — the tick loop would see the same entry.
    assert_eq!(state.queue.len(), 1, "queue state visible through Arc");
}
