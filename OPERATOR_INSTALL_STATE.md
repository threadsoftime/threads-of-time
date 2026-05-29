# Operator Install Stack — State (Plan 5, ToT 1.0.0)

**Date:** 2026-05-29
**Branch:** `dev` · **Plan:** `docs/superpowers/plans/2026-05-29-threads-of-time-1.0.0-operator-install.md` · **Design:** `docs/superpowers/specs/2026-05-29-threads-of-time-1.0.0-operator-install-design.md`
**Tag:** `operator-install-complete` — pushed 2026-05-29

## Status

**SHIPPED — PUBLICLY RELEASED (2026-05-29)**

All Plan 5 deliverables complete. Clean Tier-2 e2e passed all gates (a–e) with NO workarounds — 5 defects discovered during go-live and fixed (content SQL db names, lockout target db, compose DB connection strings, playerbots DB info env var, install-tot.sh dir permissions). Six images pushed public to GHCR. Public GitHub org + repo live.

**History rewrite (2026-05-29):** `git filter-repo` was run before going public to purge `tot/internal-docs/` from all history (directory contained a live OpenRouter key + harness/memory/brain bearer tokens). All pre-2026-05-29 commit SHAs are now stale/dead; tags were re-pointed and remain valid by name. Secrets rotated: OpenRouter key replaced on Heimdal; leaked bearer tokens to be regenerated.

## Public release facts

| Item | Value |
|---|---|
| **Public org** | `github.com/threadsoftime` (profile README live) |
| **Public repo** | `github.com/threadsoftime/threads-of-time` |
| **Default branch** | `dev` (ToT content); `main` = stock AC base (AC updates flow `main`→PR→`dev`) |
| **Release tag** | `operator-install-complete` |
| **GitHub Discussions** | Enabled (org-level + repo-level); Bug report + QoL/feature issue forms; content/gameplay direction stays maintainer-curated |

## GHCR images — PUBLIC at 1.0.0

| Image | Local SHA | GHCR Digest |
|---|---|---|
| `ghcr.io/threadsoftime/worldserver:1.0.0` | `6f83b41ad650` | `sha256:6655204fe46d724e42bb1e9daad04dadd35fa31f6a500f512ffefad785341df8` |
| `ghcr.io/threadsoftime/authserver:1.0.0` | `5813c385c449` | `sha256:7ee0b990aa3170f28cc9ab2239d324ccc11add76670f2c52be5d7c31312954b7` |
| `ghcr.io/threadsoftime/db-import:1.0.0` | `96e0380984a1` | `sha256:cc72946d6da1200a27fe8e1d92530481e3007b66d6588687504ce9340e6c03ff` |
| `ghcr.io/threadsoftime/harness:1.0.0` | `3ec7b35f48fc` | `sha256:a5b21680c12c9ec79269602d5083efb44ec728710dc3ab6b0560ab6fec3f6e3a` |
| `ghcr.io/threadsoftime/brain:1.0.0` | `bf8e47a7fdc3` | `sha256:297151f5ccab00ca387e40d9dd76d6891c5e4174f95dcd4c246ebcc257ed81af` |
| `ghcr.io/threadsoftime/memory:1.0.0` | `489b9426ac7a` | `sha256:b33ac3f060514da8e0853edeef7498d45566d007442a4e5206b599713b189db8` |

Visibility: PUBLIC. All gates green.

## What shipped (this plan)

| Area | Files | Notes |
|---|---|---|
| Operator `.env` surface | `tot/deploy/.env.example` | Full §6.3 surface + resolved-by-install `AC_*_DATABASE_INFO` placeholders + bearer aliases. |
| Reference Quadlet stack | `tot/deploy/quadlet/*` | Generalized in place: `.env`-driven, `@TOT_HOME@`/`@TOT_VERSION@`/`@GHCR_NS@` install-time tokens, dev `…/source` bind-mount stripped, GHCR images. New units: `tot-firstboot`, `tot-brain`, `tot-memory`. Heimdal-only bracket/dungeon units relocated (in history purge: `tot/internal-docs/` removed). |
| Reference Compose stack | `tot/deploy/compose.yml` | Hand-authored translation (native `${VAR}` interpolation). All 7 services. `docker compose config` validated. |
| Quadlet gate | `tot/deploy/validate-quadlets.sh` | Renders tokens → greps the RENDERED output for Heimdal literals. Green. |
| First-boot bootstrap | `tot/deploy/firstboot/{firstboot.sh,apply_content_sql.py,dbimport.conf.tmpl}`, `tot/content/sql/apply-order.toml` | db-import image wraps AC `dbimport` (stock+module SQL, idempotent) + explicit-manifest `apply-order.toml` content-SQL pass + realm seed + token + `bootstrap.flag`. 4 pytest green. |
| Operator CLI | `tot/deploy/bin/{tot,tot-backup.sh}` | `up`/`down`/`restart`/`logs`/`run-console`/`upgrade`; backup = mysqldump 4 DBs + memory.sqlite snapshot + retention. shellcheck-clean. |
| Release pipeline | `tot/release/{release.sh,build-images.sh}`, `tot/memory/Containerfile` | 11-step `release.sh` (`--dry-run`/`--skip-images`/`--skip-tests`); `build-images.sh` builds 6 images (wraps `build.sh` for worldserver+db-import). Memory Containerfile was missing — added; factory = `tot_memory.app:create_app`. |
| Install script | `install-tot.sh`, `tot/deploy/render-quadlets.sh` | 7-step bootstrap; resolves `AC_*_DATABASE_INFO` + bearer aliases + writes harness `tokens.yaml`. |
| Operator docs | `docs/{install,operator-troubleshooting,byollm-setup}.md` | Grep gates green. Links existing `docs/player-install.md`. |
| Integration tests | `tot/release/integration/{tier1_live_probe.py,tier2_e2e.sh,playerbots_runtime_probe.md}` | Tier-1 read-only live probe (soft); Tier-2 isolated-pod e2e (hard gate). **Tier-2 GREEN** on go-live run. |
| CI | `.github/workflows/tot-{ci,release-branch,release,nightly-ac-drift}.yml` | GH-hosted: push/PR tests; release-branch rc images; tag→Release; nightly AC-drift. Valid YAML. |

## Locked decisions (design §2)

1. Quadlets generalized in place + dev-isms stripped; Compose hand-authored.
2. First-boot = thin db-import wrapper (AC dbimport + apply-order.toml manifest for content SQL).
3. GH-hosted CI now; worldserver build stays on-demand `build.sh`; self-hosted runner → 1.1.0.
4. Two-tier integration (live read-only smoke + isolated-pod e2e).

**GHCR image set = 6**: `worldserver`, `authserver`, `db-import`, `harness`, `brain`, `memory`. `db-import` is the init-container — the only image bundling AC base/updates + all module SQL (incl. mod-playerbots' 62 files).

## Go-live verification (Tier-2 e2e)

Clean run 2026-05-29: all 5 gates passed (a–e). 5 defects discovered and fixed during the run:
- Content SQL used hardcoded `acore_*` db names → fixed to `tot_*`
- `world_bfd_raid_lockout.sql` targeted wrong db (world→characters) → fixed
- compose firstboot/worldserver needed `AC_PLAYERBOTS_DATABASE_INFO` + `AC_*_DATABASE_INFO` overrides → fixed
- install-tot.sh dir permissions blocked container writes → fixed
- Brain bearer env var inconsistency (stopgap: `.env` sets all three) → see Carry-forwards

## Carry-forwards

### A. Secrets + history (DONE as of 2026-05-29)
- `tot/internal-docs/` purged from all history via `git filter-repo` — no longer in repo.
- OpenRouter key rotated + replaced on Heimdal.
- Leaked bearer tokens (`HARNESS_BEARER`, `MEMORY_BEARER`, `HARNESS_BEARER_TOKEN`) to be regenerated by operator on first deploy (generated by install-tot.sh).
- Archive of internal-docs available locally at `/Users/tbrack/Documents/Projects/tot-internal-docs-archive` for future wiki curation.

### B. Must be verified on Linux/Tier-2 (can't on macOS)
- **Quadlet `${VAR}` non-interpolation:** confirm `EnvironmentFile`-fed `AC_*_DATABASE_INFO` reach the containers and the `$$`-escaped DB `HealthCmd` renders a literal `$` (`systemctl cat tot-database`).
- **`ac_bridge_url` default** (`http://127.0.0.1:8080` in `tokens.yaml`): confirm against the port `mod-harness-bridge` actually listens on in the worldserver.
- **Vestigial bind-mount:** confirm mod-playerbots bots spawn after the stripped `…/source/modules` mount (`obs.list_bot_population` non-zero). If zero → bake `modules/mod-playerbots/data/sql` into the worldserver image (Dockerfile follow-up — do NOT do speculatively).

### C. Code-layer follow-ups (other agents)
- **Brain bearer var inconsistency** → `agent-orchestration-architect`: `brain_sidecar/settings.py` reads `HARNESS_BEARER`/`MEMORY_BEARER`; `memory_client.py` reads `HARNESS_BEARER_TOKEN`. `.env` sets all three to the same value as a stopgap; reconcile the code.
- **Memory bind vars:** `MEM_BIND_HOST`/`MEM_BIND_PORT` aren't wired into `tot_memory/__main__.py` (hardcoded `0.0.0.0:8090`). Harmless today; wire if operator bind config is wanted.
- **AC-API correction (spec/kb):** the spec/kickoff named a `IsPlayerBot()` / `PlayerbotsMgr::GetPlayerBotsCount()` probe — neither exists in this fork. Real path: `GET_PLAYERBOT_AI(p) != nullptr` over `sRandomPlayerbotMgr.GetAllBots()` (see `playerbots_runtime_probe.md`). Correct the design doc + kb wording.
- **Consider git-LFS** for the two large stock-AC SQL files (`world.sql` ~87MB, `broadcast_text_locale.sql` ~76MB).

### D. Polish / deferred to 1.1.0–1.0.x (design §5)
- `docs/player-install.md` still has Heimdal-private framing — generalize for public operators.
- The 14 stock-AzerothCore `.github/workflows/` still trigger on `push: master` — prune/disable (low priority).
- Wiki curation from the internal-docs archive (`/Users/tbrack/Documents/Projects/tot-internal-docs-archive`).
- Deferred per design: signed releases / cosign / SBOM; auto-triggered Heimdal self-hosted runner; Grafana dashboard; `tot memory vacuum` + advanced day-2.
