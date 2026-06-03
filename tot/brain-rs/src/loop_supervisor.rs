// SPDX-License-Identifier: AGPL-3.0
//! LoopSupervisor — faithful Rust port of `brain_sidecar/loop.py LoopSupervisor`.
//!
//! # Concurrency model
//!
//! The Python supervisor uses asyncio cooperative multitasking, which makes plain
//! dict fields safe without locking. Rust needs explicit sharing:
//!
//! * Per-bot mutable state lives in a `StdMutex<HashMap<i64, PerBotState>>`. The
//!   outer lock is never held across `.await` points — it is taken briefly to
//!   read/write the map, then released before any async work.
//! * The per-bot `tick_lock: Arc<TokioMutex<()>>` serialises tick execution
//!   (poll tick vs SSE out-of-band tick), matching Python `asyncio.Lock`.
//! * `SseConsumer::on_events` is SYNC. The bridge closure calls `tokio::spawn`
//!   to enter async, reproducing Python's `async def _on_sse_events` behaviour.
//! * `StateStore::append_decision` is sync SQLite; called via
//!   `tokio::task::spawn_blocking`, mirroring Python `run_in_executor(None, ...)`.
//!
//! # Parity invariants (must not change)
//!
//! * Poll: lock held → drop tick, emit `tick_skipped_busy` record.
//! * Poll: lock free → acquire, drain `pending_sse`, run `_one_tick`.
//! * SSE handler: lock held → buffer items to `pending_sse`; return immediately.
//! * SSE handler: lock free → acquire, fire out-of-band `_one_tick`.
//! * Dedup fence: only on poll path when `triage.reason == "fresh_chat"` and no
//!   SSE inputs were present — mirrors Python `is_poll_fresh_chat` check.
//! * Wakeup clamp: `max(60_000, min(proposed_delta, 600_000))`, default 180_000.
//! * Tier-aware sleep: tier is read at the loop boundary each iteration so a tier
//!   change takes effect on the next sleep without restarting the task.
//! * Telemetry record emitted ALWAYS — even on early-return (no-decide) and
//!   exception paths (Python `finally: self._log(record)`).

use std::collections::HashMap;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::{Mutex as TokioMutex, Notify};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::decide::Decider;
use crate::dedup::SeenMemoryIds;
use crate::dispatch::Dispatcher;
use crate::memory_client::MemoryClient;
use crate::models::{Decision, TickState};
use crate::salience::SalienceScorer;
use crate::sse_consumer::SseConsumer;
use crate::state::StateStore;
use crate::triage::{TriageGate, CHAT_PREFIXES};

// ---------------------------------------------------------------------------
// DecisionLogWriter trait
// ---------------------------------------------------------------------------

/// Injectable telemetry sink. Mirrors Python `DecisionLogWriter` protocol.
pub trait DecisionLogWriter: Send + Sync {
    fn write(&self, record: &Value);
}

// ---------------------------------------------------------------------------
// Per-bot state bundle
// ---------------------------------------------------------------------------

struct PerBotState {
    /// Tick serializer — held while any tick (poll or SSE-triggered) runs.
    tick_lock: Arc<TokioMutex<()>>,
    /// Signals the polling loop to exit.
    stop_notify: Arc<Notify>,
    /// Last known TickState; updated after every completed tick.
    last_state: TickState,
    /// SSE items buffered while a tick is in-flight.
    pending_sse: Vec<Value>,
    /// Shared dedup set (cap 200) — fence between SSE and polling paths.
    seen_memory_ids: Arc<StdMutex<SeenMemoryIds>>,
    /// Organic-wakeup absolute Unix-ms target (None until first LLM tick).
    next_wakeup_ms: Option<i64>,
}

// ---------------------------------------------------------------------------
// LoopSupervisor
// ---------------------------------------------------------------------------

/// Manages per-bot periodic decision loops.
///
/// Faithful Rust port of `brain_sidecar/loop.py LoopSupervisor`.
pub struct LoopSupervisor {
    // ── injected collaborators ─────────────────────────────────────────────
    pub triage: Arc<TriageGate>,
    pub decider: Arc<Decider>,
    pub dispatcher: Arc<Dispatcher>,
    pub state_store: Arc<StateStore>,
    pub tick_interval_s: f64,
    pub reduced_tick_interval_s: f64,
    pub decision_log_writer: Arc<dyn DecisionLogWriter>,

    // ── optional memory wiring ─────────────────────────────────────────────
    pub memory_client: Option<Arc<MemoryClient>>,
    pub salience_scorer: Option<Arc<SalienceScorer>>,
    pub memory_recall_top_k: usize,

    // ── SSE settings ──────────────────────────────────────────────────────
    pub brain_sse_enabled: bool,
    pub memory_mcp_url: String,
    pub memory_bearer: String,
    pub brain_sse_coalesce_ms: u64,

    // ── per-bot runtime state ──────────────────────────────────────────────
    // StdMutex: never held across await — only locked briefly to read/modify.
    bots: StdMutex<HashMap<i64, PerBotState>>,
    tasks: StdMutex<HashMap<i64, JoinHandle<()>>>,
    sse_tasks: StdMutex<HashMap<i64, JoinHandle<()>>>,
    /// Shared cancel flags for SSE consumers so `stop()` can signal them.
    sse_cancel_flags: StdMutex<HashMap<i64, Arc<AtomicBool>>>,
}

impl LoopSupervisor {
    /// Full constructor — all collaborators supplied.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        triage: Arc<TriageGate>,
        decider: Arc<Decider>,
        dispatcher: Arc<Dispatcher>,
        state_store: Arc<StateStore>,
        tick_interval_s: f64,
        reduced_tick_interval_s: f64,
        decision_log_writer: Arc<dyn DecisionLogWriter>,
        memory_client: Option<Arc<MemoryClient>>,
        salience_scorer: Option<Arc<SalienceScorer>>,
        memory_recall_top_k: usize,
        brain_sse_enabled: bool,
        memory_mcp_url: String,
        memory_bearer: String,
        brain_sse_coalesce_ms: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            triage,
            decider,
            dispatcher,
            state_store,
            tick_interval_s,
            reduced_tick_interval_s,
            decision_log_writer,
            memory_client,
            salience_scorer,
            memory_recall_top_k,
            brain_sse_enabled,
            memory_mcp_url,
            memory_bearer,
            brain_sse_coalesce_ms,
            bots: StdMutex::new(HashMap::new()),
            tasks: StdMutex::new(HashMap::new()),
            sse_tasks: StdMutex::new(HashMap::new()),
            sse_cancel_flags: StdMutex::new(HashMap::new()),
        })
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Test-only: expose the per-bot tick_lock so tests can hold it to drive
    /// `tick_skipped_busy` behaviour.
    #[doc(hidden)]
    pub fn tick_lock_for_test(&self, bot_guid: i64) -> Option<Arc<TokioMutex<()>>> {
        let bots = self.bots.lock().unwrap();
        bots.get(&bot_guid).map(|pb| Arc::clone(&pb.tick_lock))
    }

    /// Return bot_guids whose polling task is still running.
    pub fn list_active(&self) -> Vec<i64> {
        let tasks = self.tasks.lock().unwrap();
        tasks
            .iter()
            .filter(|(_, h)| !h.is_finished())
            .map(|(g, _)| *g)
            .collect()
    }

    /// Start the brain loop for `bot_guid`. Idempotent if already running.
    ///
    /// Mirrors Python `start()`.
    pub fn start(self: &Arc<Self>, bot_guid: i64) {
        // Idempotency guard.
        {
            let tasks = self.tasks.lock().unwrap();
            if let Some(h) = tasks.get(&bot_guid) {
                if !h.is_finished() {
                    return;
                }
            }
        }

        let tick_lock = Arc::new(TokioMutex::new(()));
        let stop_notify = Arc::new(Notify::new());
        let seen = Arc::new(StdMutex::new(SeenMemoryIds::new(200)));

        {
            let mut bots = self.bots.lock().unwrap();
            bots.insert(
                bot_guid,
                PerBotState {
                    tick_lock: Arc::clone(&tick_lock),
                    stop_notify: Arc::clone(&stop_notify),
                    last_state: TickState::new(bot_guid),
                    pending_sse: Vec::new(),
                    seen_memory_ids: Arc::clone(&seen),
                    next_wakeup_ms: None,
                },
            );
        }

        // Spawn polling task.
        let this = Arc::clone(self);
        let poll_handle = tokio::spawn(async move {
            this._run(bot_guid).await;
        });
        {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(bot_guid, poll_handle);
        }

        // Optionally spawn SSE consumer task.
        if self.brain_sse_enabled && !self.memory_mcp_url.is_empty() {
            self._start_sse_consumer(bot_guid, Arc::clone(&tick_lock), Arc::clone(&seen));
        }

        info!("brain-loop started bot_guid={}", bot_guid);
    }

    /// Stop the brain loop for `bot_guid`.
    ///
    /// Mirrors Python `stop()` ordering:
    /// 1. Signal stop.
    /// 2. Cancel + await SSE task (timeout 2s, best-effort).
    /// 3. Await polling task (timeout 10s, then abort).
    /// 4. Clean up all per-bot maps.
    pub async fn stop(&self, bot_guid: i64) {
        // Signal stop to the polling loop.
        {
            let bots = self.bots.lock().unwrap();
            if let Some(pb) = bots.get(&bot_guid) {
                pb.stop_notify.notify_waiters();
            }
        }

        // Signal SSE consumer cancellation.
        {
            let flags = self.sse_cancel_flags.lock().unwrap();
            if let Some(flag) = flags.get(&bot_guid) {
                flag.store(true, Ordering::Relaxed);
            }
        }

        // Await SSE task (timeout 2s).
        let sse_task = {
            let mut sse_tasks = self.sse_tasks.lock().unwrap();
            sse_tasks.remove(&bot_guid)
        };
        if let Some(t) = sse_task {
            if !t.is_finished() {
                let _ = tokio::time::timeout(tokio::time::Duration::from_secs(2), async {
                    // Abort is the only way to cancel a spawned task from outside.
                    // The abort handle was consumed by JoinHandle — abort via abort().
                    t.abort();
                    // Brief yield so tokio processes the abort.
                    tokio::task::yield_now().await;
                })
                .await;
            }
        }

        // Await polling task (timeout 10s, then abort). Mirrors Python:
        //   try: await asyncio.wait_for(task, 10)
        //   except asyncio.TimeoutError: task.cancel()
        let poll_task = {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.remove(&bot_guid)
        };
        if let Some(t) = poll_task {
            if !t.is_finished() {
                match tokio::time::timeout(tokio::time::Duration::from_secs(10), t).await {
                    Ok(_) => {}
                    Err(_) => {
                        // Timeout — task was already moved into wait_for; no handle left
                        // to call abort(). The stop_notify.notify_waiters() above will
                        // cause the loop to exit on the next sleep boundary.
                    }
                }
            }
        }

        // Clean up per-bot state.
        {
            let mut bots = self.bots.lock().unwrap();
            bots.remove(&bot_guid);
        }
        {
            let mut flags = self.sse_cancel_flags.lock().unwrap();
            flags.remove(&bot_guid);
        }

        info!("brain-loop stopped bot_guid={}", bot_guid);
    }

    /// Stop all active bot loops.
    pub async fn stop_all(&self) {
        let guids: Vec<i64> = {
            let bots = self.bots.lock().unwrap();
            bots.keys().copied().collect()
        };
        for guid in guids {
            self.stop(guid).await;
        }
    }

    // ------------------------------------------------------------------
    // SubsetGate integration (Plan 3 T21)
    // ------------------------------------------------------------------

    /// Called by SubsetGate to activate a bot's brain loop.
    /// The bot must already be enrolled (status='active') in `state_store`.
    pub fn enroll_bot(self: &Arc<Self>, bot_guid: i64) {
        self.start(bot_guid);
    }

    /// Called by SubsetGate to deactivate a bot.
    /// Sets state_store status="released" AFTER stopping, matching Python.
    pub async fn release_bot(&self, bot_guid: i64) {
        self.stop(bot_guid).await;
        let _ = self.state_store.set_status(bot_guid, "released");
    }

    // ------------------------------------------------------------------
    // Internal: polling loop (_run)
    // ------------------------------------------------------------------

    async fn _run(self: &Arc<Self>, bot_guid: i64) {
        loop {
            // Retrieve the tick_lock for this bot — if the entry is gone, exit.
            let tick_lock_opt = {
                let bots = self.bots.lock().unwrap();
                bots.get(&bot_guid).map(|pb| Arc::clone(&pb.tick_lock))
            };
            let Some(tick_lock) = tick_lock_opt else {
                break; // bot removed
            };

            match tick_lock.try_lock() {
                Err(_already_locked) => {
                    // Previous tick still running — drop this tick entirely.
                    // Python: self._log({...tick_skipped_busy...})
                    self._log(json!({
                        "event_id": _eid(),
                        "ts_ms": _now_ms(),
                        "bot_guid": bot_guid,
                        "triage_reason": "tick_skipped_busy",
                        "llm_called": false,
                        "llm_latency_ms": null,
                        "decision_kind": null,
                        "tool": null,
                        "confidence": null,
                        "dispatch_result": null,
                        "error": null,
                    }));
                }
                Ok(_guard) => {
                    // Drain any pending SSE items accumulated during sleep.
                    let (last_state, sse_inputs) = {
                        let mut bots = self.bots.lock().unwrap();
                        match bots.get_mut(&bot_guid) {
                            None => break, // bot removed
                            Some(pb) => {
                                let last_state = pb.last_state.clone();
                                let sse_inputs = if pb.pending_sse.is_empty() {
                                    None
                                } else {
                                    let items: Vec<Value> =
                                        pb.pending_sse.drain(..).collect();
                                    let mut m = HashMap::new();
                                    m.insert(
                                        "fresh_chat".to_string(),
                                        Value::Array(items),
                                    );
                                    Some(m)
                                };
                                (last_state, sse_inputs)
                            }
                        }
                    };

                    let new_state =
                        self._one_tick(bot_guid, &last_state, sse_inputs.as_ref()).await;

                    // Update last_state while the tick_lock guard is still held.
                    {
                        let mut bots = self.bots.lock().unwrap();
                        if let Some(pb) = bots.get_mut(&bot_guid) {
                            pb.last_state = new_state;
                        }
                    }
                    // _guard dropped here — tick lock released.
                }
            }

            // ── Tier-aware sleep ─────────────────────────────────────────
            // Tier is read at the loop boundary so a tier change (full→reduced)
            // takes effect on the next sleep without restarting the task.
            let (interval_s, stop_notify_opt) = {
                let bots = self.bots.lock().unwrap();
                let stop = bots.get(&bot_guid).map(|pb| Arc::clone(&pb.stop_notify));
                let tier = self
                    .state_store
                    .get_tier(bot_guid)
                    .unwrap_or_else(|_| "full".to_string());
                let interval = if tier == "reduced" {
                    self.reduced_tick_interval_s
                } else {
                    self.tick_interval_s
                };
                (interval, stop)
            };
            let Some(stop_notify) = stop_notify_opt else {
                break;
            };
            let dur = tokio::time::Duration::from_secs_f64(interval_s);
            tokio::select! {
                _ = stop_notify.notified() => break,
                _ = tokio::time::sleep(dur) => {}
            }
        }
        info!("brain-loop exited bot_guid={}", bot_guid);
    }

    // ------------------------------------------------------------------
    // Internal: one tick (_one_tick)
    // ------------------------------------------------------------------

    /// Execute one brain cycle — triage → (optional) decide → dispatch → log.
    ///
    /// The telemetry record is emitted ALWAYS, even on early-return paths
    /// (Python `finally: self._log(record)`).
    async fn _one_tick(
        self: &Arc<Self>,
        bot_guid: i64,
        last_state: &TickState,
        sse_inputs: Option<&HashMap<String, Value>>,
    ) -> TickState {
        let now_ms = _now_ms();
        let event_id = _eid();

        let has_sse_chat = sse_inputs
            .and_then(|m| m.get("fresh_chat"))
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);

        let mut record = json!({
            "event_id": event_id,
            "ts_ms": now_ms,
            "bot_guid": bot_guid,
            "triage_reason": null,
            "llm_called": false,
            "llm_latency_ms": null,
            "decision_kind": null,
            "tool": null,
            "confidence": null,
            "dispatch_result": null,
            "error": null,
            "wakeup_at_ms": null,
            "wakeup_in_ms": null,
            "event_source": if has_sse_chat { "sse" } else { "poll" },
        });

        // Run the inner tick; whatever happens, emit the record afterward.
        let result = self
            ._one_tick_inner(
                bot_guid,
                last_state,
                sse_inputs,
                now_ms,
                &event_id,
                &mut record,
            )
            .await;

        // Always emit (Python `finally: self._log(record)`).
        self._log(record);

        result
    }

    async fn _one_tick_inner(
        self: &Arc<Self>,
        bot_guid: i64,
        last_state: &TickState,
        sse_inputs: Option<&HashMap<String, Value>>,
        now_ms: i64,
        event_id: &str,
        record: &mut Value,
    ) -> TickState {
        // ── Triage ──────────────────────────────────────────────────────
        let next_wakeup_at_ms = {
            let bots = self.bots.lock().unwrap();
            bots.get(&bot_guid).and_then(|pb| pb.next_wakeup_ms)
        };

        let triage = self
            .triage
            .evaluate(bot_guid, last_state, now_ms, sse_inputs, next_wakeup_at_ms)
            .await;

        record["triage_reason"] = Value::String(triage.reason.clone());

        if !triage.should_decide {
            return TickState {
                bot_guid,
                last_tick_ms: now_ms,
                last_decision_id: last_state.last_decision_id.clone(),
            };
        }

        // ── v0.2.2 Poll/SSE dedup fence ──────────────────────────────────
        // Applied only on the POLL path when triage returned "fresh_chat"
        // but sse_inputs had no fresh_chat (i.e. this is a polled chat).
        let mut hot_inputs = triage.hot_inputs.clone();
        let is_poll_fresh_chat = triage.reason == "fresh_chat" && !has_sse_fresh_chat(sse_inputs);

        if is_poll_fresh_chat {
            let seen_opt = {
                let bots = self.bots.lock().unwrap();
                bots.get(&bot_guid).map(|pb| Arc::clone(&pb.seen_memory_ids))
            };
            if let Some(seen_lock) = seen_opt {
                let raw = hot_inputs
                    .get("fresh_chat")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                let filtered: Vec<Value> = {
                    let seen = seen_lock.lock().unwrap();
                    raw.iter()
                        .filter(|item| {
                            let mid = item
                                .get("memory_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            !seen.seen(mid)
                        })
                        .cloned()
                        .collect()
                };

                if filtered.is_empty() {
                    // All already decided via SSE → skip LLM.
                    record["triage_reason"] = Value::String("no_change".to_string());
                    return TickState {
                        bot_guid,
                        last_tick_ms: now_ms,
                        last_decision_id: last_state.last_decision_id.clone(),
                    };
                }

                // Mark survivors.
                {
                    let mut seen = seen_lock.lock().unwrap();
                    for item in &filtered {
                        let mid = item
                            .get("memory_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !mid.is_empty() {
                            seen.mark(mid);
                        }
                    }
                }
                hot_inputs.insert("fresh_chat".to_string(), Value::Array(filtered));
            }
        }

        // ── Optional memory recall ────────────────────────────────────────
        if let Some(ref mc) = self.memory_client {
            let query = _recall_query_from_hot_inputs(&hot_inputs);
            match mc
                .recall(&bot_guid.to_string(), &query, self.memory_recall_top_k)
                .await
            {
                Ok(recalled) => {
                    let recalled_json: Vec<Value> = recalled
                        .iter()
                        .map(|ep| {
                            json!({
                                "episode_id": ep.episode_id,
                                "content_text": ep.content_text,
                                "timestamp": ep.timestamp,
                                "salience_score": ep.salience_score,
                                "score": ep.score,
                            })
                        })
                        .collect();
                    hot_inputs.insert(
                        "recalled_memories".to_string(),
                        Value::Array(recalled_json),
                    );
                }
                Err(e) => {
                    warn!("memory_recall_failed bot_guid={} err={:?}", bot_guid, e);
                    hot_inputs
                        .insert("recalled_memories".to_string(), Value::Array(vec![]));
                }
            }
        }

        // ── LLM decide ───────────────────────────────────────────────────
        record["llm_called"] = Value::Bool(true);
        let triage_reason_str = triage.reason.clone();

        let (decision, llm_latency_ms, at_cap) = self
            .decider
            .decide(bot_guid, &hot_inputs, Some(&triage_reason_str))
            .await;

        record["at_cap"] = Value::Bool(at_cap);
        record["decision_kind"] = serde_json::to_value(&decision.kind)
            .unwrap_or(Value::Null);
        record["tool"] = decision
            .tool
            .as_deref()
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null);
        record["confidence"] =
            serde_json::Number::from_f64(decision.confidence)
                .map(Value::Number)
                .unwrap_or(Value::Null);
        record["llm_latency_ms"] = llm_latency_ms
            .and_then(|ms| serde_json::Number::from_f64(ms).map(Value::Number))
            .unwrap_or(Value::Null);

        // ── Wakeup clamp ─────────────────────────────────────────────────
        // V3.7.1: brain-returned delta is clamped to [60_000, 600_000] ms.
        // None / non-positive → 3-min (180_000 ms) default.
        let delta: i64 = match decision.wakeup_in_ms {
            Some(d) if d > 0 => d.clamp(60_000, 600_000),
            _ => 180_000,
        };
        let next_wakeup = now_ms + delta;
        {
            let mut bots = self.bots.lock().unwrap();
            if let Some(pb) = bots.get_mut(&bot_guid) {
                pb.next_wakeup_ms = Some(next_wakeup);
            }
        }
        record["wakeup_at_ms"] = Value::Number(next_wakeup.into());
        record["wakeup_in_ms"] = Value::Number(delta.into());

        // ── Dispatch ─────────────────────────────────────────────────────
        let tier = self
            .state_store
            .get_tier(bot_guid)
            .unwrap_or_else(|_| "full".to_string());

        let dispatch_result = match self
            .dispatcher
            .dispatch(bot_guid, &decision, &tier)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                record["error"] = Value::String(format!("tick_exception: {}", e));
                return TickState {
                    bot_guid,
                    last_tick_ms: now_ms,
                    last_decision_id: Some(event_id.to_string()),
                };
            }
        };

        record["dispatch_result"] =
            Value::String(dispatch_result.disposition.clone());
        if !dispatch_result.error.is_empty() {
            record["error"] = Value::String(dispatch_result.error.clone());
        }

        // ── append_decision via spawn_blocking ────────────────────────────
        // Mirrors Python `run_in_executor(None, self._append_decision_sync, ...)`.
        let store = Arc::clone(&self.state_store);
        let dec_clone = decision.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = store.append_decision(bot_guid, now_ms, &dec_clone);
        })
        .await;

        // ── Optional salience score + memory write ─────────────────────────
        if let (Some(mc), Some(scorer)) =
            (&self.memory_client, &self.salience_scorer)
        {
            let mc = Arc::clone(mc);
            let scorer = Arc::clone(scorer);
            let perception_val = Value::Object(
                _perception_from_hot_inputs(&hot_inputs)
                    .into_iter()
                    .collect(),
            );
            let decision_val = serde_json::to_value(&decision).unwrap_or_default();
            let action_result_val = Value::Object(
                _action_result_from_dispatch(&dispatch_result)
                    .into_iter()
                    .collect(),
            );

            let salience = scorer.score(&perception_val, &decision_val, &action_result_val);

            if salience >= scorer.threshold {
                let content = _episode_text(&hot_inputs, &decision, &dispatch_result);
                let episode_type = hot_inputs
                    .get("episode_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("observation")
                    .to_string();
                let ts_iso = _iso_from_ms(now_ms);
                let bot_id = bot_guid.to_string();
                if let Err(e) = mc
                    .write_episode(&bot_id, &content, &episode_type, &ts_iso, salience)
                    .await
                {
                    warn!(
                        "memory_write_failed bot_guid={} err={:?}",
                        bot_guid, e
                    );
                }
            }
        }

        TickState {
            bot_guid,
            last_tick_ms: now_ms,
            last_decision_id: Some(event_id.to_string()),
        }
    }

    // ------------------------------------------------------------------
    // Internal: SSE consumer spawn
    // ------------------------------------------------------------------

    fn _start_sse_consumer(
        self: &Arc<Self>,
        bot_guid: i64,
        tick_lock: Arc<TokioMutex<()>>,
        seen: Arc<StdMutex<SeenMemoryIds>>,
    ) {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        {
            let mut flags = self.sse_cancel_flags.lock().unwrap();
            flags.insert(bot_guid, Arc::clone(&cancel_flag));
        }

        // Build the SYNC on_events closure.
        // Bridges to async by spawning a tokio task — reproduces Python's
        // `async def _on_sse_events`.
        let this_cb = Arc::clone(self);
        let tick_lock_cb = Arc::clone(&tick_lock);
        let on_events = move |items: Vec<Value>| {
            let sup = Arc::clone(&this_cb);
            let lock = Arc::clone(&tick_lock_cb);

            tokio::spawn(async move {
                // Reproduce Python _on_sse_events:
                //   if lock is gone (bot released) → return
                //   if lock.locked() → buffer to pending_sse
                //   else → acquire lock, fire out-of-band _one_tick
                let bot_still_exists = {
                    let bots = sup.bots.lock().unwrap();
                    bots.contains_key(&bot_guid)
                };
                if !bot_still_exists {
                    return; // bot released
                }

                match lock.try_lock() {
                    Err(_locked) => {
                        // Tick in-flight — buffer items for the next poll drain.
                        let mut bots = sup.bots.lock().unwrap();
                        if let Some(pb) = bots.get_mut(&bot_guid) {
                            pb.pending_sse.extend(items);
                        }
                    }
                    Ok(_guard) => {
                        // Lock free — fire out-of-band tick.
                        let last_state = {
                            let bots = sup.bots.lock().unwrap();
                            match bots.get(&bot_guid) {
                                Some(pb) => pb.last_state.clone(),
                                None => return, // released between check and here
                            }
                        };
                        let mut sse_map = HashMap::new();
                        sse_map.insert(
                            "fresh_chat".to_string(),
                            Value::Array(items),
                        );
                        let new_state = sup
                            ._one_tick(bot_guid, &last_state, Some(&sse_map))
                            .await;
                        {
                            let mut bots = sup.bots.lock().unwrap();
                            if let Some(pb) = bots.get_mut(&bot_guid) {
                                pb.last_state = new_state;
                            }
                        }
                        // _guard dropped — tick lock released.
                    }
                }
            });
        };

        let consumer = SseConsumer {
            bot_guid,
            memory_url: self.memory_mcp_url.clone(),
            bearer: self.memory_bearer.clone(),
            state_store: Arc::clone(&self.state_store),
            on_events: Arc::new(on_events),
            prefixes: CHAT_PREFIXES.iter().map(|s| s.to_string()).collect(),
            coalesce_window_ms: self.brain_sse_coalesce_ms,
            dedup_set: seen,
            cancel: Arc::clone(&cancel_flag),
        };

        let sse_handle = tokio::spawn(async move {
            consumer.run_until_complete(None).await;
        });

        {
            let mut sse_tasks = self.sse_tasks.lock().unwrap();
            sse_tasks.insert(bot_guid, sse_handle);
        }
    }

    // ------------------------------------------------------------------
    // Internal: telemetry log
    // ------------------------------------------------------------------

    fn _log(&self, record: Value) {
        // Mirror Python: `try: self.decision_log_writer.write(record)`
        //                 `except Exception: log.exception(...)`
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.decision_log_writer.write(&record);
        }));
        if let Err(e) = result {
            warn!("decision_log_write_panicked: {:?}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Module-level helper functions (mirrors loop.py)
// ---------------------------------------------------------------------------

fn _now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn _eid() -> String {
    Uuid::new_v4().to_string()
}

/// Render a Unix-ms timestamp as ISO-8601 UTC string.
/// Mirrors Python `_iso_from_ms` / `datetime.fromtimestamp(..., tz=utc).isoformat()`.
fn _iso_from_ms(ms: i64) -> String {
    let secs = (ms.max(0) / 1000) as u64;
    let millis = (ms.max(0) % 1000) as u32;
    let dt = _secs_to_datetime(secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}+00:00",
        dt.0, dt.1, dt.2, dt.3, dt.4, dt.5, millis
    )
}

/// Convert Unix seconds to (year, month, day, hour, min, sec) via
/// proleptic Gregorian calendar arithmetic.
fn _secs_to_datetime(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let sec = (secs % 60) as u32;
    let mins = secs / 60;
    let min = (mins % 60) as u32;
    let hours = mins / 60;
    let hour = (hours % 24) as u32;
    let days = (hours / 24) as u32;
    // Convert days-since-epoch to Gregorian calendar via Julian Day.
    let jd = days + 2_440_588u32; // Unix epoch = JD 2440588
    let l = jd + 68569;
    let n = (4 * l) / 146097;
    let l = l - (146097 * n + 3) / 4;
    let i = (4000 * (l + 1)) / 1461001;
    let l = l - (1461 * i) / 4 + 31;
    let j = (80 * l) / 2447;
    let day = l - (2447 * j) / 80;
    let l = j / 11;
    let month = j + 2 - 12 * l;
    let year = 100 * (n - 49) + i + l;
    (year, month, day, hour, min, sec)
}

/// Build a recall query from hot_inputs. Mirrors Python `_recall_query_from_hot_inputs`.
fn _recall_query_from_hot_inputs(hot_inputs: &HashMap<String, Value>) -> String {
    if let Some(arr) = hot_inputs.get("fresh_chat").and_then(|v| v.as_array()) {
        if let Some(first) = arr.first() {
            let sender = first
                .get("from")
                .or_else(|| first.get("sender"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let text = first
                .get("text")
                .or_else(|| first.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !sender.is_empty() || !text.is_empty() {
                let raw = format!("{}: {}", sender, text);
                let trimmed = raw.trim_matches(':').trim().to_string();
                return if trimmed.is_empty() {
                    "recent chat".to_string()
                } else {
                    trimmed
                };
            }
        }
    }
    if let Some(ep) = hot_inputs.get("episode_type").and_then(|v| v.as_str()) {
        if !ep.is_empty() {
            return format!("recent {}", ep);
        }
    }
    "current situation".to_string()
}

/// Project hot_inputs to SalienceScorer's perception dict.
/// Mirrors Python `_perception_from_hot_inputs`.
fn _perception_from_hot_inputs(
    hot_inputs: &HashMap<String, Value>,
) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for k in ["episode_type", "source", "salience_hint"] {
        if let Some(v) = hot_inputs.get(k) {
            out.insert(k.to_string(), v.clone());
        }
    }
    out
}

/// Extract salience-relevant fields from DispatchResult.
/// Mirrors Python `_action_result_from_dispatch`.
fn _action_result_from_dispatch(
    result: &crate::dispatch::DispatchResult,
) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert(
        "disposition".to_string(),
        Value::String(result.disposition.clone()),
    );
    m.insert(
        "error".to_string(),
        if result.error.is_empty() {
            Value::Null
        } else {
            Value::String(result.error.clone())
        },
    );
    m.insert("outcome".to_string(), Value::Null);
    m.insert("event".to_string(), Value::Null);
    m
}

/// Render the episode text written to memory.
/// Mirrors Python `_episode_text` (max 4000 chars).
fn _episode_text(
    hot_inputs: &HashMap<String, Value>,
    decision: &Decision,
    result: &crate::dispatch::DispatchResult,
) -> String {
    let payload = json!({
        "decision_kind": serde_json::to_value(&decision.kind).unwrap_or_default(),
        "tool": decision.tool,
        "reasoning": decision.reasoning.chars().take(200).collect::<String>(),
        "disposition": result.disposition,
        "triage": hot_inputs.get("episode_type").and_then(|v| v.as_str()).unwrap_or("tick"),
    });
    payload.to_string().chars().take(4000).collect()
}

/// Return true iff sse_inputs contains a non-empty fresh_chat list.
#[inline]
fn has_sse_fresh_chat(sse_inputs: Option<&HashMap<String, Value>>) -> bool {
    sse_inputs
        .and_then(|m| m.get("fresh_chat"))
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}
