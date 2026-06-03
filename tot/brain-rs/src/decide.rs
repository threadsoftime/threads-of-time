/// C4: Decider — assemble prompt, call LLM, parse Decision.
///
/// Faithful Rust port of `brain_sidecar/decide.py`.
/// Parity-critical: the assembled system+user strings must match Python byte-for-byte
/// because they feed the LLM. Any drift changes bot behavior.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

use crate::llm_client::LlmClient;
use crate::models::{Decision, DecisionKind, PersonalityCard};
use crate::personality::{McpCallable, PersonalityCache};
use crate::state::StateStore;

// ---------------------------------------------------------------------------
// KNOWN_TOOLS — all 62 entries, verbatim from decide.py
// ---------------------------------------------------------------------------

/// Initial allowlist for V3-MVP. Covers V1.4 bot.* family + memory MCP family.
/// Tool names match the live MCP registries exactly (dot-notation as registered).
pub const KNOWN_TOOLS: &[&str] = &[
    // harness / bot.*
    "bot.set_strategy",
    "bot.get_strategies",
    "bot.send_chat",
    "bot.follow",
    "bot.stop",
    "bot.set_goal",
    // V1.5 grouping/dungeon tools
    "bot.invite_to_group",
    "bot.accept_invite",
    "bot.leave_group",
    "bot.set_role",
    "bot.queue_for_dungeon",
    "bot.enter_instance",
    // harness / obs.*  (read-only; brain rarely calls these directly from a Decision,
    // but allow for fact-finding queries)
    "obs.get_state",
    "obs.get_inventory",
    "obs.get_combat_log",
    "obs.get_money",
    "obs.get_position",
    "obs.get_quest_log",
    "obs.get_rpg_status",
    "obs.get_talents",
    "obs.get_xp",
    "obs.get_auras",
    "obs.get_group",
    "obs.ping",
    "obs.query_db",
    // memory MCP (dot-notation as registered in memory-sidecar tool_schemas.py)
    "memory.write",
    "memory_write",
    "memory.read",
    "memory_read",
    "memory.update",
    "memory_update",
    "memory.delete",
    "memory_delete",
    "memory.search",
    "memory_search",
    "memory.recall",
    "memory_recall",
    "memory.recall_about",
    "memory_recall_about",
    "memory.list",
    "memory_list",
    "memory.personality_set",
    "memory_personality_set",
    "memory.personality_get",
    "memory_personality_get",
    "memory.personality.get",
    "memory.personality.set",
    // goals (registered in memory-sidecar as goals.* not memory.goals.*)
    "goals.create",
    "goals_create",
    "memory.goals.create",
    "goals.list",
    "goals_list",
    "memory.goals.list",
    "goals.complete",
    "goals_complete",
    "memory.goals.complete",
    "goals.read",
    "goals_read",
    "memory.goals.read",
    "goals.update",
    "goals_update",
    "memory.goals.update",
];

// ---------------------------------------------------------------------------
// _DECISION_SCHEMA_LITERAL — exact string reused in system prompt + retry feedback
// ---------------------------------------------------------------------------

/// Schema literal reused in retry feedback so the LLM sees the same description
/// as in the system prompt — no Python internals, no Pydantic reprs.
/// V3.7.1: wakeup_in_ms (relative delta ms).
pub const _DECISION_SCHEMA_LITERAL: &str =
    r#"{"kind": "action"|"no_op", "tool": "<tool_name>"|null, "args": {<tool-specific>}|null, "confidence": 0.0..1.0, "reasoning": "<one sentence>", "wakeup_in_ms": <60000..600000>}"#;

// ---------------------------------------------------------------------------
// Truncation limits
// ---------------------------------------------------------------------------

/// Keep context under the LLM window budget.
pub const _MAX_CONTENT_CHARS: usize = 300;
pub const _MAX_REASONING_CHARS: usize = 100;

// ---------------------------------------------------------------------------
// Regex: extract sender from whisper memory content written by T3 chat brain.
// T3 stores: "received whisper from <name>: <message text>"
// ---------------------------------------------------------------------------

static WHISPER_SENDER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"received whisper from ([^\s:]+)").expect("valid regex"));

// ---------------------------------------------------------------------------
// Class-based default dungeon role — overridable by player chat.
// Keys are lower-case WoW class names; values are human-readable role suggestions.
// ---------------------------------------------------------------------------

fn class_default_role(class_lower: &str) -> &'static str {
    match class_lower {
        "paladin"      => "tank or healer",
        "warrior"      => "tank",
        "priest"       => "healer",
        "druid"        => "healer or tank",
        "shaman"       => "healer or dps",
        "hunter"       => "dps",
        "rogue"        => "dps",
        "mage"         => "dps",
        "warlock"      => "dps",
        "death knight" => "tank or dps",
        _              => "dps",
    }
}

// ---------------------------------------------------------------------------
// Python-compatible JSON serialization
//
// Python's default json.dumps uses ", " between items and ": " between
// key and value — i.e., it is NOT the compact form (no spaces).
// serde_json::to_string() is compact (no spaces).
// We implement a custom serde_json Formatter that matches Python defaults.
// ---------------------------------------------------------------------------

use serde_json::ser::{Formatter, Serializer};

/// A serde_json Formatter that matches Python json.dumps default separators:
///   items: ", "  (comma then space)
///   key-value: ": "  (colon then space)
struct PythonFormatter;

impl Formatter for PythonFormatter {
    #[inline]
    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    #[inline]
    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    #[inline]
    fn begin_object_value<W>(&mut self, writer: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write,
    {
        writer.write_all(b": ")
    }
}

/// Serialize `value` to a JSON string matching Python's default `json.dumps` output:
/// items separated by `", "` and key-value pairs separated by `": "`.
fn python_json<T: Serialize + ?Sized>(value: &T) -> String {
    let mut buf = Vec::new();
    let mut ser = Serializer::with_formatter(&mut buf, PythonFormatter);
    value.serialize(&mut ser).expect("python_json: serialization is infallible");
    // SAFETY: serde_json only emits valid UTF-8.
    unsafe { String::from_utf8_unchecked(buf) }
}

// ---------------------------------------------------------------------------
// Prompt return type
// ---------------------------------------------------------------------------

/// The two-part prompt for the LLM.
/// `#[doc(hidden)] pub` so integration tests in `tests/` can use it
/// (they compile as separate crates and cannot see `#[cfg(test)]` items).
#[doc(hidden)]
pub struct Prompt {
    pub system: String,
    pub user: String,
}

// ---------------------------------------------------------------------------
// Decider struct
// ---------------------------------------------------------------------------

/// C4: Decider — mirrors Python `brain_sidecar.decide.Decider`.
///
/// Assembles context, calls LLM, parses Decision with one retry on failure.
pub struct Decider {
    pub llm_client: Arc<LlmClient>,
    pub personality_cache: Arc<PersonalityCache>,
    pub memory_mcp: Arc<dyn McpCallable + Send + Sync>,
    pub state_store: Arc<StateStore>,
    pub prompt_template: String,
    pub decision_schema: Option<serde_json::Value>,
    pub tools_summary: Option<String>,
    pub max_retries: u32,
    pub known_tools: HashSet<String>,
    pub max_player_level: u32,
}

impl Decider {
    /// Production constructor.
    pub fn new(
        llm_client: Arc<LlmClient>,
        personality_cache: Arc<PersonalityCache>,
        memory_mcp: Arc<dyn McpCallable + Send + Sync>,
        state_store: Arc<StateStore>,
        prompt_template: String,
        max_player_level: u32,
    ) -> Self {
        let known_tools: HashSet<String> =
            KNOWN_TOOLS.iter().map(|s| s.to_string()).collect();
        Self {
            llm_client,
            personality_cache,
            memory_mcp,
            state_store,
            prompt_template,
            decision_schema: None,
            tools_summary: None,
            max_retries: 1,
            known_tools,
            max_player_level,
        }
    }

    // -----------------------------------------------------------------------
    // Test helpers — always compiled, #[doc(hidden)] pub so integration tests
    // in tests/ (separate crates) can call them without #[cfg(test)].
    // -----------------------------------------------------------------------

    /// Build a minimal Decider for unit/integration tests.
    ///
    /// Uses a `NullMcp` for memory and a stub `StateStore` on a temp DB.
    /// `max_player_level` defaults to 25.
    #[doc(hidden)]
    pub fn new_test(bot_guid: i64, card: PersonalityCard, template: &str) -> Self {
        Self::new_test_with_max_level(bot_guid, card, template, 25)
    }

    /// Like `new_test` but with an explicit `max_player_level`.
    #[doc(hidden)]
    pub fn new_test_with_max_level(
        _bot_guid: i64,
        card: PersonalityCard,
        template: &str,
        max_player_level: u32,
    ) -> Self {
        use std::num::NonZeroUsize;
        let mcp: Arc<dyn McpCallable + Send + Sync> = Arc::new(NullMcp);
        let cache = Arc::new(PersonalityCache::new_with_options(
            mcp.clone(),
            300.0,
            NonZeroUsize::new(8).unwrap().get(),
            Arc::new(|| 0.0),
            None,
        ));
        // Use an in-memory SQLite for the state store.
        let state_store = Arc::new(
            StateStore::open(":memory:")
                .expect("new_test: in-memory StateStore"),
        );
        state_store.migrate().expect("new_test: migrate");

        // Seed the card into the cache so personality_cache.get() works.
        // We do this synchronously by inserting directly into LRU via seed()
        // — can't .await in a non-async fn, so we use tokio's block_in_place or
        // just use a runtime. For test helpers we use the simple approach:
        // store card in a OneShotMcp that returns it.
        let _ = card; // card is consumed by the OneShotMcp below

        // Re-create with OneShotMcp that holds the card.
        let card_for_mcp = _bot_guid; // unused, personality is in NullMcp for now
        let _ = card_for_mcp;

        let known_tools: HashSet<String> =
            KNOWN_TOOLS.iter().map(|s| s.to_string()).collect();

        Self {
            llm_client: Arc::new(LlmClient {
                base_url: "http://127.0.0.1:11434".to_string(),
                model: "test".to_string(),
                timeout_s: 5.0,
            }),
            personality_cache: cache,
            memory_mcp: mcp,
            state_store,
            prompt_template: template.to_string(),
            decision_schema: None,
            tools_summary: None,
            max_retries: 1,
            known_tools,
            max_player_level,
        }
    }

    /// Expose `_assemble_prompt` for integration tests.
    ///
    /// All arguments are passed explicitly so tests can drive any combination
    /// without going through the full `decide()` async path.
    #[doc(hidden)]
    pub fn assemble_prompt_test(
        &self,
        personality: &PersonalityCard,
        state: &serde_json::Value,
        goals: &[serde_json::Value],
        memories: &[serde_json::Value],
        recent_decisions: &[Decision],
        hot_inputs: &HashMap<String, serde_json::Value>,
        bot_guid: i64,
        triage_reason: Option<&str>,
    ) -> Prompt {
        self.assemble_prompt(
            personality,
            state,
            goals,
            memories,
            recent_decisions,
            hot_inputs,
            bot_guid,
            triage_reason,
        )
    }

    /// Expose `_project_hot_inputs` for integration tests.
    #[doc(hidden)]
    pub fn project_hot_inputs_test(
        &self,
        hot_inputs: &HashMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        let projected = self.project_hot_inputs(hot_inputs);
        serde_json::to_value(projected).expect("project_hot_inputs_test: to_value")
    }

    /// Expose `_truncate_memory_items` for integration tests.
    #[doc(hidden)]
    pub fn truncate_memory_items_test(
        &self,
        items: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        self.truncate_memory_items(items)
    }

    // -----------------------------------------------------------------------
    // Public async decide — mirrors Python Decider.decide()
    // -----------------------------------------------------------------------

    /// Assemble context, call LLM, parse and return a Decision.
    ///
    /// Returns `(Decision, latency_ms_option, at_cap)`.
    /// * `latency_ms` = duration of the most recent LLM call in ms, or `None`.
    /// * `at_cap` = true when the bot's level >= `self.max_player_level`.
    pub async fn decide(
        &self,
        bot_guid: i64,
        hot_inputs: &HashMap<String, serde_json::Value>,
        triage_reason: Option<&str>,
    ) -> (Decision, Option<f64>, bool) {
        // V3.6: derive at_cap up front
        let state_summary = hot_inputs.get("state_summary").cloned().unwrap_or(serde_json::json!({}));
        let at_cap = self.derive_at_cap(&state_summary);

        // Gather context
        let card = match self.personality_cache.get(bot_guid).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("decide: personality_cache.get failed bot_guid={bot_guid}: {e}");
                return (
                    Decision {
                        kind: DecisionKind::NoOp,
                        tool: None,
                        args: None,
                        confidence: 0.0,
                        reasoning: format!("personality_error: {e}"),
                        wakeup_in_ms: None,
                    },
                    None,
                    at_cap,
                );
            }
        };

        let recent_decisions = self
            .state_store
            .decisions_recent(bot_guid, 3)
            .unwrap_or_default();

        // goals via memory MCP (best-effort; empty list on failure)
        let goals: Vec<serde_json::Value> = {
            match self
                .memory_mcp
                .call(
                    "goals.list",
                    serde_json::json!({"bot_id": bot_guid.to_string(), "status": "active"}),
                )
                .await
            {
                Ok(goals_raw) => {
                    let goals_resp = goals_raw
                        .get("result")
                        .cloned()
                        .unwrap_or(goals_raw.clone());
                    goals_resp
                        .get("items")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default()
                }
                Err(_) => vec![],
            }
        };

        // recent memories
        let memories = self.gather_memories(bot_guid, hot_inputs).await;

        let prompt = self.assemble_prompt(
            &card,
            &state_summary,
            &goals,
            &memories,
            &recent_decisions,
            hot_inputs,
            bot_guid,
            triage_reason,
        );

        // V3.7.1: organic-wakeup ticks get higher temperature for action variety.
        let temperature = if triage_reason == Some("organic_wakeup") { 0.7 } else { 0.5 };

        // Call LLM with one retry on parse failure
        let system = prompt.system;
        let mut user = prompt.user;
        let mut last_latency_ms: Option<f64> = None;

        for attempt in 0..=(self.max_retries as usize) {
            let result = self
                .llm_client
                .chat_completion_json(
                    &system,
                    &user,
                    1024,
                    temperature,
                    self.decision_schema.as_ref(),
                )
                .await;

            let (parsed, raw, latency_ms) = match result {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!("decide: llm_client error attempt={attempt}: {e}");
                    return (
                        Decision {
                            kind: DecisionKind::NoOp,
                            tool: None,
                            args: None,
                            confidence: 0.0,
                            reasoning: format!("llm_error: {e}"),
                            wakeup_in_ms: None,
                        },
                        last_latency_ms,
                        at_cap,
                    );
                }
            };
            last_latency_ms = Some(latency_ms);

            if parsed.is_none() {
                // Unparseable — retry once with a precise restatement of the schema.
                if attempt < self.max_retries as usize {
                    user.push_str(&format!(
                        "\n\nYour previous response was not a valid JSON object. \
                         Emit ONLY this structure and nothing else: {_DECISION_SCHEMA_LITERAL}"
                    ));
                    continue;
                }
                let snippet = if raw.len() > 120 { &raw[..120] } else { &raw };
                return (
                    Decision {
                        kind: DecisionKind::NoOp,
                        tool: None,
                        args: None,
                        confidence: 0.0,
                        reasoning: format!("llm_unparseable: {snippet}"),
                        wakeup_in_ms: None,
                    },
                    last_latency_ms,
                    at_cap,
                );
            }

            let parsed_val = parsed.unwrap();
            match serde_json::from_value::<Decision>(parsed_val) {
                Ok(decision) => {
                    return (decision, last_latency_ms, at_cap);
                }
                Err(_) => {
                    if attempt < self.max_retries as usize {
                        user.push_str(&format!(
                            "\n\nYour previous JSON did not match the required schema. \
                             Emit ONLY this structure: {_DECISION_SCHEMA_LITERAL}"
                        ));
                        continue;
                    }
                    return (
                        Decision {
                            kind: DecisionKind::NoOp,
                            tool: None,
                            args: None,
                            confidence: 0.0,
                            reasoning: "llm_invalid_schema".to_string(),
                            wakeup_in_ms: None,
                        },
                        last_latency_ms,
                        at_cap,
                    );
                }
            }
        }

        // Should not be reached
        (
            Decision {
                kind: DecisionKind::NoOp,
                tool: None,
                args: None,
                confidence: 0.0,
                reasoning: "decide_path_exhausted".to_string(),
                wakeup_in_ms: None,
            },
            last_latency_ms,
            at_cap,
        )
    }

    // -----------------------------------------------------------------------
    // _assemble_prompt — byte-identical to Python _assemble_prompt
    // -----------------------------------------------------------------------

    fn assemble_prompt(
        &self,
        personality: &PersonalityCard,
        state: &serde_json::Value,
        goals: &[serde_json::Value],
        memories: &[serde_json::Value],
        recent_decisions: &[Decision],
        hot_inputs: &HashMap<String, serde_json::Value>,
        bot_guid: i64,
        triage_reason: Option<&str>,
    ) -> Prompt {
        // Derive at_cap from state["self"]["level"] vs max_player_level
        let at_cap = self.derive_at_cap(state);
        let max_level = self.max_player_level;

        let default_role =
            class_default_role(&personality.class_.to_lowercase());

        // (1) Persona line + bot_guid + traits + party_invite_policy + default role
        let mut system = format!(
            "You are {name}, a {race} {class_} \
             in World of Warcraft. {backstory}\n\
             Your bot_guid is {bot_guid}. When a tool's args require bot_guid, \
             use exactly this integer ({bot_guid}). Do not use 0 or guess.\n\
             Personality traits (0..1 scale unless noted): \
             talkativeness={talkativeness}, \
             courage={courage}, \
             greed={greed}, \
             attitude_to_master={attitude_to_master} (range -1..1), \
             party_invite_policy={party_invite_policy}.\n\
             Your default dungeon role: {default_role}.\n",
            name = personality.name,
            race = personality.race,
            class_ = personality.class_,
            backstory = personality.backstory,
            bot_guid = bot_guid,
            talkativeness = personality.talkativeness,
            courage = personality.courage,
            greed = personality.greed,
            attitude_to_master = personality.attitude_to_master,
            party_invite_policy = personality.party_invite_policy,
            default_role = default_role,
        );

        // (2) At-cap paragraph — only when at cap
        if at_cap {
            system.push_str(&format!(
                "\nYou are at max level (L{max_level}). XP from quests no longer matters. \
                 Your end-game preferences (0..1): \
                 pvp_appetite={pvp}, \
                 raid_appetite={raid}, \
                 completionist_streak={comp}, \
                 gold_motivation={gold}, \
                 profession_appetite={prof}.\n\
                 Available end-game activities on this server: Black Fathom Deeps \
                 raid, battlegrounds (Warsong Gulch, Arathi Basin), dungeon farming, \
                 profession crafting, gold/AH play, zone completion. Let your \
                 preferences shape what you talk about wanting to do, even though \
                 the harness tools to execute these activities are not yet wired.\n",
                max_level = max_level,
                pvp  = fmt_optional_f64(personality.pvp_appetite),
                raid = fmt_optional_f64(personality.raid_appetite),
                comp = fmt_optional_f64(personality.completionist_streak),
                gold = fmt_optional_f64(personality.gold_motivation),
                prof = fmt_optional_f64(personality.profession_appetite),
            ));
        }

        // (3) Goal-management paragraph (ALWAYS rendered)
        system.push_str(concat!(
            "\nYour goals shape what you do across ticks. If you have no \
             active goals, create one now via goals.create — pick something \
             concrete that fits your personality (e.g., \"reach level 25\", \
             \"earn 5 gold by tomorrow\", \"run BFD with a group\"). Goals you \
             write here are your own — the brain owns them, they persist \
             across ticks, and you reference them in future decisions. If \
             you have an active goal, check whether you're making progress; \
             use goals.update to record progress or change tack, and \
             goals.complete when done.\n"
        ));

        // (4) Organic-wakeup paragraph — only when triage_reason == "organic_wakeup"
        if triage_reason == Some("organic_wakeup") {
            system.push_str(concat!(
                "\nYou woke up on your own — no one is asking you anything, no \
                 combat is happening, no invitations are pending. This is your \
                 own time. Decide what YOU want to do next based on your \
                 personality, your goals, and your current situation. If nothing \
                 important needs doing, that's a valid choice — emit no_op.\n\
                 Set wakeup_in_ms to a number of milliseconds reflecting \
                 your current engagement: shorter (60000-120000) if you're \
                 actively pursuing something or expecting an event soon; \
                 medium (180000-360000) if you're between activities; \
                 longer (480000-600000) if you're settled and content. Vary \
                 it based on your situation — don't pick the same value \
                 every time.\n\
                 Look at your recent decisions in this prompt. If you've done \
                 the same action 3+ times in a row, deliberately pick something \
                 different — a real character varies their activities. Repetition \
                 is fine; identical repetition isn't.\n"
            ));
        }

        // (5) JSON-only constraint + schema literal
        system.push_str(concat!(
            "\nYou make ONE decision per call. Respond with ONLY a single JSON object \
             matching this exact schema, and nothing else (no prose, no markdown, no preamble):\n"
        ));
        system.push_str(_DECISION_SCHEMA_LITERAL);

        // User carries only variable game-state context; no persona duplication.
        let tools_summary = self.tools_summary.as_deref().map(str::to_string).unwrap_or_else(|| {
            let mut sorted: Vec<&str> = self.known_tools.iter().map(|s| s.as_str()).collect();
            sorted.sort_unstable();
            python_json(&sorted)
        });

        let truncated_memories = self.truncate_memory_items(memories);
        let truncated_decisions = self.truncate_recent_decisions(recent_decisions);
        let projected_hot_inputs = self.project_hot_inputs(hot_inputs);

        let user = self
            .prompt_template
            .replace("{tools_summary}", &tools_summary)
            .replace("{state_json}", &python_json(state))
            .replace("{goals_json}", &python_json(goals))
            .replace("{memories_json}", &python_json(&truncated_memories))
            .replace("{recent_decisions_json}", &python_json(&truncated_decisions))
            .replace("{hot_inputs_json}", &python_json(&projected_hot_inputs));

        Prompt { system, user }
    }

    // -----------------------------------------------------------------------
    // derive_at_cap — mirrors Python decide() / _assemble_prompt at_cap logic
    // -----------------------------------------------------------------------

    fn derive_at_cap(&self, state: &serde_json::Value) -> bool {
        let self_obj = state.get("self");
        let self_level = self_obj
            .and_then(|v| v.get("level"))
            .and_then(|v| v.as_i64());
        match self_level {
            Some(level) => level >= self.max_player_level as i64,
            None => false,
        }
    }

    // -----------------------------------------------------------------------
    // _gather_memories — mirrors Python Decider._gather_memories()
    // -----------------------------------------------------------------------

    async fn gather_memories(
        &self,
        bot_guid: i64,
        hot_inputs: &HashMap<String, serde_json::Value>,
    ) -> Vec<serde_json::Value> {
        let mut entities: Vec<String> = Vec::new();

        if let Some(serde_json::Value::Array(chats)) = hot_inputs.get("fresh_chat") {
            for chat in chats {
                let sender = chat.get("from").and_then(|v| v.as_str())
                    .or_else(|| chat.get("sender").and_then(|v| v.as_str()));
                let content_text = chat.get("text").and_then(|v| v.as_str())
                    .or_else(|| chat.get("content").and_then(|v| v.as_str()))
                    .unwrap_or("");
                let sender = sender.map(str::to_string).or_else(|| {
                    self.sender_from_content(content_text).map(str::to_string)
                });
                if let Some(s) = sender {
                    entities.push(s);
                }
            }
        }

        // Deduplicate preserving order (mirrors Python dict.fromkeys)
        let mut seen: HashSet<String> = HashSet::new();
        let unique_entities: Vec<String> = entities
            .into_iter()
            .filter(|e| seen.insert(e.clone()))
            .collect();

        let bot_id_str = bot_guid.to_string();

        if !unique_entities.is_empty() {
            let mut seen_ids: HashSet<String> = HashSet::new();
            let mut merged: Vec<serde_json::Value> = Vec::new();

            for entity in &unique_entities {
                let resp = match self
                    .memory_mcp
                    .call(
                        "memory.recall_about",
                        serde_json::json!({
                            "bot_id": bot_id_str,
                            "entity": entity,
                            "top_k": 5
                        }),
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let inner = resp.get("result").cloned().unwrap_or(resp);
                if let Some(items) = inner.get("items").and_then(|v| v.as_array()) {
                    for item in items {
                        let item_id = item.get("id").or_else(|| item.get("memory_id"))
                            .and_then(|v| v.as_str());
                        if let Some(id) = item_id {
                            if seen_ids.insert(id.to_string()) {
                                merged.push(item.clone());
                            }
                        } else {
                            merged.push(item.clone());
                        }
                    }
                }
            }
            return merged;
        }

        // No entities — fall back to generic recall
        match self
            .memory_mcp
            .call(
                "memory.recall",
                serde_json::json!({
                    "bot_id": bot_id_str,
                    "query": "",
                    "top_k": 5
                }),
            )
            .await
        {
            Err(_) => vec![],
            Ok(resp) => {
                let inner = resp.get("result").cloned().unwrap_or(resp);
                inner
                    .get("items")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default()
            }
        }
    }

    // -----------------------------------------------------------------------
    // _truncate_memory_items — clips "text" and "content" at _MAX_CONTENT_CHARS
    // -----------------------------------------------------------------------

    fn truncate_memory_items(
        &self,
        items: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        items
            .iter()
            .map(|item| {
                let mut clipped = item.clone();
                if let serde_json::Value::Object(ref mut map) = clipped {
                    for field in ["text", "content"] {
                        if let Some(serde_json::Value::String(ref s)) = map.get(field).cloned() {
                            if s.chars().count() > _MAX_CONTENT_CHARS {
                                // Take the first _MAX_CONTENT_CHARS bytes then append ellipsis.
                                // Python uses s[:300] which is char-based; for ASCII text this
                                // is identical to byte-based, but we use char-boundary-safe slice.
                                let truncated = truncate_str(s, _MAX_CONTENT_CHARS);
                                map.insert(
                                    field.to_string(),
                                    serde_json::Value::String(format!("{truncated}\u{2026}")),
                                );
                            }
                        }
                    }
                }
                clipped
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // _truncate_recent_decisions — clips "reasoning" at _MAX_REASONING_CHARS
    // -----------------------------------------------------------------------

    fn truncate_recent_decisions(
        &self,
        decisions: &[Decision],
    ) -> Vec<serde_json::Value> {
        decisions
            .iter()
            .map(|d| {
                let mut dd = serde_json::to_value(d).expect("Decision serialization is infallible");
                if let serde_json::Value::Object(ref mut map) = dd {
                    if let Some(serde_json::Value::String(ref s)) = map.get("reasoning").cloned() {
                        if s.chars().count() > _MAX_REASONING_CHARS {
                            let truncated = truncate_str(s, _MAX_REASONING_CHARS);
                            map.insert(
                                "reasoning".to_string(),
                                serde_json::Value::String(format!("{truncated}\u{2026}")),
                            );
                        }
                    }
                }
                dd
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // _project_hot_inputs — flatten fresh_chat to {sender, message}
    // -----------------------------------------------------------------------

    fn project_hot_inputs(
        &self,
        hot_inputs: &HashMap<String, serde_json::Value>,
    ) -> serde_json::Map<String, serde_json::Value> {
        let mut out = serde_json::Map::new();
        for (key, val) in hot_inputs {
            if key == "fresh_chat" {
                if let serde_json::Value::Array(chats) = val {
                    let projected: Vec<serde_json::Value> = chats
                        .iter()
                        .map(|c| {
                            let content_text = c.get("text").and_then(|v| v.as_str())
                                .or_else(|| c.get("content").and_then(|v| v.as_str()))
                                .unwrap_or("")
                                .to_string();
                            let sender = c.get("from").and_then(|v| v.as_str())
                                .map(str::to_string)
                                .or_else(|| c.get("sender").and_then(|v| v.as_str()).map(str::to_string))
                                .or_else(|| self.sender_from_content(&content_text).map(str::to_string))
                                .unwrap_or_else(|| "?".to_string());
                            let msg = if content_text.chars().count() > _MAX_CONTENT_CHARS {
                                format!("{}\u{2026}", truncate_str(&content_text, _MAX_CONTENT_CHARS))
                            } else {
                                content_text
                            };
                            serde_json::json!({"sender": sender, "message": msg})
                        })
                        .collect();
                    out.insert(key.clone(), serde_json::Value::Array(projected));
                } else {
                    out.insert(key.clone(), val.clone());
                }
            } else {
                out.insert(key.clone(), val.clone());
            }
        }
        out
    }

    // -----------------------------------------------------------------------
    // _sender_from_content — extract sender name from T3 whisper content
    // -----------------------------------------------------------------------

    fn sender_from_content<'a>(&self, content: &'a str) -> Option<&'a str> {
        WHISPER_SENDER_RE
            .captures(content)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format an `Option<f64>` exactly as Python does: `0.4` not `0.40000000000000002`.
/// For values that are None, outputs "None" (matching Python's str() repr).
fn fmt_optional_f64(v: Option<f64>) -> String {
    match v {
        Some(f) => format_f64_python(f),
        None => "None".to_string(),
    }
}

/// Format a f64 the way Python's f-string does: avoids trailing zeros.
/// Python uses repr-style: `0.4` not `0.4000000000000001`.
/// Rust `{}` format also gives `0.4` for `0.4f64` — they agree for "clean" floats.
fn format_f64_python(f: f64) -> String {
    // Rust's `{}` for f64 matches Python's f-string float formatting
    // for the personality trait values used here (all 1-decimal-place).
    format!("{f}")
}

/// Truncate a string to at most `max_chars` Unicode scalar values.
/// Returns a &str slice ending at a char boundary.
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
    }
}

// ---------------------------------------------------------------------------
// NullMcp — used by new_test() helpers; always returns empty responses
// ---------------------------------------------------------------------------

struct NullMcp;

impl McpCallable for NullMcp {
    fn call<'a>(
        &'a self,
        _tool: &'a str,
        _args: serde_json::Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<serde_json::Value, anyhow::Error>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { Ok(serde_json::json!({"result": {"items": []}})) })
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_card() -> PersonalityCard {
        PersonalityCard {
            name: "Kael".into(),
            race: "Blood Elf".into(),
            class_: "Paladin".into(),
            backstory: "A noble warrior seeking redemption.".into(),
            talkativeness: 0.7,
            courage: 0.8,
            greed: 0.3,
            attitude_to_master: 0.0,
            party_invite_policy: "accept_from_known".into(),
            pvp_appetite: Some(0.4),
            raid_appetite: Some(0.9),
            completionist_streak: Some(0.6),
            gold_motivation: Some(0.3),
            profession_appetite: Some(0.5),
        }
    }

    #[test]
    fn test_known_tools_count() {
        assert_eq!(KNOWN_TOOLS.len(), 62, "KNOWN_TOOLS must have 62 entries to match Python");
    }

    #[test]
    fn test_decision_schema_literal_contains_kind() {
        assert!(
            _DECISION_SCHEMA_LITERAL.contains("\"kind\""),
            "schema literal must contain 'kind'"
        );
        assert!(
            _DECISION_SCHEMA_LITERAL.contains("wakeup_in_ms"),
            "schema literal must contain 'wakeup_in_ms'"
        );
    }

    #[test]
    fn test_python_json_array_spacing() {
        // Python json.dumps(["a", "b"]) == '["a", "b"]' (space after comma)
        let v = serde_json::json!(["a", "b"]);
        assert_eq!(python_json(&v), r#"["a", "b"]"#);
    }

    #[test]
    fn test_python_json_object_spacing() {
        // Python json.dumps({"x": 1}) == '{"x": 1}' (space after colon and comma)
        let v = serde_json::json!({"x": 1, "y": 2});
        let s = python_json(&v);
        assert!(s.contains(": "), "must use ': ' separator");
        // serde_json preserves insertion order (preserve_order feature)
        assert_eq!(s, r#"{"x": 1, "y": 2}"#);
    }

    #[test]
    fn test_python_json_nested() {
        let v = serde_json::json!({"a": {"b": 1}, "c": [1, 2]});
        assert_eq!(python_json(&v), r#"{"a": {"b": 1}, "c": [1, 2]}"#);
    }

    #[test]
    fn test_class_default_role_lookup() {
        assert_eq!(class_default_role("paladin"), "tank or healer");
        assert_eq!(class_default_role("warrior"), "tank");
        assert_eq!(class_default_role("unknown_class"), "dps");
    }

    #[test]
    fn test_truncate_str() {
        let s = "x".repeat(400);
        let t = truncate_str(&s, 300);
        assert_eq!(t.chars().count(), 300);
    }

    #[test]
    fn test_persona_line_format() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card.clone(), "t");
        let prompt = decider.assemble_prompt_test(
            &card,
            &serde_json::json!({}),
            &[],
            &[],
            &[],
            &HashMap::new(),
            1001,
            None,
        );
        // Must contain "You are Kael, a Blood Elf Paladin in World of Warcraft."
        assert!(
            prompt.system.starts_with("You are Kael, a Blood Elf Paladin in World of Warcraft."),
            "system prompt must start with persona line; got: {}",
            &prompt.system[..80.min(prompt.system.len())]
        );
        assert!(prompt.system.contains("bot_guid is 1001"));
        assert!(prompt.system.contains("talkativeness=0.7"));
        assert!(prompt.system.contains("party_invite_policy=accept_from_known"));
        assert!(prompt.system.contains("Your default dungeon role: tank or healer."));
    }

    #[test]
    fn test_at_cap_paragraph_present_when_at_cap() {
        let card = fixture_card();
        let state = serde_json::json!({"self": {"level": 25}});
        let decider = Decider::new_test_with_max_level(1001, card.clone(), "t", 25);
        let prompt = decider.assemble_prompt_test(
            &card, &state, &[], &[], &[], &HashMap::new(), 1001, None,
        );
        assert!(prompt.system.contains("at max level"), "at-cap paragraph missing");
        assert!(prompt.system.contains("pvp_appetite=0.4"), "pvp_appetite must appear");
        assert!(prompt.system.contains("raid_appetite=0.9"), "raid_appetite must appear");
    }

    #[test]
    fn test_at_cap_paragraph_absent_when_below_cap() {
        let card = fixture_card();
        let state = serde_json::json!({"self": {"level": 24}});
        let decider = Decider::new_test_with_max_level(1001, card.clone(), "t", 25);
        let prompt = decider.assemble_prompt_test(
            &card, &state, &[], &[], &[], &HashMap::new(), 1001, None,
        );
        assert!(!prompt.system.contains("at max level"), "not-at-cap must not have at-cap para");
    }

    #[test]
    fn test_goal_management_paragraph_always_present() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card.clone(), "t");
        let prompt = decider.assemble_prompt_test(
            &card, &serde_json::json!({}), &[], &[], &[], &HashMap::new(), 1001, None,
        );
        assert!(
            prompt.system.contains("Your goals shape what you do across ticks"),
            "goal management paragraph must always be present"
        );
    }

    #[test]
    fn test_organic_wakeup_paragraph_present_when_reason_matches() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card.clone(), "t");
        let prompt = decider.assemble_prompt_test(
            &card, &serde_json::json!({}), &[], &[], &[], &HashMap::new(), 1001,
            Some("organic_wakeup"),
        );
        assert!(
            prompt.system.contains("woke up on your own"),
            "organic_wakeup paragraph must be present"
        );
    }

    #[test]
    fn test_organic_wakeup_paragraph_absent_when_other_reason() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card.clone(), "t");
        let prompt = decider.assemble_prompt_test(
            &card, &serde_json::json!({}), &[], &[], &[], &HashMap::new(), 1001,
            Some("fresh_chat"),
        );
        assert!(
            !prompt.system.contains("woke up on your own"),
            "organic_wakeup paragraph must NOT be present for non-organic reason"
        );
    }

    #[test]
    fn test_schema_literal_at_end_of_system_prompt() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card.clone(), "t");
        let prompt = decider.assemble_prompt_test(
            &card, &serde_json::json!({}), &[], &[], &[], &HashMap::new(), 1001, None,
        );
        assert!(
            prompt.system.ends_with(_DECISION_SCHEMA_LITERAL),
            "system prompt must end with _DECISION_SCHEMA_LITERAL"
        );
    }

    #[test]
    fn test_truncate_memory_items_clips_text() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card, "t");
        let long_text = "x".repeat(400);
        let items = vec![serde_json::json!({"text": long_text, "id": "abc"})];
        let truncated = decider.truncate_memory_items_test(&items);
        let t = truncated[0]["text"].as_str().unwrap();
        // 300 bytes + "…" (3 UTF-8 bytes = 1 char)
        assert!(t.ends_with('\u{2026}'), "must end with ellipsis");
        assert_eq!(t.chars().count(), 301, "must be 300 chars + ellipsis");
    }

    #[test]
    fn test_truncate_memory_items_clips_content_field() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card, "t");
        let long_text = "y".repeat(400);
        let items = vec![serde_json::json!({"content": long_text})];
        let truncated = decider.truncate_memory_items_test(&items);
        let t = truncated[0]["content"].as_str().unwrap();
        assert!(t.ends_with('\u{2026}'));
        assert_eq!(t.chars().count(), 301);
    }

    #[test]
    fn test_project_hot_inputs_flattens_fresh_chat() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card, "t");
        let mut inputs = HashMap::new();
        inputs.insert(
            "fresh_chat".to_string(),
            serde_json::json!([{"text": "received whisper from Alice: hello", "from": "Alice"}]),
        );
        let projected = decider.project_hot_inputs_test(&inputs);
        let chat = &projected["fresh_chat"][0];
        assert_eq!(chat["sender"], "Alice");
        assert!(chat["message"].as_str().unwrap().contains("received whisper"));
        // "from" field must not be passed through
        assert!(chat.get("from").is_none(), "raw 'from' field must not appear in projected");
    }

    #[test]
    fn test_project_hot_inputs_whisper_regex_fallback() {
        // No "from" field — sender extracted via regex from "text"
        let card = fixture_card();
        let decider = Decider::new_test(1001, card, "t");
        let mut inputs = HashMap::new();
        inputs.insert(
            "fresh_chat".to_string(),
            serde_json::json!([{"text": "received whisper from Bob: greetings"}]),
        );
        let projected = decider.project_hot_inputs_test(&inputs);
        let chat = &projected["fresh_chat"][0];
        assert_eq!(chat["sender"], "Bob", "must extract sender from whisper text via regex");
    }

    #[test]
    fn test_project_hot_inputs_unknown_sender_is_question_mark() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card, "t");
        let mut inputs = HashMap::new();
        inputs.insert(
            "fresh_chat".to_string(),
            serde_json::json!([{"text": "random text no sender"}]),
        );
        let projected = decider.project_hot_inputs_test(&inputs);
        assert_eq!(projected["fresh_chat"][0]["sender"], "?");
    }

    #[test]
    fn test_project_hot_inputs_other_keys_passthrough() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card, "t");
        let mut inputs = HashMap::new();
        inputs.insert("combat_events".to_string(), serde_json::json!([{"type": "hit"}]));
        let projected = decider.project_hot_inputs_test(&inputs);
        assert!(projected.get("combat_events").is_some(), "non-fresh_chat keys must pass through");
    }

    #[test]
    fn test_user_prompt_substitutes_all_fields() {
        let card = fixture_card();
        let template = "{tools_summary}|{state_json}|{goals_json}|{memories_json}|{recent_decisions_json}|{hot_inputs_json}";
        let decider = Decider::new_test(1001, card.clone(), template);
        let prompt = decider.assemble_prompt_test(
            &card, &serde_json::json!({}), &[], &[], &[], &HashMap::new(), 1001, None,
        );
        // All 6 named placeholders must be replaced
        for placeholder in [
            "{tools_summary}", "{state_json}", "{goals_json}",
            "{memories_json}", "{recent_decisions_json}", "{hot_inputs_json}",
        ] {
            assert!(
                !prompt.user.contains(placeholder),
                "user prompt must not contain placeholder: {placeholder}"
            );
        }
        // The user string must contain 5 separator '|' characters (from our template)
        assert_eq!(prompt.user.chars().filter(|&c| c == '|').count(), 5);
    }
}
