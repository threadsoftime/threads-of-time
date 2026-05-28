# Plan 3 In-Game Test — 2026-05-28

First live exercise of subset gating on Heimdal after the
`subset-gating-complete` deploy. Driven by character **Puun** (player_guid=2654,
level 25, on map 530 Outland).

## What was validated end-to-end

All proven against a live worldserver with 1000 bots in `obs.list_bot_population`:

| Check | Evidence |
|---|---|
| `obs.list_players` returns logged-in human players only (bots excluded) | Returned exactly `{"players": [{"name": "Puun", "player_guid": 2654, ...}]}` — none of the 1000 bots leaked into the player list |
| `obs.list_bot_population` returns all bots with map/position/party/master | 1000 rows, map distribution 553/377/70 across maps 1/0/530; bot 1003 reported `party_guid=1, master_guid=2654` after party join |
| Proximity ranking picks closest-N bots on player's map | After fix B1, recompute targeted exactly 10 bots on map 530 (Puun's map) |
| Tier assignment: same-map bot → FULL | All 10 proximity picks marked `to_full` |
| Tier assignment: enrolled bot on different map → REDUCED | After teleporting Casmina (1003) to map 0 Stormwind, recompute correctly placed her in `to_reduced` (off-map) |
| Sticky party override preserves enrollment regardless of distance | Bot 1003 retained in `target` even when teleported to a different map, because `bot.party_guid == player.party_guid` |
| Hysteresis correctly displaces stale enrollments | The 7 originally-enrolled bots (none on map 530) were correctly released after 2 cycles of out-of-range |
| Three-tier model in production | Snapshot showed FULL=[10 same-map], REDUCED=[1 sticky off-map], BACKGROUND=999 not enrolled |
| `/admin/subset/snapshot,recompute,pin,unpin` endpoints | All returned correct JSON; pin correctly 404s for bots not in `living_bots` |

## Real bugs surfaced

### B1 — Snapshot fetcher missed harness response unwrap (FIXED in `99b66b257`)

`app.py` `_snapshot_fetcher` read `players_raw.get("players")` at the top level
of the response, but the harness wraps tool responses as
`{"ok": True, "result": {"players": [...]}}` per kb_87a7eade. The snapshot was
silently empty → SubsetGate never saw any players → `to_full` was permanently
`[]`, even when Puun (map 530) and bot 1003 (map 530) were co-located.

Fix: extracted `parse_world_snapshot(players_raw, bots_raw)` as a module-level
helper that handles both wrapped and unwrapped shapes. 4 regression tests
added. Brain tests now at **341 pass** (was 337).

### B4 — `enroll_bot` doesn't persist new bots into `living_bots` (FIXED in `e1d2937f0`)

When SubsetGate's proximity pick contains bots that were **never previously
enrolled via POST /enroll**, the `enroll_fn = supervisor.enroll_bot()` call:

1. Spawns an asyncio task (via `start()`) — succeeds
2. Does **not** insert a row into `living_bots` — leaves DB in pre-state
3. Does **not** set status to `'active'` for previously-released bots

Consequence: the next tick fires for these orphan tasks, fails with
`json.decoder.JSONDecodeError` (no personality, no LLM context), logs "uncaught
tick exception", and the bot is invisible to `/admin/subset/snapshot`
(which lists `state_store.list_active()`).

The previous Plan 3 agent flagged this in its T20-T22 report:
> "enroll_bot on LoopSupervisor assumes the bot is already `active` in
> state_store. ... The operator must still use POST /enroll to add bots to
> the pool; SubsetGate just decides which of the pooled bots get active loops."

The spec (§1, §4.2) is ambiguous about whether SubsetGate is supposed to
**select from a pre-enrolled pool** (current implementation) or
**auto-enroll any bot from `TOT_BOT_POPULATION`** (what the in-game behavior
needs). The default Heimdal population of 1000 bots vs. 7 manually-enrolled
bots makes the current behavior much less useful than intended.

**Fix shipped (`e1d2937f0`):**

Three-part change:
- `StateStore.reactivate(bot_guid)`: atomic `status='active'` + reset hysteresis. Three new unit tests.
- `SubsetGate.enroll_fn` signature: `Callable[[int, Optional[BotSnapshot]], Awaitable[None]]`. `_apply` builds `bot_by_guid` lookup from the snapshot and passes the matching `BotSnapshot` per `to_enroll` bot. One new contract test.
- `app.py _enroll_via_api` handles three cases:
  - **active**: idempotent `supervisor.start()`
  - **released**: `state_store.reactivate()` + `start()`
  - **missing**: build default `PersonalityCard` from `BotSnapshot` → best-effort `obs.get_state` for race/class → best-effort `morph_personality` → `state_store.enroll()` → `personality_cache.seed()` → `start()`. Graceful degradation on every step (LLM/obs/cache failures log + proceed).

**Live verification:**

After deploying `localhost/brain-sidecar:v0.5-subset-b4-20260528-0956`:

1. Cycle 1 with Puun on Outland (map 530): 10 active bots, 9 brand-new auto-enrolled, all `tier=full`. Casmina (1003, sticky-party, teleported to Stormwind) → `tier=reduced`.
2. Cycle 1 after Puun teleported to Stormwind (map 0):
   - Casmina → `tier=full` (now on Puun's map; sticky-party preserved)
   - 9 new map-0 bots auto-enrolled → `tier=full`
   - 9 old map-530 bots → `tier=reduced` (warm-cache transit; not yet hysteresis-released)
3. Cycle 2: hysteresis releases the displaced map-530 bots; 2 new picks enroll as bots drift in/out of proximity. System self-stabilizes around the proximity ranking.

`morph_personality` warnings appear in the brain log ("All connection attempts failed; keeping seed values") because the thomas-pc LLM primary is offline. **The graceful-degradation path keeps enrollment working** — bots get default v1 personality fields and v2 fields stay None (will fill on next morph attempt when LLM recovers).

## Pre-existing issue (not Plan 3 regression)

### thomas-pc LLM primary unreachable

Tick exceptions like `json.decoder.JSONDecodeError: Expecting value: line 1
column 1` are not Plan 3 bugs — they trace to the slots-router reporting
`primary_reachable: false` for the thomas-pc Nemo --parallel 8 endpoint. The
deploy agent flagged this; secondary (heimdal Nemo --parallel 1) is the only
working LLM. Will resolve when thomas-pc comes back online.

## Live system state at end of test

- Puun on map 530, party_guid=1, currently still in party with Casmina (1003)
- Casmina teleported to Stormwind (map 0); will reappear in Puun's group UI as
  on a different map but in-party
- 6 other previously-enrolled bots (1004, 1014, 1024, 1026, 1940, 2173) in
  `living_bots` with `status='released'` — can be manually re-activated via
  `UPDATE living_bots SET status='active' WHERE bot_guid=X` until B4 lands
- Brain `:current` = `localhost/brain-sidecar:v0.5-subset-fix-20260528-0937`
  (the rebake including B1 fix)

## Recommended next steps

1. **~~Fix B4~~** ✓ SHIPPED in `e1d2937f0`.
2. **Restore the 6 originally-released bots** — no longer urgent; SubsetGate
   now auto-enrolls from population as needed. The 6 originals will resurface
   if a player moves into their proximity (or via operator pin).
3. **Test sticky raid override** — convert the Puun+Casmina party to a raid,
   verify the algorithm still matches on `raid_guid` instead of `party_guid`.
   Requires user action (`/raid` in-game).
4. **~~Test hysteresis stability~~** ✓ validated: cycle 2 after teleport showed
   2 enrolls + 1 release as bots drifted across the proximity boundary —
   expected churn, not thrash. No bot oscillated `enroll→release→enroll` within
   the observation window.
5. **LLM primary recovery** — when thomas-pc Nemo comes back online, the
   `morph_personality` warnings will stop and v2 personality fields will
   fill on next bot enrollment.
