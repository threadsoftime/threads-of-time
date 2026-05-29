# Operator Install Stack — State (Plan 5, ToT 1.0.0)

**Date:** 2026-05-29
**Branch:** `dev` · **Plan:** `docs/superpowers/plans/2026-05-29-threads-of-time-1.0.0-operator-install.md` · **Design:** `docs/superpowers/specs/2026-05-29-threads-of-time-1.0.0-operator-install-design.md`
**Commits:** 36 on `dev` (`cb3ad2925`..`175e50509`).

## Status

**Authoring COMPLETE and committed on `dev` (Phases 0–9 + state doc).** All work that can be done + validated on the macOS dev host is done. **Go-live is PENDING** — it requires the GHCR org, a Heimdal off-gaming-hours build window, and live runs (see Carry-forwards §A). The `operator-install-complete` tag is **NOT yet set** (it gates on a green Tier-2 e2e).

## What shipped (this plan)

| Area | Files | Notes |
|---|---|---|
| Operator `.env` surface | `tot/deploy/.env.example` | Full §6.3 surface + resolved-by-install `AC_*_DATABASE_INFO` placeholders + bearer aliases. |
| Reference Quadlet stack | `tot/deploy/quadlet/*` | Generalized in place: `.env`-driven, `@TOT_HOME@`/`@TOT_VERSION@`/`@GHCR_NS@` install-time tokens, dev `…/source` bind-mount stripped, GHCR images. New units: `tot-firstboot`, `tot-brain`, `tot-memory`. Heimdal-only bracket/dungeon units relocated to `tot/internal-docs/deploy/`. |
| Reference Compose stack | `tot/deploy/compose.yml` | Hand-authored translation (native `${VAR}` interpolation). All 7 services. `docker compose config` validated. |
| Quadlet gate | `tot/deploy/validate-quadlets.sh` | Renders tokens → greps the RENDERED output for Heimdal literals. Green. |
| First-boot bootstrap | `tot/deploy/firstboot/{firstboot.sh,apply_content_sql.py,dbimport.conf.tmpl}`, `tot/content/sql/apply-order.toml` | db-import image wraps AC `dbimport` (stock+module SQL, idempotent) + explicit-manifest content-SQL pass + realm seed + token + `bootstrap.flag`. 4 pytest green. |
| Operator CLI | `tot/deploy/bin/{tot,tot-backup.sh}` | `up`/`down`/`restart`/`logs`/`run-console`/`upgrade`; backup = mysqldump 4 DBs + memory.sqlite snapshot + retention. shellcheck-clean. |
| Release pipeline | `tot/release/{release.sh,build-images.sh}`, `tot/memory/Containerfile` | 11-step `release.sh` (`--dry-run`/`--skip-images`/`--skip-tests`); `build-images.sh` builds 6 images (wraps `build.sh` for worldserver+db-import). Memory Containerfile was missing (lived in mod-playerbots) — added; factory = `tot_memory.app:create_app`. Dry-run exercised steps 1,2,4,6,7,8 locally. |
| Install script | `install-tot.sh`, `tot/deploy/render-quadlets.sh` | 7-step bootstrap; resolves `AC_*_DATABASE_INFO` + bearer aliases + writes harness `tokens.yaml` (schema from `harness_daemon/config.py`). Scaffold test green. |
| Operator docs | `docs/{install,operator-troubleshooting,byollm-setup}.md` | Grep gates green. Links existing `docs/player-install.md`. |
| Integration tests | `tot/release/integration/{tier1_live_probe.py,tier2_e2e.sh,playerbots_runtime_probe.md}` | Tier-1 read-only live probe (soft); Tier-2 isolated-pod e2e (hard gate); C++ probe call-path documented. Authored; not yet run (need live/GHCR). |
| CI | `.github/workflows/tot-{ci,release-branch,release,nightly-ac-drift}.yml` | GH-hosted: push/PR tests; release-branch rc images; tag→Release; nightly AC-drift. Valid YAML. |

## Locked decisions (design §2)
1. Quadlets generalized in place + dev-isms stripped; Compose hand-authored. 2. First-boot = thin db-import wrapper (AC dbimport + apply-order.toml). 3. GH-hosted CI now; worldserver build stays on-demand `build.sh`; self-hosted runner → 1.1.0. 4. Two-tier integration (live read-only smoke + isolated-pod e2e).

**GHCR image set = 6** (not the kb's 4): `worldserver`, `authserver`, `db-import`, `harness`, `brain`, `memory`. `db-import` is the init-container image — the only image bundling AC base/updates + all module SQL (incl. mod-playerbots' 62 files).

## Verified locally
- `python3 -m pytest tests/firstboot/` → 4 passed.
- `tot/client-patch` compositor suite collects 53 tests (kb says "70" — **actual is 53**).
- `bash tot/deploy/validate-quadlets.sh` → OK.
- `docker compose -f tot/deploy/compose.yml config` → exit 0.
- shellcheck clean (error-level) on `firstboot.sh`, `tot`, `tot-backup.sh`, `build-images.sh`, `release.sh`, `install-tot.sh`, `render-quadlets.sh`, `tier2_e2e.sh`.
- `release.sh --dry-run --skip-images --skip-tests` → steps 1,2,4,6,7,8 ran clean.
- `install-tot.sh` scaffold test → all assertions pass (secrets, AC DB info, bearer aliases, tokens.yaml, mode).

## Carry-forwards

### A. Go-live batch (REQUIRES the user + a Heimdal off-gaming-hours window)
1. **Provision GHCR org** `ghcr.io/threadsoftime` with push perms (user action).
2. **Build + push the 6 images** at the release tag: run `build-images.sh <ver>` (invokes `build.sh` on Heimdal, `-j4`, binary-mtime cross-check) with `PUSH=1`. Heavy build — schedule off gaming hours.
3. **Tier-2 e2e** (`tier2_e2e.sh <ver>`) on an isolated pod — the hard gate. Requires the images from step 2.
4. **Tier-1 live probe** + **C++ runtime probe** against the live baseline (read-only, safe).
5. **Tag** `operator-install-complete` once Tier-2 is green.
6. **kb refresh** (`knowledge-curator`): `kb_a217a790` (6 images; dbimport+apply-order reality; clean stale Plan-4 pre-flight), `kb_87a7eade` (Thread L done + new thread), `kb_677e753f` (scope-freeze rows shipped).

### B. Must be verified on Linux/Tier-2 (can't on macOS)
- **Quadlet `${VAR}` non-interpolation:** confirm `EnvironmentFile`-fed `AC_*_DATABASE_INFO` reach the containers and the `$$`-escaped DB `HealthCmd` renders a literal `$` (`systemctl cat tot-database`).
- **`ac_bridge_url` default** (`http://127.0.0.1:8080` in `tokens.yaml`): confirm against the port `mod-harness-bridge` actually listens on in the worldserver.
- **Vestigial bind-mount:** confirm mod-playerbots bots spawn after the stripped `…/source/modules` mount (`obs.list_bot_population` non-zero). If zero → bake `modules/mod-playerbots/data/sql` into the worldserver image (Dockerfile follow-up — do NOT do speculatively).

### C. Code-layer follow-ups (other agents)
- **Brain bearer var inconsistency** → `agent-orchestration-architect`: `brain_sidecar/settings.py` reads `HARNESS_BEARER`/`MEMORY_BEARER`; `memory_client.py` reads `HARNESS_BEARER_TOKEN`. `.env` sets all three to the same value as a stopgap; reconcile the code.
- **Memory bind vars:** `MEM_BIND_HOST`/`MEM_BIND_PORT` aren't wired into `tot_memory/__main__.py` (hardcoded `0.0.0.0:8090`). Harmless today; wire if operator bind config is wanted.
- **AC-API correction (spec/kb):** the spec/kickoff named a `IsPlayerBot()` / `PlayerbotsMgr::GetPlayerBotsCount()` probe — **neither exists in this fork.** Real path: `GET_PLAYERBOT_AI(p) != nullptr` over `sRandomPlayerbotMgr.GetAllBots()` (see `playerbots_runtime_probe.md`). Correct the design doc + kb wording.

### D. Polish / deferred to 1.1.0–1.0.x (design §5)
- `docs/player-install.md` still has Heimdal-private framing — generalize for public operators.
- The 14 stock-AzerothCore `.github/workflows/` still trigger on `push: master` — prune/disable (low priority).
- Deferred per design: signed releases / cosign / SBOM; auto-triggered Heimdal self-hosted runner; Grafana dashboard; `tot memory vacuum` + advanced day-2.
