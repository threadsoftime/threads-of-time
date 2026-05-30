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
    let mut note = String::from("formed+placed");
    for m in &members {
        match h.enter_instance_direct(*m, dungeon.map_id, dungeon.x, dungeon.y, dungeon.z, dungeon.o).await {
            Ok(_) => placed += 1,
            Err(e) => note = format!("place {m} failed: {e}"),
        }
    }

    FormResult { dungeon_id: p.dungeon_id, leader, members, formed: true, placed, note }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, routing::post, Json, Router};
    use std::sync::{Arc, Mutex};

    // Mock harness that records the ordered sequence of tool calls.
    async fn spawn_recording_mock() -> (String, Arc<Mutex<Vec<String>>>) {
        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_h = calls.clone();
        let app = Router::new().route(
            "/v1/tools/:name",
            post(move |Path(name): Path<String>, Json(args): Json<serde_json::Value>| {
                let calls = calls_h.clone();
                async move {
                    let who = args.get("bot_guid").and_then(|v| v.as_u64()).unwrap_or(0);
                    calls.lock().unwrap().push(format!("{name}:{who}"));
                    Json(serde_json::json!({ "ok": true, "result": {} }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        (format!("http://{addr}"), calls)
    }

    fn dungeon() -> Dungeon {
        Dungeon { id: 36, map_id: 389, x: 1.0, y: 2.0, z: 3.0, o: 0.0 }
    }

    #[tokio::test]
    async fn fulfill_invites_accepts_then_places_all() {
        let (base, calls) = spawn_recording_mock().await;
        let h = Harness::new(base, "tok".into());
        let p = MatchProposal { dungeon_id: 36, tank: 1, healer: 2, dps: vec![3, 4, 5] };

        let res = fulfill(&h, &p, &dungeon()).await;
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
}
