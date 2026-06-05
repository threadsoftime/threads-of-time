# Python→Rust Parity Coverage Map

**Branch:** `feat/tot-parity-hardening`
**Baseline test count:** 944 passed, 0 failed (all test binaries green)
**Produced by:** Task 1 enumeration — coverage map only, no test code.

---

## Root-cause reminder

`brain-rs/Cargo.toml` enables `serde_json` feature `preserve_order`. Cargo
resolver-2 unifies this flag ON workspace-wide, flipping every
`serde_json::Map` to insertion-order (IndexMap). Code that assumed alphabetic
key order — most critically audit SHA-256 — silently diverges from Python's
`json.dumps(sort_keys=True)` output. **Already fixed in harness-rs/src/audit.rs**
by a `canonicalize()` pre-pass before hashing.

---

## Status legend

| Status | Meaning |
|---|---|
| **LOCKED** | A Rust test already asserts the Python observable value (not just internal Rust behaviour). |
| **GAP** | No test asserts parity with Python. A lock test is warranted. |
| **DECISION** | Correct behaviour is ambiguous or involves an intentional schema divergence — operator input needed before locking. |
| **N/A** | Cannot diverge / not a parity-relevant surface (no Python observable at that site). |

---

## Class 1 — Hash / Canonicalization / Dedup / Idempotency / Cache keys

| Site (file:line) | Class | Python contract (observable) | Current Rust behaviour | Status |
|---|---|---|---|---|
| `harness-rs/src/audit.rs` — `sha256_args` + `canonicalize` | 1 | `hashlib.sha256(json.dumps(body, sort_keys=True, separators=(",",":")).encode()).hexdigest()` — sorted keys at every nesting depth | `canonicalize()` recursively sorts keys before hashing; exact digest fixtures pinned against Python-derived oracle values | **LOCKED** (exemplar — do not add tests here) |
| `brain-rs/src/dedup.rs` — `SeenMemoryIds::mark` / `seen` | 1 | `OrderedDict` LRU — `mark(x)` on existing id moves to most-recent; eviction pops oldest; `seen(x)` is membership test | `VecDeque` + `HashMap` — identical semantics: mark refreshes position, evicts oldest. Capacity tests exercise boundary | **LOCKED** (4 tests: mark+seen, eviction, idempotent-re-mark, never-exceed-capacity) |

No further Class-1 sites found outside `audit.rs` and `dedup.rs`. The
`memory-rs/src/bin/embed_stub.rs` SHA-256 is an internal fixture generator
(not a Python-parity surface); `memory-rs/src/db/mod.rs` "canonical storage"
refers to byte encoding, not sort-order.

---

## Class 2 — Float / NaN / Inf rendering on serialized DTOs

### Non-finite reachability analysis

The Python `recall.py` used a **v0.3 hybrid formula**:
`(alpha·bm25_norm + beta·dense_norm) · decay · (1+gamma·salience) + delta·entity_match`
whose seven float fields (`bm25_norm`, `dense_norm`, `decay`, `salience`,
`entity_match`, `salience_score`, `score`) populated a `RecallHit` DTO.

The Rust `memory-rs` uses a **v0.2 weighted-sum formula**:
`w_rel·cosine(emb,q) + w_rec·exp(−age/τ) + w_imp·clamp(salience,0,1)`
exposed in `memory-rs/src/retrieval/recall.rs::score_memory`. This is a
**different API revision** with a different field set — `RecallHit` with its
component breakdown does not exist in memory-rs; the response shape is
`{memory_id, text, score, ts}`. The Python v0.3 hybrid route was
`POST /v1/memory/{bot_guid}/recall` (episode-store path); the Rust v0.2 path
is `POST /memory/recall` (memory-store path). **These are not the same
endpoint.** The Python v0.3 hybrid was never ported to Rust — it was
superseded when the schema was revised. No parity gap exists on the v0.3
surface because the v0.3 Python code was retired, not ported.

**For the v0.2 scoring formula (the active Rust path):**

| Field | Formula path | Can go NaN? | Can go Inf? | Verdict |
|---|---|---|---|---|
| `relevance` (`cosine`) | `dot(a,b) / (norm(a)*norm(b))` | YES if both norms zero | NO | Reachable: zero-length embedding → 0/0 = NaN |
| `recency` | `exp(-age/τ)` | NO | NO | `age ≥ 0`, `τ > 0` always; result in `(0,1]` |
| `importance` | `clamp(salience, 0, 1)` | NO | NO | Bounded by definition |
| `score` (weighted sum) | `w_rel·rel + w_rec·rec + w_imp·imp` | YES (if cosine NaN) | NO | Propagates NaN from cosine |
| `salience` (stored f32) | DB read → `f64` | Only if DB corrupt | NO | Practically unreachable |

**Cosine NaN path:** `memory-rs/src/retrieval/cosine.rs` — if embedding is
all-zeros, `norm = 0.0`, `dot/norm = 0.0/0.0 = NaN`. Python's
`numpy.dot / (norm_a * norm_b)` behaves identically (both emit NaN). However
`score_memory` passes `None` when embedding is absent, so the NaN path is
`Some(all-zeros-vec)` — a zero-vector embedding. The embedding client
normalises vectors on response (memory-rs/src/embeddings.rs line ~133: "JSON
numbers are f64; collect as f32"). A legitimate embedding from the model will
never be all-zeros; only a bug-injected or corrupt stored embedding would be.

**Verdict:** Non-finite reachability for `score` is **theoretically possible**
(zero-norm embedding produces NaN) but **practically unreachable** under normal
operation. `serde_json` serialises NaN as `null` (not Python's bare `NaN`),
which would silently corrupt the score field if it ever appeared. **Mark as GAP
— a guard (`is_finite` check, map NaN→0.0 or return error) should be locked
by a test even if the path is unlikely.**

| Site (file:line) | Class | Python contract | Current Rust behaviour | Status |
|---|---|---|---|---|
| `memory-rs/src/retrieval/recall.rs:46` — `score_memory` return value | 2 | Python formula with identical NaN-on-zero-norm behaviour; Python result would also be NaN. No special handling in Python. | Returns NaN if `cosine` NaN (propagated to caller). serde_json serialises NaN → `null` on wire. | **GAP** — no test pins non-finite output; serde_json's `null` vs Python's `NaN`/`null` distinction is immaterial since Python also returned NaN (both are broken). A guard test `score.is_finite()` on typical inputs + a doc comment is warranted. |
| `memory-rs/src/retrieval/cosine.rs` — cosine divide-by-zero | 2 | `numpy` returns NaN on zero-norm; Python never guarded it | No guard; returns NaN if denominator zero | **GAP** — same reasoning; add a zero-norm guard returning 0.0 with a test. |
| `brain-rs/src/decide.rs:636-639` — `fmt_optional_f64(personality.pvp_appetite)` etc. | 2 | Python f-string: `f"pvp_appetite={personality.pvp_appetite}"` emits `"pvp_appetite=0.4"` for `Some(0.4)` and `"pvp_appetite=None"` for `None` | `fmt_optional_f64` matches: `Some(f) → format!("{f}")`, `None → "None"` | **GAP** — the `Some(0.4)` case is tested (`pvp_appetite=0.4` asserted) but the `None` case is **not** tested. A test pinning `"pvp_appetite=None"` in the prompt when v2 fields are absent is missing. |
| `memory-rs/src/sse_format.rs:138-139` — `salience` f32→f64 round-trip | 2 | Python memory sidecar stored salience as float; no NaN guard | f32 stored in DB, read as f64; `serde_json` serialises finite f64 correctly | **N/A** — stored values are clamped to `[0,1]` on write; non-finite unreachable. |

---

## Class 3 — `Option`/`None`/`#[serde(default)]`/`skip_serializing_if` wire-shape

Python pydantic models used `exclude_none=True` in specific contexts (e.g.
`personality.py:109 json.dumps(card.model_dump(by_alias=True))`) and `Field(None)`
defaults. Rust uses `#[serde(default)]` for deserialization defaults and
currently does **not** use `skip_serializing_if` on any of the surveyed DTOs.

| Site (file:line) | Class | Python contract | Current Rust behaviour | Status |
|---|---|---|---|---|
| `brain-rs/src/models.rs:50-59` — `PersonalityCard` v2 fields (`pvp_appetite`, `raid_appetite`, `completionist_streak`, `gold_motivation`, `profession_appetite`) | 3 | Python `model_dump(by_alias=True)` **does not** use `exclude_none` here (personality.py:109) — emits `null` for unset v2 fields. `PersonalityCard.model_dump()` includes `"pvp_appetite": null` by default. | Rust serialises `Option<f64>` fields as `null` when `None` (no `skip_serializing_if`). Matches Python's default `model_dump()` behaviour. | **LOCKED** — `test_personality_card_v2_fields_default_to_none` confirms deserialization; serialization to `null` matches Python. Round-trip test present. |
| `brain-rs/src/models.rs:12-19` — `Decision` (`tool`, `args`, `wakeup_in_ms`) | 3 | Python `Decision.model_dump()` emits `null` for `None` fields unless `exclude_none=True`. `decide.py:435` uses bare `d.model_dump()` (no exclude_none) when truncating recent decisions for the prompt context. | Rust serialises all three as `null` when `None`. | **LOCKED** — `test_no_op_decision_round_trips_json` covers round-trip; no `skip_serializing_if` divergence. |
| `tot-schema-transform/src/lib.rs` — `strip_top_level_nulls` | 3 | Python `mcp_server.py:119` `args.model_dump(exclude_none=True)` — removes `None` fields at top level only, not nested | `strip_top_level_nulls` removes `Value::Null` at top level only; does NOT recurse | **GAP** — existing 10 tests verify the top-level strip and non-recursion, but no test pins the Python oracle: specifically that a pydantic model's `None` field maps to a missing key (not `null`) in the outgoing tool-call args. The test asserts Rust-internal behaviour, not the Python `exclude_none` contract. Mark **partially covered** — the structural behaviour is correct, but no test uses a real Python-derived fixture to assert the parity value. |
| `memory-rs/src/mcp/schemas.rs:45-308` — MCP request schemas with `#[serde(default)]` | 3 | Python pydantic `Field(None)` / `Field(default=...)` → absent fields default during request parsing | `#[serde(default)]` provides identical default-on-absent semantics | **N/A** — deserialization defaults are structural, not parity-relevant on the wire (Python and Rust both accept missing fields and default them identically). |
| `memory-rs/src/mcp/handler.rs:87-89` — `ok_error` response shape `{"ok": false, "error": msg}` | 3 | Python `{"ok": False, "error": msg}` — no `detail` key on internal errors | Rust `{"ok": false, "error": msg}` — matches; no `detail` on this path | **LOCKED** — `memory-rs/src/mcp/mod.rs` tests check the error shape. |

---

## Class 4 — Error envelope / status mapping

| Site (file:line) | Class | Python contract | Current Rust behaviour | Status |
|---|---|---|---|---|
| `tot-harness-client/src/lib.rs:57-84` — `HarnessClient::call` | 4 | Python `memory_client.py:recall` called `response.raise_for_status()` BEFORE parsing body — a 4xx would raise `httpx.HTTPStatusError` and discard the `detail` field | Rust reads body first; `ok:false` or non-2xx maps to `HarnessError::Tool{tool,message}` preserving `detail`. This is the Phase-25 bug fix. | **LOCKED** — `non_2xx_envelope_preserves_detail` test pins this behaviour. The PYTHON behaviour (detail loss) was a BUG — Rust intentionally improves on it. |
| `brain-rs/src/memory_client.rs:87-130` — `MemoryClient::recall` / `write_episode` | 4 | Python raised `httpx.HTTPStatusError` on 4xx — detail lost. Python `result = body.get("result", body)` fallback: if `result` key missing, uses whole body | Rust uses `HarnessClient::call` which preserves `detail` on 4xx. Falls through to `HarnessError::Shape` on missing `result`. | **LOCKED** — `test_recall_ok_false_returns_err` + `test_recall_returns_episodes` cover happy path + error path. The Python's `body.get("result", body)` fallback is NOT present in Rust (it uses strict `missing result` error instead) — this is a **deliberate improvement**. |
| `brain-rs/src/sse_consumer.rs:247` — `error_for_status_ref()` | 4 | Python `sse_consumer.py::_stream_once` calls `response.raise_for_status()` on the stream HTTP response — raises `httpx.HTTPStatusError` on 4xx/5xx with status code only; **no response body is read** (streaming response, body is the event stream). | Rust `error_for_status_ref()` on the reqwest response before consuming the byte stream. Same semantics: HTTP status code only, no body. Maps to `anyhow::anyhow!("SseConsumer: HTTP status: {e}")` with the reqwest error message. | **N/A** — the response body IS the SSE event stream; reading the body as an error detail is not applicable. The error is a connection-phase error (non-2xx status), not a tool envelope error. Status-only is correct. |
| `brain-rs/src/llm_client.rs:73` — `error_for_status()` | 4 | Python `llm_client.py` calls `r.raise_for_status()` after the optional 400-fallback retry. Raises `httpx.HTTPStatusError` — HTTP status in message, no body surfaced. | Rust `error_for_status()` on reqwest response — same semantics: raises `reqwest::Error` carrying HTTP status; JSON body is not read before the status check. Error propagates as `reqwest::Error` to caller. | **N/A** — Python and Rust both surface only the HTTP status code for LLM errors; no body/detail needs to survive. The LLM API at `/v1/chat/completions` does not return a `{ok,result}` envelope. |
| `memory-rs/src/error.rs:47-64` — `AppError::into_response` | 4 | Python FastAPI `HTTPException(status_code=404, detail="no persona for this bot")` → `{"detail": "..."}` JSON body | Rust `AppError::NotFound(msg) → (404, Json({"detail": msg}))`. Same shape. | **LOCKED** — 5 unit tests (not_found_404, bad_request_400, embedding_503, db_500, internal_500) each assert `status + body["detail"]`. |
| `harness-rs/src/rest.rs` — `{ok,result}` response wrapping | 4 | Python FastMCP wraps tool results in `{"ok": True, "result": {...}}` | Rust wraps tool results identically | **LOCKED** — harness-rs integration test suite (186 tests) covers the dispatch + envelope shape. |
| `brain-rs/src/app.rs:391` — `resp.status() != StatusCode::OK` check | 4 | Python N/A (brain app.py is a different surface) | Internal consistency check (not a parity site) | **N/A** |

---

## Summary counts

| Status | Count |
|---|---|
| LOCKED | 11 |
| GAP | 4 |
| DECISION | 0 |
| N/A | 6 |
| **Total sites** | **21** |

---

## Open DECISION rows requiring operator input

**None.** All ambiguous sites were resolved as either GAP or N/A during analysis:

- The non-finite float divergence (Python NaN → Rust `null`) is a **GAP** (not a DECISION) because Python also produced NaN in that path — the serde_json `null` serialisation is actually more client-friendly, but the policy should be explicit (add `is_finite()` guard + test).
- The SSE and LLM `error_for_status` sites are **N/A** because both Python and Rust correctly surface only the HTTP status code (the response body is either an event stream or a JSON structure parsed separately).

---

## GAP rows — action list for Task 2

| Priority | GAP site | Required lock test |
|---|---|---|
| P1 | `memory-rs/src/retrieval/cosine.rs` — zero-norm guard | Unit test: `cosine(zero_vec, any_vec)` returns `0.0`, not NaN. Add a guard + test. |
| P1 | `memory-rs/src/retrieval/recall.rs:46` — `score_memory` non-finite | Unit test: score with zero embedding returns finite value (no NaN propagation). Depends on cosine guard above. |
| P2 | `brain-rs/src/decide.rs:636-639` — `fmt_optional_f64` None path | Unit test: `fmt_optional_f64(None)` → `"None"` (Python oracle: `f"pvp_appetite={None}"` → `"pvp_appetite=None"`). Add a direct `fmt_optional_f64` unit test. |
| P3 | `tot-schema-transform/src/lib.rs` — `strip_top_level_nulls` Python-oracle test | Add a test driven from a real Python `model_dump(exclude_none=True)` fixture: an object with a `None` field must produce an output where that key is absent (not present as `null`). Existing tests already verify this structurally; the new test should explicitly cite the Python oracle. |

---

## Architectural note: v0.2 vs v0.3 scoring formulas

The Python `tot/memory/src/tot_memory/routes/recall.py` implemented a **v0.3
hybrid formula** (BM25 + dense + decay + salience + entity-match). This was
the `POST /v1/memory/{bot_guid}/recall` endpoint for an episode-store schema.
The Rust `memory-rs` implements a **v0.2 weighted-sum formula**
(`w_rel·cosine + w_rec·exp(−age/τ) + w_imp·salience`) for a memory-store
schema (`POST /memory/recall`). These are **different API revisions on
different schemas** — the v0.3 Python route was never ported; it was retired
with the Python source. There is no parity gap to close on the v0.3 surface.
