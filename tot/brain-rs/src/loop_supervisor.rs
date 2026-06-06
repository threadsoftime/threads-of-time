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

use std::collections::{BTreeMap, HashMap};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tot_goal_contract::GoalSink;

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

    // ── G2: goal emission (additive, gated) ─────────────────────────────────
    /// When `Some`, goal emission is active. When `None` (the default),
    /// the supervisor behaves exactly as before — byte-identical parity.
    goal_sink: Option<Arc<dyn GoalSink>>,
    /// Dedup map: bot_guid → last emitted goal_id. Prevents re-emitting the
    /// same goal every tick. StdMutex — never held across an `.await`.
    emitted_goals: StdMutex<HashMap<i64, String>>,
}

impl LoopSupervisor {
    /// Full constructor — all collaborators supplied.
    ///
    /// The `goal_sink` parameter controls goal emission (G2):
    /// - `None` → pure parity mode (no goal emission; behaviour identical to before).
    /// - `Some(sink)` → deterministic `Grind` goal emitted at the end of each successful
    ///   tick when the bot is below cap. Goals are deduplicated per (bot, level).
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
        goal_sink: Option<Arc<dyn GoalSink>>,
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
            goal_sink,
            emitted_goals: StdMutex::new(HashMap::new()),
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
        //
        // Key: save the AbortHandle BEFORE moving the JoinHandle into
        // `timeout(...)`.  `tokio::time::timeout` consumes the JoinHandle —
        // on timeout there is nothing left to abort() unless we kept the
        // handle separately.  Without this, a mid-LLM-call tick (up to
        // LLM_TIMEOUT_S=60s) would block stop() for the full 60s instead of
        // the intended ≤10s.
        let poll_task = {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.remove(&bot_guid)
        };
        if let Some(t) = poll_task {
            if !t.is_finished() {
                let abort_handle = t.abort_handle();
                match tokio::time::timeout(tokio::time::Duration::from_secs(10), t).await {
                    Ok(_) => {}
                    Err(_timeout) => {
                        // Task is still running after 10s — abort it, matching
                        // Python's `task.cancel()` on asyncio.TimeoutError.
                        abort_handle.abort();
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
        //
        // Python: try: ... except Exception as e: record["error"] = f"tick_exception: {e}"
        //         finally: self._log(record)
        //
        // Rust: _one_tick_inner returns Result<TickState, anyhow::Error>.
        // Errors bubble out via `?` (matching Python's implicit exception
        // propagation), are caught here, written into record["error"], and then
        // the record is always emitted (the `finally` equivalent).
        let tick_result = self
            ._one_tick_inner(
                bot_guid,
                last_state,
                sse_inputs,
                now_ms,
                &event_id,
                &mut record,
            )
            .await;

        // Mirror Python except + finally: on Err set the error field, then
        // always emit the record.
        let result = match tick_result {
            Ok(state) => state,
            Err(e) => {
                // Python: except Exception as e: log.exception(...); record["error"] = ...
                warn!("uncaught tick exception bot={} err={:?}", bot_guid, e);
                record["error"] = Value::String(format!("tick_exception: {}", e));
                // Return same TickState shape Python does: last_decision_id = record["event_id"]
                // (i.e. current event_id, not the prior state's — Python always returns
                // TickState(..., last_decision_id=record["event_id"]) at the outer level).
                TickState {
                    bot_guid,
                    last_tick_ms: now_ms,
                    last_decision_id: Some(event_id.clone()),
                }
            }
        };

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
    ) -> Result<TickState, anyhow::Error> {
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
            return Ok(TickState {
                bot_guid,
                last_tick_ms: now_ms,
                last_decision_id: last_state.last_decision_id.clone(),
            });
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
                    return Ok(TickState {
                        bot_guid,
                        last_tick_ms: now_ms,
                        last_decision_id: last_state.last_decision_id.clone(),
                    });
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

        // Mirror Python: dispatcher.dispatch raises → caught by outer except,
        // record["error"] set, finally logs. We use `?` to propagate, which
        // is caught by the outer _one_tick handler.
        let dispatch_result = self
            .dispatcher
            .dispatch(bot_guid, &decision, &tier)
            .await?;

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

        // ── G2: goal emission (additive, gated) ───────────────────────────────
        // Called AFTER all existing telemetry/dispatch/memory paths.
        // With goal_sink=None this is a complete no-op — parity preserved.
        self.maybe_emit_goal(bot_guid, &hot_inputs).await;

        Ok(TickState {
            bot_guid,
            last_tick_ms: now_ms,
            last_decision_id: Some(event_id.to_string()),
        })
    }

    /// Emit a goal to the sink if the bot is below cap and the goal has not
    /// already been emitted for this (bot, level) combination.
    ///
    /// Gated on `self.goal_sink`. With `None`, this is a complete no-op.
    /// Called at the end of `_one_tick_inner` — after dispatch + telemetry.
    /// `pub(crate)` for unit tests.
    pub(crate) async fn maybe_emit_goal(&self, bot_guid: i64, hot_inputs: &HashMap<String, Value>) {
        let sink = match &self.goal_sink {
            Some(s) => Arc::clone(s),
            None => return, // ← pure parity path (no-op)
        };

        // Resolve max_level from the state_store if needed — re-use state_summary
        // from hot_inputs (same field path as decide()'s at_cap logic).
        let state_summary = match hot_inputs.get("state_summary") {
            Some(v) => v.clone(),
            None => return, // no observation this tick → skip
        };

        // Derive max_player_level — stored on the Decider, but we keep it
        // accessible via the state_store path. For simplicity we use a
        // hard-wired default from config; in practice this value is also
        // derived from the Decider::max_player_level field which we cannot
        // reach here without coupling. We rely on the caller (app.rs) to set
        // the same cap. For now, resolve via the Decider's public field.
        let max_level = self.decider.max_player_level;

        let envelope = match crate::goal_emitter::synthesize_grind(bot_guid, &state_summary, max_level) {
            Some(e) => e,
            None => return, // at cap → no goal
        };

        // Dedup: emit once per (bot, level). goal_id encodes both.
        {
            let mut emitted = self.emitted_goals.lock().unwrap();
            if emitted.get(&bot_guid).map(|id| id == &envelope.goal_id).unwrap_or(false) {
                return; // already emitted this (bot, level) goal
            }
            emitted.insert(bot_guid, envelope.goal_id.clone());
        }

        // Emit — best-effort: log on error, do NOT propagate (must not perturb the tick).
        if let Err(e) = sink.set_goal(bot_guid as u64, envelope).await {
            warn!("goal_emit_failed bot_guid={} err={:?}", bot_guid, e);
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
///
/// Python serialises with `json.dumps(payload, ensure_ascii=False, sort_keys=True)`.
/// We match that by inserting into a `BTreeMap` (which iterates in key order) before
/// serialising, giving alphabetical key order: decision_kind, disposition, reasoning,
/// triage, tool.
fn _episode_text(
    hot_inputs: &HashMap<String, Value>,
    decision: &Decision,
    result: &crate::dispatch::DispatchResult,
) -> String {
    let mut payload: BTreeMap<String, Value> = BTreeMap::new();
    payload.insert("decision_kind".to_string(), serde_json::to_value(&decision.kind).unwrap_or_default());
    payload.insert("disposition".to_string(), Value::String(result.disposition.clone()));
    payload.insert("reasoning".to_string(), Value::String(decision.reasoning.chars().take(200).collect::<String>()));
    payload.insert("triage".to_string(), Value::String(
        hot_inputs.get("episode_type").and_then(|v| v.as_str()).unwrap_or("tick").to_string()
    ));
    payload.insert("tool".to_string(), serde_json::to_value(&decision.tool).unwrap_or_default());
    serde_json::to_string(&payload)
        .unwrap_or_default()
        .chars()
        .take(4000)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Decision, DecisionKind};
    use crate::dispatch::DispatchResult;
    use std::pin::Pin;
    use std::future::Future;
    use tot_goal_contract::{GoalEnvelope, GoalSink};

    // ── TestGoalSink — captures set_goal calls without async_trait ──────────

    struct TestGoalSink {
        tx: tokio::sync::mpsc::UnboundedSender<(u64, GoalEnvelope)>,
    }

    impl GoalSink for TestGoalSink {
        fn set_goal<'life0, 'async_trait>(
            &'life0 self,
            bot_guid: u64,
            envelope: GoalEnvelope,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            let _ = self.tx.send((bot_guid, envelope));
            Box::pin(async { Ok(()) })
        }
    }

    fn make_test_sink() -> (Arc<dyn GoalSink>, tokio::sync::mpsc::UnboundedReceiver<(u64, GoalEnvelope)>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Arc::new(TestGoalSink { tx }), rx)
    }

    /// Build a minimal LoopSupervisor for unit tests.
    /// Uses the simplest possible Decider (new_test_with_max_level) and no-op
    /// collaborators for everything not under test.
    fn minimal_supervisor(goal_sink: Option<Arc<dyn GoalSink>>, max_level: u32) -> Arc<LoopSupervisor> {
        use crate::decide::Decider;
        use crate::dispatch::Dispatcher;
        use crate::models::PersonalityCard;
        use crate::personality::McpCallable;
        use crate::state::StateStore;
        use crate::triage::TriageGate;
        use tempfile::NamedTempFile;

        struct NoopMcp;
        impl McpCallable for NoopMcp {
            fn call<'a>(&'a self, _: &'a str, _: serde_json::Value)
                -> Pin<Box<dyn Future<Output = Result<serde_json::Value, anyhow::Error>> + Send + 'a>>
            {
                Box::pin(async { Ok(serde_json::json!({})) })
            }
        }

        struct TestLogWriter;
        impl DecisionLogWriter for TestLogWriter {
            fn write(&self, _: &serde_json::Value) {}
        }

        let mcp: Arc<dyn McpCallable> = Arc::new(NoopMcp);
        let f = NamedTempFile::new().unwrap();
        let store = Arc::new(StateStore::open(f.path().to_str().unwrap()).unwrap());
        store.migrate().unwrap();

        // Keep the tempfile alive for the test by leaking it (small, test-only)
        std::mem::forget(f);

        let card = PersonalityCard {
            name: "T".into(), race: "Human".into(), class_: "Warrior".into(),
            backstory: "T".into(), talkativeness: 0.5, courage: 0.5, greed: 0.0,
            attitude_to_master: 0.0, party_invite_policy: "none".into(),
            pvp_appetite: None, raid_appetite: None, completionist_streak: None,
            gold_motivation: None, profession_appetite: None,
        };
        let decider = Arc::new(Decider::new_test_with_max_level(1001, card, "{}", max_level));
        let triage = Arc::new(TriageGate::new(Arc::clone(&mcp), Arc::clone(&mcp)));
        let dispatcher = Arc::new(Dispatcher::new(Arc::clone(&mcp), Arc::clone(&mcp), "whisper", None));

        LoopSupervisor::new(
            triage, decider, dispatcher, store,
            5.0, 300.0,
            Arc::new(TestLogWriter),
            None, None, 3,
            false, String::new(), String::new(), 200,
            goal_sink,
        )
    }

    fn hot_inputs_with_level(level: u64) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert("state_summary".into(), serde_json::json!({ "self": { "level": level } }));
        m
    }

    // ── G2 tests ────────────────────────────────────────────────────────────

    /// With goal_sink=None, maybe_emit_goal must be a complete no-op.
    #[tokio::test]
    async fn goal_sink_none_is_noop() {
        let sup = minimal_supervisor(None, 25);
        let inputs = hot_inputs_with_level(6);
        // Call twice — if it were to panic or do anything, the test would catch it.
        sup.maybe_emit_goal(1003, &inputs).await;
        sup.maybe_emit_goal(1003, &inputs).await;
        // No assertion needed beyond "doesn't panic" — parity preserved.
    }

    /// With goal_sink=Some, the first tick at level 6 emits exactly one goal.
    #[tokio::test]
    async fn goal_sink_some_emits_on_first_tick_below_cap() {
        let (sink, mut rx) = make_test_sink();
        let sup = minimal_supervisor(Some(sink), 25);
        let inputs = hot_inputs_with_level(6);
        sup.maybe_emit_goal(1003, &inputs).await;
        let (guid, env) = rx.try_recv().expect("expected one goal");
        assert_eq!(guid, 1003u64);
        assert_eq!(env.goal_id, "grind-1003-6");
    }

    /// A second tick at the same level must NOT re-emit (dedup).
    #[tokio::test]
    async fn goal_sink_dedupes_same_level() {
        let (sink, mut rx) = make_test_sink();
        let sup = minimal_supervisor(Some(sink), 25);
        let inputs = hot_inputs_with_level(6);
        sup.maybe_emit_goal(1003, &inputs).await;
        sup.maybe_emit_goal(1003, &inputs).await; // second call — same level
        // First call succeeds
        let _ = rx.try_recv().expect("expected first goal");
        // No second goal
        assert!(rx.try_recv().is_err(), "second tick at same level must not re-emit");
    }

    /// A tick at level 7 after one at level 6 MUST emit a new goal.
    #[tokio::test]
    async fn goal_sink_re_emits_on_level_change() {
        let (sink, mut rx) = make_test_sink();
        let sup = minimal_supervisor(Some(sink), 25);
        sup.maybe_emit_goal(1003, &hot_inputs_with_level(6)).await;
        sup.maybe_emit_goal(1003, &hot_inputs_with_level(7)).await;
        let (_, e1) = rx.try_recv().expect("first goal");
        let (_, e2) = rx.try_recv().expect("second goal on level-up");
        assert_eq!(e1.goal_id, "grind-1003-6");
        assert_eq!(e2.goal_id, "grind-1003-7");
    }

    /// At cap (level == max_level), must not emit.
    #[tokio::test]
    async fn goal_sink_no_emit_at_cap() {
        let (sink, mut rx) = make_test_sink();
        let sup = minimal_supervisor(Some(sink), 25);
        sup.maybe_emit_goal(1003, &hot_inputs_with_level(25)).await;
        assert!(rx.try_recv().is_err(), "at cap must not emit");
    }

    /// Missing state_summary in hot_inputs → no emit.
    #[tokio::test]
    async fn goal_sink_no_emit_without_state_summary() {
        let (sink, mut rx) = make_test_sink();
        let sup = minimal_supervisor(Some(sink), 25);
        let empty: HashMap<String, serde_json::Value> = HashMap::new();
        sup.maybe_emit_goal(1003, &empty).await;
        assert!(rx.try_recv().is_err(), "missing state_summary must not emit");
    }

    #[test]
    fn test_episode_text_keys_sorted_alphabetically() {
        let hot_inputs: HashMap<String, Value> = [(
            "episode_type".to_string(),
            Value::String("fresh_chat".to_string()),
        )]
        .into_iter()
        .collect();
        let decision = Decision {
            kind: DecisionKind::NoOp,
            tool: None,
            args: None,
            confidence: 0.0,
            reasoning: "test reason".to_string(),
            wakeup_in_ms: None,
        };
        let dispatch_result = DispatchResult {
            disposition: "skipped".to_string(),
            tool: None,
            result: None,
            error: String::new(),
        };
        let text = _episode_text(&hot_inputs, &decision, &dispatch_result);
        // Keys must appear in alphabetical order: decision_kind, disposition, reasoning, tool, triage
        let dk = text.find("\"decision_kind\"").expect("decision_kind present");
        let di = text.find("\"disposition\"").expect("disposition present");
        let re = text.find("\"reasoning\"").expect("reasoning present");
        let to = text.find("\"tool\"").expect("tool present");
        let tr = text.find("\"triage\"").expect("triage present");
        assert!(dk < di, "decision_kind must come before disposition");
        assert!(di < re, "disposition must come before reasoning");
        assert!(re < to, "reasoning must come before tool");
        assert!(to < tr, "tool must come before triage");
    }
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
