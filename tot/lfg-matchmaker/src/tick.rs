use crate::api::AppState;
use crate::config::Config;
use crate::harness::Harness;
use crate::matcher::{build_real_player_proposal, find_matches};
use crate::orchestrator::{form_via_lfg, fulfill};
use crate::roster::select_fill_bots;
use crate::types::QueueEntry;
use std::sync::Arc;
use std::time::Duration;

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
                for entry in taken {
                    state.queue.upsert(entry);
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

    // T5 (existing behaviour): one tick pass forms the matchable bot-only group,
    // removes its members from the queue, and leaves unmatched entries behind.
    #[tokio::test]
    async fn one_tick_removes_matched_and_keeps_unmatched() {
        let (base, _) = spawn_ok_recording_mock().await;
        let state = Arc::new(AppState { queue: crate::queue::Queue::new() });
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
        let state = Arc::new(AppState { queue: crate::queue::Queue::new() });

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
        let state = Arc::new(AppState { queue: crate::queue::Queue::new() });
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
        let state = Arc::new(AppState { queue: crate::queue::Queue::new() });
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
}
