# Threads of Time 1.0.0 — MPQ Compositor (Design)

**Status:** Design — pending user review
**Date:** 2026-05-28
**Owner:** Thomas Brackin
**Plan:** Plan 4 of the Threads of Time 1.0.0 roadmap
**Lane:** Client-side patch pipeline. Composes every `modules/*/client/` DBC + AddOn
contribution plus a ToT branding overlay into a single shippable client MPQ, with
fail-loud DBC ID-range collision detection.
**Branch:** `dev` (active; Plan 3 + AH fix shipped at `121e32d4d`)
**Completion tag (target):** `mpq-compositor-complete`

**Predecessors:**
- `kb_87a7eade` — START HERE (project nav, live state)
- `docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md` — release architecture
  (§1.1 scope, §5.8 composition + collision discipline, §7 player install, §9.3/§9.4
  ship-or-defer, §10.3/§10.4 Blizzard IP + disclaimer)
- `kb_99501ac5` — Bracket 1 tier-set ship (dbc-patch-builder, 27 ItemSet rows, 54 bonuses,
  the live patch-ZZ.MPQ sha256 `3142d236…`, the `itemcache.wdb` cache-clear gotcha)
- `kb_de5ca41a` — mod-warforged v1.0.2 ship (client contributions; AC fork conventions)
- `kb_163c3e89` — ItemSet schema gotcha (`itemset_dbc`, not `item_set`)
- `kb_6950a902` — spec-writing pre-flight checklist

---

## 1. Goal + scope

### 1.1 What Plan 4 produces

A single shippable client MPQ for the WoW 3.3.5a client (build 12340) that bundles
every module's client contribution plus ToT branding into one additive patch the player
drops into `Data/`. The MPQ contains:

- **DBC overrides** — tier-set tooltip rows (mod-bracket-sets: `ItemSet.dbc` + `Spell.dbc`)
  and Warforged proc rows (mod-warforged), merged with fail-loud ID-range collision detection.
- **The unified `ThreadsOfTime` AddOn** — already composed by Stage A (shipped `82a6583`).
- **Login-screen branding** — version watermark + mandatory fan-project disclaimer
  (spec §3.3 + §10.4), shipped as a GlueXML AddOn.

### 1.2 What already exists (do not re-build)

- **Stage A — AddOn composition.** `tot/client-patch/compose-tot-addon.py` scans
  `modules/<mod>/data/addon-contrib/manifest.toml`, reads per-mod `priority` + `files`,
  and produces a unified `Interface/AddOns/ThreadsOfTime/` folder. Shipped 2026-05-25
  (`82a6583`). First consumer: mod-warforged (priority 10).
- **DBC builder (mod-bracket-sets-specific).** `tot/client-patch/dbc-patch-builder/` —
  a deterministic Python package (21 pytests green) that composes mod-bracket-sets's
  `ItemSet.dbc` (28 rows) + `Spell.dbc` (54 override rows) and packs `patch-Z.MPQ` via
  StormLib (ctypes). Hard-coded to mod-bracket-sets paths in `build.py`.
- **mod-warforged DBC tooling.** `modules/mod-warforged/tools/build-warforged-dbc.py` +
  `pack-mpq.py` + `tests/test_dbc_build.py` produce the prebuilt
  `modules/mod-warforged/client/patch-W.MPQ` (53 KB).
- **AddOn priority allocation.** `tot/client-patch/PRIORITIES.md`.

### 1.3 What Plan 4 builds (Stage B + supporting refactor)

1. **DBC ID-range manifest** — `modules/<mod>/client/MANIFEST.toml` per module.
2. **Generalized compositor library** — refactor `dbc-patch-builder` in place into a shared
   `tot/client-patch/lib/dbc_compositor/` library + per-module recipes.
3. **MPQ pack orchestrator** — `tot/client-patch/pack-mpq.py` (Stage B): discover manifests,
   collision-detect, run recipes, merge DBCs, resolve FrameXML overrides, pull in Stage A
   output + branding, pack one MPQ.
4. **Login-screen branding overlay** — `tot/client-patch/branding/`.
5. **Player install docs** — `docs/player-install.md`.
6. **CI hook** — `tot/release/build-mpq.sh`.

### 1.4 Explicitly NOT in Plan 4

- Heimdal integration test for MPQ auto-download delivery (server-hosts-the-MPQ) — Plan 5.
- A launcher / automatic install (manual drop-in for 1.0.0, spec §1.2).
- Splash imagery as bitmap art — text-only branding for 1.0.0 (spec §10.4 allows external
  branding work later; placeholder OK).
- Cross-machine reproducible builds (same-machine determinism only — see §9.3).
- Localization of DBC text beyond enUS (existing builder is enUS-only; carry forward).

---

## 2. Architecture

A two-stage pipeline. Both stages discover work by globbing module directories.

```
STAGE A — AddOn composition (SHIPPED, 82a6583)
  tot/client-patch/compose-tot-addon.py
  reads  modules/<mod>/data/addon-contrib/manifest.toml
         + tot/client-patch/branding/addon-contrib/manifest.toml   [NEW glob target]
  writes tot/client-patch/build/tot-addon/ThreadsOfTime/
                         │
                         ▼
STAGE B — MPQ pack (NEW, Plan 4)
  tot/client-patch/pack-mpq.py
    1. discover modules/<mod>/client/MANIFEST.toml
    2. load [id_ranges] → RangeRegistry → fail-loud on overlap
    3. import [recipe].entry, call build(sources) → {dbc_file: [rows]}
    4. assert every row ID ∈ that module's declared range (2nd net)
    5. union rows per DBC file, sort by ID, encode blob (shared lib)
    6. resolve [framexml_overrides] by priority → fail-loud on conflict
    7. pull in Stage A AddOn output (on disk)
    8. provenance audit — every packed DBC came from a recipe (no stock .dbc copy)
    9. pack ONE MPQ via shared StormLib wrapper
   10. emit patch-ZZ-tot-X.Y.Z.MPQ + sha256
                         │
                         ▼
          patch-ZZ-tot-1.0.0.MPQ  (release artifact)
          ↓ player renames at install
          patch-ZZ.MPQ            (drop into Data/)
```

**Design rationale.** Stage A's manifest-per-module pattern works; Stage B mirrors it for
DBC + branding. Per-module **recipes** own mod-specific composition logic (warforged's
enchant-row generation lives with warforged, not in the shared library). The shared
**library** does only what is generic: DBC row encoding, MPQ packing, manifest loading +
collision detection, FrameXML resolution.

### 2.1 Library refactor (generalize in place)

The existing `dbc-patch-builder` internals migrate into `tot/client-patch/lib/dbc_compositor/`.
Mod-specific path constants and composition flow from the old `build.py` move into a new
`modules/mod-bracket-sets/client/build_dbc.py` recipe. The 21 existing pytests migrate
alongside the code they cover — no loss of coverage, relocation only.

---

## 3. Per-module manifest + recipe contract

Each module shipping client DBC content gets `modules/<mod>/client/MANIFEST.toml`
(declarative collision surface) and a recipe Python module (imperative row producer).

### 3.1 `modules/mod-bracket-sets/client/MANIFEST.toml`

```toml
[manifest]
mod = "mod-bracket-sets"

[sources]                       # paths relative to module root; resolved by pack-mpq.py
bonus_map      = "data/sql/world/2026_05_13_01_bracket_set_bonus_map_seed.sql"
itemset_map    = "data/sql/world/2026_05_23_07_bracket_set_itemset_map_seed.sql"
descriptions   = "data/fixtures/bracket_set_descriptions.tsv"
spell_baseline = "client/baseline/spell_dbc_baseline.tsv"   # moved here from dbc-patch-builder/baseline/

[recipe]
entry = "build_dbc"             # imports modules/mod-bracket-sets/client/build_dbc.py
                                # exposes build(sources: dict[str, Path]) -> dict[str, list[DbcRow]]

[id_ranges]                     # one DBC file → one-or-more inclusive [min,max] ranges
"ItemSet.dbc" = [{ min = 90100, max = 90199 }]
"Spell.dbc"   = [ ... derived from baseline TSV — see §3.4 PRE-FLIGHT ... ]
```

### 3.2 The recipe contract

Every `client/build_dbc.py` exposes:

```python
def build(sources: dict[str, Path]) -> dict[str, list[DbcRow]]:
    """Return {dbc_filename: [rows]}. Each row carries its ID for collision-check
    + ordering. DbcRow is a shared-lib type (ItemSetRow / SpellRow / EnchantRow, or a
    generic {id: int, blob: bytes}) that the shared library knows how to encode."""
```

`pack-mpq.py`:
1. Loads every `MANIFEST.toml`; validates `[id_ranges]` against the `RangeRegistry`
   (fail-loud on overlap — §4.1).
2. Resolves `[sources]` paths relative to the module root.
3. Imports `[recipe].entry`, calls `build(resolved_sources)`.
4. Asserts every returned row's ID ∈ that module's declared range for that file
   (fail-loud on out-of-range — §4.2).
5. Unions rows per DBC filename, sorts by ID, encodes via the shared library.

### 3.3 mod-warforged

`modules/mod-warforged/client/MANIFEST.toml` + `client/build_dbc.py` get the same treatment,
porting the logic from the existing `modules/mod-warforged/tools/build-warforged-dbc.py`
(+ its `tests/test_dbc_build.py`) into the recipe contract. The prebuilt
`modules/mod-warforged/client/patch-W.MPQ` is **removed** — superseded by the unified
compositor. (Build-time discovery task in the plan; cpp-systems-engineer owns warforged.)

### 3.4 PRE-FLIGHT — derive the real Spell.dbc ID set before writing the manifest

Spec §5.8's sketch showed mod-bracket-sets `Spell.dbc = 49000-49999`, but the actual shipped
override IDs are the **54 marker spell IDs**, which are real retail T7–T10 spell IDs
(e.g. 64752, 64760, 64818 per kb_99501ac5) — NOT a clean 49000 range. Per kb_6950a902 #3
(data-existence) + #5 (verify on disk), the plan MUST grep the actual baseline TSV
(`spell_dbc_baseline.tsv`, 54 rows) to derive the true ID set before writing the manifest's
`[id_ranges]` for `Spell.dbc`. The result will be either several disjoint ranges or an
explicit ID list; the manifest schema supports a list of `{min,max}` entries to express this.
mod-warforged's spell/enchant ranges get the same grep treatment from its baseline.

---

## 4. Collision detection + DBC merge engine

Two independent fail-loud collision concerns.

### 4.1 Declared-range overlap (manifest-level)

A `RangeRegistry` ingests every module's `[id_ranges]`. For each DBC filename it holds a
list of `(module, min, max)` claims; each insert checks against existing claims for that
file. Overlap → hard error:

```
ERROR: DBC ID-range collision in Spell.dbc
  mod-bracket-sets claims [64752, 64818]
  mod-warforged    claims [64800, 99099]
  overlap: [64800, 64818]
  Resolve by editing [id_ranges] in one module's client/MANIFEST.toml
  (allocation map: tot/client-patch/DBC-RANGES.md)
```

### 4.2 Row-level containment (recipe-level, second net)

After a recipe runs, every row ID it produced must fall inside that module's own declared
range for that file. A row outside the declared range → hard error. This catches a recipe
that declares `[64752,64818]` but accidentally emits `90201`.

Together the two checks guarantee: no two modules' rows collide, and every shipped row sits
inside a declared, non-overlapping range. The union therefore has no duplicate IDs by
construction — but the merge step still asserts no duplicate IDs (cheap defense-in-depth).

### 4.3 Merge

For each DBC filename: union all modules' rows, sort by ID ascending, encode the final blob
via the shared library. The client's MPQ patch chain merges our row-level DBC over the
client's stock DBC by ID.

### 4.4 Allocation map — `tot/client-patch/DBC-RANGES.md`

The DBC analogue of the existing `PRIORITIES.md`. A human-readable table of which module
owns which DBC ID ranges, updated in the same commit as any manifest `[id_ranges]` change.
Not machine-read (manifests are the source of truth) — it is the at-a-glance "which ranges
are free" map for the next contributor.

### 4.5 Stock-DBC provenance audit (spec §10.3 + §10.9 — legal)

Spec §10.3: no stock Blizzard DBC files in the MPQ — only ToT-original rows. Because every
DBC is built row-by-row from ToT sources (never extracted wholesale from the client), this
holds by construction. The compositor makes it **verifiable**: it asserts every DBC blob it
packs was produced by a recipe (not copied from a `.dbc` file on disk) and emits a
per-asset provenance line. This turns the §10.9 "grep audit confirming no Blizzard assets"
checklist item into an automated CI gate.

**Noted assumption (Spell.dbc).** The shipped `Spell.dbc` contains only the 54 override rows
(not the full stock Spell.dbc). Each override row carries ToT-authored Name + Description on a
ToT-chosen ID, with non-text stat fields mechanically copied from `spell_dbc_baseline.tsv`
(extracted stock data) so the 3.3.5a client doesn't treat the row as malformed. The existing
live patch-ZZ.MPQ already does exactly this and was deemed acceptable. The provenance audit
checks "no whole stock `.dbc` file is packed," NOT "no field value is ever derived from
baseline." This interpretation is carried forward unchanged from the shipped tier-set patch.

---

## 5. Login-screen branding overlay

Branding ships as a **GlueXML AddOn** composed through the existing Stage A pipeline — no
new packing path; branding is just another contributor.

### 5.1 Location + manifest

`tot/client-patch/branding/` — a synthetic "module" (not under `modules/`, since it is
client chrome, not an AC module). It carries `addon-contrib/manifest.toml`:

```toml
mod = "tot-branding"
priority = 5          # core band (0-9): loads before feature mods
files = ["ToTBranding.lua"]
```

Stage A currently globs only `modules/*/data/addon-contrib/manifest.toml`. Plan 4 extends
the glob to also include `tot/client-patch/branding/addon-contrib/manifest.toml` — a small,
surgical change to `compose-tot-addon.py`.

### 5.2 `ToTBranding.lua`

Runs on the glue (login) screen; sets the version watermark + mandatory disclaimer:

```lua
-- ToTBranding.lua — login-screen watermark + mandatory fan-project disclaimer.
ThreadsOfTime = ThreadsOfTime or {}

local TOT_VERSION = "1.0.0"   -- templated at compose time (§5.4)
local function applyBranding()
    if _G.VersionLabel then    -- the 3.3.5a glue version FontString (VERIFY — §5.3)
        _G.VersionLabel:SetText("Threads of Time " .. TOT_VERSION
            .. "\n|cff888888Unofficial non-commercial fan project. "
            .. "Not affiliated with Blizzard Entertainment.|r")
    end
end
```

### 5.3 PRE-FLIGHT — verify against the real client (kb_6950a902 #10)

1. **Glue FontString name.** `VersionLabel` is the common 3.3.5a name but must be verified
   against the actual client's FrameXML before finalizing (the test machines have a client at
   `ChromieCraft_3.3.5a/`). If the version string is built in Lua via `GetBuildInfo()` rather
   than a static FontString, the hook adapts (hook `GlueParent` OnShow or override the global).
2. **GlueXML AddOn loadability on 3.3.5a.** Whether glue-screen AddOns load from
   `Interface/AddOns/` or require `Interface/GlueXML/` placement varies by client build.
   Verify on the actual client. **Fallback:** the FrameXML-override path (built in §6) — if
   the AddOn approach fails, branding rides the FrameXML rail. This is the primary
   justification for building FrameXML override support (§6).

### 5.4 Version lockstep

`TOT_VERSION` is templated at compose time from a single source (a `--version` flag /
`TOT_VERSION` env). The compositor injects the same version into the branding watermark, the
AddOn TOC `## Version`, and the MPQ artifact filename, keeping them in lockstep (spec §3.5).

### 5.5 Disclaimer copy + placement

The disclaimer text is the verbatim spec §10.4 string. The full multi-line trademark
disclaimer is long for a single login FontString; final copy + placement (shortened watermark
line on login + full text on a secondary line or character-select) is owned by the
game-design-architect agent per the kickoff dispatch.

---

## 6. FrameXML override surface (priority-based)

No current module ships FrameXML overrides (all moved to AddOn-shipped Lua, spec §5.1). Plan 4
builds the surface anyway because branding (§5.3) needs it as an insurance fallback, and a
priority-resolution model future-proofs multi-module overrides.

### 6.1 Manifest section

```toml
[framexml_overrides]
# dest path inside the MPQ ← source file in the module, with a priority for resolution
"Interface/GlueXML/GlueParent.xml" = { src = "client/framexml/GlueParent.xml", priority = 5 }
```

### 6.2 Resolution

`tot/client-patch/lib/dbc_compositor/framexml.py` collects every module's overrides keyed by
destination path. When two modules target the same destination, the **higher-priority** entry
wins AND a hard error is raised if two entries share both the same destination and the same
priority (ambiguous) — last-write-wins is explicitly never silent. The resolved set is packed
into the MPQ alongside DBCs + AddOn. (For 1.0.0 the only expected user is branding's fallback,
so in practice no conflicts occur; the engine is built for the general case.)

---

## 7. File layout

New/changed in Plan 4:

```
tot/client-patch/
├── compose-tot-addon.py            [CHANGED: also glob branding/addon-contrib/]
├── pack-mpq.py                     [NEW: Stage B orchestrator + CLI]
├── PRIORITIES.md                   [existing — AddOn priority allocation]
├── DBC-RANGES.md                   [NEW: DBC ID-range allocation map]
├── lib/dbc_compositor/             [NEW: shared library, refactored from dbc-patch-builder]
│   ├── manifest.py                 [NEW: MANIFEST.toml loader + RangeRegistry]
│   ├── mpq_pack.py                 [MOVED from dbc-patch-builder, unchanged]
│   ├── dbc_io.py                   [MOVED: DBC read/write primitives]
│   ├── itemset_dbc.py              [MOVED: ItemSet row encoder]
│   ├── spell_dbc.py                [MOVED: Spell row encoder]
│   ├── enchant_dbc.py              [NEW if warforged needs it — TBD by warforged port]
│   ├── framexml.py                 [NEW: FrameXML override resolver, priority-based]
│   └── check.py                    [MOVED: determinism --check gate, generalized]
├── branding/
│   ├── addon-contrib/manifest.toml [NEW]
│   ├── addon-contrib/ToTBranding.lua [NEW]
│   └── README.md                   [NEW]
└── tests/                          [NEW: compositor-level tests]
    ├── test_manifest_collision.py
    ├── test_dbc_merge.py
    ├── test_framexml_priority.py
    ├── test_branding_compose.py
    └── test_pack_determinism.py

modules/mod-bracket-sets/client/
├── MANIFEST.toml                   [NEW]
├── build_dbc.py                    [NEW: recipe — mod-specific logic from old build.py]
└── baseline/spell_dbc_baseline.tsv [MOVED from dbc-patch-builder/baseline/]

modules/mod-warforged/client/
├── MANIFEST.toml                   [NEW]
├── build_dbc.py                    [NEW: recipe — ported from tools/build-warforged-dbc.py]
└── (patch-W.MPQ removed — superseded by unified compositor)

docs/player-install.md              [NEW: spec §7 outline]
tot/release/build-mpq.sh            [NEW: CI hook]
```

The old `tot/client-patch/dbc-patch-builder/` internals migrate into `lib/dbc_compositor/`
+ the mod-bracket-sets recipe; its 21 pytests migrate alongside. The directory itself is
removed once migration is proven green. The existing `--check` determinism gate generalizes
to the unified MPQ.

---

## 8. Testing

TDD per superpowers — tests precede implementation in each plan task.

### 8.1 Unit

- **Collision detection** — declared overlap raises; non-overlap passes; out-of-range row
  raises; duplicate-ID at merge raises.
- **DBC merge** — union/sort/dedup correctness.
- **FrameXML priority** — higher priority wins; equal-priority-same-dest raises.
- **Manifest loading** — source-path resolution; malformed manifest raises with a clear message.
- **Branding compose** — branding lands in the Stage A AddOn output at priority 5.

### 8.2 Determinism gate

`pack-mpq.py --check` builds twice and asserts byte-identical MPQ sha256 (same-machine + same
CI environment — §9.3 scope decision). CI gate on every push (spec §5.4).

### 8.3 Regression — the load-bearing guarantee (don't break the live server)

The unified MPQ's `ItemSet.dbc` + `Spell.dbc` blobs MUST be byte-identical to what the live
server already expects — the shipped patch-ZZ.MPQ sha256 `3142d236…` (kb_99501ac5). A
golden-file test asserts the bracket-sets DBC output is unchanged by the refactor. If the
refactor changes a single byte of those DBC blobs, the test fails. This is the kickoff's
"don't break the tier-set tooltip rewrite that's currently shipped in bracket-hook-v4."

### 8.4 In-game verification (deferred to user; mirrors Plan 3)

1. Build `patch-ZZ-tot-1.0.0.MPQ`, rename to `patch-ZZ.MPQ`, drop into a 3.3.5a client's
   `Data/`, clear `Cache/WDB/enUS/itemcache.wdb` (kb_99501ac5 load-bearing gotcha; documented
   in `docs/player-install.md`).
2. Connect → verify "Threads of Time 1.0.0" watermark + disclaimer on login screen.
3. Hover a Bracket 1 tier piece → verify spec-specific bonus tooltip rows (proves tier-set
   DBC composed correctly; matches dbc-patch-builder output that landed in bracket-hook-v4).
4. Loot a Bracket 1 item → ~10% Warforged tag (proves warforged DBC composed correctly).

---

## 9. CI hook + reproducibility

### 9.1 `tot/release/build-mpq.sh`

```
1. python compose-tot-addon.py                                        # Stage A
2. python pack-mpq.py --version "$TOT_VERSION" \
        --out "patch-ZZ-tot-$TOT_VERSION.MPQ"                         # Stage B
3. python pack-mpq.py --check                                         # determinism gate
4. sha256 emit
```

Wired into the release pipeline per spec §5.1 step 6 + §5.4. StormLib is the one external
dependency — the script checks for `libstorm` and emits the brew/source install hint if
absent (matching existing `mpq_pack.py` behavior).

### 9.2 Filename convention

- **Release artifact:** `patch-ZZ-tot-1.0.0.MPQ` (version-pinned name on GitHub Releases).
- **Install name:** `patch-ZZ.MPQ` (player renames when dropping into `Data/`; install doc +
  any future install script handle the rename).

Rationale: the 3.3.5a MPQ patch chain only loads `patch.MPQ`, `patch-<char>.MPQ`, or
`patch-<2chars>.MPQ` — the spec's `patch-Z-tot-1.0.0.MPQ` is not a loadable name. `ZZ` matches
the existing Bracket 1 deploy convention (kb_99501ac5) and sorts after stock `Z` so ToT
overrides load last. Version-pinned artifact name keeps downloads unambiguous; sha256
disambiguates further.

### 9.3 Reproducibility scope

Same-machine + same-CI-environment bit-identical MPQ output (the existing dbc-patch-builder
`--check` discipline, generalized). Cross-machine reproducibility (pinned StormLib + zlib +
`SOURCE_DATE_EPOCH`) is deferred — StormLib/zlib versions differ across dev laptops, Heimdal,
and GitHub runners, and 1.0.0 does not need release attestation (spec §5.6 defers signing).
Carry forward as a 1.0.x candidate.

---

## 10. Constraints (from kickoff + spec)

- **License:** GPL-2.0-or-later; SPDX header on every new source file (spec §10.1).
- **No stock Blizzard DBC files in the MPQ** — provenance audit at compose time (§4.5).
- **Required fan-project disclaimer** in the login-screen branding (§5; spec §10.4).
- **BYOLLM constraint does not apply** — pure client work.
- **Co-Authored-By:** `Claude Opus 4.7 (1M context) <noreply@anthropic.com>` trailer on every
  commit.
- **Don't break:** 345 brain tests, 121 harness parity tests, 79 memory tests (Plan 4 touches
  none of these — pure client tooling — but the CI suite must stay green).
- **Don't break the tier-set tooltip rewrite** shipped in bracket-hook-v4 — golden-file
  regression test (§8.3) is the guard.
- **`cmake -j4` max on Heimdal** — N/A for Plan 4 (no C++ build), but noted for any incidental
  worldserver rebuild during in-game verification.
- **File deletion safety:** removing `patch-W.MPQ` and the old `dbc-patch-builder/` directory
  uses `git rm` (tracked) — no `rm -rf`. Confirm with user before removing any untracked file.

---

## 11. "Plan 4 done" checklist

- [ ] DBC ID-range manifest (`modules/<mod>/client/MANIFEST.toml`) committed + collision
      detection wired into composition (§3, §4)
- [ ] Composition pipeline produces a single `patch-ZZ-tot-1.0.0.MPQ` (§2, §9)
- [ ] Login-screen branding overlay shipped (disclaimer + watermark; text-only) (§5)
- [ ] Tier-set UI composed cleanly into the MPQ; golden-file regression green (§8.3)
- [ ] mod-warforged DBC composed via recipe; `patch-W.MPQ` removed (§3.3)
- [ ] FrameXML override surface built (priority-based) (§6)
- [ ] Player-install doc at `docs/player-install.md` (§1.3, spec §7)
- [ ] CI hook at `tot/release/build-mpq.sh` (§9.1)
- [ ] Determinism gate green; all migrated + new pytests green (§8)
- [ ] State doc `MPQ_COMPOSITOR_STATE.md` (following FOUNDATION_STATE.md /
      MEMORY_SUBSYSTEM_STATE.md / SUBSET_GATING_STATE.md pattern)
- [ ] kb_87a7eade updated; Thread M added, marked SHIPPED
- [ ] Completion tag `mpq-compositor-complete`

---

## 12. Agent dispatch (per project CLAUDE.md)

- **`cpp-systems-engineer`** — owns the dbc-patch-builder generalization, the mod-bracket-sets
  + mod-warforged recipe ports (incl. the warforged `tools/build-warforged-dbc.py` migration),
  and the DBC merge/collision engine.
- **`agentic-harness-engineer` or `cpp-systems-engineer`** — jointly own the Stage B MPQ pack
  generalization (spec §9.4 "Generalize the MPQ pack stage with DBC ID-range manifest +
  collision detection").
- **`game-design-architect`** — owns the branding/disclaimer copy + player-facing tone
  (spec §10.4 disclaimer is mandatory).
- **`deploy-orchestrator`** — owns CI integration in `tot/release/` + nightly AC drift
  validation of the composition.
- **`knowledge-curator`** — owns kb hygiene at completion (kb_99501ac5 follow-up; new kb for
  the manifest format if non-trivial; kb_87a7eade Thread M).

---

## 13. Open questions

| Question | Owner | When |
|---|---|---|
| Exact Spell.dbc ID set for mod-bracket-sets (grep the baseline TSV) | cpp-systems-engineer | Plan task 1 pre-flight |
| Exact glue FontString name + GlueXML AddOn loadability on the 3.3.5a client | game-design-architect / cpp-systems-engineer | Branding task pre-flight |
| Final disclaimer copy + login-vs-charselect placement | game-design-architect | Branding task |
| Does warforged need a new `enchant_dbc.py` encoder, or does it reuse spell rows? | cpp-systems-engineer | Warforged recipe port |
| Confirm `patch-W.MPQ` removal is safe (nothing else references it) | cpp-systems-engineer | Warforged recipe port |
