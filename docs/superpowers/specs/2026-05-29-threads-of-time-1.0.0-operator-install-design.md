# Threads of Time 1.0.0 — Operator Install Stack (Design)

**Date:** 2026-05-29
**Plan:** 5 of 5 — the final plan of the ToT 1.0.0 roadmap. Closes 1.0.0.
**Owner agent:** `deploy-orchestrator` (release pipeline, deploy stack, install script, first-boot, CI, Heimdal integration runs) · `cpp-systems-engineer` (C++ live runtime probe) · `knowledge-curator` (kb hygiene at completion).
**Parent spec (authoritative):** `docs/superpowers/specs/2026-05-24-threads-of-time-1.0.0-design.md` §§5–8, §9.1/§9.4.
**Design-locked source:** `kb_a217a790` (Release Pipeline + Operator Install). Most of §§5–8 is already settled there; this doc confirms against on-disk reality and resolves the still-open decisions.

---

## 1. Context & purpose

Plans 1–4 shipped: Foundation (`foundation-complete`), V3 Memory (`memory-subsystem-complete`), Subset Gating (`subset-gating-complete`), MPQ Compositor (`mpq-compositor-complete`). What remains for 1.0.0 is the **operator install stack**: the release pipeline that produces published artifacts + GHCR images + a GitHub Release, an operator-portable deploy stack generalized from the Heimdal-specific quadlets, a one-command install script + first-boot bootstrap, operator docs, the integration tests Plans 2–4 deferred here, and the CI cadence.

**"Plan 5 done" looks like:** a fresh operator runs `curl … install-tot.sh | sh`, answers the prompts, `tot up`, and reaches a working ToT server (worldserver + bots + brain + memory + harness) with zero Heimdal-specific assumptions. `release.sh vX.Y.Z` produces the published artifacts + GHCR images + GitHub Release. The deferred Heimdal e2e + live runtime probes pass. Operator docs cover install / troubleshoot / BYOLLM.

### 1.1 On-disk ground truth verified for this design (2026-05-29)

These findings (not memory) shape the decisions below:

- **`release.sh` / `build-images.sh` do not exist.** `tot/release/` contains only `build.sh` (a Heimdal SSH+rsync worldserver builder) and `build-mpq.sh`. `build.sh`'s own comment defers the full release pipeline to "Plan 6." The 11-step pipeline is greenfield.
- **First-boot's "apply `tot/content/sql/` in dated order" (§6.4) is inaccurate.** `tot/content/sql/` files are *undated* (`world_bracket1_loot.sql`, `characters_wipe_bots.sql`) split across `bracket1/` + `bracket1-raid/`. The *dated* deltas live elsewhere: `modules/mod-bracket-sets/data/sql/world/2026_05_13_00_*.sql` (10 files) and `modules/mod-rotation-mode/data/sql/characters/2026_05_13_00_*.sql` (1 file). Brain + memory are **SQLite**, applied by their own sidecar entrypoints (`tot/brain/migrations/`, `tot/memory/src/tot_memory/db/migrations/`), **not** the MySQL init container. AC ships a native `dbimport` tool that already orders + tracks stock + module SQL via an `updates` table.
- **No ToT CI exists.** All 14 `.github/workflows/` are stock AzerothCore (`codestyle`, `core-build-pch`, `windows_build`, `macos_build`, …). No `ghcr.io` reference exists outside design/spec docs.
- **Deploy quadlets are Heimdal-specific AND carry dev-only patterns.** Hardcoded `192.168.1.3:8099`, `/opt/containers/wow/*`, `/var/mnt/nas/backups`, `brackin`. They bind-mount `/opt/containers/wow/source` at runtime (the dev rsync-overlay pattern) and reference `localhost/wow-server:current` (local build) — neither is appropriate for operators. The repo-root `docker-compose.yml` is stock AC, not a ToT operator stack. `tot/deploy/README.md` already states "DO NOT distribute these files as-is" and that Plan 5 generalizes them.

---

## 2. Locked decisions

| # | Decision | Resolution |
|---|---|---|
| 1 | **Deploy-stack generalization** | Parameterize the existing Heimdal quadlets **in place** (`.env`-driven), **strip dev-isms** (drop `…/source` runtime bind-mounts; repoint worldserver `localhost/wow-server:current` → `ghcr.io/threadsoftime/worldserver:${TOT_VERSION}`), and **hand-author `compose.yml`** as a faithful translation of the generalized Quadlet topology. Quadlet is the primary artifact; Compose is the alternative (both ship; install prompts which). |
| 2 | **First-boot DB bootstrap** | A **thin init container** wrapping AC's native `dbimport` (stock base + updates + module SQL — AC tracks applied updates in its `updates` table, natively idempotent), **plus an explicit-manifest pass** for `tot/content/sql/`, then realm/GM/token seed + `bootstrap.flag`. Reuses the worldserver image (already contains `dbimport` + module SQL). Brain + memory SQLite stay sidecar-applied. |
| 3 | **CI runner topology** | **GH-hosted CI now**; the **worldserver image build stays the existing `build.sh`**, invoked on-demand by `release.sh` (the gated Heimdal step). The **auto-triggered self-hosted runner is deferred to 1.1.0** — no remote push/tag event ever drives a `-j4` build on the always-on gaming PC. The **nightly AC-drift job is GH-hosted** and ships in Plan 5. |
| 4 | **Deferred integration tests** | **Two-tier.** Tier-1: read-only live-probe smoke (safe against the live baseline), a soft pre-release check. Tier-2: full e2e (fresh install → first-boot → bot-greets-player) on an **isolated ephemeral pod** (distinct pod name / ports / DB / volumes, never touching live), a **hard gate** before cutting `release/1.0`, run in a bounded window off the user's gaming hours. |

**Scope reconciliation:** the kickoff folds CI into Plan 5 (the parent spec §9.4 had it as Plan 6). Resolution: the **GH-hosted CI + nightly drift job ship in Plan 5**; only the **auto-triggered self-hosted worldserver-build runner defers to 1.1.0**. This closes 1.0.0 as intended without letting GitHub events saturate Heimdal.

---

## 3. Components & deliverables

### 3.A Reference deploy stack — `tot/deploy/`

**3.A.1 Generalize the Quadlet stack (in place).** For each unit under `tot/deploy/quadlet/`:

- Replace every hardcoded value with an `.env` / systemd `EnvironmentFile` variable: `192.168.1.3` → `${HARNESS_BIND}` host; `/opt/containers/wow/*` → `${TOT_HOME}/*`; `/var/mnt/nas/backups` → `${TOT_BACKUP_DIR}`; image refs → `ghcr.io/threadsoftime/<svc>:${TOT_VERSION}`.
- **Remove dev-only runtime bind-mounts** (`/opt/containers/wow/source/{modules,src}` → container) — operators run the baked image; nothing is bind-mounted from a host source tree. Config (`etc/`), logs, SQL content, secrets, and data volumes remain (parameterized).
- **Add the init-container unit** (`tot-firstboot.container`, `Type=oneshot`, ordered `Before=` worldserver, gated by `bootstrap.flag`).
- Keep the four-service topology: `tot-database` (MySQL 8.x), `tot-firstboot` (init), `tot-authserver`, `tot-worldserver`, plus the sidecar pod (`harness-daemon`, `brain`, `memory`). Pod + named volumes parameterized.

**3.A.2 Author `tot/deploy/compose.yml`** — a hand-written Compose translation of the generalized Quadlet (the repo-root `docker-compose.yml` is stock AC and is left untouched). Same services, same `.env` surface, `depends_on` ordering that reproduces the init-container gate. `docker compose` and `podman-compose` compatible.

**3.A.3 `tot/deploy/.env.example`** — expand from the current subset-gating-only file to the full §6.3 surface: Identity (`TOT_VERSION`, `TOT_REALM_NAME`, `TOT_REALM_HOST`), Database (`MYSQL_ROOT_PASSWORD` + per-DB users, all `<generated>`), LLM/BYOLLM (`BRAIN_LLM_URL/MODEL/API_KEY`, `BRAIN_EMBEDDINGS_URL/MODEL/API_KEY`), Harness (`HARNESS_BEARER_TOKEN`, `HARNESS_BIND`), Bots (`TOT_BOT_POPULATION`, `TOT_LIVING_BOT_COUNT`, `TOT_BOT_MIN/MAX_LEVEL`), Resource limits, and the existing subset-gating vars. Every variable carries an explanatory comment. No multi-endpoint LLM fields (proxy-in-front pattern, documented in `byollm-setup.md`).

**3.A.4 Stays Heimdal-only** — moved to `tot/internal-docs/deploy/` (not distributed): the bracket-progression automation (`wow-bracket-trigger.{service,path}` + `wow-bracket-trigger-runner.sh` + `advance-bracket.sh` — a dev progression tool, irrelevant to a single-bracket 1.0.0 operator), the NAS backup path specifics, the GPU/router host services (`amdgpu-fancurve`, `llama-server-rocm` — already host services, never in the stack), and the `build.sh` SSH/rsync flow. The generalized operator backup uses `tot-backup.sh` → `${TOT_BACKUP_DIR}` (operator-chosen).

### 3.B Release pipeline — `tot/release/`

**3.B.1 `release.sh vX.Y.Z`** — orchestrates the 11-step flow from `kb_a217a790`. Runs locally on the maintainer's control machine; aborts safely at any step (nothing published until step 9):

1. Verify `git status` clean, on `release/X.Y`, tag `vX.Y.Z` not already used.
2. Read `UPSTREAMS.toml` for the pinned AC SHA (assert HEAD's AC base matches).
3. Full test suite (unit + schema parity + compositor determinism); abort on red.
4. AC patch series: `git format-patch upstream/ac..HEAD -- src/ deps/ apps/ data/` → `release-artifacts/ac-patches/`.
5. Build + tag container images via `build-images.sh` → `:X.Y.Z` (see 3.B.2).
6. Compose the client MPQ via the shipped two-stage pipeline (Stage A `compose-tot-addon.py` + Stage B `pack-mpq.py`, already live as of Plan 4) — `build-mpq.sh vX.Y.Z`.
7. Bundle the reference deploy stack → `release-artifacts/tot-X.Y.Z-stack.tar.gz` (the generalized `quadlet/`, `compose.yml`, `.env.example`, `docs/`, `install-tot.sh`, `tot-backup.sh`).
8. SHA-256 checksums → `release-artifacts/SHA256SUMS`.
9. Tag `vX.Y.Z`, push tag + branch.
10. Push images to GHCR (`ghcr.io/threadsoftime/{worldserver,harness,brain,memory}:X.Y.Z`).
11. Create the GitHub Release; upload artifacts + checksums + changelog.

**3.B.2 `build-images.sh`** — builds and tags the four images at `:X.Y.Z`:
- `harness`, `brain`, `memory` — buildable anywhere (GH-hosted CI builds them on `release/X.Y`; `build-images.sh` can also build them locally for a maintainer release run).
- `worldserver` — **invokes the existing `build.sh`** (the gated Heimdal SSH builder, `-j4`, binary-mtime cross-check, static boost, MySQL-version pin), then retags/pushes its output to GHCR. This is the single heavy step that touches Heimdal; it is operator-triggered (decision #3), never auto-fired.
- mod-playerbots is rsync'd/cloned into the ephemeral build tree only (never vendored — §10.2 / `kb_acf2bfd4` item 26); the resulting worldserver image satisfies the runtime dependency for container operators.

### 3.C Install script — `install-tot.sh`

POSIX `sh`, self-contained, distributed both as a curl-able one-liner and a release artifact. Per §6.2:

1. Verify `podman` (4+) or `docker` (24+); clear-message abort if missing.
2. Verify mod-playerbots reachability — **source-build operators only**: already in `modules/mod-playerbots/`, or offer to clone upstream at the range in `UPSTREAMS.toml [playerbots-dependency]`. Operators pulling the pre-built worldserver image skip this (the image bakes it in).
3. Scaffold `$TOT_HOME` (default `/opt/tot`): `compose.yml`, `quadlet/`, `.env.example` → `.env`, `secrets/`, `data/`, `docs/`.
4. Generate random MySQL passwords + `HARNESS_BEARER_TOKEN` → `secrets/`.
5. Pull all images by exact tag (`:${TOT_VERSION}`).
6. Prompt: realmlist hostname (`TOT_REALM_HOST`), LLM endpoint URL + API key (skippable), Quadlet vs. Compose mode.
7. Print next-step instructions (`tot up`, where to edit `.env`, where docs live).

Operators wanting full control skip the script and use `tot-X.Y.Z-stack.tar.gz` directly.

### 3.D First-boot bootstrap (init container)

Image base: **reuse the worldserver image** (already contains AC `dbimport` + all module SQL + a MySQL client). `Type=oneshot`, ordered before worldserver, gated by `data/bootstrap.flag`. Idempotent; every failure logs the exact step + a recovery suggestion and exits non-zero.

Steps (corrects §6.4 against on-disk reality):

1. Wait for MySQL with retry/backoff (`kb_9d1289dd` pattern; reference Quadlet also ships `Restart=on-failure RestartSec=15`).
2. Create the four DBs: `tot_auth`, `tot_characters`, `tot_world`, `tot_playerbots`.
3. Run AC `dbimport` — applies AC stock base + AC updates + **module SQL** (`modules/*/data/sql/{world,characters}/*.sql`, dated, DB-routed by directory). AC's `updates` table makes this idempotent and correctly ordered; ToT inherits it for free.
4. Apply **`tot/content/sql/`** via a new explicit ordering manifest **`tot/content/sql/apply-order.toml`** (each entry: relative path + target DB), since these files are undated and routed by filename prefix today. Init reads the manifest in declared order. This replaces the inaccurate "dated order" assumption with a single source of truth for the content-SQL sequence.
5. Seed the default realm row pointing at `TOT_REALM_HOST` / `TOT_REALM_NAME`.
6. Seed default GM accounts (`admin/admin`, "change me" notice in logs + docs).
7. Generate `secrets/harness-token` if absent.
8. Write `data/bootstrap.flag` (subsequent boots skip).

**Out of init-container scope (explicit):** brain + memory SQLite migrations run from their own sidecar entrypoints at startup. The init container never touches them.

### 3.E Operator docs — `docs/`

- `docs/install.md` — prerequisites (§6.1), the `curl | sh` happy path, the manual stack-tarball path, first `tot up`, verifying a healthy boot.
- `docs/operator-troubleshooting.md` — the §6.8 failure-mode table (MySQL race / BYOLLM unreachable / memory-store corruption / out-of-disk) with the exact recovery for each, plus day-2 ops (§6.5).
- `docs/byollm-setup.md` — pointing `BRAIN_LLM_URL` at Ollama / llama.cpp / hosted OpenAI-compatible endpoints; the LiteLLM-proxy-in-front pattern for fancy routing.
- `.env.example` is the documented config surface (3.A.3).
- The **player-install doc shipped in Plan 4** (MPQ rename + `itemcache.wdb` clear) — Plan 5 verifies it is current and links it from `docs/install.md`.

### 3.F Deferred integration tests (two-tier)

**Tier-1 — read-only live-probe smoke (soft gate).** Runs against the **live Heimdal baseline**, all read-only and safe:
- Harness MCP/HTTP probes: `obs.ping`, `obs.list_bot_population`, `obs.list_players`, a `memory.recall` round-trip (read), `obs.get_state` shape check.
- The **C++ `IsPlayerBot()` / `PlayerbotsMgr` live runtime probe** deferred from subset gating (`cpp-systems-engineer`): confirm `PlayerbotsMgr` sees the enrolled bot population and `IsPlayerBot()` returns true for them at runtime (via a console command / obs tool, no state mutation).
- Surfaces stale-response / schema-drift early; runnable anytime; a documented soft pre-release check, not a blocking CI gate.

**Tier-2 — isolated-stack e2e (hard gate before `release/1.0`).** On an **ephemeral pod** with distinct pod name / ports / DB / volumes, never touching the live stack:
- Run `install-tot.sh` → `tot up` → first-boot completes → worldserver listens → a player joins a party with a living bot → bot greets by name (memory + brain + harness end-to-end) → tear the pod down.
- Bounded resources (small `TOT_BOT_POPULATION`, e.g. 5) and scheduled off the user's gaming hours. Hard gate: `release/1.0` is not cut until this passes.

### 3.G CI — `.github/workflows/` (GH-hosted)

New ToT workflows (the 14 stock-AC workflows are left as-is or pruned in a follow-up; not load-bearing here):

| Trigger | Jobs | Host |
|---|---|---|
| Push to `dev` / any PR | Unit + schema-parity + lint + compositor determinism (must keep the 70 client-patch + brain/harness-parity/memory suites green) | GH-hosted |
| Push to `release/X.Y` | Above + build & push `harness/brain/memory:X.Y.Z-rc` to GHCR | GH-hosted |
| Tag `vX.Y.Z*` | Validate artifacts + `SHA256SUMS` + create GitHub Release + push the GH-buildable images (worldserver image supplied beforehand by the maintainer's `build.sh`/`release.sh` run) | GH-hosted |
| Nightly on `dev` | Rebase onto `upstream/ac@HEAD`; conflict report → GH issue | GH-hosted |

GHCR auth via `GITHUB_TOKEN` for GH-hosted jobs. The worldserver build is **not** a CI job in 1.0.0 (decision #3).

### 3.I Operator CLI — `tot` + `tot-backup.sh`

The operator's runtime interface, installed onto `PATH` by `install-tot.sh` (a thin wrapper over `podman`/`systemctl`/`docker compose`, dispatching to whichever mode the operator chose):

- `tot up` / `tot down` — start/stop the stack (Quadlet: `systemctl --user start/stop`; Compose: `compose up -d`/`down`). First `tot up` triggers the init container (3.D).
- `tot restart <service>` — e.g. `tot restart brain` (the §6.5 brain-reload path).
- `tot logs <service>` — `podman logs threadsoftime-<service>`.
- `tot upgrade X.Y.Z` — the §6.6 flow: snapshot DBs → pull new images → stop sidecars → run new SQL migrations (init container in upgrade mode) → restart on new tag → verify healthy. Refuses across-major by default (§6.6); forward-only, no auto-rollback (§6.7).
- `tot-backup.sh` — `mysqldump` of all four DBs + a `data/memory/memory.sqlite` file snapshot (WAL checkpoint first) → `${TOT_BACKUP_DIR}`; 7 daily + 4 weekly retention (§6.5).

`tot memory vacuum` and other advanced day-2 verbs are deferred (§5).

### 3.H Completion

`OPERATOR_INSTALL_STATE.md` (end-state + any carry-forwards), tag `operator-install-complete` on `dev`, and a `knowledge-curator` pass: refresh `kb_a217a790` (mark release.sh / install / first-boot / CI shipped; clean its stale Plan-4 pre-flight sub-section), update `kb_87a7eade` Thread L + add a new thread for the operator install stack, mark `kb_677e753f` scope-freeze rows shipped.

---

## 4. Testing & verification strategy

- **Deploy stack:** lint quadlets (`systemd-analyze verify` where feasible) and validate `compose.yml` (`docker compose config` / `podman-compose config`). A grep-gate asserting no Heimdal-specific literal (`192.168.1.3`, `/opt/containers/wow`, `/var/mnt/nas`, `brackin`) survives in any distributed asset.
- **First-boot:** unit-test the `apply-order.toml` parser + idempotency (second run is a no-op given `bootstrap.flag`); integration-validated by Tier-2 e2e.
- **`install-tot.sh`:** `shellcheck` clean; dry-run mode (`--no-pull --dry-run`) exercised in CI; full path exercised in Tier-2.
- **`release.sh`:** dry-run mode that performs steps 1–8 without publishing (no tag push, no GHCR, no Release); asserts the artifact set + checksums; CI runs the dry-run on `release/X.Y`.
- **Non-regression (non-negotiable):** the 70 client-patch compositor tests (`mpq-compositor-complete`), brain / harness-parity / memory suites must stay green; the live Heimdal baseline (worldserver + 100 bots + llama-server) must not be destabilized — all heavy/destructive testing is isolated-pod (Tier-2) or off-hours.

---

## 5. Scope freeze

**In Plan 5:** generalized deploy stack (Quadlet + Compose + `.env.example`); `release.sh` + `build-images.sh`; `install-tot.sh`; first-boot init container + `apply-order.toml`; operator docs (`install`, `operator-troubleshooting`, `byollm-setup`); two-tier integration tests (incl. the C++ runtime probe); GH-hosted CI + nightly AC-drift; the core `tot` CLI wrapper (`up`/`down`/`restart`/`logs`/`upgrade`) + `tot-backup.sh`; `OPERATOR_INSTALL_STATE.md` + tag + kb refresh.

**Deferred to 1.1.0 / 1.0.x:** signed releases / cosign / GPG / SBOM (§9.2); auto-triggered Heimdal self-hosted runner; `tot memory vacuum` + advanced day-2 tooling; the Grafana dashboard (optional — harness `/metrics` suffices; include a minimal `dashboard.json` only if cheap); launcher / ToTBotChat (§9.2).

---

## 6. Risks, assumptions & open items

- **Assumption — shipping bot module is already in the worldserver image.** Plan 5 is install/release/docs/CI only. It assumes the worldserver image contains the shipping bot module(s) (`foundation-complete` linked all 6). **mod-agenticbots authoring** (§9.1 "needs authoring") is a separate open item, explicitly **out of Plan 5 scope** — noted here as a release precondition, not built here.
- **Risk — migration rot in `tot/deploy/`.** The f094b9f6 repo migration silently broke other tooling via stale paths (bit Plan 4 three times). Every generalized asset MUST be tested (`config`-validated; Tier-2 boot), not trusted because it was "migrated."
- **Risk — GHCR org / repo provisioning.** `ghcr.io/threadsoftime/*` must exist with push perms before step 10 succeeds; release.sh fails loud (and safe — nothing published before step 9) if not.
- **Open — AC `dbimport` invocation details.** The exact `dbimport` entrypoint + how module SQL dirs are registered in the ToT image must be confirmed on disk during planning (verify the worldserver image's dbimport app + module-SQL registration before writing the init container).
- **Open — stock-AC workflow pruning.** The 14 stock-AC `.github/workflows/` trigger on `push: master` and are noise/failures in this fork; pruning/disabling them is desirable but a low-priority follow-up, not a Plan 5 gate.

---

## 7. Definition of done

A fresh operator on clean Linux + podman: `curl … install-tot.sh | sh` → answer prompts → `tot up` → first-boot completes → worldserver listens → player installs the MPQ, edits realmlist, logs in → login screen reads "Threads of Time 1.0.0" with the fan-project disclaimer → player joins a party with a living bot that greets by name → `tot upgrade 1.0.1` runs without manual intervention → `tot-backup.sh` produces a recoverable dump. `release.sh vX.Y.Z` produces the published artifacts + GHCR images + GitHub Release. Tier-1 + Tier-2 integration pass. `OPERATOR_INSTALL_STATE.md` + tag `operator-install-complete`; `kb_a217a790` + `kb_87a7eade` updated. **1.0.0 is closed.**
