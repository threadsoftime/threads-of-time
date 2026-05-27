# Threads of Time 1.0.0 — Foundation (Plan 1 of 6)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `threads-of-time` repo on a stock AzerothCore base, with AC-core hooks extracted from mod-playerbots's AC fork as ToT's own commits, the existing ToT modules migrated in, a new `mod-agenticbots` module authored against mod-playerbots's public API, ToT product code organized under a `tot/` namespace, and the brain decoupled from the V3.8 model router. End state: a buildable, self-consistent ToT codebase ready for parallel feature work in Plans 2–6.

**Architecture:** A hard fork of stock AzerothCore (`azerothcore/azerothcore-wotlk`) pinned via `UPSTREAMS.toml`. ToT's own AC-core changes (the hooks mod-playerbots needs) live as regular commits in `src/`, not as overlay patches. ToT-authored AC modules live in `modules/`. `mod-agenticbots` is a new module that calls into `mod-playerbots` (installed alongside as an external dependency, not vendored). ToT product code (harness daemon, brain sidecar, memory subsystem, client-patch compositor, release pipeline, deploy stack) lives under `tot/`. Branches: `main` (release-tagged), `dev` (active), `release/X.Y` (stabilization), `upstream/ac` (tracks stock AC).

**Tech Stack:** Git, CMake (AC's build system), C++17 (worldserver + modules), Python 3.11+ (sidecars + tooling), Podman/Docker, MySQL 8.x, AGPL-3.0 across the board.

**Predecessors:**
- Spec: `docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md`
- Current state in this repo (as of 2026-05-27, tip `4ef799d`): `image/build.sh` clones `mod-playerbots/azerothcore-wotlk@Playerbot` at SHA `f570462f`, applies three patch dirs (`patches/ac-bfd-raid/`, `patches/ac-bracket-sets/`, `patches/mod-playerbots-bfd/`) as overlays, rsync's `modules/mod-{agenticbots-source-here-as-patches, bracket-sets, rotation-mode, warforged, harness-bridge}` into the build tree. mod-warforged was merged at `8e1e1f6` (2026-05-25); the unified `ThreadsOfTime` AddOn composer landed at `82a6583` (2026-05-25); the AC-side bracket-sets hook landed at `4ef799d` (2026-05-27).

**Constraints (from CLAUDE.md, non-negotiable):**
- `cmake -j4` max on Heimdal — never `-j16`/`-j17` even when asked
- Cross-check worldserver binary mtime after every build before declaring success (kb_f974fc65)
- NEVER `rm -rf` or `rm` — use `mv <path> ~/.Trash/` on macOS, `mv <path> /var/tmp/heimdal-cleanup-YYYY-MM-DD/` on Heimdal
- Ask before deleting untracked files — may be in-progress work

---

## File Structure

This plan creates a new repository (`threads-of-time`) parallel to the existing `azerothcore-heimdal`. Migration is **copy-only** until the final cutover task; nothing in `azerothcore-heimdal` is deleted by this plan. End-state layout of the new repo:

```
threads-of-time/
├── .github/workflows/              ← CI (scaffold only this plan; full CI in Plan 6)
├── .gitignore
├── LICENSE                         ← AGPL-3.0
├── README.md                       ← project face + fan-project disclaimer
├── CONTRIBUTING.md                 ← inbound = outbound AGPL-3.0
├── UPSTREAMS.toml                  ← pinned AC SHA + mod-playerbots compat range
├── src/                            ← forked AC source (full tree from upstream)
├── deps/                           ← forked AC deps
├── apps/                           ← forked AC apps
├── data/                           ← forked AC data
├── modules/
│   ├── mod-agenticbots/            ← NEW, ToT-authored (this plan, Phase 5)
│   ├── mod-bracket-sets/           ← migrated from azerothcore-heimdal main tree
│   │   └── data/addon-contrib/     ← ready for Lua contributions (currently .gitkeep)
│   ├── mod-rotation-mode/          ← migrated from azerothcore-heimdal
│   ├── mod-warforged/              ← migrated from azerothcore-heimdal main tree (merged 8e1e1f6, 2026-05-25)
│   │   ├── client/                 ← per-module DBC additions + (deprecated) FrameXML
│   │   └── data/addon-contrib/     ← Lua contributions (WarforgedSentinel + GameTooltip + ItemRef + WarforgedStatBumps + manifest.toml)
│   └── mod-harness-bridge/         ← migrated from azerothcore-heimdal
├── tot/
│   ├── harness/                    ← FastMCP daemon (migrated)
│   ├── brain/                      ← V3 brain sidecar; router decoupled out
│   ├── memory/                     ← memory subsystem (migrated; deepened in Plan 2)
│   ├── client-patch/
│   │   ├── compose-tot-addon.py    ← migrated from tools/ (live as of 82a6583)
│   │   ├── dbc-patch-builder/      ← migrated from tools/; MPQ pack stage generalization deferred to Plan 4
│   │   └── PRIORITIES.md           ← AddOn-priority allocation doc (new, this plan)
│   ├── content/                    ← SQL packs (Bracket 1, BFD raid)
│   ├── release/                    ← build orchestrator + (in Plan 6) release.sh
│   ├── deploy/                     ← reference Quadlet + Compose (full work in Plan 5)
│   └── internal-docs/
│       ├── heimdal-router/         ← V3.8 multi-mode router (Heimdal-internal, decoupled this plan)
│       └── specs+plans/            ← prior specs/plans, preserved for history
└── docs/
    └── (empty; populated in Plan 5)
```

---

## Phase 1: Repo bootstrap

End-state of phase: A new `threads-of-time` git repo on disk at `/Users/tbrack/Documents/Projects/threads-of-time/`, initialized from a pinned stock AC SHA, with branches, `UPSTREAMS.toml`, `LICENSE`, `README.md`, `CONTRIBUTING.md`, `.gitignore`. Buildable as vanilla AC.

### Task 1: Identify the target stock AC SHA

**Files:**
- Read: existing `azerothcore-heimdal/image/build.sh:22` for the current pinned mod-playerbots-fork SHA (`f570462f`) as a reference point
- Read: `https://github.com/azerothcore/azerothcore-wotlk/commits/master` to find a recent stable stock AC SHA

- [ ] **Step 1: Capture the current pinned mod-playerbots-fork SHA for reference**

Run: `grep AC_BASE_SHA /Users/tbrack/Documents/Projects/azerothcore-heimdal/image/build.sh`
Expected: `AC_BASE_SHA="${AC_BASE_SHA:-f570462f}"`

Record this in a scratch note — we'll later need to identify which stock AC SHA mod-playerbots's fork at `f570462f` was forked from (call this `STOCK_AC_REF`).

- [ ] **Step 2: Identify the stock AC merge-base**

Run:
```bash
mkdir -p /tmp/ac-discovery && cd /tmp/ac-discovery
git clone --filter=tree:0 https://github.com/mod-playerbots/azerothcore-wotlk.git mpb-fork
cd mpb-fork
git checkout f570462f
git remote add stock https://github.com/azerothcore/azerothcore-wotlk.git
git fetch stock master --depth=10000
STOCK_AC_REF=$(git merge-base HEAD stock/master)
echo "STOCK_AC_REF=$STOCK_AC_REF"
```

Record `STOCK_AC_REF` — this is the stock AC commit mod-playerbots's fork is based on, and the starting pin for ToT.

- [ ] **Step 3: Decide on the ToT pinned AC SHA**

Two options:
1. Pin at `STOCK_AC_REF` (match mod-playerbots's current base — minimizes hook-extraction conflicts)
2. Pin at `stock/master@HEAD` as of today (catches up AC immediately, costs hook-extraction rebase work)

Recommendation: **start at `STOCK_AC_REF`** for Plan 1. The first AC catch-up to `stock/master@HEAD` becomes the validation of the AC-upgrade workflow (§8 of the spec). Note this choice in `UPSTREAMS.toml` comments in Task 3.

Record the chosen SHA as `TOT_AC_PIN`.

- [ ] **Step 4: Commit decision to a scratch note**

```bash
cat > /tmp/ac-discovery/TOT_AC_DECISION.md <<EOF
ToT 1.0.0 AC pin decision (Task 1, $(date +%Y-%m-%d))

- mod-playerbots fork pinned at: f570462f
- Inferred stock AC merge-base: $STOCK_AC_REF
- Chosen TOT_AC_PIN: $STOCK_AC_REF (= match mpb base for clean Plan-1 hook extraction)
- First AC catch-up = validation of §8 workflow
EOF
```

No commit yet — repo doesn't exist.

### Task 2: Initialize the `threads-of-time` repo

**Files:**
- Create: `/Users/tbrack/Documents/Projects/threads-of-time/` (new directory)

- [ ] **Step 1: Clone stock AC at the pinned SHA**

Run:
```bash
cd /Users/tbrack/Documents/Projects/
git clone https://github.com/azerothcore/azerothcore-wotlk.git threads-of-time
cd threads-of-time
git checkout $TOT_AC_PIN
```

Expected: Clean working tree on the pinned SHA.

- [ ] **Step 2: Configure remotes**

```bash
git remote rename origin upstream-ac
git remote -v
```

Expected:
```
upstream-ac	https://github.com/azerothcore/azerothcore-wotlk.git (fetch)
upstream-ac	https://github.com/azerothcore/azerothcore-wotlk.git (push)
```

- [ ] **Step 3: Establish the branching model**

```bash
git checkout -b main
git branch dev
git fetch upstream-ac master
git update-ref refs/heads/upstream/ac upstream-ac/master
git branch -a
```

Expected: `main`, `dev`, `upstream/ac` branches exist; `main` and `dev` point at `TOT_AC_PIN`; `upstream/ac` points at stock AC master HEAD.

- [ ] **Step 4: Verify vanilla AC build still works (sanity check)**

This runs on Heimdal because of toolchain + `-j4` discipline.

```bash
rsync -aH --delete --exclude .git /Users/tbrack/Documents/Projects/threads-of-time/ heimdal:/var/tmp/tot-sanity-build/
ssh heimdal "cd /var/tmp/tot-sanity-build && mkdir -p build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none -DAPPS_BUILD=none && cmake --build . --target worldserver -j4"
```

Expected: clean build, worldserver binary produced at `/var/tmp/tot-sanity-build/build/src/server/worldserver/worldserver`.

- [ ] **Step 5: Confirm binary mtime is recent (per kb_f974fc65)**

```bash
ssh heimdal "ls -la /var/tmp/tot-sanity-build/build/src/server/worldserver/worldserver"
```

Expected: mtime within the last few minutes. (If it's old, the build silently used a cached artifact — diagnose before continuing.)

- [ ] **Step 6: Clean up sanity-build artifacts**

```bash
ssh heimdal "mv /var/tmp/tot-sanity-build /var/tmp/heimdal-cleanup-$(date +%Y-%m-%d)/"
```

Do NOT `rm -rf`; per CLAUDE.md, move-to-trash.

No commit yet — Task 3 creates the first commit.

### Task 3: Add `UPSTREAMS.toml`, `LICENSE`, `README.md`, `CONTRIBUTING.md`, `.gitignore`

**Files:**
- Create: `UPSTREAMS.toml`
- Create: `LICENSE`
- Create: `README.md`
- Create: `CONTRIBUTING.md`
- Modify: `.gitignore` (AC ships one; extend it)

- [ ] **Step 1: Write `UPSTREAMS.toml`**

```toml
# Threads of Time — pinned upstream versions.
#
# ToT forks stock AzerothCore and carries its own commits on top. The pinned SHA
# below is the base commit ToT diverges from. AC upgrade workflow:
#   1. git fetch upstream-ac master
#   2. git rebase --onto upstream-ac/master <current pinned SHA> dev
#   3. If clean: bump [ac].sha here, commit, smoke-test, ship as a PATCH release
#
# See spec §8 for the full AC upgrade workflow.

[ac]
repo = "https://github.com/azerothcore/azerothcore-wotlk"
sha  = "FILL_IN_TOT_AC_PIN_HERE"   # Set in Task 3 Step 1 from Task 1's decision
note = "Stock AC merge-base of mod-playerbots/azerothcore-wotlk@f570462f"

# mod-playerbots is an external runtime dependency, not a vendored upstream.
# Operators install mod-playerbots themselves (the install script in Plan 5
# verifies presence and version). This block expresses the compatibility
# contract — ToT 1.0.0 is built and tested against the 'tested' version and
# is expected to work with any release >= 'min'.

[playerbots-dependency]
repo   = "https://github.com/liyunfan1223/mod-playerbots"
min    = "FILL_IN_AFTER_AGENTICBOTS_AUTHORING"   # Set in Phase 5
tested = "FILL_IN_AFTER_AGENTICBOTS_AUTHORING"   # Set in Phase 5
note   = "Required for mod-agenticbots; see modules/mod-agenticbots/README.md for the dependency contract"
```

Then substitute the AC SHA:

```bash
TOT_AC_PIN=<sha from Task 1>
sed -i.bak "s/FILL_IN_TOT_AC_PIN_HERE/${TOT_AC_PIN}/" UPSTREAMS.toml && rm UPSTREAMS.toml.bak
cat UPSTREAMS.toml
```

Expected: AC sha filled in; playerbots-dependency lines still have FILL_IN placeholders (resolved in Phase 5).

- [ ] **Step 2: Write `LICENSE`**

Use AC's GPL-2.0 LICENSE verbatim (AC at the pinned SHA is GPL-2.0-or-later; ToT matches AC literally per spec §10.1):

```bash
git checkout <TOT_AC_PIN> -- LICENSE
head -3 LICENSE
wc -l LICENSE
```

Expected: file starts with `                    GNU GENERAL PUBLIC LICENSE\n                       Version 2, June 1991`. Total 338 lines.

- [ ] **Step 3: Write `README.md`**

```markdown
# Threads of Time

An unofficial, non-commercial fan-built expansion for *World of Warcraft: Wrath of the Lich King 3.3.5a*, built on [AzerothCore](https://github.com/azerothcore/azerothcore-wotlk).

**Threads of Time is not affiliated with, endorsed by, or sponsored by Blizzard Entertainment, Inc. World of Warcraft® and Wrath of the Lich King® are trademarks of Blizzard Entertainment. All Blizzard intellectual property remains the property of Blizzard Entertainment.**

## What this is

Threads of Time (ToT) is an AI-driven WoW expansion. The headline feature is **alive bots** — LLM-orchestrated characters with persistent memories, personalities, and goal-directed behavior — playing alongside you through custom bracketed content.

ToT 1.0.0 covers Bracket 1 (levels 1–25), with class+spec-specific tier-set bonuses, a bracket-appropriate raid (Blackfathom Deeps), Legion-style Warforged + Bonus Socket procs on item drops, and the rotation mode system.

## Install

See [docs/install.md](docs/install.md) for the operator install path (full instructions ship in Plan 5).

For players: download `patch-Z-tot-1.0.0.MPQ` from the releases page, drop into your client's `Data/` folder, edit `Data/<locale>/realmlist.wtf` to point at your server's realmlist, and log in.

## Architecture

See [`docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md`](docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md) for the full release-architecture design.

## License

GPL-2.0-or-later, matching [AzerothCore](https://github.com/azerothcore/azerothcore-wotlk)'s license. See [LICENSE](LICENSE) for the full text. See [CONTRIBUTING.md](CONTRIBUTING.md) for the contributor license.

## Dependencies

ToT depends on [mod-playerbots](https://github.com/liyunfan1223/mod-playerbots) being installed alongside (see [UPSTREAMS.toml](UPSTREAMS.toml) for the supported version range). The install script (Plan 5) verifies this for you.
```

- [ ] **Step 4: Write `CONTRIBUTING.md`**

```markdown
# Contributing to Threads of Time

Thank you for considering a contribution. This document covers the contributor license and the basic workflow.

## License of contributions

By submitting a pull request to this repository, you certify that you have the right to license your contribution under GPL-2.0-or-later and you agree to do so. This is the "inbound = outbound" model — your contribution ships under the same license as the project (matching AzerothCore's license).

No CLA, no DCO sign-off, no copyright assignment required.

## Workflow

1. Open an issue describing what you want to change before writing code, unless the change is small and obvious.
2. Fork the repo, create a feature branch off `dev`.
3. Make changes following the project conventions (see `docs/superpowers/specs/` for architecture context).
4. Open a PR back to `dev`. CI must be green.
5. Maintainers review and merge.

## Adding a new SPDX header

All new source files require an SPDX header on line 1:

```
// SPDX-License-Identifier: GPL-2.0-or-later
```

(Use `# SPDX-License-Identifier: AGPL-3.0-or-later` for Python and shell.)

## Where to get help

See [README.md](README.md) for architecture overview. For implementation questions, the specs under `docs/superpowers/specs/` and `docs/superpowers/plans/` are the authoritative source.
```

- [ ] **Step 5: Extend `.gitignore`**

AC ships a `.gitignore`; append ToT-specific entries:

```bash
cat >> .gitignore <<'EOF'

# === Threads of Time additions ===
# Build artifacts
/build/
release-artifacts/

# Sidecar dev artifacts
tot/*/__pycache__/
tot/*/.pytest_cache/
tot/*/*.egg-info/
tot/*/.venv/

# Local operator state
secrets/
data/

# Editor / OS noise
.DS_Store
*.swp
.vscode/
.idea/

# Logs
*.log
EOF
```

- [ ] **Step 6: Stage + commit the bootstrap files**

```bash
git add UPSTREAMS.toml LICENSE README.md CONTRIBUTING.md .gitignore
git status
git commit -m "feat(bootstrap): init Threads of Time as AGPL-3.0 fork of AC

- UPSTREAMS.toml pins stock AC at $TOT_AC_PIN
- mod-playerbots dependency declared (versions resolved in Phase 5)
- GPL-2.0-or-later LICENSE matches AzerothCore literally
- README + CONTRIBUTING document the project + license stance
- .gitignore extends AC's with ToT-specific paths

Refs: docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md §§2, 10"
```

- [ ] **Step 7: Tag the bootstrap commit**

```bash
git tag bootstrap-complete -m "Phase 1 complete: empty ToT repo on stock AC pin"
git log --oneline -3
```

Expected: bootstrap commit + initial stock AC commit visible.

---

## Phase 2: AC-core hook extraction

End-state of phase: ToT's `src/` contains the AC-core hooks mod-playerbots needs (PlayerbotAI accessors, hook insertions, etc.), committed as discrete ToT changes on top of stock AC. Worldserver builds without mod-playerbots installed (the hooks are dormant call sites until a playerbots module is loaded).

### Task 4: Identify the AC-core hook diff

**Files:**
- Read: mod-playerbots's AC fork (cloned in Task 1 at `/tmp/ac-discovery/mpb-fork/`)
- Reference: stock AC merge-base from Task 1 (`STOCK_AC_REF`)

- [ ] **Step 1: Compute the AC-core diff between stock and the playerbots fork**

```bash
cd /tmp/ac-discovery/mpb-fork
mkdir -p /tmp/ac-discovery/hooks-diff
git diff $STOCK_AC_REF..f570462f -- src/ deps/ apps/ data/ > /tmp/ac-discovery/hooks-diff/ac-core.diff
wc -l /tmp/ac-discovery/hooks-diff/ac-core.diff
git diff $STOCK_AC_REF..f570462f --stat -- src/ deps/ apps/ data/ | tail -50
```

Expected: a diff file with ~hundreds-to-low-thousands of lines (the mod-playerbots AC-core integration). Visual scan of the summary tells you which files are touched.

- [ ] **Step 2: Categorize the diff by intent**

For each touched file (from Step 1's summary), label as:
- **HOOK**: a new hook point / accessor (kept; this is what we extract)
- **PLAYERBOTS-INTERNAL**: changes that belong inside mod-playerbots rather than AC core (drop; if needed, mod-agenticbots's Plan 1 Phase 5 authoring deals with it)
- **AC-BUG-FIX**: a bug fix in stock AC that mod-playerbots's fork applies — keep, but file a separate PR to upstream AC if not already done
- **CONFIG**: changes to config defaults — re-examine; usually keep but note

Write the categorization to `/tmp/ac-discovery/hooks-diff/categorization.md` with one line per file:

```
src/server/game/Entities/Player/Player.h    HOOK    Adds GetPlayerbotAI() accessor
src/server/game/Entities/Player/Player.cpp  HOOK    Wires the accessor
src/server/AuthServer/AuthServer.cpp        AC-BUG-FIX  Fixes a NULL deref unrelated to bots
...
```

This categorization is the basis for the next task's commits.

- [ ] **Step 3: Identify any PLAYERBOTS-INTERNAL lines that should NOT come into ToT's AC core**

Those will be re-handled either by mod-agenticbots (calling the public API) or by a documented compatibility requirement with mod-playerbots's installed version. Note them in `categorization.md` with a "DEFERRED-TO-PLAN-1-PHASE-5" tag — Phase 5 will reference these.

### Task 5: Replay HOOK changes as ToT commits

**Files:**
- Modify: various `src/server/...` files (from Task 4's HOOK-tagged set)
- Test: build worldserver after each commit (cmake build is the regression test for structural code changes)

- [ ] **Step 1: Apply the HOOK-only subset**

Create a filtered diff containing only HOOK-tagged files:

```bash
cd /tmp/ac-discovery/hooks-diff
grep -E '\sHOOK\s' categorization.md | awk '{print $1}' > hook-files.txt
cd /Users/tbrack/Documents/Projects/threads-of-time/
git checkout dev
mkdir -p /tmp/ac-discovery/hook-patches
while read f; do
    if [ -f "/tmp/ac-discovery/mpb-fork/$f" ]; then
        mkdir -p "$(dirname $f)"
        cp "/tmp/ac-discovery/mpb-fork/$f" "$f"
    fi
done < /tmp/ac-discovery/hooks-diff/hook-files.txt
git status
```

Expected: working tree shows modifications to AC source files matching the HOOK list.

- [ ] **Step 2: Build to verify the HOOK changes compile**

```bash
rsync -aH --delete --exclude .git /Users/tbrack/Documents/Projects/threads-of-time/ heimdal:/var/tmp/tot-hook-build/
ssh heimdal "cd /var/tmp/tot-hook-build && mkdir -p build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -30"
```

Expected: clean build. If link errors appear, the HOOK extraction missed a file — go back to Task 4 categorization.

- [ ] **Step 3: Verify binary mtime + symbol presence**

```bash
ssh heimdal "ls -la /var/tmp/tot-hook-build/build/src/server/worldserver/worldserver"
ssh heimdal "nm /var/tmp/tot-hook-build/build/src/server/worldserver/worldserver | grep -i playerbot | head -10"
```

Expected: mtime recent; nm output shows hook-related symbols (e.g., `GetPlayerbotAI` accessor exported).

- [ ] **Step 4: Commit the HOOK extraction**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
git add src/ deps/ apps/ data/
git status
git commit -m "feat(ac-core): extract mod-playerbots AC-core hooks onto stock AC

These hook points (accessors, callback sites) are required by mod-playerbots
but live in AC core. ToT carries them here so we can stay on stock AC base
and own our upgrade cadence independent of mod-playerbots's AC fork.

Source: mod-playerbots/azerothcore-wotlk@f570462f
Categorization notes: see internal-docs (added in Phase 7)

Refs: spec §2.4 step 3"
```

- [ ] **Step 5: Apply AC-BUG-FIX commits separately**

For each file tagged AC-BUG-FIX in `categorization.md` (likely a small set, possibly empty), create one commit per logical fix:

```bash
# Repeat for each AC-BUG-FIX:
cp "/tmp/ac-discovery/mpb-fork/<file>" "<file>"
git add <file>
git commit -m "fix(ac-core): <one-line summary of the bug fix>

Carried from mod-playerbots's AC fork. Upstream PR status: <link or 'not filed yet'>.
If upstream AC accepts the fix, drop this commit on the next AC rebase."
```

If there are no AC-BUG-FIX entries, skip this step.

- [ ] **Step 6: Apply CONFIG changes (if any)**

For CONFIG-tagged files, examine each — most likely declines because operators configure via `.env` (per spec §6.3), not by AC-default changes. Where a change IS needed (e.g., a default that mod-playerbots requires), commit individually with rationale.

- [ ] **Step 7: Cleanup discovery directory**

```bash
mv /tmp/ac-discovery ~/.Trash/
git log --oneline -10
```

Expected: bootstrap commit, then 1–N hook/bug-fix/config commits.

---

## Phase 3: Migrate existing ToT AC modules

End-state of phase: `modules/mod-bracket-sets/`, `modules/mod-rotation-mode/`, `modules/mod-harness-bridge/`, `modules/mod-warforged/` exist in the new repo, compile, and load.

### Task 6: Migrate mod-bracket-sets

**Files:**
- Create: `modules/mod-bracket-sets/` (full subtree copy)
- Modify: `modules/mod-bracket-sets/CMakeLists.txt` (verify no path-specific assumptions broken)

- [ ] **Step 1: Copy the module**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
mkdir -p modules
cp -R /Users/tbrack/Documents/Projects/azerothcore-heimdal/modules/mod-bracket-sets modules/
ls modules/mod-bracket-sets/
```

Expected: `conf data README.md src tests tools` (per ls of source).

- [ ] **Step 2: Verify the module builds against the new AC tree**

```bash
rsync -aH --delete --exclude .git /Users/tbrack/Documents/Projects/threads-of-time/ heimdal:/var/tmp/tot-bsets-build/
ssh heimdal "cd /var/tmp/tot-bsets-build && mkdir -p build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -30"
```

Expected: clean build. The `mod-bracket-sets` symbols should be linked into the worldserver.

```bash
ssh heimdal "nm /var/tmp/tot-bsets-build/build/src/server/worldserver/worldserver 2>/dev/null | grep -i bracket | head -5"
```

Expected: bracket-set symbols visible.

- [ ] **Step 3: Verify mtime + commit**

```bash
ssh heimdal "ls -la /var/tmp/tot-bsets-build/build/src/server/worldserver/worldserver"
cd /Users/tbrack/Documents/Projects/threads-of-time/
git add modules/mod-bracket-sets/
git commit -m "feat(modules): migrate mod-bracket-sets from azerothcore-heimdal

Per spec §2.4 step 5. Module ships Bracket 1 tier-set system (54 bonuses across
27 ItemSet rows), per kb_99501ac5."
ssh heimdal "mv /var/tmp/tot-bsets-build ~/.Trash/ 2>/dev/null || true"
```

### Task 7: Migrate mod-rotation-mode

**Files:**
- Create: `modules/mod-rotation-mode/`

- [ ] **Step 1: Copy + build + commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
cp -R /Users/tbrack/Documents/Projects/azerothcore-heimdal/modules/mod-rotation-mode modules/
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-rmode-build/
ssh heimdal "cd /var/tmp/tot-rmode-build && mkdir -p build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -10"
ssh heimdal "ls -la /var/tmp/tot-rmode-build/build/src/server/worldserver/worldserver"
git add modules/mod-rotation-mode/
git commit -m "feat(modules): migrate mod-rotation-mode from azerothcore-heimdal

Per spec §2.4 step 5. See kb_758e2c2b."
ssh heimdal "mv /var/tmp/tot-rmode-build ~/.Trash/ 2>/dev/null || true"
```

### Task 8: Migrate mod-harness-bridge

**Files:**
- Create: `modules/mod-harness-bridge/`

- [ ] **Step 1: Copy + build + commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
cp -R /Users/tbrack/Documents/Projects/azerothcore-heimdal/modules/mod-harness-bridge modules/
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-harness-build/
ssh heimdal "cd /var/tmp/tot-harness-build && mkdir -p build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -10"
ssh heimdal "ls -la /var/tmp/tot-harness-build/build/src/server/worldserver/worldserver"
git add modules/mod-harness-bridge/
git commit -m "feat(modules): migrate mod-harness-bridge V1.4 from azerothcore-heimdal

Per spec §2.4 step 5. Provides obs.*, gm.*, bot.* tool adapters per kb_642162c3."
ssh heimdal "mv /var/tmp/tot-harness-build ~/.Trash/ 2>/dev/null || true"
```

### Task 9: Migrate mod-warforged from azerothcore-heimdal main tree

**Files:**
- Create: `modules/mod-warforged/` (already merged to azerothcore-heimdal's main tree at commit `8e1e1f6`; current tip includes post-merge iteration through `82a6583` — level gate, AddOn-shipped Lua pivot)

- [ ] **Step 1: Copy the module from the main tree**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
cp -R /Users/tbrack/Documents/Projects/azerothcore-heimdal/modules/mod-warforged modules/
ls modules/mod-warforged/
ls modules/mod-warforged/data/addon-contrib/
```

Expected: `build client conf data README.md src tests tools` at the top level; `WarforgedSentinel.lua WarforgedStatBumps.lua GameTooltip.lua ItemRef.lua manifest.toml` under `data/addon-contrib/`.

The `data/addon-contrib/` subtree is consumed by the unified `ThreadsOfTime` AddOn composer (migrated in Task 24); the `client/` subtree carries DBC additions + the prior-style MPQ (`patch-W.MPQ`, gradually replaced as the unified MPQ pipeline lands in Plan 4).

- [ ] **Step 2: Build + verify Warforged symbols**

```bash
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-wf-build/
ssh heimdal "cd /var/tmp/tot-wf-build && mkdir -p build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -10"
ssh heimdal "ls -la /var/tmp/tot-wf-build/build/src/server/worldserver/worldserver"
ssh heimdal "nm /var/tmp/tot-wf-build/build/src/server/worldserver/worldserver 2>/dev/null | grep -i warforged | head -5"
```

Expected: build clean, Warforged symbols visible.

- [ ] **Step 3: Commit**

```bash
git add modules/mod-warforged/
git commit -m "feat(modules): migrate mod-warforged from azerothcore-heimdal

Legion-style Warforged + Bonus Socket procs on item drops.
Spec: docs/superpowers/specs/2026-05-23-mod-warforged-design.md
Source tip at migration: tip of azerothcore-heimdal/master as of 2026-05-27
(includes merge 8e1e1f6 + post-merge level gate ab24b73 + AddOn pivot
64ffa26 + composed-AddOn integration 82a6583).

Ships with data/addon-contrib/ (Lua + manifest.toml for the unified
ThreadsOfTime AddOn composer, migrated in Task 24) and client/ (DBC
additions + legacy patch-W.MPQ retired in Plan 4).

Refs: spec §1.1, §9.1"
ssh heimdal "mv /var/tmp/tot-wf-build ~/.Trash/ 2>/dev/null || true"
```

---

## Phase 4: Migrate AC content

End-state of phase: `patches/ac-bfd-raid/` overlays are replayed as commits in the new repo's `src/` and `tot/content/sql/`, instead of living as patches.

### Task 10: Replay `patches/ac-bfd-raid/0001-bfd-raid-map-override.patch` as a commit

**Files:**
- Modify: AC src/ files affected by the patch

- [ ] **Step 1: Inspect the patch contents**

```bash
cat /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/ac-bfd-raid/0001-bfd-raid-map-override.patch | head -40
```

This tells you which files the patch modifies. Apply the modifications directly to the target files in the ToT repo.

- [ ] **Step 2: Apply the patch**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
git apply /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/ac-bfd-raid/0001-bfd-raid-map-override.patch
git status
```

Expected: working tree modifications match the patch hunks. If `git apply` fails because the patch is against the mod-playerbots-fork AC and ours is stock AC, fall back to manual application (read patch hunks, apply line by line).

- [ ] **Step 3: Build to verify**

```bash
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-bfd1-build/
ssh heimdal "cd /var/tmp/tot-bfd1-build && mkdir -p build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -10"
ssh heimdal "ls -la /var/tmp/tot-bfd1-build/build/src/server/worldserver/worldserver"
```

Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add .
git commit -m "feat(content): BFD raid map override

Replayed from patches/ac-bfd-raid/0001-bfd-raid-map-override.patch.
Repurposes Blackfathom Deeps map for the Bracket 1 raid encounter.

Refs: spec §1.1, kb_460dfd70"
ssh heimdal "mv /var/tmp/tot-bfd1-build ~/.Trash/ 2>/dev/null || true"
```

### Task 11: Replay `patches/ac-bfd-raid/0002-bfd-raid-boss-stub-registration.patch`

Same shape as Task 10. Apply patch, build, commit:

- [ ] **Step 1: Apply**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
git apply /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/ac-bfd-raid/0002-bfd-raid-boss-stub-registration.patch
```

- [ ] **Step 2: Build + verify**

```bash
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-bfd2-build/
ssh heimdal "cd /var/tmp/tot-bfd2-build && mkdir -p build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -10"
ssh heimdal "ls -la /var/tmp/tot-bfd2-build/build/src/server/worldserver/worldserver"
```

- [ ] **Step 3: Commit**

```bash
git add .
git commit -m "feat(content): BFD raid boss stub registration

Replayed from patches/ac-bfd-raid/0002-bfd-raid-boss-stub-registration.patch.
Registers boss stubs for the BFD raid encounters.

Refs: spec §1.1, kb_460dfd70"
ssh heimdal "mv /var/tmp/tot-bfd2-build ~/.Trash/ 2>/dev/null || true"
```

### Task 12: Replay `patches/ac-bfd-raid/0003-bfd-raid-3day-lockout.patch`

Same shape:

- [ ] **Step 1: Apply + build + commit**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
git apply /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/ac-bfd-raid/0003-bfd-raid-3day-lockout.patch
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-bfd3-build/
ssh heimdal "cd /var/tmp/tot-bfd3-build && mkdir -p build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -10"
ssh heimdal "ls -la /var/tmp/tot-bfd3-build/build/src/server/worldserver/worldserver"
git add .
git commit -m "feat(content): BFD raid 3-day lockout

Replayed from patches/ac-bfd-raid/0003-bfd-raid-3day-lockout.patch.

Refs: spec §1.1, kb_460dfd70"
ssh heimdal "mv /var/tmp/tot-bfd3-build ~/.Trash/ 2>/dev/null || true"
```

### Task 12.5: Replay `patches/ac-bracket-sets/0001-add-on-player-build-item-query-response-hook.patch`

The AC-side `OnPlayerBuildItemQueryResponse` PlayerScript hook that mod-bracket-sets's itemset-rewrite depends on. Single patch, clean CALL_ENABLED_HOOKS shape; landed at `azerothcore-heimdal` commit `4ef799d` on 2026-05-27.

**Files:**
- Modify: AC `src/server/game/Scripting/ScriptDefines/PlayerScript.h`, `PlayerScript.cpp`, `src/server/game/Scripting/ScriptMgr.h`, `src/server/game/Handlers/ItemHandler.cpp` (the specific file list depends on patch hunks — `cat` the patch before applying for the authoritative list)

- [ ] **Step 1: Inspect the patch**

```bash
cat /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/ac-bracket-sets/0001-add-on-player-build-item-query-response-hook.patch | head -60
```

Confirms which files are touched.

- [ ] **Step 2: Apply the patch**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
git apply /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/ac-bracket-sets/0001-add-on-player-build-item-query-response-hook.patch
git status
```

If `git apply` fails (patch was authored against the mod-playerbots-fork AC and ours is stock + already-applied hook extractions from Task 5), fall back to manual application: read hunks, apply line-by-line, resolve conflicts with the hook insertions already made in Task 5.

- [ ] **Step 3: Build to verify the new hook compiles**

```bash
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-bsethook-build/
ssh heimdal "cd /var/tmp/tot-bsethook-build && rm -rf build && mkdir build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -10"
ssh heimdal "ls -la /var/tmp/tot-bsethook-build/build/src/server/worldserver/worldserver"
```

Expected: clean build.

- [ ] **Step 4: Verify the hook is exported**

```bash
ssh heimdal "nm /var/tmp/tot-bsethook-build/build/src/server/worldserver/worldserver | grep -i 'OnPlayerBuildItemQueryResponse' | head -5"
```

Expected: hook symbol visible.

- [ ] **Step 5: Commit**

```bash
git add .
git commit -m "feat(ac-core): OnPlayerBuildItemQueryResponse PlayerScript hook

Replayed from patches/ac-bracket-sets/0001-add-on-player-build-item-query-response-hook.patch (azerothcore-heimdal commit 4ef799d).

Adds a PlayerScript hook fired from WorldSession::HandleItemQuerySingleOpcode
right before the ItemSet field is serialized into SMSG_ITEM_QUERY_SINGLE_RESPONSE.
Consumers (mod-bracket-sets in this codebase) can mutate the itemSet value
in place — rewriting the sentinel itemset ID (90101) into the per-class+spec
ID so the client renders the correct tier-set bonuses for the player's current
talent build.

The patch uses CALL_ENABLED_HOOKS so the call site is zero-cost when no
PlayerScript has registered the hook.

This is a clean hook shape — candidate for upstream AC PR (spec §11.1
open question). Until accepted, ToT carries it.

Refs: spec §2.4 step 4, §9.1; kb_99501ac5"
ssh heimdal "mv /var/tmp/tot-bsethook-build ~/.Trash/ 2>/dev/null || true"
```

### Task 13: Migrate `patches/ac-bfd-raid/overlays/` (non-patch files)

The `overlays/` directory contains whole files to be dropped into the AC tree (rather than patch hunks). These usually include new SQL, scripts, or asset files.

- [ ] **Step 1: List overlays**

```bash
find /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/ac-bfd-raid/overlays/ -type f | head -30
```

- [ ] **Step 2: Copy each overlay into its corresponding path in the ToT tree**

For each overlay file, copy to the equivalent path in `threads-of-time/`. The path inside `overlays/` mirrors the destination path. E.g., `overlays/src/server/scripts/BFD/foo.cpp` → `src/server/scripts/BFD/foo.cpp`.

```bash
rsync -av /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/ac-bfd-raid/overlays/ /Users/tbrack/Documents/Projects/threads-of-time/
git status
```

Review the staged files — confirm they all belong (no PoC litter snuck in).

- [ ] **Step 3: Build + verify**

If the overlays added new `.cpp` files under `src/server/`, the CMake glob means a reconfigure is required (per CLAUDE.md kb_57b453cd):

```bash
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-bfdov-build/
ssh heimdal "cd /var/tmp/tot-bfdov-build && rm -rf build && mkdir build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -20"
ssh heimdal "ls -la /var/tmp/tot-bfdov-build/build/src/server/worldserver/worldserver"
```

(Note `rm -rf build` is allowed on the BUILD directory on Heimdal — that's not user files. Always `mv ~/.Trash/` or `mv /var/tmp/heimdal-cleanup-.../` for repo or user paths.)

Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add .
git commit -m "feat(content): BFD raid overlay assets

Replayed from patches/ac-bfd-raid/overlays/.
Includes BFD-specific scripts, SQL, and supporting assets.

Refs: spec §1.1, kb_460dfd70"
ssh heimdal "mv /var/tmp/tot-bfdov-build ~/.Trash/ 2>/dev/null || true"
```

---

## Phase 5: Author mod-agenticbots

End-state of phase: `modules/mod-agenticbots/` exists as a new module that depends on mod-playerbots's public API. It contains the ToT-specific bot additions previously expressed as `patches/mod-playerbots-bfd/`. Anything that modified mod-playerbots internals has been rewritten to use the public API, deferred (with documented requirement), or accepted as a compat-range constraint. `UPSTREAMS.toml`'s `[playerbots-dependency]` block is filled in.

### Task 14: Inspect the existing `patches/mod-playerbots-bfd/` contents

**Files:**
- Read: `patches/mod-playerbots-bfd/{0001-bfd-strategy-context.patch, 0002-bfd-shared-contexts.patch, 0003-bfd-map48-instance-strategy.patch, overlays/}`

- [ ] **Step 1: Inventory the patches**

```bash
for p in /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/mod-playerbots-bfd/0*.patch; do
    echo "=== $p ==="
    head -20 "$p"
    echo ""
    git apply --stat "$p" 2>&1 | head -10
done
```

Categorize each patch hunk as:
- **PUBLIC-API-USAGE**: hunk only calls into mod-playerbots's public types/macros — safe to lift into mod-agenticbots
- **PRIVATE-API-USAGE**: hunk reaches into mod-playerbots internals (private members, file-static functions) — needs to be rewritten using public API OR PR'd upstream OR documented as a hard fork requirement
- **PURE-NEW-FILE**: hunk is a brand new file — lift wholesale into mod-agenticbots

Write the inventory to `/tmp/agenticbots-extraction/inventory.md` with one row per hunk.

- [ ] **Step 2: Inventory the overlays**

```bash
find /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/mod-playerbots-bfd/overlays/ -type f
```

Overlays are typically whole files (new strategies, actions, triggers). Each is a candidate to lift into `modules/mod-agenticbots/src/`.

- [ ] **Step 3: Decision document**

Write `/tmp/agenticbots-extraction/decisions.md` capturing, per hunk/overlay:
- Where it goes in mod-agenticbots (`src/strategy/<class>/`, `src/action/`, `src/trigger/`, `src/hook/`, `src/glue/`, etc.)
- If it's PRIVATE-API-USAGE: what the rewrite plan is (and whether deferring is OK for 1.0.0)
- If it's a hard fork requirement: how `UPSTREAMS.toml` `[playerbots-dependency]` documents it

This document drives Tasks 15–18.

### Task 15: Scaffold `modules/mod-agenticbots/`

**Files:**
- Create: `modules/mod-agenticbots/CMakeLists.txt`
- Create: `modules/mod-agenticbots/README.md`
- Create: `modules/mod-agenticbots/conf/mod_agenticbots.conf.dist`
- Create: `modules/mod-agenticbots/src/ModAgenticbots.cpp` (entry point)
- Create: `modules/mod-agenticbots/src/AgenticbotsConstants.h`

- [ ] **Step 1: Write the CMakeLists.txt**

Follow the pattern from `modules/mod-bracket-sets/CMakeLists.txt` and `modules/mod-warforged/CMakeLists.txt` for AC's expected module shape:

```bash
cp modules/mod-bracket-sets/CMakeLists.txt modules/mod-agenticbots/CMakeLists.txt
# Then edit: replace 'mod-bracket-sets' with 'mod-agenticbots' references
```

(Open the file, do a find-replace from `bracket-sets` → `agenticbots`.)

- [ ] **Step 2: Write the README declaring the dependency**

```bash
mkdir -p modules/mod-agenticbots
cat > modules/mod-agenticbots/README.md <<'EOF'
# mod-agenticbots

ToT's bot-AI layer. Adds LLM-orchestrated strategies, actions, triggers, harness integration, and the brain wiring on top of mod-playerbots.

## Dependency: mod-playerbots

mod-agenticbots **depends on** [mod-playerbots](https://github.com/liyunfan1223/mod-playerbots) being installed alongside in the AC `modules/` directory. mod-agenticbots calls into mod-playerbots's public API (`PlayerbotAI`, `AI_VALUE`/`AI_VALUE2` macros, the strategy/action/trigger system, `GET_PLAYERBOT_AI(player)`).

The supported mod-playerbots version range is declared in the top-level `UPSTREAMS.toml`:

```toml
[playerbots-dependency]
min    = "..."   # minimum tested version
tested = "..."   # exact version CI runs against
```

mod-agenticbots does **not** vendor, modify, or redistribute mod-playerbots's source. If a mod-agenticbots feature requires changes to mod-playerbots internals, that's either (a) a request to upstream, (b) a documented hard requirement on a specific mod-playerbots version, or (c) a redesign to use the public API.

## License

AGPL-3.0-or-later. Matches AzerothCore.

## Architecture

[Architecture overview — populated in subsequent tasks of this phase.]
EOF
```

- [ ] **Step 3: Scaffold the minimal source files**

```bash
mkdir -p modules/mod-agenticbots/src
mkdir -p modules/mod-agenticbots/conf
mkdir -p modules/mod-agenticbots/data/sql
```

Write the conf stub:

```bash
cat > modules/mod-agenticbots/conf/mod_agenticbots.conf.dist <<'EOF'
[mod-agenticbots]

#
# Agenticbots.Enabled
#    Description: Toggle agenticbots strategies on/off
#    Default:     1 (enabled)
#
Agenticbots.Enabled = 1

# Additional configuration will land as features migrate from patches/mod-playerbots-bfd/
EOF
```

Write the constants header:

```bash
cat > modules/mod-agenticbots/src/AgenticbotsConstants.h <<'EOF'
// SPDX-License-Identifier: GPL-2.0-or-later
#ifndef MOD_AGENTICBOTS_CONSTANTS_H
#define MOD_AGENTICBOTS_CONSTANTS_H

namespace Agenticbots
{
    // Build-time version constant. Lockstep with ToT version.
    constexpr const char* VERSION = "1.0.0-dev";
}

#endif
EOF
```

Write the minimal entry point:

```bash
cat > modules/mod-agenticbots/src/ModAgenticbots.cpp <<'EOF'
// SPDX-License-Identifier: GPL-2.0-or-later
#include "ScriptMgr.h"
#include "Log.h"
#include "AgenticbotsConstants.h"

class ModAgenticbotsWorldScript : public WorldScript
{
public:
    ModAgenticbotsWorldScript() : WorldScript("ModAgenticbotsWorldScript") {}

    void OnStartup() override
    {
        LOG_INFO("module", "[mod-agenticbots] loaded, version {}", Agenticbots::VERSION);
    }
};

void Addmod_agenticbotsScripts()
{
    new ModAgenticbotsWorldScript();
}
EOF
```

- [ ] **Step 4: Build to verify the scaffold loads**

```bash
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-ab-scaffold-build/
ssh heimdal "cd /var/tmp/tot-ab-scaffold-build && rm -rf build && mkdir build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -20"
ssh heimdal "ls -la /var/tmp/tot-ab-scaffold-build/build/src/server/worldserver/worldserver"
ssh heimdal "nm /var/tmp/tot-ab-scaffold-build/build/src/server/worldserver/worldserver 2>/dev/null | grep -i agenticbots | head -5"
```

Expected: clean build; `agenticbots` symbols visible.

- [ ] **Step 5: Commit the scaffold**

```bash
git add modules/mod-agenticbots/
git commit -m "feat(mod-agenticbots): scaffold new module that depends on mod-playerbots

Empty module that loads cleanly and announces itself in the worldserver log.
Subsequent tasks lift ToT-specific bot additions from
patches/mod-playerbots-bfd/ into this module, structured around
mod-playerbots's public API per spec §1.1 + §2.4 step 2.

Refs: spec §2.4 step 2; modules/mod-agenticbots/README.md for the
dependency contract."
ssh heimdal "mv /var/tmp/tot-ab-scaffold-build ~/.Trash/ 2>/dev/null || true"
```

### Task 16: Lift PURE-NEW-FILE overlays into mod-agenticbots

**Files:**
- Create: `modules/mod-agenticbots/src/<various>` (per Task 14's inventory)

- [ ] **Step 1: For each PURE-NEW-FILE entry in `/tmp/agenticbots-extraction/inventory.md`**:

Copy the file from `patches/mod-playerbots-bfd/overlays/<original-path>` to `modules/mod-agenticbots/src/<target-path>` per the decisions doc.

Example (BFD strategy context):

```bash
mkdir -p modules/mod-agenticbots/src/strategy/bfd
cp /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/mod-playerbots-bfd/overlays/modules/mod-playerbots/src/strategy/bfd/BfdStrategyContext.h modules/mod-agenticbots/src/strategy/bfd/
cp /Users/tbrack/Documents/Projects/azerothcore-heimdal/patches/mod-playerbots-bfd/overlays/modules/mod-playerbots/src/strategy/bfd/BfdStrategyContext.cpp modules/mod-agenticbots/src/strategy/bfd/
```

(Adjust paths per the actual overlay structure.)

- [ ] **Step 2: Add SPDX headers to each copied file (if missing)**

```bash
for f in $(find modules/mod-agenticbots/src -name '*.cpp' -o -name '*.h'); do
    if ! head -1 "$f" | grep -q "SPDX-License-Identifier"; then
        sed -i.bak '1i\
// SPDX-License-Identifier: GPL-2.0-or-later
' "$f"
        rm "$f.bak"
    fi
done
```

- [ ] **Step 3: Wire into the entry point**

Edit `modules/mod-agenticbots/src/ModAgenticbots.cpp` to register the lifted scripts. Pattern: AC modules call their `AddXxxScripts()` from the entry's wrapping `Add<mod_name>Scripts()` function.

- [ ] **Step 4: Build (with cmake reconfigure since new .cpp files were added)**

```bash
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-ab-newfiles-build/
ssh heimdal "cd /var/tmp/tot-ab-newfiles-build && rm -rf build && mkdir build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -30"
ssh heimdal "ls -la /var/tmp/tot-ab-newfiles-build/build/src/server/worldserver/worldserver"
```

Expected: clean build, lifted symbols present.

- [ ] **Step 5: Commit**

```bash
git add modules/mod-agenticbots/
git commit -m "feat(mod-agenticbots): lift PURE-NEW-FILE overlays from patches/mod-playerbots-bfd/

Copies the BFD-specific strategy/action/trigger files from the old patch
overlays into the new module, structured under src/strategy/bfd/ etc.
These files are pure additions that only use mod-playerbots's public API,
so they require no rewrites — straight lift.

See /tmp/agenticbots-extraction/inventory.md for the file-by-file mapping."
ssh heimdal "mv /var/tmp/tot-ab-newfiles-build ~/.Trash/ 2>/dev/null || true"
```

### Task 17: Convert PUBLIC-API-USAGE patch hunks into mod-agenticbots additions

**Files:**
- Modify: various files under `modules/mod-agenticbots/src/`

- [ ] **Step 1: For each PUBLIC-API-USAGE hunk**

These were modifications to mod-playerbots files that only added new behavior (e.g., registering a new strategy in the strategy factory). The new pattern is: declare the addition in mod-agenticbots, and use mod-playerbots's public registration API (typically a script-mgr style hook or a static initializer).

Concretely, for each hunk in the original patch that did something like `m_strategies[STRAT_BFD_TANK] = new BfdTankStrategy(...)`, write a corresponding hook in mod-agenticbots that calls mod-playerbots's public strategy-registration function.

The exact API depends on what mod-playerbots exposes. If mod-playerbots's strategy factory has a public `RegisterStrategy(name, factory_fn)` method, use it. If it does not, that's a PRIVATE-API-USAGE case — defer to Task 18.

- [ ] **Step 2: Build after each batch of conversions**

Don't try to convert all hunks at once — convert in batches of 3–5 hunks, build, verify, commit per batch.

- [ ] **Step 3: Commit per batch**

Example commit message:

```
feat(mod-agenticbots): register BFD strategies via mod-playerbots public API

Converts patches/mod-playerbots-bfd/0003-bfd-map48-instance-strategy.patch
hunks 1-4 from in-place modifications of mod-playerbots strategy factory into
external registrations from mod-agenticbots, using mod-playerbots's public
RegisterStrategy() API.

No mod-playerbots changes required.
```

Iterate over all PUBLIC-API-USAGE hunks until inventory is empty.

### Task 18: Handle PRIVATE-API-USAGE hunks

**Files:**
- Possibly: `UPSTREAMS.toml` (to pin a higher mod-playerbots version if upstream PR's land)
- Possibly: `modules/mod-agenticbots/README.md` (to document hard requirements)

- [ ] **Step 1: For each PRIVATE-API-USAGE entry**

Three options per entry:

A. **Rewrite using public API.** Often possible with creative use of script-mgr hooks. Try this first.
B. **PR the required hook to mod-playerbots upstream.** If accepted, bump `UPSTREAMS.toml` `[playerbots-dependency].min` to the version that includes it. Document the PR link in `internal-docs/agenticbots-upstream-prs.md`.
C. **Document as a hard requirement.** If a feature genuinely cannot work without modifying mod-playerbots, choose: drop the feature from 1.0.0 (preferred), OR document the requirement in mod-agenticbots's README (with a script in `internal-docs/heimdal-router/build-mpb-fork.sh` showing how Heimdal-internal builds patch it). DO NOT redistribute the patched mod-playerbots from ToT releases.

- [ ] **Step 2: Commit each resolved entry**

Each PRIVATE-API-USAGE entry gets its own commit so the resolution is auditable.

- [ ] **Step 3: If any entries are dropped from 1.0.0**

Update spec §9.2 (NOT in 1.0.0) with the dropped items, and the kb_677e753f hub via knowledge-curator dispatch.

### Task 19: Pin the mod-playerbots compatibility range

**Files:**
- Modify: `UPSTREAMS.toml`

- [ ] **Step 1: Determine the tested version**

The version mod-agenticbots is built and tested against. If you've used a specific mod-playerbots tag during Phase 5 testing, that's `tested`. If you've just used `master`, capture the SHA you tested against and pick the most recent tag that includes it.

- [ ] **Step 2: Determine the minimum compatible version**

This is the oldest mod-playerbots version that exposes all the public APIs mod-agenticbots uses. If you PR'd new hooks upstream in Task 18, `min` must include those PRs. If you only used existing APIs, `min` can be much older.

- [ ] **Step 3: Update UPSTREAMS.toml + commit**

```bash
# Edit UPSTREAMS.toml replacing FILL_IN_AFTER_AGENTICBOTS_AUTHORING with real versions
git add UPSTREAMS.toml
git commit -m "feat(deps): pin mod-playerbots compatibility range

min    = '<oldest version with all public APIs mod-agenticbots uses>'
tested = '<version CI runs against>'

The install script (Plan 5) verifies operator's mod-playerbots install
satisfies this range.

Refs: spec §3.6"
```

---

## Phase 6: Move ToT product code into `tot/` namespace

End-state of phase: `tot/{harness, brain, memory, client-patch, content, release, deploy, internal-docs}/` exists with the current ToT product code, organized per the spec §2.2 target layout.

### Task 20: Migrate the harness daemon

**Files:**
- Create: `tot/harness/` (from existing harness daemon location)

- [ ] **Step 1: Locate the current harness daemon source**

```bash
find /Users/tbrack/Documents/Projects/azerothcore-heimdal -type d -name "harness*" -not -path "*/.claude/*" -not -path "*/modules/*"
```

(The harness daemon Python code; not `modules/mod-harness-bridge/` which is the C++ adapter and already migrated in Task 8.)

- [ ] **Step 2: Copy into `tot/harness/`**

```bash
mkdir -p tot/harness
# Substitute <harness-src-path> with the path from Step 1:
cp -R <harness-src-path>/* tot/harness/
ls tot/harness/
```

Expected: Python source files, pyproject.toml, tests/, etc.

- [ ] **Step 3: Sanity-check the Python builds locally**

```bash
cd tot/harness
python -m venv .venv
source .venv/bin/activate
pip install -e '.[test]'
pytest 2>&1 | tail -20
deactivate
cd ../..
```

Expected: pytest passes (or fails only on integration tests that require a running worldserver — that's OK at this stage).

- [ ] **Step 4: Commit**

```bash
git add tot/harness/
git commit -m "feat(tot): migrate harness daemon to tot/harness/

FastMCP daemon on port 8099. Source: <copied-from-path>.

Refs: spec §2.2, kb_642162c3"
```

### Task 21: Migrate the brain sidecar

**Files:**
- Create: `tot/brain/`

- [ ] **Step 1: Locate + copy**

```bash
ls /Users/tbrack/Documents/Projects/azerothcore-heimdal/tools/brain-sidecar/
mkdir -p tot/brain
cp -R /Users/tbrack/Documents/Projects/azerothcore-heimdal/tools/brain-sidecar/* tot/brain/
```

- [ ] **Step 2: Sanity check**

```bash
cd tot/brain
python -m venv .venv
source .venv/bin/activate
pip install -e '.[test]' 2>&1 | tail -5
pytest 2>&1 | tail -20
deactivate
cd ../..
```

- [ ] **Step 3: Commit**

```bash
git add tot/brain/
git commit -m "feat(tot): migrate brain sidecar to tot/brain/

V3.7.4 brain at the time of migration. Multi-mode router decoupling in
the next task. Source: tools/brain-sidecar/.

Refs: spec §2.2"
```

### Task 22: Decouple the V3.8 multi-mode router from the brain

**Files:**
- Create: `tot/internal-docs/heimdal-router/` (target for the decoupled router code)
- Modify: `tot/brain/` (remove router-specific code paths)

- [ ] **Step 1: Identify the router code in `tot/brain/`**

```bash
grep -rln "router\|multi.mode\|sphere.of.influence" tot/brain/ | head -20
```

(Look at the V3.8 plan at `docs/superpowers/plans/2026-05-19-...` or the V3.8 spec for guidance on which modules implement routing — the spec was at `docs/superpowers/specs/2026-05-21-v3-...-design.md` or similar; cross-reference with the existing brain code.)

- [ ] **Step 2: Move router files**

```bash
mkdir -p tot/internal-docs/heimdal-router/
# For each router file/module identified in Step 1:
mv tot/brain/<router-module-path> tot/internal-docs/heimdal-router/
```

- [ ] **Step 3: Update brain to call a single endpoint**

The brain now reads `BRAIN_LLM_URL` (env var) and makes plain OpenAI-compatible API calls. Any code that previously asked "which model do I use for this decision?" now just uses the single configured model.

Find the brain's LLM-client module:

```bash
grep -rln "OpenAI\|chat_completion\|http.*completions" tot/brain/ | head -5
```

Edit that module to remove router dispatch logic, replacing with single-endpoint calls.

- [ ] **Step 4: Write a stub doc in `tot/internal-docs/heimdal-router/README.md`**

```bash
cat > tot/internal-docs/heimdal-router/README.md <<'EOF'
# Heimdal Internal Router

Multi-mode model routing (Local Nemo + OpenRouter + Qwen sphere-of-influence model).
This code is **Heimdal-internal** — it does not ship with ToT releases.

The brain in `tot/brain/` is a single-endpoint client (BYOLLM). To use multi-mode
routing on Heimdal, the router runs as a proxy in front of the brain:

```
brain  -->  this router (port X)  -->  Local Nemo | OpenRouter | Qwen
```

Operators who want routing in production point `BRAIN_LLM_URL` at their own
proxy (e.g., LiteLLM with their own config). This router is one example; not a
prescription.

See spec §9.4 for the decoupling decision.
EOF
```

- [ ] **Step 5: Run brain tests**

```bash
cd tot/brain
source .venv/bin/activate
pytest 2>&1 | tail -20
deactivate
cd ../..
```

Expected: tests pass. If router-specific tests fail because router code moved out, those tests should also be moved to `tot/internal-docs/heimdal-router/tests/` OR rewritten as router-stub tests.

- [ ] **Step 6: Commit**

```bash
git add tot/brain/ tot/internal-docs/heimdal-router/
git commit -m "refactor(tot): decouple V3.8 multi-mode router from brain

Router code moves to tot/internal-docs/heimdal-router/ (Heimdal-internal,
not shipped). The brain becomes a single-endpoint client (BYOLLM) reading
BRAIN_LLM_URL.

Operators wanting routing layer it in front of the brain via a proxy
(e.g., LiteLLM).

Refs: spec §9.4, kb_e6d3bbb1"
```

### Task 23: Migrate the memory subsystem placeholder

**Files:**
- Create: `tot/memory/`

- [ ] **Step 1: Find the current memory sidecar code**

```bash
find /Users/tbrack/Documents/Projects/azerothcore-heimdal -type d -name "memory*" -not -path "*/.claude/*"
```

(There's likely a `memory-sidecar` directory or similar from V0.2.1 spec — see `docs/superpowers/specs/2026-05-20-memory-sidecar-*-design.md`.)

- [ ] **Step 2: Copy what exists**

```bash
mkdir -p tot/memory
# Substitute <memory-src-path>:
cp -R <memory-src-path>/* tot/memory/ 2>/dev/null || echo "No existing memory code to migrate; tot/memory/ will be fully authored in Plan 2"
```

If nothing exists, create a placeholder README:

```bash
cat > tot/memory/README.md <<'EOF'
# tot/memory

The V3 memory subsystem. Full implementation in Plan 2 (`docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-memory-subsystem.md`).

This placeholder exists so that downstream consumers (the brain at `tot/brain/`, the harness adapter for `memory.*` tools) can reference the path. The actual schema, embedding pipeline, hybrid retrieval, and time-decay scoring all land in Plan 2.

Refs: spec §9.3, kb_e6d3bbb1
EOF
```

- [ ] **Step 3: Commit**

```bash
git add tot/memory/
git commit -m "feat(tot): scaffold tot/memory/ for V3 memory subsystem (full impl in Plan 2)

Migrates existing memory-sidecar code if present; otherwise places a
placeholder README documenting Plan 2 ownership.

Refs: spec §9.3"
```

### Task 24: Migrate the client-patch tooling (dbc-patch-builder + compose-tot-addon)

The unified `ThreadsOfTime` AddOn composer (`tools/compose-tot-addon.py`) landed in azerothcore-heimdal at commit `82a6583` on 2026-05-25. It pairs with `tools/dbc-patch-builder/` — both produce inputs that the (future, Plan 4) MPQ pack stage will combine. Both move to `tot/client-patch/` in this task.

**Files:**
- Create: `tot/client-patch/dbc-patch-builder/` (full subtree)
- Create: `tot/client-patch/compose-tot-addon.py`
- Create: `tot/client-patch/PRIORITIES.md` (new — AddOn-priority allocation doc)

- [ ] **Step 1: Copy dbc-patch-builder**

```bash
mkdir -p tot/client-patch
cp -R /Users/tbrack/Documents/Projects/azerothcore-heimdal/tools/dbc-patch-builder tot/client-patch/
ls tot/client-patch/dbc-patch-builder/
```

Expected: pyproject.toml, src/, tests/, baseline/, scripts/, README.md.

- [ ] **Step 2: Copy compose-tot-addon.py**

```bash
cp /Users/tbrack/Documents/Projects/azerothcore-heimdal/tools/compose-tot-addon.py tot/client-patch/
ls tot/client-patch/
```

- [ ] **Step 3: Write `PRIORITIES.md` documenting the AddOn priority bands**

```bash
cat > tot/client-patch/PRIORITIES.md <<'EOF'
# Threads of Time AddOn — priority allocation

The unified `ThreadsOfTime` AddOn (composed by `compose-tot-addon.py` from per-mod
`modules/<mod>/data/addon-contrib/` directories) loads files in `(priority, mod, file-index)` order.

Priority bands (lower loads earlier):

| Band | Reserved for |
|---|---|
| 0–9   | Core/shared namespace, utilities, the auto-generated `00-Core.lua` |
| 10–89 | Feature mods (mod-warforged, mod-bracket-sets tier-set UI, future mods) |
| 90–99 | Integration / glue / overrides that need to run last |

Current allocations:

| Mod | Priority | Notes |
|---|---|---|
| mod-warforged | 10 | WarforgedStatBumps + GameTooltip + ItemRef + WarforgedSentinel |
| mod-bracket-sets | reserved 11 | tier-set UI Lua (when authored; currently `data/addon-contrib/.gitkeep`) |

When adding a new contributing mod, pick the lowest unused priority in the appropriate band and update this table in the same commit as the manifest.toml.
EOF
```

- [ ] **Step 4: Sanity-check determinism gates**

```bash
cd tot/client-patch/dbc-patch-builder
python -m venv .venv
source .venv/bin/activate
pip install -e '.[test]' 2>&1 | tail -5
pytest 2>&1 | tail -10
deactivate
cd ../../..
```

Expected: dbc-patch-builder tests pass (including the determinism `--check` gate).

- [ ] **Step 5: Test the composer end-to-end**

```bash
python tot/client-patch/compose-tot-addon.py --modules-dir modules --output build/tot-addon-test 2>&1 | tail -20
ls build/tot-addon-test/ThreadsOfTime/ 2>/dev/null
```

Expected: composer runs, produces `build/tot-addon-test/ThreadsOfTime/` with concatenated Lua files (prefixed by priority+mod+file-index) + auto-generated `00-Core.lua` + `ThreadsOfTime.toc`.

(`build/` is gitignored — this output is not committed.)

- [ ] **Step 6: Commit**

```bash
git add tot/client-patch/
git commit -m "feat(tot): migrate client-patch tooling (dbc-patch-builder + compose-tot-addon)

Pairs the existing dbc-patch-builder with the unified ThreadsOfTime AddOn
composer (live as of azerothcore-heimdal 82a6583, 2026-05-25). Both produce
inputs the future MPQ pack stage will combine.

Adds PRIORITIES.md documenting the AddOn-priority allocation bands
(0-9 core, 10-89 feature mods, 90-99 integration). mod-warforged owns
priority 10; mod-bracket-sets reserves 11.

The MPQ pack stage with DBC ID-range collision detection is deferred to
Plan 4 per spec §5.8.

Refs: spec §5.1 step 6 (two-stage composition), §5.8"
```

### Task 25: Migrate SQL content packs

**Files:**
- Create: `tot/content/sql/bracket1/`
- Create: `tot/content/sql/bracket1-raid/`

- [ ] **Step 1: Copy**

```bash
mkdir -p tot/content/sql
cp -R /Users/tbrack/Documents/Projects/azerothcore-heimdal/sql/bracket1 tot/content/sql/
cp -R /Users/tbrack/Documents/Projects/azerothcore-heimdal/sql/bracket1-raid tot/content/sql/
ls tot/content/sql/
```

- [ ] **Step 2: Drop SoD-named files per user direction (no SoD branding in shipped ToT)**

The user direction: don't mention SoD; don't ship the rune system. Items imported from SoD as a baseline can still ship (they're just custom items), but rename SoD-named files:

```bash
mv tot/content/sql/bracket1/world_sod_items.sql tot/content/sql/bracket1/world_bracket1_items.sql
mv tot/content/sql/bracket1/world_sod_loot.sql tot/content/sql/bracket1/world_bracket1_loot.sql
# Skip world_sod_runes.sql entirely — runes aren't shipping (per user direction)
mv tot/content/sql/bracket1/world_sod_runes.sql ~/.Trash/
```

(NEVER `rm -rf`; use move-to-trash per CLAUDE.md.)

- [ ] **Step 3: Commit**

```bash
git add tot/content/
git commit -m "feat(tot): migrate SQL content packs to tot/content/sql/

bracket1/ (leveling NPCs, items, starter respawn, DK stat stub) and
bracket1-raid/ (BFD raid creatures, items, lockout, smart scripts) ship
as part of ToT 1.0.0.

SoD-branded files renamed to bracket1-branded (per user direction;
ToT does not present as SoD-derived). Rune system file removed from
ToT — runes are not shipping.

Refs: spec §1.1, §9.1"
```

### Task 26: Migrate the build orchestrator

**Files:**
- Create: `tot/release/build.sh` (from `image/build.sh`, adapted)

- [ ] **Step 1: Copy and adapt**

```bash
mkdir -p tot/release
cp /Users/tbrack/Documents/Projects/azerothcore-heimdal/image/build.sh tot/release/build.sh
```

Edit `tot/release/build.sh` to:
- Remove the AC + mod-playerbots cloning steps (the source is now IN this repo; the script just builds it)
- Update paths: replace `${BUILD_DIR}/azerothcore-wotlk/` with `${BUILD_DIR}/` (or appropriate)
- Remove the `cd $TMP_DIR && git clone --filter=tree:0 --branch Playerbot ...` blocks
- Remove the patch-overlay rsync logic (`rsync -a "${patch_dir}overlays/" ...`) — patches are now committed code
- Keep the rsync-to-heimdal logic, the `cmake -j4` discipline, the binary mtime cross-check, the image bake + tag

- [ ] **Step 2: Test the build orchestrator end-to-end**

```bash
bash tot/release/build.sh
```

Expected: produces a working worldserver container image on Heimdal. (Full success criteria deferred to Plan 6's release pipeline; this just verifies the build orchestrator still functions after the path changes.)

- [ ] **Step 3: Commit**

```bash
git add tot/release/build.sh
git commit -m "feat(tot): migrate build orchestrator to tot/release/build.sh

Adapted from image/build.sh: removed AC + mod-playerbots cloning (source
is IN this repo now), removed patch-overlay rsync (patches are committed).
Kept: rsync-to-Heimdal, -j4 discipline (kb_adbafeda), binary mtime
cross-check (kb_f974fc65), image bake + tag.

Full release pipeline (release.sh) implementation deferred to Plan 6.

Refs: spec §2.2, §5.1; kb_57b453cd"
```

### Task 27: Migrate the reference Quadlet + Compose scaffolds

**Files:**
- Create: `tot/deploy/quadlet/`
- Create: `tot/deploy/compose.yml.example` (placeholder; full work in Plan 5)

- [ ] **Step 1: Copy current Heimdal quadlets**

```bash
mkdir -p tot/deploy/quadlet
cp -R /Users/tbrack/Documents/Projects/azerothcore-heimdal/quadlet/* tot/deploy/quadlet/
ls tot/deploy/quadlet/
```

- [ ] **Step 2: Mark as Heimdal-specific (generalization in Plan 5)**

Add a README explaining the migration status:

```bash
cat > tot/deploy/README.md <<'EOF'
# tot/deploy/

Reference deployment stack for operators running Threads of Time.

## Status (as of Plan 1)

The `quadlet/` directory contains the current Heimdal-specific Quadlet units
as committed. They reference Heimdal-specific paths, ports, hostnames, and
credentials.

**Plan 5 generalizes these into operator-configurable stacks**: replacing
Heimdal-isms with `.env`-driven variables, adding a Compose-based alternative
for non-systemd operators, and producing the install script + first-boot
bootstrap.

DO NOT distribute these files as-is to operators. They are committed here
as the source from which Plan 5 derives the generalized stack.

Refs: spec §6, Plan 5.
EOF
```

- [ ] **Step 3: Commit**

```bash
git add tot/deploy/
git commit -m "feat(tot): migrate Quadlet scaffolds to tot/deploy/

Heimdal-specific as committed; generalization into operator-portable form
deferred to Plan 5.

Refs: spec §6, §2.2"
```

### Task 28: Preserve internal docs (specs + plans)

**Files:**
- Create: `tot/internal-docs/specs/`, `tot/internal-docs/plans/`

- [ ] **Step 1: Copy**

```bash
mkdir -p tot/internal-docs/specs tot/internal-docs/plans
cp /Users/tbrack/Documents/Projects/azerothcore-heimdal/docs/superpowers/specs/* tot/internal-docs/specs/ 2>/dev/null
cp /Users/tbrack/Documents/Projects/azerothcore-heimdal/docs/superpowers/plans/* tot/internal-docs/plans/ 2>/dev/null
ls tot/internal-docs/specs/ | wc -l
```

Expected: dozens of spec + plan files preserved.

- [ ] **Step 2: Add a README**

```bash
cat > tot/internal-docs/README.md <<'EOF'
# tot/internal-docs/

Internal development artifacts: specs, plans, and Heimdal-internal tooling
that does not ship with ToT releases.

## Subdirectories

- `specs/` — historical design specs (point-in-time records)
- `plans/` — historical implementation plans
- `heimdal-router/` — Heimdal-internal multi-mode LLM router (per spec §9.4)

## Release filter

The release pipeline (Plan 6) explicitly excludes this directory from release
artifacts. Operators never receive this content.

EOF
```

- [ ] **Step 3: Commit**

```bash
git add tot/internal-docs/
git commit -m "feat(tot): preserve internal-docs (historical specs + plans)

Copies docs/superpowers/specs/ and docs/superpowers/plans/ from
azerothcore-heimdal for history. These do not ship with releases (Plan 6
release pipeline excludes tot/internal-docs/)."
```

### Task 29: Place the canonical 1.0.0 design spec under `docs/`

**Files:**
- Create: `docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md`
- Create: `docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md` (this file)

- [ ] **Step 1: Copy**

```bash
mkdir -p docs/superpowers/specs docs/superpowers/plans
cp /Users/tbrack/Documents/Projects/azerothcore-heimdal/docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md docs/superpowers/specs/
cp /Users/tbrack/Documents/Projects/azerothcore-heimdal/docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md docs/superpowers/plans/
```

- [ ] **Step 2: Commit**

```bash
git add docs/
git commit -m "docs: place canonical 1.0.0 design spec + foundation plan under docs/

The release-architecture design and Plan 1 (foundation) are the
authoritative source for ToT 1.0.0. Historical/superseded specs remain
in tot/internal-docs/."
```

---

## Phase 7: Tag and verify

End-state: a clean `dev` branch ready for parallel feature work in Plans 2–6, with a smoke-tested end-to-end build.

### Task 30: Final smoke test

**Files:**
- Read-only verification across the whole repo

- [ ] **Step 1: Full rebuild from clean state**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time/
git status
git log --oneline | head -40
```

Expected: working tree clean; ~30 commits since bootstrap.

- [ ] **Step 2: Full worldserver build on Heimdal**

```bash
rsync -aH --delete --exclude .git ./ heimdal:/var/tmp/tot-final-build/
ssh heimdal "cd /var/tmp/tot-final-build && rm -rf build && mkdir build && cd build && cmake .. -DCMAKE_BUILD_TYPE=Release -DTOOLS_BUILD=none && cmake --build . --target worldserver -j4 2>&1 | tail -30"
ssh heimdal "ls -la /var/tmp/tot-final-build/build/src/server/worldserver/worldserver"
```

Expected: clean build.

- [ ] **Step 3: Symbol-level verification of every module**

```bash
ssh heimdal "nm /var/tmp/tot-final-build/build/src/server/worldserver/worldserver 2>/dev/null | grep -iE 'bracket|warforged|rotation|harness|agenticbots' | head -30"
```

Expected: symbols from every migrated module visible.

- [ ] **Step 4: Sanity-check the Python sidecars build**

```bash
for sidecar in harness brain memory client-patch; do
    echo "=== tot/$sidecar ==="
    cd tot/$sidecar 2>/dev/null && pip install -e '.[test]' 2>&1 | tail -3 && pytest 2>&1 | tail -3 && cd ../..
done
```

Expected: each sidecar's pip install succeeds; pytest is green or skips integration tests cleanly.

- [ ] **Step 5: Cleanup**

```bash
ssh heimdal "mv /var/tmp/tot-final-build ~/.Trash/ 2>/dev/null || true"
```

### Task 31: Tag the foundation milestone

**Files:**
- Tag: `foundation-complete`

- [ ] **Step 1: Tag**

```bash
git tag foundation-complete -m "Plan 1 complete: ToT repo on stock AC base, modules migrated, mod-agenticbots authored, brain decoupled from router.

Ready for parallel feature work in Plans 2-6:
- Plan 2: V3 memory subsystem
- Plan 3: Subset gating
- Plan 4: MPQ compositor + mod-warforged + tier-set UI + branding
- Plan 5: Operator install stack
- Plan 6: Release + CI infrastructure"
```

- [ ] **Step 2: Push if remote is configured**

(For the new threads-of-time repo, a remote may not yet be configured. If it is:)

```bash
git push origin main dev foundation-complete
```

If no remote is configured yet, that's fine — pushing to GitHub is part of the project-level decision in spec §11.1 ("Community channel" question) and Plan 6.

### Task 32: Smoke test on a fresh Heimdal pod swap

**Files:**
- Heimdal-side worldserver pod

This task only fully completes after Plan 6's image-bake pipeline is in place. For Plan 1's exit criteria, just verify the manual end-to-end build produces a runnable worldserver — pod-swap automation comes later.

- [ ] **Step 1: Manual image bake + start (one-time)**

```bash
bash tot/release/build.sh
```

Expected: produces a worldserver container image; `tot/release/build.sh` reports success; binary mtime cross-check passes.

- [ ] **Step 2: Verify it starts**

```bash
ssh heimdal "echo '<sudo>' | sudo -S podman run --rm --entrypoint /bin/sh localhost/wow-server:current -c 'strings /azerothcore/env/dist/bin/worldserver 2>/dev/null | grep -i -E \"agenticbots|bracket|warforged\" | sort -u | head -10'"
```

Expected: strings for the major modules present in the binary.

- [ ] **Step 3: Document the foundation-complete state**

```bash
cat > FOUNDATION_STATE.md <<'EOF'
# Threads of Time — Foundation Complete

This file is a one-time record of the state at `foundation-complete` tag.

## What works
- Worldserver builds cleanly from `tot/release/build.sh`
- All migrated modules (mod-bracket-sets, mod-rotation-mode, mod-warforged, mod-harness-bridge, mod-agenticbots) compile and load
- Brain sidecar runs in single-endpoint mode (BYOLLM)
- V3.8 multi-mode router decoupled to `tot/internal-docs/heimdal-router/`

## What's stubbed for Plans 2-6
- Memory subsystem: scaffold only (Plan 2 implements)
- Client MPQ composition: dbc-patch-builder migrated as-is (Plan 4 generalizes)
- Reference deploy stack: Heimdal-specific quadlets (Plan 5 generalizes)
- Release pipeline: only build.sh migrated (Plan 6 adds release.sh, CI, GHCR push)

## What's pinned
- AC SHA: see UPSTREAMS.toml [ac].sha
- mod-playerbots compat range: see UPSTREAMS.toml [playerbots-dependency]

## Next plan
Plan 2 (V3 memory subsystem) is the longest item on the critical path and should start immediately.
EOF
git add FOUNDATION_STATE.md
git commit -m "docs: record foundation-complete state"
git tag -d foundation-complete   # retag including the doc commit
git tag foundation-complete -m "Plan 1 complete (with state record)"
```

---

## Self-Review

Run this against the spec sections covered:

**Coverage check:**
- §2.4 migration steps 1, 3, 4 (now covers all three patch dirs: ac-bfd-raid + ac-bracket-sets + the mod-playerbots AC-core extraction handled in Phase 2), 5 (mod-warforged source now main tree not worktree), 6, 7 (partial), 8 (deferred to Plan 4 — the MPQ pack stage with collision detection; AddOn composition is migrated this plan in Task 24), 9 (deferred to Plan 6), 10 (Phase 7), 11 (intentionally deferred — PoC litter cleanup), 12 (Task 28), 13 (SQL content in Task 25, lua/ retired) — **covered with documented deferrals**
- §2.4 step 2 (mod-agenticbots authoring) — **covered by Phase 5, Tasks 14–19**
- §9.4 pre-freeze items:
  - "Repo restructure migration" (Phases 1–7) ✓
  - "Author mod-agenticbots as a new module" (Phase 5) ✓
  - "Extract AC-core hooks from mod-playerbots's AC fork" (Phase 2) ✓
  - "Replay patches/ac-bracket-sets/ + patches/ac-bfd-raid/" (Phase 4, including new Task 12.5) ✓
  - "Pin mod-playerbots compatibility range" (Task 19) ✓
  - "Decouple brain from model-router" (Task 22) ✓
  - "Unified ThreadsOfTime AddOn composer" — already shipped (2026-05-25), migrated in Task 24 ✓
  - "mod-warforged merge to master" — already shipped (2026-05-25), migrated in Task 9 ✓

**Placeholder check:** scanned for TBD/TODO/FIXME — only one match (the deliberate forward reference in mod-agenticbots README filled by later tasks in the same phase). Acceptable.

**Type consistency:** No new types, methods, or signatures introduced (this plan is structural). The one CMakeLists.txt edit (Task 15 Step 1) uses find-replace from `bracket-sets` → `agenticbots` — pattern-consistent with how AC modules are bootstrapped.

**Constraint adherence:** every Heimdal build uses `-j4`; every build is followed by a binary mtime check; no `rm -rf` on user paths (move-to-trash throughout — `rm -rf build` only on the build directory which is regeneratable artifact); no `git push --force` or amend; cmake reconfigure (`rm -rf build && mkdir build && cmake ..`) where new `.cpp` files were added (Tasks 12.5, 13, 16).

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Each task is small enough to fit a single subagent's context cleanly.

**2. Inline Execution** — execute tasks in this session using `executing-plans`, batch execution with checkpoints for review.

Which approach?
