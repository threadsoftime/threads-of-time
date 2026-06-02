//! Per-bot publish/subscribe bus for SSE fan-out.
//!
//! # Design (matches Python `pubsub.py` semantics)
//!
//! Each subscriber gets an `Arc<SubscriberQueue>` which wraps a bounded
//! `VecDeque` (capacity 64) + a `Notify`.  The publisher holds a `Weak`
//! reference; when the subscriber is dropped the `Weak` upgrade fails and the
//! sender is pruned on the next publish.
//!
//! Drop-oldest on overflow (Python parity):
//! ```text
//! if deque.len() >= 64 { deque.pop_front(); }  // drop oldest
//! deque.push_back(row);
//! notify.notify_one();
//! ```
//!
//! Subscribe-then-replay ordering (critical):
//! The subscriber must be registered **before** the replay phase so that any
//! rows written during replay are buffered and available for the drain phase.
//! The dedup watermark (`max_replayed_rowid`) filters duplicates in Phase 2.
//!
//! # Sync-safe publish
//!
//! `publish` uses `std::sync::Mutex` (blocking, not tokio) because it is
//! called from `MemoryService::write` which runs after a `spawn_blocking`
//! returns — i.e. back in the async executor task but not inside a
//! `spawn_blocking` closure.  `std::sync::Mutex::lock` is safe here: the
//! critical section is tiny (a VecDeque push + pop at most) and never yields.
//! Using tokio's async Mutex here would require `.await` and infect the
//! publish signature unnecessarily.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::Notify;
use tracing::warn;

// ---------------------------------------------------------------------------
// MemoryRow — published after every successful write
// ---------------------------------------------------------------------------

/// A memory row broadcast to SSE subscribers after a successful write.
///
/// Carries `row_id` (the SQLite `rowid` of the newly inserted row) so that
/// the SSE client can use it as a `Last-Event-ID` cursor for replay.
#[derive(Debug, Clone)]
pub struct MemoryRow {
    /// SQLite `rowid` — used as the monotonic SSE `id:` cursor.
    pub row_id: i64,
    /// String PK — e.g. `"m_abc123defgh"`.
    pub memory_id: String,
    pub bot_id: String,
    pub text: String,
    /// Unix epoch seconds (`created_ts` column).
    pub created_ts: i64,
    pub salience: f32,
}

// ---------------------------------------------------------------------------
// SubscriberQueue — one per live SSE connection
// ---------------------------------------------------------------------------

const QUEUE_MAXSIZE: usize = 64;

/// Per-subscriber bounded queue.  The SSE handler holds an `Arc`; the
/// `PubSub` registry holds a `Weak` so it is pruned automatically when
/// the subscriber disconnects.
pub struct SubscriberQueue {
    inner: Mutex<VecDeque<MemoryRow>>,
    notify: Notify,
}

impl SubscriberQueue {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(VecDeque::with_capacity(QUEUE_MAXSIZE + 1)),
            notify: Notify::new(),
        })
    }

    /// Push a row.  Drops the **oldest** entry on overflow (Python parity).
    fn push(&self, row: MemoryRow, bot_id: &str) {
        let mut deque = self.inner.lock().expect("subscriber queue poisoned");
        if deque.len() >= QUEUE_MAXSIZE {
            deque.pop_front(); // drop oldest
            warn!(bot_id = bot_id, "sse_subscriber_overflow");
        }
        deque.push_back(row);
        // Drop the lock before notifying to avoid holding it during wakeup.
        drop(deque);
        self.notify.notify_one();
    }

    /// Pop the oldest row (non-blocking).  Returns `None` when empty.
    pub fn try_recv(&self) -> Option<MemoryRow> {
        self.inner.lock().expect("subscriber queue poisoned").pop_front()
    }

    /// Wait until at least one row is available (or the future is cancelled).
    ///
    /// This is a `Notified` future — callers typically race it with a timeout:
    /// ```ignore
    /// tokio::time::timeout(Duration::from_secs(15), queue.notified()).await
    /// ```
    pub async fn notified(&self) {
        self.notify.notified().await
    }
}

// ---------------------------------------------------------------------------
// PubSub — shared bus
// ---------------------------------------------------------------------------

/// Per-bot subscriber registry.
///
/// Stored as `Arc<PubSub>` in `AppState` so both the SSE handler and the
/// write path share the same instance.
pub struct PubSub {
    /// bot_id → list of weak subscriber handles.
    subs: std::sync::RwLock<HashMap<String, Vec<Weak<SubscriberQueue>>>>,
}

impl std::fmt::Debug for PubSub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PubSub").finish_non_exhaustive()
    }
}

impl Default for PubSub {
    fn default() -> Self {
        Self::new()
    }
}

impl PubSub {
    /// Construct a new `PubSub`.
    pub fn new() -> Self {
        PubSub {
            subs: std::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Register a new subscriber for `bot_id`.
    ///
    /// Must be called **before** replay (subscribe-then-replay ordering).
    /// Returns an `Arc<SubscriberQueue>` that the SSE handler polls.
    pub fn subscribe(&self, bot_id: &str) -> Arc<SubscriberQueue> {
        let queue = SubscriberQueue::new();
        let mut map = self.subs.write().expect("pubsub rwlock poisoned");
        map.entry(bot_id.to_owned())
            .or_default()
            .push(Arc::downgrade(&queue));
        queue
    }

    /// Publish a row to all live subscribers for `row.bot_id`.
    ///
    /// - On full queue: drop-oldest (Python parity), log warning.
    /// - On disconnected subscriber (`Weak::upgrade` fails): prune in place.
    ///
    /// Never panics; never blocks on I/O; never requires async context.
    pub fn publish(&self, row: &MemoryRow) {
        let bot_id = &row.bot_id;

        // Fast path: no subscribers for this bot.
        {
            let map = self.subs.read().expect("pubsub rwlock poisoned");
            if !map.contains_key(bot_id) {
                return;
            }
        }

        // Upgrade to write lock to allow pruning dead Weaks.
        let mut map = self.subs.write().expect("pubsub rwlock poisoned");
        if let Some(senders) = map.get_mut(bot_id) {
            senders.retain(|weak| {
                if let Some(queue) = weak.upgrade() {
                    queue.push(row.clone(), bot_id);
                    true // keep
                } else {
                    false // prune disconnected
                }
            });
            if senders.is_empty() {
                map.remove(bot_id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    fn make_row(row_id: i64, bot_id: &str, text: &str) -> MemoryRow {
        MemoryRow {
            row_id,
            memory_id: format!("m_{row_id}"),
            bot_id: bot_id.to_owned(),
            text: text.to_owned(),
            created_ts: 1_700_000_000 + row_id,
            salience: 0.5,
        }
    }

    /// subscribe → publish delivers the row.
    #[tokio::test]
    async fn subscribe_then_publish_delivers() {
        let ps = PubSub::new();
        let q = ps.subscribe("bot1");
        ps.publish(&make_row(1, "bot1", "hello"));

        // notify fires; try_recv returns the row
        timeout(Duration::from_millis(100), q.notified())
            .await
            .expect("notified should fire");
        let row = q.try_recv().expect("row present");
        assert_eq!(row.row_id, 1);
        assert_eq!(row.text, "hello");
    }

    /// Publish to a different bot_id does not deliver.
    #[tokio::test]
    async fn publish_to_different_bot_does_not_deliver() {
        let ps = PubSub::new();
        let q = ps.subscribe("bot1");
        ps.publish(&make_row(1, "bot2", "hello")); // different bot
        assert!(q.try_recv().is_none(), "should not receive cross-bot row");
    }

    /// Full channel (64) drops the oldest entry (Python parity).
    #[test]
    fn full_channel_drops_oldest() {
        let ps = PubSub::new();
        let q = ps.subscribe("bot1");

        // Fill to capacity
        for i in 0..64 {
            ps.publish(&make_row(i, "bot1", &format!("msg {i}")));
        }

        // Oldest (row_id=0) should still be present
        let first = q.try_recv().expect("first row");
        assert_eq!(first.row_id, 0);

        // Re-fill to capacity + 1 to trigger drop
        // (already popped one, so 63 remain; push row 64 → no drop yet; push 65 → drops oldest remaining)
        ps.publish(&make_row(64, "bot1", "msg 64")); // total = 64 again, no drop
        ps.publish(&make_row(65, "bot1", "msg 65")); // total would be 65 → drop oldest (row 1)

        let new_oldest = q.try_recv().expect("new oldest");
        assert_eq!(new_oldest.row_id, 2, "row_id=1 should have been dropped as oldest");
    }

    /// Dropped receiver is pruned on next publish (no leak).
    #[test]
    fn dropped_receiver_pruned_on_publish() {
        let ps = PubSub::new();
        let q = ps.subscribe("bot1");
        drop(q); // drop the subscriber

        // Should not panic; the dead Weak is pruned
        ps.publish(&make_row(1, "bot1", "after drop"));

        // Registry entry cleaned up
        let map = ps.subs.read().unwrap();
        assert!(
            !map.contains_key("bot1"),
            "empty senders list should be removed"
        );
    }

    /// Multiple subscribers for the same bot all receive the row.
    #[test]
    fn multiple_subscribers_all_receive() {
        let ps = PubSub::new();
        let q1 = ps.subscribe("bot1");
        let q2 = ps.subscribe("bot1");

        ps.publish(&make_row(42, "bot1", "broadcast"));

        assert_eq!(q1.try_recv().unwrap().row_id, 42);
        assert_eq!(q2.try_recv().unwrap().row_id, 42);
    }

    /// MemoryRow must be Clone (publish clones for each subscriber).
    #[test]
    fn memory_row_is_clone() {
        let row = make_row(1, "bot1", "text");
        let _cloned = row.clone();
    }

    /// PubSub::publish must not panic even with no prior publish call.
    #[test]
    fn publish_with_no_subscribers_no_panic() {
        let ps = PubSub::new();
        ps.publish(&make_row(1, "unknown_bot", "text"));
    }
}
