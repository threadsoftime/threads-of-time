# Threads of Time 1.0.0 — MPQ Compositor (Plan 4) — Design

**Status:** Design — pending user review
**Date:** 2026-05-28
**Owner:** Thomas Brackin
**Repo / branch:** `threads-of-time` / `dev` (tip `121e32d4d` at design time)
**Lane:** Client-side patch pipeline. Produces the single shippable client MPQ that bundles every `modules/*/client/` contribution (DBC rows, FrameXML overrides, Lua AddOns) plus ToT branding, with fail-loud DBC ID-range collision detection.

**Predecessors / authoritative references:**
- `docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md` — parent release design. §1.1 (1.0.0 scope), §5.8 (client MPQ composition + collision discipline — authoritative), §7 (player install), §9.3 + §9.4 (ship-or-defer + pre-freeze checklist), §10.3 + §10.4 (Blizzard IP posture + required disclaimer).
- `kb_99501ac5` — Bracket 1 tier-set ship: the existing `dbc-patch-builder`, 28 ItemSet rows, 54 Spell overrides, the shipped `patch-ZZ.MPQ` (sha256 `3142d2362430287717bfaab836c45e764ecd0f26c2a24f1820241ec628aa5d47`), and the `itemcache.wdb` first-deploy gotcha.
- `kb_de5ca41a` — mod-warforged v1.0.2 ship + AC-fork conventions checklist.
- `kb_163c3e89` — itemset_dbc schema divergence in the playerbots fork.
- `kb_6950a902` — spec-writing pre-flight checklist.
- `kb_a217a790` — Release Pipeline + Operator Install (this design's decisions are mirrored into its "DBC-side manifest — DESIGN LOCKED" section).
- `tot/client-patch/compose-tot-addon.py` — the shipped Stage A AddOn composer (commit `82a6583`).
- `tot/client-patch/dbc-patch-builder/` — the existing mod-bracket-sets DBC builder being generalized.

---

## 1. Scope

### 1.1 What Plan 4 produces

A single shippable client patch for the WoW 3.3.5a (build 12340) client. The player drops it into their client's `Data/` folder alongside the stock client; it is additive and reversible by deletion. The patch bundles:

- **DBC additions** — ToT-original rows for `ItemSet.dbc`, `Spell.dbc`, and any future per-module DBC files (e.g. mod-warforged enchant/spell rows). Built row-by-row from ToT sources; never wholesale stock Blizzard DBC files (§10.3 of the parent design).
- **Lua AddOns** — the unified `ThreadsOfTime` in-game AddOn already produced by Stage A.
- **FrameXML overrides** — supported but currently unused (all modules ship AddOn Lua instead).
- **Login-screen branding** — version watermark + the mandatory fan-project disclaimer (§10.4), shipped as a GlueXML AddOn.

### 1.2 Key prior-art discovery — Stage A already shipped

The parent design's §5.8 specifies a two-stage compositor. **Stage A (the unified-AddOn composer) already shipped** at commit `82a6583` (2026-05-25) as `tot/client-patch/compose-tot-addon.py`. It scans `modules/<mod>/data/addon-contrib/manifest.toml` and assembles a single `Interface/AddOns/ThreadsOfTime/` folder with an auto-generated `00-Core.lua` + `ThreadsOfTime.toc`, prefixing files by `(priority, mod, file-index)`.

**Plan 4 is therefore only Stage B plus its supporting work** — not a from-scratch compositor:
1. Generalize the existing `dbc-patch-builder` into a shared library + per-module recipes.
2. Define the per-module DBC manifest (`modules/<mod>/client/MANIFEST.toml`) with ID-range collision detection.
3. Build `pack-mpq.py` — the Stage B orchestrator that produces the final MPQ.
4. Ship login-screen branding through the existing Stage A pipeline.
5. Add FrameXML override support (priority-based).
6. Player-install doc + CI hook.

### 1.3 Out of scope (deferred)

- **Heimdal MPQ auto-download / server-hosted delivery** → Plan 5 (operator install stack).
- **Cross-machine reproducible builds** (identical sha256 across dev laptop / Heimdal / CI) → deferred; same-machine determinism only (§5.3).
- **Brackets 2–7 DBC content** → 2.0.0+ (no rows authored).
- **Splash imagery / brand-asset art** → external/parallel track; 1.0.0 ships text-only branding.
- **HD client textures** → parked (`kb_6354a4aa`); legally separate posture.

---

## 2. Architecture

### 2.1 Two-stage pipeline

```
┌──────────────────────────────────────────────────────────────────────────┐
│ STAGE A — AddOn composition  (SHIPPED 2026-05-25, commit 82a6583)         │
│   tot/client-patch/compose-tot-addon.py                                   │
│   reads modules/<mod>/data/addon-contrib/manifest.toml  (+ branding, NEW) │
│   writes tot/client-patch/build/tot-addon/ThreadsOfTime/                  │
└──────────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ STAGE B — MPQ pack  (NEW — Plan 4)                                        │
│   tot/client-patch/pack-mpq.py                                            │
│                                                                           │
│   1. Discover modules/<mod>/client/MANIFEST.toml                          │
│   2. Load each manifest's [id_ranges] → RangeRegistry collision check     │
│   3. For each module, import [recipe].entry, call build(sources)          │
│   4. Assert every returned row ID ∈ that module's declared range          │
│   5. Union rows per DBC file, sort by ID, assert no dup, encode blob      │
│   6. Resolve [framexml_overrides] by priority (error on path conflict)    │
│   7. Pull in Stage A AddOn output (already on disk)                       │
│   8. Provenance audit: every packed DBC came from a recipe (§10.3 gate)   │
│   9. Pack everything into ONE MPQ via shared StormLib wrapper             │
│  10. Emit patch-ZZ-tot-<version>.MPQ + sha256                            │
└──────────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
                  patch-ZZ-tot-1.0.0.MPQ   (GitHub release artifact)
                  ↓ player renames at install time
                  patch-ZZ.MPQ             (drop into client Data/)
```

### 2.2 Library / recipe split

The existing `dbc-patch-builder` package is generalized **in place** into two layers:

- **Shared library** — `tot/client-patch/lib/dbc_compositor/`. Everything truly generic: DBC row encoders (`itemset_dbc`, `spell_dbc`, and `enchant_dbc` if warforged needs it), the StormLib MPQ packer (`mpq_pack`, moved unchanged), DBC read/write primitives (`dbc_io`), the manifest loader + `RangeRegistry` (`manifest`), the FrameXML resolver (`framexml`), and the `--check` determinism gate (`check`).
- **Per-module recipes** — `modules/<mod>/client/build_dbc.py`. The mod-specific composition logic. The current `dbc-patch-builder/src/dbc_patch_builder/build.py` (which hard-codes mod-bracket-sets paths) becomes `modules/mod-bracket-sets/client/build_dbc.py`, importing the shared library.

**Why this split:** the shared library does what every module needs identically (encode a DBC, pack an MPQ, detect a collision). Each module owns its own composition quirks where they belong — mod-warforged's enchant-row generation lives with mod-warforged, not in a central file that grows a branch per module. This matches the Stage A pattern (central composer + per-module manifests) and keeps each unit independently understandable and testable.

The existing 21 `dbc-patch-builder` pytests (deterministic, green per `kb_99501ac5`) migrate alongside the code they cover — split between `lib/dbc_compositor/tests/` (encoder + packer + io tests) and `modules/mod-bracket-sets/client/tests/` (the bracket-sets recipe tests). No coverage is lost; it is relocated.

---

## 3. Per-module DBC manifest + recipe contract

### 3.1 Manifest

Each module shipping client DBC content gets `modules/<mod>/client/MANIFEST.toml`. The manifest is the **declarative collision surface**; the recipe is the **imperative row producer**.

```toml
[manifest]
mod = "mod-bracket-sets"

[sources]                       # relative to the module root; resolved by pack-mpq.py
bonus_map      = "data/sql/world/2026_05_13_01_bracket_set_bonus_map_seed.sql"
itemset_map    = "data/sql/world/2026_05_23_07_bracket_set_itemset_map_seed.sql"
descriptions   = "data/fixtures/bracket_set_descriptions.tsv"
spell_baseline = "client/baseline/spell_dbc_baseline.tsv"

[recipe]
entry = "build_dbc"             # importable: modules/mod-bracket-sets/client/build_dbc.py

[id_ranges]                     # per-DBC-file inclusive ranges; a list allows disjoint ranges
"ItemSet.dbc" = [{ min = 90100, max = 90199 }]
"Spell.dbc"   = [ ... derived from the real baseline TSV — see §6 pre-flight ... ]
```

`[sources]` references existing files in their authoritative location (no duplication). mod-bracket-sets's DBC source data stays under `data/` because the server-side C++ loader also reads it; the manifest points the recipe at it via module-root-relative paths. The one relocation is the Spell.dbc baseline TSV, which is client-only and moves from `dbc-patch-builder/baseline/` to `modules/mod-bracket-sets/client/baseline/`.

### 3.2 Recipe contract

Every `build_dbc.py` exposes a single entry point:

```python
def build(sources: dict[str, Path]) -> dict[str, list[DbcRow]]:
    """Return {dbc_filename: [rows]}.

    sources: the [sources] table from MANIFEST.toml, resolved to absolute Paths.
    Each row carries its ID (for collision + ordering) and is a typed
    shared-lib dataclass (ItemSetRow / SpellRow / EnchantRow) that the
    shared library knows how to encode into DBC bytes.
    """
```

The compositor:
1. Loads every `MANIFEST.toml`, validates `[id_ranges]` against the `RangeRegistry` (§4.1) — fails loud on overlap.
2. Resolves `[sources]` to absolute paths, imports `[recipe].entry`, calls `build(sources)`.
3. Asserts every returned row's ID falls inside that module's declared range for that file (§4.2 — the second collision net).
4. Unions rows per DBC filename across modules, sorts by ID, asserts no duplicate ID, encodes the final blob via the shared library.

---

## 4. Collision detection + DBC merge engine

Two independent fail-loud checks plus a defense-in-depth assertion.

### 4.1 Declared-range overlap (manifest level — check A)

A `RangeRegistry` ingests every module's `[id_ranges]`. For each DBC filename it holds a list of `(module, min, max)` claims; on insert it checks the new claim against existing claims for that file. Any overlap is a hard error:

```
ERROR: DBC ID-range collision in Spell.dbc
  mod-bracket-sets claims [64752, 64818]
  mod-warforged    claims [64800, 99099]
  overlap: [64800, 64818]
  Resolve by editing [id_ranges] in one module's client/MANIFEST.toml
  (project-wide allocation tracked in tot/client-patch/DBC-RANGES.md)
```

### 4.2 Row-level containment (recipe level — check B)

After a recipe runs, every row ID it produced must fall inside that module's *own* declared range for that file. A recipe that emits a row outside its declared range is a hard error. This catches the case where the manifest declares `[64752,64818]` but the recipe accidentally emits `90201`.

Together, A + B guarantee: *no two modules' rows ever collide, and every shipped row sits inside a declared, non-overlapping range.*

### 4.3 Merge

For each DBC filename: union all modules' rows, sort by ID ascending, encode via the shared library. Because ranges are provably non-overlapping (A) and rows are provably in-range (B), the union has no duplicate IDs by construction — but the merge still asserts no duplicate IDs (defense in depth; cheap).

### 4.4 Project-wide allocation doc

`tot/client-patch/DBC-RANGES.md` — the DBC analogue of the existing `PRIORITIES.md` for AddOns. A human-readable table of which module owns which DBC ID ranges per file. Updated in the same commit as any manifest `[id_ranges]` change. Not machine-read (the manifests are the source of truth) but it is the at-a-glance "which ranges are free" map.

### 4.5 Stock-DBC provenance audit (§10.3 + §10.9 legal gate)

The final MPQ must contain only ToT-original DBC rows, never a wholesale stock Blizzard DBC file. Because every DBC blob is built row-by-row from ToT sources (never extracted from a client `.dbc`), this holds by construction. The compositor makes it *verifiable*: it asserts every DBC blob it packs was produced by a recipe (never copied from a `.dbc` file on disk) and emits a per-asset provenance line. This satisfies the §10.9 "grep audit confirming no Blizzard assets in MPQ" checklist item as an automated gate rather than a manual review.

**Noted assumption (carried from the shipped patch-ZZ.MPQ):** the shipped Spell.dbc contains only the 54 override rows — not the full stock Spell.dbc. The client's MPQ patch chain merges our 54-row file over the stock one (DBC patching is row-level by ID in the 3.3.5a client). Those 54 rows carry ToT-authored Name + Description text on ToT-chosen marker IDs; their non-text stat fields are mechanically copied from a baseline TSV (extracted from stock 3.3.5a Spell.dbc) so the client does not treat the row as malformed. The already-shipped `patch-ZZ.MPQ` does exactly this and was deemed acceptable. The provenance gate checks "no whole stock `.dbc` file packed," not "no field value ever derived from stock."

---

## 5. Stage B orchestrator, branding, FrameXML, determinism

### 5.1 `pack-mpq.py`

The Stage B CLI. Inputs: the Stage A AddOn output (on disk), every `modules/<mod>/client/MANIFEST.toml`, the branding overlay, and a `--version` value. Output: one MPQ + its sha256. Flow per §2.1 steps 1–10. StormLib is the one external dependency; the script checks for `libstorm` and emits the brew/source install hint if absent (matching existing `mpq_pack.py` behavior).

```
python pack-mpq.py --version 1.0.0 --out build/patch-ZZ-tot-1.0.0.MPQ
python pack-mpq.py --check        # determinism gate (§5.4)
```

### 5.2 Login-screen branding overlay

Branding ships as a **GlueXML AddOn** composed through the existing Stage A pipeline — no new packing path; branding is just another contributor.

- **Location:** `tot/client-patch/branding/` — a synthetic "module" (not an AC module, so not under `modules/`). It carries `addon-contrib/manifest.toml`:
  ```toml
  mod = "tot-branding"
  priority = 5          # core band (0-9): loads before feature mods
  files = ["ToTBranding.lua"]
  ```
- **Composer change:** `compose-tot-addon.py` currently globs only `modules/*/data/addon-contrib/manifest.toml`. Plan 4 extends the glob to also include `tot/client-patch/branding/addon-contrib/manifest.toml`. Small, surgical change.
- **`ToTBranding.lua`** runs on the glue (login) screen, sets the version watermark + the mandatory §10.4 disclaimer on the existing version FontString:
  ```lua
  ThreadsOfTime = ThreadsOfTime or {}
  local TOT_VERSION = "1.0.0"   -- templated at compose time (see §5.5)
  local function applyBranding()
      if _G.VersionLabel then
          _G.VersionLabel:SetText("Threads of Time " .. TOT_VERSION
              .. "\n|cff888888Unofficial non-commercial fan project."
              .. " Not affiliated with Blizzard Entertainment.|r")
      end
  end
  ```
- **Disclaimer text:** the verbatim §10.4 string is the mandatory content. The full multi-line trademark notice is long for a login FontString; final copy + placement (login watermark line vs. a secondary line vs. character-select) is owned by `game-design-architect` per the kickoff dispatch. Minimum bar: the fan-project disclaimer is visible on the login screen.

**Two pre-flight client-verification items** (kb_6950a902 #10 — verify against the real client, do not trust memory):
1. **Exact glue version FontString name.** `VersionLabel` is the common 3.3.5a name but must be confirmed against the actual client FrameXML (test machines have a client at `ChromieCraft_3.3.5a/`). If the version string is built in Lua via `GetBuildInfo()` rather than a static FontString, the hook adapts (hook `GlueParent` OnShow / override the relevant global).
2. **GlueXML AddOn loadability on this client build.** Whether glue-screen AddOns load from `Interface/AddOns/` or require `Interface/GlueXML/` placement varies by build. Verify on the actual client before finalizing. **Fallback:** the FrameXML-override path (§5.3) — branding rides that rail if glue AddOns do not load cleanly. This is the concrete justification for building FrameXML override support now.

### 5.3 FrameXML overrides (priority-based)

No current module ships FrameXML overrides (all moved to AddOn Lua per the parent design's deprecation in §5.1). Plan 4 nonetheless implements full priority-based override support, because it is branding's insurance policy (§5.2 item 2) and because the parent design's §5.8 names FrameXML as a Stage B input.

- A `[framexml_overrides]` section in `MANIFEST.toml` lists `{ src, dest, priority }` entries.
- `pack-mpq.py` resolves them by priority; when two modules target the same `dest` path, the higher-priority entry wins and the loser is reported. A same-priority conflict on the same `dest` is a hard error (never silent last-write-wins).
- This mirrors the Stage A AddOn priority pattern, so the project has one consistent mental model for "two modules touch the same client thing."

### 5.4 Determinism

Same-machine byte-identical MPQ output. `pack-mpq.py --check` builds twice and asserts the two outputs have identical sha256. CI gate. This does **not** guarantee cross-machine reproducibility (StormLib + zlib versions differ across dev laptops, Heimdal, and GitHub-hosted runners); that is deferred. The check catches the real non-determinism bugs — unsorted iteration, embedded timestamps, dict ordering — which is what the existing `dbc-patch-builder --check` already protects against, generalized to the multi-source MPQ.

### 5.5 Version lockstep

`pack-mpq.py --version <v>` is the single source of the version string at compose time. The compositor injects it into (a) the branding watermark (`TOT_VERSION` in `ToTBranding.lua`), (b) the AddOn TOC `## Version`, and (c) the MPQ artifact filename `patch-ZZ-tot-<v>.MPQ`. This satisfies the parent design's §3.5 subcomponent-versioning lockstep requirement.

### 5.6 MPQ filename (load-bearing correction to the parent design)

The parent design (§3.3, §5.8, §7) names the client MPQ `patch-Z-tot-1.0.0.MPQ`. **That is not a loadable WoW 3.3.5a MPQ-chain filename** — the client only loads `patch.MPQ`, `patch-<1 char>.MPQ`, or `patch-<2 chars>.MPQ` (no dashes or version suffix in the loaded name). Decision:

- **Release artifact** (GitHub Releases): `patch-ZZ-tot-<version>.MPQ` — version-pinned for clarity.
- **Install name** (in the client's `Data/`): `patch-ZZ.MPQ` — the player (or a future install script) renames on drop-in.

`ZZ` matches the already-shipped Bracket 1 deploy (`kb_99501ac5`) and sorts last in the patch chain, so ToT's DBC overrides win over stock and over any pre-existing custom MPQ. The player-install doc covers the rename plus the `Cache/WDB/enUS/itemcache.wdb` clear step.

---

## 6. Pre-flight verification (kb_6950a902)

Run before writing manifest ranges or recipe code to disk:

1. **Spell.dbc ID ranges are NOT `49000-49999`.** That value in the parent design's §5.8 is a placeholder. The actual override IDs are real retail T7-T10 spell IDs (e.g. 64752, 64760, 64818 per `kb_99501ac5`). The plan MUST grep the real baseline (`spell_dbc_baseline.tsv` — currently at `tot/client-patch/dbc-patch-builder/baseline/`) and the bonus-map seed to derive the true ID set, then express `[id_ranges]` for `Spell.dbc` as the real (likely disjoint) ranges or an explicit ID list. (kb_6950a902 #3 — data existence.)
2. **mod-warforged ships a prebuilt `patch-W.MPQ`** in `modules/mod-warforged/client/`, built by `modules/mod-warforged/tools/pack-mpq.py`. That packing logic must be reverse-engineered into a `modules/mod-warforged/client/build_dbc.py` recipe + `MANIFEST.toml`, and the prebuilt MPQ retired. Owner: `cpp-systems-engineer` (owns warforged). This is a discovery task at the start of implementation.
3. **AC-fork conventions** (kb_de5ca41a) do not apply to this Python-only Plan, but the C++ side is untouched: Plan 4 must NOT change `modules/mod-bracket-sets/src/` behavior. The golden-file test (§7) is the guard.

---

## 7. Testing

TDD per `superpowers:test-driven-development` — tests precede implementation in each plan task.

- **Unit (collision):** overlap → raises; non-overlap → passes; out-of-range row → raises; duplicate ID at merge → raises.
- **Unit (merge):** union across modules, sort by ID, no-dup invariant.
- **Unit (FrameXML):** priority resolution; same-priority same-dest conflict → raises.
- **Unit (branding):** the branding AddOn lands in the Stage A output with priority 5; the disclaimer string is present.
- **Unit (manifest):** `[sources]` path resolution; malformed manifest → clear error.
- **Determinism:** `pack-mpq.py --check` builds twice, asserts identical sha256. CI gate.
- **Golden-file regression (non-negotiable):** the composed bracket-sets `ItemSet.dbc` + `Spell.dbc` blobs must be **byte-identical to what the live server already expects** — the DBC content inside the shipped `patch-ZZ.MPQ` (sha256 `3142d236…`, `kb_99501ac5`). If the library refactor changes a single byte of those blobs, the test fails. This is the "don't break the tier-set tooltip rewrite (bracket-hook-v4)" guarantee from the kickoff, made mechanical.

**In-game verification path** (mirrors the Plan 3 in-game test; user-executed, not CI):
1. Build `patch-ZZ-tot-1.0.0.MPQ`, rename to `patch-ZZ.MPQ`, drop into a 3.3.5a client's `Data/`, clear `Cache/WDB/enUS/itemcache.wdb` (the `kb_99501ac5` first-deploy gotcha — documented in the player-install doc).
2. Connect → verify the "Threads of Time 1.0.0" watermark + disclaimer on the login screen.
3. Hover a Bracket 1 tier piece → verify spec-specific bonus tooltip rows (proves the tier-set DBC composed correctly).
4. Loot a Bracket 1 item → ~10% chance of an orange Warforged tag (proves the warforged DBC composed correctly).

---

## 8. File layout

New / changed in Plan 4:

```
tot/client-patch/
├── compose-tot-addon.py            [CHANGED: also glob tot/client-patch/branding/]
├── pack-mpq.py                     [NEW: Stage B orchestrator + CLI]
├── PRIORITIES.md                   [existing — AddOn priority allocation]
├── DBC-RANGES.md                   [NEW: DBC ID-range allocation map]
├── lib/
│   └── dbc_compositor/             [NEW: shared library, refactored from dbc-patch-builder]
│       ├── manifest.py             [NEW: MANIFEST.toml loader + RangeRegistry]
│       ├── mpq_pack.py             [MOVED from dbc-patch-builder, unchanged]
│       ├── dbc_io.py               [MOVED: DBC read/write primitives]
│       ├── itemset_dbc.py          [MOVED: ItemSet row encoder]
│       ├── spell_dbc.py            [MOVED: Spell row encoder]
│       ├── enchant_dbc.py          [NEW if warforged needs it — confirmed in pre-flight #2]
│       ├── framexml.py             [NEW: FrameXML override resolver, priority-based]
│       ├── check.py                [MOVED: determinism --check gate, generalized]
│       └── tests/                  [encoder + packer + io + manifest + framexml tests]
├── branding/
│   ├── addon-contrib/
│   │   ├── manifest.toml           [NEW]
│   │   └── ToTBranding.lua         [NEW]
│   └── README.md                   [NEW]
└── tests/                          [NEW: compositor-level tests]
    ├── test_manifest_collision.py
    ├── test_dbc_merge.py
    ├── test_framexml_priority.py
    ├── test_branding_compose.py
    ├── test_pack_determinism.py
    └── test_golden_bracket_sets.py [the non-negotiable regression guard]

modules/mod-bracket-sets/client/
├── MANIFEST.toml                   [NEW]
├── build_dbc.py                    [NEW: recipe — logic from old dbc-patch-builder/build.py]
├── baseline/spell_dbc_baseline.tsv [MOVED from dbc-patch-builder/baseline/]
└── tests/                          [bracket-sets recipe tests, migrated]

modules/mod-warforged/client/
├── MANIFEST.toml                   [NEW]
├── build_dbc.py                    [NEW: recipe — ported from warforged tools/pack-mpq.py]
└── (patch-W.MPQ removed — superseded by the unified compositor)

docs/player-install.md              [NEW: parent design §7 outline]
tot/release/build-mpq.sh            [NEW: CI hook — Stage A + Stage B + --check]
```

The old `tot/client-patch/dbc-patch-builder/` directory's internals migrate into `lib/dbc_compositor/` + the mod-bracket-sets recipe; the directory is retired once the migration is green.

---

## 9. CI hook

`tot/release/build-mpq.sh`:
1. `python compose-tot-addon.py` (Stage A)
2. `python pack-mpq.py --version "$TOT_VERSION" --out "build/patch-ZZ-tot-$TOT_VERSION.MPQ"` (Stage B)
3. `python pack-mpq.py --check` (determinism gate)
4. emit sha256

Wired into the release pipeline per the parent design's §5.1 step 6 and run as the "compositor determinism check" on every push to `dev` / PR per §5.4. StormLib presence is checked with a clear install hint on absence.

---

## 10. Deliverables → "Plan 4 done"

Maps 1:1 to the kickoff completion criteria:

| Deliverable | Where |
|---|---|
| DBC ID-range manifest + collision detection | `modules/<mod>/client/MANIFEST.toml` + `lib/dbc_compositor/manifest.py` (RangeRegistry) |
| Composition pipeline → single MPQ | `tot/client-patch/pack-mpq.py` → `patch-ZZ-tot-1.0.0.MPQ` |
| Login-screen branding overlay | `tot/client-patch/branding/` (disclaimer text minimum; splash deferred) |
| Tier-set UI composed cleanly | mod-bracket-sets recipe + the golden-file regression test |
| Player-install doc | `docs/player-install.md` |
| CI hook | `tot/release/build-mpq.sh` |
| State doc | `MPQ_COMPOSITOR_STATE.md` (following FOUNDATION_STATE / MEMORY_SUBSYSTEM_STATE / SUBSET_GATING_STATE pattern) |
| kb update | `kb_87a7eade` gains Thread M (SHIPPED); `kb_a217a790` already updated |

**Completion tag:** `mpq-compositor-complete`.

**Constraints honored:** GPL-2.0-or-later (SPDX headers on new files); no stock Blizzard DBC in the MPQ (§4.5 provenance gate); mandatory fan-project disclaimer in the branding AddOn (§5.2); BYOLLM N/A (pure client work); do not break 345 brain / 121 harness parity / 79 memory tests (none touched — Python client tooling only); do not break the bracket-hook-v4 tier-set tooltip rewrite (§7 golden-file guard). Commits carry the `Co-Authored-By: Claude Opus 4.7 (1M context)` trailer.

---

## 11. Agent dispatch (per project CLAUDE.md)

- `cpp-systems-engineer` — DBC + dbc-patch-builder generalization, the shared encoders, the mod-warforged `patch-W.MPQ` → recipe port (owns warforged).
- `cpp-systems-engineer` / `agentic-harness-engineer` — jointly own the Stage B pack stage + DBC ID-range manifest + collision detection (parent design §9.4 line).
- `game-design-architect` — branding text + disclaimer copy + player-facing tone (§10.4 disclaimer mandatory).
- `deploy-orchestrator` — CI integration in `tot/release/` + nightly AC-drift validation of the composition.
- `knowledge-curator` — kb hygiene at completion (Thread M in `kb_87a7eade`; the `kb_a217a790` DBC-manifest section is already updated).
```
