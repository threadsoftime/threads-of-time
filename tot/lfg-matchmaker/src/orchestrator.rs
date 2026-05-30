use crate::config::Dungeon;
use crate::harness::Harness;
use crate::types::MatchProposal;

#[derive(Debug, Clone, PartialEq)]
pub struct FormResult {
    pub dungeon_id: u32,
    pub leader: u64,
    pub members: Vec<u64>,
    pub formed: bool,
    pub placed: usize,
    pub note: String,
}

/// Form the group (leader invites each other member, each accepts), then
/// direct-teleport every member into the dungeon instance. Bails out on the
/// first form failure (the data-plane adapters' live guards are the
/// reconciliation point — see plan's reconciliation note).
pub async fn fulfill(h: &Harness, p: &MatchProposal, dungeon: &Dungeon) -> FormResult {
    let leader = p.leader();
    let members = p.members();
    let others: Vec<u64> = members.iter().copied().filter(|g| *g != leader).collect();

    for m in &others {
        if let Err(e) = h.invite_to_group(leader, *m).await {
            return FormResult {
                dungeon_id: p.dungeon_id, leader, members, formed: false, placed: 0,
                note: format!("invite {leader}->{m} failed: {e}"),
            };
        }
        if let Err(e) = h.accept_invite(*m).await {
            return FormResult {
                dungeon_id: p.dungeon_id, leader, members, formed: false, placed: 0,
                note: format!("accept {m} failed: {e}"),
            };
        }
    }

    let mut placed = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for m in &members {
        match h.enter_instance_direct(*m, dungeon).await {
            Ok(_) => placed += 1,
            Err(e) => errors.push(format!("place {m}: {e}")),
        }
    }
    let note = if errors.is_empty() { "formed+placed".to_string() } else { errors.join("; ") };

    FormResult { dungeon_id: p.dungeon_id, leader, members, formed: true, placed, note }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, http::StatusCode, routing::post, Json, Router};
    use std::sync::{Arc, Mutex};

    // A (tool, guid) pair the mock should reject with a 422 (real-wire shape).
    // `guid` is matched against EITHER the call's `bot_guid` or its `target_guid`,
    // so an invite (whose bot_guid is always the leader) can be targeted by the
    // member being invited.
    type FailSpec = Option<(String, u64)>;

    // Mock harness that records the ordered sequence of tool calls and can be told
    // to fail one specific (tool, guid) with a typed 422 executor failure.
    async fn spawn_recording_mock_with_fail(fail: FailSpec) -> (String, Arc<Mutex<Vec<String>>>) {
        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_h = calls.clone();
        let fail = Arc::new(fail);
        let handler = move |Path(name): Path<String>, Json(args): Json<serde_json::Value>| {
            let calls = calls_h.clone();
            let fail = fail.clone();
            async move {
                let bot = args.get("bot_guid").and_then(serde_json::Value::as_u64).unwrap_or(0);
                let target = args.get("target_guid").and_then(serde_json::Value::as_u64);
                // Record by bot_guid (the actor), matching the existing sequence test.
                calls.lock().unwrap().push(format!("{name}:{bot}"));
                if let Some((ftool, fguid)) = fail.as_ref() {
                    if *ftool == name && (*fguid == bot || Some(*fguid) == target) {
                        return (
                            StatusCode::UNPROCESSABLE_ENTITY,
                            Json(serde_json::json!({
                                "ok": false,
                                "error": "executor_failed",
                                "detail": "simulated failure",
                            })),
                        );
                    }
                }
                (StatusCode::OK, Json(serde_json::json!({ "ok": true, "result": {} })))
            }
        };
        let app = Router::new().route("/v1/tools/:name", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (format!("http://{addr}"), calls)
    }

    async fn spawn_recording_mock() -> (String, Arc<Mutex<Vec<String>>>) {
        spawn_recording_mock_with_fail(None).await
    }

    fn dungeon() -> Dungeon {
        Dungeon { id: 36, map_id: 389, x: 1.0, y: 2.0, z: 3.0, o: 0.0 }
    }

    fn proposal() -> MatchProposal {
        MatchProposal { dungeon_id: 36, tank: 1, healer: 2, dps: vec![3, 4, 5] }
    }

    #[tokio::test]
    async fn fulfill_invites_accepts_then_places_all() {
        let (base, calls) = spawn_recording_mock().await;
        let h = Harness::new(base, "tok".into());

        let res = fulfill(&h, &proposal(), &dungeon()).await;
        assert!(res.formed);
        assert_eq!(res.placed, 5);

        let seq = calls.lock().unwrap().clone();
        // 4 invites + 4 accepts (leader excluded) + 5 placements = 13 calls
        assert_eq!(seq.len(), 13);
        // leader (1) invites each of 2,3,4,5; each accepts; then all 5 enter
        assert_eq!(seq[0], "bot.invite_to_group:1");
        assert_eq!(seq[1], "bot.accept_invite:2");
        assert!(seq[8..13].iter().all(|c| c.starts_with("bot.enter_instance:")));
    }

    // T2(a): the 2nd invite fails (leader -> dps 3). The mock matches the failure
    // against the invite's target_guid (3). The group never forms, nothing is placed,
    // and the note names the failing bot.
    #[tokio::test]
    async fn fulfill_bails_on_invite_failure() {
        let (base, calls) = spawn_recording_mock_with_fail(Some(("bot.invite_to_group".into(), 3))).await;
        let h = Harness::new(base, "tok".into());

        let res = fulfill(&h, &proposal(), &dungeon()).await;
        assert!(!res.formed, "group must not form on a failed invite");
        assert_eq!(res.placed, 0, "no placement when form fails");
        assert!(res.note.contains('3'), "note must name the failing bot: {}", res.note);

        // Bail at the 2nd invite: invite(2)+accept(2)+invite(3 fails) = 3 calls; no placement.
        let seq = calls.lock().unwrap().clone();
        assert_eq!(seq.len(), 3, "must bail at the failed 2nd invite: {seq:?}");
        assert!(seq.iter().all(|c| !c.starts_with("bot.enter_instance:")));
    }

    // T2(b): all joins succeed but one enter_instance fails. The group DID form, so
    // formed:true; 4 of 5 placed; note is the joined error list naming the failed bot.
    #[tokio::test]
    async fn fulfill_forms_but_one_placement_fails() {
        let (base, _calls) = spawn_recording_mock_with_fail(Some(("bot.enter_instance".into(), 4))).await;
        let h = Harness::new(base, "tok".into());

        let res = fulfill(&h, &proposal(), &dungeon()).await;
        assert!(res.formed, "group formed; only placement degraded");
        assert_eq!(res.placed, 4, "one of five placements failed");
        assert!(res.note.contains("place 4"), "note must be the joined error list: {}", res.note);
    }
}
