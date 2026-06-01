use crate::types::{Faction, MatchProposal, QueueEntry, Role};
use std::collections::HashMap;

/// Build a `MatchProposal` for a real-player intent, filling the remaining 4
/// slots with same-faction bots from `filler_bots`. The real player takes their
/// declared role; the first bot is assigned the complementary tank/healer role
/// and the rest take DPS (a simplified assignment for Inc-1 — role assignment
/// from `obs.lfg_pending` roles bitmask is a Stage-3 refinement).
///
/// Layout:
/// - real player role == Tank   → real=tank, filler[0]=healer, filler[1..4]=dps
/// - real player role == Healer → real=healer, filler[0]=tank, filler[1..4]=dps
/// - real player role == Dps    → filler[0]=tank, filler[1]=healer, real+filler[2..4]=dps
///   (real player is last in dps to put them adjacent in the member list)
///
/// Requires exactly 4 filler bots; returns None if `filler_bots.len() != 4`.
pub fn build_real_player_proposal(
    real_player: &QueueEntry,
    filler_bots: &[u64],
    dungeon_id: u32,
) -> Option<MatchProposal> {
    if filler_bots.len() != 4 {
        return None;
    }
    let rp = real_player.guid;
    let (tank, healer, dps) = match real_player.role {
        Role::Tank => (rp, filler_bots[0], vec![filler_bots[1], filler_bots[2], filler_bots[3]]),
        Role::Healer => {
            (filler_bots[0], rp, vec![filler_bots[1], filler_bots[2], filler_bots[3]])
        }
        Role::Dps => (
            filler_bots[0],
            filler_bots[1],
            vec![filler_bots[2], filler_bots[3], rp],
        ),
    };
    Some(MatchProposal {
        dungeon_id,
        tank,
        healer,
        dps,
        has_real_player: true,
        real_player_guid: Some(rp),
    })
}

#[derive(Default)]
struct RoleBuckets {
    tanks: Vec<u64>,
    healers: Vec<u64>,
    dps: Vec<u64>,
}

/// Faction as a sort-stable primitive so bucket keys order deterministically.
fn faction_ord(f: Faction) -> u8 {
    match f {
        Faction::Alliance => 0,
        Faction::Horde => 1,
    }
}

/// Greedy, deterministic role-balanced matcher.
/// Forms as many 1-tank / 1-healer / 3-dps groups per `(dungeon_id, faction)`
/// as the queue allows — a group is always same-dungeon AND same-faction, so
/// the data plane never sees a cross-faction invite. Pure — no I/O, no clock,
/// no randomness. Output is sorted by (dungeon id, faction) then by member
/// guids for reproducibility.
pub fn find_matches(queue: &[QueueEntry]) -> Vec<MatchProposal> {
    // bucket guids by (dungeon, faction), then by role
    let mut by_bucket: HashMap<(u32, Faction), RoleBuckets> = HashMap::new();
    for e in queue {
        let b = by_bucket.entry((e.dungeon_id, e.faction)).or_default();
        match e.role {
            Role::Tank => b.tanks.push(e.guid),
            Role::Healer => b.healers.push(e.guid),
            Role::Dps => b.dps.push(e.guid),
        }
    }

    // Sort keys on primitives only: (dungeon_id, faction-as-u8).
    let mut keys: Vec<(u32, Faction)> = by_bucket.keys().copied().collect();
    keys.sort_unstable_by_key(|(d, f)| (*d, faction_ord(*f)));

    let mut out = Vec::new();
    for key in keys {
        let RoleBuckets { mut tanks, mut healers, mut dps } =
            by_bucket.remove(&key).expect("key collected from same map");
        let (d, _faction) = key;
        tanks.sort_unstable();
        healers.sort_unstable();
        dps.sort_unstable();

        let (mut ti, mut hi, mut di) = (0usize, 0usize, 0usize);
        while ti < tanks.len() && hi < healers.len() && di + 3 <= dps.len() {
            out.push(MatchProposal {
                dungeon_id: d,
                tank: tanks[ti],
                healer: healers[hi],
                dps: vec![dps[di], dps[di + 1], dps[di + 2]],
                has_real_player: false,
                real_player_guid: None,
            });
            ti += 1;
            hi += 1;
            di += 3;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Default to Alliance: single-faction queues exercise role/dungeon balancing.
    fn q(guid: u64, role: Role, dungeon_id: u32) -> QueueEntry {
        QueueEntry { guid, role, dungeon_id, faction: Faction::Alliance, is_real_player: false }
    }

    // Faction-aware variant for the cross-faction bucketing tests.
    fn qf(guid: u64, role: Role, dungeon_id: u32, faction: Faction) -> QueueEntry {
        QueueEntry { guid, role, dungeon_id, faction, is_real_player: false }
    }

    #[test]
    fn exact_five_forms_one_balanced_group() {
        let queue = vec![
            q(1, Role::Tank, 36),
            q(2, Role::Healer, 36),
            q(3, Role::Dps, 36),
            q(4, Role::Dps, 36),
            q(5, Role::Dps, 36),
        ];
        let m = find_matches(&queue);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].tank, 1);
        assert_eq!(m[0].healer, 2);
        assert_eq!(m[0].dps, vec![3, 4, 5]);
    }

    #[test]
    fn healer_shortage_limits_group_count() {
        // 2 tanks, 1 healer, 6 dps → only 1 group (healer-bound)
        let mut queue = vec![q(1, Role::Tank, 36), q(2, Role::Tank, 36), q(3, Role::Healer, 36)];
        for g in 10..16 {
            queue.push(q(g, Role::Dps, 36));
        }
        let m = find_matches(&queue);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn two_full_sets_form_two_groups() {
        let mut queue = Vec::new();
        for g in 0..2 {
            let base = g * 100;
            queue.push(q(base + 1, Role::Tank, 36));
            queue.push(q(base + 2, Role::Healer, 36));
            queue.push(q(base + 3, Role::Dps, 36));
            queue.push(q(base + 4, Role::Dps, 36));
            queue.push(q(base + 5, Role::Dps, 36));
        }
        let m = find_matches(&queue);
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn different_dungeons_do_not_mix() {
        let queue = vec![
            q(1, Role::Tank, 36),
            q(2, Role::Healer, 36),
            q(3, Role::Dps, 36),
            q(4, Role::Dps, 36),
            q(5, Role::Dps, 47), // different dungeon — should NOT complete the 36 group
        ];
        let m = find_matches(&queue);
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn insufficient_queue_yields_nothing() {
        let queue = vec![q(1, Role::Tank, 36), q(2, Role::Healer, 36)];
        assert!(find_matches(&queue).is_empty());
    }

    #[test]
    fn leftovers_form_one_group_and_strand_extra_dps() {
        // 1 tank, 1 healer, 4 dps → exactly 1 group; 1 dps left unmatched.
        let queue = vec![
            q(1, Role::Tank, 36),
            q(2, Role::Healer, 36),
            q(3, Role::Dps, 36),
            q(4, Role::Dps, 36),
            q(5, Role::Dps, 36),
            q(6, Role::Dps, 36),
        ];
        let m = find_matches(&queue);
        assert_eq!(m.len(), 1);
        // The group consumes the 3 lowest dps (sorted, deterministic); 6 is stranded.
        assert_eq!(m[0].dps, vec![3, 4, 5]);
        let matched: std::collections::HashSet<u64> = m[0].members().into_iter().collect();
        assert!(!matched.contains(&6), "6th dps must be left unmatched");
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(find_matches(&[]).is_empty());
    }

    #[test]
    fn output_is_deterministic() {
        let queue = vec![
            q(5, Role::Dps, 36),
            q(2, Role::Healer, 36),
            q(4, Role::Dps, 36),
            q(1, Role::Tank, 36),
            q(3, Role::Dps, 36),
        ];
        let a = find_matches(&queue);
        let b = find_matches(&queue);
        assert_eq!(a, b);
        assert_eq!(a[0].dps, vec![3, 4, 5]); // sorted
    }

    // Finding 1 (a): a full Alliance 5-man plus a lone Horde dps in the SAME dungeon
    // must yield exactly one Alliance group; the Horde dps is left unmatched (the
    // matcher must never propose a cross-faction group — the data plane rejects it).
    #[test]
    fn mixed_faction_same_dungeon_forms_only_same_faction_group() {
        let queue = vec![
            qf(1, Role::Tank, 36, Faction::Alliance),
            qf(2, Role::Healer, 36, Faction::Alliance),
            qf(3, Role::Dps, 36, Faction::Alliance),
            qf(4, Role::Dps, 36, Faction::Alliance),
            qf(5, Role::Dps, 36, Faction::Alliance),
            qf(6, Role::Dps, 36, Faction::Horde), // lone Horde dps — cannot complete a group
        ];
        let m = find_matches(&queue);
        assert_eq!(m.len(), 1, "exactly one same-faction group");
        assert_eq!(m[0].tank, 1);
        assert_eq!(m[0].healer, 2);
        assert_eq!(m[0].dps, vec![3, 4, 5]);
        let matched: std::collections::HashSet<u64> = m[0].members().into_iter().collect();
        assert!(!matched.contains(&6), "Horde dps must never join an Alliance group");
    }

    // build_real_player_proposal tests
    fn rp(guid: u64, role: Role) -> QueueEntry {
        QueueEntry {
            guid,
            role,
            dungeon_id: 4,
            faction: Faction::Alliance,
            is_real_player: true,
        }
    }

    #[test]
    fn real_player_tank_fills_healer_and_dps() {
        let fillers = [20u64, 30, 40, 50];
        let p = build_real_player_proposal(&rp(10, Role::Tank), &fillers, 4).unwrap();
        assert_eq!(p.tank, 10, "real player is tank");
        assert_eq!(p.healer, 20, "first filler is healer");
        assert_eq!(p.dps, vec![30, 40, 50]);
        assert!(p.has_real_player);
        assert_eq!(p.real_player_guid, Some(10));
        assert_eq!(p.dungeon_id, 4);
    }

    #[test]
    fn real_player_healer_fills_tank_and_dps() {
        let fillers = [20u64, 30, 40, 50];
        let p = build_real_player_proposal(&rp(10, Role::Healer), &fillers, 4).unwrap();
        assert_eq!(p.tank, 20, "first filler is tank");
        assert_eq!(p.healer, 10, "real player is healer");
        assert_eq!(p.dps, vec![30, 40, 50]);
        assert!(p.has_real_player);
    }

    #[test]
    fn real_player_dps_fills_tank_healer_and_two_dps() {
        let fillers = [20u64, 30, 40, 50];
        let p = build_real_player_proposal(&rp(10, Role::Dps), &fillers, 4).unwrap();
        assert_eq!(p.tank, 20);
        assert_eq!(p.healer, 30);
        // real player last in dps list
        assert_eq!(p.dps, vec![40, 50, 10]);
        assert!(p.has_real_player);
        assert_eq!(p.real_player_guid, Some(10));
    }

    #[test]
    fn real_player_proposal_wrong_filler_count_is_none() {
        // 3 fillers instead of 4 → None
        let p = build_real_player_proposal(&rp(1, Role::Tank), &[2, 3, 4], 4);
        assert!(p.is_none());
        // 5 fillers → None
        let p = build_real_player_proposal(&rp(1, Role::Tank), &[2, 3, 4, 5, 6], 4);
        assert!(p.is_none());
    }

    // Finding 1 (b): a full Alliance group AND a full Horde group queued for the same
    // dungeon must yield two groups — each homogeneous, never mixed.
    #[test]
    fn full_alliance_and_full_horde_same_dungeon_form_two_unmixed_groups() {
        let queue = vec![
            // Alliance 5-man
            qf(1, Role::Tank, 36, Faction::Alliance),
            qf(2, Role::Healer, 36, Faction::Alliance),
            qf(3, Role::Dps, 36, Faction::Alliance),
            qf(4, Role::Dps, 36, Faction::Alliance),
            qf(5, Role::Dps, 36, Faction::Alliance),
            // Horde 5-man
            qf(101, Role::Tank, 36, Faction::Horde),
            qf(102, Role::Healer, 36, Faction::Horde),
            qf(103, Role::Dps, 36, Faction::Horde),
            qf(104, Role::Dps, 36, Faction::Horde),
            qf(105, Role::Dps, 36, Faction::Horde),
        ];
        let m = find_matches(&queue);
        assert_eq!(m.len(), 2, "two groups, one per faction");
        // Deterministic order: Alliance (faction_ord 0) before Horde (1).
        let alliance: std::collections::HashSet<u64> = m[0].members().into_iter().collect();
        let horde: std::collections::HashSet<u64> = m[1].members().into_iter().collect();
        assert_eq!(alliance, [1, 2, 3, 4, 5].into_iter().collect());
        assert_eq!(horde, [101, 102, 103, 104, 105].into_iter().collect());
        // No group mixes the two guid bands.
        assert!(alliance.is_disjoint(&horde));
    }
}
