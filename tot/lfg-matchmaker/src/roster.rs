use crate::config::Dungeon;
use crate::harness::{Harness, HarnessError};
use crate::types::Faction;
use serde_json::Value;

/// Maximum number of `obs.get_state` probes per `select_fill_bots` call.
/// Avoids flooding the harness when the bot population is large.
const MAX_PROBES: usize = 25;

/// Select `needed` bot guids to fill the remaining slots of a group.
///
/// Candidates are drawn from `obs.list_bot_population` (only bots on the same
/// map as the dungeon entry OR any map — we don't filter by map here, just
/// by faction/group/combat eligibility). Up to `MAX_PROBES` are probed with
/// `obs.get_state`; the rest are skipped to bound harness round-trips.
///
/// Eligibility rules (checked via `obs.get_state`):
/// - Same faction as `faction` (decoded from `result.self.race`).
/// - `result.social.in_group == false`.
/// - `result.self.is_in_combat == false`.
///
/// Bots in `exclude` (typically the real player's guid) are never selected.
/// If fewer than `needed` eligible bots are found, an error is returned (no
/// silent partial fill — the caller decides how to handle short pools).
pub async fn select_fill_bots(
    harness: &Harness,
    faction: Faction,
    _dungeon: &Dungeon,
    exclude: &[u64],
    needed: usize,
) -> Result<Vec<u64>, HarnessError> {
    if needed == 0 {
        return Ok(Vec::new());
    }

    let pop = harness.list_bot_population().await?;
    let bots_arr = pop
        .get("bots")
        .and_then(Value::as_array)
        .ok_or_else(|| HarnessError::Shape("obs.list_bot_population: missing bots array".into()))?;

    // Collect candidate guids, excluding the provided list.
    let exclude_set: std::collections::HashSet<u64> = exclude.iter().copied().collect();
    let candidates: Vec<u64> = bots_arr
        .iter()
        .filter_map(|b| b.get("bot_guid").and_then(Value::as_u64))
        .filter(|g| !exclude_set.contains(g))
        .collect();

    let mut selected: Vec<u64> = Vec::with_capacity(needed);

    for (probed, candidate) in candidates.into_iter().enumerate() {
        if selected.len() >= needed {
            break;
        }
        if probed >= MAX_PROBES {
            eprintln!(
                "[roster] probe cap ({MAX_PROBES}) reached; pool may be short \
                 (needed={needed}, found={})",
                selected.len()
            );
            break;
        }

        let state = match harness.get_state(candidate).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[roster] get_state {candidate} failed: {e}; skipping");
                continue;
            }
        };

        if is_eligible(&state, faction) {
            selected.push(candidate);
        }
    }

    if selected.len() < needed {
        eprintln!(
            "[roster] short pool: needed={needed} eligible={} (faction={faction:?})",
            selected.len()
        );
        return Err(HarnessError::Shape(format!(
            "roster: only {} eligible bots found, needed {needed}",
            selected.len()
        )));
    }

    Ok(selected)
}

/// Check eligibility from a raw `obs.get_state` result value.
fn is_eligible(state: &Value, want_faction: Faction) -> bool {
    let self_node = match state.get("self") {
        Some(v) => v,
        None => return false,
    };
    let race = self_node.get("race").and_then(Value::as_str).unwrap_or("");
    let bot_faction = Faction::from_race(race);
    if bot_faction != want_faction {
        return false;
    }
    let in_group = state
        .get("social")
        .and_then(|s| s.get("in_group"))
        .and_then(Value::as_bool)
        .unwrap_or(true); // default to true (busy) when missing
    if in_group {
        return false;
    }
    let in_combat = self_node.get("is_in_combat").and_then(Value::as_bool).unwrap_or(true);
    !in_combat
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, routing::post, Json, Router};

    fn dungeon() -> Dungeon {
        Dungeon { id: 4, map_id: 389, x: 3.81, y: -14.82, z: -17.84, o: 4.39 }
    }

    /// Spawn a mock harness that:
    /// - `obs.list_bot_population` returns bots with guids from `bot_guids`.
    /// - `obs.get_state` returns a state shaped for `faction` and eligible
    ///   (in_group=false, is_in_combat=false), UNLESS the guid is in `busy_guids`.
    async fn spawn_roster_mock(
        bot_guids: Vec<u64>,
        faction: &'static str,
        busy_guids: Vec<u64>,
    ) -> String {
        let bots_json: Vec<serde_json::Value> =
            bot_guids.iter().map(|g| serde_json::json!({ "bot_guid": g })).collect();
        let bots_json = serde_json::Value::Array(bots_json);
        let busy: std::collections::HashSet<u64> = busy_guids.into_iter().collect();

        let handler = move |Path(name): Path<String>,
                             Json(args): Json<serde_json::Value>| {
            let bots_json = bots_json.clone();
            let busy = busy.clone();
            async move {
                if name == "obs.list_bot_population" {
                    return Json(
                        serde_json::json!({ "ok": true, "result": { "bots": bots_json } }),
                    );
                }
                if name == "obs.get_state" {
                    let guid = args.get("target_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                    let in_group = busy.contains(&guid);
                    return Json(serde_json::json!({
                        "ok": true,
                        "result": {
                            "self": { "race": faction, "is_in_combat": false },
                            "social": { "in_group": in_group }
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
        format!("http://{addr}")
    }

    // Basic fill: 4 eligible Alliance bots available, need 4 → returns all 4.
    #[tokio::test]
    async fn selects_needed_eligible_bots() {
        let base = spawn_roster_mock(vec![10, 20, 30, 40], "human", vec![]).await;
        let h = Harness::new(base, "tok".into());
        let selected = select_fill_bots(&h, Faction::Alliance, &dungeon(), &[], 4).await.unwrap();
        assert_eq!(selected.len(), 4);
    }

    // Exclude list: guid 10 excluded → only 20,30,40,50 candidates, need 4.
    #[tokio::test]
    async fn respects_exclude_list() {
        let base = spawn_roster_mock(vec![10, 20, 30, 40, 50], "human", vec![]).await;
        let h = Harness::new(base, "tok".into());
        let selected =
            select_fill_bots(&h, Faction::Alliance, &dungeon(), &[10], 4).await.unwrap();
        assert_eq!(selected.len(), 4);
        assert!(!selected.contains(&10), "excluded guid must not appear");
    }

    // Busy bots (in_group=true) are skipped — need 2, 2 of 4 are busy → finds 2.
    #[tokio::test]
    async fn skips_busy_bots() {
        // bots 10 and 20 are busy; 30 and 40 are free.
        let base = spawn_roster_mock(vec![10, 20, 30, 40], "human", vec![10, 20]).await;
        let h = Harness::new(base, "tok".into());
        let selected =
            select_fill_bots(&h, Faction::Alliance, &dungeon(), &[], 2).await.unwrap();
        assert_eq!(selected.len(), 2);
        assert!(!selected.contains(&10));
        assert!(!selected.contains(&20));
    }

    // Short pool: only 2 eligible bots, need 4 → error.
    #[tokio::test]
    async fn short_pool_returns_error() {
        let base = spawn_roster_mock(vec![10, 20], "human", vec![]).await;
        let h = Harness::new(base, "tok".into());
        let err =
            select_fill_bots(&h, Faction::Alliance, &dungeon(), &[], 4).await.unwrap_err();
        match err {
            HarnessError::Shape(msg) => assert!(msg.contains("roster"), "msg: {msg}"),
            other => panic!("expected Shape error, got {other:?}"),
        }
    }

    // Faction mismatch: all bots are Horde (orc), requesting Alliance fill → error.
    #[tokio::test]
    async fn faction_mismatch_yields_error() {
        let base = spawn_roster_mock(vec![10, 20, 30, 40], "orc", vec![]).await;
        let h = Harness::new(base, "tok".into());
        let err =
            select_fill_bots(&h, Faction::Alliance, &dungeon(), &[], 1).await.unwrap_err();
        match err {
            HarnessError::Shape(msg) => assert!(msg.contains("roster"), "msg: {msg}"),
            other => panic!("expected Shape error, got {other:?}"),
        }
    }

    // Need 0 → returns empty without any harness calls.
    #[tokio::test]
    async fn need_zero_returns_empty() {
        // This mock would panic if any tool is called (unreachable path).
        let base = spawn_roster_mock(vec![], "human", vec![]).await;
        let h = Harness::new(base, "tok".into());
        let selected =
            select_fill_bots(&h, Faction::Alliance, &dungeon(), &[], 0).await.unwrap();
        assert!(selected.is_empty());
    }
}
