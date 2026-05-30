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

    pub fn remove_many(&self, guids: &[u64]) {
        let mut g = self.inner.lock().unwrap();
        for guid in guids {
            g.remove(guid);
        }
    }

    pub fn snapshot(&self) -> Vec<QueueEntry> {
        self.inner.lock().unwrap().values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Role;

    fn e(guid: u64, role: Role) -> QueueEntry {
        QueueEntry { guid, role, dungeon_id: 36 }
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
}
