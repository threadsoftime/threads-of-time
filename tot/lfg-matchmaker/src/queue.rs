use crate::types::QueueEntry;
use std::collections::HashMap;
use std::sync::Mutex;

/// Authoritative in-memory queue, keyed by bot guid (re-queue = upsert).
/// Locks are never held across `.await` — every method returns owned data.
#[derive(Default)]
pub struct Queue {
    inner: Mutex<HashMap<u64, QueueEntry>>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, entry: QueueEntry) {
        self.inner.lock().unwrap().insert(entry.guid, entry);
    }

    pub fn remove(&self, guid: u64) -> bool {
        self.inner.lock().unwrap().remove(&guid).is_some()
    }

    #[allow(dead_code)] // superseded by take_many in the tick loop; kept for API symmetry
    pub fn remove_many(&self, guids: &[u64]) {
        let mut g = self.inner.lock().unwrap();
        for guid in guids {
            g.remove(guid);
        }
    }

    /// Remove AND return the entries for `guids` that were present. Used by the
    /// tick loop so a transient form failure can re-`upsert` the exact entries
    /// (role + dungeon preserved) instead of silently dropping the bots.
    pub fn take_many(&self, guids: &[u64]) -> Vec<QueueEntry> {
        let mut g = self.inner.lock().unwrap();
        guids.iter().filter_map(|guid| g.remove(guid)).collect()
    }

    pub fn snapshot(&self) -> Vec<QueueEntry> {
        self.inner.lock().unwrap().values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    #[allow(dead_code)] // conventional collection-wrapper method; kept for completeness
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Faction, Role};

    fn e(guid: u64, role: Role) -> QueueEntry {
        QueueEntry { guid, role, dungeon_id: 4, faction: Faction::Alliance, is_real_player: false }
    }

    #[test]
    fn upsert_dedupes_by_guid() {
        let q = Queue::new();
        q.upsert(e(1, Role::Tank));
        q.upsert(e(1, Role::Dps)); // same guid → replace, not duplicate
        assert_eq!(q.len(), 1);
        assert_eq!(q.snapshot()[0].role, Role::Dps);
    }

    #[test]
    fn remove_and_remove_many() {
        let q = Queue::new();
        for i in 1..=5 {
            q.upsert(e(i, Role::Dps));
        }
        assert!(q.remove(3));
        assert!(!q.remove(3));
        q.remove_many(&[1, 2]);
        assert_eq!(q.len(), 2); // 4 and 5 remain
    }

    #[test]
    fn take_many_returns_removed_entries_and_skips_absent() {
        let q = Queue::new();
        q.upsert(e(1, Role::Tank));
        q.upsert(e(2, Role::Healer));
        q.upsert(e(3, Role::Dps));

        let taken = q.take_many(&[1, 2, 99]); // 99 absent — silently skipped
        assert_eq!(taken.len(), 2);
        assert_eq!(q.len(), 1); // only 3 remains
        let roles: Vec<Role> = taken.iter().map(|e| e.role).collect();
        assert!(roles.contains(&Role::Tank));
        assert!(roles.contains(&Role::Healer));

        // Re-upsert restores them exactly (round-trip for the re-queue path).
        for entry in taken {
            q.upsert(entry);
        }
        assert_eq!(q.len(), 3);
    }
}
