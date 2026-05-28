# Threads of Time — Subset Gating Complete

This file is a one-time record of the state at the `subset-gating-complete` tag.

## What works

- `obs.list_players` + `obs.list_bot_population` harness tools — 121 parity tests passing
- `SubsetGate` asyncio.Task in brain recomputes living set every 60s
- Proximity-to-player selection with sticky party/raid/PvP overrides
- Hysteresis (out=2 ticks, in=1 tick) prevents thrash
- Operator pin/unpin via `/admin/subset/pin/{guid}` + `/admin/subset/unpin/{guid}`
- `/admin/subset/snapshot` + `/admin/subset/recompute` for debug + ops
- Three-tier model fully enabled: FULL / REDUCED / BACKGROUND
- `ToolPolicyEnforcer` restricts REDUCED tier to {bot.set_strategy, bot.set_goal, bot.set_role}
- REDUCED-tier ticks at 300s; FULL-tier ticks at 60s
- Phase A + Phase B eval gates pass (recompute p95 << 100ms)
- 337 brain tests passing (282 memory + integration + new unit + 2 eval gates)
- 121 harness parity tests passing (115 baseline + 6 new from 2 new obs tools)

## What's deferred (Plan 5 — operator install stack)

- Live MCP probe of obs.list_players + obs.list_bot_population against a real worldserver
- Heimdal end-to-end test with multiple real players
- Multi-player real-time proximity validation
- mod-playerbots virtual-GUID stability check (open Q from Plan 2 C2)
- Reference Compose/Quadlet stack consuming the new env vars

## What's deferred to 1.1.0+

- Player-facing chat command for nominating companion bots (operator-only pin in 1.0.0)
- Multi-region affinity (memory-weighted scoring of "regular party" bots)
- Dynamic TOT_LIVING_BOT_COUNT (varies by player count)
- REDUCED-tier auto-restart on goal completion

## Known issues + follow-up work

1. **C++ adapters were grep-verified but never live-MCP-probed.** The preflight notes at `tot/internal-docs/plan-3-preflight-notes.md` documented every accessor used (sRandomPlayerbotMgr.GetAllBots, sWorldSessionMgr->GetAllSessions, etc.), but a runtime probe on Heimdal is deferred to Plan 5. See kb_d7a2ef30 + kb_6950a902 item 10.
2. **Phase A → Phase B is a single code-level flag.** Operators can disable Phase B by setting `phase_b_enabled=False` in `app.py` (no env var; deliberate per spec §7). If we later need an operator-facing kill switch, add `TOT_PHASE_B_ENABLED` env var (5-minute change).
3. **7 pre-existing harness test failures** in test_mcp_auth.py / test_mcp_parity.py / test_mcp_server.py are unrelated to Plan 3 (missing `asyncio_mode = "auto"` in tot/harness/pyproject.toml). Pre-date Phase A tagging; tracked as a separate cleanup task.
4. **One `_reconcile` bug** was discovered and fixed during T30: tier assignment silently skipped bots absent from snapshot. Now correctly assigns REDUCED tier in empty-world cases. Caught by writing the Phase B integration test before flipping the flag.
5. **MemoryStore tests** were not affected by Plan 3 (no schema changes touching memory subsystem).

## What's pinned

- Tag: `subset-gating-complete`
- All hyperparameters at spec §7 defaults (5/15/60/2/1/300 living-bot-count + recompute + hysteresis; 300 for REDUCED tick)
- Per-bot DB columns added: tier, in_range_ticks, out_of_range_ticks, last_recompute_at, pinned
- Brain migration: 0004_subset_tier.sql

## Next plans

Plan 4 (MPQ compositor) and Plan 5 (operator install stack) can now proceed in parallel.
Plan 5 picks up the deferred Heimdal e2e integration (Plan 2 Task 38 + Plan 3 §9.3).
