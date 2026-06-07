// SPDX-License-Identifier: AGPL-3.0
//! Axum app factory + lifespan wiring.
//!
//! Faithful Rust port of `brain_sidecar/app.py create_app` + `brain_sidecar/api.py make_router`.
//!
//! # Startup sequence (mirrors Python lifespan order)
//! 1. Build Settings, open StateStore + migrate.
//! 2. Build LlmClient, load prompt template (include_str! or BRAIN_PROMPT_PATH).
//! 3. Open McpClient for harness + memory.
//! 4. Validate SSE endpoint if brain_sse_enabled.
//! 5. `fetch_schemas` from both MCPs → `compose_oneof` + `render_prompt_summary`.
//! 6. Log `schema_loaded harness_tools=N memory_tools=M oneof_branches=K` (soak gate).
//! 7. Build PersonalityCache, TriageGate, Decider, Dispatcher, LoopSupervisor.
//! 8. Rehydrate active bots → warm personality cache.
//! 9. Build SubsetGate + spawn task.
//! 10. Bind routes; serve.
//! On shutdown: cancel SubsetGate, parallel stop() all active bots.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::Settings;
use crate::decide::Decider;
use crate::dispatch::Dispatcher;
use crate::llm_client::LlmClient;
use crate::logging::JsonlDecisionLogWriter;
use crate::loop_supervisor::LoopSupervisor;
use crate::mcp_client::McpClient;
use crate::models::PersonalityCard;
use crate::morph::morph_personality;
use crate::personality::PersonalityCache;
use crate::schema_builder::{compose_oneof, fetch_schemas, render_prompt_summary};
use crate::state::StateStore;
use crate::subset_gate::{BotSnapshot, SubsetGate, SubsetGateConfig};
use crate::triage::TriageGate;

use exec_rs::supervisor::BotSupervisor;
use tot_goal_contract::{wire, ChannelStatusSource, GoalSink, GoalSinkRegistry, GoalStatus, StatusSource};
use tot_harness_client::HarnessClient;

// ---------------------------------------------------------------------------
// Compile-time default prompt template
// ---------------------------------------------------------------------------

/// Default prompt template embedded at compile time.
/// Runtime override: set BRAIN_PROMPT_PATH env var.
const DEFAULT_PROMPT_TEMPLATE: &str =
    include_str!("../prompts/decide_v1.txt");

// ---------------------------------------------------------------------------
// Shared app state — injected into route handlers
// ---------------------------------------------------------------------------

/// Shared app state injected via axum `State` extractor.
#[derive(Clone)]
pub struct AppState {
    pub state_store: Arc<StateStore>,
    pub supervisor: Arc<LoopSupervisor>,
    pub personality_cache: Arc<PersonalityCache>,
    pub harness_mcp: Arc<McpClient>,
    pub llm_client: Arc<LlmClient>,
    pub brain_bearer: String,
    pub subset_gate: Arc<SubsetGate>,
}

// ---------------------------------------------------------------------------
// Request / Response types (mirrors api.py Pydantic models)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    pub bot_guid: i64,
    pub personality_seed: PersonalityCard,
}

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub bot_guid: i64,
    pub status: String,
    pub enrolled_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseRequest {
    pub bot_guid: i64,
}

#[derive(Debug, Serialize)]
pub struct ReleaseResponse {
    pub bot_guid: i64,
    pub status: String,
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

/// GET /healthz — pre-lifespan, always 200, no auth.
async fn healthz() -> impl IntoResponse {
    Json(serde_json::json!({"ok": true}))
}

/// GET /status (auth) → {"active_bots": [...]}
async fn status_route(State(state): State<AppState>) -> impl IntoResponse {
    let active = state.supervisor.list_active();
    Json(serde_json::json!({"active_bots": active}))
}

/// POST /enroll (auth) → EnrollResponse 201
async fn enroll_route(
    State(state): State<AppState>,
    Json(mut req): Json<EnrollRequest>,
) -> Response {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    // Check duplicate active bot.
    match state.state_store.get_bot(req.bot_guid) {
        Ok(Some(existing)) if existing.status == "active" => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!("bot_guid {} already enrolled", req.bot_guid)
                })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response();
        }
        _ => {}
    }

    // Enroll in state store.
    if let Err(e) = state
        .state_store
        .enroll(req.bot_guid, now_ms, &req.personality_seed)
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": e})),
        )
            .into_response();
    }

    // Auto-populate identity from obs.get_state (best-effort, 5s timeout).
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state
            .harness_mcp
            .call("obs.get_state", &serde_json::json!({"target_guid": req.bot_guid})),
    )
    .await
    {
        Ok(Ok(raw)) => {
            let obs = raw.get("result").cloned().unwrap_or(raw);
            let self_obj = obs.get("self").and_then(|v| v.as_object()).cloned().unwrap_or_default();
            let live_name = self_obj.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let live_race = self_obj.get("race").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let live_class = self_obj.get("class").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !live_name.is_empty() && !live_race.is_empty() && !live_class.is_empty() {
                req.personality_seed.name = live_name;
                req.personality_seed.race = live_race;
                req.personality_seed.class_ = live_class;
            } else {
                warn!(
                    "enroll bot_guid={}: obs.get_state missing identity fields \
                     (name={:?} race={:?} class={:?}); keeping seed values",
                    req.bot_guid, live_name, live_race, live_class
                );
            }
        }
        Ok(Err(e)) => {
            warn!(
                "enroll bot_guid={}: obs.get_state failed ({}); keeping seed values",
                req.bot_guid, e
            );
        }
        Err(_timeout) => {
            warn!(
                "enroll bot_guid={}: obs.get_state timed out; keeping seed values",
                req.bot_guid
            );
        }
    }

    // V3.6: morph personality v2 fields.
    req.personality_seed = morph_personality(
        &req.personality_seed,
        &state.llm_client,
        None::<&mut rand::rngs::StdRng>,
    )
    .await;

    // Seed personality cache (write-through). On failure, rollback state_store.
    if let Err(e) = state
        .personality_cache
        .seed(req.bot_guid, req.personality_seed.clone())
        .await
    {
        let _ = state.state_store.set_status(req.bot_guid, "released");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": format!(
                    "personality seed failed; memory MCP may be unavailable: {e}"
                )
            })),
        )
            .into_response();
    }

    state.supervisor.start(req.bot_guid);

    (
        StatusCode::CREATED,
        Json(EnrollResponse {
            bot_guid: req.bot_guid,
            status: "active".to_string(),
            enrolled_at: now_ms,
        }),
    )
        .into_response()
}

/// POST /release (auth) → ReleaseResponse
async fn release_route(
    State(state): State<AppState>,
    Json(req): Json<ReleaseRequest>,
) -> Response {
    match state.state_store.get_bot(req.bot_guid) {
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "not enrolled"})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response();
        }
        Ok(Some(_)) => {}
    }

    state.supervisor.stop(req.bot_guid).await;
    let _ = state.state_store.set_status(req.bot_guid, "released");

    Json(ReleaseResponse {
        bot_guid: req.bot_guid,
        status: "released".to_string(),
    })
    .into_response()
}

/// POST /admin/subset/pin/{bot_guid} (auth)
async fn subset_pin(State(state): State<AppState>, Path(bot_guid): Path<i64>) -> Response {
    match state.state_store.get_bot(bot_guid) {
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!("bot_guid {bot_guid} not in living_bots")
                })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response();
        }
        Ok(Some(_)) => {}
    }
    let _ = state.state_store.set_pin(bot_guid, true);
    Json(serde_json::json!({"ok": true, "bot_guid": bot_guid, "pinned": true})).into_response()
}

/// POST /admin/subset/unpin/{bot_guid} (auth)
async fn subset_unpin(State(state): State<AppState>, Path(bot_guid): Path<i64>) -> Response {
    let _ = state.state_store.set_pin(bot_guid, false);
    Json(serde_json::json!({"ok": true, "bot_guid": bot_guid, "pinned": false})).into_response()
}

/// GET /admin/subset/snapshot (auth)
async fn subset_snapshot(State(state): State<AppState>) -> Response {
    let currently_enrolled = state
        .state_store
        .list_active()
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            let tier = state
                .state_store
                .get_tier(row.bot_guid)
                .unwrap_or_else(|_| "full".to_string());
            serde_json::json!({"bot_guid": row.bot_guid, "tier": tier})
        })
        .collect::<Vec<_>>();
    let pinned = state.state_store.list_pinned().unwrap_or_default();
    let cfg = &state.subset_gate.config;
    let config_payload = serde_json::json!({
        "living_bot_count": cfg.living_bot_count,
        "recompute_interval_s": cfg.recompute_interval_s,
        "phase_b_enabled": cfg.phase_b_enabled,
    });
    Json(serde_json::json!({
        "currently_enrolled": currently_enrolled,
        "pinned": pinned,
        "config": config_payload,
    }))
    .into_response()
}

/// POST /admin/subset/recompute (auth)
async fn subset_recompute(State(state): State<AppState>) -> Response {
    match state.subset_gate._recompute_and_apply().await {
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": format!("{e}")})),
        )
            .into_response(),
        Ok(decision) => {
            let mut target: Vec<i64> = decision.target_living_set.into_iter().collect();
            target.sort();
            let mut to_enroll: Vec<i64> = decision.to_enroll.into_iter().collect();
            to_enroll.sort();
            let mut to_release: Vec<i64> = decision.to_release.into_iter().collect();
            to_release.sort();
            let mut to_full: Vec<i64> = decision.to_full.into_iter().collect();
            to_full.sort();
            let mut to_reduced: Vec<i64> = decision.to_reduced.into_iter().collect();
            to_reduced.sort();
            Json(serde_json::json!({
                "target": target,
                "to_enroll": to_enroll,
                "to_release": to_release,
                "to_full": to_full,
                "to_reduced": to_reduced,
            }))
            .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// SSE validation (mirrors app.py validate_sse_endpoint_or_raise)
// ---------------------------------------------------------------------------

/// Validate the memory-sidecar SSE endpoint.
/// Split timeout: connect=5s, read=20s.
/// Returns Err(RuntimeError-style string) on failure.
async fn validate_sse_endpoint(
    memory_url: &str,
    bearer: &str,
) -> Result<(), String> {
    let url = format!("{memory_url}/v1/events/stream?bot_id=0&prefixes=received+whisper");
    let mut req_builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("SSE validation: build client failed: {e}"))?
        .get(&url)
        .header("Accept", "text/event-stream");
    if !bearer.is_empty() {
        req_builder = req_builder.header("Authorization", format!("Bearer {bearer}"));
    }
    let resp = req_builder
        .send()
        .await
        .map_err(|e| format!("SSE endpoint validation failed: {e}"))?;
    if resp.status() != reqwest::StatusCode::OK {
        return Err(format!(
            "SSE endpoint validation failed: HTTP {} from {url}",
            resp.status()
        ));
    }
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ctype.contains("text/event-stream") {
        return Err(format!(
            "SSE endpoint validation failed: unexpected Content-Type {ctype:?} from {url}"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// create_app — public entry point
// ---------------------------------------------------------------------------

/// Build and return the axum Router with lifespan wiring.
///
/// Mirrors Python `create_app()`:
/// - Opens MCPs (harness + memory).
/// - Validates SSE endpoint if `settings.brain_sse_enabled`.
/// - Fetches schemas → emits `schema_loaded` log line.
/// - Builds all components.
/// - Rehydrates active bots + warms personality cache.
/// - Spawns SubsetGate task.
/// - Returns Router with all routes.
pub async fn create_app(settings: Settings) -> anyhow::Result<Router> {
    // ── Open state store ──────────────────────────────────────────────────────
    let state_store = Arc::new(
        StateStore::open(&settings.state_db_path)
            .map_err(|e| anyhow::anyhow!("StateStore open failed: {e}"))?,
    );
    state_store
        .migrate()
        .map_err(|e| anyhow::anyhow!("StateStore migrate failed: {e}"))?;

    // ── Decision log writer ───────────────────────────────────────────────────
    let decision_log = Arc::new(JsonlDecisionLogWriter::new(
        settings.decisions_log_path.clone(),
    ));

    // ── LLM client ───────────────────────────────────────────────────────────
    let llm_client = Arc::new(LlmClient {
        base_url: settings.llm_base_url.clone(),
        model: settings.llm_model.clone(),
        timeout_s: settings.llm_timeout_s,
    });

    // ── Prompt template (compile-time default or BRAIN_PROMPT_PATH override) ──
    let prompt_template: String = match std::env::var("BRAIN_PROMPT_PATH") {
        Ok(path) if !path.is_empty() => std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("BRAIN_PROMPT_PATH read failed ({path}): {e}"))?,
        _ => DEFAULT_PROMPT_TEMPLATE.to_string(),
    };

    // ── MCP clients ───────────────────────────────────────────────────────────
    let harness_mcp = Arc::new(McpClient::new(
        settings.harness_mcp_url.clone(),
        settings.harness_bearer.clone(),
    ));
    let memory_mcp = Arc::new(McpClient::new(
        settings.memory_mcp_url.clone(),
        settings.memory_bearer.clone(),
    ));

    // ── SSE endpoint validation ───────────────────────────────────────────────
    if settings.brain_sse_enabled {
        let memory_base = settings.memory_mcp_url.trim_end_matches("/mcp/mcp").to_string();
        match validate_sse_endpoint(&memory_base, &settings.memory_bearer).await {
            Ok(()) => info!("sse_endpoint_validated memory_url={}", settings.memory_mcp_url),
            Err(e) => {
                tracing::error!("sse_endpoint_validation_failed reason={}", e);
                return Err(anyhow::anyhow!(e));
            }
        }
    }

    // ── Schema fetch — fail-loud on error ────────────────────────────────────
    let per_tool = fetch_schemas(harness_mcp.as_ref(), memory_mcp.as_ref())
        .await
        .map_err(|e| {
            tracing::error!("schema_builder_fetch_failed error={e:?}");
            e
        })?;

    let decision_schema = compose_oneof(&per_tool);
    let tools_summary = render_prompt_summary(&per_tool);
    let harness_count = per_tool.values().filter(|e| e.source_mcp == "harness").count();
    let memory_count = per_tool.values().filter(|e| e.source_mcp == "memory").count();
    let oneof_branches = decision_schema["oneOf"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    // soak gate line — must match Python exactly
    info!(
        "schema_loaded harness_tools={} memory_tools={} oneof_branches={}",
        harness_count, memory_count, oneof_branches
    );

    // ── Build components ──────────────────────────────────────────────────────
    let personality_cache = Arc::new(PersonalityCache::new(
        memory_mcp.clone(),
        settings.personality_ttl_s,
        32,
    ));

    let triage = Arc::new(TriageGate::new(harness_mcp.clone(), memory_mcp.clone()));

    let mut decider = Decider::new(
        llm_client.clone(),
        personality_cache.clone(),
        memory_mcp.clone(),
        state_store.clone(),
        prompt_template,
        settings.max_player_level,
    );
    decider.decision_schema = Some(decision_schema);
    decider.tools_summary = Some(tools_summary);
    let decider = Arc::new(decider);

    let dispatcher = Arc::new(Dispatcher::new(
        harness_mcp.clone(),
        memory_mcp.clone(),
        "whisper", // confirmation_channel — Python app.py uses the dispatch.py default "whisper"
        None,  // tool_policy
    ));

    let memory_http_url = settings
        .memory_mcp_url
        .trim_end_matches("/mcp/mcp")
        .to_string();

    // ── G3 / M2-F: exec supervisor embed (roster) ─────────────────────────────
    // For each enrolled bot, wire an in-process goal→exec channel and start a
    // per_bot_loop on the shared BotSupervisor. All per-bot sinks are collected
    // into a GoalSinkRegistry, which is the single Arc<dyn GoalSink> handed to
    // LoopSupervisor — it routes set_goal(bot_guid) to the right channel.
    // Empty roster → goal_sink stays None → pure parity mode.
    let goal_sink: Option<Arc<dyn GoalSink>> = {
        let roster = settings.exec_roster();
        if roster.is_empty() {
            None // pure parity mode
        } else {
            // Derive the harness REST base from the MCP URL (strip "/mcp/mcp").
            let harness_base = settings
                .harness_mcp_url
                .trim_end_matches("/mcp/mcp")
                .to_string();
            let exec_harness = Arc::new(HarnessClient::new(
                harness_base,
                settings.harness_bearer.clone(),
                std::time::Duration::from_secs(30),
            ));

            let mut exec_sup = BotSupervisor::new();
            let mut registry = GoalSinkRegistry::new();
            let mut sources: Vec<(u64, ChannelStatusSource)> = Vec::new();

            for &guid in &roster {
                let (sink, source, goal_rx, status_tx) = wire(guid);
                exec_sup.start(guid, exec_harness.clone(), goal_rx, status_tx);
                registry.register(guid, sink);
                sources.push((guid, source));
            }

            // One drain task owns the supervisor (lifetime) + all status sources.
            // It owns exec_sup for process lifetime; ownership alone prevents the
            // drop that would close the per-bot task channels. M1/F shutdown =
            // process exit (join_all is wired but not invoked here).
            tokio::spawn(async move {
                let _exec_sup_lifetime = exec_sup;
                loop {
                    for (guid, source) in &sources {
                        match source.poll_status(*guid).await {
                            Ok(Some(status)) => log_exec_status(*guid, &status),
                            Ok(None) => {}
                            Err(e) => warn!("exec_status_poll_error bot_guid={} err={:?}", guid, e),
                        }
                    }
                    // Poll all sources at ~1s cadence — inexpensive mpsc try_recv.
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            });

            info!("exec_embed_started roster={:?}", roster);
            Some(Arc::new(registry) as Arc<dyn GoalSink>)
        }
    };

    let supervisor = LoopSupervisor::new(
        triage.clone(),
        decider.clone(),
        dispatcher.clone(),
        state_store.clone(),
        settings.tick_interval_s,
        settings.reduced_tick_interval_s,
        decision_log.clone(),
        None, // memory_client
        None, // salience_scorer
        3,    // memory_recall_top_k
        settings.brain_sse_enabled,
        memory_http_url.clone(),
        settings.memory_bearer.clone(),
        settings.brain_sse_coalesce_ms,
        goal_sink,
    );

    // ── Rehydrate active bots ─────────────────────────────────────────────────
    let active_rows = state_store.list_active().unwrap_or_default();
    for row in &active_rows {
        supervisor.start(row.bot_guid);
        info!("rehydrated bot_guid={}", row.bot_guid);
    }

    // ── Warm personality cache ────────────────────────────────────────────────
    for row in &active_rows {
        match personality_cache.get(row.bot_guid).await {
            Ok(_) => info!("personality_warmed bot_guid={}", row.bot_guid),
            Err(e) => warn!("personality_warm_failed bot_guid={} err={}", row.bot_guid, e),
        }
    }

    // ── SubsetGate ────────────────────────────────────────────────────────────
    let harness_for_sg = harness_mcp.clone();
    let state_for_sg = state_store.clone();
    let supervisor_for_sg = supervisor.clone();
    let personality_cache_for_sg = personality_cache.clone();
    let llm_for_sg = llm_client.clone();
    let settings_for_sg = settings.clone();

    let snapshot_fetcher: crate::subset_gate::SnapshotFetcher = Arc::new(move || {
        let h = harness_for_sg.clone();
        Box::pin(async move {
            let players_raw = h
                .call("obs.list_players", &serde_json::json!({}))
                .await
                .unwrap_or_default();
            let bots_raw = h
                .call("obs.list_bot_population", &serde_json::json!({}))
                .await
                .unwrap_or_default();
            parse_world_snapshot(players_raw, bots_raw)
        })
    });

    let enroll_fn: crate::subset_gate::EnrollFn = {
        let ss = state_for_sg.clone();
        let sup = supervisor_for_sg.clone();
        let pc = personality_cache_for_sg.clone();
        let llm = llm_for_sg.clone();
        let harness = harness_mcp.clone();
        Arc::new(move |bot_guid: i64, bot_snapshot: Option<BotSnapshot>| {
            let ss = ss.clone();
            let sup = sup.clone();
            let pc = pc.clone();
            let llm = llm.clone();
            let harness = harness.clone();
            Box::pin(async move {
                enroll_via_api(bot_guid, bot_snapshot, &ss, &sup, &pc, &llm, &harness).await;
            })
        })
    };

    let release_fn: crate::subset_gate::ReleaseFn = {
        let sup = supervisor_for_sg.clone();
        Arc::new(move |bot_guid: i64| {
            let sup = sup.clone();
            Box::pin(async move {
                sup.release_bot(bot_guid).await;
            })
        })
    };

    let subset_gate_config = SubsetGateConfig {
        living_bot_count: settings.living_bot_count,
        recompute_interval_s: settings.subset_recompute_interval_s,
        hysteresis_out_ticks: settings_for_sg.subset_hysteresis_out_ticks as i64,
        hysteresis_in_ticks: settings_for_sg.subset_hysteresis_in_ticks as i64,
        enroll_backoff_s: settings_for_sg.subset_enroll_backoff_s,
        enabled: settings_for_sg.subset_gate_enabled,
        phase_b_enabled: true, // Phase B: warm cache + REDUCED tier
    };

    let subset_gate = Arc::new(SubsetGate::new(
        state_for_sg.clone(),
        snapshot_fetcher,
        enroll_fn,
        release_fn,
        subset_gate_config,
    ));

    // Spawn SubsetGate task (stored in the AppState for cancellation).
    // In this design we don't cancel at shutdown (clean shutdown is left to
    // the process exit); the task is fire-and-forget.
    let sg_clone = subset_gate.clone();
    tokio::spawn(async move { sg_clone.run().await });

    // ── Build AppState ────────────────────────────────────────────────────────
    let app_state = AppState {
        state_store: state_store.clone(),
        supervisor: supervisor.clone(),
        personality_cache: personality_cache.clone(),
        harness_mcp: harness_mcp.clone(),
        llm_client: llm_client.clone(),
        brain_bearer: settings.brain_bearer.clone(),
        subset_gate: subset_gate.clone(),
    };

    // ── Build router ──────────────────────────────────────────────────────────
    // /healthz: pre-auth, always 200.
    // All other routes: protected by bearer-auth middleware.
    let bearer = settings.brain_bearer.clone();
    let authed = Router::new()
        .route("/status", get(status_route))
        .route("/enroll", post(enroll_route))
        .route("/release", post(release_route))
        .route("/admin/subset/pin/{bot_guid}", post(subset_pin))
        .route("/admin/subset/unpin/{bot_guid}", post(subset_unpin))
        .route("/admin/subset/snapshot", get(subset_snapshot))
        .route("/admin/subset/recompute", post(subset_recompute))
        .layer(middleware::from_fn_with_state(bearer, crate::auth::check_bearer))
        .with_state(app_state);

    let router = Router::new()
        .route("/healthz", get(healthz))
        .merge(authed);

    Ok(router)
}

// ---------------------------------------------------------------------------
// parse_world_snapshot (mirrors app.py parse_world_snapshot)
// ---------------------------------------------------------------------------

fn parse_world_snapshot(
    players_raw: serde_json::Value,
    bots_raw: serde_json::Value,
) -> crate::subset_gate::WorldSnapshot {
    let players_data = players_raw
        .get("result")
        .cloned()
        .unwrap_or(players_raw);
    let bots_data = bots_raw.get("result").cloned().unwrap_or(bots_raw);

    let players: Vec<crate::subset_gate::PlayerSnapshot> = players_data
        .get("players")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    let bots: Vec<BotSnapshot> = bots_data
        .get("bots")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    crate::subset_gate::WorldSnapshot { players, bots }
}

// ---------------------------------------------------------------------------
// enroll_via_api (mirrors app.py _enroll_via_api)
// ---------------------------------------------------------------------------

async fn enroll_via_api(
    bot_guid: i64,
    bot_snapshot: Option<BotSnapshot>,
    state_store: &StateStore,
    supervisor: &Arc<LoopSupervisor>,
    personality_cache: &Arc<PersonalityCache>,
    llm_client: &Arc<LlmClient>,
    harness_mcp: &Arc<McpClient>,
) {
    // Case 1: already active.
    match state_store.get_bot(bot_guid) {
        Ok(Some(existing)) if existing.status == "active" => {
            supervisor.start(bot_guid);
            return;
        }
        Ok(Some(existing)) if existing.status == "released" => {
            let _ = state_store.reactivate(bot_guid);
            supervisor.start(bot_guid);
            return;
        }
        _ => {}
    }

    // Case 3: brand new bot — bootstrap.
    let Some(snap) = bot_snapshot else {
        warn!(
            "enroll_via_api: bot_guid={bot_guid} not in living_bots and no BotSnapshot provided"
        );
        return;
    };

    let mut seed = PersonalityCard {
        name: snap.name.clone(),
        race: "Unknown".to_string(),
        class_: "Unknown".to_string(),
        backstory: "Adventurer encountered in the world.".to_string(),
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
    };

    // Auto-populate identity.
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        harness_mcp.call("obs.get_state", &serde_json::json!({"target_guid": bot_guid})),
    )
    .await
    {
        Ok(Ok(raw)) => {
            let obs = raw.get("result").cloned().unwrap_or(raw);
            let self_obj = obs.get("self").and_then(|v| v.as_object()).cloned().unwrap_or_default();
            let live_name = self_obj.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let live_race = self_obj.get("race").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let live_class = self_obj.get("class").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !live_name.is_empty() && !live_race.is_empty() && !live_class.is_empty() {
                seed.name = live_name;
                seed.race = live_race;
                seed.class_ = live_class;
            }
        }
        Ok(Err(e)) => warn!("subset_gate enroll bot_guid={bot_guid}: obs.get_state failed ({e}); keeping seed values"),
        Err(_) => warn!("subset_gate enroll bot_guid={bot_guid}: obs.get_state timed out; keeping seed values"),
    }

    // v2 morph.
    seed = morph_personality(&seed, llm_client, None::<&mut rand::rngs::StdRng>).await;

    // State store row.
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if let Err(e) = state_store.enroll(bot_guid, now_ms, &seed) {
        if !e.contains("already enrolled") {
            warn!("subset_gate enroll bot_guid={bot_guid}: state_store.enroll failed ({e})");
            // Race: treat as success
        }
    }

    // Seed personality cache.
    if let Err(e) = personality_cache.seed(bot_guid, seed).await {
        warn!(
            "subset_gate enroll bot_guid={bot_guid}: personality_cache.seed failed ({e}); first tick will cold-fetch"
        );
    }

    supervisor.start(bot_guid);
}

// ---------------------------------------------------------------------------
// Exec-embed helpers
// ---------------------------------------------------------------------------

/// Log one exec→brain status line for a bot. Extracted so the multi-bot status
/// drain can call it per source without duplicating the match arms.
fn log_exec_status(guid: u64, status: &GoalStatus) {
    match status {
        GoalStatus::Running { progress } => {
            info!("exec_status bot_guid={} status=Running progress={:?}", guid, progress);
        }
        GoalStatus::Completed { summary } => {
            info!("exec_status bot_guid={} status=Completed summary={:?}", guid, summary);
        }
        GoalStatus::NeedsDecision { event } => {
            // M1/F: log only. Re-decide hook is a later slice.
            info!("exec_status bot_guid={} status=NeedsDecision event={:?}", guid, event);
        }
        GoalStatus::Blocked { reason, detail } => {
            warn!("exec_status bot_guid={} status=Blocked reason={:?} detail={:?}", guid, reason, detail);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a minimal axum Router with just the healthz + a stub authed route
    /// for testing auth middleware independently of the full lifespan.
    fn build_test_healthz_app() -> Router {
        Router::new().route("/healthz", get(healthz))
    }

    // ── healthz tests ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_healthz_returns_200_ok_json() {
        let app = build_test_healthz_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], true);
    }

    #[tokio::test]
    async fn test_healthz_accessible_without_auth() {
        // /healthz must be reachable even when bearer auth is configured.
        let app = build_test_healthz_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
