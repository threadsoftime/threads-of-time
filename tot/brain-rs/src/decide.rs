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
// DecideOutcome — returned by Decider::decide()
// ---------------------------------------------------------------------------

/// Result of `Decider::decide` — decision plus observability metadata.
pub struct DecideOutcome {
    pub decision: Decision,
    pub latency_ms: Option<f64>,
    pub at_cap: bool,
    /// None on success; else "timeout"|"transport"|"http_<code>"|"parse"|"invalid_schema"|"personality_error".
    pub error_class: Option<String>,
    pub json_schema_fell_back: bool,
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
    /// Returns a `DecideOutcome` carrying the decision plus observability metadata.
    /// * `latency_ms` = duration of the most recent LLM call in ms, or `None`.
    /// * `at_cap` = true when the bot's level >= `self.max_player_level`.
    /// * `error_class` = None on success; else the error class string.
    /// * `json_schema_fell_back` = true when the LLM did not support structured output.
    pub async fn decide(
        &self,
        bot_guid: i64,
        hot_inputs: &HashMap<String, serde_json::Value>,
        triage_reason: Option<&str>,
    ) -> DecideOutcome {
        // V3.6: derive at_cap up front
        let state_summary = hot_inputs.get("state_summary").cloned().unwrap_or(serde_json::json!({}));
        let at_cap = self.derive_at_cap(&state_summary);

        // Gather context
        let card = match self.personality_cache.get(bot_guid).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("decide: personality_cache.get failed bot_guid={bot_guid}: {e}");
                return DecideOutcome {
                    decision: Decision {
                        kind: DecisionKind::NoOp, tool: None, args: None,
                        confidence: 0.0, reasoning: format!("personality_error: {e}"),
                        wakeup_in_ms: None,
                    },
                    latency_ms: None, at_cap,
                    error_class: Some("personality_error".to_string()),
                    json_schema_fell_back: false,
                };
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
                    // M2: include pending so self-authored goals.create goals (created
                    // pending, never auto-activated) appear in the prompt's ACTIVE GOALS
                    // list — otherwise the LLM never sees its own goals and loops
                    // re-creating them. memory-rs goals.list supports the comma form
                    // (status IN (...)). Intentional Rust-side divergence from the Python
                    // brain; the fetched-goals change is upstream of assemble_prompt, so
                    // the prompt-assembly goldens are unaffected.
                    serde_json::json!({"bot_id": bot_guid.to_string(), "status": "active,pending"}),
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
        let mut json_schema_fell_back = false;

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

            let r = match result {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("decide: llm_client error attempt={attempt}: {e}");
                    if e.json_schema_fell_back { json_schema_fell_back = true; }
                    return DecideOutcome {
                        decision: Decision {
                            kind: DecisionKind::NoOp, tool: None, args: None,
                            confidence: 0.0, reasoning: format!("llm_error: {e}"),
                            wakeup_in_ms: None,
                        },
                        latency_ms: Some(e.elapsed_ms), at_cap,
                        error_class: Some(e.class.as_str()),
                        json_schema_fell_back,
                    };
                }
            };
            last_latency_ms = Some(r.latency_ms);
            if r.json_schema_fell_back { json_schema_fell_back = true; }
            let parsed = r.parsed;
            let raw = r.raw;

            if parsed.is_none() {
                // Unparseable — retry once with a precise restatement of the schema.
                if attempt < self.max_retries as usize {
                    user.push_str(&format!(
                        "\n\nYour previous response was not a valid JSON object. \
                         Emit ONLY this structure and nothing else: {_DECISION_SCHEMA_LITERAL}"
                    ));
                    continue;
                }
                // Char-safe truncation: Python's raw[:120] slices by CHARACTER, not byte.
                // &raw[..120] panics when byte 120 is not a UTF-8 char boundary.
                // chars().take(120).collect() is byte-safe AND matches Python semantics.
                let snippet: String = raw.chars().take(120).collect();
                return DecideOutcome {
                    decision: Decision {
                        kind: DecisionKind::NoOp, tool: None, args: None,
                        confidence: 0.0, reasoning: format!("llm_unparseable: {snippet}"),
                        wakeup_in_ms: None,
                    },
                    latency_ms: last_latency_ms, at_cap,
                    error_class: Some("parse".to_string()),
                    json_schema_fell_back,
                };
            }

            let parsed_val = parsed.unwrap();
            match serde_json::from_value::<Decision>(parsed_val) {
                Ok(decision) => {
                    return DecideOutcome {
                        decision, latency_ms: last_latency_ms, at_cap,
                        error_class: None, json_schema_fell_back,
                    };
                }
                Err(_) => {
                    if attempt < self.max_retries as usize {
                        user.push_str(&format!(
                            "\n\nYour previous JSON did not match the required schema. \
                             Emit ONLY this structure: {_DECISION_SCHEMA_LITERAL}"
                        ));
                        continue;
                    }
                    return DecideOutcome {
                        decision: Decision {
                            kind: DecisionKind::NoOp, tool: None, args: None,
                            confidence: 0.0, reasoning: "llm_invalid_schema".to_string(),
                            wakeup_in_ms: None,
                        },
                        latency_ms: last_latency_ms, at_cap,
                        error_class: Some("invalid_schema".to_string()),
                        json_schema_fell_back,
                    };
                }
            }
        }

        // Should not be reached
        DecideOutcome {
            decision: Decision {
                kind: DecisionKind::NoOp, tool: None, args: None,
                confidence: 0.0, reasoning: "decide_path_exhausted".to_string(),
                wakeup_in_ms: None,
            },
            latency_ms: last_latency_ms, at_cap,
            error_class: Some("path_exhausted".to_string()),
            json_schema_fell_back,
        }
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
        //
        // Route each f64 trait through format_f64_python so that whole-number
        // values (0.0, 1.0, -1.0) render as "0.0" / "1.0" / "-1.0", matching
        // the Python f-string behaviour rather than Rust Display's "0" / "1" / "-1".
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
            talkativeness = format_f64_python(personality.talkativeness),
            courage = format_f64_python(personality.courage),
            greed = format_f64_python(personality.greed),
            attitude_to_master = format_f64_python(personality.attitude_to_master),
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

        // Use python_format() — NOT plain .replace() — because the template contains
        // `{{`/`}}` escape sequences (JSON examples in few-shot blocks) that Python's
        // str.format() unescapes to literal `{`/`}`.  Plain .replace() would leave
        // them as `{{`/`}}`, corrupting every JSON example the LLM sees in the prompt.
        let user = python_format(
            &self.prompt_template,
            &[
                ("tools_summary", tools_summary.as_str()),
                ("state_json",    &python_json(state)),
                ("goals_json",    &python_json(goals)),
                ("memories_json", &python_json(&truncated_memories)),
                ("recent_decisions_json", &python_json(&truncated_decisions)),
                ("hot_inputs_json", &python_json(&projected_hot_inputs)),
            ],
        );

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

/// Format a f64 the way Python's f-string does: avoids trailing zeros for
/// non-whole decimals, but emits a trailing ".0" for whole-number values.
///
/// Python `f"{v}"` on a float uses shortest-round-trip notation:
///   f"{0.4}" -> "0.4"   (agrees with Rust `format!("{}", 0.4)`)
///   f"{1.0}" -> "1.0"   (Rust `format!("{}", 1.0)` gives "1" — DIVERGES)
///   f"{0.0}" -> "0.0"   (Rust `format!("{}", 0.0)` gives "0" — DIVERGES)
///
/// This function restores Python parity for the whole-number case.
/// Non-whole decimals already agree (both use shortest round-trip).
///
/// Personality trait values span [0.0, 1.0] with no clamping away from
/// exactly-0.0 / exactly-1.0, so this fix is reachable in the at-cap
/// paragraph and the persona-line traits (talkativeness, courage, etc.).
///
/// See `format_f64_python_whole_number_matches_python_fstring` test.
fn format_f64_python(f: f64) -> String {
    // Python f-string renders whole-number floats with a trailing ".0"
    // (f"{1.0}" -> "1.0"); Rust's Display drops it (format!("{}", 1.0) -> "1").
    // Personality trait values are in [0.0, 1.0] and can be exactly 0.0 or 1.0,
    // so restore Python parity for the whole-number case. Non-whole decimals
    // already agree (both use shortest round-trip).
    if f.is_finite() && f.fract() == 0.0 {
        format!("{f:.1}")          // 1.0 -> "1.0", 0.0 -> "0.0"
    } else {
        format!("{f}")             // 0.4 -> "0.4"
    }
}

/// Faithful equivalent of Python's `str.format(**kwargs)` for the decide_v1.txt template.
///
/// Python `str.format()` does two things that plain `str::replace` does NOT:
/// 1. Substitutes `{name}` with the supplied value.
/// 2. Unescapes `{{` → `{` and `}}` → `}` (double-brace escape sequences).
///
/// The decide_v1.txt template uses `{{`/`}}` extensively in its few-shot JSON examples
/// so that they survive `.format()` unmodified as literal braces. Plain `.replace()` would
/// leave those double-braces in the final prompt, corrupting every JSON example the LLM
/// sees. This function replicates Python's two-phase behaviour exactly:
///   Phase 1: replace each `{name}` with its value (longest-match-first to avoid partial
///             matches — `{tools_summary}` before `{tools}`).
///   Phase 2: replace `{{` → `{` and `}}` → `}`.
///
/// Contract: `subs` must list ALL named placeholders present in the template; any
/// unrecognised `{name}` left after phase 1 will be passed through unchanged (matching
/// Python's KeyError-on-missing behaviour would be stricter, but the template is static).
fn python_format(template: &str, subs: &[(&str, &str)]) -> String {
    // Phase 1: named substitutions. Apply in longest-key-first order to avoid a
    // shorter key matching a prefix of a longer one (e.g., "tools" vs "tools_summary").
    let mut result = template.to_string();
    let mut ordered: Vec<(&str, &str)> = subs.to_vec();
    ordered.sort_by(|a, b| b.0.len().cmp(&a.0.len())); // longest key first
    for (name, value) in &ordered {
        let placeholder = format!("{{{}}}", name);
        result = result.replace(&placeholder, value);
    }
    // Phase 2: unescape {{ → { and }} → } (Python str.format() semantics).
    result.replace("{{", "{").replace("}}", "}")
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
        // Integration-level guard for the format_f64_python reroute (decide.rs ~L612):
        // attitude_to_master=0.0 is the whole-number case — must render "0.0", not "0"
        // (Python f"{0.0}" -> "0.0"). courage/greed lock the other rerouted f64 traits.
        assert!(prompt.system.contains("attitude_to_master=0.0"),
            "whole-number trait must render as 0.0 not 0; got: {}",
            &prompt.system[..200.min(prompt.system.len())]);
        assert!(prompt.system.contains("courage=0.8"));
        assert!(prompt.system.contains("greed=0.3"));
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

    // -----------------------------------------------------------------------
    // Regression: raw[:120] char-safety (Fix 1)
    // -----------------------------------------------------------------------

    /// Regression: `raw.chars().take(120)` must not panic on multi-byte UTF-8
    /// and must slice by CHARACTER not byte (matching Python's `raw[:120]`).
    ///
    /// "—" is U+2014 EM DASH: 3 UTF-8 bytes. 40 of them = 120 chars / 120 bytes.
    /// An old `&raw[..120]` would have taken bytes 0..120 = exactly 40 em-dashes
    /// (lucky alignment), BUT with 41+ em-dashes it slices mid-char and panics.
    /// This test uses 50 em-dashes (150 bytes); chars().take(120) must yield
    /// exactly 120 characters (the first 120 chars of whatever multi-byte string
    /// is fed in).
    #[test]
    fn test_raw_truncation_char_safe_multibyte() {
        // 50 em-dashes: 50 chars, 150 bytes.
        // Python raw[:120] returns all 50 (< 120 chars), so snippet == raw.
        let raw_short: String = "—".repeat(50);
        let snippet_short: String = raw_short.chars().take(120).collect();
        assert_eq!(snippet_short.chars().count(), 50, "short string: all chars preserved");
        assert_eq!(snippet_short, raw_short, "short string: identical to input");

        // 200 em-dashes: 200 chars, 600 bytes.
        // Python raw[:120] returns the first 120 chars (= 120 em-dashes).
        // Old `&raw[..120]` would take 120 BYTES = 40 em-dashes — wrong AND
        // byte 120 is a char boundary here only by luck; byte 121 is mid-char.
        // Actually with em-dash (3 bytes each): byte 120 = 40*3, IS a boundary —
        // but the count is wrong (40 ≠ 120). Use a mix to force a mid-byte boundary:
        // interleave em-dash (3 bytes) so byte 120 is NOT a char boundary.
        // "a—" repeating: 'a' (1 byte) + '—' (3 bytes) = 4 bytes per pair, 2 chars.
        // 30 repetitions = 60 chars, 120 bytes.  Adding one more 'a' = char 61, byte 121.
        // So raw[..120] = 60 chars (byte-safe), raw[..121] would be mid-em-dash PANIC.
        // We want a string where the OLD &raw[..120] would give fewer chars than
        // chars().take(120). Use: "—a" repeated → 3+1=4 bytes per pair, 2 chars.
        // 30 pairs = 60 chars, 120 bytes. 31 pairs = 62 chars, 124 bytes.
        // Not a panic trigger here either. Best panic trigger: byte 120 inside a char.
        // "x" * 119 + "—" = 119 + 3 = 122 bytes. byte 120 = inside the em-dash. PANIC.
        let raw_panic_trigger = "x".repeat(119) + "—" + &"—".repeat(100);
        // This must NOT panic (would have panicked with old &raw[..120]):
        let snippet: String = raw_panic_trigger.chars().take(120).collect();
        assert_eq!(
            snippet.chars().count(),
            120,
            "must yield exactly 120 chars from multi-byte string"
        );
        // The first 119 chars are 'x', the 120th is '—'
        assert!(snippet.ends_with('—'), "120th char must be the em-dash");
        assert_eq!(&snippet[..119], "x".repeat(119), "first 119 chars must be 'x'");
    }

    // -----------------------------------------------------------------------
    // Regression: python_format() {{/}} unescaping (Fix 2)
    // -----------------------------------------------------------------------

    /// python_format() must substitute all 6 named placeholders AND unescape
    /// `{{` → `{` and `}}` → `}`, matching Python str.format() semantics.
    ///
    /// This locks in that the decide_v1.txt few-shot JSON examples reach the LLM
    /// as clean `{"key": "value"}` not corrupted `{{"key": "value"}}`.
    #[test]
    fn test_python_format_substitutes_and_unescapes_braces() {
        // Template with a named placeholder AND escaped braces (like decide_v1.txt examples)
        let template = "State: {state_json}, Example: {{\"kind\": \"action\"}}";
        let result = python_format(template, &[("state_json", "MYSTATE")]);
        assert_eq!(
            result,
            r#"State: MYSTATE, Example: {"kind": "action"}"#,
            "python_format must substitute AND unescape double-braces to single braces"
        );
    }

    /// When the template has NO double-brace escapes (plain placeholders only),
    /// python_format() behaves identically to chained .replace() calls.
    /// This documents the equivalence for templates without JSON examples.
    #[test]
    fn test_python_format_no_double_braces_matches_replace() {
        let template = "A={a_val} B={b_val}";
        let result = python_format(template, &[("a_val", "hello"), ("b_val", "world")]);
        assert_eq!(result, "A=hello B=world");
    }

    /// Verify that the real decide_v1.txt template, after python_format substitution,
    /// contains no stray `{{` or `}}` sequences and has no unresolved placeholders
    /// of the form `{name}` with alphabetic name.
    #[test]
    fn test_real_template_rendered_has_no_double_braces_or_unresolved_placeholders() {
        // CARGO_MANIFEST_DIR = <repo>/tot/brain-rs; the template now lives in-crate.
        let template_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/prompts/decide_v1.txt"
        );
        // If the template is not reachable from the test environment, skip gracefully.
        let tmpl = match std::fs::read_to_string(template_path) {
            Ok(s) => s,
            Err(_) => {
                // Template not available at this relative path in this build env — skip.
                return;
            }
        };
        let rendered = python_format(
            &tmpl,
            &[
                ("tools_summary", "TOOLS"),
                ("state_json", "{}"),
                ("goals_json", "[]"),
                ("memories_json", "[]"),
                ("recent_decisions_json", "[]"),
                ("hot_inputs_json", "{}"),
            ],
        );
        // After python_format(), ALL `{{` escape sequences must be unescaped to `{`.
        // Python .format() produces zero remaining `{{` in this template.
        assert!(
            !rendered.contains("{{"),
            "rendered template must not contain '{{' — all double-open-braces must be unescaped"
        );
        // Note: `}}` (two adjacent `}`) CAN legitimately remain in the output when the
        // template uses `}}}}` escape sequences for literal `}}` in JSON examples.
        // We do NOT assert on `}}` count — the template is meant to be edited freely
        // and the number of literal `}}` in JSON examples may change innocently.
        // No unresolved named placeholders: check for { followed by alpha chars followed by }
        let unresolved: Vec<&str> = {
            let mut found = vec![];
            let mut chars = rendered.char_indices().peekable();
            while let Some((i, c)) = chars.next() {
                if c == '{' {
                    // collect until }
                    let start = i + 1;
                    let rest = &rendered[start..];
                    if let Some(end_offset) = rest.find('}') {
                        let inner = &rest[..end_offset];
                        // A "named placeholder" is purely alphabetic/underscore, non-empty
                        if !inner.is_empty()
                            && inner.chars().all(|x| x.is_ascii_alphabetic() || x == '_')
                        {
                            found.push(&rendered[i..start + end_offset + 1]);
                        }
                    }
                }
            }
            found
        };
        assert!(
            unresolved.is_empty(),
            "rendered template has unresolved placeholders: {:?}",
            unresolved
        );
    }

    // -----------------------------------------------------------------------
    // format_f64_python / fmt_optional_f64 parity tests — LOCKED
    // -----------------------------------------------------------------------
    //
    // format_f64_python(f) now matches Python f-string behaviour for all cases:
    //   - Non-whole decimals: both Rust and Python use shortest round-trip.
    //     f"{0.4}" == "0.4"; format_f64_python(0.4) == "0.4" ✓
    //   - Whole numbers: Python emits trailing ".0"; Rust now matches.
    //     f"{1.0}" == "1.0"; format_f64_python(1.0) == "1.0" ✓  (FIXED)
    //     f"{0.0}" == "0.0"; format_f64_python(0.0) == "0.0" ✓  (FIXED)
    //
    // Fix applied: if f.is_finite() && f.fract() == 0.0 { format!("{f:.1}") }
    // else { format!("{f}") }
    //
    // Parity verified against: python3 -c "print(f'{1.0}', f'{0.0}', f'{0.4}', f'{0.5}', f'{1.5}')"
    //   -> "1.0 0.0 0.4 0.5 1.5"
    //
    // Sibling sites also fixed: persona-line traits (talkativeness, courage,
    // greed, attitude_to_master) are non-optional f64s rendered in assemble_prompt;
    // they now go through format_f64_python rather than bare Rust Display.

    /// In-scope values: representative 1-decimal-place personality trait floats.
    /// Oracle: python3 -c "for v in [0.4, 0.9, 0.6, 0.3, 0.5]: print(f'{v}')"
    ///   0.4 / 0.9 / 0.6 / 0.3 / 0.5 — Rust Display agrees; format_f64_python matches.
    #[test]
    fn format_f64_python_in_scope_values_match_python_fstring() {
        // python3 -c "print(f'{0.4}')" -> "0.4"
        assert_eq!(format_f64_python(0.4), "0.4");
        // python3 -c "print(f'{0.9}')" -> "0.9"
        assert_eq!(format_f64_python(0.9), "0.9");
        // python3 -c "print(f'{0.6}')" -> "0.6"
        assert_eq!(format_f64_python(0.6), "0.6");
        // python3 -c "print(f'{0.3}')" -> "0.3"
        assert_eq!(format_f64_python(0.3), "0.3");
        // python3 -c "print(f'{0.5}')" -> "0.5"
        assert_eq!(format_f64_python(0.5), "0.5");
    }

    /// Whole-number parity — LOCKED. Faithful-port fix: whole-number f64 now
    /// emits trailing ".0", matching Python f-string.
    ///
    /// Oracle: python3 -c "print(f'{1.0}')" -> "1.0"
    ///         python3 -c "print(f'{0.0}')" -> "0.0"
    ///         python3 -c "print(f'{-1.0}')" -> "-1.0"
    ///         python3 -c "print(f'{1.5}')" -> "1.5"
    #[test]
    fn format_f64_python_whole_number_matches_python_fstring() {
        // python3 -c "print(f'{1.0}')" -> "1.0"
        assert_eq!(format_f64_python(1.0), "1.0");
        // python3 -c "print(f'{0.0}')" -> "0.0"
        assert_eq!(format_f64_python(0.0), "0.0");
        // python3 -c "print(f'{-1.0}')" -> "-1.0"
        assert_eq!(format_f64_python(-1.0), "-1.0");
        // Non-whole: shortest round-trip path, no change.
        // python3 -c "print(f'{1.5}')" -> "1.5"
        assert_eq!(format_f64_python(1.5), "1.5");
    }

    /// fmt_optional_f64(None) must produce "None", matching Python f"{None}" -> "None".
    /// Oracle: python3 -c "print(f'{None}')" -> "None"
    #[test]
    fn fmt_optional_f64_none_produces_none_string() {
        // python3 -c "print(f'{None}')" -> "None"
        assert_eq!(fmt_optional_f64(None), "None");
    }

    /// fmt_optional_f64(Some(f)) delegates to format_f64_python.
    /// Oracle: python3 -c "print(f'{0.4}')" -> "0.4"; python3 -c "print(f'{1.0}')" -> "1.0"
    #[test]
    fn fmt_optional_f64_some_delegates_to_format_f64_python() {
        assert_eq!(fmt_optional_f64(Some(0.4)), "0.4");
        assert_eq!(fmt_optional_f64(Some(0.9)), "0.9");
        // Whole-number case: delegates correctly now that format_f64_python is fixed.
        // python3 -c "print(f'{1.0}')" -> "1.0"; python3 -c "print(f'{0.0}')" -> "0.0"
        assert_eq!(fmt_optional_f64(Some(1.0)), "1.0");
        assert_eq!(fmt_optional_f64(Some(0.0)), "0.0");
    }

    // -----------------------------------------------------------------------
    // M2: goals.list arg-shape + behavior tests
    // -----------------------------------------------------------------------

    /// RecordingMcp — returns a fixed JSON response AND records every (tool, args) pair.
    /// Used to assert what args decide() sends to goals.list.
    struct RecordingMcp {
        response: serde_json::Value,
        recorded: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl RecordingMcp {
        fn new(response: serde_json::Value) -> Arc<Self> {
            Arc::new(Self {
                response,
                recorded: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn recorded_args_for(&self, tool: &str) -> Vec<serde_json::Value> {
            self.recorded
                .lock()
                .unwrap()
                .iter()
                .filter(|(t, _)| t == tool)
                .map(|(_, a)| a.clone())
                .collect()
        }
    }

    impl McpCallable for RecordingMcp {
        fn call<'a>(
            &'a self,
            tool: &'a str,
            args: serde_json::Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, anyhow::Error>> + Send + 'a>,
        > {
            let response = self.response.clone();
            self.recorded.lock().unwrap().push((tool.to_string(), args));
            Box::pin(async move { Ok(response) })
        }
    }

    /// Build a Decider whose memory_mcp is the given RecordingMcp, with the personality
    /// cache pre-seeded so decide() reaches the goals.list call.
    ///
    /// The personality_cache uses NullMcp (returns Ok({})) for its own MCP calls.
    /// We seed the card into the LRU directly via cache.seed() so the cache-hit
    /// fast-path fires and no MCP call is needed on personality_cache.get().
    async fn decider_with_recording_mcp(
        bot_guid: i64,
        recording: Arc<RecordingMcp>,
    ) -> Decider {
        use std::num::NonZeroUsize;
        // NullMcp for the personality cache; memory.personality_set returns Ok({}).
        let persona_mcp: Arc<dyn McpCallable + Send + Sync> = Arc::new(NullMcp);
        let cache = Arc::new(crate::personality::PersonalityCache::new_with_options(
            persona_mcp,
            300.0,
            NonZeroUsize::new(8).unwrap().get(),
            Arc::new(|| 0.0),
            None,
        ));
        // Seed the card so the LRU hit path fires; NullMcp returns Ok({}) for
        // memory.personality_set so seed() succeeds.
        let card = fixture_card();
        // seed() calls memory.personality_set on the NullMcp, which returns Ok({"result":{"items":[]}}).
        // That is a valid Ok(), so seed succeeds and the card is in the LRU.
        let _ = cache.seed(bot_guid, card).await;

        let state_store = Arc::new(
            crate::state::StateStore::open(":memory:").expect("in-memory StateStore"),
        );
        state_store.migrate().expect("migrate");

        let known_tools: HashSet<String> = KNOWN_TOOLS.iter().map(|s| s.to_string()).collect();

        Decider {
            llm_client: Arc::new(LlmClient {
                base_url: "http://127.0.0.1:11434".to_string(),
                model: "test".to_string(),
                timeout_s: 5.0,
            }),
            personality_cache: cache,
            memory_mcp: recording as Arc<dyn McpCallable + Send + Sync>,
            state_store,
            prompt_template: "t".to_string(),
            decision_schema: None,
            tools_summary: None,
            max_retries: 1,
            known_tools,
            max_player_level: 25,
        }
    }

    /// M2 arg-shape: decide() must call goals.list with status="active,pending" so that
    /// self-authored goals (created pending, never auto-activated) appear in the prompt's
    /// ACTIVE GOALS section — otherwise the LLM never sees its own goals and loops
    /// re-creating them.
    ///
    /// We drive decide() until after the goals.list call (it will fail at the LLM HTTP
    /// call since no server is listening, which is fine — goals.list runs before the LLM
    /// call and the recorded args are visible regardless of later failure).
    #[tokio::test]
    async fn test_decide_goals_list_queries_active_and_pending() {
        let recording = RecordingMcp::new(serde_json::json!({
            "result": { "items": [] }
        }));
        let decider = decider_with_recording_mcp(1001, recording.clone()).await;

        // drive decide(); it will fail at the LLM HTTP step (no server) — that's fine.
        // The goals.list call happens before LLM and is unconditional once we reach it.
        let _ = decider.decide(1001, &HashMap::new(), None).await;

        let goals_calls = recording.recorded_args_for("goals.list");
        assert_eq!(goals_calls.len(), 1, "goals.list must be called exactly once per decide()");
        let status = goals_calls[0]
            .get("status")
            .and_then(|v| v.as_str())
            .expect("goals.list args must contain a 'status' field");
        assert_eq!(
            status, "active,pending",
            "M2 fix: decide() must pass status='active,pending' to goals.list"
        );
    }

    // --- DecideOutcome error-mapping tests ---
    use axum::{routing::post, Router, response::IntoResponse};

    async fn spawn_status_llm(status: axum::http::StatusCode, body: &'static str) -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || async move { (status, body).into_response() }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn decide_http_error_sets_class_and_latency() {
        let card = fixture_card();
        let mut decider = Decider::new_test(1001, card.clone(), "{tools_summary}");
        // Seed the personality card so decide() reaches the LLM call.
        decider.personality_cache.seed(1001, card).await.unwrap();
        let base = spawn_status_llm(axum::http::StatusCode::BAD_REQUEST, "no").await;
        decider.llm_client = std::sync::Arc::new(crate::llm_client::LlmClient {
            base_url: base, model: "test".into(), timeout_s: 5.0,
        });
        decider.decision_schema = Some(serde_json::json!({"oneOf": []}));
        let out = decider.decide(1001, &std::collections::HashMap::new(), Some("organic_wakeup")).await;
        assert_eq!(out.decision.kind, DecisionKind::NoOp);
        assert!(out.latency_ms.is_some(), "latency must be recorded even on error");
        assert_eq!(out.error_class.as_deref(), Some("http_400"));
        assert!(out.json_schema_fell_back);
    }

    #[tokio::test]
    async fn decide_success_has_no_error_class() {
        let card = fixture_card();
        let mut decider = Decider::new_test(1001, card.clone(), "{tools_summary}");
        // Seed the personality card so decide() reaches the LLM call.
        decider.personality_cache.seed(1001, card).await.unwrap();
        let base = spawn_status_llm(
            axum::http::StatusCode::OK,
            r#"{"choices":[{"message":{"content":"{\"kind\":\"no_op\",\"tool\":null,\"args\":null,\"confidence\":0.2,\"reasoning\":\"ok\"}"}}]}"#,
        ).await;
        decider.llm_client = std::sync::Arc::new(crate::llm_client::LlmClient {
            base_url: base, model: "test".into(), timeout_s: 5.0,
        });
        let out = decider.decide(1001, &std::collections::HashMap::new(), None).await;
        assert_eq!(out.decision.kind, DecisionKind::NoOp);
        assert!(out.error_class.is_none());
        assert!(!out.json_schema_fell_back);
        assert!(out.latency_ms.is_some());
    }

    /// M2 behavior: a pending goal fed directly to assemble_prompt_test appears in the
    /// assembled user prompt's goals_json section (ACTIVE GOALS).
    ///
    /// We test assemble_prompt_test directly because the full decide() path requires a
    /// live LLM; assemble_prompt is the exact function that builds the prompt text the
    /// LLM sees, so this is the correct assertion boundary.
    #[test]
    fn test_pending_goal_appears_in_assembled_prompt() {
        let card = fixture_card();
        let decider = Decider::new_test(1001, card.clone(), "goals: {goals_json}");
        let pending_goal = serde_json::json!({
            "id": "g1",
            "status": "pending",
            "description": "Explore Stormwind"
        });
        let prompt = decider.assemble_prompt_test(
            &card,
            &serde_json::json!({}),
            &[pending_goal],
            &[],
            &[],
            &HashMap::new(),
            1001,
            None,
        );
        assert!(
            prompt.user.contains("Explore Stormwind"),
            "pending goal description must appear in assembled user prompt; got: {}",
            &prompt.user[..200.min(prompt.user.len())]
        );
    }
}
