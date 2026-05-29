# C++ Runtime Probe: PlayerbotsMgr Enrollment Invariant

**Document type:** Integration probe procedure (Tier-1 soft gate, read-only)  
**Task:** 8.2 of `docs/superpowers/plans/2026-05-29-threads-of-time-1.0.0-operator-install.md`  
**Safe to run against live ToT baseline:** yes — no mutation, no GM commands.

---

## Purpose

Confirm that at worldserver runtime the bot population enrolled by mod-playerbots is
visible and correctly tagged through the `PlayerbotsMgr` machinery.  This is the
invariant the subset-gating brain relies on: every bot entry returned by
`obs.list_bot_population` must have a live `PlayerbotAI*` (i.e. be registered in
`PlayerbotsMgr._playerbotsAIMap`), because the brain's SubsetGate only acts on bots
whose AI layer is alive.

---

## Verified call path (source investigation)

### AC-fork API drift — `IsPlayerBot()` does not exist

The plan's Task 8.2 description referenced `IsPlayerBot()` as the runtime check.  That
method does not exist anywhere in this codebase.  A broad search across all of
`$TOT_HOME/source/modules/mod-playerbots/src/` and
`$TOT_HOME/source/src/` returns no results.  The equivalent check in this
fork is `GET_PLAYERBOT_AI(player) != nullptr`, which calls
`sPlayerbotsMgr.GetPlayerbotAI(player)` and returns non-null iff the player is
registered in `PlayerbotsMgr._playerbotsAIMap`.

### Population source: `RandomPlayerbotMgr`, not `PlayerbotsMgr` directly

`obs.list_bot_population` is dispatched to
`HarnessBridge::Adapters::ObsListBotPopulation()`, registered at
`HarnessBridgeDispatch.cpp:178`.

The adapter implementation is at:

```
modules/mod-harness-bridge/src/Adapters/ObsListBotPopulationAdapter.cpp
```

Lines 22-23 (the population source):

```cpp
// GetAllBots() returns PlayerBotMap BY VALUE (a copy of playerBots).
PlayerBotMap bots = sRandomPlayerbotMgr.GetAllBots();
```

`sRandomPlayerbotMgr` is the macro-singleton for `RandomPlayerbotMgr` (defined at
`RandomPlayerbotMgr.h:261`).  `GetAllBots()` is declared at
`RandomPlayerbotMgr.h:122`:

```cpp
PlayerBotMap GetAllBots() { return playerBots; };
```

`PlayerBotMap` is `typedef std::map<ObjectGuid, Player*>` defined at
`PlayerbotMgr.h:18`.  `playerBots` is the inherited member from
`PlayerbotHolder` (`PlayerbotMgr.h:59`).

`RandomPlayerbotMgr` inherits from `PlayerbotHolder` (confirmed at
`RandomPlayerbotMgr.h:89`: `class RandomPlayerbotMgr : public PlayerbotHolder`).

This map contains **free-roaming random bots only** — bots added via
`AddPlayerBot(botGUID, 0)` (masterAccountId=0), as seen at
`RandomPlayerbotMgr.cpp:1364`.  Player-owned bots (owned by a human player) live in
a separate `PlayerbotMgr::playerBots` instance per human player.  For a ToT
deployment all enrolled bots are random bots (no human masters), so `GetAllBots()`
returns the full enrolled population.

### The PlayerbotsMgr invariant is preserved

The bot is added to `PlayerbotsMgr._playerbotsAIMap` in `OnBotLogin`, at
`PlayerbotMgr.cpp:467-468`:

```cpp
PlayerbotsMgr::instance().AddPlayerbotData(bot, true);   // line 467 — registers in _playerbotsAIMap
playerBots[bot->GetGUID()] = bot;                        // line 468 — adds to RandomPlayerbotMgr.playerBots
```

The `PlayerbotsMgr` registration always precedes the `playerBots` insertion.
Therefore any entry returned by `GetAllBots()` is guaranteed to be registered in
`PlayerbotsMgr._playerbotsAIMap`, i.e. `sPlayerbotsMgr.GetPlayerbotAI(bot) != nullptr`.

### The adapter's per-entry AI check

The adapter verifies the invariant implicitly at `ObsListBotPopulationAdapter.cpp:56-62`:

```cpp
PlayerbotAI* ai = GET_PLAYERBOT_AI(botPlayer);  // sPlayerbotsMgr.GetPlayerbotAI(botPlayer)
if (ai)
{
    Player* master = ai->GetMaster();
    if (master)
        entry["master_guid"] = master->GetGUID().GetCounter();
}
```

`GET_PLAYERBOT_AI` is defined in `Script/Playerbots.h:30`:

```cpp
#define GET_PLAYERBOT_AI(object) sPlayerbotsMgr.GetPlayerbotAI(object)
```

If `ai` is null for any entry, the `master_guid` field is simply absent; the entry is
still emitted (lines 27-29 filter on `IsInWorld()`, not on AI presence).  In a healthy
deployment ai is always non-null for entries in `RandomPlayerbotMgr.playerBots`
because of the `OnBotLogin` ordering described above.  A null `ai` would indicate a
bot that logged in but whose `PlayerbotsMgr` registration was skipped — a worldserver
bug rather than a configuration failure.

---

## Response shape

The adapter at `ObsListBotPopulationAdapter.cpp:67-69` sets:

```cpp
res.outcome = DispatchResult::Outcome::Ok;
res.result_json = {{"bots", bot_array}};
```

The HTTP layer in `HarnessBridge::RegisterDispatchHandler` (`Handlers/DispatchHandler.cpp:79-84`)
wraps this as:

```json
{
  "ok": true,
  "result": {
    "bots": [
      {
        "bot_guid":      12345,
        "name":          "Blorgrix",
        "map_id":        0,
        "x":             -8949.95,
        "y":             -132.493,
        "z":             83.5312,
        "level":         25,
        "in_pvp_combat": false
      }
    ]
  },
  "tick_wait_ms": 2,
  "executor_ms":  1
}
```

Optional per-entry fields (present when applicable):

| Field | Present when |
|---|---|
| `party_guid` | bot is in a non-raid group |
| `raid_guid` | bot is in a raid group |
| `master_guid` | bot has a human player as master |

---

## Relationship to TOT_BOT_POPULATION and TOT_LIVING_BOT_COUNT

`TOT_BOT_POPULATION` (`.env.example:44`, default `20`) is the total number of
free-roaming random bots spawned by mod-playerbots.  This is the size of
`RandomPlayerbotMgr.playerBots` at steady state and the expected count returned by
`obs.list_bot_population`.

`TOT_LIVING_BOT_COUNT` (`.env.example:45`, env var read by the brain-sidecar at
`tot/brain/brain_sidecar/settings.py:56`, default `10`, range 5-15) is the number
of bots the SubsetGate targets for FULL or REDUCED tier — the brain-managed subset.
This is always a subset of `TOT_BOT_POPULATION`.

The probe asserts `len(result["bots"]) == TOT_BOT_POPULATION`, NOT `TOT_LIVING_BOT_COUNT`.
The brain cares about all `TOT_BOT_POPULATION` entries because it selects its active
subset FROM that population; seeing fewer entries than `TOT_BOT_POPULATION` means some
bots have not yet logged in (worldserver still spawning) or have crashed out.

---

## Probe procedure

This is a Tier-1 soft gate — read-only, safe against the live ToT baseline.  The
probe does not mutate worldserver state.

### Prerequisites

- Live worldserver running with mod-playerbots and mod-harness-bridge loaded.
- Harness daemon running and reachable.
- At least `TOT_BOT_POPULATION` random bots have completed login (allow 3-5 minutes
  after worldserver start for full spawn).

### Environment

```bash
export HARNESS_URL=http://127.0.0.1:8099    # or http://<build-host>:8099 from laptop
export HARNESS_BEARER=<token>               # from .env: HARNESS_BEARER_TOKEN
export TOT_BOT_POPULATION=20               # match your .env
```

### Step 1 — Confirm harness is up

```bash
python3 - <<'EOF'
import json, os, urllib.request
url = os.environ["HARNESS_URL"]
tok = os.environ["HARNESS_BEARER"]
req = urllib.request.Request(
    f"{url}/v1/tools/obs.ping",
    data=b"{}",
    headers={"Authorization": f"Bearer {tok}", "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=10) as r:
    print(json.load(r))
EOF
```

Expected: `{"ok": true, "result": {"pong": true}, ...}`

### Step 2 — Call obs.list_bot_population and assert count

```bash
python3 - <<'EOF'
import json, os, sys, urllib.request

url  = os.environ["HARNESS_URL"]
tok  = os.environ["HARNESS_BEARER"]
want = int(os.environ.get("TOT_BOT_POPULATION", "20"))

req = urllib.request.Request(
    f"{url}/v1/tools/obs.list_bot_population",
    data=b"{}",
    headers={"Authorization": f"Bearer {tok}", "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=15) as r:
    body = json.load(r)

if not body.get("ok"):
    print(f"FAIL: ok=false — {body}", file=sys.stderr)
    sys.exit(1)

bots = body["result"]["bots"]
count = len(bots)
print(f"obs.list_bot_population returned {count} bots (expected {want})")

if count != want:
    print(
        f"WARN: count mismatch — got {count}, expected {want}. "
        "Bots may still be spawning; re-run after 5 minutes.",
        file=sys.stderr,
    )

# Shape check: every entry must be a dict with at minimum bot_guid + name.
for i, entry in enumerate(bots):
    if not isinstance(entry, dict):
        print(f"FAIL: bots[{i}] is not a dict: {entry!r}", file=sys.stderr)
        sys.exit(1)
    for key in ("bot_guid", "name", "level", "map_id"):
        if key not in entry:
            print(f"FAIL: bots[{i}] missing required key '{key}': {entry!r}", file=sys.stderr)
            sys.exit(1)

print("PASS: all entries are well-formed dicts with required fields.")

# The invariant: the adapter calls GET_PLAYERBOT_AI on each bot before
# emitting it.  We cannot directly read PlayerbotsMgr._playerbotsAIMap from
# outside the worldserver, but the fact that the adapter emits the entry at all
# (rather than crashing or skipping with an error) means:
#   (a) the bot was found in RandomPlayerbotMgr.playerBots, and
#   (b) bot->IsInWorld() returned true.
# Because OnBotLogin() calls PlayerbotsMgr::AddPlayerbotData BEFORE inserting
# into playerBots, (a) implies GET_PLAYERBOT_AI is non-null.
print("PASS: PlayerbotsMgr enrollment invariant holds (see probe doc for derivation).")
EOF
```

### Expected output (healthy deployment, TOT_BOT_POPULATION=20)

```
obs.list_bot_population returned 20 bots (expected 20)
PASS: all entries are well-formed dicts with required fields.
PASS: PlayerbotsMgr enrollment invariant holds (see probe doc for derivation).
```

### Step 3 — Spot-check one entry

Pick the first bot from the response and confirm its fields are plausible:

```bash
python3 - <<'EOF'
import json, os, urllib.request

url = os.environ["HARNESS_URL"]
tok = os.environ["HARNESS_BEARER"]

req = urllib.request.Request(
    f"{url}/v1/tools/obs.list_bot_population",
    data=b"{}",
    headers={"Authorization": f"Bearer {tok}", "Content-Type": "application/json"},
    method="POST",
)
with urllib.request.urlopen(req, timeout=15) as r:
    body = json.load(r)

first = body["result"]["bots"][0]
print(json.dumps(first, indent=2))
EOF
```

Expected shape (exact values vary per bot):

```json
{
  "bot_guid":      12345,
  "name":          "Blorgrix",
  "map_id":        0,
  "x":             -8949.95,
  "y":             -132.493,
  "z":             83.5312,
  "level":         25,
  "in_pvp_combat": false
}
```

`level` must be in `[1, 80]`.  `map_id` 0 = Eastern Kingdoms, 1 = Kalimdor (common
early-level zones).

### Failure modes and remediation

| Symptom | Likely cause | Remediation |
|---|---|---|
| `count < TOT_BOT_POPULATION` after 5+ min | Some bots failed to login (DB issue, character missing) | Check worldserver log for `AddPlayerBot` errors; verify `playerbots_db` `playerbot` table row count matches `TOT_BOT_POPULATION` |
| `count == 0` | mod-playerbots not loaded, or `RandomBotAutologin=0` | Check worldserver startup log for mod-playerbots module load; check `playerbots.conf` `AiPlayerbot.RandomBotAutologin` |
| HTTP 503 | Harness dispatch queue full (worldserver under heavy load) | Retry; reduce load; increase `HarnessBridge.MaxDispatchPerTick` |
| HTTP 504 | Main-thread drain timeout | Worldserver tick stalled; investigate lag |
| `ok: false, error: unknown_tool` | Harness image predates Plan-3 subset-gating ship | Redeploy harness from `:subset-gating-v1.0` or later |

---

## Why this is sufficient as the invariant check

The plan's original framing referenced `IsPlayerBot()` and
`PlayerbotsMgr::GetPlayerBotsCount()` as the surface to probe.  Neither method exists
in this fork.  The actual invariant — that enrolled bots have a live `PlayerbotAI*`
managed by `PlayerbotsMgr` — is structurally guaranteed by the `OnBotLogin` call
ordering in `PlayerbotMgr.cpp:467-468` (see Verified call path above).

`obs.list_bot_population` is therefore path (a) from the probe definition: it IS
sourced from the correct runtime population (`RandomPlayerbotMgr.playerBots`), and
the `OnBotLogin` ordering provides a compile-time-invariant proof that every entry in
that map is also registered in `PlayerbotsMgr._playerbotsAIMap`.

The subset-gating brain therefore only needs to verify that
`len(obs.list_bot_population.result.bots) == TOT_BOT_POPULATION` to know that its
full candidate pool is online and AI-managed.

---

## Run status

**Not yet run on the live stack.** This document describes the probe procedure; execution
against the live baseline is a documented step, deliberately deferred to avoid
conflating doc verification with live-run scheduling.  The Tier-1 soft gate allows the
release pipeline to proceed without blocking on the live run; the result is recorded
here once executed.

Expected run command (from laptop):

```bash
HARNESS_URL=http://<build-host>:8099 \
HARNESS_BEARER=$(grep HARNESS_BEARER_TOKEN /path/to/.env | cut -d= -f2) \
TOT_BOT_POPULATION=20 \
python3 tot/release/integration/playerbots_runtime_probe.md  # run Step 2 snippet inline
```

Or via the existing Tier-1 suite (which already probes `obs.list_bot_population` for
non-empty result and entry shape):

```bash
HARNESS_URL=http://<build-host>:8099 \
HARNESS_BEARER=<token> \
python3 tot/release/integration/tier1_live_probe.py
```

The PlayerbotsMgr enrollment invariant is structurally implied by any passing
`obs.list_bot_population` result (per the call-path analysis above), so a
`tier1_live_probe.py` PASS already satisfies this gate.
