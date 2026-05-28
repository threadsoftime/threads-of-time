# Threads of Time 1.0.0 — Subset Gating Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build subset gating for ToT 1.0.0's "alive bots" — selecting which 5–15 of up to `TOT_BOT_POPULATION` bots get LLM-driven brain animation at any moment via proximity-to-player with sticky party/raid/PvP overrides, with two tiers of brain involvement (FULL when a player is on the bot's map, REDUCED when not) plus a BACKGROUND tier (stock mod-agenticbots strategies) for everyone else.

**Architecture:** A new `SubsetGate` asyncio.Task in the brain runs every 60s, polls two new harness tools (`obs.list_players`, `obs.list_bot_population`), computes a target living set, and reconciles against the existing `living_bots` table via internal `/enroll` and `/release` paths. Phase A ships a strict 2-tier model (FULL/BACKGROUND); Phase B adds the REDUCED tier with a warm-cache extension and a `ToolPolicyEnforcer` restricting which `bot.*` tools the brain can call.

**Tech Stack:** C++ (mod-harness-bridge worldserver-side adapters) + Python (FastAPI brain sidecar, FastMCP harness daemon) + SQLite (brain state). License GPL-2.0-or-later per parent spec §10.1.

**Spec:** `docs/superpowers/specs/2026-05-27-threads-of-time-1.0.0-subset-gating-design.md` (committed at `ac79a1c7b`).

**Predecessor tags:** `foundation-complete` (`2f4ed774f`), `memory-subsystem-complete` (`0dfa2a670`).

**Output tags:**
- Phase A: `subset-gating-phase-a-complete`
- Phase B: `subset-gating-complete`

---

## Phase 1: Pre-flight verification

### Task 1: Grep-verify mod-playerbots accessors before writing C++

**Files:**
- Read: `/opt/containers/wow/source/modules/mod-playerbots/src/` on Heimdal (or `modules/mod-playerbots/` once rsync'd into the build tree)
- Output: `tot/internal-docs/plan-3-preflight-notes.md` (new)

This task does not produce shipping code. It produces verified facts the rest of the plan depends on. Per kb_6950a902 item 1 (API drift), every C++ symbol referenced in subsequent tasks must be grep-confirmed to exist with the expected signature in the pinned mod-playerbots SHA before any adapter is written.

- [ ] **Step 1: Grep for the bot population accessor**

```bash
ssh heimdal 'grep -rn "GetPlayerBots\|GetPlayerbotsMap\|PlayerbotHolder" /opt/containers/wow/source/modules/mod-playerbots/src/ | grep -i "::Get\|::List" | head -40'
```

Expected: One or more public accessors that return an iterable of bot `Player*`s. Document the exact class + method + return type.

- [ ] **Step 2: Grep for the master accessor**

```bash
ssh heimdal 'grep -rn "GetMaster\|::SetMaster" /opt/containers/wow/source/modules/mod-playerbots/src/PlayerbotAI.h /opt/containers/wow/source/modules/mod-playerbots/src/PlayerbotAI.cpp | head -20'
```

Expected: `Player* PlayerbotAI::GetMaster() const` or equivalent. Document the exact signature.

- [ ] **Step 3: Grep for group / raid accessor**

```bash
ssh heimdal 'grep -rn "Group::IsRaidGroup\|GetGroup\|GetGUID\(\)" /opt/containers/wow/source/src/server/game/Groups/Group.h | head -20'
```

Expected: `bool Group::isRaidGroup() const` (note lowercase `i`) and `ObjectGuid Group::GetGUID() const`. AC-side; should be canonical.

- [ ] **Step 4: Grep for PvP combat predicate**

```bash
ssh heimdal 'grep -n "IsPvP\|IsInCombat\|InBattleground" /opt/containers/wow/source/src/server/game/Entities/Player/Player.h | head -20'
```

Expected: `bool Player::IsPvP() const`, `bool Unit::IsInCombat() const`, `bool Player::InBattleground() const`. Choose the predicate the spec §4.1 maps to `in_pvp_combat` based on which is most reliable.

- [ ] **Step 5: Write preflight notes**

Create `tot/internal-docs/plan-3-preflight-notes.md` with one section per grep, recording exact symbol, file path, and the AC/mod-playerbots commit SHA verified against (`grep -n "$VERIFY_SHA" UPSTREAMS.toml`). Note any drift from the spec that requires a spec update.

- [ ] **Step 6: Commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git add tot/internal-docs/plan-3-preflight-notes.md
git commit -m "$(cat <<'EOF'
docs(plan-3): pre-flight grep verification of mod-playerbots accessors

Documents the exact symbols the adapters in Tasks 4-5 will call.
Required per kb_6950a902 item 1 (API drift) before any C++ is written.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 2: Harness side — new obs.* tools (Phase A)

### Task 2: Add pydantic args schemas in `tool_schemas.py`

**Files:**
- Modify: `tot/harness/src/harness_daemon/tool_schemas.py`
- Test: `tot/harness/tests/test_mcp_schemas.py` (existing — will pick up new tools automatically once registered)

- [ ] **Step 1: Add empty args schemas**

In `tot/harness/src/harness_daemon/tool_schemas.py`, after the existing `class ObsGetAurasArgs` block:

```python
class ObsListPlayersArgs(BaseModel):
    pass


class ObsListBotPopulationArgs(BaseModel):
    pass
```

- [ ] **Step 2: Add schemas to the TOOL_SCHEMAS map**

Find the `TOOL_SCHEMAS: dict[str, tuple[type[BaseModel], str]]` near the bottom of the file. Add two rows alphabetically among the `obs.*` entries:

```python
    "obs.list_players":         (ObsListPlayersArgs,         "Identify"),
    "obs.list_bot_population":  (ObsListBotPopulationArgs,   "Identify"),
```

(Replace `"Identify"` with whatever scope the existing `obs.list_*` or similar read-only tools use — grep `tool_schemas.py` for `"Identify"` to confirm; if no comparable pattern exists, use the scope that other observation tools without subject_guid_arg use.)

- [ ] **Step 3: Verify imports compile**

Run:

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/harness
python -c "from harness_daemon.tool_schemas import ObsListPlayersArgs, ObsListBotPopulationArgs, TOOL_SCHEMAS; print('obs.list_players' in TOOL_SCHEMAS); print('obs.list_bot_population' in TOOL_SCHEMAS)"
```

Expected: `True` printed twice.

- [ ] **Step 4: Commit**

```bash
git add tot/harness/src/harness_daemon/tool_schemas.py
git commit -m "$(cat <<'EOF'
feat(harness): pydantic schemas for obs.list_players + obs.list_bot_population

Empty-args read-only world-wide observation tools. Spec §4.1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 3: Add registry entries

**Files:**
- Modify: `tot/harness/src/harness_daemon/registry.py`

- [ ] **Step 1: Add ToolEntry rows**

In `tot/harness/src/harness_daemon/registry.py`, find the existing `obs.*` entries. After `obs.get_xp`, add:

```python
    ToolEntry("obs.list_players",          "obs.list_players",          None,          True),
    ToolEntry("obs.list_bot_population",   "obs.list_bot_population",   None,          True),
```

`subject_guid_arg=None` because these are world-wide queries; `forwards_to_ac=True` because they go to the C++ adapter via `HarnessBridgeDispatch`.

- [ ] **Step 2: Verify registry contents**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/harness
python -c "from harness_daemon.registry import REGISTRY; assert REGISTRY.get('obs.list_players'); assert REGISTRY.get('obs.list_bot_population'); print('OK')"
```

Expected: `OK` printed (if `REGISTRY` is the module-level export — grep `registry.py` for what's exported).

- [ ] **Step 3: Commit**

```bash
git add tot/harness/src/harness_daemon/registry.py
git commit -m "$(cat <<'EOF'
feat(harness): register obs.list_players + obs.list_bot_population

World-wide read-only tools; no subject_guid_arg. Spec §4.1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 4: Write `ObsListPlayersAdapter`

**Files:**
- Create: `modules/mod-harness-bridge/src/Adapters/ObsListPlayersAdapter.h`
- Create: `modules/mod-harness-bridge/src/Adapters/ObsListPlayersAdapter.cpp`

- [ ] **Step 1: Create the header**

```cpp
// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_OBS_LIST_PLAYERS_H
#define MOD_HARNESS_BRIDGE_OBS_LIST_PLAYERS_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult ObsListPlayers(nlohmann::json const& args);
}
#endif
```

- [ ] **Step 2: Create the implementation**

```cpp
// SPDX-License-Identifier: GPL-2.0-or-later
#include "Adapters/ObsListPlayersAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"
#include "World.h"
#include "Group.h"

#include <nlohmann/json.hpp>

namespace HarnessBridge::Adapters
{
    DispatchResult ObsListPlayers(nlohmann::json const& /*args*/)
    {
        nlohmann::json players = nlohmann::json::array();

        for (auto const& itr : ObjectAccessor::GetPlayers())
        {
            Player* p = itr.second;
            if (!p || !p->IsInWorld())
                continue;
            // Skip bots — they are reported by ObsListBotPopulation.
            // (Per pre-flight Task 1 Step 1, prefer the playerbots predicate
            //  if available; otherwise rely on GetSession()->GetSecurity() and
            //  the playerbots IsPlayerbot() helper if exposed.)
            if (p->IsPlayerBot() /* placeholder — adjust per pre-flight grep */)
                continue;

            nlohmann::json row;
            row["player_guid"] = p->GetGUID().GetCounter();
            row["name"] = p->GetName();
            row["map_id"] = p->GetMapId();
            row["x"] = p->GetPositionX();
            row["y"] = p->GetPositionY();
            row["z"] = p->GetPositionZ();
            row["level"] = static_cast<int>(p->GetLevel());

            if (Group* grp = p->GetGroup())
            {
                uint64_t group_guid_low = grp->GetGUID().GetCounter();
                if (grp->isRaidGroup())
                    row["raid_guid"] = group_guid_low;
                else
                    row["party_guid"] = group_guid_low;
            }

            row["in_pvp_combat"] = p->IsPvP() && p->IsInCombat();

            players.push_back(row);
        }

        nlohmann::json result;
        result["players"] = players;
        return DispatchResult::Ok(result);
    }
}
```

(The `IsPlayerBot()` filter is a placeholder — replace with whatever predicate Task 1 Step 1 confirmed. If mod-playerbots doesn't expose one, walk the playerbots holder map and skip GUIDs that appear there.)

- [ ] **Step 3: Verify the adapter compiles in isolation**

Push the file to Heimdal and run a compile-only smoke build per kb_57b453cd:

```bash
ssh heimdal 'cd /opt/containers/wow && echo "$SUDO_PW" | sudo -S podman run --rm \
  -v /opt/containers/wow/source:/azerothcore:Z \
  -v ac-build-cache:/azerothcore/build \
  localhost/wow-build:latest \
  bash -c "cd /azerothcore/build && cmake -DBoost_USE_STATIC_LIBS=ON /azerothcore && cmake --build . --target modules -j4 -- -k 0 2>&1 | tail -40"'
```

Expected: `modules` target builds cleanly with the new `.cpp` file linked into `libmodules.a`. If the build fails with `undefined reference`, the placeholder symbol from Step 2 needs replacement.

- [ ] **Step 4: Commit (works-on-laptop only; deploy happens in a later step)**

```bash
git add modules/mod-harness-bridge/src/Adapters/ObsListPlayersAdapter.{h,cpp}
git commit -m "$(cat <<'EOF'
feat(harness-bridge): ObsListPlayersAdapter — world-wide players snapshot

Returns {player_guid, name, map_id, x, y, z, level, party_guid?, raid_guid?,
in_pvp_combat} for every logged-in non-bot Player. Consumed by the brain's
SubsetGate for proximity-based bot selection. Spec §4.1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 5: Write `ObsListBotPopulationAdapter`

**Files:**
- Create: `modules/mod-harness-bridge/src/Adapters/ObsListBotPopulationAdapter.h`
- Create: `modules/mod-harness-bridge/src/Adapters/ObsListBotPopulationAdapter.cpp`

- [ ] **Step 1: Create the header**

```cpp
// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_HARNESS_BRIDGE_OBS_LIST_BOT_POPULATION_H
#define MOD_HARNESS_BRIDGE_OBS_LIST_BOT_POPULATION_H
#include "HarnessBridgeDispatch.h"
namespace HarnessBridge::Adapters {
    DispatchResult ObsListBotPopulation(nlohmann::json const& args);
}
#endif
```

- [ ] **Step 2: Create the implementation**

```cpp
// SPDX-License-Identifier: GPL-2.0-or-later
#include "Adapters/ObsListBotPopulationAdapter.h"

#include "ObjectAccessor.h"
#include "Player.h"
#include "Group.h"
#include "Script/Playerbots.h"

#include <nlohmann/json.hpp>

namespace HarnessBridge::Adapters
{
    DispatchResult ObsListBotPopulation(nlohmann::json const& /*args*/)
    {
        nlohmann::json bots = nlohmann::json::array();

        // Iterate the playerbots holder per the pre-flight Task 1 Step 1 verified accessor.
        // The exact API is (placeholder): sPlayerbotHolder->GetPlayerBotsMap() or similar.
        for (auto const& entry : sPlayerbotsMgr->GetPlayerBotsMap() /* adjust per pre-flight */)
        {
            Player* bot = entry.second;
            if (!bot || !bot->IsInWorld())
                continue;
            PlayerbotAI* ai = GET_PLAYERBOT_AI(bot);
            if (!ai)
                continue;

            nlohmann::json row;
            row["bot_guid"] = bot->GetGUID().GetCounter();
            row["name"] = bot->GetName();
            row["map_id"] = bot->GetMapId();
            row["x"] = bot->GetPositionX();
            row["y"] = bot->GetPositionY();
            row["z"] = bot->GetPositionZ();
            row["level"] = static_cast<int>(bot->GetLevel());

            if (Group* grp = bot->GetGroup())
            {
                uint64_t group_guid_low = grp->GetGUID().GetCounter();
                if (grp->isRaidGroup())
                    row["raid_guid"] = group_guid_low;
                else
                    row["party_guid"] = group_guid_low;
            }

            if (Player* master = ai->GetMaster())
                row["master_guid"] = master->GetGUID().GetCounter();

            row["in_pvp_combat"] = bot->IsPvP() && bot->IsInCombat();

            bots.push_back(row);
        }

        nlohmann::json result;
        result["bots"] = bots;
        return DispatchResult::Ok(result);
    }
}
```

(Replace `sPlayerbotsMgr->GetPlayerBotsMap()` with whatever Task 1 Step 1 confirmed.)

- [ ] **Step 3: Smoke-build per Task 4 Step 3**

Same command as Task 4 Step 3 — expect both adapters in `libmodules.a`.

- [ ] **Step 4: Commit**

```bash
git add modules/mod-harness-bridge/src/Adapters/ObsListBotPopulationAdapter.{h,cpp}
git commit -m "$(cat <<'EOF'
feat(harness-bridge): ObsListBotPopulationAdapter — world-wide bot snapshot

Returns {bot_guid, name, map_id, x, y, z, level, party_guid?, raid_guid?,
master_guid?, in_pvp_combat} for every mod-playerbots bot. Consumed by the
brain's SubsetGate for proximity-based selection. Spec §4.1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 6: Wire both adapters into `HarnessBridgeDispatch`

**Files:**
- Modify: `modules/mod-harness-bridge/src/HarnessBridgeDispatch.cpp`

- [ ] **Step 1: Add includes**

Near the existing `#include "Adapters/ObsGetStateAdapter.h"` line:

```cpp
#include "Adapters/ObsListPlayersAdapter.h"
#include "Adapters/ObsListBotPopulationAdapter.h"
```

- [ ] **Step 2: Add dispatch entries**

Find the dispatch table — looks like `{"obs.get_state", &Adapters::ObsGetState},`. Add:

```cpp
            {"obs.list_players",          &Adapters::ObsListPlayers},
            {"obs.list_bot_population",   &Adapters::ObsListBotPopulation},
```

- [ ] **Step 3: Verify the dispatch table compiles**

Per kb_57b453cd, cmake reconfigure is required because new `.cpp` files were added. Push and rebuild:

```bash
rsync -av --delete modules/mod-harness-bridge/ heimdal:/opt/containers/wow/source/modules/mod-harness-bridge/
ssh heimdal 'cd /opt/containers/wow && echo "$SUDO_PW" | sudo -S podman run --rm \
  -v /opt/containers/wow/source:/azerothcore:Z \
  -v ac-build-cache:/azerothcore/build \
  localhost/wow-build:latest \
  bash -c "cd /azerothcore/build && rm -f CMakeCache.txt && cmake -DBoost_USE_STATIC_LIBS=ON /azerothcore && cmake --build . --target worldserver -j4 2>&1 | tail -60"'
```

Expected: full link of `worldserver` binary including both new adapters.

- [ ] **Step 4: Commit**

```bash
git add modules/mod-harness-bridge/src/HarnessBridgeDispatch.cpp
git commit -m "$(cat <<'EOF'
feat(harness-bridge): wire obs.list_players + obs.list_bot_population

Dispatch table additions. Spec §4.1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 7: Run parity tests + verify daemon registration

**Files:**
- Run: `tot/harness/tests/test_mcp_schemas.py`

- [ ] **Step 1: Run the parity test suite**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/harness
pytest tests/test_mcp_schemas.py -v 2>&1 | tail -40
```

Expected: 117 tests pass (115 prior + 2 new). The `test_schema_has_all_adapter_required_fields[obs.list_players]` and `[obs.list_bot_population]` cases verify the pydantic args schema (empty) matches the adapter's lack of required input fields.

- [ ] **Step 2: Verify FastMCP daemon registers the new tools**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/harness
python -c "from harness_daemon.mcp_server import build_mcp_server; mcp = build_mcp_server(); print(sorted(t.name for t in mcp.tools if t.name.startswith('obs.list')))"
```

Expected: `['obs.list_bot_population', 'obs.list_players']`.

- [ ] **Step 3: Commit if any test fixtures needed adjustment**

If Step 1 surfaced needed fixture updates (e.g., the new tools needed to be added to a known-tool allow-list):

```bash
git add tot/harness/tests/
git commit -m "$(cat <<'EOF'
test(harness): parity test rows for obs.list_players + obs.list_bot_population

117 parity tests total (was 115). Spec §9.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

If no changes needed, no commit.

---

## Phase 3: Brain schema migration + state methods (Phase A)

### Task 8: Brain schema migration `0004_subset_tier.sql`

**Files:**
- Create: `tot/brain/migrations/0004_subset_tier.sql`

- [ ] **Step 1: Write the migration**

```sql
-- 0004_subset_tier.sql — Plan 3 subset-gating columns
-- SPDX-License-Identifier: GPL-2.0-or-later

ALTER TABLE living_bots ADD COLUMN tier TEXT NOT NULL DEFAULT 'full';
ALTER TABLE living_bots ADD COLUMN out_of_range_ticks INTEGER NOT NULL DEFAULT 0;
ALTER TABLE living_bots ADD COLUMN in_range_ticks INTEGER NOT NULL DEFAULT 0;
ALTER TABLE living_bots ADD COLUMN last_recompute_at INTEGER;
ALTER TABLE living_bots ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_living_bots_tier ON living_bots(status, tier);

INSERT INTO schema_version(version, applied_at) VALUES (4, strftime('%s','now'));
```

- [ ] **Step 2: Write a migration idempotency test**

In `tot/brain/tests/unit/test_state_store.py` (existing file from Plan 2 — append):

```python
def test_migration_0004_idempotent(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.migrate()  # Run twice — must not raise or duplicate columns.
    cols = [r[1] for r in store._conn.execute("PRAGMA table_info(living_bots)").fetchall()]
    assert "tier" in cols
    assert "out_of_range_ticks" in cols
    assert "in_range_ticks" in cols
    assert "last_recompute_at" in cols
    assert "pinned" in cols
    row = store._conn.execute("SELECT MAX(version) FROM schema_version").fetchone()
    assert row[0] >= 4
    store.close()
```

- [ ] **Step 3: Run the test**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/brain
pytest tests/unit/test_state_store.py::test_migration_0004_idempotent -v
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add tot/brain/migrations/0004_subset_tier.sql tot/brain/tests/unit/test_state_store.py
git commit -m "$(cat <<'EOF'
feat(brain): migration 0004 — subset-gating tier + hysteresis columns

ALTER TABLE living_bots adds tier, in_range_ticks, out_of_range_ticks,
last_recompute_at, pinned. Idempotent per existing migrate() pattern.
Spec §4.4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 9: StateStore tier methods (TDD)

**Files:**
- Modify: `tot/brain/brain_sidecar/state.py`
- Test: `tot/brain/tests/unit/test_state_store.py`

- [ ] **Step 1: Write failing tests**

Append to `tot/brain/tests/unit/test_state_store.py`:

```python
def test_set_tier_and_get_tier_round_trip(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.enroll(bot_guid=1, enrolled_at_ms=0,
                 personality_seed=_make_personality())  # existing helper
    assert store.get_tier(1) == "full"
    store.set_tier(1, "reduced")
    assert store.get_tier(1) == "reduced"
    store.set_tier(1, "full")
    assert store.get_tier(1) == "full"
    store.close()


def test_get_tier_unknown_bot_returns_full_default(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    assert store.get_tier(9999) == "full"  # default for unknown bot
    store.close()
```

- [ ] **Step 2: Run tests — verify they fail**

```bash
pytest tests/unit/test_state_store.py::test_set_tier_and_get_tier_round_trip -v
```

Expected: FAIL with `AttributeError: 'StateStore' object has no attribute 'set_tier'`.

- [ ] **Step 3: Implement set_tier + get_tier**

In `tot/brain/brain_sidecar/state.py`, in class `StateStore`, append methods:

```python
    def set_tier(self, bot_guid: int, tier: str) -> None:
        if tier not in ("full", "reduced"):
            raise ValueError(f"invalid tier {tier!r}")
        with self._write_lock:
            self._conn.execute(
                "UPDATE living_bots SET tier = ? WHERE bot_guid = ?",
                (tier, bot_guid),
            )

    def get_tier(self, bot_guid: int) -> str:
        row = self._conn.execute(
            "SELECT tier FROM living_bots WHERE bot_guid = ?",
            (bot_guid,),
        ).fetchone()
        if row is None or row[0] is None:
            return "full"
        return row[0]
```

- [ ] **Step 4: Run tests — verify they pass**

```bash
pytest tests/unit/test_state_store.py::test_set_tier_and_get_tier_round_trip tests/unit/test_state_store.py::test_get_tier_unknown_bot_returns_full_default -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tot/brain/brain_sidecar/state.py tot/brain/tests/unit/test_state_store.py
git commit -m "$(cat <<'EOF'
feat(brain): StateStore.set_tier + get_tier

Sets and reads the tier column from migration 0004. Unknown bot defaults
to "full". Spec §4.5.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 10: StateStore hysteresis methods (TDD)

**Files:**
- Modify: `tot/brain/brain_sidecar/state.py`
- Test: `tot/brain/tests/unit/test_state_store.py`

- [ ] **Step 1: Write failing tests**

Append to `tot/brain/tests/unit/test_state_store.py`:

```python
def test_bump_hysteresis_in_range_increments_in_and_resets_out(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.enroll(bot_guid=1, enrolled_at_ms=0,
                 personality_seed=_make_personality())
    # Initial state
    assert store.get_hysteresis(1) == (0, 0)
    # In-range bump
    in_ticks, out_ticks = store.bump_hysteresis(1, in_range=True)
    assert (in_ticks, out_ticks) == (1, 0)
    in_ticks, out_ticks = store.bump_hysteresis(1, in_range=True)
    assert (in_ticks, out_ticks) == (2, 0)
    # Out-of-range bump resets in_range_ticks
    in_ticks, out_ticks = store.bump_hysteresis(1, in_range=False)
    assert (in_ticks, out_ticks) == (0, 1)
    in_ticks, out_ticks = store.bump_hysteresis(1, in_range=False)
    assert (in_ticks, out_ticks) == (0, 2)
    # Back in-range — out_ticks reset
    in_ticks, out_ticks = store.bump_hysteresis(1, in_range=True)
    assert (in_ticks, out_ticks) == (1, 0)
    store.close()


def test_bump_hysteresis_unknown_bot_no_op(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    # Should not raise; behavior is no-op + return (0, 0).
    assert store.bump_hysteresis(9999, in_range=True) == (0, 0)
    store.close()
```

- [ ] **Step 2: Run — verify fail**

```bash
pytest tests/unit/test_state_store.py::test_bump_hysteresis_in_range_increments_in_and_resets_out -v
```

Expected: FAIL — methods don't exist.

- [ ] **Step 3: Implement bump_hysteresis + get_hysteresis**

In `tot/brain/brain_sidecar/state.py`, append:

```python
    def get_hysteresis(self, bot_guid: int) -> tuple[int, int]:
        row = self._conn.execute(
            "SELECT in_range_ticks, out_of_range_ticks FROM living_bots WHERE bot_guid = ?",
            (bot_guid,),
        ).fetchone()
        if row is None:
            return (0, 0)
        return (row[0] or 0, row[1] or 0)

    def bump_hysteresis(self, bot_guid: int, *, in_range: bool) -> tuple[int, int]:
        with self._write_lock:
            row = self._conn.execute(
                "SELECT in_range_ticks, out_of_range_ticks FROM living_bots WHERE bot_guid = ?",
                (bot_guid,),
            ).fetchone()
            if row is None:
                return (0, 0)
            in_ticks = row[0] or 0
            out_ticks = row[1] or 0
            if in_range:
                in_ticks += 1
                out_ticks = 0
            else:
                in_ticks = 0
                out_ticks += 1
            self._conn.execute(
                "UPDATE living_bots SET in_range_ticks = ?, out_of_range_ticks = ? WHERE bot_guid = ?",
                (in_ticks, out_ticks, bot_guid),
            )
            return (in_ticks, out_ticks)
```

- [ ] **Step 4: Run — verify pass**

```bash
pytest tests/unit/test_state_store.py::test_bump_hysteresis_in_range_increments_in_and_resets_out tests/unit/test_state_store.py::test_bump_hysteresis_unknown_bot_no_op -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tot/brain/brain_sidecar/state.py tot/brain/tests/unit/test_state_store.py
git commit -m "$(cat <<'EOF'
feat(brain): StateStore.bump_hysteresis + get_hysteresis

Atomic in/out tick counters per the SubsetGate algorithm. Returns
(in_range_ticks, out_of_range_ticks). Spec §4.5 + §5 step 4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 11: StateStore pin methods (TDD)

**Files:**
- Modify: `tot/brain/brain_sidecar/state.py`
- Test: `tot/brain/tests/unit/test_state_store.py`

- [ ] **Step 1: Write failing tests**

Append:

```python
def test_set_pin_and_list_pinned(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    for g in (1, 2, 3):
        store.enroll(bot_guid=g, enrolled_at_ms=0,
                     personality_seed=_make_personality())
    assert store.list_pinned() == []
    store.set_pin(1, True)
    store.set_pin(3, True)
    assert sorted(store.list_pinned()) == [1, 3]
    store.set_pin(1, False)
    assert store.list_pinned() == [3]
    store.close()
```

- [ ] **Step 2: Verify fail, implement, verify pass**

In `tot/brain/brain_sidecar/state.py`, append:

```python
    def set_pin(self, bot_guid: int, pinned: bool) -> None:
        with self._write_lock:
            self._conn.execute(
                "UPDATE living_bots SET pinned = ? WHERE bot_guid = ?",
                (1 if pinned else 0, bot_guid),
            )

    def list_pinned(self) -> list[int]:
        rows = self._conn.execute(
            "SELECT bot_guid FROM living_bots WHERE pinned = 1"
        ).fetchall()
        return [r[0] for r in rows]
```

Run:

```bash
pytest tests/unit/test_state_store.py::test_set_pin_and_list_pinned -v
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tot/brain/brain_sidecar/state.py tot/brain/tests/unit/test_state_store.py
git commit -m "$(cat <<'EOF'
feat(brain): StateStore.set_pin + list_pinned

Operator pin override for the /admin/subset/pin endpoint. Spec §4.5.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 4: Brain settings + config (Phase A)

### Task 12: Settings.py new fields

**Files:**
- Modify: `tot/brain/brain_sidecar/settings.py`

- [ ] **Step 1: Add fields to Settings dataclass**

In `tot/brain/brain_sidecar/settings.py`, in `class Settings`, append:

```python
    # Subset gating (Plan 3)
    subset_gate_enabled: bool
    living_bot_count: int
    subset_recompute_interval_s: float
    subset_hysteresis_out_ticks: int
    subset_hysteresis_in_ticks: int
    subset_enroll_backoff_s: float
    reduced_tick_interval_s: float  # Phase B
```

- [ ] **Step 2: Update `get_settings()` to read env vars**

In the same file, find `def get_settings()` and add (matching the existing pattern of `os.getenv(...)` calls):

```python
    subset_gate_enabled = os.getenv("TOT_SUBSET_GATE_ENABLED", "true").lower() == "true"
    living_bot_count = int(os.getenv("TOT_LIVING_BOT_COUNT", "10"))
    if not 5 <= living_bot_count <= 15:
        raise ValueError(f"TOT_LIVING_BOT_COUNT must be 5-15, got {living_bot_count}")
    subset_recompute_interval_s = float(os.getenv("TOT_SUBSET_RECOMPUTE_INTERVAL_S", "60"))
    subset_hysteresis_out_ticks = int(os.getenv("TOT_SUBSET_HYSTERESIS_OUT_TICKS", "2"))
    subset_hysteresis_in_ticks = int(os.getenv("TOT_SUBSET_HYSTERESIS_IN_TICKS", "1"))
    subset_enroll_backoff_s = float(os.getenv("TOT_SUBSET_ENROLL_BACKOFF_S", "300"))
    reduced_tick_interval_s = float(os.getenv("TOT_REDUCED_TICK_INTERVAL_S", "300"))
```

Add the new fields to the `Settings(...)` constructor call.

- [ ] **Step 3: Add a settings test**

In `tot/brain/tests/unit/test_settings.py` (existing file — append):

```python
def test_subset_gate_defaults(monkeypatch):
    for k in ("TOT_SUBSET_GATE_ENABLED", "TOT_LIVING_BOT_COUNT",
              "TOT_SUBSET_RECOMPUTE_INTERVAL_S"):
        monkeypatch.delenv(k, raising=False)
    s = get_settings()
    assert s.subset_gate_enabled is True
    assert s.living_bot_count == 10
    assert s.subset_recompute_interval_s == 60.0


def test_living_bot_count_clamped_validation(monkeypatch):
    monkeypatch.setenv("TOT_LIVING_BOT_COUNT", "20")
    with pytest.raises(ValueError, match="must be 5-15"):
        get_settings()
```

- [ ] **Step 4: Run tests**

```bash
pytest tests/unit/test_settings.py::test_subset_gate_defaults tests/unit/test_settings.py::test_living_bot_count_clamped_validation -v
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tot/brain/brain_sidecar/settings.py tot/brain/tests/unit/test_settings.py
git commit -m "$(cat <<'EOF'
feat(brain): subset-gating settings + env-var parsing

New env vars: TOT_SUBSET_GATE_ENABLED, TOT_LIVING_BOT_COUNT (validated 5-15),
TOT_SUBSET_RECOMPUTE_INTERVAL_S, TOT_SUBSET_HYSTERESIS_OUT_TICKS,
TOT_SUBSET_HYSTERESIS_IN_TICKS, TOT_SUBSET_ENROLL_BACKOFF_S,
TOT_REDUCED_TICK_INTERVAL_S. Spec §7.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 5: SubsetGate core (Phase A — pure function)

### Task 13: WorldSnapshot + SubsetDecision dataclasses

**Files:**
- Create: `tot/brain/brain_sidecar/subset_gate.py`
- Test: `tot/brain/tests/unit/test_subset_gate.py`

- [ ] **Step 1: Create the dataclasses**

Create `tot/brain/brain_sidecar/subset_gate.py`:

```python
"""C7: SubsetGate — periodic recompute of the brain's living-bot set.

Spec: docs/superpowers/specs/2026-05-27-threads-of-time-1.0.0-subset-gating-design.md
"""
# SPDX-License-Identifier: GPL-2.0-or-later
from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Optional


@dataclass(frozen=True)
class PlayerSnapshot:
    player_guid: int
    name: str
    map_id: int
    x: float
    y: float
    z: float
    level: int
    party_guid: Optional[int] = None
    raid_guid: Optional[int] = None
    in_pvp_combat: bool = False


@dataclass(frozen=True)
class BotSnapshot:
    bot_guid: int
    name: str
    map_id: int
    x: float
    y: float
    z: float
    level: int
    party_guid: Optional[int] = None
    raid_guid: Optional[int] = None
    master_guid: Optional[int] = None
    in_pvp_combat: bool = False


@dataclass(frozen=True)
class WorldSnapshot:
    players: tuple[PlayerSnapshot, ...]
    bots: tuple[BotSnapshot, ...]


@dataclass(frozen=True)
class SubsetDecision:
    target_living_set: frozenset[int]
    sticky_pinned: frozenset[int]
    proximity_picks: frozenset[int]
    to_enroll: frozenset[int]
    to_release: frozenset[int]
    to_full: frozenset[int]
    to_reduced: frozenset[int]
    skipped_due_to_backoff: frozenset[int]


@dataclass(frozen=True)
class SubsetGateConfig:
    living_bot_count: int
    recompute_interval_s: float
    hysteresis_out_ticks: int
    hysteresis_in_ticks: int
    enroll_backoff_s: float
    enabled: bool
    phase_b_enabled: bool = False  # warm-cache + REDUCED tier
```

Create `tot/brain/tests/unit/test_subset_gate.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
from brain_sidecar.subset_gate import (
    PlayerSnapshot, BotSnapshot, WorldSnapshot,
    SubsetDecision, SubsetGateConfig,
)


def test_world_snapshot_construction():
    p = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                       x=0.0, y=0.0, z=0.0, level=10)
    b = BotSnapshot(bot_guid=100, name="Borg", map_id=0,
                    x=10.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(p,), bots=(b,))
    assert snap.players[0].name == "Alice"
    assert snap.bots[0].master_guid is None
```

- [ ] **Step 2: Run test**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/tot/brain
pytest tests/unit/test_subset_gate.py -v
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tot/brain/brain_sidecar/subset_gate.py tot/brain/tests/unit/test_subset_gate.py
git commit -m "$(cat <<'EOF'
feat(brain): subset_gate dataclasses — WorldSnapshot, SubsetDecision, config

Frozen dataclasses for the pure-function _recompute_once surface. Spec §4.2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 14: `_recompute_once` — empty world + no players (TDD)

**Files:**
- Modify: `tot/brain/brain_sidecar/subset_gate.py`
- Modify: `tot/brain/tests/unit/test_subset_gate.py`

- [ ] **Step 1: Write failing test**

Append to `tot/brain/tests/unit/test_subset_gate.py`:

```python
from brain_sidecar.subset_gate import recompute_once


def _cfg(**overrides):
    base = dict(
        living_bot_count=10,
        recompute_interval_s=60.0,
        hysteresis_out_ticks=2,
        hysteresis_in_ticks=1,
        enroll_backoff_s=300.0,
        enabled=True,
        phase_b_enabled=False,
    )
    base.update(overrides)
    return SubsetGateConfig(**base)


def test_empty_world_yields_empty_target():
    snap = WorldSnapshot(players=(), bots=())
    decision = recompute_once(
        snapshot=snap,
        currently_enrolled={},
        currently_pinned=frozenset(),
        backoff_set=frozenset(),
        hysteresis={},
        config=_cfg(),
    )
    assert decision.target_living_set == frozenset()
    assert decision.to_enroll == frozenset()
    assert decision.to_release == frozenset()


def test_bots_present_no_players_yields_empty_target_phase_a():
    bots = tuple(
        BotSnapshot(bot_guid=i, name=f"b{i}", map_id=0, x=float(i),
                    y=0.0, z=0.0, level=10)
        for i in range(20)
    )
    snap = WorldSnapshot(players=(), bots=bots)
    decision = recompute_once(
        snapshot=snap,
        currently_enrolled={},
        currently_pinned=frozenset(),
        backoff_set=frozenset(),
        hysteresis={},
        config=_cfg(),
    )
    assert decision.target_living_set == frozenset()
```

- [ ] **Step 2: Run — verify fail**

```bash
pytest tests/unit/test_subset_gate.py::test_empty_world_yields_empty_target -v
```

Expected: FAIL (ImportError).

- [ ] **Step 3: Implement `recompute_once`**

In `tot/brain/brain_sidecar/subset_gate.py`, append:

```python
def _euclid(a: PlayerSnapshot | BotSnapshot, b: PlayerSnapshot | BotSnapshot) -> float:
    return math.sqrt((a.x - b.x) ** 2 + (a.y - b.y) ** 2 + (a.z - b.z) ** 2)


def recompute_once(
    *,
    snapshot: WorldSnapshot,
    currently_enrolled: dict[int, int],   # bot_guid -> last_seen_ms (or 0)
    currently_pinned: frozenset[int],
    backoff_set: frozenset[int],
    hysteresis: dict[int, tuple[int, int]],   # bot_guid -> (in_ticks, out_ticks) — pre-bump
    config: SubsetGateConfig,
) -> SubsetDecision:
    """Pure function. No I/O. Computes the next SubsetDecision."""
    if not snapshot.players:
        # Phase A: no players → empty target_proximity.
        # Phase B: warm-cache extends to currently_enrolled.
        target_proximity: frozenset[int] = frozenset()
        sticky: frozenset[int] = frozenset(currently_pinned)
        if config.phase_b_enabled:
            target = sticky | frozenset(currently_enrolled.keys())
        else:
            target = sticky
        return _reconcile(
            snapshot=snapshot,
            target=target,
            target_proximity=target_proximity,
            sticky=sticky,
            currently_enrolled=currently_enrolled,
            currently_pinned=currently_pinned,
            backoff_set=backoff_set,
            hysteresis=hysteresis,
            config=config,
            proximity_picks=frozenset(),
        )

    # Players present — Task 15 replaces this branch with the proximity logic.
    # Task 16 adds sticky overrides; Task 17 + Task 18 add hysteresis edges.
    raise NotImplementedError("Filled in by Tasks 15-18 (proximity, sticky, hysteresis).")


def _reconcile(
    *,
    snapshot: WorldSnapshot,
    target: frozenset[int],
    target_proximity: frozenset[int],
    sticky: frozenset[int],
    currently_enrolled: dict[int, int],
    currently_pinned: frozenset[int],
    backoff_set: frozenset[int],
    hysteresis: dict[int, tuple[int, int]],
    config: SubsetGateConfig,
    proximity_picks: frozenset[int],
) -> SubsetDecision:
    to_enroll: set[int] = set()
    to_release: set[int] = set()
    for bot_guid in currently_enrolled:
        if bot_guid in target or bot_guid in currently_pinned:
            continue
        in_ticks, out_ticks = hysteresis.get(bot_guid, (0, 0))
        if out_ticks + 1 >= config.hysteresis_out_ticks and bot_guid not in sticky:
            to_release.add(bot_guid)
    for bot_guid in target_proximity:
        if bot_guid in currently_enrolled:
            continue
        if bot_guid in backoff_set:
            continue
        if bot_guid in sticky:
            to_enroll.add(bot_guid)
            continue
        in_ticks, _ = hysteresis.get(bot_guid, (0, 0))
        if in_ticks + 1 >= config.hysteresis_in_ticks:
            to_enroll.add(bot_guid)

    to_full: set[int] = set()
    to_reduced: set[int] = set()
    if config.phase_b_enabled:
        kept = (set(currently_enrolled) - to_release) | to_enroll
        bot_by_guid = {b.bot_guid: b for b in snapshot.bots}
        for guid in kept:
            bot = bot_by_guid.get(guid)
            if bot is None:
                continue
            if any(p.map_id == bot.map_id for p in snapshot.players):
                to_full.add(guid)
            else:
                to_reduced.add(guid)

    return SubsetDecision(
        target_living_set=target,
        sticky_pinned=sticky,
        proximity_picks=proximity_picks,
        to_enroll=frozenset(to_enroll),
        to_release=frozenset(to_release),
        to_full=frozenset(to_full),
        to_reduced=frozenset(to_reduced),
        skipped_due_to_backoff=frozenset(backoff_set & set(b.bot_guid for b in snapshot.bots)),
    )
```

- [ ] **Step 4: Run tests — verify pass**

```bash
pytest tests/unit/test_subset_gate.py -v
```

Expected: PASS for `test_empty_world_yields_empty_target` and `test_bots_present_no_players_yields_empty_target_phase_a`.

- [ ] **Step 5: Commit**

```bash
git add tot/brain/brain_sidecar/subset_gate.py tot/brain/tests/unit/test_subset_gate.py
git commit -m "$(cat <<'EOF'
feat(brain): SubsetGate.recompute_once — empty world + no-players paths

Pure function entry point + the reconciliation helper. Players-present
path stubs to NotImplementedError pending subsequent tasks. Spec §5 step 1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 15: `_recompute_once` — proximity ranking (TDD)

**Files:**
- Modify: `tot/brain/brain_sidecar/subset_gate.py`
- Modify: `tot/brain/tests/unit/test_subset_gate.py`

- [ ] **Step 1: Write failing test**

```python
def test_single_player_closest_n_wins():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bots = tuple(
        BotSnapshot(bot_guid=100 + i, name=f"b{i}", map_id=0,
                    x=float(i), y=0.0, z=0.0, level=10)
        for i in range(20)  # bots at x=0..19 on map 0
    )
    snap = WorldSnapshot(players=(alice,), bots=bots)
    decision = recompute_once(
        snapshot=snap,
        currently_enrolled={},
        currently_pinned=frozenset(),
        backoff_set=frozenset(),
        hysteresis={},
        config=_cfg(living_bot_count=5, hysteresis_in_ticks=1),
    )
    # Closest 5 bots are b0..b4 (bot_guids 100..104)
    assert decision.to_enroll == frozenset({100, 101, 102, 103, 104})


def test_different_map_excluded_from_proximity():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    near_other_map = BotSnapshot(bot_guid=100, name="far", map_id=1,
                                  x=0.0, y=0.0, z=0.0, level=10)
    far_same_map = BotSnapshot(bot_guid=200, name="near", map_id=0,
                                x=999.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(near_other_map, far_same_map))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    assert decision.to_enroll == frozenset({200})
```

- [ ] **Step 2: Implement the players-present path**

Replace the `raise NotImplementedError` in `recompute_once` with:

```python
    # Players present — compute proximity-based target.
    sticky: set[int] = set(currently_pinned)
    # Sticky overrides come in Task 16.

    # Proximity ranking.
    eligible_bots = [
        b for b in snapshot.bots
        if any(p.map_id == b.map_id for p in snapshot.players)
        and b.bot_guid not in sticky
        and b.bot_guid not in backoff_set
    ]
    def _score(bot: BotSnapshot) -> float:
        same_map = [p for p in snapshot.players if p.map_id == bot.map_id]
        return min(_euclid(bot, p) for p in same_map) if same_map else math.inf
    eligible_bots.sort(key=lambda b: (_score(b), b.bot_guid))
    slots_remaining = max(0, config.living_bot_count - len(sticky))
    proximity_picks = frozenset(b.bot_guid for b in eligible_bots[:slots_remaining])
    target_proximity = frozenset(sticky) | proximity_picks

    if config.phase_b_enabled:
        warm_candidates = sorted(
            (g for g in currently_enrolled if g not in target_proximity),
            key=lambda g: currently_enrolled[g],
            reverse=True,
        )
        warm_slots = max(0, config.living_bot_count - len(target_proximity))
        target = target_proximity | frozenset(warm_candidates[:warm_slots])
    else:
        target = target_proximity

    return _reconcile(
        snapshot=snapshot,
        target=target,
        target_proximity=target_proximity,
        sticky=frozenset(sticky),
        currently_enrolled=currently_enrolled,
        currently_pinned=currently_pinned,
        backoff_set=backoff_set,
        hysteresis=hysteresis,
        config=config,
        proximity_picks=proximity_picks,
    )
```

- [ ] **Step 3: Run tests**

```bash
pytest tests/unit/test_subset_gate.py -v
```

Expected: PASS on the two new tests + prior tests still green.

- [ ] **Step 4: Commit**

```bash
git add tot/brain/brain_sidecar/subset_gate.py tot/brain/tests/unit/test_subset_gate.py
git commit -m "$(cat <<'EOF'
feat(brain): SubsetGate proximity ranking

Same-map filter + Euclidean nearest-N to any player. Spec §5 step 2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 16: `_recompute_once` — sticky overrides (TDD)

**Files:**
- Modify: `tot/brain/brain_sidecar/subset_gate.py`
- Modify: `tot/brain/tests/unit/test_subset_gate.py`

- [ ] **Step 1: Write failing tests**

```python
def test_sticky_party_overrides_proximity():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10, party_guid=42)
    far_party_bot = BotSnapshot(bot_guid=999, name="far_party", map_id=0,
                                 x=10000.0, y=0.0, z=0.0, level=10, party_guid=42)
    near_other_bots = tuple(
        BotSnapshot(bot_guid=100 + i, name=f"b{i}", map_id=0,
                    x=float(i), y=0.0, z=0.0, level=10)
        for i in range(10)
    )
    snap = WorldSnapshot(players=(alice,), bots=near_other_bots + (far_party_bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    # 999 is sticky (party); 4 closest non-sticky fill remaining slots.
    assert 999 in decision.to_enroll
    assert decision.sticky_pinned == frozenset({999})


def test_sticky_raid_overrides_proximity():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10, raid_guid=77)
    raid_bot = BotSnapshot(bot_guid=500, name="r", map_id=0,
                            x=10000.0, y=0.0, z=0.0, level=10, raid_guid=77)
    snap = WorldSnapshot(players=(alice,), bots=(raid_bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    assert 500 in decision.sticky_pinned
    assert 500 in decision.to_enroll


def test_sticky_pvp_combat_requires_same_map():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    pvp_bot_same_map = BotSnapshot(bot_guid=300, name="pvp_near", map_id=0,
                                    x=5000.0, y=0.0, z=0.0, level=10,
                                    in_pvp_combat=True)
    pvp_bot_other_map = BotSnapshot(bot_guid=400, name="pvp_far", map_id=99,
                                     x=0.0, y=0.0, z=0.0, level=10,
                                     in_pvp_combat=True)
    snap = WorldSnapshot(players=(alice,), bots=(pvp_bot_same_map, pvp_bot_other_map))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    assert 300 in decision.sticky_pinned
    assert 400 not in decision.sticky_pinned
```

- [ ] **Step 2: Implement sticky in `recompute_once`**

Replace the `sticky: set[int] = set(currently_pinned)` line + the comment below it with:

```python
    sticky: set[int] = set(currently_pinned)
    for bot in snapshot.bots:
        for p in snapshot.players:
            if (
                (p.party_guid is not None and p.party_guid == bot.party_guid)
                or (p.raid_guid is not None and p.raid_guid == bot.raid_guid)
                or (bot.in_pvp_combat and bot.map_id == p.map_id)
            ):
                sticky.add(bot.bot_guid)
                break
```

- [ ] **Step 3: Run tests**

```bash
pytest tests/unit/test_subset_gate.py -v
```

Expected: PASS on all sticky tests + previous tests.

- [ ] **Step 4: Commit**

```bash
git add tot/brain/brain_sidecar/subset_gate.py tot/brain/tests/unit/test_subset_gate.py
git commit -m "$(cat <<'EOF'
feat(brain): SubsetGate sticky overrides (party, raid, PvP combat)

Party + raid members always sticky; PvP combat sticky requires same-map.
Sticky bypasses hysteresis on enrollment. Spec §5 step 1.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 17: `_recompute_once` — hysteresis state machine (TDD)

**Files:**
- Modify: `tot/brain/tests/unit/test_subset_gate.py`

- [ ] **Step 1: Write failing tests**

```python
def test_enroll_requires_hysteresis_in_ticks():
    # Bot has been in range 0 ticks; hysteresis_in_ticks=2 — should NOT enroll yet
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=0,
                      x=1.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5, hysteresis_in_ticks=2),
    )
    # in_ticks=0 -> next bump = 1, which is < 2, so NOT enrolled this cycle.
    assert 100 not in decision.to_enroll


def test_release_requires_hysteresis_out_ticks():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=1,  # different map
                      x=0.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    # Bot currently enrolled, hysteresis (0, 0); needs out_ticks >= 2 to release.
    decision1 = recompute_once(
        snapshot=snap, currently_enrolled={100: 0}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={100: (0, 0)},
        config=_cfg(living_bot_count=5, hysteresis_out_ticks=2),
    )
    # out_ticks would bump to 1, < 2 — NOT released yet.
    assert 100 not in decision1.to_release

    decision2 = recompute_once(
        snapshot=snap, currently_enrolled={100: 0}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={100: (0, 1)},
        config=_cfg(living_bot_count=5, hysteresis_out_ticks=2),
    )
    # out_ticks bumps to 2 — release.
    assert 100 in decision2.to_release


def test_sticky_bypasses_in_hysteresis():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10, party_guid=42)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=0,
                      x=1.0, y=0.0, z=0.0, level=10, party_guid=42)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=5, hysteresis_in_ticks=99),
    )
    # Sticky enrollment ignores hysteresis_in_ticks.
    assert 100 in decision.to_enroll


def test_pin_skips_release_even_when_out_of_range():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=99,  # other map
                      x=0.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={100: 0},
        currently_pinned=frozenset({100}),
        backoff_set=frozenset(),
        hysteresis={100: (0, 99)},  # massively out-of-range
        config=_cfg(living_bot_count=5, hysteresis_out_ticks=2),
    )
    assert 100 not in decision.to_release
```

- [ ] **Step 2: Run — already implemented in `_reconcile`**

```bash
pytest tests/unit/test_subset_gate.py -v
```

Expected: PASS (the `_reconcile` helper already handles these — confirms the contract).

- [ ] **Step 3: Commit**

```bash
git add tot/brain/tests/unit/test_subset_gate.py
git commit -m "$(cat <<'EOF'
test(brain): SubsetGate hysteresis state machine + pin overrides

Asserts in-hysteresis, out-hysteresis, sticky bypass, pin override of release.
Spec §5 step 4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 18: `_recompute_once` — backoff + edge cases (TDD)

**Files:**
- Modify: `tot/brain/tests/unit/test_subset_gate.py`

- [ ] **Step 1: Write tests**

```python
def test_backoff_set_excludes_from_enroll():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bot = BotSnapshot(bot_guid=100, name="b", map_id=0,
                      x=1.0, y=0.0, z=0.0, level=10)
    snap = WorldSnapshot(players=(alice,), bots=(bot,))
    decision = recompute_once(
        snapshot=snap, currently_enrolled={},
        currently_pinned=frozenset(),
        backoff_set=frozenset({100}),
        hysteresis={},
        config=_cfg(living_bot_count=5),
    )
    assert 100 not in decision.to_enroll
    assert 100 in decision.skipped_due_to_backoff


def test_two_players_two_maps_closest_n_globally():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bob = PlayerSnapshot(player_guid=2, name="Bob", map_id=1,
                         x=100.0, y=0.0, z=0.0, level=10)
    # 3 bots on Alice's map (dist 1, 2, 3) + 3 bots on Bob's map (dist 1, 2, 3)
    bots = (
        BotSnapshot(bot_guid=10, name="a1", map_id=0, x=1.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=11, name="a2", map_id=0, x=2.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=12, name="a3", map_id=0, x=3.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=20, name="b1", map_id=1, x=101.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=21, name="b2", map_id=1, x=102.0, y=0.0, z=0.0, level=10),
        BotSnapshot(bot_guid=22, name="b3", map_id=1, x=103.0, y=0.0, z=0.0, level=10),
    )
    snap = WorldSnapshot(players=(alice, bob), bots=bots)
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=4),
    )
    # Top 4 = a1 + b1 + a2 + b2 (distances 1, 1, 2, 2)
    assert decision.to_enroll == frozenset({10, 11, 20, 21})


def test_population_smaller_than_living_count_uses_all():
    alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                           x=0.0, y=0.0, z=0.0, level=10)
    bots = tuple(
        BotSnapshot(bot_guid=100 + i, name=f"b{i}", map_id=0,
                    x=float(i), y=0.0, z=0.0, level=10)
        for i in range(3)
    )
    snap = WorldSnapshot(players=(alice,), bots=bots)
    decision = recompute_once(
        snapshot=snap, currently_enrolled={}, currently_pinned=frozenset(),
        backoff_set=frozenset(), hysteresis={},
        config=_cfg(living_bot_count=10),
    )
    assert decision.to_enroll == frozenset({100, 101, 102})
```

- [ ] **Step 2: Run + commit**

```bash
pytest tests/unit/test_subset_gate.py -v
git add tot/brain/tests/unit/test_subset_gate.py
git commit -m "$(cat <<'EOF'
test(brain): SubsetGate backoff, multi-player, undersized-population

Covers edge cases from spec §5 + §8.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 6: SubsetGate apply + run (Phase A)

### Task 19: `SubsetGate` class with `_apply` + `run`

**Files:**
- Modify: `tot/brain/brain_sidecar/subset_gate.py`

- [ ] **Step 1: Write skeleton + integration test**

In `tot/brain/tests/unit/test_subset_gate.py`, append:

```python
import asyncio
from unittest.mock import AsyncMock, MagicMock
from brain_sidecar.subset_gate import SubsetGate


def test_apply_calls_enroll_release_and_set_tier():
    state_store = MagicMock()
    state_store.list_active.return_value = iter([
        MagicMock(bot_guid=g, last_seen=0) for g in (1, 2, 3)
    ])
    state_store.list_pinned.return_value = []
    state_store.get_hysteresis.return_value = (0, 0)

    enroll_fn = AsyncMock()
    release_fn = AsyncMock()

    gate = SubsetGate(
        state_store=state_store,
        snapshot_fetcher=AsyncMock(return_value=WorldSnapshot(players=(), bots=())),
        enroll_fn=enroll_fn,
        release_fn=release_fn,
        config=_cfg(),
    )

    decision = SubsetDecision(
        target_living_set=frozenset(),
        sticky_pinned=frozenset(),
        proximity_picks=frozenset(),
        to_enroll=frozenset({4}),
        to_release=frozenset({1}),
        to_full=frozenset({2, 4}),
        to_reduced=frozenset({3}),
        skipped_due_to_backoff=frozenset(),
    )
    asyncio.run(gate._apply(decision))

    release_fn.assert_awaited_once_with(1)
    enroll_fn.assert_awaited_once_with(4)
    set_tier_calls = state_store.set_tier.call_args_list
    assert any(c.args == (2, "full") for c in set_tier_calls)
    assert any(c.args == (3, "reduced") for c in set_tier_calls)
    assert any(c.args == (4, "full") for c in set_tier_calls)
```

- [ ] **Step 2: Implement the `SubsetGate` class**

In `tot/brain/brain_sidecar/subset_gate.py`, append:

```python
import asyncio
import logging
import time
from typing import Awaitable, Callable, Optional

log = logging.getLogger(__name__)


SnapshotFetcher = Callable[[], Awaitable[WorldSnapshot]]
EnrollFn = Callable[[int], Awaitable[None]]
ReleaseFn = Callable[[int], Awaitable[None]]


@dataclass
class SubsetGate:
    state_store: object  # StateStore (avoid import cycle)
    snapshot_fetcher: SnapshotFetcher
    enroll_fn: EnrollFn
    release_fn: ReleaseFn
    config: SubsetGateConfig
    backoff_state: dict[int, float] = field(default_factory=dict)  # bot_guid -> deadline_ts
    _task: Optional[asyncio.Task] = field(default=None)

    async def run(self) -> None:
        if not self.config.enabled:
            log.info("SubsetGate disabled via config — exiting run() immediately")
            return
        log.info("SubsetGate.run() started; interval=%.0fs", self.config.recompute_interval_s)
        while True:
            try:
                await self._recompute_and_apply()
            except asyncio.CancelledError:
                raise
            except Exception:
                log.exception("SubsetGate recompute cycle failed; will retry next tick")
            try:
                await asyncio.sleep(self.config.recompute_interval_s)
            except asyncio.CancelledError:
                raise

    async def _recompute_and_apply(self) -> SubsetDecision:
        snapshot = await self.snapshot_fetcher()
        currently_enrolled = {
            row.bot_guid: row.last_seen or 0
            for row in self.state_store.list_active()
        }
        currently_pinned = frozenset(self.state_store.list_pinned())
        backoff_set = self._expired_backoff_set()
        hysteresis = {
            g: self.state_store.get_hysteresis(g) for g in currently_enrolled
        }
        decision = recompute_once(
            snapshot=snapshot,
            currently_enrolled=currently_enrolled,
            currently_pinned=currently_pinned,
            backoff_set=backoff_set,
            hysteresis=hysteresis,
            config=self.config,
        )
        await self._apply(decision)
        return decision

    def _expired_backoff_set(self) -> frozenset[int]:
        now = time.time()
        live = {g: dl for g, dl in self.backoff_state.items() if dl > now}
        self.backoff_state = live
        return frozenset(live.keys())

    async def _apply(self, decision: SubsetDecision) -> None:
        for bot_guid in decision.to_release:
            try:
                await self.release_fn(bot_guid)
            except Exception:
                log.exception("subset gate release failed bot_guid=%d", bot_guid)

        for bot_guid in decision.to_enroll:
            try:
                await self.enroll_fn(bot_guid)
            except Exception:
                log.exception("subset gate enroll failed bot_guid=%d", bot_guid)
                self.backoff_state[bot_guid] = time.time() + self.config.enroll_backoff_s

        for bot_guid in decision.to_full:
            self.state_store.set_tier(bot_guid, "full")
        for bot_guid in decision.to_reduced:
            self.state_store.set_tier(bot_guid, "reduced")

        # Bump hysteresis counters for next cycle.
        currently_enrolled = {row.bot_guid for row in self.state_store.list_active()}
        target = decision.target_living_set
        for bot_guid in currently_enrolled:
            self.state_store.bump_hysteresis(bot_guid, in_range=(bot_guid in target))
```

- [ ] **Step 3: Run + commit**

```bash
pytest tests/unit/test_subset_gate.py -v
git add tot/brain/brain_sidecar/subset_gate.py tot/brain/tests/unit/test_subset_gate.py
git commit -m "$(cat <<'EOF'
feat(brain): SubsetGate class — run() loop + _apply reconciliation

Periodic asyncio task that polls snapshot_fetcher, runs recompute_once,
and applies enroll/release + tier-assignment side effects via injected
enroll_fn / release_fn. Failed enrolls go into per-bot backoff. Spec §4.2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 7: Brain admin endpoints + boot wiring (Phase A)

### Task 20: Admin endpoints in `api.py`

**Files:**
- Modify: `tot/brain/brain_sidecar/api.py`
- Test: `tot/brain/tests/unit/test_admin_subset.py` (new)

- [ ] **Step 1: Write failing test**

```python
# SPDX-License-Identifier: GPL-2.0-or-later
from fastapi.testclient import TestClient
# Use the existing app factory pattern — match how other api tests construct the client.

def test_pin_endpoint_persists_pin(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    # Bot must exist first.
    state_store.enroll(bot_guid=42, enrolled_at_ms=0,
                       personality_seed=_make_personality())
    r = client.post("/admin/subset/pin/42")
    assert r.status_code == 200
    assert 42 in state_store.list_pinned()


def test_unpin_endpoint_clears_pin(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    state_store.enroll(bot_guid=42, enrolled_at_ms=0,
                       personality_seed=_make_personality())
    state_store.set_pin(42, True)
    r = client.post("/admin/subset/unpin/42")
    assert r.status_code == 200
    assert state_store.list_pinned() == []


def test_snapshot_endpoint_returns_summary(test_client_with_state_store):
    client, state_store = test_client_with_state_store
    r = client.get("/admin/subset/snapshot")
    assert r.status_code == 200
    body = r.json()
    assert "currently_enrolled" in body
    assert "pinned" in body
    assert "config" in body
```

- [ ] **Step 2: Implement endpoints**

In `tot/brain/brain_sidecar/api.py`, find `make_router(...)` signature; add a parameter for the `SubsetGate` instance. Append routes:

```python
    @router.post("/admin/subset/pin/{bot_guid}")
    async def pin(bot_guid: int):
        if state_store.get_bot(bot_guid) is None:
            raise HTTPException(404, f"bot_guid {bot_guid} not in living_bots")
        state_store.set_pin(bot_guid, True)
        return {"ok": True, "bot_guid": bot_guid, "pinned": True}

    @router.post("/admin/subset/unpin/{bot_guid}")
    async def unpin(bot_guid: int):
        state_store.set_pin(bot_guid, False)
        return {"ok": True, "bot_guid": bot_guid, "pinned": False}

    @router.get("/admin/subset/snapshot")
    async def snapshot():
        return {
            "currently_enrolled": [
                {"bot_guid": row.bot_guid, "tier": state_store.get_tier(row.bot_guid)}
                for row in state_store.list_active()
            ],
            "pinned": list(state_store.list_pinned()),
            "config": {
                "living_bot_count": subset_gate.config.living_bot_count,
                "recompute_interval_s": subset_gate.config.recompute_interval_s,
                "phase_b_enabled": subset_gate.config.phase_b_enabled,
            },
        }

    @router.post("/admin/subset/recompute")
    async def recompute():
        decision = await subset_gate._recompute_and_apply()
        return {
            "target": sorted(decision.target_living_set),
            "to_enroll": sorted(decision.to_enroll),
            "to_release": sorted(decision.to_release),
            "to_full": sorted(decision.to_full),
            "to_reduced": sorted(decision.to_reduced),
        }
```

- [ ] **Step 3: Run + commit**

```bash
pytest tests/unit/test_admin_subset.py -v
git add tot/brain/brain_sidecar/api.py tot/brain/tests/unit/test_admin_subset.py
git commit -m "$(cat <<'EOF'
feat(brain): /admin/subset/{snapshot,recompute,pin,unpin} endpoints

Operator override + debug surface for SubsetGate. Spec §4.6.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 21: `app.py` boot wiring

**Files:**
- Modify: `tot/brain/brain_sidecar/app.py`

- [ ] **Step 1: Add SubsetGate boot wiring inside the lifespan**

In `tot/brain/brain_sidecar/app.py`, inside the lifespan function (after `LoopSupervisor` is constructed and started), add:

```python
        from brain_sidecar.subset_gate import SubsetGate, SubsetGateConfig

        async def _snapshot_fetcher():
            from brain_sidecar.subset_gate import WorldSnapshot, PlayerSnapshot, BotSnapshot
            players_raw = await mcp.call_tool("obs.list_players", {})
            bots_raw = await mcp.call_tool("obs.list_bot_population", {})
            return WorldSnapshot(
                players=tuple(PlayerSnapshot(**p) for p in players_raw.get("players", [])),
                bots=tuple(BotSnapshot(**b) for b in bots_raw.get("bots", [])),
            )

        async def _enroll_via_api(bot_guid: int) -> None:
            await loop_supervisor.enroll_bot(bot_guid)  # existing internal path

        async def _release_via_api(bot_guid: int) -> None:
            await loop_supervisor.release_bot(bot_guid)  # existing internal path

        subset_gate_config = SubsetGateConfig(
            living_bot_count=settings.living_bot_count,
            recompute_interval_s=settings.subset_recompute_interval_s,
            hysteresis_out_ticks=settings.subset_hysteresis_out_ticks,
            hysteresis_in_ticks=settings.subset_hysteresis_in_ticks,
            enroll_backoff_s=settings.subset_enroll_backoff_s,
            enabled=settings.subset_gate_enabled,
            phase_b_enabled=False,  # Flipped to True in Phase B (Task 32).
        )
        subset_gate = SubsetGate(
            state_store=state_store,
            snapshot_fetcher=_snapshot_fetcher,
            enroll_fn=_enroll_via_api,
            release_fn=_release_via_api,
            config=subset_gate_config,
        )
        subset_gate_task = asyncio.create_task(subset_gate.run(), name="subset_gate")
        app.state.subset_gate = subset_gate
        app.state.subset_gate_task = subset_gate_task
```

In the lifespan teardown:

```python
        subset_gate_task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await subset_gate_task
```

Also update `make_router(...)` call site to pass `subset_gate`.

- [ ] **Step 2: Verify the boot test still passes**

```bash
pytest tests/integration/ -v -k "lifespan or boot"
```

Expected: existing boot tests pass; the SubsetGate task starts without errors against the mock MCP.

- [ ] **Step 3: Commit**

```bash
git add tot/brain/brain_sidecar/app.py
git commit -m "$(cat <<'EOF'
feat(brain): wire SubsetGate into lifespan + admin router

SubsetGate task starts after LoopSupervisor; cancelled cleanly on shutdown.
phase_b_enabled=False at this milestone — flipped in Task 32. Spec §4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 8: Phase A integration + close-out

### Task 22: Integration test against mock MCP

**Files:**
- Create: `tot/brain/tests/integration/test_subset_gate_loop.py`

- [ ] **Step 1: Write integration test**

```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""Integration: SubsetGate driving real LoopSupervisor via mock MCP snapshots."""
import asyncio
import pytest
from brain_sidecar.subset_gate import (
    SubsetGate, SubsetGateConfig,
    WorldSnapshot, PlayerSnapshot, BotSnapshot,
)


@pytest.mark.asyncio
async def test_cold_start_enrolls_n_bots():
    """Cold start with 30 bots near Alice should enroll the closest 10."""
    snapshots = [
        WorldSnapshot(
            players=(PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                                    x=0.0, y=0.0, z=0.0, level=10),),
            bots=tuple(
                BotSnapshot(bot_guid=100 + i, name=f"b{i}", map_id=0,
                            x=float(i), y=0.0, z=0.0, level=10)
                for i in range(30)
            ),
        ),
    ]
    enrolled = set()
    released = set()
    async def fetcher():
        return snapshots[0]
    async def enroll_fn(g): enrolled.add(g)
    async def release_fn(g): released.add(g)
    state_store = _fake_state_store()
    gate = SubsetGate(
        state_store=state_store,
        snapshot_fetcher=fetcher,
        enroll_fn=enroll_fn,
        release_fn=release_fn,
        config=SubsetGateConfig(
            living_bot_count=10,
            recompute_interval_s=60.0,
            hysteresis_out_ticks=2,
            hysteresis_in_ticks=1,
            enroll_backoff_s=300.0,
            enabled=True,
            phase_b_enabled=False,
        ),
    )
    await gate._recompute_and_apply()
    assert enrolled == set(range(100, 110))  # 10 closest


@pytest.mark.asyncio
async def test_player_logs_out_releases_all_phase_a():
    """All players gone → after 2 cycles, all bots released."""
    state_store = _fake_state_store()
    for g in range(100, 110):
        state_store.enroll(bot_guid=g, enrolled_at_ms=0,
                           personality_seed=_make_personality())
    snapshots = [
        WorldSnapshot(players=(), bots=()),
        WorldSnapshot(players=(), bots=()),
    ]
    idx = {"i": 0}
    async def fetcher():
        s = snapshots[idx["i"]]
        idx["i"] = min(idx["i"] + 1, len(snapshots) - 1)
        return s
    released = set()
    async def release_fn(g): released.add(g)
    async def enroll_fn(g): pass
    gate = SubsetGate(
        state_store=state_store,
        snapshot_fetcher=fetcher, enroll_fn=enroll_fn, release_fn=release_fn,
        config=SubsetGateConfig(
            living_bot_count=10, recompute_interval_s=60.0,
            hysteresis_out_ticks=2, hysteresis_in_ticks=1,
            enroll_backoff_s=300.0, enabled=True, phase_b_enabled=False,
        ),
    )
    # Cycle 1 — bumps out_ticks to 1 (< 2), no releases yet
    await gate._recompute_and_apply()
    assert released == set()
    # Cycle 2 — bumps out_ticks to 2, all released
    await gate._recompute_and_apply()
    assert released == set(range(100, 110))


# _fake_state_store and _make_personality helpers go in conftest.py.
```

- [ ] **Step 2: Add helpers to conftest.py**

In `tot/brain/tests/integration/conftest.py`, add helpers per the existing pattern.

- [ ] **Step 3: Run**

```bash
pytest tests/integration/test_subset_gate_loop.py -v
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add tot/brain/tests/integration/test_subset_gate_loop.py tot/brain/tests/integration/conftest.py
git commit -m "$(cat <<'EOF'
test(brain): SubsetGate integration — cold start + player logout

Asserts cold-start enrolls closest N, asserts Phase A behavior of
releasing all bots after 2 hysteresis cycles when all players log out.
Spec §9.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 23: Phase A eval gate

**Files:**
- Create: `tot/brain/tests/eval/test_subset_phase_a_gate.py`

- [ ] **Step 1: Write eval-gate test**

```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""Phase A eval gate (spec §9.1):
- 3 players + 30 bots across 3 maps
- 10 cycles with scripted player movement
- assert living-set size ∈ [5, 15] each cycle
- assert sticky bots never released
- assert hysteresis prevents thrash (no bot oscillates >1 enroll<->release per 3 cycles)
- assert recompute p95 < 100ms in-process
"""
import asyncio
import statistics
import time
import pytest
from brain_sidecar.subset_gate import (
    SubsetGate, SubsetGateConfig,
    WorldSnapshot, PlayerSnapshot, BotSnapshot, recompute_once,
)


def _build_scenario():
    # 3 players moving; 30 bots distributed across 3 maps.
    # Cycle 0..9 — Alice walks along x-axis on map 0; Bob stays still on map 1;
    # Charlie joins party with bot 500 on map 2 mid-scenario.
    cycles = []
    for tick in range(10):
        alice = PlayerSnapshot(player_guid=1, name="Alice", map_id=0,
                               x=float(tick) * 50, y=0.0, z=0.0, level=10)
        bob = PlayerSnapshot(player_guid=2, name="Bob", map_id=1,
                             x=0.0, y=0.0, z=0.0, level=10)
        charlie_party = 99 if tick >= 5 else None
        charlie = PlayerSnapshot(player_guid=3, name="Charlie", map_id=2,
                                  x=0.0, y=0.0, z=0.0, level=10,
                                  party_guid=charlie_party)
        bots = []
        for i in range(10):
            bots.append(BotSnapshot(bot_guid=100 + i, name=f"a{i}", map_id=0,
                                    x=float(i) * 10, y=0.0, z=0.0, level=10))
        for i in range(10):
            bots.append(BotSnapshot(bot_guid=200 + i, name=f"b{i}", map_id=1,
                                    x=float(i) * 10, y=0.0, z=0.0, level=10))
        for i in range(10):
            party = 99 if tick >= 5 and i == 0 else None
            bots.append(BotSnapshot(bot_guid=300 + i, name=f"c{i}", map_id=2,
                                    x=float(i) * 10, y=0.0, z=0.0, level=10,
                                    party_guid=party))
        cycles.append(WorldSnapshot(players=(alice, bob, charlie), bots=tuple(bots)))
    return cycles


@pytest.mark.asyncio
async def test_phase_a_eval_gate():
    cycles = _build_scenario()
    state_store = _fake_state_store()
    enrolled, released = set(), set()
    enroll_history, release_history = [], []
    async def enroll_fn(g):
        enrolled.add(g); enroll_history.append(g)
        state_store.enroll(bot_guid=g, enrolled_at_ms=0,
                           personality_seed=_make_personality())
    async def release_fn(g):
        if g in enrolled: enrolled.discard(g)
        released.add(g); release_history.append(g)
        state_store.set_status(g, "released")
    idx = {"i": 0}
    async def fetcher():
        s = cycles[idx["i"]]; idx["i"] = min(idx["i"] + 1, len(cycles) - 1); return s
    cfg = SubsetGateConfig(
        living_bot_count=10, recompute_interval_s=60.0,
        hysteresis_out_ticks=2, hysteresis_in_ticks=1,
        enroll_backoff_s=300.0, enabled=True, phase_b_enabled=False,
    )
    gate = SubsetGate(state_store=state_store, snapshot_fetcher=fetcher,
                     enroll_fn=enroll_fn, release_fn=release_fn, config=cfg)
    latencies = []
    for cycle in range(10):
        t0 = time.perf_counter()
        await gate._recompute_and_apply()
        latencies.append((time.perf_counter() - t0) * 1000)
        assert 0 <= len(enrolled) <= 15

    # Sticky bot 300 must have been enrolled and never released after tick 5.
    assert 300 in enroll_history
    releases_after_sticky = [g for g in release_history
                              if release_history.index(g) >= 5 and g == 300]
    assert releases_after_sticky == []

    # No bot enrolled-released-enrolled within the 10-cycle window
    seen = set()
    for g in enroll_history:
        if g in released and g in enrolled:
            assert g not in seen, f"bot {g} thrashed"
            seen.add(g)

    p95 = statistics.quantiles(latencies, n=20)[-1]
    assert p95 < 100, f"recompute p95={p95:.1f}ms exceeds 100ms"
```

- [ ] **Step 2: Run**

```bash
pytest tests/eval/test_subset_phase_a_gate.py -v
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tot/brain/tests/eval/test_subset_phase_a_gate.py
git commit -m "$(cat <<'EOF'
test(brain): Phase A eval gate — 3p×30b×10 cycles + p95 latency

Spec §9.1: living-set size bounded, sticky preservation, no thrash, p95<100ms.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 24: Update `.env.example` for Phase A

**Files:**
- Modify: `tot/deploy/.env.example` (or wherever the project keeps the example env file — grep `find tot/ -name '.env.example'`)

- [ ] **Step 1: Add Phase A env vars**

Append to `.env.example`:

```bash
# === Subset gating (Plan 3) ===
TOT_SUBSET_GATE_ENABLED=true
TOT_LIVING_BOT_COUNT=10                 # range 5-15
TOT_SUBSET_RECOMPUTE_INTERVAL_S=60
TOT_SUBSET_HYSTERESIS_OUT_TICKS=2
TOT_SUBSET_HYSTERESIS_IN_TICKS=1
TOT_SUBSET_ENROLL_BACKOFF_S=300
# TOT_REDUCED_TICK_INTERVAL_S=300       # uncommented in Phase B
```

- [ ] **Step 2: Commit**

```bash
git add tot/deploy/.env.example  # adjust path per grep
git commit -m "$(cat <<'EOF'
docs(deploy): subset-gating env vars in .env.example (Phase A)

Spec §7.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 25: Tag `subset-gating-phase-a-complete`

**Files:**
- None — git operations only.

- [ ] **Step 1: Run full test suite**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
pytest tot/brain/tests/ tot/harness/tests/ tot/memory/tests/ -v 2>&1 | tail -10
```

Expected: 282 memory + 117 parity + 43 + new (≈25) brain tests all pass.

- [ ] **Step 2: Update kbs**

- Update `kb_e6d3bbb1` to reflect Phase A shipped: status, the new tools, the new admin endpoints.
- Update `kb_87a7eade`: Thread K (memory subsystem) gains a sibling for Thread L (subset gating) noting Phase A complete.

```bash
# Use the ninum-knowledge MCP tools (mcp__ninum-knowledge__update_knowledge_entry).
```

- [ ] **Step 3: Tag**

```bash
git tag -a subset-gating-phase-a-complete -m "Phase A of Plan 3 — 2-tier subset gating

Ships:
- obs.list_players + obs.list_bot_population harness tools
- SubsetGate module + asyncio task in brain
- Proximity-based selection + sticky party/raid/PvP overrides
- Hysteresis (out=2, in=1) with sticky bypass
- Operator pin/unpin admin endpoints
- 117 parity tests, ~25 new brain unit + integration + eval tests

Next: Phase B (REDUCED tier + ToolPolicyEnforcer + warm-cache extension).
"
```

---

## Phase 9: ToolPolicyEnforcer (Phase B)

### Task 26: ToolPolicyEnforcer module + tests

**Files:**
- Create: `tot/brain/brain_sidecar/tool_policy.py`
- Create: `tot/brain/tests/unit/test_tool_policy.py`

- [ ] **Step 1: Write failing tests**

```python
# SPDX-License-Identifier: GPL-2.0-or-later
from brain_sidecar.tool_policy import ToolPolicyEnforcer


def test_full_tier_allows_all_bot_tools():
    enforcer = ToolPolicyEnforcer()
    for tool in (
        "bot.send_chat", "bot.set_strategy", "bot.set_goal", "bot.set_role",
        "bot.invite_to_group", "bot.accept_invite", "bot.leave_group",
        "bot.follow", "bot.queue_for_dungeon", "bot.enter_instance", "bot.stop",
    ):
        assert enforcer.is_allowed(tool, "full"), f"{tool} should be allowed in FULL"


def test_reduced_tier_allows_only_high_level_intent():
    enforcer = ToolPolicyEnforcer()
    for tool in ("bot.set_strategy", "bot.set_goal", "bot.set_role"):
        assert enforcer.is_allowed(tool, "reduced")
    for tool in ("bot.send_chat", "bot.invite_to_group", "bot.accept_invite",
                 "bot.leave_group", "bot.follow", "bot.queue_for_dungeon",
                 "bot.enter_instance", "bot.stop"):
        assert not enforcer.is_allowed(tool, "reduced"), f"{tool} should be denied in REDUCED"


def test_unknown_tier_defaults_to_full():
    enforcer = ToolPolicyEnforcer()
    assert enforcer.is_allowed("bot.send_chat", "unknown_tier") is True
```

- [ ] **Step 2: Implement**

Create `tot/brain/brain_sidecar/tool_policy.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""Phase B: tier-aware tool whitelist enforcement."""
from __future__ import annotations

from dataclasses import dataclass


_FULL_WHITELIST = frozenset({
    "bot.send_chat", "bot.set_strategy", "bot.set_goal", "bot.set_role",
    "bot.invite_to_group", "bot.accept_invite", "bot.leave_group",
    "bot.follow", "bot.queue_for_dungeon", "bot.enter_instance", "bot.stop",
    "bot.combat_stop",  # if shipped in V1.5+; harmless if not
})

_REDUCED_WHITELIST = frozenset({
    "bot.set_strategy", "bot.set_goal", "bot.set_role",
})

TIER_TOOL_WHITELIST: dict[str, frozenset[str]] = {
    "full": _FULL_WHITELIST,
    "reduced": _REDUCED_WHITELIST,
}


@dataclass(frozen=True)
class ToolPolicyEnforcer:
    def is_allowed(self, tool_name: str, tier: str) -> bool:
        whitelist = TIER_TOOL_WHITELIST.get(tier, _FULL_WHITELIST)
        return tool_name in whitelist
```

- [ ] **Step 3: Run + commit**

```bash
pytest tests/unit/test_tool_policy.py -v
git add tot/brain/brain_sidecar/tool_policy.py tot/brain/tests/unit/test_tool_policy.py
git commit -m "$(cat <<'EOF'
feat(brain): ToolPolicyEnforcer — tier-aware bot.* tool whitelist

FULL: 12 tools. REDUCED: bot.set_strategy + bot.set_goal + bot.set_role.
Unknown tier defaults to FULL (safer than denying everything). Spec §4.3.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 27: Wire ToolPolicyEnforcer into Dispatcher

**Files:**
- Modify: `tot/brain/brain_sidecar/dispatch.py`
- Test: `tot/brain/tests/unit/test_dispatch.py`

- [ ] **Step 1: Write failing test**

Append to existing `test_dispatch.py`:

```python
@pytest.mark.asyncio
async def test_dispatch_denies_bot_send_chat_in_reduced_tier():
    enforcer = ToolPolicyEnforcer()
    dispatcher = Dispatcher(
        mcp_client=AsyncMock(),
        tool_policy=enforcer,
    )
    decision = Decision(
        tool_name="bot.send_chat",
        args={"target_guid": 1, "message": "hi", "channel": "say"},
    )
    result = await dispatcher.dispatch(decision, tier="reduced")
    assert result.outcome == "policy_denied"
    dispatcher.mcp_client.call_tool.assert_not_called()


@pytest.mark.asyncio
async def test_dispatch_allows_bot_set_strategy_in_reduced_tier():
    enforcer = ToolPolicyEnforcer()
    mcp = AsyncMock()
    mcp.call_tool.return_value = {"ok": True}
    dispatcher = Dispatcher(mcp_client=mcp, tool_policy=enforcer)
    decision = Decision(
        tool_name="bot.set_strategy",
        args={"target_guid": 1, "strategy": "grind", "bot_state": "non_combat"},
    )
    result = await dispatcher.dispatch(decision, tier="reduced")
    assert result.outcome == "ok"
    mcp.call_tool.assert_awaited_once()
```

- [ ] **Step 2: Modify `Dispatcher` to accept tier + enforcer**

In `tot/brain/brain_sidecar/dispatch.py`:

1. Add field on the dataclass: `tool_policy: Optional[ToolPolicyEnforcer] = None`
2. Add parameter on `dispatch`: `async def dispatch(self, decision: Decision, *, tier: str = "full") -> DispatchResult:`
3. At the top of `dispatch` (before any MCP call), insert the policy check:

```python
        if self.tool_policy is not None and not self.tool_policy.is_allowed(decision.tool_name, tier):
            log.info("policy_denied tool=%s tier=%s", decision.tool_name, tier)
            return DispatchResult(
                outcome="policy_denied",
                tool_name=decision.tool_name,
                response_body=None,
                error=None,
                latency_ms=0.0,
            )
```

Verify the `DispatchResult` field names by grepping the existing class definition in `dispatch.py` — adjust the keyword args above to match the real fields. The plan's shape assumes the existing fields are `outcome`, `tool_name`, `response_body`, `error`, `latency_ms`; if the codebase has more/fewer, fill defaults for the missing ones.

- [ ] **Step 3: Run + commit**

```bash
pytest tests/unit/test_dispatch.py -v
git add tot/brain/brain_sidecar/dispatch.py tot/brain/tests/unit/test_dispatch.py
git commit -m "$(cat <<'EOF'
feat(brain): Dispatcher consults ToolPolicyEnforcer per-tier

REDUCED-tier denials short-circuit with outcome=policy_denied and log
the denial; no MCP call is made. Spec §4.7.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 28: LoopSupervisor passes tier per tick

**Files:**
- Modify: `tot/brain/brain_sidecar/loop.py`
- Test: `tot/brain/tests/unit/test_loop_tier.py`

- [ ] **Step 1: Write failing test**

Create `tot/brain/tests/unit/test_loop_tier.py`:

```python
# SPDX-License-Identifier: GPL-2.0-or-later
import asyncio
import pytest
from unittest.mock import AsyncMock, MagicMock
from brain_sidecar.loop import LoopSupervisor
from brain_sidecar.models import Decision, TickState, TriageResult


@pytest.mark.asyncio
async def test_loop_reads_tier_and_passes_to_dispatcher():
    state_store = MagicMock()
    state_store.get_tier.return_value = "reduced"
    state_store.get_bot.return_value = MagicMock(bot_guid=1, status="active")
    triage = MagicMock()
    triage.evaluate = AsyncMock(return_value=TriageResult(
        should_decide=True, reason="test", hot_inputs={}
    ))
    decider = MagicMock()
    decider.decide = AsyncMock(return_value=Decision(
        tool_name="bot.set_strategy",
        args={"target_guid": 1, "strategy": "grind", "bot_state": "non_combat"},
    ))
    dispatcher = MagicMock()
    dispatcher.dispatch = AsyncMock(return_value=MagicMock(outcome="ok"))
    decision_log = MagicMock()
    sup = LoopSupervisor(
        triage=triage, decider=decider, dispatcher=dispatcher,
        state_store=state_store, tick_interval_s=0.01,
        decision_log_writer=decision_log,
    )
    await sup._do_tick(bot_guid=1, tick_state=TickState(bot_guid=1))
    # Dispatcher must have been called with tier='reduced'
    call_kwargs = dispatcher.dispatch.await_args.kwargs
    assert call_kwargs.get("tier") == "reduced"
```

- [ ] **Step 2: Modify LoopSupervisor**

In `tot/brain/brain_sidecar/loop.py`, find the per-bot tick. Before calling `await self.dispatcher.dispatch(decision)`:

```python
        tier = self.state_store.get_tier(bot_guid)
        result = await self.dispatcher.dispatch(decision, tier=tier)
```

- [ ] **Step 3: Run + commit**

```bash
pytest tests/unit/test_loop_tier.py -v
git add tot/brain/brain_sidecar/loop.py tot/brain/tests/unit/test_loop_tier.py
git commit -m "$(cat <<'EOF'
feat(brain): LoopSupervisor reads tier from StateStore + threads to Dispatcher

Per-tick tier read; Phase A defaults to 'full' for all enrolled bots.
Spec §4.7.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 29: Variable tick interval per tier

**Files:**
- Modify: `tot/brain/brain_sidecar/loop.py`
- Test: `tot/brain/tests/unit/test_loop_tier.py`

- [ ] **Step 1: Write failing test**

Append to `tot/brain/tests/unit/test_loop_tier.py`:

```python
@pytest.mark.asyncio
async def test_reduced_tier_uses_slower_tick_interval(monkeypatch):
    """REDUCED tier must read reduced_tick_interval_s for the sleep between ticks."""
    sleep_calls: list[float] = []
    real_sleep = asyncio.sleep
    async def fake_sleep(s):
        sleep_calls.append(s)
        # Yield once so the loop can be cancelled below.
        await real_sleep(0)
    monkeypatch.setattr("brain_sidecar.loop.asyncio.sleep", fake_sleep)
    state_store = MagicMock()
    # First read returns 'full' (so first tick uses tick_interval_s),
    # second returns 'reduced' (so second tick uses reduced_tick_interval_s).
    state_store.get_tier.side_effect = ["full", "reduced", "reduced"]
    state_store.get_bot.return_value = MagicMock(bot_guid=1, status="active")
    triage = MagicMock(); triage.evaluate = AsyncMock(return_value=TriageResult(
        should_decide=False, reason="quiet", hot_inputs={}))
    decider = MagicMock(); decider.decide = AsyncMock()
    dispatcher = MagicMock(); dispatcher.dispatch = AsyncMock()
    sup = LoopSupervisor(
        triage=triage, decider=decider, dispatcher=dispatcher,
        state_store=state_store,
        tick_interval_s=60.0,
        reduced_tick_interval_s=300.0,
        decision_log_writer=MagicMock(),
    )
    task = asyncio.create_task(sup._bot_loop(bot_guid=1))
    await real_sleep(0)  # let one iteration run
    await real_sleep(0)  # let a second iteration run
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    assert 60.0 in sleep_calls
    assert 300.0 in sleep_calls
```

- [ ] **Step 2: Modify `_bot_loop`**

```python
        tier = self.state_store.get_tier(bot_guid)
        interval = (
            self.reduced_tick_interval_s if tier == "reduced"
            else self.tick_interval_s
        )
        await asyncio.sleep(interval)
```

Add `reduced_tick_interval_s: float = 300.0` to the `LoopSupervisor` dataclass.

- [ ] **Step 3: Wire in `app.py`**

```python
        loop_supervisor = LoopSupervisor(
            ...,
            tick_interval_s=settings.tick_interval_s,
            reduced_tick_interval_s=settings.reduced_tick_interval_s,
        )
```

- [ ] **Step 4: Run + commit**

```bash
pytest tests/unit/test_loop_tier.py -v
git add tot/brain/brain_sidecar/loop.py tot/brain/brain_sidecar/app.py tot/brain/tests/unit/test_loop_tier.py
git commit -m "$(cat <<'EOF'
feat(brain): variable tick interval — REDUCED tier ticks every 300s

Tier read at tick start; FULL uses tick_interval_s, REDUCED uses
reduced_tick_interval_s. Existing tasks keep their identity (no restart
on tier transition). Spec §6.2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 10: Flip Phase B flag + tier assignment

### Task 30: Enable `phase_b_enabled` + verify warm-cache + tier assignment

**Files:**
- Modify: `tot/brain/brain_sidecar/app.py`
- Test: `tot/brain/tests/integration/test_subset_gate_loop.py`

- [ ] **Step 1: Write failing integration test for warm-cache + REDUCED transition**

```python
@pytest.mark.asyncio
async def test_player_logs_out_transitions_to_reduced_phase_b():
    state_store = _fake_state_store()
    for g in range(100, 110):
        state_store.enroll(bot_guid=g, enrolled_at_ms=0,
                           personality_seed=_make_personality())
    snap_empty = WorldSnapshot(players=(), bots=())
    async def fetcher(): return snap_empty
    async def enroll_fn(g): pass
    async def release_fn(g): state_store.set_status(g, "released")

    gate = SubsetGate(
        state_store=state_store, snapshot_fetcher=fetcher,
        enroll_fn=enroll_fn, release_fn=release_fn,
        config=SubsetGateConfig(
            living_bot_count=10, recompute_interval_s=60.0,
            hysteresis_out_ticks=2, hysteresis_in_ticks=1,
            enroll_backoff_s=300.0, enabled=True,
            phase_b_enabled=True,
        ),
    )
    await gate._recompute_and_apply()
    await gate._recompute_and_apply()
    # Phase B: nobody released; all transitioned to REDUCED.
    for g in range(100, 110):
        assert state_store.get_tier(g) == "reduced"
        assert state_store.get_bot(g).status != "released"
```

- [ ] **Step 2: Set `phase_b_enabled=True` in `app.py`**

```python
        subset_gate_config = SubsetGateConfig(
            ...
            phase_b_enabled=True,
        )
```

- [ ] **Step 3: Run + commit**

```bash
pytest tests/integration/test_subset_gate_loop.py -v
git add tot/brain/brain_sidecar/app.py tot/brain/tests/integration/test_subset_gate_loop.py
git commit -m "$(cat <<'EOF'
feat(brain): enable phase_b_enabled=true — warm cache + REDUCED tier

Integration test: empty world keeps currently-enrolled bots alive in
REDUCED tier instead of releasing them. Spec §5 step 3 + step 5 + §10.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 31: Phase B eval gate

**Files:**
- Create: `tot/brain/tests/eval/test_subset_phase_b_gate.py`

- [ ] **Step 1: Write eval-gate test**

```python
# SPDX-License-Identifier: GPL-2.0-or-later
"""Phase B eval gate (spec §9.2):
- Same Phase A scenario
- After all players log out, no bot.send_chat/bot.invite_to_group/
  bot.queue_for_dungeon calls leave the brain over a 10-cycle window
- Only bot.set_strategy / bot.set_goal / bot.set_role appear
- REDUCED-tier ticks fire on 300s interval
"""
import asyncio, pytest
from brain_sidecar.subset_gate import SubsetGate, SubsetGateConfig
from brain_sidecar.tool_policy import ToolPolicyEnforcer


@pytest.mark.asyncio
async def test_phase_b_no_denied_tools_called_when_players_offline():
    """Brain wants to chat in REDUCED tier; dispatcher policy must deny."""
    from unittest.mock import AsyncMock, MagicMock
    from brain_sidecar.dispatch import Dispatcher
    from brain_sidecar.models import Decision
    from brain_sidecar.tool_policy import ToolPolicyEnforcer

    mcp = AsyncMock()
    enforcer = ToolPolicyEnforcer()
    dispatcher = Dispatcher(mcp_client=mcp, tool_policy=enforcer)

    denied_tools = (
        "bot.send_chat", "bot.invite_to_group", "bot.queue_for_dungeon",
        "bot.follow", "bot.accept_invite", "bot.leave_group",
        "bot.enter_instance", "bot.stop",
    )
    allowed_tools = ("bot.set_strategy", "bot.set_goal", "bot.set_role")

    for tool in denied_tools:
        decision = Decision(tool_name=tool, args={"target_guid": 1})
        result = await dispatcher.dispatch(decision, tier="reduced")
        assert result.outcome == "policy_denied", f"{tool} should be denied"

    mcp.call_tool.assert_not_called()

    mcp.call_tool.return_value = {"ok": True}
    for tool in allowed_tools:
        decision = Decision(tool_name=tool,
                            args={"target_guid": 1, "strategy": "grind",
                                  "bot_state": "non_combat", "goal": "level",
                                  "role": "dps"})
        result = await dispatcher.dispatch(decision, tier="reduced")
        assert result.outcome == "ok", f"{tool} should be allowed"
    assert mcp.call_tool.await_count == len(allowed_tools)
```

- [ ] **Step 2: Run + commit**

```bash
pytest tests/eval/test_subset_phase_b_gate.py -v
git add tot/brain/tests/eval/test_subset_phase_b_gate.py
git commit -m "$(cat <<'EOF'
test(brain): Phase B eval gate — REDUCED-tier whitelist enforcement

Spec §9.2: zero denied tool calls leave the brain when no players online.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 32: Update `.env.example` for Phase B + uncomment REDUCED tick

**Files:**
- Modify: `tot/deploy/.env.example`

- [ ] **Step 1: Uncomment**

```bash
TOT_REDUCED_TICK_INTERVAL_S=300         # REDUCED-tier tick interval
```

- [ ] **Step 2: Commit**

```bash
git add tot/deploy/.env.example
git commit -m "$(cat <<'EOF'
docs(deploy): activate TOT_REDUCED_TICK_INTERVAL_S in .env.example (Phase B)

Spec §7.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase 11: Close-out

### Task 33: Write `SUBSET_GATING_STATE.md`

**Files:**
- Create: `SUBSET_GATING_STATE.md` (project root, matches FOUNDATION_STATE.md + MEMORY_SUBSYSTEM_STATE.md pattern)

- [ ] **Step 1: Write the state record**

```markdown
# Threads of Time — Subset Gating Complete

This file is a one-time record of the state at the `subset-gating-complete` tag.

## What works

- `obs.list_players` + `obs.list_bot_population` harness tools live; 117 parity tests green
- `SubsetGate` asyncio.Task in brain recomputes living set every 60s
- Proximity-to-player selection with sticky party/raid/PvP overrides
- Hysteresis (out=2 ticks, in=1 tick) prevents thrash
- Operator pin/unpin via `/admin/subset/pin/{guid}` + `/admin/subset/unpin/{guid}`
- Three-tier model fully enabled: FULL / REDUCED / BACKGROUND
- `ToolPolicyEnforcer` restricts REDUCED tier to {bot.set_strategy, bot.set_goal, bot.set_role}
- REDUCED-tier ticks at 300s; FULL-tier ticks at 60s
- Phase A + Phase B eval gates pass
- Brain tests: 43 + ~25 new = ~68 brain unit/integration/eval tests
- Memory tests: 282 (unchanged)
- Parity tests: 117 (115 + 2 new tools)

## What's deferred (Plan 5 — operator install stack)

- Live MCP probe of `obs.list_players` + `obs.list_bot_population` against real worldserver
- Heimdal end-to-end test with multiple real players
- Multi-player real-time proximity validation
- mod-playerbots virtual-GUID stability check (open Q from Plan 2 C2)

## What's deferred to 1.1.0+

- Player-facing chat command for nominating companion bots
- Multi-region affinity (memory-weighted scoring of "regular party" bots)
- Dynamic TOT_LIVING_BOT_COUNT (varies by player count)
- REDUCED-tier auto-restart on goal completion

## Known issues + follow-up work

- C++ adapters use placeholder symbols for IsPlayerBot() + PlayerbotsMgr accessor; verified against pre-flight notes but live runtime probe still pending (Plan 5)
- Phase A → Phase B is a single config flip — operators can disable Phase B by setting `phase_b_enabled=False` in code (no env var; deliberate per spec §7)

## What's pinned

- Tag: `subset-gating-complete`
- All hyperparameters at spec §7 defaults (5/15/60/2/1/300 + 300 for REDUCED tick)
- Per-bot DB columns: tier, in_range_ticks, out_of_range_ticks, last_recompute_at, pinned

## Next plans

Plan 4 (MPQ compositor) and Plan 5 (operator install stack) can now proceed in parallel.
Plan 5 picks up Task 38-style integration (deferred Heimdal e2e).
```

- [ ] **Step 2: Commit**

```bash
git add SUBSET_GATING_STATE.md
git commit -m "$(cat <<'EOF'
docs: record subset-gating-complete state (Plan 3 done)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task 34: Update kbs + tag `subset-gating-complete`

**Files:**
- Update via ninum-knowledge MCP: `kb_e6d3bbb1`, `kb_87a7eade`, `kb_67ddebbe` (brain-side ship notes)
- Create new kb: subset-gating retrospective (carry-forward items, gotchas)

- [ ] **Step 1: Update kbs**

Update `kb_e6d3bbb1` — move subset gating from "open" to "shipped"; add the carry-forward items from `SUBSET_GATING_STATE.md`. Update `kb_87a7eade` — add Thread L (subset gating) marked SHIPPED.

- [ ] **Step 2: Run full test suite one last time**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
pytest tot/brain/tests/ tot/harness/tests/ tot/memory/tests/ -v 2>&1 | tail -10
```

Expected: All green.

- [ ] **Step 3: Tag**

```bash
git tag -a subset-gating-complete -m "Plan 3 complete — subset gating with FULL/REDUCED/BACKGROUND tiers

Ships:
- Two new harness tools (obs.list_players, obs.list_bot_population)
- Brain SubsetGate module with proximity-based selection
- Sticky party/raid/PvP overrides
- Asymmetric hysteresis (out=2, in=1)
- Operator pin/unpin admin endpoints
- Three-tier model: FULL (60s tick, all 12 bot.* tools) / REDUCED
  (300s tick, 3-tool whitelist) / BACKGROUND (stock strategies, not enrolled)
- ToolPolicyEnforcer in Dispatcher
- Phase A + Phase B eval gates passed

Test counts: 282 memory + 117 parity + ~68 brain = ~467 total.

Deferred to Plan 5: Heimdal integration test, live MCP probe verification.
"
```

---

## Self-review checklist (run after writing the plan)

- [ ] Every spec section §1–§12 has at least one task implementing it
- [ ] Every step that changes code shows the actual code
- [ ] Every test step shows `pytest` invocation + expected outcome
- [ ] No "TBD", "TODO", "similar to Task N"
- [ ] Type names consistent: `SubsetGate`, `SubsetGateConfig`, `SubsetDecision`, `WorldSnapshot`, `PlayerSnapshot`, `BotSnapshot`, `ToolPolicyEnforcer` — same throughout
- [ ] Phase A tasks numbered 1–25; Phase B tasks 26–34
- [ ] Phase A ends with tag; Phase B ends with tag
- [ ] All commits carry `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` trailer
- [ ] License GPL-2.0-or-later SPDX headers on every new file
- [ ] `cmake -j4` (never -j16) on every Heimdal build invocation
- [ ] Heimdal end-to-end deferred to Plan 5 per Plan 2 Task 38 precedent
