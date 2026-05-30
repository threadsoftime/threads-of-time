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
/// direct-teleport every member into the dungeon instance. On ANY invite or
/// accept failure, roll back the partially formed group: every member that has
/// already joined (the leader plus each accepted member) is sent
/// `bot.leave_group`, so the orchestrator never leaves an orphan group behind
/// (live-proof Finding 2). Rollback is best-effort — errors from the teardown
/// calls are folded into the note, not surfaced as a failure.
pub async fn fulfill(h: &Harness, p: &MatchProposal, dungeon: &Dungeon) -> FormResult {
    let leader = p.leader();
    let members = p.members();
    let others: Vec<u64> = members.iter().copied().filter(|g| *g != leader).collect();

    // Members currently in the group. The leader is "in" from the first invite
    // it sends (the group is created leader-first); each `m` joins on accept.
    let mut joined: Vec<u64> = vec![leader];

    for m in &others {
        if let Err(e) = h.invite_to_group(leader, *m).await {
            let rb = rollback(h, &joined).await;
            return FormResult {
                dungeon_id: p.dungeon_id, leader, members, formed: false, placed: 0,
                note: format!("invite {leader}->{m} failed: {e}; rolled back {joined:?}{rb}"),
            };
        }
        if let Err(e) = h.accept_invite(*m).await {
            let rb = rollback(h, &joined).await;
            return FormResult {
                dungeon_id: p.dungeon_id, leader, members, formed: false, placed: 0,
                note: format!("accept {m} failed: {e}; rolled back {joined:?}{rb}"),
            };
        }
        joined.push(*m);
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

/// Form a group that contains a real human player via `lfg.form_group`, then
/// place every **bot** member via `bot.enter_instance` (F-S1: TeleportPlayer
/// is unreliable for playerbots). The real human player is placed by
/// `lfg.form_group`'s own TeleportPlayer and must NOT receive an additional
/// `bot.enter_instance` call.
///
/// In Inc-1 every "real player" in a live test is actually a bot stand-in;
/// the caller communicates this via `p.real_player_guid`: guids NOT equal to
/// `real_player_guid` receive `enter_instance`. A truly real human (None or a
/// guid whose identity is not a known bot) is skipped for `enter_instance`.
///
/// On `lfg.form_group` failure: the adapter pre-validates and never partially
/// forms a group, so there is NO orphan to roll back — just return formed:false
/// with the error. Contrast with `fulfill` (bot-only invite/accept path) which
/// DOES roll back.
pub async fn form_via_lfg(h: &Harness, p: &MatchProposal, dungeon: &Dungeon) -> FormResult {
    let leader = p.leader();
    let members = p.members();

    // Build the member list for lfg.form_group with placeholder roles bitmask.
    // We default to DAMAGE (8) for all; a future Stage-3 refinement will carry
    // per-member roles from the intent through to here.
    let members_with_roles: Vec<(u64, u32)> = members.iter().map(|&g| (g, 8u32)).collect();

    match h.form_group(leader, &members_with_roles, dungeon.id).await {
        Err(e) => FormResult {
            dungeon_id: p.dungeon_id,
            leader,
            members,
            formed: false,
            placed: 0,
            note: format!("lfg.form_group failed: {e}"),
        },
        Ok(_) => {
            // Place each bot member. The real human player (real_player_guid)
            // was already placed by lfg.form_group's TeleportPlayer — skip them.
            let real_guid = p.real_player_guid;
            let mut placed = 0usize;
            let mut errors: Vec<String> = Vec::new();

            for &m in &members {
                if Some(m) == real_guid {
                    // True human player — already placed by lfg.form_group.
                    // In Inc-1 bot stand-ins have real_player_guid == their guid;
                    // if you intend a bot stand-in to also be enter_instanced,
                    // set real_player_guid = None for that member.
                    placed += 1; // count the human as placed (by the adapter)
                    continue;
                }
                match h.enter_instance_direct(m, dungeon).await {
                    Ok(_) => placed += 1,
                    Err(e) => errors.push(format!("place {m}: {e}")),
                }
            }

            let note = if errors.is_empty() {
                "formed+placed via lfg.form_group".to_string()
            } else {
                errors.join("; ")
            };
            FormResult { dungeon_id: p.dungeon_id, leader, members, formed: true, placed, note }
        }
    }
}

/// Best-effort teardown of a partially formed group: send `bot.leave_group` to
/// every joined member (leader included). Errors are swallowed (folded into the
/// returned suffix) — the data plane is the source of truth, and a failed
/// leave just means the bot was already ungrouped. Returns a note suffix that
/// is empty on full success or `" (rollback errors: ...)"` otherwise.
async fn rollback(h: &Harness, joined: &[u64]) -> String {
    let mut errs: Vec<String> = Vec::new();
    for g in joined {
        if let Err(e) = h.leave_group(*g).await {
            errs.push(format!("leave {g}: {e}"));
        }
    }
    if errs.is_empty() {
        String::new()
    } else {
        format!(" (rollback errors: {})", errs.join("; "))
    }
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
        Dungeon { id: 4, map_id: 389, x: 3.81, y: -14.82, z: -17.84, o: 4.39 }
    }

    fn proposal() -> MatchProposal {
        MatchProposal {
            dungeon_id: 4,
            tank: 1,
            healer: 2,
            dps: vec![3, 4, 5],
            has_real_player: false,
            real_player_guid: None,
        }
    }

    fn real_player_proposal(real_guid: u64) -> MatchProposal {
        MatchProposal {
            dungeon_id: 4,
            tank: real_guid,
            healer: 2,
            dps: vec![3, 4, 5],
            has_real_player: true,
            real_player_guid: Some(real_guid),
        }
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
    // and the partially formed group (leader 1 + already-joined member 2) is torn down
    // via bot.leave_group (Finding 2 — no orphan group left behind).
    #[tokio::test]
    async fn fulfill_bails_on_invite_failure_and_rolls_back() {
        let (base, calls) = spawn_recording_mock_with_fail(Some(("bot.invite_to_group".into(), 3))).await;
        let h = Harness::new(base, "tok".into());

        let res = fulfill(&h, &proposal(), &dungeon()).await;
        assert!(!res.formed, "group must not form on a failed invite");
        assert_eq!(res.placed, 0, "no placement when form fails");
        assert!(res.note.contains('3'), "note must name the failing bot: {}", res.note);
        assert!(res.note.contains("rolled back"), "note must record the rollback: {}", res.note);

        // Bail at the 2nd invite: invite(2)+accept(2)+invite(3 fails) = 3 calls, then
        // roll back the joined members (leader 1 + member 2) = 2 leave_group calls.
        let seq = calls.lock().unwrap().clone();
        assert_eq!(seq.len(), 5, "3 form calls + 2 rollback leave_group calls: {seq:?}");
        // No placement happened.
        assert!(seq.iter().all(|c| !c.starts_with("bot.enter_instance:")), "no placement: {seq:?}");
        // Rollback tore down the leader and the one already-joined member.
        assert!(seq.contains(&"bot.leave_group:1".to_string()), "leader must be disbanded: {seq:?}");
        assert!(seq.contains(&"bot.leave_group:2".to_string()), "joined member must leave: {seq:?}");
        // The member that never joined (3) is NOT torn down.
        assert!(!seq.contains(&"bot.leave_group:3".to_string()), "un-joined bot must not be torn down: {seq:?}");
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

    // form_via_lfg tests.

    // T3(a): happy path — lfg.form_group succeeds; real player (guid=1) is counted as
    // placed (TeleportPlayer); the 4 bot members each receive enter_instance.
    #[tokio::test]
    async fn form_via_lfg_happy_path_places_bots_skips_real_player() {
        let (base, calls) = spawn_recording_mock().await;
        let h = Harness::new(base, "tok".into());
        let p = real_player_proposal(1);

        let res = form_via_lfg(&h, &p, &dungeon()).await;
        assert!(res.formed, "group must form: {}", res.note);
        // 5 placed: real player (counted by adapter) + 4 enter_instanced bots.
        assert_eq!(res.placed, 5, "note: {}", res.note);

        let seq = calls.lock().unwrap().clone();
        // 1 lfg.form_group call + 4 bot.enter_instance calls (real guid=1 skipped).
        let form_calls: Vec<&String> = seq.iter().filter(|c| c.starts_with("lfg.form_group")).collect();
        let enter_calls: Vec<&String> = seq.iter().filter(|c| c.starts_with("bot.enter_instance")).collect();
        assert_eq!(form_calls.len(), 1, "exactly one lfg.form_group: {seq:?}");
        assert_eq!(enter_calls.len(), 4, "4 enter_instance for bot members: {seq:?}");
        // bot 1 (real player) must NOT get enter_instance
        assert!(
            !seq.iter().any(|c| c == "bot.enter_instance:1"),
            "real player guid 1 must not get enter_instance: {seq:?}"
        );
    }

    // T3(b): lfg.form_group fails → formed:false, no enter_instance calls, no rollback.
    #[tokio::test]
    async fn form_via_lfg_fails_cleanly_on_form_group_failure() {
        let (base, calls) =
            spawn_recording_mock_with_fail(Some(("lfg.form_group".into(), 0))).await;
        let h = Harness::new(base, "tok".into());
        let p = real_player_proposal(1);

        let res = form_via_lfg(&h, &p, &dungeon()).await;
        assert!(!res.formed, "must not be formed when lfg.form_group fails");
        assert_eq!(res.placed, 0);
        assert!(res.note.contains("lfg.form_group failed"), "note: {}", res.note);

        let seq = calls.lock().unwrap().clone();
        assert!(
            seq.iter().all(|c| !c.starts_with("bot.enter_instance")),
            "no enter_instance on form failure: {seq:?}"
        );
        // No leave_group rollback — lfg.form_group pre-validates.
        assert!(
            seq.iter().all(|c| !c.starts_with("bot.leave_group")),
            "no rollback needed for lfg.form_group failure: {seq:?}"
        );
    }

    // T3(c): when real_player_guid is None, all 5 members get enter_instance.
    #[tokio::test]
    async fn form_via_lfg_enter_instances_all_when_no_real_player_guid() {
        let (base, calls) = spawn_recording_mock().await;
        let h = Harness::new(base, "tok".into());
        // Proposal flagged has_real_player=true but real_player_guid=None
        // (bot stand-in where we want all to be enter_instanced).
        let p = MatchProposal {
            dungeon_id: 4,
            tank: 1,
            healer: 2,
            dps: vec![3, 4, 5],
            has_real_player: true,
            real_player_guid: None,
        };

        let res = form_via_lfg(&h, &p, &dungeon()).await;
        assert!(res.formed);
        assert_eq!(res.placed, 5);

        let seq = calls.lock().unwrap().clone();
        let enter_calls: Vec<&String> = seq.iter().filter(|c| c.starts_with("bot.enter_instance")).collect();
        assert_eq!(enter_calls.len(), 5, "all 5 should be enter_instanced: {seq:?}");
    }
}
