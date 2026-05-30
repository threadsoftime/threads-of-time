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
            // Remove members BEFORE acting so the next tick can't double-book them.
            // v1: on failure, members are NOT re-queued (they may re-queue themselves);
            // re-queue-on-failure is a documented hardening step.
            state.queue.remove_many(&p.members());
            let res = fulfill(&harness, &p, &cfg.dungeon).await;
            eprintln!(
                "[match] dungeon={} leader={} formed={} placed={}/{} note={}",
                res.dungeon_id, res.leader, res.formed, res.placed, res.members.len(), res.note
            );
        }
    }
}
