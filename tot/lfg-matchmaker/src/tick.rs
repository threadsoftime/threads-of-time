use crate::api::AppState;
use crate::config::Config;
use crate::harness::Harness;
use crate::matcher::{build_real_player_proposal, find_matches};
use crate::orchestrator::{form_via_lfg, fulfill};
use crate::roster::select_fill_bots;
use crate::types::QueueEntry;
use std::sync::Arc;
use std::time::Duration;

/// Maximum number of `bot.enter_instance` attempts (initial + retries) for a
/// straggler before we give up and drop the pending entry. A bot that resists
/// five consecutive teleports is either dead, in combat, or in a broken state —
/// further retries are unlikely to succeed and would accumulate indefinitely.
const MAX_PLACEMENT_ATTEMPTS: u32 = 5;

pub async fn run(state: Arc<AppState>, harness: Harness, cfg: Config) {
    let mut ticker = tokio::time::interval(Duration::from_secs(cfg.tick_secs.max(1)));
    loop {
        ticker.tick().await;

        // Step 1: Drain obs.lfg_pending intents from the server-side queue and
        // upsert them into our local queue. The adapter clears server-side on read
        // (drain semantics) so we will NOT see the same intent again next tick.
        // The harness client returns an empty Vec on an ok:false shape error; a real
        // transport failure is logged and we continue with whatever was already queued.
        match harness.lfg_pending(50).await {
            Ok(intents) => {
                for intent in intents {
                    // Map each intent to a QueueEntry. Use the first dungeon_id from
                    // the intent's list, falling back to the configured dungeon id.
                    let dungeon_id = intent
                        .dungeon_ids
                        .first()
                        .copied()
                        .unwrap_or(cfg.dungeon.id);
                    state.queue.upsert(QueueEntry {
                        guid: intent.guid,
                        role: intent.role,
                        dungeon_id,
                        faction: intent.faction,
                        is_real_player: intent.is_real_player,
                    });
                }
            }
            Err(e) => {
                eprintln!("[tick] obs.lfg_pending failed: {e}; continuing with existing queue");
            }
        }

        // Steps 2 & 3 perform live game-object mutations (lfg.form_group,
        // bot.invite_to_group, bot.enter_instance). They are gated behind
        // cfg.enabled (env var LFG_ENABLED, default false) so that when
        // slice-host compiles in this slice the deployed service starts inert —
        // no matchmaking side-effects until LFG_ENABLED=true is explicitly set.
        //
        // The obs.lfg_pending drain in Step 1 above (a read-only side-effect:
        // it clears the server-side intent queue) runs unconditionally so the
        // local queue accurately reflects intent state even in shadow mode.
        if !cfg.enabled {
            eprintln!("[tick] LFG_ENABLED=false — skipping matchmaking actions (inert mode)");
            continue;
        }

        // Re-placement pass: retry any members whose initial bot.enter_instance
        // failed (TeleportTo returned false). Runs every enabled tick so stragglers
        // get ~tick_secs to become teleportable between attempts (non-blocking —
        // no sleep inside; the next tick IS the retry delay). Drained atomically
        // (no lock held across await), updated, then re-inserted.
        {
            let pending = state.pending_placements.drain_all();
            if !pending.is_empty() {
                let mut keep = Vec::with_capacity(pending.len());
                for mut entry in pending {
                    match harness.enter_instance_direct(entry.guid, &entry.dungeon).await {
                        Ok(_) => {
                            eprintln!(
                                "[place] late-placed {} on attempt {}",
                                entry.guid, entry.attempts + 1
                            );
                            // Successfully placed — do not re-insert.
                        }
                        Err(e) => {
                            entry.attempts += 1;
                            if entry.attempts >= MAX_PLACEMENT_ATTEMPTS {
                                eprintln!(
                                    "[place] gave up on {} after {} attempts: {}",
                                    entry.guid, entry.attempts, e
                                );
                                // Drop the entry — do not re-insert.
                            } else {
                                keep.push(entry);
                            }
                        }
                    }
                }
                state.pending_placements.extend(keep);
            }
        }

        // Step 2: Real-player-priority matching — if there is at least one
        // real-player entry in the queue, try to build a bot-fill proposal first.
        let snapshot = state.queue.snapshot();
        let real_players: Vec<QueueEntry> =
            snapshot.iter().filter(|e| e.is_real_player).cloned().collect();

        for rp_entry in &real_players {
            // Check the real player is still in the queue (could have been taken by
            // a prior iteration in this tick).
            if state.queue.snapshot().iter().all(|e| e.guid != rp_entry.guid) {
                continue;
            }

            // Attempt to fill the remaining 4 slots with same-faction bots.
            match select_fill_bots(
                &harness,
                rp_entry.faction,
                &cfg.dungeon,
                &[rp_entry.guid],
                4,
            )
            .await
            {
                Err(e) => {
                    eprintln!(
                        "[tick] bot-fill for real player {} failed: {e}; will retry next tick",
                        rp_entry.guid
                    );
                    // Real player stays in queue — next tick re-attempts fill.
                }
                Ok(fillers) => {
                    let proposal =
                        match build_real_player_proposal(rp_entry, &fillers, cfg.dungeon.id) {
                            Some(p) => p,
                            None => {
                                eprintln!(
                                    "[tick] build_real_player_proposal failed for guid={}",
                                    rp_entry.guid
                                );
                                continue;
                            }
                        };

                    // Remove the real player from the queue before acting. Filler bots
                    // come from the live population (not the queue) so no take_many for them.
                    let taken = state.queue.take_many(&[rp_entry.guid]);
                    let res = form_via_lfg(&harness, &proposal, &cfg.dungeon).await;
                    if !res.formed {
                        // Re-queue the real player on failure (fillers are not in the queue).
                        for entry in taken {
                            state.queue.upsert(entry);
                        }
                    } else {
                        // Group formed — record any members whose placement failed for
                        // next-tick retry. The real player is never in unplaced (they
                        // were placed by the lfg.form_group adapter itself).
                        for &guid in &res.unplaced {
                            state.pending_placements.push(guid, cfg.dungeon.clone());
                        }
                    }
                    eprintln!(
                        "[match/lfg] dungeon={} leader={} formed={} placed={}/{} note={}",
                        res.dungeon_id,
                        res.leader,
                        res.formed,
                        res.placed,
                        res.members.len(),
                        res.note
                    );
                }
            }
        }

        // Step 3: Bot-only matching for the remainder of the queue.
        let snapshot = state.queue.snapshot();
        if snapshot.len() < 5 {
            continue;
        }
        for p in find_matches(&snapshot) {
            let taken = state.queue.take_many(&p.members());
            let res = fulfill(&harness, &p, &cfg.dungeon).await;
            if !res.formed {
                // Re-queue innocent members; evict the poison-pill bot that
                // caused the failure. If `failed_guid` is None (no attributable
                // member), re-queue everyone — this preserves the old behaviour
                // for unexpected error shapes.
                if let Some(evicted) = res.failed_guid {
                    eprintln!(
                        "[match] evicted {} after form failure: {}",
                        evicted, res.note
                    );
                    for entry in taken {
                        if entry.guid != evicted {
                            state.queue.upsert(entry);
                        }
                    }
                } else {
                    for entry in taken {
                        state.queue.upsert(entry);
                    }
                }
            } else {
                // Group formed — record any members whose placement failed for
                // next-tick retry (TeleportTo returned false on first attempt).
                for &guid in &res.unplaced {
                    state.pending_placements.push(guid, cfg.dungeon.clone());
                }
            }
            eprintln!(
                "[match] dungeon={} leader={} formed={} placed={}/{} note={}",
                res.dungeon_id,
                res.leader,
                res.formed,
                res.placed,
                res.members.len(),
                res.note
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Dungeon;
    use crate::types::{Faction, Role};
    use axum::{extract::Path, routing::post, Json, Router};
    use std::sync::{Arc, Mutex};
    use std::time::Duration as StdDuration;

    // Mock that records calls and returns ok for everything.
    async fn spawn_ok_recording_mock() -> (String, Arc<Mutex<Vec<String>>>) {
        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_h = calls.clone();
        let handler = move |Path(name): Path<String>,
                             Json(args): Json<serde_json::Value>| {
            let calls = calls_h.clone();
            async move {
                let bot = args.get("bot_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                let target = args.get("target_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                let id = if bot != 0 { bot } else { target };
                calls.lock().unwrap().push(format!("{name}:{id}"));

                if name == "obs.lfg_pending" {
                    // Return empty pending list (drain semantics).
                    return Json(
                        serde_json::json!({ "ok": true, "result": { "pending": [] } }),
                    );
                }
                if name == "obs.list_bot_population" {
                    return Json(serde_json::json!({
                        "ok": true,
                        "result": { "bots": [
                            { "bot_guid": 10 }, { "bot_guid": 20 },
                            { "bot_guid": 30 }, { "bot_guid": 40 },
                        ]}
                    }));
                }
                if name == "obs.get_state" {
                    let tgt = args.get("target_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                    calls.lock().unwrap().push(format!("obs.get_state:{tgt}"));
                    return Json(serde_json::json!({
                        "ok": true,
                        "result": {
                            "self": { "race": "human", "is_in_combat": false },
                            "social": { "in_group": false }
                        }
                    }));
                }
                Json(serde_json::json!({ "ok": true, "result": {} }))
            }
        };
        let app = Router::new().route("/v1/tools/:name", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (format!("http://{addr}"), calls)
    }

    // Mock that returns one real-player intent from obs.lfg_pending on the first call,
    // then empty on subsequent calls (drain semantics).
    async fn spawn_lfg_pending_mock(
        real_guid: u64,
        faction_race: &'static str,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_h = calls.clone();
        let drained = Arc::new(Mutex::new(false));

        let handler = move |Path(name): Path<String>,
                             Json(args): Json<serde_json::Value>| {
            let calls = calls_h.clone();
            let drained = drained.clone();
            async move {
                let bot = args.get("bot_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                calls.lock().unwrap().push(format!("{name}:{bot}"));

                if name == "obs.lfg_pending" {
                    let already = {
                        let mut g = drained.lock().unwrap();
                        let v = *g;
                        *g = true;
                        v
                    };
                    let pending = if already {
                        serde_json::json!([])
                    } else {
                        serde_json::json!([{
                            "guid": real_guid,
                            "team_id": 0u32,
                            "roles": 2u32,
                            "dungeon_ids": [4u32],
                            "comment": "",
                            "is_bot": false
                        }])
                    };
                    return Json(serde_json::json!({
                        "ok": true,
                        "result": { "pending": pending }
                    }));
                }
                if name == "obs.list_bot_population" {
                    return Json(serde_json::json!({
                        "ok": true,
                        "result": { "bots": [
                            { "bot_guid": 10u64 }, { "bot_guid": 20u64 },
                            { "bot_guid": 30u64 }, { "bot_guid": 40u64 },
                        ]}
                    }));
                }
                if name == "obs.get_state" {
                    let tgt = args.get("target_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                    calls.lock().unwrap().push(format!("obs.get_state:{tgt}"));
                    return Json(serde_json::json!({
                        "ok": true,
                        "result": {
                            "self": { "race": faction_race, "is_in_combat": false },
                            "social": { "in_group": false }
                        }
                    }));
                }
                Json(serde_json::json!({ "ok": true, "result": {} }))
            }
        };
        let app = Router::new().route("/v1/tools/:name", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (format!("http://{addr}"), calls)
    }

    fn cfg(base: String) -> Config {
        Config {
            listen_addr: "127.0.0.1:0".into(),
            harness_base_url: base.clone(),
            harness_bearer: "tok".into(),
            tick_secs: 1,
            dungeon: Dungeon { id: 4, map_id: 389, x: 3.81, y: -14.82, z: -17.84, o: 4.39 },
            enabled: true, // tests exercise the live path; default=false only matters for slice-host
        }
    }

    fn cfg_disabled(base: String) -> Config {
        Config { enabled: false, ..cfg(base) }
    }

    /// Like `spawn_ok_recording_mock` but the mock returns a 422 executor failure
    /// for (tool=`fail_tool`, target_guid=`fail_target`) — allowing the tick loop's
    /// eviction logic to be exercised. All other calls succeed.
    async fn spawn_ok_mock_with_fail(
        fail_tool: &'static str,
        fail_target: u64,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_h = calls.clone();
        let handler = move |Path(name): Path<String>, Json(args): Json<serde_json::Value>| {
            let calls = calls_h.clone();
            async move {
                let bot = args.get("bot_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                let target = args.get("target_guid").and_then(|v| v.as_u64());
                let id = if bot != 0 { bot } else { target.unwrap_or(0) };
                calls.lock().unwrap().push(format!("{name}:{id}"));

                if name == "obs.lfg_pending" {
                    return Json(
                        serde_json::json!({ "ok": true, "result": { "pending": [] } }),
                    );
                }

                // Reject the specific (tool, target) we want to fail.
                if name == fail_tool && (bot == fail_target || target == Some(fail_target)) {
                    return Json(serde_json::json!({
                        "ok": false,
                        "error": "executor_failed",
                        "detail": "simulated failure (poison pill)",
                    }));
                }

                Json(serde_json::json!({ "ok": true, "result": {} }))
            }
        };
        let app = Router::new().route("/v1/tools/:name", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (format!("http://{addr}"), calls)
    }

    // T5 (existing behaviour): one tick pass forms the matchable bot-only group,
    // removes its members from the queue, and leaves unmatched entries behind.
    #[tokio::test]
    async fn one_tick_removes_matched_and_keeps_unmatched() {
        let (base, _) = spawn_ok_recording_mock().await;
        let state = Arc::new(AppState { queue: crate::queue::Queue::new(), pending_placements: crate::api::PendingPlacements::new() });
        for (g, r) in [
            (1, Role::Tank),
            (2, Role::Healer),
            (3, Role::Dps),
            (4, Role::Dps),
            (5, Role::Dps),
            (6, Role::Healer),
            (7, Role::Dps),
        ] {
            state.queue.upsert(QueueEntry {
                guid: g,
                role: r,
                dungeon_id: 4,
                faction: Faction::Alliance,
                is_real_player: false,
            });
        }
        assert_eq!(state.queue.len(), 7);

        let h = crate::harness::Harness::new(base.clone(), "tok".into());
        let conf = cfg(base);
        let handle = {
            let state = state.clone();
            tokio::spawn(async move { run(state, h, conf).await })
        };
        tokio::time::sleep(StdDuration::from_millis(150)).await;
        handle.abort();

        let remaining: Vec<u64> = state.queue.snapshot().iter().map(|e| e.guid).collect();
        assert_eq!(state.queue.len(), 2, "only the 2 unmatched should remain: {remaining:?}");
        assert!(state.queue.snapshot().iter().all(|e| e.guid == 6 || e.guid == 7));
    }

    // T6 (new): a real-player intent injected via obs.lfg_pending is drained into
    // the queue, bot-fill runs, lfg.form_group is called, and the real player is
    // removed from the queue after a successful form.
    #[tokio::test]
    async fn tick_drains_lfg_pending_and_forms_real_player_group() {
        let real_guid = 99u64;
        let (base, calls) = spawn_lfg_pending_mock(real_guid, "human").await;
        let state = Arc::new(AppState { queue: crate::queue::Queue::new(), pending_placements: crate::api::PendingPlacements::new() });

        let h = crate::harness::Harness::new(base.clone(), "tok".into());
        let conf = cfg(base);
        let handle = {
            let state = state.clone();
            tokio::spawn(async move { run(state, h, conf).await })
        };
        // Two tick intervals: first tick drains + forms; second tick sees empty pending.
        tokio::time::sleep(StdDuration::from_millis(250)).await;
        handle.abort();

        let seq = calls.lock().unwrap().clone();
        // lfg.form_group must have been called.
        assert!(
            seq.iter().any(|c| c.starts_with("lfg.form_group")),
            "lfg.form_group must be called for real-player group: {seq:?}"
        );
        // Real player must be removed from queue after successful form.
        assert!(
            state.queue.snapshot().iter().all(|e| e.guid != real_guid),
            "real player must be removed from queue after form: {:?}",
            state.queue.snapshot()
        );
    }

    // T7 (new): when obs.lfg_pending fails, the tick logs and continues with the
    // existing queue (does not panic or skip the bot-only matching step).
    #[tokio::test]
    async fn tick_continues_on_lfg_pending_failure() {
        // Mock where obs.lfg_pending returns ok:false.
        async fn spawn_fail_pending_mock() -> String {
            let app = Router::new().route(
                "/v1/tools/:name",
                post(|Path(name): Path<String>,
                      Json(_): Json<serde_json::Value>| async move {
                    if name == "obs.lfg_pending" {
                        return Json(serde_json::json!({
                            "ok": false,
                            "error": "not_implemented",
                            "detail": "stub not live yet"
                        }));
                    }
                    Json(serde_json::json!({ "ok": true, "result": {} }))
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
            format!("http://{addr}")
        }

        let base = spawn_fail_pending_mock().await;
        let state = Arc::new(AppState { queue: crate::queue::Queue::new(), pending_placements: crate::api::PendingPlacements::new() });
        // Pre-populate a full bot-only group so we can verify the bot path still runs.
        for (g, r) in [(1, Role::Tank), (2, Role::Healer), (3, Role::Dps), (4, Role::Dps), (5, Role::Dps)] {
            state.queue.upsert(QueueEntry {
                guid: g,
                role: r,
                dungeon_id: 4,
                faction: Faction::Alliance,
                is_real_player: false,
            });
        }

        let h = crate::harness::Harness::new(base.clone(), "tok".into());
        let conf = cfg(base);
        let handle = {
            let state = state.clone();
            tokio::spawn(async move { run(state, h, conf).await })
        };
        tokio::time::sleep(StdDuration::from_millis(150)).await;
        handle.abort();

        // The bot-only group should have formed even though lfg_pending failed.
        assert_eq!(state.queue.len(), 0, "bot-only group must still form despite pending failure");
    }

    // T8 (new): when LFG_ENABLED=false (cfg.enabled=false), the tick loop is inert —
    // no mutating matchmaking actions are taken. A fully-matchable queue of 5 bots
    // must remain intact after multiple ticks.
    #[tokio::test]
    async fn inert_mode_does_not_form_groups() {
        let (base, calls) = spawn_ok_recording_mock().await;
        let state = Arc::new(AppState { queue: crate::queue::Queue::new(), pending_placements: crate::api::PendingPlacements::new() });
        // Pre-populate a full bot-only group.
        for (g, r) in [
            (1, Role::Tank),
            (2, Role::Healer),
            (3, Role::Dps),
            (4, Role::Dps),
            (5, Role::Dps),
        ] {
            state.queue.upsert(QueueEntry {
                guid: g,
                role: r,
                dungeon_id: 4,
                faction: Faction::Alliance,
                is_real_player: false,
            });
        }

        let h = crate::harness::Harness::new(base.clone(), "tok".into());
        let conf = cfg_disabled(base);
        let handle = {
            let state = state.clone();
            tokio::spawn(async move { run(state, h, conf).await })
        };
        // Let several tick intervals pass.
        tokio::time::sleep(StdDuration::from_millis(300)).await;
        handle.abort();

        // Queue must be untouched — inert mode skips Steps 2 & 3 entirely.
        assert_eq!(
            state.queue.len(),
            5,
            "inert mode must not remove entries from the queue"
        );
        // No mutating harness calls should have been made (invite, enter_instance, form_group).
        let seq = calls.lock().unwrap().clone();
        let mutating: Vec<_> = seq
            .iter()
            .filter(|c| {
                c.starts_with("bot.invite_to_group")
                    || c.starts_with("bot.enter_instance")
                    || c.starts_with("lfg.form_group")
            })
            .collect();
        assert!(
            mutating.is_empty(),
            "inert mode must not call mutating harness tools: {mutating:?}"
        );
    }

    // T9: when a bot-only match fails because one invite is rejected (poison-pill),
    // the tick loop must evict ONLY that bot and re-queue the innocent members.
    // Queue before: 1T 2H 3D 4D 5D (all bot-only, all Alliance).
    // Bot 3 (first DPS in sorted order) is the poison pill — its invite always fails.
    // After one tick:
    //   - guid 3 must be absent from the queue (evicted).
    //   - All other members (1,2,4,5) must be back in the queue (innocents re-queued).
    #[tokio::test]
    async fn bot_match_fail_evicts_poison_pill_and_requeues_innocents() {
        // Bot guid 3 (DPS) is the poison pill: invite_to_group targeting it fails.
        // The mock matches `target_guid == 3` for the invite call.
        let (base, _calls) =
            spawn_ok_mock_with_fail("bot.invite_to_group", 3).await;

        let state = Arc::new(AppState { queue: crate::queue::Queue::new(), pending_placements: crate::api::PendingPlacements::new() });
        for (g, r) in [
            (1, Role::Tank),
            (2, Role::Healer),
            (3, Role::Dps),
            (4, Role::Dps),
            (5, Role::Dps),
        ] {
            state.queue.upsert(QueueEntry {
                guid: g,
                role: r,
                dungeon_id: 4,
                faction: Faction::Alliance,
                is_real_player: false,
            });
        }

        let h = crate::harness::Harness::new(base.clone(), "tok".into());
        let conf = cfg(base);
        let handle = {
            let state = state.clone();
            tokio::spawn(async move { run(state, h, conf).await })
        };
        // One tick interval is sufficient — the tick loop fires immediately.
        tokio::time::sleep(StdDuration::from_millis(150)).await;
        handle.abort();

        let remaining: Vec<u64> = {
            let mut v: Vec<u64> = state.queue.snapshot().iter().map(|e| e.guid).collect();
            v.sort();
            v
        };
        // Poison-pill (guid=3) must be gone.
        assert!(
            !remaining.contains(&3),
            "poison-pill bot 3 must be evicted from the queue: {remaining:?}"
        );
        // Innocent members (1, 2, 4, 5) must be re-queued.
        for innocent in [1u64, 2, 4, 5] {
            assert!(
                remaining.contains(&innocent),
                "innocent bot {innocent} must be back in the queue: {remaining:?}"
            );
        }
        assert_eq!(
            remaining.len(),
            4,
            "exactly 4 innocents remain (poison-pill evicted): {remaining:?}"
        );
    }

    // T10: progress guarantee — two ticks where the first fails on a poison-pill and
    // the second forms cleanly. Demonstrates that the queue never deadlocks.
    //
    // Tick 1 queue: 1T 2H 3D(poison) 4D 5D  →  fail on guid 3 invite
    //   After tick 1: queue = {1T, 2H, 4D, 5D} (guid 3 evicted)
    //   Not enough for a 1T/1H/3D match — queue len = 4, Step 3 skips.
    // Tick 2: we manually add guid 6 (DPS) to complete a new matchable set.
    //   Queue becomes {1T, 2H, 4D, 5D, 6D} — matcher finds [1,2,4,5,6], all invites succeed.
    //   After tick 2: queue empty (group formed).
    //
    // We drive this by: pre-populate 5 bots (guid 3 is poison), let one tick run,
    // add guid 6, let another tick run, assert queue is empty.
    #[tokio::test]
    async fn two_tick_progress_after_poison_pill_eviction() {
        let (base, _calls) =
            spawn_ok_mock_with_fail("bot.invite_to_group", 3).await;

        let state = Arc::new(AppState { queue: crate::queue::Queue::new(), pending_placements: crate::api::PendingPlacements::new() });
        for (g, r) in [
            (1, Role::Tank),
            (2, Role::Healer),
            (3, Role::Dps),
            (4, Role::Dps),
            (5, Role::Dps),
        ] {
            state.queue.upsert(QueueEntry {
                guid: g,
                role: r,
                dungeon_id: 4,
                faction: Faction::Alliance,
                is_real_player: false,
            });
        }

        let h = crate::harness::Harness::new(base.clone(), "tok".into());
        let conf = cfg(base);
        let handle = {
            let state = state.clone();
            tokio::spawn(async move { run(state, h, conf).await })
        };

        // Let tick 1 fire and settle (poison pill evicted, 4 innocents re-queued).
        tokio::time::sleep(StdDuration::from_millis(150)).await;

        // Inject the 5th DPS to complete a new matchable set.
        state.queue.upsert(QueueEntry {
            guid: 6,
            role: Role::Dps,
            dungeon_id: 4,
            faction: Faction::Alliance,
            is_real_player: false,
        });

        // Let tick 2 fire — the clean set {1T,2H,4D,5D,6D} should form.
        tokio::time::sleep(StdDuration::from_millis(1200)).await;
        handle.abort();

        let remaining: Vec<u64> = state.queue.snapshot().iter().map(|e| e.guid).collect();
        assert_eq!(
            remaining.len(),
            0,
            "queue must be empty after second tick forms the clean group: {remaining:?}"
        );
    }

    // T11: placement failure on tick 1 records a pending placement; the following
    // tick where enter_instance now succeeds clears it from pending_placements.
    //
    // Setup: 5 bots form a group (all invites succeed), but bot.enter_instance for
    // guid 5 fails on the FIRST call and succeeds on all subsequent calls.
    // After tick 1: group formed, placed=4, pending_placements has guid 5.
    // After tick 2: enter_instance for guid 5 succeeds → pending_placements is empty.
    #[tokio::test]
    async fn placement_failure_records_pending_then_clears_on_retry() {
        // Mock that fails bot.enter_instance for guid 5 on the first call only.
        let enter_fail_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let calls_h = calls.clone();
            let fail_count = enter_fail_count.clone();
            let handler = move |Path(name): Path<String>, Json(args): Json<serde_json::Value>| {
                let calls = calls_h.clone();
                let fail_count = fail_count.clone();
                async move {
                    let bot = args.get("bot_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                    let target = args.get("target_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                    let id = if bot != 0 { bot } else { target };
                    calls.lock().unwrap().push(format!("{name}:{id}"));

                    if name == "obs.lfg_pending" {
                        return Json(
                            serde_json::json!({ "ok": true, "result": { "pending": [] } }),
                        );
                    }

                    // Fail bot.enter_instance for guid 5 on its first invocation.
                    if name == "bot.enter_instance" && bot == 5 {
                        let mut count = fail_count.lock().unwrap();
                        if *count == 0 {
                            *count += 1;
                            return Json(serde_json::json!({
                                "ok": false,
                                "error": "executor_failed",
                                "detail": "TeleportTo returned false",
                            }));
                        }
                    }
                    Json(serde_json::json!({ "ok": true, "result": {} }))
                }
            };
            let app = Router::new().route("/v1/tools/:name", post(handler));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let base = format!("http://{addr}");
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

            let state = Arc::new(AppState {
                queue: crate::queue::Queue::new(),
                pending_placements: crate::api::PendingPlacements::new(),
            });
            for (g, r) in [
                (1, Role::Tank),
                (2, Role::Healer),
                (3, Role::Dps),
                (4, Role::Dps),
                (5, Role::Dps),
            ] {
                state.queue.upsert(QueueEntry {
                    guid: g,
                    role: r,
                    dungeon_id: 4,
                    faction: Faction::Alliance,
                    is_real_player: false,
                });
            }

            let h = crate::harness::Harness::new(base.clone(), "tok".into());
            let conf = cfg(base);
            let handle = {
                let state = state.clone();
                tokio::spawn(async move { run(state, h, conf).await })
            };

            // Tick 1: group forms, enter_instance for guid 5 fails, recorded as pending.
            tokio::time::sleep(StdDuration::from_millis(150)).await;
            assert_eq!(
                state.pending_placements.len(),
                1,
                "guid 5 must be in pending_placements after tick 1"
            );
            let snap = state.pending_placements.snapshot();
            assert_eq!(snap[0].guid, 5, "the pending guid must be 5: {:?}", snap);
            assert_eq!(snap[0].attempts, 1, "attempts starts at 1 (initial attempt)");

            // Tick 2: re-placement pass runs, enter_instance for guid 5 now succeeds.
            tokio::time::sleep(StdDuration::from_millis(1200)).await;
            handle.abort();

            assert_eq!(
                state.pending_placements.len(),
                0,
                "pending_placements must be empty after successful retry"
            );
        }
    }

    // T12: a persistently failing placement is dropped after MAX_PLACEMENT_ATTEMPTS.
    // The pending store must not grow indefinitely.
    //
    // Setup: 5 bots form (all invites ok), but bot.enter_instance for guid 5 always
    // fails. After MAX_PLACEMENT_ATTEMPTS ticks the entry must be gone from pending.
    #[tokio::test]
    async fn persistent_placement_failure_dropped_after_max_attempts() {
        use super::MAX_PLACEMENT_ATTEMPTS;

        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let calls_h = calls.clone();
            let handler = move |Path(name): Path<String>, Json(args): Json<serde_json::Value>| {
                let calls = calls_h.clone();
                async move {
                    let bot = args.get("bot_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                    let target = args.get("target_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                    let id = if bot != 0 { bot } else { target };
                    calls.lock().unwrap().push(format!("{name}:{id}"));

                    if name == "obs.lfg_pending" {
                        return Json(
                            serde_json::json!({ "ok": true, "result": { "pending": [] } }),
                        );
                    }

                    // Always fail bot.enter_instance for guid 5.
                    if name == "bot.enter_instance" && bot == 5 {
                        return Json(serde_json::json!({
                            "ok": false,
                            "error": "executor_failed",
                            "detail": "TeleportTo returned false",
                        }));
                    }
                    Json(serde_json::json!({ "ok": true, "result": {} }))
                }
            };
            let app = Router::new().route("/v1/tools/:name", post(handler));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            // Use tick_secs=1 so that MAX_PLACEMENT_ATTEMPTS ticks pass in a short wall time.
            let base = format!("http://{addr}");
            tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

            let state = Arc::new(AppState {
                queue: crate::queue::Queue::new(),
                pending_placements: crate::api::PendingPlacements::new(),
            });
            for (g, r) in [
                (1, Role::Tank),
                (2, Role::Healer),
                (3, Role::Dps),
                (4, Role::Dps),
                (5, Role::Dps),
            ] {
                state.queue.upsert(QueueEntry {
                    guid: g,
                    role: r,
                    dungeon_id: 4,
                    faction: Faction::Alliance,
                    is_real_player: false,
                });
            }

            // tick_secs=1 so MAX_PLACEMENT_ATTEMPTS ticks pass within ~6s.
            let mut conf = cfg(base.clone());
            conf.tick_secs = 1;
            let h = crate::harness::Harness::new(base.clone(), "tok".into());
            let handle = {
                let state = state.clone();
                tokio::spawn(async move { run(state, h, conf).await })
            };

            // Wait enough time for the group to form on tick 1 and then for
            // MAX_PLACEMENT_ATTEMPTS total ticks to pass (each tick = 1s).
            // Add a comfortable buffer so the last give-up tick definitely completes.
            let wait_ms = (MAX_PLACEMENT_ATTEMPTS as u64 + 2) * 1200;
            tokio::time::sleep(StdDuration::from_millis(wait_ms)).await;
            handle.abort();

            assert_eq!(
                state.pending_placements.len(),
                0,
                "persistent failure must be dropped after {} attempts; store must be empty",
                MAX_PLACEMENT_ATTEMPTS
            );
        }
    }

    // T13: a clean group (all members placed) records nothing in pending_placements.
    #[tokio::test]
    async fn clean_group_records_nothing_pending() {
        let (base, _calls) = spawn_ok_recording_mock().await;
        let state = Arc::new(AppState {
            queue: crate::queue::Queue::new(),
            pending_placements: crate::api::PendingPlacements::new(),
        });
        for (g, r) in [
            (1, Role::Tank),
            (2, Role::Healer),
            (3, Role::Dps),
            (4, Role::Dps),
            (5, Role::Dps),
        ] {
            state.queue.upsert(QueueEntry {
                guid: g,
                role: r,
                dungeon_id: 4,
                faction: Faction::Alliance,
                is_real_player: false,
            });
        }

        let h = crate::harness::Harness::new(base.clone(), "tok".into());
        let conf = cfg(base);
        let handle = {
            let state = state.clone();
            tokio::spawn(async move { run(state, h, conf).await })
        };
        tokio::time::sleep(StdDuration::from_millis(150)).await;
        handle.abort();

        assert_eq!(
            state.pending_placements.len(),
            0,
            "clean group (all placed) must record nothing in pending_placements"
        );
    }
}
