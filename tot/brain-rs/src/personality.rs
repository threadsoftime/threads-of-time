/// In-process LRU personality cache backed by memory MCP.
///
/// Faithful Rust port of brain_sidecar/personality.py PersonalityCache.
///
/// Memory-sidecar contract:
///   - `memory.personality_set`: `{"bot_id": str, "persona": str}` — persona is
///     a JSON-encoded PersonalityCard.
///   - `memory.personality_get`: `{"bot_id": str}` → `{"persona": str}`.
///
/// V3.6: runs lazy v2 migration (morph) on cache-miss when the persisted card
/// has any None v2 field. Per-bot Mutex serializes the migrate-and-persist path.
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;
use tokio::sync::Mutex;

use crate::models::PersonalityCard;

// ---------------------------------------------------------------------------
// McpCallable — injectable abstraction so tests can provide a mock.
//
// Uses Pin<Box<dyn Future>> for object safety (required for Arc<dyn McpCallable>).
// Rust 1.75+ stable async-fn-in-traits is not yet dyn-safe without the
// Pin<Box> indirection, so we use the classical workaround.
// ---------------------------------------------------------------------------

/// Minimal async trait for a memory MCP client.
/// Tests implement this with a mock; Phase 9 wires in the real rmcp client.
pub trait McpCallable: Send + Sync {
    fn call<'a>(
        &'a self,
        tool: &'a str,
        args: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, anyhow::Error>> + Send + 'a>,
    >;
}

// ---------------------------------------------------------------------------
// MorphCallable — injectable v2 migration hook.
// Phase 9 wires the real LLM-backed morph_personality here.
// ---------------------------------------------------------------------------

/// Async trait for the v2 morph step. Allows tests to inject a fake morph.
pub trait MorphCallable: Send + Sync {
    fn morph<'a>(
        &'a self,
        bot_guid: i64,
        card: PersonalityCard,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<PersonalityCard, anyhow::Error>> + Send + 'a>,
    >;
}

// ---------------------------------------------------------------------------
// V2 field names — mirrors morph.py _V2_FIELD_NAMES
// ---------------------------------------------------------------------------

/// True if any v2 field is None (full or partial-fill triggers morph).
pub fn needs_morph(card: &PersonalityCard) -> bool {
    card.pvp_appetite.is_none()
        || card.raid_appetite.is_none()
        || card.completionist_streak.is_none()
        || card.gold_motivation.is_none()
        || card.profession_appetite.is_none()
}

// ---------------------------------------------------------------------------
// PersonalityCache
// ---------------------------------------------------------------------------

struct CacheInner {
    cache: LruCache<i64, (PersonalityCard, f64)>,
}

/// LRU-backed personality cache with TTL and lazy v2 migration.
///
/// All mutable state lives behind `Mutex` so `get`/`seed` can be called
/// with `&self` across concurrent async tasks (matching the Python class).
pub struct PersonalityCache {
    inner: Mutex<CacheInner>,
    mcp: Arc<dyn McpCallable + Send + Sync>,
    ttl_s: f64,
    /// Per-bot locks — serializes concurrent migrations for the same bot.
    bot_locks: Mutex<HashMap<i64, Arc<Mutex<()>>>>,
    /// Injectable clock — monotonic f64 seconds. Defaults to wall-clock.
    now_fn: Arc<dyn Fn() -> f64 + Send + Sync>,
    /// Optional v2 morph hook. None in production until Phase 9 wiring.
    morph: Option<Arc<dyn MorphCallable + Send + Sync>>,
}

impl PersonalityCache {
    /// Construct with a real wall clock and no morph hook (production default).
    pub fn new(
        mcp: Arc<dyn McpCallable + Send + Sync>,
        ttl_s: f64,
        capacity: usize,
    ) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).expect("capacity must be > 0");
        Self {
            inner: Mutex::new(CacheInner { cache: LruCache::new(cap) }),
            mcp,
            ttl_s,
            bot_locks: Mutex::new(HashMap::new()),
            now_fn: Arc::new(|| {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0)
            }),
            morph: None,
        }
    }

    /// Full constructor — injectable clock + morph hook (used in tests).
    pub fn new_with_options(
        mcp: Arc<dyn McpCallable + Send + Sync>,
        ttl_s: f64,
        capacity: usize,
        now_fn: Arc<dyn Fn() -> f64 + Send + Sync>,
        morph: Option<Arc<dyn MorphCallable + Send + Sync>>,
    ) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).expect("capacity must be > 0");
        Self {
            inner: Mutex::new(CacheInner { cache: LruCache::new(cap) }),
            mcp,
            ttl_s,
            bot_locks: Mutex::new(HashMap::new()),
            now_fn,
            morph,
        }
    }

    /// Lazily create and return the per-bot lock (mirrors Python _lock_for).
    async fn lock_for(&self, bot_guid: i64) -> Arc<Mutex<()>> {
        let mut locks = self.bot_locks.lock().await;
        locks
            .entry(bot_guid)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Retrieve the personality card for `bot_guid`.
    ///
    /// Fast path: cache hit within TTL — no MCP call.
    /// Slow path: acquire per-bot lock, double-check, fetch from MCP,
    /// run v2 migration if needed, cache result.
    pub async fn get(&self, bot_guid: i64) -> Result<PersonalityCard, anyhow::Error> {
        // Fast path: cache hit without acquiring the bot lock.
        let now = (self.now_fn)();
        {
            let mut inner = self.inner.lock().await;
            if let Some((card, fetched_at)) = inner.cache.get(&bot_guid) {
                if now - *fetched_at < self.ttl_s {
                    return Ok(card.clone());
                }
            }
        }

        // Cache miss or expired — acquire per-bot lock to serialize migration.
        let bot_lock = self.lock_for(bot_guid).await;
        let _guard = bot_lock.lock().await;

        // Double-check: another caller may have populated cache while we waited.
        let now2 = (self.now_fn)();
        {
            let mut inner = self.inner.lock().await;
            if let Some((card, fetched_at)) = inner.cache.get(&bot_guid) {
                if now2 - *fetched_at < self.ttl_s {
                    return Ok(card.clone());
                }
            }
        }

        // Fetch from memory MCP.
        let raw = self
            .mcp
            .call(
                "memory.personality_get",
                serde_json::json!({"bot_id": bot_guid.to_string()}),
            )
            .await?;
        let payload = raw.get("result").unwrap_or(&raw);
        let persona_json = payload
            .get("persona")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        let mut card: PersonalityCard = serde_json::from_str(persona_json)
            .map_err(|e| anyhow::anyhow!("personality JSON parse error: {e}"))?;

        // V3.6 lazy migration: if any v2 field is None, run morph + persist back.
        if needs_morph(&card) {
            if let Some(morph) = &self.morph {
                match morph.morph(bot_guid, card.clone()).await {
                    Ok(morphed) => {
                        card = morphed.clone();
                        // Persist back — soft-fail on error (matches Python behaviour).
                        let persona_str = serde_json::to_string(&morphed)
                            .unwrap_or_else(|_| "{}".to_string());
                        if let Err(e) = self
                            .mcp
                            .call(
                                "memory.personality_set",
                                serde_json::json!({
                                    "bot_id": bot_guid.to_string(),
                                    "persona": persona_str,
                                }),
                            )
                            .await
                        {
                            tracing::error!(
                                "PersonalityCache.get bot_guid={bot_guid}: \
                                 migration persist failed: {e}. \
                                 Cache populated; will retry on eviction."
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "PersonalityCache.get bot_guid={bot_guid}: morph failed: {e}. \
                             v2 fields remain None."
                        );
                    }
                }
            } else {
                tracing::error!(
                    "PersonalityCache.get bot_guid={bot_guid}: morph hook is None; \
                     cannot migrate v1 card. v2 fields remain None."
                );
            }
        }

        // Insert into LRU, evicting oldest if at capacity.
        let fetched_at = (self.now_fn)();
        {
            let mut inner = self.inner.lock().await;
            inner.cache.put(bot_guid, (card.clone(), fetched_at));
        }
        Ok(card)
    }

    /// Seed the cache with a known card and persist it to the memory MCP.
    /// Mirrors Python `PersonalityCache.seed`.
    pub async fn seed(
        &self,
        bot_guid: i64,
        card: PersonalityCard,
    ) -> Result<(), anyhow::Error> {
        let persona_str = serde_json::to_string(&card)
            .map_err(|e| anyhow::anyhow!("personality serialize error: {e}"))?;
        self.mcp
            .call(
                "memory.personality_set",
                serde_json::json!({
                    "bot_id": bot_guid.to_string(),
                    "persona": persona_str,
                }),
            )
            .await?;
        let fetched_at = (self.now_fn)();
        let mut inner = self.inner.lock().await;
        inner.cache.put(bot_guid, (card, fetched_at));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ----- Test helpers -----

    /// A minimal v2-complete PersonalityCard.
    fn make_card(name: &str) -> PersonalityCard {
        PersonalityCard {
            name: name.to_string(),
            race: "Human".to_string(),
            class_: "Warrior".to_string(),
            backstory: "A test hero.".to_string(),
            talkativeness: 0.5,
            courage: 0.5,
            greed: 0.3,
            attitude_to_master: 0.0,
            party_invite_policy: "accept_from_known".to_string(),
            pvp_appetite: Some(0.4),
            raid_appetite: Some(0.6),
            completionist_streak: Some(0.5),
            gold_motivation: Some(0.3),
            profession_appetite: Some(0.5),
        }
    }

    /// A v1 card — all v2 fields None, triggers morph.
    fn make_v1_card(name: &str) -> PersonalityCard {
        PersonalityCard {
            name: name.to_string(),
            race: "Human".to_string(),
            class_: "Warrior".to_string(),
            backstory: "A test hero.".to_string(),
            talkativeness: 0.5,
            courage: 0.5,
            greed: 0.3,
            attitude_to_master: 0.0,
            party_invite_policy: "accept_from_known".to_string(),
            pvp_appetite: None,
            raid_appetite: None,
            completionist_streak: None,
            gold_motivation: None,
            profession_appetite: None,
        }
    }

    // ----- Mock MCP client -----

    struct MockMcp {
        call_count: Arc<AtomicUsize>,
        card: PersonalityCard,
    }

    impl McpCallable for MockMcp {
        fn call<'a>(
            &'a self,
            _tool: &'a str,
            _args: serde_json::Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<serde_json::Value, anyhow::Error>,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                self.call_count.fetch_add(1, Ordering::SeqCst);
                let persona_str = serde_json::to_string(&self.card).unwrap();
                Ok(serde_json::json!({"persona": persona_str}))
            })
        }
    }

    fn make_mock_mcp(card: PersonalityCard) -> (Arc<MockMcp>, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let mock = Arc::new(MockMcp { call_count: counter.clone(), card });
        (mock, counter)
    }

    // ----- Mock morph hook -----

    struct MockMorph {
        call_count: Arc<AtomicUsize>,
    }

    impl MorphCallable for MockMorph {
        fn morph<'a>(
            &'a self,
            _bot_guid: i64,
            mut card: PersonalityCard,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<PersonalityCard, anyhow::Error>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                self.call_count.fetch_add(1, Ordering::SeqCst);
                card.pvp_appetite = Some(0.5);
                card.raid_appetite = Some(0.5);
                card.completionist_streak = Some(0.5);
                card.gold_motivation = Some(0.5);
                card.profession_appetite = Some(0.5);
                Ok(card)
            })
        }
    }

    // ----- Clock helpers -----

    /// Frozen clock: always returns `t`.
    fn frozen_clock(t: f64) -> Arc<dyn Fn() -> f64 + Send + Sync> {
        Arc::new(move || t)
    }

    /// Advancing clock: each call returns the next value from `values`.
    /// After exhaustion, returns the last value repeatedly.
    fn advancing_clock(values: Vec<f64>) -> Arc<dyn Fn() -> f64 + Send + Sync> {
        let idx = Arc::new(AtomicUsize::new(0));
        let values = Arc::new(values);
        Arc::new(move || {
            let i = idx.fetch_add(1, Ordering::SeqCst);
            *values.get(i).unwrap_or_else(|| values.last().unwrap_or(&0.0))
        })
    }

    // ----- Tests -----

    #[tokio::test]
    async fn test_cache_hit_returns_without_mcp_call() {
        let card = make_card("Alice");
        let (mock, counter) = make_mock_mcp(card);

        let cache = PersonalityCache::new_with_options(
            mock,
            300.0,
            10,
            frozen_clock(1000.0),
            None,
        );

        // First call: cache miss → 1 MCP call.
        let c1 = cache.get(42).await.unwrap();
        assert_eq!(c1.name, "Alice");

        // Second call: cache hit → no additional MCP call.
        let c2 = cache.get(42).await.unwrap();
        assert_eq!(c2.name, "Alice");

        // Card is v2-complete so no morph; exactly 1 MCP call total.
        assert_eq!(counter.load(Ordering::SeqCst), 1, "second get must be a cache hit");
    }

    #[tokio::test]
    async fn test_cache_eviction_at_capacity() {
        // Capacity = 2; insert 3 bots → bot 1 evicted (LRU order).
        let (mock, counter) = make_mock_mcp(make_card("Bot"));

        let cache = PersonalityCache::new_with_options(mock, 300.0, 2, frozen_clock(1000.0), None);

        cache.get(1).await.unwrap(); // call 1
        cache.get(2).await.unwrap(); // call 2
        cache.get(3).await.unwrap(); // call 3 — bot 1 evicted from [1,2] → [2,3]

        let before = counter.load(Ordering::SeqCst);
        assert_eq!(before, 3, "expected exactly 3 MCP calls before eviction refetch");

        cache.get(1).await.unwrap(); // cache miss (bot 1 evicted) → call 4
        let after = counter.load(Ordering::SeqCst);
        assert_eq!(after, 4, "evicted bot must trigger a new MCP call");
    }

    #[tokio::test]
    async fn test_ttl_expiry_refetches() {
        let (mock, counter) = make_mock_mcp(make_card("Carol"));

        // TTL = 100 s. Clock schedule:
        //   get() call 1 fast-path check:        t=0.0  → cache empty → miss
        //   get() call 1 double-check:           t=0.0  → still miss
        //   get() call 1 store fetched_at:       t=0.0
        //   get() call 2 fast-path check:        t=500.0 → 500 >= 100 → expired
        //   get() call 2 double-check:           t=500.0 → still expired
        //   get() call 2 store fetched_at:       t=500.0
        let clock = advancing_clock(vec![
            0.0, 0.0, 0.0,
            500.0, 500.0, 500.0,
        ]);

        let cache = PersonalityCache::new_with_options(mock, 100.0, 10, clock, None);

        cache.get(99).await.unwrap(); // cold fetch
        cache.get(99).await.unwrap(); // TTL expired → refetch

        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "TTL expiry must trigger a second MCP call"
        );
    }

    #[tokio::test]
    async fn test_v2_migration_runs_when_none_fields_present() {
        // v1 card (all v2 fields None) → morph hook called once → result has v2 fields.
        let (mock, _mcp_counter) = make_mock_mcp(make_v1_card("Dave"));

        let morph_counter = Arc::new(AtomicUsize::new(0));
        let morph: Arc<dyn MorphCallable + Send + Sync> =
            Arc::new(MockMorph { call_count: morph_counter.clone() });

        let cache = PersonalityCache::new_with_options(
            mock,
            300.0,
            10,
            frozen_clock(1000.0),
            Some(morph),
        );

        let card = cache.get(77).await.unwrap();

        assert_eq!(morph_counter.load(Ordering::SeqCst), 1, "morph must be called exactly once");
        assert!(card.pvp_appetite.is_some(), "pvp_appetite must be populated after morph");
        assert!(card.raid_appetite.is_some(), "raid_appetite must be populated after morph");
        assert!(card.completionist_streak.is_some());
        assert!(card.gold_motivation.is_some());
        assert!(card.profession_appetite.is_some());
    }

    #[tokio::test]
    async fn test_no_morph_when_v2_complete() {
        // v2-complete card — morph hook must not be called.
        let (mock, _mcp_counter) = make_mock_mcp(make_card("Frank"));
        let morph_counter = Arc::new(AtomicUsize::new(0));
        let morph: Arc<dyn MorphCallable + Send + Sync> =
            Arc::new(MockMorph { call_count: morph_counter.clone() });

        let cache = PersonalityCache::new_with_options(
            mock,
            300.0,
            10,
            frozen_clock(1000.0),
            Some(morph),
        );

        cache.get(88).await.unwrap();
        assert_eq!(
            morph_counter.load(Ordering::SeqCst),
            0,
            "morph must NOT run when v2 fields are already present"
        );
    }

    #[tokio::test]
    async fn test_seed_persists_and_caches() {
        let card = make_card("Eve");
        let (mock, counter) = make_mock_mcp(card.clone());

        let cache = PersonalityCache::new_with_options(
            mock,
            300.0,
            10,
            frozen_clock(1000.0),
            None,
        );

        cache.seed(55, card.clone()).await.unwrap();
        // seed calls personality_set → 1 MCP call.
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // get should be a cache hit (no additional MCP call).
        let got = cache.get(55).await.unwrap();
        assert_eq!(got.name, "Eve");
        assert_eq!(counter.load(Ordering::SeqCst), 1, "get after seed must be a cache hit");
    }
}
