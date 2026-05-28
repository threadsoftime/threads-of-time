# Threads of Time — V3 Memory Subsystem Complete

This file is a one-time record of the state at the `memory-subsystem-complete` tag (2026-05-27).

## What works

- `tot/memory/` FastAPI sidecar with full retrieval engine: 282 unit tests passing
- Per-bot SQLite + sqlite-vec storage at `data/memory/<bot_guid>/memory.sqlite`
- 4 schema migrations idempotent + version-tracked (episodes, entities + episode_entities, embeddings_vec [768-dim], episodes_fts)
- BYOLLM embeddings client (httpx; verifies returned dim matches EMBEDDING_DIM)
- Sync write path with graceful degradation (NULL embedding + future backfill if endpoint down)
- Full hybrid retrieval engine: BM25 + dense + entity-filter + time-decay + salience scorer + rerank orchestrator. Subspec §6 formula: `(α·bm25 + β·dense) · decay · (1 + γ·salience) + δ·entity_match` with defaults α=0.45, β=0.45, γ=0.20, δ=0.15
- Per-episode-type exponential decay (chat 48h, quest 7d, reflection 14d, discovery 30d)
- 7 HTTP routes: write/read/recall/search/list/update/delete
- 7 mod-harness-bridge C++ adapters at `modules/mod-harness-bridge/src/Adapters/Memory*Adapter.{h,cpp}`
- FastMCP daemon registration: 7 memory.* tools with subject_guid_arg=bot_guid; 115 parity tests passing per kb_642162c3
- Brain integration: memory_client.py + decision-loop wrap (recall-before-think + write-after-notable) + salience scorer per subspec §8
- Eval gate PASSES with stub embeddings: recall@5=0.940 (threshold 0.80), precision@5=0.940 (threshold 0.70), p95=0.67ms (threshold 100ms)

## What's deferred (Task 38)

**End-to-end integration on Heimdal** is deferred to Plan 5 (Operator install stack). Plan 5 builds the reference Compose/Quadlet stack + install-tot.sh script that makes the multi-service deploy automatable. Until then, E2E test would require manual one-off podman invocations on Heimdal.

The deferral is acceptable because:
- 282 unit tests cover each component in isolation
- Eval harness verifies hybrid-retrieval correctness end-to-end at the algorithm level
- mod-harness-bridge C++ adapters follow the existing adapter pattern (proven in Plan 1 Task 30 build)
- Pydantic-to-C++ parity test (kb_642162c3) verifies tool surface alignment

When Plan 5 lands, the integration test should:
1. Start tot/memory + tot/harness + tot/brain + worldserver containers via the reference compose stack
2. Drive a bot through several ticks
3. Assert `memory.list` returns at least one episode after N ticks
4. Assert the brain's prompt context includes a recalled episode after N+1 ticks
5. Restart the stack and verify memories persist across restarts (validates open question C2 — playerbots virtual GUID stability)

## What's deferred to 1.1.0+

- Multi-bot shared memory (Plan 2 scope was per-bot only)
- Async write queue (sync path is fine at expected scale; revisit only if write throughput becomes a bottleneck)
- Eviction policy beyond `memory vacuum` CLI (sufficient at expected episode growth rate)
- Brain prompt-engineering for *using* recalls effectively (Plan 2 is wire-up only; prompt tuning is a separate track per Plan 2 header)
- The full server-side rule-based salience scorer (subspec §8 implemented brain-side; server-side scorer is a follow-up)

## Open questions still pending verification

- **C2 (bot_guid stability):** mod-playerbots virtual player GUIDs across server restarts — needs Heimdal integration testing to confirm one bot's memories don't fragment across multiple GUIDs over time. If unstable, switch to per-character-NAME directories OR use mod-playerbots's persistent ID.
- **C4 (reflection trigger):** When does the brain generate a `reflection` episode? Cron? Threshold? On-demand? Deferred to brain-side design.
- **C5 (combat metadata.outcome vocabulary):** Brain needs to emit `kill|wipe|near_death` for salience scorer to use. Brain-side wiring task.

## Known issues + follow-up work

1. **Brain `pyproject.toml` declares AGPL-3.0** but new source files (memory_client.py, salience.py) use GPL-2.0-or-later SPDX per ToT's project license. Either bring brain to GPL-2.0-or-later or bring new files to AGPL-3.0; one-line fix in pyproject.toml. **License of record (per FOUNDATION_STATE.md + spec §10.1) is GPL-2.0-or-later.**
2. **`bot_guid` typing drift:** Plan 2's MemoryClient signature uses `str`, subspec §10 schemas say `int`. Phase 6 C++ adapters need a parity check at Heimdal integration time.
3. **`triage.hot_inputs["episode_type"]` not currently set by brain triage code** — every organic tick falls back to `"observation"` which the salience scorer gates out. Bots won't auto-write episodes until triage starts emitting typed signals. Brain-side wiring task (not Plan 2 scope).
4. **Brain env var name mismatch** (carried over from FOUNDATION_STATE.md item 2): brain code uses `LLM_BASE_URL`, spec uses `BRAIN_LLM_URL`. Not yet reconciled.

## What's pinned

- Memory subsystem tag: `memory-subsystem-complete`
- All hyperparameters at subspec §6.2 defaults (no tuning required by eval gate)
- Embedding dimension: 768 (`EMBEDDING_DIM` constant in `db/schema.py`)
- Recommended embedding model: `nomic-embed-text`
- Per-bot DB path: `data/memory/<bot_guid>/memory.sqlite`

## Next plans

Per spec §11.2 wave plan, Plans 3-6 can now run in parallel:
- **Plan 3:** Subset gating (selection algorithm, rotation cadence, brain-side opt-in vector)
- **Plan 4:** MPQ compositor + tier-set UI final + branding MPQ
- **Plan 5:** Operator install stack (Compose/Quadlet, install-tot.sh, first-boot bootstrap, operator + player docs) — also picks up Task 38 deferred integration
- **Plan 6:** Release pipeline + GHCR + CI + nightly AC drift

Plan 5 has dependencies on Plan 2 (the memory subsystem is one of the services it deploys), so it's the natural sequencing successor.
