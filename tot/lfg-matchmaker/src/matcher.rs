use crate::types::{MatchProposal, QueueEntry, Role};
use std::collections::HashMap;

#[derive(Default)]
struct RoleBuckets {
    tanks: Vec<u64>,
    healers: Vec<u64>,
    dps: Vec<u64>,
}

/// Greedy, deterministic role-balanced matcher.
/// Forms as many 1-tank / 1-healer / 3-dps groups per `dungeon_id` as the
/// queue allows. Pure — no I/O, no clock, no randomness. Output is sorted by
/// dungeon id then by member guids for reproducibility.
pub fn find_matches(queue: &[QueueEntry]) -> Vec<MatchProposal> {
    // bucket guids by dungeon, then by role
    let mut by_dungeon: HashMap<u32, RoleBuckets> = HashMap::new();
    for e in queue {
        let b = by_dungeon.entry(e.dungeon_id).or_default();
        match e.role {
            Role::Tank => b.tanks.push(e.guid),
            Role::Healer => b.healers.push(e.guid),
            Role::Dps => b.dps.push(e.guid),
        }
    }

    let mut dungeons: Vec<u32> = by_dungeon.keys().copied().collect();
    dungeons.sort_unstable();

    let mut out = Vec::new();
    for d in dungeons {
        let RoleBuckets { mut tanks, mut healers, mut dps } = by_dungeon.remove(&d).expect("key collected from same map");
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

    fn q(guid: u64, role: Role, dungeon_id: u32) -> QueueEntry {
        QueueEntry { guid, role, dungeon_id }
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
}
