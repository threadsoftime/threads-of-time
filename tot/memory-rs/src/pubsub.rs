//! Minimal publish/subscribe stub — full SSE fan-out is Phase 6 (Task 6.6).
//!
//! `PubSub::publish` is a safe no-op hook that `MemoryService::write` calls
//! after each successful commit.  The full implementation (per-bot mpsc
//! channels + SSE streaming) will replace this stub in Task 6.6 without
//! changing the call site.
//!
//! # Design note (from Python parity)
//!
//! In `routes_memory.py::remember`:
//! ```python
//! if pubsub is not None:
//!     try:
//!         pubsub.publish(req.bot_id, row_dict)
//!     except Exception as e:
//!         logger.warning(...)
//! ```
//! The publish is fire-and-forget; errors are logged but not propagated.
//! This stub matches that contract: `publish` returns `()`, never panics.

/// A memory row broadcast to SSE subscribers after a successful write.
///
/// This is the minimal payload the write path assembles; the Phase 6
/// implementation will fan it out to per-bot `tokio::sync::broadcast` channels.
#[derive(Debug, Clone)]
pub struct MemoryRow {
    pub memory_id: String,
    pub bot_id: String,
    pub text: String,
    pub created_ts: i64,
    pub salience: f32,
}

/// No-op pub/sub bus.  Phase 6 replaces the internals without changing the API.
#[derive(Debug, Default, Clone)]
pub struct PubSub;

impl PubSub {
    /// Construct a new (no-op) `PubSub`.
    pub fn new() -> Self {
        PubSub
    }

    /// Publish a memory row to any registered subscribers.
    ///
    /// Current implementation: no-op.  Phase 6 will fan `row` out to
    /// per-bot `tokio::sync::broadcast` channels held inside this struct.
    ///
    /// Never panics; never blocks; never returns an error.
    pub fn publish(&self, _row: &MemoryRow) {
        // Phase 6 will replace this body.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// publish must not panic on a well-formed MemoryRow.
    #[test]
    fn publish_does_not_panic() {
        let ps = PubSub::new();
        let row = MemoryRow {
            memory_id: "m_testrow001".to_string(),
            bot_id: "bot_1".to_string(),
            text: "test memory".to_string(),
            created_ts: 1_700_000_000,
            salience: 0.8,
        };
        ps.publish(&row); // must not panic
    }

    /// PubSub must be Clone (stored in Arc<PubSub> inside MemoryService).
    #[test]
    fn pubsub_is_clone() {
        let ps = PubSub::new();
        let _ = ps.clone();
    }
}
