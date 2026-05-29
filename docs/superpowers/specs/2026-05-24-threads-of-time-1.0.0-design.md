# Threads of Time 1.0.0 — Release Architecture (Design)

**Status:** Design — pending user review
**Date:** 2026-05-24
**Owner:** Thomas Brackin
**Lane:** Cross-cutting — defines the product identity, repo structure, versioning scheme, dev workflow, release pipeline, operator + player install experience, AC-upgrade workflow, scope freeze, and licensing posture for the first public release of Threads of Time. Subsequent design docs (V3 memory subsystem, subset gating, composition pipeline, etc.) hang off this one.
**Predecessors:**
- `kb_87a7eade` — START HERE (project nav, live state)
- `kb_99501ac5` — Phase 1.5 Bracket 1 tier sets (54 bonuses shipped 2026-05-17)
- `kb_57b453cd` — Heimdal deploy pipeline
- `kb_642162c3` — Harness V1.x patterns
- `2026-05-23-mod-warforged-design.md` — Legion-style Warforged + Bonus Sockets
- `2026-05-23-bracket1-tier-set-ui-client-patch-design.md` — tier-set UI MPQ
- `2026-05-22-v3-6-max-level-personality-v2-design.md` — V3 brain personality
- V3.7.4 brain shipped at commit `565f5b7`; V3.8 multi-mode routing **does not ship** in 1.0.0 (see §9.4)
- `feat+mod-warforged-v1` worktree — mod-warforged implementation through commit `c18330d`

---

## 1. Product definition

Threads of Time is an AI-driven *World of Warcraft: Wrath of the Lich King 3.3.5a* expansion built on AzerothCore. It is not a content mod and it is not a server fork — it is a complete play experience defined by three product pillars:

1. **Alive bots.** Every Bracket 1 player party is a mix of you and AI-driven companions whose decisions, memories, and personalities persist across sessions. The bots are powered by **mod-agenticbots**, ToT's bot-AI layer built on top of **mod-playerbots** (an external dependency operators install alongside ToT). mod-agenticbots adds LLM-orchestrated strategies/actions/triggers, the harness integration, the brain wiring, and the LlmAgent worker thread; it calls into mod-playerbots's public API (PlayerbotAI, AI_VALUE macros, strategy/action/trigger system) rather than forking or vendoring it. Bots perceive game state, hold goals, recall past play, and act through a typed tool surface (`bot.*`, `obs.*`, `gm.*`, `memory.*`). This is the headline feature.
2. **Bracketed progression.** The level cap, available zones, and earnable gear are scoped to a single progression bracket per major release. ToT 1.0.0 = Bracket 1 (L1–25). Future majors extend the cap and the world. Within a bracket, gameplay is designed to be *complete* — you don't outgrow the content, you master it.
3. **Custom content for the bracket.** Class+spec-specific tier-set bonuses (54 in 1.0.0), Legion-style Warforged + Bonus Socket procs on item drops, a bracket-appropriate raid (Blackfathom Deeps), and content systems (e.g., rotation mode) tuned for the bracket's pacing.

### 1.1 Scope of 1.0.0

- **Server:** forked AzerothCore + mod-agenticbots + mod-bracket-sets + mod-rotation-mode + mod-warforged + mod-harness-bridge + the brain sidecar + the memory subsystem
- **Content:** Bracket 1 leveling (L1–25), Phase 1.5 tier-set system (27 ItemSet rows, 54 spec-specific bonuses), BFD raid, Warforged + Bonus Socket loot procs
- **Client patch:** single composed `patch-Z-tot-1.0.0.MPQ` containing tier-set tooltip DBC overrides + Warforged FrameXML overrides + ToT branding (login-screen version watermark, fan-project disclaimer, splash)
- **Operator stack:** worldserver + harness daemon + brain sidecar + memory store; BYOLLM (OpenAI-compatible)
- **Player stack:** standard 3.3.5a client + ToT client MPQ + realmlist line

### 1.2 Explicitly NOT in 1.0.0

- Brackets 2–7 (deferred to 2.0.0+)
- End-game content, dungeons/raids beyond BFD, PvP systems
- HD client work (`kb_6354a4aa`, parked)
- A launcher (manual MPQ install for 1.0.0)
- A hosted/managed deployment (BYOLLM only; "Threads of Time: Hosted" is a possible future variant)
- Tertiary stats (Speed/Leech/Avoidance), Titanforged, re-roll consumables (mod-warforged v2 territory)
- Multi-endpoint LLM routing (Heimdal-internal only — see §9.4)
- BFD-tuned bot strategies (custom Strategy/Action/Trigger contexts for the BFD raid; deferred pending mod-playerbots upstream PR — see §9.2 + `tot/internal-docs/agenticbots-upstream-prs.md`)
- Retail-grade polish (cinematics, voice acting, etc.)

---

## 2. Repo restructure

### 2.1 Current shape (the problem)

This repo (`azerothcore-heimdal`) is a *build orchestrator with overlays*, not a fork. AC and mod-playerbots aren't checked in — `image/build.sh` clones them at build time and applies `patches/*` as rsync overlays. ToT product code (harness daemon, brain sidecar, memory subsystem, dbc-patch-builder, deploy/image/quadlet) is mixed with the orchestration script. The "edit AC source" workflow today means "edit a file in `patches/ac-bfd-raid/overlays/`, rebuild on Heimdal, see what breaks" — exactly the pain point a real fork eliminates.

### 2.2 Target layout (`threads-of-time/`)

This repo, renamed, becomes a real hard fork of AC. Top-level mirrors AC's tree so rebasing onto upstream is trivial. ToT additions live under a `tot/` namespace so the AC-vs-ToT seam is visually obvious:

```
threads-of-time/
├── src/ deps/ apps/ data/          ← forked AC, edited directly
├── modules/                        ← ToT-authored AC modules only; mod-playerbots is an external dependency operators install separately
│   ├── mod-agenticbots/            ← ToT's bot-AI layer; depends on mod-playerbots's public API
│   ├── mod-bracket-sets/
│   │   └── client/                 ← DBC additions + tooltip authoring
│   ├── mod-rotation-mode/
│   ├── mod-warforged/
│   │   └── client/                 ← FrameXML overrides + sentinel addon + DBC additions
│   └── mod-harness-bridge/         ← server-side only; no client assets
├── tot/
│   ├── harness/                    ← FastMCP daemon (port 8099)
│   ├── brain/                      ← V3 brain sidecar (single-endpoint; router decoupled)
│   ├── memory/                     ← V3 memory subsystem
│   ├── client-patch/               ← multi-source MPQ compositor + collision detection
│   ├── content/                    ← SQL packs (Bracket 1, BFD raid, etc.)
│   ├── release/                    ← release pipeline, build orchestrator, CI configs
│   ├── deploy/                     ← reference Quadlet + Compose stack
│   └── internal-docs/              ← Heimdal-internal stuff (model router, dev notes, in-flight specs)
├── docs/                           ← public-facing: install, upgrade, BYOLLM setup
├── UPSTREAMS.toml                  ← pinned AC SHA (only AC; mod-agenticbots no longer pinned)
└── README.md
```

Why the `tot/` namespace: anything outside `tot/` is "what AC owns" → that's the diff surface for the patch-series generator. Anything inside `tot/` is "what ToT adds" → ships as files, not as patches.

### 2.3 Branching model

- **`main`** — stable, release-tagged, every commit buildable. Where releases get cut from.
- **`dev`** — active development. Day-to-day work lands here.
- **`release/X.Y`** — release stabilization branches cut from `dev` near version-freeze; only patch-level fixes land. Tagged when ready.
- **`upstream/ac`** — tracks stock AC. Used for the AC-upgrade workflow.

mod-agenticbots is ToT-authored from scratch — there is no upstream to track. mod-playerbots is consumed at runtime as an installed module, not vendored in the repo, so no `upstream/playerbots` branch.

ToT also pins a **compatible mod-playerbots version range** in `UPSTREAMS.toml` for documentation + install-script verification — e.g., `[playerbots-dependency] min = "<version>"`, `tested = "<version>"`. ToT does not vendor mod-playerbots's source; this entry exists only to express the dependency contract.

### 2.4 Migration from current state (one-time work)

A migration plan, not a daily workflow:

1. New empty `threads-of-time` repo, init from stock AC at chosen pinned SHA
2. Author `modules/mod-agenticbots/` as a new module: extract the ToT-specific bot additions currently expressed as `patches/mod-playerbots-bfd/` (custom strategies, actions, triggers, the harness integration glue) into a self-contained module that calls into mod-playerbots's public API. Anything that was modifying mod-playerbots internals gets either (a) rewritten to use the public API, (b) submitted as a PR upstream to mod-playerbots, or (c) shipped as a documented "compatible with mod-playerbots ≥ vX.Y.Z" requirement
3. Extract the AC-core hooks that mod-playerbots's AC fork carries (PlayerbotAI accessors, hook points, etc.) and commit them as ToT's own commits in `src/`. This keeps ToT on stock AC base while satisfying mod-playerbots's AC-core requirements without depending on mod-playerbots's AC fork
4. Replay AC-side patch series as real commits in the fork (not overlay files):
   - `patches/ac-bfd-raid/` (3 patches + overlays — BFD raid map override, boss-stub registration, 3-day lockout)
   - `patches/ac-bracket-sets/` (1 patch — `OnPlayerBuildItemQueryResponse` PlayerScript hook, load-bearing for mod-bracket-sets's itemset rewrite; clean hook shape, candidate for upstream AC PR)
5. Move `modules/mod-bracket-sets`, `mod-rotation-mode`, `mod-warforged`, `mod-harness-bridge` over as-is
6. Merge mod-warforged from `feat+mod-warforged-v1` worktree into the new repo
7. Restructure ToT product code from current paths into `tot/{harness,brain,memory,client-patch,content,release,deploy}/`
8. Generalize `tools/dbc-patch-builder` → `tot/client-patch/` as a multi-source compositor
9. Move `image/build.sh` → `tot/release/build.sh` (now builds *from* the repo instead of *cloning* into it)
10. Decouple V3.7.4 brain's router logic into `tot/internal-docs/heimdal-router/` (or its own repo, deferred)
11. Drop PoC litter (`backend.log`, `sod-import-*.log`, stale logs)
12. `docs/superpowers/specs/` and `plans/` move to `tot/internal-docs/` (kept for history, excluded from release artifacts)
13. `lua/` either moves under `tot/content/lua/` or gets retired if not used in current build

### 2.5 What this repo (`azerothcore-heimdal`) becomes

After migration: archived (read-only on GitHub, marked "see `threads-of-time`"). Heimdal as a *name* survives as the dev deployment ("Heimdal is the box ToT is developed on"), but it's no longer a separate repo identity. The harness/brain/memory/etc. ARE Threads of Time.

---

## 3. Versioning + handshake

### 3.1 Version scheme

SemVer with bracket-anchored majors. `MAJOR.MINOR.PATCH`:

- **MAJOR** — bracket scope expansion. `1.X.X` = Bracket 1, `2.X.X` = Bracket 2, etc. Breaking changes for existing player characters live here.
- **MINOR** — additive features within a bracket. New content, new bot capabilities, new mod-agenticbots strategies, new tier set, new dungeon. Backwards-compatible.
- **PATCH** — fixes, balance tweaks, AC-upstream catch-ups that don't change content semantically.

Pre-release tags: `1.0.0-rc.1`, `1.0.0-rc.2`. `1.1.0-dev` on `dev` between releases.

### 3.2 What triggers each bump

| Change | Bump |
|---|---|
| Bracket cap raised, new zones unlocked | MAJOR |
| New raid, new tier set, new mod-agenticbots subsystem | MINOR |
| New non-essential mod-agenticbots strategy | MINOR |
| Tier-set bonus number tweak | PATCH |
| AC-upstream SHA catch-up (no semantic change) | PATCH |
| Brain prompt-tuning, default model swap recommendation | PATCH |
| Server protocol change requiring client re-patch | MINOR (carries new client MPQ) |

### 3.3 Version-string surface

| Location | Format |
|---|---|
| Container image tags | `ghcr.io/threadsoftime/worldserver:1.0.0`, `:harness-1.0.0`, etc. |
| Compose/Quadlet stack file | `# Threads of Time 1.0.0 reference stack` |
| Worldserver MotD on login | `Welcome to Threads of Time 1.0.0` |
| Login-screen text (client MPQ) | `World of Warcraft\nThreads of Time 1.0.0\n\n[fan project disclaimer]` |
| Realm announce on join | `[ToT 1.0.0] Welcome, <name>.` |
| Client MPQ filename | `patch-Z-tot-1.0.0.MPQ` |
| `worldserver --version` | `worldserver 1.0.0 (ToT 1.0.0, AC base abc1234)` |
| Harness `obs.version` tool | `{"tot": "1.0.0", "ac_base_sha": "abc1234", "build_date": "..."}` |

### 3.4 Client/server compat handshake — compat range model

Server declares `min_client = 1.0.0`, `max_client = 1.X.X` in config. Within a major version, minor/patch upgrades stay client-compatible (the client MPQ is *additive*). Cross-major requires a client upgrade.

Implementation: a new `mod-tot-handshake` adapter (or piggyback on mod-bracket-sets) reads the client's GlueStrings.dbc version string on `WorldSession::HandleAuthSession`. Mismatched-but-acceptable clients get an in-game MotD warning suggesting they update. Mismatched-and-unacceptable clients are disconnected with a clear "please update" message.

### 3.5 Subcomponent versioning

mod-agenticbots, brain sidecar, memory subsystem, harness daemon, and client MPQ all version in lockstep with ToT. No separate version numbers. Internally each has a build-time constant including the git SHA (e.g., `MOD_AGENTICBOTS_VERSION = "1.0.0+abc1234"`); operators and players see only "ToT 1.0.0."

### 3.6 External dependency versioning

ToT depends on **mod-playerbots** as an external module installed by the operator. The dependency is expressed as a compatible version range in `UPSTREAMS.toml` (e.g., `min = "<min mod-playerbots tag>"`, `tested = "<exact tag CI validated against>"`). The install script (§6.2) verifies the operator's mod-playerbots install satisfies the range and refuses to start if not. ToT does not ship, distribute, or modify mod-playerbots — it is an operator-installed dependency, same model as MySQL or the LLM endpoint.

**Important caveat — build-time vs runtime:** mod-playerbots is needed at **both** build time and runtime, not just runtime. mod-harness-bridge's tool adapters and (1.1.0+) mod-agenticbots's BFD strategy glue both `#include` mod-playerbots headers and link against its symbols. Implications by operator path:

- **Pre-built container operators (the primary distribution path):** mod-playerbots is baked into the worldserver image by ToT's CI before publishing to GHCR. Operators pulling `ghcr.io/threadsoftime/worldserver:X.Y.Z` get a self-contained image; no separate mod-playerbots install required at deploy time.
- **Source-build operators:** Must clone mod-playerbots into `modules/mod-playerbots/` BEFORE running the build. The build pipeline (`tot/release/build.sh`) rsyncs/clones it into the ephemeral build tree.

ToT's "no redistribution" stance (§10.2) is preserved either way: the ToT git repo never contains mod-playerbots source; the build-tree copy is ephemeral and doesn't appear in any ToT release artifact.

---

## 4. Dev workflow

### 4.1 Repo split: what builds where

| Surface | Lives in | Builds on | Why |
|---|---|---|---|
| **Native C++** — worldserver, mod-agenticbots, mod-bracket-sets, mod-rotation-mode, mod-warforged, mod-harness-bridge | `src/`, `modules/*/` | Heimdal (`-j4` cap, `kb_adbafeda`) | macOS toolchain mismatch + AC's build deps; Heimdal also has the running pod to test against |
| **Python sidecars + tooling** — brain sidecar, harness daemon, memory subsystem, client-patch compositor, release pipeline | `tot/{brain,harness,memory,client-patch,release}/` | Local laptop (preferred) + Heimdal for integration | Python has unit tests that run fast locally; full integration requires the worldserver pod |

Standing rule: native-side work is "edit locally over SSHFS/sync → build on Heimdal → cross-check binary mtime per `kb_f974fc65` → restart pod." Python-side work is "edit + pytest locally → sync to Heimdal only for integration testing."

### 4.2 Editing AC source

Open the file, edit it, commit. No more overlay-patch round-trips. At release time, the pipeline calls `git format-patch upstream/ac..HEAD -- src/ deps/ apps/ data/` and the diff becomes the shipped patch series. Patches are an *output*, not an input.

### 4.3 Adding a new mod-agenticbots strategy

Standard strategy/action/trigger pattern (per `cpp-systems-engineer`'s domain): new files under `modules/mod-agenticbots/src/strategy/<class>/`, wire into the strategy factory, SQL seed for any new config rows in `modules/mod-agenticbots/data/sql/`. Add a chat-command toggle if the brain should be able to enable/disable it via `bot.set_strategy`.

### 4.4 Adding a new ToT-specific feature

- **Content** (new SQL, new bracket bonus, new raid encounter): under `tot/content/`. Migrations follow `YYYY_MM_DD_NN_description.sql`.
- **Brain behavior** (new prompt mode, new tool-call discipline, new memory salience rule): under `tot/brain/`, with unit tests in `tot/brain/tests/`.
- **Harness** (new `bot.*` / `obs.*` / `gm.*` / `memory.*` tool): hand off to `agentic-harness-engineer`. Adapter goes in `modules/mod-harness-bridge/`, schema in `tot/harness/tool_schemas.py`, parity test in `tot/harness/tests/test_mcp_schemas.py` per `kb_642162c3`.
- **Client-visible content** (new DBC rows, new FrameXML override, new addon): under `modules/<mod>/client/`. Compose pipeline picks it up automatically; reserve DBC ID range in `modules/<mod>/client/MANIFEST.toml`.

### 4.5 The local dev stack (BYOLLM)

```
docker compose -f tot/deploy/dev-stack.compose.yml up
```

Spins up: worldserver (built locally OR pulled from `:dev-latest`), harness daemon, brain sidecar, memory store. Contributor brings their own LLM endpoint via `BRAIN_LLM_URL`. Cheap default: `http://localhost:11434/v1` for Ollama with `qwen2.5:14b-instruct` (chat) + `nomic-embed-text` (embeddings).

### 4.6 Test discipline

Three tiers, all CI-gated:

- **Unit** — Python pytest per sidecar + compositor determinism check. Every commit. Fast.
- **Schema parity** — `test_mcp_schemas.py` keeps `tool_schemas.py` and the C++ adapters from drifting. Every commit.
- **Integration** — full stack on Heimdal: spin up dev compose stack, run end-to-end harness tool calls, smoke-test bot behaviors. PR-to-`dev` and every RC. Slow but catches the bugs `kb_6950a902` documents.

### 4.7 Pre-commit discipline

A `pre-commit` hook enforces:
- `cmake` reconfigure marker bumped if any new `.cpp` files added under `src/`, `modules/`, or `tot/` C++ subdirs (per `kb_57b453cd` Step 3)
- SQL migration filenames follow the dated convention
- No `LOG_FATAL`-only logging without a corresponding `LOG_ERROR` for ops visibility
- No `rm -rf` in shell scripts (per CLAUDE.md non-negotiables)
- SPDX header on every new source file

### 4.8 PR + merge flow

1. Feature branch off `dev` → PR back to `dev` → squash-merge on green CI
2. When `dev` accumulates a release's worth of features: cut `release/X.Y` from `dev`
3. Only patch-level fixes merge to `release/X.Y` from here
4. Tag `vX.Y.0` on `release/X.Y` when ready → CI builds + publishes release artifacts (§5)
5. Merge `release/X.Y` back into `dev` to bring patch-fixes forward
6. Fast-forward `main` to the tagged commit

---

## 5. Release pipeline

### 5.1 The release command

A single entry point:

```
tot/release/release.sh vX.Y.Z
```

What it does, in order:

1. Verifies `git status` clean, on `release/X.Y` branch, tag `vX.Y.Z` not already used
2. Reads `UPSTREAMS.toml` for pinned AC SHA
3. Runs the full test suite (unit + schema parity); aborts on red
4. Generates the AC patch series: `git format-patch upstream/ac..HEAD -- src/ deps/ apps/ data/` → `release-artifacts/ac-patches/`
5. Builds container images via `tot/release/build-images.sh` (worldserver, harness, brain, memory) → tags `:X.Y.Z`
6. Composes the client MPQ via a two-stage pipeline:
   - **Stage A (unified AddOn):** `tot/client-patch/compose-tot-addon.py` scans every `modules/<mod>/data/addon-contrib/manifest.toml`, reads the per-mod `priority` + `files` ordering, and concatenates each mod's Lua contributions into a single `Interface/AddOns/ThreadsOfTime/` folder with an auto-generated `00-Core.lua` (namespace init) + `ThreadsOfTime.toc`. Files are prefixed by (priority, mod, file-index) so per-mod LOC stays attributable on the installed AddOn folder.
   - **Stage B (MPQ pack):** `tot/client-patch/pack-mpq.py` takes (a) the unified AddOn folder from Stage A, (b) DBC additions from every `modules/<mod>/client/` directory, (c) any FrameXML overrides that still ship as FrameXML rather than AddOn (deprecated; new work uses AddOns), and packs them into a single `patch-Z-tot-X.Y.Z.MPQ`. DBC row-ID collision detection runs at this stage; aborts with a clear error if two mods claim the same ID. Output is byte-deterministic given identical sources (existing dbc-patch-builder `--check` discipline).
7. Bundles the reference deploy stack (`tot/deploy/compose.yml`, `tot/deploy/quadlet/`, env templates, install README) → `release-artifacts/tot-X.Y.Z-stack.tar.gz`
8. Computes SHA-256 checksums for all artifacts → `release-artifacts/SHA256SUMS`
9. Tags `vX.Y.Z` locally, pushes tag + branch
10. Pushes container images to GHCR (`ghcr.io/threadsoftime/{worldserver,harness,brain,memory}:X.Y.Z`)
11. Creates GitHub Release from the tag, uploads artifacts + checksums + auto-generated changelog

Aborting at any step is safe — nothing is published until step 9.

### 5.2 What gets published

| Artifact | Filename | Purpose |
|---|---|---|
| Client MPQ | `patch-Z-tot-X.Y.Z.MPQ` | Players download, drop into `Data/` |
| Reference deploy stack | `tot-X.Y.Z-stack.tar.gz` | Operators clone, edit `.env`, deploy |
| AC patch series | `tot-X.Y.Z-ac-patches.tar.gz` | For operators who want to build worldserver themselves |
| Install script | `install-tot.sh` | One-command bootstrap |
| Source tarball | `tot-X.Y.Z-source.tar.gz` | GPL-compliance, audit |
| Checksums | `SHA256SUMS` | Verification |
| Changelog | Rendered in release body | Auto-generated from commits + hand-edited |

Container images live in GHCR, not as release assets — operators pull by tag.

### 5.3 Container image strategy

- `ghcr.io/threadsoftime/worldserver:X.Y.Z` — built from forked AC tree, includes all ToT C++ modules compiled in
- `ghcr.io/threadsoftime/harness:X.Y.Z` — FastMCP daemon
- `ghcr.io/threadsoftime/brain:X.Y.Z` — V3 brain sidecar
- `ghcr.io/threadsoftime/memory:X.Y.Z` — memory subsystem

Per-image tags: `:X.Y.Z` (exact), `:X.Y` (latest patch in minor), `:X` (latest minor in major), `:latest` (latest stable). Pre-releases tag `:X.Y.Z-rc.N` only — never roll up to `:latest`.

The worldserver image builds on Heimdal (self-hosted runner) under the `-j4` discipline. Python sidecar images build on GitHub-hosted runners.

### 5.4 CI cadence

- **On push to `dev` or any PR:** unit tests, schema parity, lint, compositor determinism check. ~3 min on GitHub-hosted runners.
- **On push to `release/X.Y`:** above + integration tests on Heimdal self-hosted runner. ~15 min.
- **On tag `vX.Y.Z*`:** above + the full release pipeline (§5.1). ~30–45 min including worldserver build on Heimdal.
- **Nightly on `dev`:** smoke-test "rebase onto `upstream/ac@HEAD`" — does the current `dev` still cleanly apply on top of bleeding-edge AC? Posts a report to a GitHub issue if conflicts appear.

### 5.5 Reproducibility

- **Client MPQ compositor** is deterministic (existing dbc-patch-builder `--check` flag pattern, generalized to multi-source). CI gate stays.
- **Container builds** pin base images by SHA, pass `SOURCE_DATE_EPOCH`, disable cache for release builds.
- **AC patch series** generation deterministic given a fixed `git format-patch` version + same source tree.
- **Source tarball** uses `git archive` with `SOURCE_DATE_EPOCH`.

### 5.6 Signing + provenance (deferred for 1.0.0)

SHA-256 checksums in `SHA256SUMS` are the verification surface for 1.0.0. GPG-signed tags, cosign-signed container images, and SBOMs deferred to 1.0.x or 1.1.0.

### 5.7 Cadence + tagging discipline

- **MAJOR releases** (`2.0.0`): when a new bracket ships. Months apart.
- **MINOR releases** (`1.1.0`): when accumulated `dev` work crosses a useful boundary. Probably every few weeks during active dev.
- **PATCH releases** (`1.0.1`): on demand. AC catch-up, security fix, balance hotfix.

Tagging is irrevocable: a tag `vX.Y.Z` once pushed is never moved or deleted. Mistakes mean a new patch version.

### 5.8 Client MPQ composition + collision discipline

**Two manifest surfaces, two collision concerns.**

**1. AddOn-side manifest (live as of 2026-05-25, commit `82a6583`)** — Each mod contributing Lua/UI declares its load-order in `modules/<mod>/data/addon-contrib/manifest.toml`:

```toml
# Per-mod contribution manifest. Read by tools/compose-tot-addon.py.
#
# priority — lower numbers load earlier. Core/shared files use 0-9; feature
# mods use 10-89; integration/glue uses 90-99.
#
# files — load order within this contribution. Files outside this list are
# IGNORED by the composer (lets you keep WIP files alongside production).

mod = "mod-warforged"
priority = 10

files = [
    "WarforgedStatBumps.lua",   # defines WF_STAT_BUMPS table; load first
    "GameTooltip.lua",          # consumes WF_STAT_BUMPS
    "ItemRef.lua",              # same shape but for ItemRefTooltip
    "WarforgedSentinel.lua",    # CHAT_MSG_ADDON responder
]
```

The composer enforces unique `(priority, mod, file)` tuples and detects when two mods would inject the same global symbol (greppable conflict detection). New mods pick an unused priority band; the project-wide AddOn-priority allocation lives at `tot/client-patch/PRIORITIES.md`.

**2. DBC-side manifest (still future as of 2026-05-27)** — DBC ID collision discipline for the MPQ pack stage is not yet implemented. Plan 4 adds `modules/<mod>/client/MANIFEST.toml` for DBC ID ranges:

```toml
[manifest]
mod = "mod-bracket-sets"
client_assets = ["DBFilesClient/ItemSet.dbc", "DBFilesClient/Spell.dbc"]

[id_ranges]
"ItemSet.dbc"  = { min = 90100, max = 90199 }
"Spell.dbc"    = { min = 49000, max = 49999 }
```

`tot/client-patch/pack-mpq.py` will validate non-overlap at compose time and abort on conflict. Until Plan 4 lands, collision detection relies on author discipline + the existing per-module conventions (mod-bracket-sets uses 90100-range, mod-warforged uses 4080000+ for enchant rows, etc.).

---

## 6. Operator install experience

### 6.1 Prerequisites

| Requirement | Minimum | Recommended | Notes |
|---|---|---|---|
| OS | Linux x86_64 with podman 4+ or docker 24+ | Fedora/RHEL 9 + podman + systemd Quadlet | macOS dev-only |
| CPU | 4 cores | 8+ cores | `-j4` cap is build-time only |
| RAM | 8 GB | 16+ GB | Worldserver + 50 bots ~6 GB; brain + memory ~1 GB each |
| Disk | 20 GB | 50+ GB SSD | Images + MySQL + memory store growth |
| Database | MySQL 8.x or MariaDB 10.6+ | MySQL 8.x | AC requirement; ToT inherits |
| **mod-playerbots** | Installed alongside ToT modules — **build-time + runtime** | Latest mod-playerbots stable | Per-version compatibility range declared in `UPSTREAMS.toml`; install script verifies (§6.2). Pre-built container operators don't need to install separately (ToT's CI bakes mod-playerbots into the worldserver image). Source-build operators must clone mod-playerbots into `modules/mod-playerbots/` BEFORE running the build — mod-harness-bridge's adapters and (1.1.0+) mod-agenticbots's BFD glue both need its headers at compile time. ToT does not vendor mod-playerbots source (no redistribution per §10.2); the build pipeline rsyncs/clones it into the ephemeral build tree only. |
| LLM endpoint | OpenAI-compatible chat + embeddings | Local Ollama with `qwen2.5:14b-instruct` + `nomic-embed-text` | BYOLLM |
| WoW 3.3.5a client | Player-side | — | Players install MPQ separately |

ToT does **not** require a GPU on the worldserver host — the LLM runs wherever the operator points `BRAIN_LLM_URL`. Typical small-scale: worldserver + sidecars on a modest VPS, LLM on a separate GPU box (or hosted API).

### 6.2 Install — the happy path

```
curl -fsSL https://github.com/threadsoftime/threads-of-time/releases/download/v1.0.0/install-tot.sh | sh
```

What `install-tot.sh` does:

1. Verifies podman or docker; clear-message abort if missing
2. Verifies mod-playerbots is reachable (either already installed in `modules/mod-playerbots/` of a local AC checkout, or offers to clone the upstream mod-playerbots repo at the version range ToT requires)
3. Creates `/opt/tot/` (or `$TOT_HOME`) with `compose.yml`, `quadlet/`, `.env.example`, `secrets/`, `data/`, `docs/`
4. Generates random MySQL passwords, writes to `secrets/`
5. Pulls all images by exact tag (`worldserver:1.0.0`, etc.) — the worldserver image is built with mod-playerbots + ToT modules compiled in, so step 2's verification is for source-build operators only
6. Prompts for: realmlist hostname, LLM endpoint URL + API key (skippable), Quadlet vs. Compose mode
7. Prints next-step instructions

Operators who want full control skip the script and download `tot-1.0.0-stack.tar.gz`.

### 6.3 The `.env` surface

```
# === Identity ===
TOT_VERSION=1.0.0
TOT_REALM_NAME="Threads of Time"
TOT_REALM_HOST=realm.example.com

# === Database (auto-generated) ===
MYSQL_ROOT_PASSWORD=<generated>
MYSQL_WORLD_USER=tot_world
MYSQL_WORLD_PASSWORD=<generated>
# ... etc.

# === LLM (BYOLLM) ===
BRAIN_LLM_URL=http://192.168.1.50:11434/v1
BRAIN_LLM_MODEL=qwen2.5:14b-instruct
BRAIN_LLM_API_KEY=
BRAIN_EMBEDDINGS_URL=http://192.168.1.50:11434/v1
BRAIN_EMBEDDINGS_MODEL=nomic-embed-text
BRAIN_EMBEDDINGS_API_KEY=

# === Harness ===
HARNESS_BEARER_TOKEN=<generated>
HARNESS_BIND=0.0.0.0:8099

# === Bots ===
TOT_BOT_POPULATION=20
TOT_LIVING_BOT_COUNT=10                     # subset gating: of TOT_BOT_POPULATION, this many are brain-driven
TOT_BOT_MIN_LEVEL=1
TOT_BOT_MAX_LEVEL=25

# === Resource limits ===
TOT_WORLDSERVER_CPU_LIMIT=4
TOT_WORLDSERVER_MEM_LIMIT=6g
TOT_BRAIN_CPU_LIMIT=2
TOT_BRAIN_MEM_LIMIT=2g
```

`.env.example` carries comments explaining every variable. Install script never asks for values it can sensibly default. No multi-endpoint LLM fallback fields — operators who want fancy routing put a proxy like LiteLLM in front and point `BRAIN_LLM_URL` at it (documented in `docs/byollm-setup.md`).

### 6.4 First-boot bootstrap

On first `tot up`, an init container runs:

1. Waits for MySQL with retry/backoff (no race per `kb_9d1289dd`)
2. Creates DBs (`tot_auth`, `tot_characters`, `tot_world`, `tot_playerbots`)
3. Applies AC stock schema
4. Applies ToT schema deltas from `tot/content/sql/` in dated order
5. Seeds default realm row pointing at `TOT_REALM_HOST`
6. Seeds default GM accounts (`admin/admin` with "change me" notice)
7. Generates `secrets/harness-token` if absent
8. Marks complete in `data/bootstrap.flag` so subsequent boots skip

Idempotent; failures log the exact step + recovery suggestion and exit non-zero.

### 6.5 Day-2 ops

- **Logs:** `podman logs threadsoftime-<service>`. Brain logs decision-loop traces (perception → reason → tool-call → act → reflect) at INFO; DEBUG via `BRAIN_LOG_LEVEL=DEBUG`.
- **Backups:** `tot-backup.sh` does `mysqldump` of all four DBs + file snapshot of `data/memory/memory.sqlite` (with WAL checkpoint first). 7 daily + 4 weekly retention by default.
- **Monitoring:** Harness exposes `obs.health`, `obs.ping`, and `/metrics` Prometheus endpoint. Reference Grafana dashboard at `tot/deploy/grafana/dashboard.json`.
- **Scaling:** Bot population via `TOT_BOT_POPULATION`. Reload by `tot restart brain`. CPU-bound; thermal lessons from `kb_adbafeda` apply.

### 6.6 Upgrade path

- **Within a minor** (`1.0.0 → 1.0.1`): `tot upgrade 1.0.1`. Snapshots DBs, pulls new images, stops sidecars, runs new SQL migrations, restarts sidecars on new tag, swaps worldserver (~30s downtime), verifies healthy → done.
- **Across a minor** (`1.0.0 → 1.1.0`): same flow + operator must distribute new client MPQ to players first. `tot upgrade` prints reminder and prompts.
- **Across a major** (`1.0.0 → 2.0.0`): requires reading the release's `UPGRADE.md`. `tot upgrade` refuses by default.

### 6.7 Rollback

DB migrations are forward-only per AC convention. Rollback flow:

1. Restore pre-upgrade DB snapshot from `backups/`
2. `tot upgrade 1.0.0` (treat as downgrade-to-older-version)
3. Container images by exact tag always available from GHCR

`tot upgrade` does NOT auto-rollback on failure — leaves failed state in place, points at recovery doc.

### 6.8 Failure modes the operator will hit

Documented in `docs/operator-troubleshooting.md`:

- **MySQL race after host reboot:** wow-authserver fails connecting because wow-database isn't ready. Recovery per `kb_9d1289dd`. Reference Quadlet ships `Restart=on-failure RestartSec=15` to self-heal most cases.
- **BYOLLM endpoint unreachable:** brain logs timeout, bots keep last goal but don't take new actions. Worldserver + bots' raw capabilities stay alive. Operator fixes LLM, brain reconnects without restart.
- **Memory store corruption:** SQLite WAL recovery handles most cases; if not, restore from backup. Memory data is per-bot, not load-bearing for gameplay.
- **Out of disk:** memory store grows over time. Reference monitoring alerts at 80%; `tot memory vacuum` compacts old episodes.

---

## 7. Player install experience

### 7.1 What the player needs

ToT does not distribute the WoW client. The player brings:

| Item | Source | Notes |
|---|---|---|
| WoW 3.3.5a client (build 12340) | Player's own | "Wrath of the Lich King" retail at 3.3.5a patch. ToT does not link to or distribute this. |
| ToT client MPQ | GitHub Releases | `patch-Z-tot-1.0.0.MPQ`, downloaded directly |
| Realmlist info | The operator | Hostname like `realm.example.com` |

### 7.2 Install — three steps

1. **Copy the MPQ into the client's `Data/` folder.**
   - Windows: `<WoW install>\Data\patch-Z-tot-1.0.0.MPQ`
   - macOS / Wine: same relative path
2. **Edit `Data/<locale>/realmlist.wtf`** (e.g., `Data/enUS/realmlist.wtf`):
   ```
   set realmlist realm.example.com
   ```
3. **Launch `Wow.exe`.** Log in with the account the operator gave you.

The MPQ is additive — doesn't modify stock client files. Reversible by deletion.

### 7.3 Verifying the install

- **Login screen:** version watermark reads `Threads of Time 1.0.0` (with fan-project disclaimer). If stock text, MPQ isn't loaded.
- **Character select:** realm name matches operator's.
- **In-game, hover any Bracket 1 tier piece:** tooltip shows spec-specific bonus rows. Stock `Vestiges of Blackfathom (0/2)` with no bonus rows = MPQ not loaded.
- **Loot a Bracket 1 item:** roughly 10% chance of seeing an orange Warforged tag under the item name in the tooltip.

### 7.4 Upgrading

Server's MotD warns when client is behind (compat-range model per §3.4). To upgrade:

1. Delete old MPQ
2. Download new MPQ from release page
3. Drop into `Data/`

Same as initial install minus the realmlist step.

### 7.5 Common errors

Documented in `docs/player-troubleshooting.md`:

| Symptom | Cause | Fix |
|---|---|---|
| `Login server unavailable` | Realmlist wrong or operator's server down | Re-check `realmlist.wtf`; ping the operator |
| Tooltips show no bonus rows / no Warforged tag | MPQ not loaded | Re-check filename + location |
| `Disconnected: client/server version mismatch` | Compat-range hard reject | Update MPQ to required version |
| Bots feel "dead" — walk around but don't talk | Server-side BYOLLM issue | Tell the operator |

### 7.6 Optional: addons

ToT does not require any addons. Standard 3.3.5a addon ecosystem works. `ToTBotChat` (deferred to 1.1.0) will surface bot personality indicators in a dedicated chat panel.

### 7.7 What 1.0.0 does *not* ship player-side

- No launcher (manual MPQ install; launcher = 1.1.0+ candidate)
- No client distribution (won't help players find a client; same legal posture)
- No anti-cheat / Warden equivalent (community-server trust model)

---

## 8. AC upgrade workflow

### 8.1 Why catch up to AC

- Security fixes
- Performance improvements
- Retail-emulation correctness bugfixes (quests, spells, AI)
- Dependency catch-ups
- New AC features ToT wants to expose

What ToT does *not* track from AC: new content (ToT defines its own), AC's bot module (ToT has mod-agenticbots), config-flag features ToT keeps off.

### 8.2 Cadence

| Trigger | Action | Becomes ToT version |
|---|---|---|
| AC tags a new release | Schedule catch-up within 2 weeks | PATCH |
| AC ships a security fix | Catch-up within 1 week | PATCH (out-of-band) |
| AC introduces a feature ToT wants | Catch up + author integration | MINOR |
| Nightly CI conflict report (§5.4) | Triage; fix-and-merge or defer | (no bump until acted on) |

Nightly CI tells you whether `dev` cleanly applies on `upstream/ac@HEAD`. Early warning instead of release-cut surprises.

### 8.3 The dry-run

```
# 1. Fetch the latest AC upstream
git fetch upstream/ac

# 2. Create a throwaway rebase branch
git checkout -b dryrun/ac-<new-sha> dev

# 3. Rebase ToT onto the new AC base
git rebase --onto <new-ac-sha> $(cat UPSTREAMS.toml | grep '^sha' | head -1 | cut -d'"' -f2)

# 4. If clean: build worldserver image locally
tot/release/build-images.sh worldserver --tag dryrun-<new-sha> --base-sha <new-ac-sha>

# 5. Boot the dry-run stack alongside production
tot/release/dryrun-stack.sh up --image-tag dryrun-<new-sha>

# 6. If green: promote
#    → UPSTREAMS.toml gets the new AC SHA committed
#    → PR to dev → review → merge → cut release/X.Y.{Z+1}
```

Dry-run runs on Heimdal in isolated namespace + ports. No production impact.

### 8.4 When the dry-run fails

- **Code-level rebase conflict:** AC refactored something ToT touches. Resolve in rebase branch. Usually 5 min; occasionally a real refactor.
- **Build break:** AC removed/renamed/changed a function ToT calls. Patch ToT to match.
- **Runtime / test break:** AC changed behavior ToT silently depended on. Worst case — reproduce, understand, adapt. Dry-run stack stays up (`--keep`) for inspection.

All three resolve via commits on `dryrun/ac-<new-sha>`, which then squash-merges to `dev` with new `UPSTREAMS.toml` SHA.

### 8.5 The "hot swap" promotion

Once dry-run passes and PR merges:

```
tot upgrade 1.0.1
```

Same flow as any patch upgrade (§6.6). Nothing special about an AC catch-up release from the operator's POV.

### 8.6 When AC's change is not compatible

Two outs:
- **Skip the catch-up.** ToT stays on pinned SHA. Nightly CI keeps flagging drift; schedule a refactor sprint when it accumulates.
- **Carry a permanent reversion patch.** ToT's patch series gets a commit that undoes the upstream change. Only when carrying the patch costs less than chasing upstream.

Document each in `tot/internal-docs/ac-divergences.md`.

### 8.7 mod-agenticbots upgrades

There are none — mod-agenticbots no longer tracks upstream. Changes ride along with whatever ToT version their commits land in. §8 is AC-only.

---

## 9. Scope freeze for 1.0.0

### 9.1 What ships in 1.0.0

| Component | Status (as of 2026-05-27) |
|---|---|
| L1–25 leveling | Shipped + live |
| 54 tier-set bonuses (Bracket 1) | Shipped 2026-05-17 (`kb_99501ac5`) |
| Tier-set UI client patch | Shipped + iterated: dbc-patch-builder 16-locale ItemSet fix (`b8e5775`), item-query hook DetermineSpec fix (`787b55f`); the new `OnPlayerBuildItemQueryResponse` AC hook landed today (`4ef799d`, `patches/ac-bracket-sets/`) |
| BFD raid (Bracket 1) | Shipped + live |
| mod-bracket-sets | Shipped + live; AC-side hook now load-bearing (`patches/ac-bracket-sets/`) |
| mod-rotation-mode | Shipped + live |
| **mod-warforged** (Warforged proc + Bonus Sockets) | **Merged to master `8e1e1f6` (2026-05-25); shipped + live.** Iterated post-merge: socket-proc level gate default 56 (`ab24b73`), pivot from FrameXML overrides to AddOn-shipped Lua v1.0.2 (`64ffa26`) |
| **Unified `ThreadsOfTime` AddOn composer** | **Shipped (`82a6583`, 2026-05-25):** `tools/compose-tot-addon.py` scans `modules/<mod>/data/addon-contrib/manifest.toml` and produces a single composed AddOn folder. Replaces per-mod AddOns. First consumer: mod-warforged. mod-bracket-sets has `data/addon-contrib/.gitkeep` ready for its tier-set UI Lua contribution |
| `patches/ac-bracket-sets/OnPlayerBuildItemQueryResponse` hook | Live on Heimdal. Clean PlayerScript hook shape (CALL_ENABLED_HOOKS); candidate to PR upstream to AC. Until accepted, ToT carries it |
| mod-harness-bridge V1.4 | Shipped (merged at `988cb67`) |
| mod-agenticbots (new module, depends on mod-playerbots) | Needs authoring — extract ToT-specific bot additions from `patches/mod-playerbots-bfd/` into a self-contained module calling mod-playerbots's public API (per §2.4 step 2) |
| Harness daemon (`obs.*`, `gm.*`, `bot.*`) | Shipped |
| Brain sidecar | V3.7.4 shipped (`565f5b7`); single-endpoint only (router decoupled to Heimdal-internal — §9.4) |
| V3 memory subsystem | Multi-week work; design pending (Plan 2) |
| Subset gating (5–15 living bots of 100) | Design + implementation pending (Plan 3) |
| MPQ pack stage with DBC collision detection | AddOn composition done; MPQ pack collision detection still future (Plan 4) |
| Login-screen branding (GlueStrings overrides + fan-project disclaimer) | Needs authoring (Plan 4 or 5) |
| Reference Compose + Quadlet stack (operator-portable) | Needs generalization from Heimdal-specific (Plan 5) |
| Install script + first-boot bootstrap | Doesn't exist (Plan 5) |
| Operator + player docs | Don't exist (Plan 5) |

**The scope-freeze list is open until `release/1.0` is cut.** Additional modules may join before then (mod-warforged just did). Anything that lands in `dev` before scope freeze and meets the integration bar (composes cleanly into client MPQ, passes integration tests, has player + operator docs) is eligible. Anything after scope freeze goes to `dev` for 1.1.0.

### 9.2 Explicitly NOT in 1.0.0

| Item | Why deferred | Earliest possible |
|---|---|---|
| Brackets 2–7 | One-bracket-per-major | 2.0.0 |
| HD client (`kb_6354a4aa`) | Parked; massive scope | unscheduled |
| Launcher | Manual MPQ acceptable for early adopters | 1.1.0+ |
| ToTBotChat addon | Optional polish | 1.1.0+ |
| Threads of Time: Hosted | Business decisions needed | unscheduled |
| Anti-cheat / Warden | Community-server trust model | unscheduled |
| Additional raids beyond BFD | Bracket-2 territory | 2.0.0 |
| Signed releases (cosign, GPG, SBOM) | Real public-project polish | 1.0.x or 1.1.0 |
| Public bug tracker / Discord / forum | Infrastructure work, parallel track | parallel |
| Brand-asset polish (logo, website, splash art) | Real work, deferrable | 1.0.x |
| Multi-endpoint LLM routing | Heimdal-internal only | n/a (stays internal) |
| Tertiary stats, Titanforged, re-roll consumables | mod-warforged v2 territory | 1.1.0+ |
| BFD-tuned bot strategies (custom Strategy/Action/Trigger contexts for the BFD raid) | mod-playerbots upstream PR needed for the registration mechanism (see `tot/internal-docs/agenticbots-upstream-prs.md`); bots play BFD with generic strategies until then | 1.1.0+ |

### 9.3 Ship-or-defer decisions (resolved)

| Decision | Resolution |
|---|---|
| V3.8 multi-mode model routing | **Does not ship.** Heimdal-internal dev tooling. Shipped brain calls one OpenAI-compatible endpoint via `BRAIN_LLM_URL`. Router stays alive internally as a proxy in front of the brain on Heimdal. |
| V3 memory subsystem (clean-slate) | **Ships.** SQLite + sqlite-vec, hybrid retrieval, time-decay, per-bot project_id namespacing copied from ninum. `memory.*` tool surface via mod-harness-bridge. |
| Subset gating | **Ships.** Centerpiece of "alive bots" framing. Brain animates configurable subset; rest run stock mod-agenticbots strategies. Operator sets `TOT_LIVING_BOT_COUNT`. |
| Multi-mod client MPQ | **Compose into one.** Release pipeline merges DBC + FrameXML + Lua from all `modules/*/client/` sources into single `patch-Z-tot-X.Y.Z.MPQ`. Collision detection at compose time per §5.8. |

### 9.4 Pre-freeze checklist

| Item | Owner agent | Rough scope | Status |
|---|---|---|---|
| Repo restructure migration (§2) | Multi-agent | One-day dedicated sprint | Open — Plan 1 |
| Author mod-agenticbots as a new module (extract ToT-specific bot additions from current `patches/mod-playerbots-bfd/` into a self-contained module calling mod-playerbots's public API) | `cpp-systems-engineer` | 2–4 days (depending on how much of the current diff modifies playerbots internals vs. just adds ToT behaviors) | Open — Plan 1 Phase 5 |
| Extract AC-core hooks from mod-playerbots's AC fork into ToT's own `src/` commits | `cpp-systems-engineer` | One day | Open — Plan 1 Phase 2 |
| Replay `patches/ac-bracket-sets/` (1 hook) + `patches/ac-bfd-raid/` (3 patches + overlays) as ToT `src/` commits | `cpp-systems-engineer` | Half day | Open — Plan 1 Phase 4 |
| Pin mod-playerbots compatibility range in `UPSTREAMS.toml` + add install-script verification | `deploy-orchestrator` | Half day | Open — Plan 1 Task 19 |
| Decouple brain from model-router | `agent-orchestration-architect` | Half day | Open — Plan 1 Task 22 |
| V3 memory subsystem | `memory-system-designer` + `agentic-harness-engineer` + `agent-orchestration-architect` | Multi-week; critical path | Open — Plan 2 |
| Subset-gating selection logic | `agent-orchestration-architect` + `game-design-architect` | One week | Open — Plan 3 |
| **Unified `ThreadsOfTime` AddOn composer** (`tools/compose-tot-addon.py` + `data/addon-contrib/manifest.toml` pattern) | `cpp-systems-engineer` | — | **Shipped 2026-05-25 (`82a6583`)** |
| **mod-warforged merge to master** | `cpp-systems-engineer` | — | **Shipped 2026-05-25 (`8e1e1f6`)** |
| Generalize the MPQ pack stage with DBC ID-range manifest + collision detection (AddOn composition already done) | `agentic-harness-engineer` or `cpp-systems-engineer` | Half week | Open — Plan 4 |
| Tier-set UI client patch (AC hook + dbc-patch-builder iterations) | `agentic-harness-engineer` | — | **Effectively shipped (commits `b8e5775`, `787b55f`, `4ef799d`); awaits final composition into the unified MPQ** |
| Login-screen branding MPQ content (GlueStrings + disclaimer) | New work — needs author | One day | Open — Plan 4 or 5 |
| Reference Compose + Quadlet stack generalized | `deploy-orchestrator` | One week | Open — Plan 5 |
| Install script (`install-tot.sh`) | `deploy-orchestrator` | Half week | Open — Plan 5 |
| Operator docs (install, BYOLLM, upgrade, troubleshooting, legal) | n/a — write up | One week | Open — Plan 5 |
| Player docs (install, troubleshooting) | n/a — write up | Half week | Open — Plan 5 |
| CI pipeline (GitHub Actions + Heimdal self-hosted runner) | `deploy-orchestrator` | Half week | Open — Plan 6 |
| Nightly AC drift CI job | `deploy-orchestrator` | Two days | Open — Plan 6 |
| Branding (logo, splash) — minimum viable | External | Variable; placeholder OK | Open — parallel track |

Rough total: 4–6 weeks focused work to `v1.0.0-rc.1`, V3 memory on critical path.

---

## 10. Licensing + legal

> **Engineering note, not legal advice.** For commercial deployment, contested takedowns, or jurisdiction-specific questions, consult a lawyer.

### 10.1 ToT's own license

**GPL-2.0-or-later** for the entire project — C++ modules, Python sidecars, dbc-patch-builder + composition pipeline, reference deploy stack. Matches AzerothCore's license literally.

Reasoning: AC at the pinned SHA (`6d83e35d`) is licensed **GPL-2.0-or-later** (LICENSE = canonical GPL-2.0 text; source-file headers say "either version 2 of the License, or (at your option) any later version"). Matching AC's license literally gives the cleanest inheritance story and zero compatibility friction. The "or later" allowance means ToT *could* relicense forward (e.g., to AGPL-3.0 for a network-use clause); the decision was to stay at GPL-2.0-or-later to avoid imposing a surprising network-use obligation on operators that AC doesn't carry.

`LICENSE` at repo root is the GPL-2.0 text (copied verbatim from AC at the pinned SHA). Every source file gets `// SPDX-License-Identifier: GPL-2.0-or-later` (or `# SPDX-License-Identifier: GPL-2.0-or-later` for Python/shell).

### 10.2 Inherited licenses

| Upstream | License | ToT obligation |
|---|---|---|
| AzerothCore | GPL-2.0-or-later | Preserve LICENSE + per-file headers; ship source (§5.2); clearly mark ToT modifications. ToT's own license matches AC's literally (§10.1). |
| mod-playerbots | External dependency operators install; ToT does not redistribute it | The combined-work license question at runtime is between AC + mod-playerbots, and is the operator's question to answer — same as any AC operator who installs mod-playerbots today, with or without ToT. ToT documents the dependency and recommends operators verify mod-playerbots's license satisfies their use case. **No legal blocker for ToT 1.0.0.** |
| AC deps | Various permissive | AC's THIRDPARTY handles attribution; ToT inherits |

### 10.3 Blizzard IP posture

ToT distributes zero Blizzard assets:
- No WoW client redistribution; no links to client downloads
- No stock Blizzard DBC files in the MPQ — only ToT-original DBC rows (new ItemSet, new Spell, new enchant rows)
- No Blizzard music, art, sound, models, animations
- No Blizzard server-side data

Same legal posture as AC itself — a fan-built emulator/expansion requiring the player to bring their own legitimate client. ToT must remain non-commercial to stay in the tolerated category.

### 10.4 Trademark + naming

"World of Warcraft" is a Blizzard trademark. ToT's official name is **"Threads of Time"** alone. Descriptive references are fine and necessary: "an unofficial fan-built expansion for World of Warcraft: Wrath of the Lich King 3.3.5a" — nominative use, generally protected.

Required disclaimer on project README, release pages, official docs, and in-game login-screen MPQ override:

> Threads of Time is an unofficial, non-commercial fan project. It is not affiliated with, endorsed by, or sponsored by Blizzard Entertainment, Inc. World of Warcraft® and Wrath of the Lich King® are trademarks of Blizzard Entertainment. All Blizzard intellectual property remains the property of Blizzard Entertainment.

### 10.5 BYOLLM licensing

ToT distributes no LLM weights; model licenses are the operator's responsibility. `docs/byollm-setup.md` documents test/dev models with their licenses:

| Model | License | Notes |
|---|---|---|
| Qwen 2.5 series | Apache 2.0 | Safe for any deployment |
| Nomic Embed | Apache 2.0 | Safe for embeddings |
| Llama 3.x | Meta Llama Community License | Has restrictions; operator verifies |
| OpenAI API | OpenAI usage policies | Check current ToS |
| OpenRouter | Per-model | Inherits routed model's license |

ToT recommends Qwen + Nomic for clean license posture; doesn't mandate.

### 10.6 Contributor license

Inbound contributions accepted under GPL-2.0-or-later (matches AC; "inbound = outbound" default). `CONTRIBUTING.md` makes explicit:

> By submitting a pull request, you certify you have the right to license your contribution under GPL-2.0-or-later and you agree to do so.

No CLA, no DCO sign-off for 1.0.0.

### 10.7 What ToT explicitly does NOT do

- No donations, no Patreon, no merch
- No advertising in project, docs, or client MPQ
- No "premium" features, paid bot tiers, microtransactions
- No referral/affiliate links to LLM providers

### 10.8 Operator's responsibilities

`docs/operator-legal.md` covers:
- Player data + GDPR (operator-provided privacy policy)
- Age restrictions in some jurisdictions
- Content moderation (operator chose the LLM; operator owns its output)
- Anti-cheat (ToT ships none; operator decides)
- Commercial use crosses Blizzard's red line regardless of ToT's license

### 10.9 Pre-freeze legal checklist

| Item | Blocker for 1.0.0? |
|---|---|
| Use AC's GPL-2.0-or-later `LICENSE` (copy verbatim from AC at the pinned SHA) | Yes |
| Add SPDX headers to all new source files | Yes |
| Write `README.md` with fan-project disclaimer | Yes |
| Write `CONTRIBUTING.md` with inbound = outbound | Yes |
| Write `docs/operator-legal.md` | Yes |
| Write `docs/byollm-setup.md` model-license table | Yes |
| Privacy-policy template for operators | Nice-to-have |
| Login-screen MPQ disclaimer text | Yes |
| Grep audit confirming no Blizzard assets in MPQ + source | Yes |

A couple of days end-to-end. **No external license-verification dependency** (the prior mod-playerbots blocker dissolved when mod-agenticbots became a dependent module rather than a fork — §10.2).

---

## 11. Open questions + next steps

### 11.1 Open questions

| Question | Owner | When |
|---|---|---|
| Exact mod-playerbots compatibility range to pin in `UPSTREAMS.toml` (which tag is "tested," which is the minimum) | `cpp-systems-engineer` (after mod-agenticbots authoring lands) | Week 2 |
| V3 memory subsystem schema (episode shape, entity rows, embedding dim, sqlite-vec vs alternatives, time-decay) | `memory-system-designer` | Week 1 |
| Subset-gating selection algorithm (initial picks, rotation cadence, swap triggers, player nominations?) | `agent-orchestration-architect` + `game-design-architect` | Week 1–2 |
| Brain ↔ router decoupling: seam location, brain's outgoing interface, fate of V3.7.4 router code | `agent-orchestration-architect` | Week 1 |
| Login-screen branding text content | Contributor with copywriting hat | Week 2–3 |
| Reference realmlist hostname convention in install docs | `deploy-orchestrator` | Week 3 |
| DBC-side `MANIFEST.toml` schema for the MPQ pack stage (Plan 4) — what ID ranges, how the project tracks allocations, what collision-detection failure looks like | `cpp-systems-engineer` | Plan 4 start |
| Should `patches/ac-bracket-sets/OnPlayerBuildItemQueryResponse` be PR'd upstream to AC? If yes, when, and how do we want it framed? | Project-level + `cpp-systems-engineer` | Anytime; not blocking 1.0.0 |
| In-game GM tools surface for 1.0.0 beyond AC's stock | `cpp-systems-engineer` | Week 2 |
| Community channel: Discord/Matrix/forum or GitHub-only? | Project-level | Pre-release |
| Bug-reporting channel: GH Issues with templates or external? | Project-level | Pre-release |
| Logo + brand identity | External | Placeholder OK for 1.0.0 |
| Release target date | Project-level | After implementation plan estimates |

### 11.2 Concrete next steps

1. **Commit this spec** to `docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md`
2. **User reviews the committed spec.** Refinements before plan?
3. **Invoke `writing-plans`** to produce the implementation plan at `docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-implementation.md`
4. **Execute the plan in waves:**
   - **Wave 1**: Repo restructure migration. AC-core hook extraction into ToT `src/`. mod-agenticbots authored as new module. mod-playerbots compatibility range pinned. Brain ↔ router decoupling.
   - **Wave 2** (parallel): V3 memory implementation (longest item). Subset-gating implementation. Composed MPQ pipeline. mod-warforged merge to master.
   - **Wave 3** (parallel): Reference deploy stack. Install script + mod-playerbots dependency check. CI pipeline + nightly AC drift. Operator + player docs.
   - **Wave 4:** First RC. Smoke test on Heimdal. Iterate. Tag `v1.0.0`.

### 11.3 Rough timeline

Back-of-envelope: 4–6 weeks of focused work to `v1.0.0-rc.1`, V3 memory on critical path. With one person + dispatched agents, expect 6–8 weeks calendar time. Composition pipeline + memory subsystem are the two pieces most likely to surprise.

### 11.4 What "done" looks like for 1.0.0

A green checkbox per item, demonstrable on a fresh non-Heimdal host:

- `curl -fsSL .../install-tot.sh | sh` succeeds on a clean Linux box with podman
- Edit `.env` to point at any OpenAI-compatible endpoint
- `tot up` brings up the full stack; first-boot bootstrap completes; worldserver listens
- A player downloads `patch-Z-tot-1.0.0.MPQ`, drops into `Data/`, edits realmlist, logs in
- Login screen reads "Threads of Time 1.0.0" with the fan-project disclaimer
- Player joins a party with a living bot; bot greets them by name (memory + brain + harness end-to-end)
- Player loots a Bracket 1 chest: ~10% chance of orange Warforged tag; tooltip shows spec-specific tier-set bonus rows; ~10% independent bonus socket
- Bot remembers the player across a session restart
- Operator runs `tot upgrade 1.0.1` and it works without manual intervention
- Operator runs `tot-backup.sh` and gets a recoverable DB dump

If those all pass on a fresh host with no Heimdal-specific assumptions, `v1.0.0` ships.
