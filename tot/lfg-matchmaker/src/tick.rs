use crate::api::AppState;
use crate::config::Config;
use crate::harness::Harness;
use crate::matcher::find_matches;
use crate::orchestrator::fulfill;
use std::sync::Arc;
use std::time::Duration;

pub async fn run(state: Arc<AppState>, harness: Harness, cfg: Config) {
    let mut ticker = tokio::time::interval(Duration::from_secs(cfg.tick_secs.max(1)));
    loop {
        ticker.tick().await;
        let snapshot = state.queue.snapshot();
        if snapshot.len() < 5 {
            continue;
        }
        for p in find_matches(&snapshot) {
            // Take (remove + capture) members BEFORE acting so the next tick can't
            // double-book them.
            let taken = state.queue.take_many(&p.members());
            let res = fulfill(&harness, &p, &cfg.dungeon).await;
            // Re-queue ONLY on a form failure: the group was never created, so the
            // bots are free — drop nothing. On success the group exists; re-queueing
            // would double-book. (Persistently-ineligible bots will retry each tick;
            // acceptable for v1.)
            if !res.formed {
                for entry in taken {
                    state.queue.upsert(entry);
                }
            }
            eprintln!(
                "[match] dungeon={} leader={} formed={} placed={}/{} note={}",
                res.dungeon_id, res.leader, res.formed, res.placed, res.members.len(), res.note
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Dungeon;
    use crate::types::{QueueEntry, Role};
    use axum::{extract::Path, routing::post, Json, Router};
    use std::time::Duration as StdDuration;

    // All-ok mock so a full group forms+places on the first tick pass.
    async fn spawn_ok_mock() -> String {
        let app = Router::new().route(
            "/v1/tools/:name",
            post(|Path(_): Path<String>, Json(_): Json<serde_json::Value>| async {
                Json(serde_json::json!({ "ok": true, "result": {} }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        format!("http://{addr}")
    }

    fn cfg(base: String) -> Config {
        Config {
            listen_addr: "127.0.0.1:0".into(),
            harness_base_url: base,
            harness_bearer: "tok".into(),
            tick_secs: 1, // interval's first tick fires immediately → one pass right away
            dungeon: Dungeon { id: 36, map_id: 389, x: 1.0, y: 2.0, z: 3.0, o: 0.0 },
        }
    }

    // T5: one tick pass forms the matchable group, removes its members from the queue,
    // and leaves unmatched entries behind. We spawn run() (it loops forever) and abort
    // after a short window — the first interval tick is immediate so one pass completes.
    #[tokio::test]
    async fn one_tick_removes_matched_and_keeps_unmatched() {
        let base = spawn_ok_mock().await;
        let state = Arc::new(AppState { queue: crate::queue::Queue::new() });
        // 1 full group (1..=5) + two leftovers that cannot complete a 2nd group.
        for (g, r) in [
            (1, Role::Tank),
            (2, Role::Healer),
            (3, Role::Dps),
            (4, Role::Dps),
            (5, Role::Dps),
            (6, Role::Healer), // extra healer — no tank/3-dps to pair → stranded
            (7, Role::Dps),    // extra dps — stranded
        ] {
            state.queue.upsert(QueueEntry { guid: g, role: r, dungeon_id: 36 });
        }
        assert_eq!(state.queue.len(), 7);

        // run() takes the already-built Harness; from Config it uses only tick_secs +
        // dungeon, so the base_url in `cfg` is unused here (kept consistent anyway).
        let h = crate::harness::Harness::new(base.clone(), "tok".into());
        let conf = cfg(base);
        let handle = {
            let state = state.clone();
            tokio::spawn(async move { run(state, h, conf).await })
        };
        // Let exactly one pass run, then stop the forever-loop.
        tokio::time::sleep(StdDuration::from_millis(150)).await;
        handle.abort();

        // Matched members 1..=5 removed; stranded 6 and 7 survive.
        let remaining: Vec<u64> = state.queue.snapshot().iter().map(|e| e.guid).collect();
        assert_eq!(state.queue.len(), 2, "only the 2 unmatched should remain: {remaining:?}");
        assert!(state.queue.snapshot().iter().all(|e| e.guid == 6 || e.guid == 7));
    }
}
