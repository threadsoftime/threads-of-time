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
// PersonalityRecovery — injectable self-heal hook.
//
// Invoked by `get()` when the stored persona is unusable (empty / blank / null /
// missing / unparseable / parseable-but-invalid). Implementations decide the
// identity source and MUST return a fully-valid, morphed PersonalityCard
// reflecting the bot's real in-game identity where possible. `get()` then
// persists the returned card so the bot heals permanently (no per-tick spam).
//
// Production wires a LiveRecovery (state_store seed → obs.get_state identity →
// degraded generic, then morph). Tests inject a mock. None in a minimal cache
// preserves the old fail-loud behavior.
// ---------------------------------------------------------------------------

/// Async trait for the self-heal step. Allows tests to inject a fake recovery.
pub trait PersonalityRecovery: Send + Sync {
    fn recover<'a>(
        &'a self,
        bot_guid: i64,
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
    /// Optional self-heal hook. When set, `get()` recovers (instead of erroring)
    /// from an unusable persona, then persists the recovered card. When None,
    /// `get()` preserves the old fail-loud behavior on an unusable persona.
    recovery: Option<Arc<dyn PersonalityRecovery + Send + Sync>>,
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
            recovery: None,
        }
    }

    /// Builder: attach a self-heal hook. Returns `self` for chaining at the
    /// construction site (app.rs wires the production LiveRecovery here).
    pub fn with_recovery(
        mut self,
        recovery: Arc<dyn PersonalityRecovery + Send + Sync>,
    ) -> Self {
        self.recovery = Some(recovery);
        self
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
            recovery: None,
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

        // Decide whether the stored persona is USABLE. Treat all of these as
        // "not usable" → self-heal (or fail-loud if no recovery is wired):
        //   - key missing or JSON null            → as_str() == None
        //   - empty / whitespace-only string      → trim().is_empty()
        //   - unparseable JSON                     → from_str(..).is_err()
        //   - parseable but not a valid card       → from_str(..).is_err()
        //     (e.g. "{}" — PersonalityCard has required identity fields)
        let usable_card: Option<PersonalityCard> = payload
            .get("persona")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .and_then(|s| serde_json::from_str::<PersonalityCard>(s).ok());

        let mut card: PersonalityCard = match usable_card {
            Some(c) => c,
            None => {
                // Stored persona is unusable. Self-heal if a recovery hook is
                // wired; else preserve the old fail-loud behavior so nothing
                // silently degrades where recovery is not configured.
                match &self.recovery {
                    Some(recovery) => {
                        tracing::warn!(
                            "PersonalityCache.get bot_guid={bot_guid}: stored persona \
                             unusable (empty/blank/null/unparseable); recovering."
                        );
                        let recovered = recovery.recover(bot_guid).await.map_err(|e| {
                            anyhow::anyhow!("personality recovery failed: {e}")
                        })?;
                        // Persist the recovered card so the bot heals
                        // permanently (no per-tick spam). Soft-fail on persist
                        // error — matches the migration-persist soft-fail below.
                        let persona_str = serde_json::to_string(&recovered)
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
                                "PersonalityCache.get bot_guid={bot_guid}: recovery \
                                 persist failed: {e}. Cache populated; will retry on eviction."
                            );
                        }
                        // Recovery returns a fully-morphed card; cache + return
                        // directly (skip the v2 morph-migration branch below).
                        let fetched_at = (self.now_fn)();
                        let mut inner = self.inner.lock().await;
                        inner.cache.put(bot_guid, (recovered.clone(), fetched_at));
                        return Ok(recovered);
                    }
                    None => {
                        return Err(anyhow::anyhow!(
                            "personality unusable for bot_guid={bot_guid} and no \
                             recovery hook configured"
                        ));
                    }
                }
            }
        };

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

    // ----- Scripted MCP: returns a fixed persona payload for personality_get,
    // counts personality_set calls separately. Lets recovery tests simulate
    // empty/blank/null/malformed personas and assert the persist-back happens. --

    struct ScriptedMcp {
        /// Raw JSON returned for memory.personality_get (the `result`-less form).
        get_response: serde_json::Value,
        get_count: Arc<AtomicUsize>,
        set_count: Arc<AtomicUsize>,
        /// Captures the persona string passed to the most recent personality_set.
        last_set_persona: std::sync::Mutex<Option<String>>,
    }

    impl McpCallable for ScriptedMcp {
        fn call<'a>(
            &'a self,
            tool: &'a str,
            args: serde_json::Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<serde_json::Value, anyhow::Error>,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                if tool == "memory.personality_set" {
                    self.set_count.fetch_add(1, Ordering::SeqCst);
                    if let Some(p) = args.get("persona").and_then(|v| v.as_str()) {
                        *self.last_set_persona.lock().unwrap() = Some(p.to_string());
                    }
                    Ok(serde_json::json!({"ok": true}))
                } else {
                    // personality_get (and any other read)
                    self.get_count.fetch_add(1, Ordering::SeqCst);
                    Ok(self.get_response.clone())
                }
            })
        }
    }

    fn make_scripted_mcp(
        get_response: serde_json::Value,
    ) -> (Arc<ScriptedMcp>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let get_count = Arc::new(AtomicUsize::new(0));
        let set_count = Arc::new(AtomicUsize::new(0));
        let mock = Arc::new(ScriptedMcp {
            get_response,
            get_count: get_count.clone(),
            set_count: set_count.clone(),
            last_set_persona: std::sync::Mutex::new(None),
        });
        (mock, get_count, set_count)
    }

    // ----- Mock recovery hook -----

    struct MockRecovery {
        call_count: Arc<AtomicUsize>,
        card: PersonalityCard,
    }

    impl PersonalityRecovery for MockRecovery {
        fn recover<'a>(
            &'a self,
            _bot_guid: i64,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<PersonalityCard, anyhow::Error>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                self.call_count.fetch_add(1, Ordering::SeqCst);
                Ok(self.card.clone())
            })
        }
    }

    fn make_mock_recovery(
        card: PersonalityCard,
    ) -> (Arc<dyn PersonalityRecovery + Send + Sync>, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let rec: Arc<dyn PersonalityRecovery + Send + Sync> =
            Arc::new(MockRecovery { call_count: counter.clone(), card });
        (rec, counter)
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

    // ----- Self-heal recovery tests (empty / blank / null / malformed) -----

    /// Build a cache wired with a scripted MCP + a recovery hook.
    fn cache_with_recovery(
        mcp: Arc<ScriptedMcp>,
        recovery: Arc<dyn PersonalityRecovery + Send + Sync>,
    ) -> PersonalityCache {
        PersonalityCache::new_with_options(
            mcp,
            300.0,
            10,
            frozen_clock(1000.0),
            None, // no v2 morph hook here; recovery returns a complete card
        )
        .with_recovery(recovery)
    }

    #[tokio::test]
    async fn test_empty_string_persona_triggers_recovery_and_persists() {
        // personality_get returns {"persona": ""} (bot 1083's live data shape).
        let (mcp, _get_c, set_c) = make_scripted_mcp(serde_json::json!({"persona": ""}));
        let (recovery, rec_c) = make_mock_recovery(make_card("Morenette"));
        let cache = cache_with_recovery(mcp, recovery);

        let card = cache
            .get(1083)
            .await
            .expect("empty persona must recover, NOT error");
        assert_eq!(card.name, "Morenette", "recovered identity preserved");

        assert_eq!(rec_c.load(Ordering::SeqCst), 1, "recovery called exactly once");
        assert_eq!(set_c.load(Ordering::SeqCst), 1, "recovered card persisted via personality_set");

        // Second get must be a cache hit — NO second recovery, NO spam.
        let card2 = cache.get(1083).await.expect("cache hit");
        assert_eq!(card2.name, "Morenette");
        assert_eq!(rec_c.load(Ordering::SeqCst), 1, "no re-recovery on cache hit (no spam)");
    }

    #[tokio::test]
    async fn test_blank_whitespace_persona_triggers_recovery() {
        let (mcp, _g, set_c) = make_scripted_mcp(serde_json::json!({"persona": "   \n\t "}));
        let (recovery, rec_c) = make_mock_recovery(make_card("Whitey"));
        let cache = cache_with_recovery(mcp, recovery);

        let card = cache.get(7).await.expect("blank persona must recover");
        assert_eq!(card.name, "Whitey");
        assert_eq!(rec_c.load(Ordering::SeqCst), 1);
        assert_eq!(set_c.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_null_persona_triggers_recovery() {
        // memory.personality_get returns {"persona": null} for a missing/NULL row.
        let (mcp, _g, set_c) = make_scripted_mcp(serde_json::json!({"persona": null}));
        let (recovery, rec_c) = make_mock_recovery(make_card("Nullsy"));
        let cache = cache_with_recovery(mcp, recovery);

        let card = cache.get(8).await.expect("null persona must recover");
        assert_eq!(card.name, "Nullsy");
        assert_eq!(rec_c.load(Ordering::SeqCst), 1);
        assert_eq!(set_c.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_missing_persona_key_triggers_recovery() {
        // Payload entirely missing the persona key.
        let (mcp, _g, _s) = make_scripted_mcp(serde_json::json!({"ok": true}));
        let (recovery, rec_c) = make_mock_recovery(make_card("Missy"));
        let cache = cache_with_recovery(mcp, recovery);

        let card = cache.get(9).await.expect("missing persona key must recover");
        assert_eq!(card.name, "Missy");
        assert_eq!(rec_c.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_malformed_json_persona_triggers_recovery() {
        let (mcp, _g, set_c) =
            make_scripted_mcp(serde_json::json!({"persona": "{not valid json"}));
        let (recovery, rec_c) = make_mock_recovery(make_card("Fixxy"));
        let cache = cache_with_recovery(mcp, recovery);

        let card = cache.get(10).await.expect("malformed persona must recover");
        assert_eq!(card.name, "Fixxy");
        assert_eq!(rec_c.load(Ordering::SeqCst), 1);
        assert_eq!(set_c.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_empty_object_persona_triggers_recovery() {
        // "{}" parses as JSON but fails PersonalityCard (missing required fields) —
        // must be treated as "not usable" → recover, not error.
        let (mcp, _g, _s) = make_scripted_mcp(serde_json::json!({"persona": "{}"}));
        let (recovery, rec_c) = make_mock_recovery(make_card("Empty"));
        let cache = cache_with_recovery(mcp, recovery);

        let card = cache.get(11).await.expect("'{}' persona must recover");
        assert_eq!(card.name, "Empty");
        assert_eq!(rec_c.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_valid_persona_does_not_trigger_recovery() {
        // A complete v2 card → existing path; recovery must NOT be called.
        let valid = make_card("Healthy");
        let persona_str = serde_json::to_string(&valid).unwrap();
        let (mcp, _g, set_c) =
            make_scripted_mcp(serde_json::json!({"persona": persona_str}));
        let (recovery, rec_c) = make_mock_recovery(make_card("ShouldNotAppear"));
        let cache = cache_with_recovery(mcp, recovery);

        let card = cache.get(12).await.expect("valid persona loads");
        assert_eq!(card.name, "Healthy", "stored persona used, not recovery");
        assert_eq!(rec_c.load(Ordering::SeqCst), 0, "recovery must NOT run for a valid persona");
        assert_eq!(set_c.load(Ordering::SeqCst), 0, "no persist for an already-valid persona");
    }

    #[tokio::test]
    async fn test_empty_persona_without_recovery_preserves_error() {
        // recovery=None (e.g. a minimal cache) + empty persona → Err, NOT a silent
        // degraded path. Preserves the old fail-loud behavior where unconfigured.
        let (mcp, _g, _s) = make_scripted_mcp(serde_json::json!({"persona": ""}));
        let cache = PersonalityCache::new_with_options(
            mcp,
            300.0,
            10,
            frozen_clock(1000.0),
            None,
        ); // NO .with_recovery

        let result = cache.get(13).await;
        assert!(
            result.is_err(),
            "empty persona with no recovery wired must still error (fail-loud)"
        );
    }
}
