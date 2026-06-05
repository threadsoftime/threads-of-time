# Python→Rust Parity Coverage Map

**Branch:** `feat/tot-parity-hardening`
**Baseline test count:** 944 passed, 0 failed (all test binaries green)
**Produced by:** Task 1 enumeration — coverage map only, no test code.

---

## Sweep outcome — 2026-06-04

Proactive parity-hardening sweep of the three faithful Python→Rust ports
(brain-rs, memory-rs, harness-rs). **Final gate: 950 passed, 0 failed, 4 ignored**
(+6 net-new parity tests over the 944 baseline; the 4 ignored are pre-existing,
none in the new test files).

**Headline finding (1 real, reachable divergence — found proactively, fixed):**
`brain-rs/src/decide.rs::format_f64_python` rendered whole-number trait floats
without the trailing `.0` (Rust Display `1.0`→`"1"`) while the Python original's
f-string emits `"1.0"`. Reachable on the **live `decide_v1` prompt path** whenever
a personality trait sits exactly at its `0.0`/`1.0` cap (schema-allowed, stored
verbatim). Resolved as a **faithful-port fix** (operator-approved): emit `.0` for
whole numbers; a sibling site (`assemble_prompt` persona-line traits) had the same
bug and was fixed in the same pass. Parity now locked by test.

**Everything else was already in parity** — the enumeration confirmed the
audit-SHA canonicalization (the Phase-24 fix), the `cosine` zero-norm guard,
the `{ok,result}`/`detail` error envelopes, and the `exclude_none`/`null`
wire-shapes are all already test-locked (or structurally N/A). The originally
feared NaN/Inf-on-the-wire class is guarded at `cosine.rs:36` and now has an
end-to-end finiteness lock. The Python v0.3 hybrid scoring route was never
ported (retired with the Python source) — no parity surface there.

**Process note:** one enumeration false-positive (cosine "missing guard") was
caught in review and corrected — the guard existed and was already tested.

**Commit series on `feat/tot-parity-hardening`:**
`39f11621e` enumerate map · `fad927aae` correct false cosine GAP ·
`4f6d3eddc` score_memory finiteness lock · `d9d3ea3e1` format/optional-f64
characterization · `5273bebe2` schema-transform pydantic oracle ·
`618dd3eb6` fix format_f64_python whole-number parity + sibling site.

**No live redeploy** — source/test only; runtime images unaffected.

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
| `memory-rs/src/retrieval/recall.rs:46` — `score_memory` return value | 2 | Python formula with identical NaN-on-zero-norm behaviour; Python result would also be NaN. No special handling in Python. | `cosine()` guards zero-norm → 0.0 (line 36 of cosine.rs); therefore `score_memory` cannot produce NaN via this path. | **LOCKED** — `score_memory_zero_norm_embedding_stays_finite` test asserts `s.is_finite()` and the exact value (0.35) for a zero-norm input embedding. The guard exists in `cosine.rs:36`: `if na == 0.0 \|\| nb == 0.0 { 0.0 }`. |
| `memory-rs/src/retrieval/cosine.rs` — cosine divide-by-zero | 2 | `numpy` returns NaN on zero-norm; Python `recall.py::cosine` guards: `if na == 0.0 or nb == 0.0: return 0.0` | Guard EXISTS at line 36: `if na == 0.0 \|\| nb == 0.0 { 0.0 }`. Returns 0.0, not NaN. | **LOCKED** — tests `zero_vector_a_returns_zero`, `zero_vector_b_returns_zero`, `both_zero_returns_zero` already asserted this. Map correction: this was incorrectly listed as GAP; the guard mirrors Python exactly. |
| `brain-rs/src/decide.rs:636-639` — `fmt_optional_f64(personality.pvp_appetite)` etc.; also `decide.rs:612-615` — persona-line non-optional traits (talkativeness, courage, greed, attitude_to_master) | 2 | Python f-string: `f"pvp_appetite={personality.pvp_appetite}"` emits `"pvp_appetite=0.4"` for `Some(0.4)` and `"pvp_appetite=None"` for `None`; whole-number values emit trailing `.0` (e.g. `f"{0.0}"` → `"0.0"`, `f"{1.0}"` → `"1.0"`) | Faithful-port fix applied (2026-06-04): `format_f64_python` now emits trailing `.0` for whole-number f64 values via `format!("{f:.1}")`. Non-optional traits (talkativeness etc.) now routed through `format_f64_python` rather than bare Rust Display. Parity holds for all float paths in the prompt. | **LOCKED (fixed)** — `format_f64_python_whole_number_matches_python_fstring` asserts `1.0`→`"1.0"`, `0.0`→`"0.0"`, `-1.0`→`"-1.0"`, `1.5`→`"1.5"`. `fmt_optional_f64_some_delegates_to_format_f64_python` extended to cover `Some(1.0)` and `Some(0.0)`. Sibling sites (persona-line f64 traits) also fixed in same commit. |
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
| `tot-schema-transform/src/lib.rs` — `strip_top_level_nulls` | 3 | Python `mcp_server.py:119` `args.model_dump(exclude_none=True)` — removes `None` fields at top level only, not nested | `strip_top_level_nulls` removes `Value::Null` at top level only; does NOT recurse | **LOCKED** — `strip_top_level_nulls_pydantic_oracle` test added: input `{"a":1,"b":null}` → output `{"a":1}` (key "b" absent), anchored to `/tmp/parity-venv` pydantic-v2 oracle command (outputs `{"a":1}`). |
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
| LOCKED | 15 |
| GAP | 0 |
| DECISION | 0 |
| N/A | 6 |
| **Total sites** | **21** |

---

## Open DECISION rows requiring operator input

**None.** All ambiguous sites resolved. The previously-noted known divergence for
`format_f64_python` whole-number output was resolved as a faithful-port fix
(2026-06-04): `format_f64_python` now emits trailing `.0` for whole-number f64
values, matching Python f-string behaviour. Test renamed from
`format_f64_python_whole_number_diverges_from_python_fstring` to
`format_f64_python_whole_number_matches_python_fstring` and now asserts parity.
Sibling sites (persona-line non-optional traits) fixed in same commit.

---

## GAP rows — all closed

All four original GAP rows are now LOCKED. See commit history on `feat/tot-parity-hardening`.

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
