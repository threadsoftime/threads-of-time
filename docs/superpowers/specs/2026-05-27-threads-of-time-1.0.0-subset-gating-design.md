# Threads of Time 1.0.0 — Subset Gating (Design Subspec)

**Status:** Design — approved for Plan 3 implementation
**Date:** 2026-05-27
**Owner:** agent-orchestration-architect (with handoffs to agentic-harness-engineer + game-design-architect)
**License:** GPL-2.0-or-later (matches AC + ToT project; per parent spec §10.1)

**Parent spec:** `docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md` §1.1, §6.3, §9.3
**Implementation plan:** `docs/superpowers/plans/2026-05-27-threads-of-time-1.0.0-subset-gating.md` (produced by `writing-plans` after this spec is approved)
**Hub kb:** `kb_677e753f` | **Scope kb:** `kb_e6d3bbb1` | **Project nav:** `kb_87a7eade`

**Predecessors:**
- `kb_e6d3bbb1` — scope decision: subset gating ships in 1.0.0; algorithm + rotation + brain-side opt-in still open as of 2026-05-27
- `kb_642162c3` — MCP transport architecture for new harness tools (`obs.list_players`, `obs.list_bot_population`)
- `kb_d7a2ef30` — live MCP probe discipline for new `obs.*` shape verification
- `kb_6950a902` — spec-writing pre-flight checklist (items 9 + 10 require contract tests + live-probe of new tool shapes before declaring the spec complete; deferred to Plan 5 alongside Heimdal integration)
- Plan 2 (`memory-subsystem-complete` at `0dfa2a670`) — supplies `living_bots` table, per-bot memory store, LoopSupervisor, MemoryClient

**Non-goals (out of scope for this subspec):**
- Memory subsystem changes (per-bot store already shipped in Plan 2)
- mod-agenticbots strategy implementations (background-bot behavior is whatever stock mod-agenticbots does)
- Multi-bot brain coordination (each bot still ticks independently)
- BFD-tuned bot strategies (deferred to 1.1.0+; see `tot/internal-docs/agenticbots-upstream-prs.md`)
- New LLM routing logic (BYOLLM single endpoint per parent spec §9.3)
- Player-facing UI for nominating bots (deferred to a possible 1.1.0 feature; see §11)

---

## 1. Headline contract

**One sentence:** Of up to `TOT_BOT_POPULATION` bots in the world, the brain animates a configurable subset (`TOT_LIVING_BOT_COUNT`, default 10, range 5–15), selected by proximity to logged-in players with sticky overrides for party/raid/PvP-grouped bots; bots outside the subset run stock mod-agenticbots strategies. Bots inside the subset run in one of two tiers — FULL when a player is on their map, REDUCED when no player is on their map — with tier governing tick rate and tool-surface access.

**Player-experience contract:** A "living" bot has both memory-driven chat (recalls past sessions, refers to shared history, in-character whispers) AND autonomous goal-directed behavior (picks quests, decides to grind, queues for dungeons). This is the "full alive envelope" — the contract is intentionally maximal because subset gating exists precisely to make that contract affordable.

---

## 2. Decisions locked (resolved 2026-05-27 brainstorm)

| Decision | Resolution |
|---|---|
| Selection algorithm | **Proximity-to-player.** Bot is eligible if on same map as ≥1 online player; ranked by Euclidean distance to nearest player; closest-N wins. |
| Sticky overrides | Bots in any online player's party, raid, or PvP combat are pinned as living regardless of distance. Sticky picks fill first; proximity fills remaining slots. |
| Recompute cadence | Periodic timer (`TOT_SUBSET_RECOMPUTE_INTERVAL_S`, default 60s) with hysteresis: out-of-range for 2 ticks before release; in-range for 1 tick to enroll. Sticky overrides bypass hysteresis. |
| Architectural seam | Selection runs **in the brain** as a new `SubsetGate` asyncio.Task. Fed by two new harness tools (`obs.list_players`, `obs.list_bot_population`). |
| Proximity unit | Same map (boolean filter) + 3D Euclidean distance (ranking). No yard cutoff; the map filter is the radius. |
| Offline-player behavior | Enrolled bots transition to REDUCED tier (not released). REDUCED tier ticks at 5min interval with restricted tool whitelist. Saves LLM tokens without losing continuity. |
| Tiers | Three: **FULL** (enrolled + player on map) / **REDUCED** (enrolled + no player on map) / **BACKGROUND** (not enrolled, stock strategies). |
| Sequencing | **Two-phase in Plan 3.** Phase A ships 2-tier (FULL/BACKGROUND) with proximity + hysteresis + sticky. Phase B adds REDUCED tier + tool policy enforcer. |

---

## 3. Architecture

```
                  ┌─────────────────────────────────────┐
                  │   Worldserver (mod-agenticbots)     │
                  │   up to TOT_BOT_POPULATION bots     │
                  └────────┬──────────────────┬─────────┘
                           │                  │
                  (calls bot.* tools)    (queries via obs.*)
                           │                  │
                  ┌────────▼──────────────────▼─────────┐
                  │  FastMCP harness daemon :8099       │
                  │  + NEW: obs.list_players            │
                  │  + NEW: obs.list_bot_population     │
                  └────────┬─────────────────────────────┘
                           │
                  ┌────────▼─────────────────────────────┐
                  │  Brain sidecar :8091                 │
                  │  ┌─────────────────────────────────┐ │
                  │  │  SubsetGate  (NEW, Phase A)     │ │
                  │  │  • 60s asyncio.Task             │ │
                  │  │  • polls obs.list_players +     │ │
                  │  │    obs.list_bot_population      │ │
                  │  │  • computes desired set         │ │
                  │  │  • calls internal enroll/release│ │
                  │  └────────┬────────────────────────┘ │
                  │           │                          │
                  │  ┌────────▼───────────────────────┐ │
                  │  │  LoopSupervisor (existing)     │ │
                  │  │  + tier-aware tick rate (B)    │ │
                  │  │  + ToolPolicyEnforcer (B)      │ │
                  │  └────────────────────────────────┘ │
                  └──────────────────────────────────────┘
```

**Surface added to the system:**
- 2 new C++ adapters in `modules/mod-harness-bridge/src/Adapters/`
- 2 new pydantic schemas in `tot/harness/src/.../tool_schemas.py`
- 2 new registry rows + parity-test rows (117 parity tests total, was 115)
- 1 new brain module `tot/brain/brain_sidecar/subset_gate.py`
- 1 new brain module `tot/brain/brain_sidecar/tool_policy.py` (Phase B)
- 1 brain schema migration `tot/brain/migrations/0004_subset_tier.sql`
- 4 new brain admin endpoints (`/admin/subset/...`)
- 5 new operator env vars in `.env.example` (parent spec §6.3 already declares `TOT_LIVING_BOT_COUNT`)
- 0 new sidecars
- 0 new tables (single ALTER TABLE on `living_bots`)

---

## 4. Components

### 4.1 New harness tools (Phase A)

Both tools follow the kb_642162c3 adapter pattern: C++ `Adapter` class → `HarnessBridgeDispatch` wiring → pydantic schema in `tool_schemas.py` → `registry.py` row → parity-test row.

**`obs.list_players`** — world-wide query, no `subject_guid_arg`.

```python
class ObsListPlayersResponse(BaseModel):
    players: list[PlayerWorldSnapshot]

class PlayerWorldSnapshot(BaseModel):
    player_guid: int
    name: str
    map_id: int
    x: float
    y: float
    z: float
    level: int
    party_guid: Optional[int] = None    # group GUID if in party
    raid_guid: Optional[int] = None     # raid GUID if in raid
    in_pvp_combat: bool = False
```

Source: iterates `sWorld->GetAllSessions()` filtered to logged-in `Player*`s; reads `GetMapId`, `GetPositionX/Y/Z`, `GetLevel`, `GetGroup()->GetGUID()` (party vs raid disambiguated by `Group::isRaidGroup()`), `IsInCombat() && IsPvP()`.

**`obs.list_bot_population`** — world-wide query, no `subject_guid_arg`.

```python
class ObsListBotPopulationResponse(BaseModel):
    bots: list[BotWorldSnapshot]

class BotWorldSnapshot(BaseModel):
    bot_guid: int
    name: str
    map_id: int
    x: float
    y: float
    z: float
    level: int
    party_guid: Optional[int] = None
    raid_guid: Optional[int] = None
    master_guid: Optional[int] = None   # player the bot is following, if any
    in_pvp_combat: bool = False
```

Source: iterates the mod-playerbots bot registry via the public `PlayerbotHolder::GetPlayerBots()` (or equivalent — final shape verified during implementation per kb_6950a902 item 1 API-drift check). Same field extraction as `obs.list_players` plus `master_guid` from `PlayerbotAI::GetMaster()`.

**Spec pre-flight note:** Per kb_6950a902 item 10 + kb_d7a2ef30, the exact response shape must be live-MCP-probed against a real worldserver before any code relies on it. That probe is deferred to Plan 5 (Heimdal integration). Phase A unit tests use fixture snapshots; final shape correctness lands when the integration test runs on Heimdal.

### 4.2 Brain module `subset_gate.py`

```python
@dataclass
class SubsetGateConfig:
    living_bot_count: int            # TOT_LIVING_BOT_COUNT (default 10, clamped to [5, 15])
    recompute_interval_s: float      # TOT_SUBSET_RECOMPUTE_INTERVAL_S (default 60)
    hysteresis_out_ticks: int        # TOT_SUBSET_HYSTERESIS_OUT_TICKS (default 2)
    hysteresis_in_ticks: int         # TOT_SUBSET_HYSTERESIS_IN_TICKS (default 1)
    enroll_backoff_s: float          # TOT_SUBSET_ENROLL_BACKOFF_S (default 300)
    enabled: bool                    # TOT_SUBSET_GATE_ENABLED (default true)

@dataclass(frozen=True)
class SubsetDecision:
    target_living_set: set[int]      # bot_guids that SHOULD be enrolled this cycle
    sticky_pinned: set[int]          # bot_guids whose inclusion was sticky-forced
    proximity_picks: set[int]        # bot_guids whose inclusion was proximity-ranked
    to_enroll: set[int]              # newly-living, hysteresis-cleared
    to_release: set[int]             # not-living, hysteresis-cleared (Phase A) or to_reduce (Phase B)
    to_full: set[int]                # currently-enrolled bots whose tier should be 'full'
    to_reduced: set[int]             # Phase B only — currently-enrolled bots whose tier should be 'reduced'
    skipped_due_to_backoff: set[int]

class SubsetGate:
    """Long-running asyncio.Task that periodically recomputes the living set."""

    async def run(self) -> None:
        """Boot the recompute loop. Cancelled at brain shutdown."""

    async def _recompute_once(
        self,
        snapshot: WorldSnapshot,
        currently_enrolled: dict[int, LivingBotRow],
        pins: set[int],
    ) -> SubsetDecision:
        """Pure function. Unit-test surface. No MCP calls. No state mutation."""

    async def _apply(self, decision: SubsetDecision) -> None:
        """Reconcile decision against state_store + LoopSupervisor.
        Calls internal enroll/release paths. Mutates living_bots tier column."""
```

The pure-function split is intentional. `_recompute_once` is the algorithm; `_apply` is the side-effecting reconciliation. Unit tests exhaustively cover `_recompute_once` with snapshot fixtures.

### 4.3 Brain module `tool_policy.py` (Phase B)

```python
TIER_TOOL_WHITELIST: dict[str, frozenset[str]] = {
    "full": frozenset({
        "bot.send_chat", "bot.set_strategy", "bot.set_goal", "bot.set_role",
        "bot.invite_to_group", "bot.accept_invite", "bot.leave_group",
        "bot.follow", "bot.queue_for_dungeon", "bot.enter_instance", "bot.stop",
        "bot.combat_stop",  # if shipped per kb_9461bc30 V1.5+ extensions
    }),
    "reduced": frozenset({
        "bot.set_strategy", "bot.set_goal", "bot.set_role",
    }),
}

class ToolPolicyEnforcer:
    def is_allowed(self, tool_name: str, tier: str) -> bool:
        return tool_name in TIER_TOOL_WHITELIST.get(tier, TIER_TOOL_WHITELIST["full"])
```

`Dispatcher.dispatch()` consults the enforcer before invoking the harness tool. Denials are logged as a `policy_denied` decision-log record (not raised as exceptions). The denied decision becomes a no-op tick.

**Whitelist rationale (REDUCED tier):**
- `bot.set_strategy` + `bot.set_goal` + `bot.set_role` — the bot picks its high-level intent; stock mod-agenticbots executes it
- No `bot.send_chat` — no player on the map to chat with
- No group manipulation — the bot doesn't drag other bots into social activities while the player is offline
- No `bot.queue_for_dungeon` — multi-bot dungeon coordination is FULL-tier territory; REDUCED bots stick to solo activities (questing, crafting, AH visits) executed by stock strategies

### 4.4 Brain schema migration `0004_subset_tier.sql`

```sql
-- Plan 3 subset-gating tier tracking
ALTER TABLE living_bots ADD COLUMN tier TEXT NOT NULL DEFAULT 'full';
ALTER TABLE living_bots ADD COLUMN out_of_range_ticks INTEGER NOT NULL DEFAULT 0;
ALTER TABLE living_bots ADD COLUMN in_range_ticks INTEGER NOT NULL DEFAULT 0;
ALTER TABLE living_bots ADD COLUMN last_recompute_at INTEGER;
ALTER TABLE living_bots ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;  -- 0|1: operator pin
CREATE INDEX IF NOT EXISTS idx_living_bots_tier ON living_bots(status, tier);
INSERT INTO schema_version(version, applied_at) VALUES (4, strftime('%s','now'));
```

Migration is idempotent per the existing `StateStore.migrate()` version-tracking pattern (see `tot/brain/brain_sidecar/state.py:43-75`).

### 4.5 New `StateStore` methods

```python
def set_tier(self, bot_guid: int, tier: str) -> None: ...
def get_tier(self, bot_guid: int) -> str: ...                  # default 'full' if column NULL
def bump_hysteresis(self, bot_guid: int, in_range: bool) -> tuple[int, int]: ...
def get_hysteresis(self, bot_guid: int) -> tuple[int, int]: ... # returns (in_range_ticks, out_of_range_ticks)
def set_pin(self, bot_guid: int, pinned: bool) -> None: ...
def list_pinned(self) -> list[int]: ...
```

### 4.6 Brain admin endpoints

```
GET  /admin/subset/snapshot       → {currently_enrolled, last_decision, pinned, config}
POST /admin/subset/recompute      → forces a recompute (returns SubsetDecision payload)
POST /admin/subset/pin/{guid}     → operator override; persists in living_bots.pinned=1
POST /admin/subset/unpin/{guid}   → clears pin; bot re-evaluated next cycle
```

Endpoints are not bearer-token-protected by 1.0.0 (the brain listens on loopback in the reference deploy stack). Pin/unpin persistence ensures pins survive brain restarts.

### 4.7 LoopSupervisor modifications

Two surgical changes:

**Phase A:** at the start of each per-bot tick, read `state_store.get_tier(bot_guid)` and store it in `TickState`. Pass to `Dispatcher.dispatch()` as context.

**Phase B:** `Dispatcher.dispatch()` consults `ToolPolicyEnforcer.is_allowed(tool_name, tier)` before invoking the tool. Tick interval is selected per-tier:
```python
tick_interval = (
    config.reduced_tick_interval_s if tier == 'reduced'
    else config.full_tick_interval_s  # existing tick_interval_s
)
```

The existing `_tasks: dict[int, asyncio.Task]` per-bot loop keeps its identity across tier transitions — no task restart on transition, just a different sleep duration.

---

## 5. Selection algorithm (per recompute cycle)

```
T+0s   SubsetGate.run() fires (TOT_SUBSET_RECOMPUTE_INTERVAL_S timer)
       │
       ├─► obs.list_players()         ──► [Alice@map1(123,456,0), Bob@map3(...)]
       └─► obs.list_bot_population()  ──► up to TOT_BOT_POPULATION rows

T+~50ms  _recompute_once(snapshot, currently_enrolled, pins):

    STEP 1 — Sticky overrides
        sticky_set = pins ∪ {
            bot for bot in snapshot.bots if any(
                p.party_guid is not None and p.party_guid == bot.party_guid
                or p.raid_guid is not None and p.raid_guid == bot.raid_guid
                or (bot.in_pvp_combat and bot.map_id == p.map_id)
                for p in snapshot.players
            )
        }

    STEP 2 — Proximity ranking
        eligible = {bot for bot in snapshot.bots
                    if any(p.map_id == bot.map_id for p in snapshot.players)
                    and bot not in sticky_set
                    and bot not in skipped_due_to_backoff}
        def score(bot):
            same_map = [p for p in snapshot.players if p.map_id == bot.map_id]
            return min(euclidean(bot, p) for p in same_map) if same_map else inf
        slots_remaining = max(0, TOT_LIVING_BOT_COUNT - len(sticky_set))
        proximity_pick = sorted(eligible, key=score)[:slots_remaining]

    STEP 3 — Target living set + warm-cache extension (Phase B)
        target_proximity = sticky_set ∪ set(proximity_pick)

        # Phase A: target = target_proximity (no warm cache; bots without proximity drop out)
        # Phase B: warm-cache extends target with currently-enrolled bots until capacity
        if PHASE_B:
            remaining_slots = max(0, TOT_LIVING_BOT_COUNT - len(target_proximity))
            # Bots currently enrolled but not in target_proximity — order by recency
            warm = [bot_guid for bot_guid in currently_enrolled
                    if bot_guid not in target_proximity]
            warm.sort(key=lambda g: currently_enrolled[g].last_seen or 0, reverse=True)
            target = target_proximity ∪ set(warm[:remaining_slots])
        else:
            target = target_proximity

    STEP 4 — Hysteresis reconciliation
        # A bot leaves the target ONLY if displaced by a competing proximity claim.
        # In Phase A that includes "no players online" (target shrinks to sticky only).
        # In Phase B, warm-cache (Step 3) extends target so non-displaced bots stay.
        for bot_guid in currently_enrolled:
            if bot_guid in target or bot_guid in pins:
                state_store.bump_hysteresis(bot_guid, in_range=True)  # resets out_of_range_ticks
            else:
                ticks_in, ticks_out = state_store.bump_hysteresis(bot_guid, in_range=False)
                if ticks_out >= hysteresis_out_ticks and bot_guid not in sticky_set:
                    to_release.add(bot_guid)
        for bot_guid in target_proximity:  # warm-cache extensions never need enrolling
            if bot_guid in currently_enrolled:
                continue
            if bot_guid in sticky_set:
                to_enroll.add(bot_guid)  # sticky bypasses hysteresis
            else:
                ticks_in, _ = state_store.bump_hysteresis(bot_guid, in_range=True)
                if ticks_in >= hysteresis_in_ticks:
                    to_enroll.add(bot_guid)

    STEP 5 — Tier assignment (post-reconciliation, Phase B only)
        for bot_guid in (currently_enrolled - to_release) ∪ to_enroll:
            bot = snapshot.bots[bot_guid]
            if any(p.map_id == bot.map_id for p in snapshot.players):
                to_full.add(bot_guid)
            else:
                to_reduced.add(bot_guid)
        # Phase A skips Step 5; the `tier` column stays at default 'full' for all enrolled.

T+~100ms  _apply():
       • for bot in to_release: POST /release internally → tick task cancelled, status='released'
       • for bot in to_enroll:  POST /enroll internally → personality morph, tick task starts
       • for bot in to_full:    state_store.set_tier(bot, 'full')
       • for bot in to_reduced: state_store.set_tier(bot, 'reduced')  # Phase B
       • LoopSupervisor picks up the new tier on the bot's next per-bot tick (no restart)

T+~150ms  decisions.jsonl gains a record:
       {"kind":"subset_recompute","at":...,"target":[...],"enrolled":[...],
        "released":[...],"full":[...],"reduced":[...],
        "sticky_pinned":[...],"proximity_picks":[...]}
```

**Properties:**
- **Idempotent** — running with same snapshot + state yields same decision; hysteresis bumps are a deterministic state machine
- **Bounded blast radius** — bug in `_recompute_once` corrects within one recompute (60s default)
- **Cheap MCP** — two round trips per cycle; no per-bot calls
- **Sticky bypass** — a player adding a bot to their party causes immediate enrollment, not a 60s wait

---

## 6. Tier semantics

### 6.1 FULL tier (Phase A + B)
- Tick interval: 60s (existing `tick_interval_s`)
- Tool surface: 12 `bot.*` tools (whitelist `tier_tool_whitelist["full"]`)
- Memory recall + write: normal (existing Plan 2 wiring)
- Decision log: records tagged `tier:"full"`

### 6.2 REDUCED tier (Phase B only)
- Tick interval: `TOT_REDUCED_TICK_INTERVAL_S`, default 300 (5min)
- Tool surface (3 tools): `bot.set_strategy`, `bot.set_goal`, `bot.set_role`
- Memory recall: still fires (REDUCED bots still benefit from recalling past goals)
- Memory writes: still fires (REDUCED bots still build episodic memory of solo activities)
- Decision log: records tagged `tier:"reduced"`; deniedtool calls tagged `policy_denied:true`

### 6.3 BACKGROUND tier (always)
- Not in `living_bots` (or status='released')
- Brain ticks NOTHING for this bot
- Stock mod-agenticbots strategies drive behavior
- No memory recall or writes
- The vast majority of `TOT_BOT_POPULATION` is BACKGROUND on a typical operator deploy (10 living / 20 bots = 10 background; 10 / 100 = 90 background)

### 6.4 Tier transitions (derived per cycle, not state-machined)
- **FULL → REDUCED** (Phase B only): bot stays enrolled but no player remains on its map
- **REDUCED → FULL**: a player moves to / logs in on bot's map
- **FULL → BACKGROUND**: bot displaced by a competing proximity claim (a closer bot takes its slot via §5 step 3 ordering) AND `out_of_range_ticks >= 2` AND not in sticky set
- **REDUCED → BACKGROUND**: same displacement path — a returning player triggers a proximity recompute that ranks a different bot higher than the warm-cached REDUCED bot, and the REDUCED bot loses its warm-cache slot
- **BACKGROUND → FULL/REDUCED**: bot enters target set + hysteresis cleared (`in_range_ticks >= 1`); tier assigned per §5 step 5

**REDUCED is a steady state.** A bot with no players nearby and capacity available stays REDUCED indefinitely. It only loses its slot when displaced (capacity exhausted by stickys + proximity picks from returning players). The warm-cache extension (§5 step 3) makes this explicit — REDUCED bots ARE in the target set, they just aren't there because of proximity.

**Empty world** (no players online globally):
- **Phase A**: target_proximity is empty (no warm cache); currently-enrolled bots fall out of range; after 2 hysteresis cycles all are released. Brain goes idle.
- **Phase B**: target_proximity is empty but warm-cache extension makes target = currently_enrolled; nobody hits hysteresis; everyone transitions to REDUCED; ticks fire every 300s with restricted whitelist.

---

## 7. Operator config surface

Additions to `.env.example` (parent spec §6.3):

```bash
# === Subset gating (Plan 3) ===
TOT_LIVING_BOT_COUNT=10                 # already declared, default 10, range 5-15
TOT_SUBSET_RECOMPUTE_INTERVAL_S=60      # how often the subset gate fires
TOT_SUBSET_HYSTERESIS_OUT_TICKS=2       # cycles before release
TOT_SUBSET_HYSTERESIS_IN_TICKS=1        # cycles before enrollment
TOT_SUBSET_ENROLL_BACKOFF_S=300         # backoff after a failed enroll
TOT_SUBSET_GATE_ENABLED=true            # operator off-switch
TOT_REDUCED_TICK_INTERVAL_S=300         # Phase B only — REDUCED-tier tick interval
```

**Knobs deliberately NOT exposed:**
- Proximity radius — same-map is the implicit radius; ranking is by raw distance, so a yard cutoff is redundant
- Per-bot tier override in env — operator uses `/admin/subset/pin/{guid}` endpoint instead
- Per-tier tool whitelist — code-level constant; changing it is a code change with corresponding test changes

**Default tuning rationale:**
- `TOT_LIVING_BOT_COUNT=10` — matches parent spec §6.3 and historical Heimdal ceiling (10 bots × Mistral Nemo ~12B at expected concurrency is the empirical safe band per `kb_67ddebbe`)
- `TOT_SUBSET_RECOMPUTE_INTERVAL_S=60` — slow enough to avoid LLM-cost thrash from a player jogging past bots; fast enough that "I logged in and joined a party" feels responsive (sticky path is immediate anyway)
- Hysteresis `2 out / 1 in` — asymmetric: easier to become living than to lose it, biasing toward continuity of the player experience

---

## 8. Error handling

| Failure mode | Behavior |
|---|---|
| `obs.list_players` MCP timeout | SubsetGate skips this cycle; logs warning; retries next cycle. No stale-snapshot tier changes. |
| `obs.list_bot_population` returns bot with invalid map_id | Bot scored as `inf` → drops out of proximity ranking; sticky overrides still apply. |
| `/enroll` raises 409 (bot already enrolled) | Treated as no-op success. Idempotent. |
| `/release` raises 404 (bot not enrolled) | Treated as no-op success. Idempotent. |
| Brain restart mid-cycle | `living_bots` table persists with `tier` + hysteresis counters; next recompute resyncs. |
| `_recompute_once` raises | Cycle aborted, exception logged, living set unchanged; next cycle retries. |
| Personality morph fails inside enroll | Existing behavior: `status='released'`, error returned. SubsetGate records bot in `enroll_backoff` set for `TOT_SUBSET_ENROLL_BACKOFF_S` to avoid retry-storms. |
| Pinned bot can't be enrolled | Pin persists; SubsetGate retries each cycle subject to backoff. Operator sees stuck pin via `/admin/subset/snapshot`. |
| `obs.list_bot_population` returns more rows than `TOT_BOT_POPULATION` | Allowed — `TOT_BOT_POPULATION` is informational, not a hard cap on the algorithm. Algorithm picks top N regardless. |
| Two players on different maps, no overlap | Each player's sticky-grouped bots are pinned; proximity ranking partitions by player. Closest-N-globally rule means an Alice-grouped bot 10y from Alice beats a Bob-grouped bot 5y from Bob ONLY if the slot count is exhausted — but sticky overrides bypass slot competition, so both sticky sets are always preserved. |

---

## 9. Testing strategy

| Test layer | File | Coverage target |
|---|---|---|
| Unit tests | `tot/brain/tests/unit/test_subset_gate.py` | ≥20 tests covering `_recompute_once`: empty world, no players, single player single map, multi-player multi-map, sticky party override, sticky raid override, sticky PvP override, hysteresis in/out, pin overrides, N>population edge case, tied distances, backoff exclusion, sticky-bypass-hysteresis, FULL/REDUCED tier assignment |
| Unit tests | `tot/brain/tests/unit/test_subset_state.py` | ≥10 tests for new `StateStore` methods: migrate idempotent, set_tier round-trip, bump_hysteresis state machine, get_tier defaults, set_pin / list_pinned |
| Unit tests (Phase B) | `tot/brain/tests/unit/test_tool_policy.py` | ≥8 tests: each whitelisted tool passes in REDUCED, each denied tool blocks in REDUCED, all 12 pass in FULL, missing tier defaults to FULL |
| Integration tests | `tot/brain/tests/integration/test_subset_gate_loop.py` | ≥6 tests against in-memory mock MCP: cold start enrolls N, player moves between maps, player logs out (A: releases all; B: transitions to REDUCED), pin survives recompute, sticky overrides immediate, hysteresis prevents thrash |
| Parity tests (existing) | `tot/harness/tests/test_mcp_schemas.py` | +2 rows for new `obs.list_players` + `obs.list_bot_population` → 117 total (was 115) |
| Harness adapter unit tests | `modules/mod-harness-bridge/tests/` | 2 new tests (one per adapter), follow kb_642162c3 pattern |
| Regression gates | existing | 282 memory tests + 43 brain tests + 115→117 parity tests all green |

### 9.1 Phase A completion criterion (eval gate)
- Spin up brain with fixture world (3 players + 30 bots across 3 maps)
- Drive SubsetGate through 10 cycles with scripted player movement
- Assert: living-set count stays within [5, 15] every cycle
- Assert: sticky-grouped bots never released
- Assert: hysteresis prevents yo-yo (bot crossing boundary once doesn't oscillate)
- Assert: no `_recompute_once` call exceeds 100ms p95 in-process

### 9.2 Phase B completion criterion (eval gate)
- All Phase A eval steps still pass
- Additionally: with all players logged out, the dispatch log over a 10-cycle window contains zero `bot.send_chat` / `bot.invite_to_group` / `bot.queue_for_dungeon` calls; only `bot.set_strategy` / `bot.set_goal` / `bot.set_role` appear
- Assert: REDUCED-tier ticks fire on the slower interval (300s), verified by tick timestamps in decision log

### 9.3 Deferred to Plan 5 (Heimdal integration test) — same pattern as Plan 2 Task 38
- End-to-end on a real worldserver with real mod-playerbots
- Live MCP probe of `obs.list_players` + `obs.list_bot_population` shapes per kb_d7a2ef30 + kb_6950a902 item 10
- Multi-player real-time movement test
- Bot population stability across worldserver restarts (existing open question C2 from Plan 2)

---

## 10. Phase A vs Phase B boundary

**Phase A — tag `subset-gating-phase-a-complete`:**

Ships everything from §4.1, §4.2, §4.4, §4.5, §4.6, §4.7 (the Phase A modifications), §5 with `PHASE_B=false` (no warm cache, step 5 skipped), §7 (env vars except `TOT_REDUCED_TICK_INTERVAL_S`), §8 (full error handling), §9.1 eval gate.

Concretely: 2-tier model (FULL/BACKGROUND), strict proximity, hysteresis, sticky, pins, both new harness tools, SubsetGate module, schema migration with all columns including `tier` (default `'full'` for all enrolled), parity tests +2, unit tests, integration tests.

**Phase A "no players online" behavior:** target_proximity is empty; currently-enrolled bots fall out of range; after 2 hysteresis cycles (120s default) they all release. Brain idle. Bots become BACKGROUND and play stock strategies. This is intentional in Phase A — REDUCED tier and warm-cache semantics arrive in Phase B.

**Phase B — tag `subset-gating-complete`:**

Ships §4.3 (`tool_policy.py`), §6.2 REDUCED-tier semantics, §7 `TOT_REDUCED_TICK_INTERVAL_S`, §9.2 Phase B eval gate. Flips `PHASE_B=true` in §5, which:
- enables the warm-cache extension in step 3 (currently-enrolled bots fill remaining slots when proximity doesn't),
- enables step 5 (FULL vs REDUCED tier assignment based on player presence on map).

Phase B also lands the `ToolPolicyEnforcer` in `Dispatcher.dispatch()` and the tier-aware tick-interval branch in `LoopSupervisor`.

**Phase B "no players online" behavior:** target_proximity empty; warm-cache extension fills target with currently_enrolled; nobody falls out of hysteresis; everyone transitions to REDUCED tier; ticks fire every 300s with the restricted whitelist. The brain stays warm with continuity preserved.

The Phase A → Phase B diff is surgical: one new module (`tool_policy.py`), one `PHASE_B` branch in §5 step 3, one `PHASE_B` branch enabling step 5, one config knob, one tier-aware tick-interval branch in LoopSupervisor.

**Why two phases:**
- Phase A is verifiable on its own; an operator can ship 1.0.0-rc.1 with Phase A and add Phase B at 1.0.0-rc.2 if regression risk emerges
- Phase A's blast radius is smaller (proximity-gated enroll/release is the primary behavioral change; tool-surface restriction is independent)
- The two phases share infrastructure (`living_bots.tier` column, snapshot data flow) but have disjoint test surfaces

---

## 11. Open items + deferrals

### 11.1 Deferred to 1.1.0+ (post-1.0.0)
- **Player nomination UI** — chat command (`.companion bot Borg`) or similar to let players explicitly pin bots. 1.0.0 ships operator-only `/admin/subset/pin` endpoint; player-facing surface is 1.1.0+ work
- **Multi-region affinity** — bots that prefer specific players (build "Alice's regular party") via memory-weighted scoring
- **Dynamic `TOT_LIVING_BOT_COUNT`** — operator currently chooses a static N; future could vary by online-player count
- **REDUCED-tier auto-restart conditions** — e.g., bot completed a goal in REDUCED mode → bump back to FULL even without player return

### 11.2 Deferred to Plan 5 (operator install stack + integration)
- Live MCP probe of `obs.list_players` + `obs.list_bot_population` shapes against real worldserver
- End-to-end test with multiple real players moving in the world
- Verification that mod-playerbots virtual-GUID stability (Plan 2 open question C2) does not fragment the living set across server restarts
- Reference deploy-stack documentation for the new env vars in `.env.example`

### 11.3 Implementation-time questions for plan author
- Exact mod-playerbots accessor for "list all bots in world" — `PlayerbotHolder::GetPlayerBots()` is the candidate but the actual public surface needs grep verification per kb_6950a902 item 1
- Whether `Player::IsPvP() && IsInCombat()` is the right C++ predicate for "in_pvp_combat" or whether a different signal (battlefield flag, BG presence) is preferable
- Whether the brain should subscribe to memory-sidecar SSE for player-login events (V3.2 SSE infra) to short-circuit the 60s recompute on player login — possible follow-up; Phase A polls only

### 11.4 What "Plan 3 done" looks like
- Both phases tagged in the threads-of-time repo
- 282 memory tests + 43 brain tests + 117 parity tests + new subset-gate unit/integration tests all green
- Phase A and Phase B eval gates documented as passed in `SUBSET_GATING_STATE.md`
- `.env.example` carries the new operator knobs with comments
- kb_e6d3bbb1 and kb_87a7eade updated to reflect Plan 3 completion
- Carry-forward items (live MCP probe, Heimdal integration test) tracked in the Plan 5 hand-off

---

## 12. Implementation hand-off

Next step: invoke `superpowers:writing-plans` to produce the Plan 3 implementation plan at `docs/superpowers/plans/2026-05-27-threads-of-time-1.0.0-subset-gating.md`. The plan author should:

- Sequence Phase A tasks before Phase B tasks (two ship checkpoints)
- Dispatch `agentic-harness-engineer` for the two new C++ adapters + harness wiring + parity tests
- Dispatch `agent-orchestration-architect` for the brain `SubsetGate` module + LoopSupervisor modifications + admin endpoints
- Keep `game-design-architect` available for whitelist tuning questions (Phase B tier whitelist is the most likely place for design pushback)
- Defer the Heimdal end-to-end test to Plan 5, following the Plan 2 Task 38 precedent
- Include a pre-flight check that grep-verifies the mod-playerbots accessor for population listing before any C++ is written (kb_6950a902 item 1)

The constraints carried into the plan:
- License: GPL-2.0-or-later; SPDX headers on every new file (kb_8ae0ed8a)
- `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` trailer on every commit
- Existing 282 + 115 + 43 tests must stay green throughout
- `cmake -j4` discipline maintained for any worldserver build steps
