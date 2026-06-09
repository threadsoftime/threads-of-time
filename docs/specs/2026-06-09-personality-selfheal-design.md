# Spec: PersonalityCache self-heal for empty/blank/unparseable persona

Date: 2026-06-09
Branch: `feat/personality-selfheal` (off `dev`)
Owner: agent-orchestration-architect (V3 brain)

## Problem (confirmed live, systematic-debugging Phase 1)

`Decider::decide()` (decide.rs:451) calls `personality_cache.get(bot_guid)`. In
`PersonalityCache::get()` (personality.rs:187-192) the persona string from
`memory.personality_get` is parsed with:

```rust
let persona_json = payload.get("persona").and_then(|v| v.as_str()).unwrap_or("{}");
let mut card: PersonalityCard = serde_json::from_str(persona_json)
    .map_err(|e| anyhow::anyhow!("personality JSON parse error: {e}"))?;
```

For bot **1083** the stored `persona` is an **empty string `""`**:
- `as_str()` returns `Some("")`, so `unwrap_or("{}")` does NOT fire.
- `from_str("")` → serde error → `Err` propagates → `decide()` returns a
  `no_op` with `error_class="personality_error"`.

Result: 1083 emits `personality_error → no_op` on **every** decide tick
(`organic_wakeup`, ~every 5 min). Live decisions.jsonl: **140 records, all 1083,
spanning ~19.6 h continuous**. 17 other deciding bots are healthy.

Note the `null`/missing case is ALSO broken: `memory.personality_get` MCP returns
`{"persona": null}` for a missing/NULL row (handler.rs:388), so `as_str()` →
`None` → `unwrap_or("{}")` fires → `from_str("{}")` → fails (PersonalityCard has
8 required non-default fields). So neither empty nor null nor `{}` recovers today.

### Origin (best-effort, not a blocker)

The brain `state.sqlite` `living_bots` row for 1083 has a **GOOD**
`personality_seed_json` (name="Morenette", race="Night Elf", class="Warrior",
full backstory, v2 fields null). status=active. But the memory store `persona`
(separate 11 GB db.sqlite, read via `memory.personality_get`) is empty.

Divergence between the two stores: the enroll-time write-through
(`personality_cache.seed()` → `memory.personality_set`) either never landed for
1083, or the memory row was later cleared to `""`. 1083 **never** had a
successful real LLM decide; the bug only became *visible* when the
`llm_error_class` field was added (sha 89ed203fc) — earlier ticks surfaced as
plain `no_op` with null latency.

### Parity note

The Python original (`brain_sidecar/personality.py:83-84`) has the SAME latent
bug (`payload.get("persona", "{}")` then `json.loads`). This is NOT a Rust port
regression — it's a latent defect in both, exposed by 1083's corrupt data. The
Rust port additionally dropped the cache's `llm_client` (Python wires it; Rust
sets `morph: None` in production), so the lazy v2 migration morph path is a
no-op-log in prod today.

## Requirements

- **(a) Robustness.** `get()` must never spam-fail-to-no_op-forever on an
  empty/blank/null/unparseable/invalid persona. Treat empty/blank/null/`{}`/
  unparseable all the same: "no usable persona → recover".
- **(b) Identity-preserving recovery.** The affected bot must end up with a REAL,
  working personality reflecting its actual in-game identity (name/race/class) —
  not a generic placeholder. Mechanism mirrors enroll: identity + morph + persist.
  Generic-name fallback only when identity is genuinely unavailable, and then it
  is a clearly-logged degraded path, not silent.

## Design

Introduce an injected recovery capability on `PersonalityCache`, mirroring the
existing `McpCallable` / `MorphCallable` injection pattern (keeps the cache
decoupled and unit-testable, no concrete coupling to state_store/harness/llm).

```rust
/// Produces a fully-valid, morphed PersonalityCard for a bot when no usable
/// persona is stored. Implementations decide the identity source.
pub trait PersonalityRecovery: Send + Sync {
    fn recover<'a>(&'a self, bot_guid: i64)
        -> Pin<Box<dyn Future<Output = Result<PersonalityCard, anyhow::Error>> + Send + 'a>>;
}
```

`PersonalityCache` gains `recovery: Option<Arc<dyn PersonalityRecovery>>`.

### `get()` change (the only behavioral change)

After fetching the persona string, decide "usable vs not":

```
persona_str = payload.persona as &str  (None if key missing/null)
let usable_card: Option<PersonalityCard> =
    persona_str
      .filter(|s| !s.trim().is_empty())          // empty/blank → not usable
      .and_then(|s| serde_json::from_str(s).ok());// unparseable/invalid → None
```

- If `usable_card` is `Some(card)` → existing flow (then v2 morph-migration if
  `needs_morph`, unchanged).
- If `usable_card` is `None` → **self-heal**:
  - if `recovery` is `Some` → `card = recovery.recover(bot_guid).await?`;
    persist via `memory.personality_set` (soft-fail on persist error, same as the
    existing migration-persist soft-fail); cache; return.
  - if `recovery` is `None` (e.g. a minimal test cache) → preserve the OLD
    error behavior (return the parse error) so nothing silently degrades where no
    recovery is wired. (Production always wires recovery.)

This persists the recovered card, so the bot heals **permanently** — no per-tick
spam (requirement a). The morph-migration branch for already-usable v1 cards is
unchanged (don't break v1→v2). Cache/TTL/double-check/lock logic unchanged.
`seed()` unchanged. Prompt assembly (decide.rs) untouched (parity-safe).

### Production recovery impl: `LiveRecovery` (in app.rs)

Layered identity fidelity (best → degraded), then morph:

1. **state_store seed** — `state_store.get_bot(guid).personality_seed` if its
   name/race/class are non-empty and not the bootstrap sentinels
   ("Unknown"/"?"/""). Richest source (real backstory + traits). For 1083 this
   yields "Morenette" with zero network calls.
2. **obs.get_state identity** — harness MCP, name/race/class on a default
   backstory. Identical to the enroll path. Used when (1) is absent/sentinel.
3. **degraded generic** — `name="Adventurer", race="Unknown", class="Warrior"`,
   generic backstory, with a loud `warn!("personality_recover degraded ...")`.
   Only when both (1) and (2) are unavailable.
4. **morph** — `morph_personality(card, llm_client)` fills v2 fields (itself
   soft-fails to random seed values). Always run.

`LiveRecovery` holds `Arc<StateStore>`, `Arc<McpClient>` (harness),
`Arc<LlmClient>`. Wired in `create_app` after those exist; `PersonalityCache::new`
gains the recovery arg (or a `with_recovery` setter to minimize call-site churn).

### Factor a shared seed-from-obs helper (de-dup)

`enroll_route` (app.rs:170-217) and `enroll_via_api` (app.rs:861-885) both do
"obs.get_state → fill name/race/class". Extract
`async fn fill_identity_from_obs(harness, guid, &mut card)` used by both AND by
`LiveRecovery` step 2. Pure refactor; covered by existing enroll tests +
new recovery tests.

## TDD (failing first)

Unit tests in `personality.rs` (mock recovery + mock mcp):
1. `get_empty_string_persona_triggers_recovery` — mcp returns `{"persona": ""}`,
   recovery returns a valid card → `get()` returns that card (NOT an error),
   AND calls `memory.personality_set` (persist), AND a second `get()` is a cache
   hit (no spam / no second recovery call).
2. `get_blank_whitespace_persona_triggers_recovery` — `{"persona": "   "}`.
3. `get_null_persona_triggers_recovery` — `{"persona": null}`.
4. `get_malformed_json_persona_triggers_recovery` — `{"persona": "{not json"}`.
5. `get_valid_persona_does_not_trigger_recovery` — usable card → recovery NOT
   called; existing path; morph-migration still works (regression guard).
6. `get_no_recovery_wired_preserves_error` — recovery=None + empty persona →
   Err (old behavior preserved where no recovery configured).

`LiveRecovery` tests (in app.rs or a recovery module):
7. `recover_prefers_state_store_seed` — state_store has good seed → returned card
   has that name/race/class (no obs needed).
8. `recover_falls_back_to_obs_identity` — no state_store seed → obs.get_state
   identity used.
9. `recover_degraded_generic_when_obs_unavailable` — neither → generic card +
   warn; card is still valid (v2 fields filled via morph).

All v2 fields populated post-recovery (morph runs) in 7-9.

## 1083 recovery on deploy

After this ships and the brain restarts, on 1083's first decide tick `get()`
finds the empty persona → recovery → state_store seed (Morenette, Night Elf,
Warrior) → morph → `personality_set` persists it to the memory store → cached.
**1083 self-heals automatically; no manual re-seed required.** (A manual
data-only re-seed via the memory MCP is the fallback if, e.g., the state_store
row were also missing — not needed here.)

## Out of scope / guardrails

- No C++/worldserver change.
- No memory schema change (memory-system-designer's domain).
- No prompt-assembly change (parity-sensitive).
- Don't break v1→v2 morph migration or cache/TTL/lock logic.
- Don't stage unrelated working-tree strays.
