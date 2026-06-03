/// SeenMemoryIds — bounded LRU set of seen memory_ids.
///
/// Faithful Rust port of brain_sidecar/dedup.py SeenMemoryIds.
///
/// Design: OrderedDict in Python → VecDeque (insertion-order queue) +
/// HashSet (O(1) membership) pair in Rust.
///
/// Semantics:
///   - `mark(id)`: if already present, move to most-recent (LRU refresh);
///     otherwise insert. Evict oldest entry when capacity is exceeded.
///   - `seen(id)`: return true iff id was previously marked.
///
/// Thread-safety: NOT thread-safe. Match Python: single event-loop cooperative
/// multitasking. Callers must serialize access (e.g. wrap in Mutex if shared).
use std::collections::{HashMap, VecDeque};

/// Default production capacity (matches Python `SeenMemoryIds(capacity=200)`).
pub const DEFAULT_CAPACITY: usize = 200;

pub struct SeenMemoryIds {
    capacity: usize,
    /// Tracks insertion order; front = oldest, back = most-recent.
    order: VecDeque<String>,
    /// Maps id → index in `order` for O(1) membership check.
    /// The index is kept as () — membership only; no index stored.
    set: HashMap<String, ()>,
}

impl SeenMemoryIds {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be > 0");
        Self {
            capacity,
            order: VecDeque::with_capacity(capacity + 1),
            set: HashMap::with_capacity(capacity + 1),
        }
    }

    /// Register `memory_id` as seen.
    ///
    /// If already present: move to most-recent (LRU refresh), no-op for capacity.
    /// If new: insert at back; evict front (oldest) when over capacity.
    pub fn mark(&mut self, memory_id: &str) {
        if self.set.contains_key(memory_id) {
            // Already seen — refresh position: remove from current slot, push back.
            // Linear scan in VecDeque is acceptable (cap 200).
            if let Some(pos) = self.order.iter().position(|x| x == memory_id) {
                self.order.remove(pos);
            }
            self.order.push_back(memory_id.to_string());
            // set membership unchanged; no capacity change.
            return;
        }
        // New entry.
        self.order.push_back(memory_id.to_string());
        self.set.insert(memory_id.to_string(), ());
        // Evict oldest if over capacity.
        if self.set.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
    }

    /// Return `true` iff `memory_id` was previously marked.
    pub fn seen(&self, memory_id: &str) -> bool {
        self.set.contains_key(memory_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mark_and_seen() {
        let mut d = SeenMemoryIds::new(10);
        assert!(!d.seen("abc"));
        d.mark("abc");
        assert!(d.seen("abc"));
    }

    #[test]
    fn test_eviction_at_capacity() {
        let mut d = SeenMemoryIds::new(3);
        d.mark("a");
        d.mark("b");
        d.mark("c");
        assert!(d.seen("a"));
        d.mark("d"); // evicts "a" (oldest / front)
        assert!(!d.seen("a"), "oldest should be evicted");
        assert!(d.seen("d"));
    }

    #[test]
    fn test_mark_existing_is_idempotent() {
        let mut d = SeenMemoryIds::new(3);
        d.mark("a");
        d.mark("b");
        d.mark("a"); // re-mark "a" → moves to most-recent; order: [b, a]
        d.mark("c"); // order: [b, a, c] — at cap
        d.mark("d"); // evicts "b" (oldest), not "a"
        assert!(d.seen("a"), "re-marked a should not be evicted");
        assert!(!d.seen("b"), "b should be evicted");
    }

    #[test]
    fn test_never_exceed_capacity() {
        let cap = 5;
        let mut d = SeenMemoryIds::new(cap);
        for i in 0..20u32 {
            d.mark(&i.to_string());
        }
        assert_eq!(d.set.len(), cap);
        assert_eq!(d.order.len(), cap);
    }

    #[test]
    fn test_seen_after_eviction_returns_false() {
        let mut d = SeenMemoryIds::new(2);
        d.mark("x");
        d.mark("y");
        d.mark("z"); // evicts "x"
        assert!(!d.seen("x"));
        assert!(d.seen("y"));
        assert!(d.seen("z"));
    }

    #[test]
    fn test_default_capacity_constant() {
        assert_eq!(DEFAULT_CAPACITY, 200);
    }
}
