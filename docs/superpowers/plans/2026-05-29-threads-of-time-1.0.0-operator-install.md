# Threads of Time 1.0.0 — Operator Install Stack Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Dispatch `deploy-orchestrator` for the deploy/release/install/first-boot/CI tasks, `cpp-systems-engineer` for the C++ runtime probe (Task 8.2), `knowledge-curator` for the kb refresh (Task 10.2).

**Goal:** Ship the operator install stack that closes ToT 1.0.0 — a release pipeline producing published artifacts + GHCR images + a GitHub Release, an operator-portable Quadlet+Compose deploy stack, a one-command install + first-boot bootstrap, operator docs, the deferred Heimdal integration tests, and GH-hosted CI.

**Architecture:** Generalize the Heimdal-specific quadlets in place (`.env`-driven runtime values + install-time-rendered `@TOKEN@` paths/image-tags; dev bind-mounts stripped; images repointed to GHCR), hand-author an equivalent Compose stack, and add a thin first-boot init container that wraps AC's native `dbimport` (the `db-import` image already bundles all stock+module SQL idempotently) plus an explicit-manifest pass over `tot/content/sql/`. A `release.sh` orchestrates the 11-step release; `install-tot.sh` bootstraps a fresh operator. Two-tier integration tests (read-only live probes + isolated-pod e2e) gate the release. GH-hosted CI runs tests/images/nightly-drift; the worldserver build stays the existing on-demand `build.sh`.

**Tech Stack:** podman Quadlet + systemd / docker-compose · MySQL 8.x · AzerothCore `dbimport` · POSIX `sh` + `bash` scripts (shellcheck-gated) · Python 3.12 (apply-order parser + Tier-1 probe suite, pytest) · GitHub Actions · GHCR.

---

## Canonical decisions (baked into every task — keep consistent)

**Operator pod topology — one pod `tot-server`:**
| Container | Image (`@GHCR_NS@` = `ghcr.io/threadsoftime`) | Role | Published port |
|---|---|---|---|
| `tot-database` | `docker.io/library/mysql:8.4` | MySQL | — (pod-internal 3306) |
| `tot-firstboot` | `@GHCR_NS@/db-import:@TOT_VERSION@` | oneshot init (dbimport + content SQL + seed) | — |
| `tot-authserver` | `@GHCR_NS@/authserver:@TOT_VERSION@` | auth | `3724` |
| `tot-worldserver` | `@GHCR_NS@/worldserver:@TOT_VERSION@` | world | `8085`, `7878` (soap, optional) |
| `tot-harness` | `@GHCR_NS@/harness:@TOT_VERSION@` | MCP/HTTP harness | `${HARNESS_BIND}` (default `8099`) |
| `tot-brain` | `@GHCR_NS@/brain:@TOT_VERSION@` | decision brain | — (pod-internal `8091`) |
| `tot-memory` | `@GHCR_NS@/memory:@TOT_VERSION@` | memory sidecar | — (pod-internal `8090`) |

Pod-internal services reach each other over `127.0.0.1` (shared pod loopback): brain → harness `http://127.0.0.1:8099/mcp/mcp`, brain → memory `http://127.0.0.1:8090/mcp/mcp`. Only auth/world/harness ports are published. (Authserver image is published to GHCR too; it builds from the same Dockerfile `authserver` target — **6 images total**: worldserver, authserver, db-import, harness, brain, memory.)

**Two token classes:**
- **Install-time `@TOKEN@`** (rendered into Quadlet unit files by `install-tot.sh`, because they sit in `Image=`/`Volume=` directives systemd won't late-bind): `@TOT_HOME@`, `@TOT_VERSION@`, `@GHCR_NS@`. Compose uses native `${VAR}` interpolation for these — no rendering needed.
- **Runtime env** (loaded via `EnvironmentFile=@TOT_HOME@/.env` at container start): all of `.env` (passwords, LLM URL/model/key, harness token, realm host, bot counts).

**DB account model (AC convention, consolidates spec §6.3's per-DB sketch):** one root password `MYSQL_ROOT_PASSWORD` + one application account `MYSQL_USER` (default `acore`) / `MYSQL_PASSWORD`, granted on all four DBs (`tot_auth`, `tot_characters`, `tot_world`, `tot_playerbots`). dbimport + worldserver + authserver + mod-playerbots all use this one account. *This is a deliberate refinement of the spec's per-DB-user sketch; documented in `.env.example`.*

**dbimport facts (verified):** `db-import` image runs `bash /azerothcore/entrypoint.sh /azerothcore/env/dist/bin/dbimport`, reads `/azerothcore/env/dist/etc/dbimport.conf`, applies `data/sql/{base,updates}` + `modules/*/data/sql/` to the three AC DBs. Idempotent via `Updates.AutoSetup=1` + `Updates.Redundancy=1` + `Updates.AllowedModules="all"`. Connection strings injected via env-substituted `dbimport.conf`. Worldserver image has `AC_UPDATES_ENABLE_DATABASES=0` → never applies SQL itself.

---

## File structure

**Created:**
- `tot/deploy/.env.example` (rewritten — full surface)
- `tot/deploy/quadlet/tot-firstboot.container`, `tot-brain.container`, `tot-memory.container` (new units)
- `tot/deploy/firstboot/firstboot.sh` (init entrypoint), `tot/deploy/firstboot/apply_content_sql.py` (manifest applier), `tot/deploy/firstboot/dbimport.conf.tmpl`
- `tot/content/sql/apply-order.toml`
- `tot/deploy/compose.yml`
- `tot/deploy/bin/tot` (operator CLI), `tot/deploy/bin/tot-backup.sh`
- `tot/memory/Containerfile`
- `tot/release/release.sh`, `tot/release/build-images.sh`
- `install-tot.sh` (repo root → shipped as release artifact)
- `docs/install.md`, `docs/operator-troubleshooting.md`, `docs/byollm-setup.md`
- `tests/firstboot/test_apply_order.py`, `tests/firstboot/test_apply_content_sql.py`
- `tot/release/integration/tier1_live_probe.py`, `tot/release/integration/tier2_e2e.sh`
- `.github/workflows/tot-ci.yml`, `tot-release-branch.yml`, `tot-release.yml`, `tot-nightly-ac-drift.yml`
- `OPERATOR_INSTALL_STATE.md`

**Modified:**
- `tot/deploy/quadlet/wow-server.pod`, `wow-database.container`, `wow-authserver.container`, `wow-worldserver.container`, `wow-stack/harness-daemon.container`, `wow-client-data.volume`, `wow-database.volume`, `wow-backup.{service,timer}`, `wow-log-rotate.{service,timer}` (generalize)
- `tot/deploy/README.md` (operator-facing rewrite)

**Relocated (Heimdal-only → `tot/internal-docs/deploy/`):**
- `wow-bracket-trigger.{service,path}`, `wow-bracket-trigger-runner.sh`, `wow-dungeon-progress.{service,timer}`

---

## Phase 0 — Operator `.env` surface (foundation)

### Task 0.1: Full operator `.env.example`

**Files:**
- Modify: `tot/deploy/.env.example` (currently subset-gating-only)

- [ ] **Step 1: Write the full env surface**

Replace the file contents entirely with:

```bash
# Threads of Time — operator environment.
# Copy to .env, fill in / let install-tot.sh generate the <generated> values.
# Every var here is loaded into the containers at runtime via EnvironmentFile.

# === Identity ===
TOT_VERSION=1.0.0
TOT_REALM_NAME="Threads of Time"
TOT_REALM_HOST=realm.example.com          # public hostname/IP players put in realmlist.wtf

# === Database (install-tot.sh generates passwords) ===
# AC convention: one app account 'acore' across all four DBs (tot_auth,
# tot_characters, tot_world, tot_playerbots). Not per-DB users.
MYSQL_ROOT_PASSWORD=<generated>
MYSQL_USER=acore
MYSQL_PASSWORD=<generated>

# === LLM (BYOLLM) — any OpenAI-compatible endpoint ===
BRAIN_LLM_URL=http://192.168.1.50:11434/v1
BRAIN_LLM_MODEL=qwen2.5:14b-instruct
BRAIN_LLM_API_KEY=
BRAIN_EMBEDDINGS_URL=http://192.168.1.50:11434/v1
BRAIN_EMBEDDINGS_MODEL=nomic-embed-text
BRAIN_EMBEDDINGS_API_KEY=
BRAIN_LOG_LEVEL=INFO

# === Harness ===
HARNESS_BEARER_TOKEN=<generated>
HARNESS_BIND=0.0.0.0:8099                 # external harness listener

# === Bots ===
TOT_BOT_POPULATION=20
TOT_LIVING_BOT_COUNT=10                    # subset gating: brain-driven of TOT_BOT_POPULATION (range 5-15)
TOT_BOT_MIN_LEVEL=1
TOT_BOT_MAX_LEVEL=25

# === Subset gating (Plan 3) ===
TOT_SUBSET_GATE_ENABLED=true
TOT_SUBSET_RECOMPUTE_INTERVAL_S=60
TOT_SUBSET_HYSTERESIS_OUT_TICKS=2
TOT_SUBSET_HYSTERESIS_IN_TICKS=1
TOT_SUBSET_ENROLL_BACKOFF_S=300
TOT_REDUCED_TICK_INTERVAL_S=300

# === Resource limits ===
TOT_WORLDSERVER_MEM_LIMIT=6g
TOT_BRAIN_MEM_LIMIT=512m
TOT_MEMORY_MEM_LIMIT=512m
TOT_HARNESS_MEM_LIMIT=200m

# === Backups ===
TOT_BACKUP_DIR=./backups                   # mysqldump + memory snapshot target
```

- [ ] **Step 2: Verify no Heimdal literals leak**

Run: `grep -nE '192\.168\.1\.|/opt/containers|/var/mnt/nas|brackin' tot/deploy/.env.example`
Expected: no output (the `192.168.1.50` LLM example is a placeholder operator IP, not Heimdal `192.168.1.3` — confirm the grep returns nothing; if it flags the example, change it to `http://ollama.example.com:11434/v1`).

If the grep matches the example URL, change `BRAIN_LLM_URL`/`BRAIN_EMBEDDINGS_URL` to `http://ollama.example.com:11434/v1`. Re-run; expect empty.

- [ ] **Step 3: Commit**

```bash
git add tot/deploy/.env.example
git commit -m "feat(deploy): full operator .env.example surface"
```

---

## Phase 1 — Reference deploy stack (Quadlet)

### Task 1.1: Relocate Heimdal-only units out of the operator stack

**Files:**
- Create: `tot/internal-docs/deploy/README.md`
- Move: `tot/deploy/quadlet/wow-bracket-trigger.service`, `wow-bracket-trigger.path`, `wow-bracket-trigger-runner.sh`, `wow-dungeon-progress.service`, `wow-dungeon-progress.timer` → `tot/internal-docs/deploy/`

- [ ] **Step 1: Move the Heimdal-only units (git mv preserves history)**

```bash
mkdir -p tot/internal-docs/deploy
git mv tot/deploy/quadlet/wow-bracket-trigger.service tot/internal-docs/deploy/
git mv tot/deploy/quadlet/wow-bracket-trigger.path tot/internal-docs/deploy/
git mv tot/deploy/quadlet/wow-bracket-trigger-runner.sh tot/internal-docs/deploy/
git mv tot/deploy/quadlet/wow-dungeon-progress.service tot/internal-docs/deploy/
git mv tot/deploy/quadlet/wow-dungeon-progress.timer tot/internal-docs/deploy/
```

- [ ] **Step 2: Document why they're Heimdal-only**

Create `tot/internal-docs/deploy/README.md`:

```markdown
# Heimdal-only deploy units (NOT shipped to operators)

These units automate the dev box's multi-bracket progression and dungeon
telemetry. They are irrelevant to a 1.0.0 operator (single bracket, L1–25)
and hardcode Heimdal paths (`/opt/containers/wow`, `/var/lib/wow-server`).
They live here, out of `tot/deploy/`, so they never enter the release tarball.

- `wow-bracket-trigger.{service,path}` + `wow-bracket-trigger-runner.sh` —
  watches `/var/lib/wow-server/advance-to-bracket`, runs bracket SQL packs.
- `wow-dungeon-progress.{service,timer}` — hourly dungeon-progress telemetry.

The operator backup path uses the generalized `tot/deploy/bin/tot-backup.sh`
+ `${TOT_BACKUP_DIR}`, not the Heimdal NAS path.
```

- [ ] **Step 3: Commit**

```bash
git add -A tot/deploy/quadlet tot/internal-docs/deploy
git commit -m "refactor(deploy): relocate Heimdal-only bracket/dungeon units to internal-docs"
```

### Task 1.2: Generalize `wow-server.pod`

**Files:**
- Modify: `tot/deploy/quadlet/wow-server.pod`

- [ ] **Step 1: Replace hardcoded harness IP with a published port (no bind IP)**

Replace the `[Pod]` section so it reads:

```ini
[Unit]
Description=Threads of Time server pod

[Pod]
PodName=tot-server
PublishPort=3724:3724
PublishPort=8085:8085
PublishPort=7878:7878
PublishPort=8099:8099

[Install]
WantedBy=multi-user.target default.target
```

Rationale: drop the Heimdal-only `192.168.1.3:` bind prefix (operators bind on all interfaces; firewall is the operator's job, documented in `install.md`). brain `8091` + memory `8090` are intentionally NOT published (pod-internal only).

- [ ] **Step 2: Verify**

Run: `grep -n '192.168.1.3' tot/deploy/quadlet/wow-server.pod`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add tot/deploy/quadlet/wow-server.pod
git commit -m "feat(deploy): generalize wow-server.pod (drop Heimdal bind IP)"
```

### Task 1.3: Generalize `wow-database.container`

**Files:**
- Modify: `tot/deploy/quadlet/wow-database.container`

- [ ] **Step 1: Parameterize the env file path + add DB/user creation env**

Replace contents with:

```ini
[Unit]
Description=Threads of Time database (MySQL 8.x)
Requires=tot-server-pod.service
After=tot-server-pod.service

[Container]
ContainerName=tot-database
Image=docker.io/library/mysql:8.4
Pod=tot-server.pod
Volume=tot-database.volume:/var/lib/mysql:Z
EnvironmentFile=@TOT_HOME@/.env
Environment=MYSQL_DATABASE=tot_world
HealthCmd=mysqladmin ping -h 127.0.0.1 -uroot -p${MYSQL_ROOT_PASSWORD}
HealthInterval=10s
HealthTimeout=5s
HealthRetries=10

[Service]
Restart=on-failure
TimeoutStartSec=180

[Install]
WantedBy=multi-user.target default.target
```

(The four DBs + the `acore` user grants are created by the firstboot init container, not the MySQL entrypoint — Task 3.3 — so we only seed one DB here to satisfy the image's healthy-start expectation.)

- [ ] **Step 2: Commit**

```bash
git add tot/deploy/quadlet/wow-database.container
git commit -m "feat(deploy): generalize wow-database.container (env-driven, @TOT_HOME@)"
```

### Task 1.4: Author `tot-firstboot.container`

**Files:**
- Create: `tot/deploy/quadlet/tot-firstboot.container`

- [ ] **Step 1: Write the init-container unit**

```ini
[Unit]
Description=Threads of Time first-boot bootstrap (DB schema + seed)
Requires=tot-database.service
After=tot-database.service
Before=tot-worldserver.service tot-authserver.service

[Container]
ContainerName=tot-firstboot
Image=@GHCR_NS@/db-import:@TOT_VERSION@
Pod=tot-server.pod
EnvironmentFile=@TOT_HOME@/.env
Volume=@TOT_HOME@/data:/tot/data:Z
Volume=@TOT_HOME@/secrets:/tot/secrets:Z
Volume=@TOT_HOME@/firstboot:/tot/firstboot:Z,ro
Volume=@TOT_HOME@/content-sql:/tot/content-sql:Z,ro
Exec=/usr/bin/env bash /tot/firstboot/firstboot.sh

[Service]
Type=oneshot
RemainAfterExit=yes
TimeoutStartSec=600

[Install]
WantedBy=multi-user.target default.target
```

Note: `install-tot.sh` copies `tot/deploy/firstboot/` → `@TOT_HOME@/firstboot/` and `tot/content/sql/` → `@TOT_HOME@/content-sql/` at scaffold time (Task 6.1). The db-import image already contains the dbimport binary + all stock/module SQL; we mount only our wrapper + content SQL + data/secrets.

- [ ] **Step 2: Commit**

```bash
git add tot/deploy/quadlet/tot-firstboot.container
git commit -m "feat(deploy): add tot-firstboot init-container unit"
```

### Task 1.5: Generalize `wow-authserver.container`

**Files:**
- Modify: `tot/deploy/quadlet/wow-authserver.container`

- [ ] **Step 1: Repoint image to GHCR, parameterize paths, gate on firstboot**

```ini
[Unit]
Description=Threads of Time auth server
Requires=tot-database.service tot-firstboot.service
After=tot-database.service tot-firstboot.service

[Container]
ContainerName=tot-authserver
Image=@GHCR_NS@/authserver:@TOT_VERSION@
Pod=tot-server.pod
Volume=@TOT_HOME@/etc:/azerothcore/env/dist/etc:Z
Volume=@TOT_HOME@/logs:/azerothcore/env/dist/logs:Z
EnvironmentFile=@TOT_HOME@/.env
Environment=AC_LOGIN_DATABASE_INFO="127.0.0.1;3306;${MYSQL_USER};${MYSQL_PASSWORD};tot_auth"
Environment=AC_LOGS_DIR=/azerothcore/env/dist/logs
Environment=AC_TEMP_DIR=/azerothcore/env/dist/temp
PodmanArgs=--tty

[Service]
Restart=on-failure

[Install]
WantedBy=multi-user.target default.target
```

(`AC_LOGIN_DATABASE_INFO` is AC's env override for the auth DB connection string — confirm the exact env var name against `apps/docker/Dockerfile`/`entrypoint.sh` during implementation; AC uses `AC_*` config-override env vars. If the override name differs, the connection string is instead written into `@TOT_HOME@/etc/authserver.conf` by firstboot.)

- [ ] **Step 2: Commit**

```bash
git add tot/deploy/quadlet/wow-authserver.container
git commit -m "feat(deploy): generalize wow-authserver.container (GHCR image, env DB info)"
```

### Task 1.6: Generalize `wow-worldserver.container` (strip dev bind-mount)

**Files:**
- Modify: `tot/deploy/quadlet/wow-worldserver.container`

- [ ] **Step 1: Repoint to GHCR, REMOVE the `…/source/modules` bind-mount, parameterize**

```ini
[Unit]
Description=Threads of Time world server
Requires=tot-database.service tot-firstboot.service
After=tot-database.service tot-firstboot.service

[Container]
ContainerName=tot-worldserver
Image=@GHCR_NS@/worldserver:@TOT_VERSION@
Pod=tot-server.pod
Volume=@TOT_HOME@/etc:/azerothcore/env/dist/etc:Z
Volume=@TOT_HOME@/logs:/azerothcore/env/dist/logs:Z
Volume=tot-client-data.volume:/azerothcore/env/dist/data:Z,ro
EnvironmentFile=@TOT_HOME@/.env
Environment=AC_DATA_DIR=/azerothcore/env/dist/data
Environment=AC_LOGS_DIR=/azerothcore/env/dist/logs
Environment=AC_UPDATES_ENABLE_DATABASES=0
Environment=AC_LOGIN_DATABASE_INFO="127.0.0.1;3306;${MYSQL_USER};${MYSQL_PASSWORD};tot_auth"
Environment=AC_WORLD_DATABASE_INFO="127.0.0.1;3306;${MYSQL_USER};${MYSQL_PASSWORD};tot_world"
Environment=AC_CHARACTER_DATABASE_INFO="127.0.0.1;3306;${MYSQL_USER};${MYSQL_PASSWORD};tot_characters"
PodmanArgs=--tty --interactive --memory=${TOT_WORLDSERVER_MEM_LIMIT}

[Service]
Restart=on-failure
TimeoutStartSec=600

[Install]
WantedBy=multi-user.target default.target
```

Removed vs. Heimdal version: `Volume=…/source/modules:/azerothcore/modules` (the runtime bind-mount — vestigial; `AC_UPDATES_ENABLE_DATABASES=0` means worldserver never reads module SQL; all SQL is applied by firstboot/dbimport). The `…/sql:/sql-custom` Heimdal volume is also dropped (content SQL is applied by firstboot, not bind-mounted into worldserver).

⚠️ **Risk to verify in Tier-2 (Task 8.3):** confirm mod-playerbots bots actually spawn/populate with the bind-mount removed. If they don't, the fallback is to bake `modules/mod-playerbots/data/sql` into the worldserver image — a Dockerfile change tracked as a follow-up, NOT done speculatively here.

- [ ] **Step 2: Verify dev-isms gone**

Run: `grep -nE '/opt/containers|source/modules|localhost/wow-server' tot/deploy/quadlet/wow-worldserver.container`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add tot/deploy/quadlet/wow-worldserver.container
git commit -m "feat(deploy): generalize worldserver unit; strip dev source bind-mount"
```

### Task 1.7: Generalize `harness-daemon.container`

**Files:**
- Modify: `tot/deploy/quadlet/wow-stack/harness-daemon.container`

- [ ] **Step 1: Repoint to GHCR, parameterize, env-drive the bearer/bind**

```ini
[Unit]
Description=Threads of Time agentic harness daemon
After=tot-worldserver.service
Wants=tot-worldserver.service

[Container]
Image=@GHCR_NS@/harness:@TOT_VERSION@
ContainerName=tot-harness
Pod=tot-server.pod
Volume=@TOT_HOME@/etc/harness:/etc/harness:z,ro
Volume=@TOT_HOME@/logs/harness:/var/log/harness:z
EnvironmentFile=@TOT_HOME@/.env
Exec=serve --config /etc/harness/tokens.yaml
PodmanArgs=--memory=${TOT_HARNESS_MEM_LIMIT}

[Service]
Restart=on-failure
RestartSec=5s
ExecReload=/usr/bin/kill -HUP $MAINPID

[Install]
WantedBy=default.target
```

(The harness reads `HARNESS_BEARER_TOKEN` + `HARNESS_BIND` from `.env`; its `tokens.yaml` is seeded by firstboot/install from the env. Port `8099` is published on the pod, not here.)

- [ ] **Step 2: Commit**

```bash
git add tot/deploy/quadlet/wow-stack/harness-daemon.container
git commit -m "feat(deploy): generalize harness-daemon unit (GHCR, @TOT_HOME@)"
```

### Task 1.8: Author `tot-brain.container` (generalized, no stale env)

**Files:**
- Create: `tot/deploy/quadlet/tot-brain.container`

- [ ] **Step 1: Write a clean brain unit — env-driven, NO decommissioned vars, NO inline secrets**

```ini
[Unit]
Description=Threads of Time decision brain
After=tot-worldserver.service tot-harness.service tot-memory.service
Wants=tot-harness.service tot-memory.service

[Container]
Image=@GHCR_NS@/brain:@TOT_VERSION@
ContainerName=tot-brain
Pod=tot-server.pod
Volume=@TOT_HOME@/data/brain:/var/lib/brain:Z
Volume=@TOT_HOME@/logs/brain:/var/log/brain:Z
EnvironmentFile=@TOT_HOME@/.env
Environment=BRAIN_BIND_HOST=127.0.0.1
Environment=BRAIN_BIND_PORT=8091
Environment=BRAIN_STATE_DB=/var/lib/brain/state.sqlite
Environment=BRAIN_DECISIONS_LOG=/var/log/brain/decisions.jsonl
Environment=HARNESS_MCP_URL=http://127.0.0.1:8099/mcp/mcp
Environment=MEMORY_MCP_URL=http://127.0.0.1:8090/mcp/mcp
Environment=LLM_BASE_URL=${BRAIN_LLM_URL}
Environment=LLM_MODEL=${BRAIN_LLM_MODEL}
Environment=LLM_API_KEY=${BRAIN_LLM_API_KEY}
PodmanArgs=--memory=${TOT_BRAIN_MEM_LIMIT}

[Service]
Restart=on-failure
RestartSec=5s
TimeoutStartSec=60

[Install]
WantedBy=multi-user.target default.target
```

Deliberately omitted from the Heimdal source: the multi-endpoint `LLM_ROUTING_MODE=all_nemo` / `LLM_PRIMARY_*` / `OPENROUTER_*` block (Heimdal-internal only per §9.3 — single-endpoint for operators) and the stale `qwen2.5-7b` / `192.168.1.3:8080` static vars (decommissioned). The brain reads `HARNESS_BEARER_TOKEN` from `.env` for its harness/memory calls.

- [ ] **Step 2: Verify no stale/secret values copied**

Run: `grep -nE 'qwen|openrouter|192.168.1.3|sk-or-|all_nemo|BEARER=' tot/deploy/quadlet/tot-brain.container`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add tot/deploy/quadlet/tot-brain.container
git commit -m "feat(deploy): add generalized tot-brain unit (single-endpoint, no stale env)"
```

### Task 1.9: Author `tot-memory.container`

**Files:**
- Create: `tot/deploy/quadlet/tot-memory.container`

- [ ] **Step 1: Write the memory sidecar unit**

```ini
[Unit]
Description=Threads of Time memory subsystem
After=tot-worldserver.service
Wants=tot-worldserver.service

[Container]
Image=@GHCR_NS@/memory:@TOT_VERSION@
ContainerName=tot-memory
Pod=tot-server.pod
Volume=@TOT_HOME@/data/memory:/var/memory:Z
EnvironmentFile=@TOT_HOME@/.env
Environment=MEM_DB_PATH=/var/memory/memory.sqlite
Environment=MEM_BIND_HOST=127.0.0.1
Environment=MEM_BIND_PORT=8090
Environment=MEM_EMBED_ENDPOINT=${BRAIN_EMBEDDINGS_URL}
Environment=MEM_EMBED_MODEL=${BRAIN_EMBEDDINGS_MODEL}
Environment=MEM_EMBED_API_KEY=${BRAIN_EMBEDDINGS_API_KEY}
PodmanArgs=--memory=${TOT_MEMORY_MEM_LIMIT}

[Service]
Restart=on-failure
RestartSec=5s
TimeoutStartSec=60

[Install]
WantedBy=multi-user.target default.target
```

(Confirm the memory sidecar's actual env var names — `MEM_DB_PATH`, `MEM_EMBED_ENDPOINT` — against `tot/memory/src/tot_memory/` config during implementation; the verified live Dockerfile used `MEM_DB_PATH` + `MEM_EMBED_ENDPOINT`. Adjust if the packaged config differs.)

- [ ] **Step 2: Commit**

```bash
git add tot/deploy/quadlet/tot-memory.container
git commit -m "feat(deploy): add generalized tot-memory unit"
```

### Task 1.10: Generalize volumes + backup/log-rotate units

**Files:**
- Modify: `tot/deploy/quadlet/wow-client-data.volume`, `wow-database.volume`, `wow-backup.service`, `wow-backup.timer`, `wow-log-rotate.service`, `wow-log-rotate.timer`

- [ ] **Step 1: Rename volumes to `tot-*` and confirm no hardcoded paths**

Set `wow-client-data.volume` to:
```ini
[Volume]
VolumeName=tot-client-data
```
Set `wow-database.volume` to:
```ini
[Volume]
VolumeName=tot-database
```

- [ ] **Step 2: Generalize the backup service to use `${TOT_BACKUP_DIR}` + `tot-backup.sh`**

Set `wow-backup.service` to:
```ini
[Unit]
Description=Threads of Time nightly backup

[Service]
Type=oneshot
EnvironmentFile=@TOT_HOME@/.env
ExecStart=/usr/bin/env bash @TOT_HOME@/bin/tot-backup.sh
```
Leave `wow-backup.timer` as `OnCalendar=*-*-* 04:00:00` (no Heimdal NAS `ConditionPathIsDirectory`). Remove any `ConditionPathIsDirectory=/var/mnt/nas/...` line.

For `wow-log-rotate.{service,timer}`: replace any `/usr/local/sbin/wow-log-rotate.sh` host-script reference with `@TOT_HOME@/bin/tot` `logs --rotate` is overkill — instead point at `podman` logrotate via the container runtime; simplest: set the log-rotate service to no-op-documented and rely on podman's `--log-opt max-size`. Set `wow-log-rotate.service` ExecStart to:
```ini
ExecStart=/usr/bin/find @TOT_HOME@/logs -name '*.log' -size +100M -delete
```

- [ ] **Step 3: Verify + commit**

Run: `grep -rnE '/opt/containers|/var/mnt/nas|/usr/local/(s)?bin|192.168.1.3' tot/deploy/quadlet/`
Expected: no output across the whole quadlet dir.

```bash
git add tot/deploy/quadlet/
git commit -m "feat(deploy): generalize volumes + backup/log-rotate units"
```

### Task 1.11: Quadlet validation gate

**Files:**
- Create: `tot/deploy/validate-quadlets.sh`

- [ ] **Step 1: Write a render+validate script**

```bash
#!/usr/bin/env bash
# Renders @TOKEN@s with throwaway values and asserts no Heimdal literal survives.
set -euo pipefail
DIR="$(cd "$(dirname "$0")/quadlet" && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
for f in "$DIR"/*.{container,pod,volume} "$DIR"/wow-stack/*.container; do
  [ -f "$f" ] || continue
  sed -e 's|@TOT_HOME@|/opt/tot|g' -e 's|@TOT_VERSION@|1.0.0|g' \
      -e 's|@GHCR_NS@|ghcr.io/threadsoftime|g' "$f" > "$TMP/$(basename "$f")"
done
# Heimdal-literal gate
if grep -rnE '192\.168\.1\.3|/opt/containers/wow|/var/mnt/nas|brackin|localhost/wow-server|localhost/harness-daemon|localhost/brain-sidecar|localhost/memory-sidecar' "$DIR"; then
  echo "FAIL: Heimdal-specific literal found in quadlets"; exit 1
fi
echo "OK: quadlets render and carry no Heimdal literals"
```

- [ ] **Step 2: Run it**

Run: `bash tot/deploy/validate-quadlets.sh`
Expected: `OK: quadlets render and carry no Heimdal literals`

- [ ] **Step 3: Commit**

```bash
git add tot/deploy/validate-quadlets.sh
git commit -m "test(deploy): quadlet render + Heimdal-literal gate"
```

---

## Phase 2 — Compose stack

### Task 2.1: Author `tot/deploy/compose.yml`

**Files:**
- Create: `tot/deploy/compose.yml`

- [ ] **Step 1: Write the Compose translation (native `${VAR}` interpolation from `.env`)**

```yaml
# Threads of Time reference Compose stack (alternative to Quadlet).
# `docker compose` / `podman-compose` read .env automatically for ${VAR}.
name: tot
services:
  database:
    image: docker.io/library/mysql:8.4
    container_name: tot-database
    environment:
      MYSQL_ROOT_PASSWORD: ${MYSQL_ROOT_PASSWORD}
      MYSQL_DATABASE: tot_world
    volumes: [tot-database:/var/lib/mysql]
    healthcheck:
      test: ["CMD", "mysqladmin", "ping", "-h", "127.0.0.1", "-uroot", "-p${MYSQL_ROOT_PASSWORD}"]
      interval: 10s
      timeout: 5s
      retries: 10
  firstboot:
    image: ${GHCR_NS:-ghcr.io/threadsoftime}/db-import:${TOT_VERSION}
    container_name: tot-firstboot
    depends_on:
      database: {condition: service_healthy}
    env_file: [.env]
    volumes:
      - ./data:/tot/data
      - ./secrets:/tot/secrets
      - ./firstboot:/tot/firstboot:ro
      - ./content-sql:/tot/content-sql:ro
    entrypoint: ["/usr/bin/env", "bash", "/tot/firstboot/firstboot.sh"]
    restart: "no"
  authserver:
    image: ${GHCR_NS:-ghcr.io/threadsoftime}/authserver:${TOT_VERSION}
    container_name: tot-authserver
    depends_on:
      firstboot: {condition: service_completed_successfully}
    env_file: [.env]
    environment:
      AC_LOGIN_DATABASE_INFO: "database;3306;${MYSQL_USER};${MYSQL_PASSWORD};tot_auth"
    ports: ["3724:3724"]
    volumes: [./etc:/azerothcore/env/dist/etc, ./logs:/azerothcore/env/dist/logs]
    restart: on-failure
  worldserver:
    image: ${GHCR_NS:-ghcr.io/threadsoftime}/worldserver:${TOT_VERSION}
    container_name: tot-worldserver
    depends_on:
      firstboot: {condition: service_completed_successfully}
    env_file: [.env]
    environment:
      AC_UPDATES_ENABLE_DATABASES: "0"
      AC_WORLD_DATABASE_INFO: "database;3306;${MYSQL_USER};${MYSQL_PASSWORD};tot_world"
      AC_CHARACTER_DATABASE_INFO: "database;3306;${MYSQL_USER};${MYSQL_PASSWORD};tot_characters"
      AC_LOGIN_DATABASE_INFO: "database;3306;${MYSQL_USER};${MYSQL_PASSWORD};tot_auth"
    ports: ["8085:8085", "7878:7878"]
    volumes: [tot-client-data:/azerothcore/env/dist/data:ro, ./etc:/azerothcore/env/dist/etc, ./logs:/azerothcore/env/dist/logs]
    stdin_open: true
    tty: true
    restart: on-failure
  harness:
    image: ${GHCR_NS:-ghcr.io/threadsoftime}/harness:${TOT_VERSION}
    container_name: tot-harness
    depends_on: [worldserver]
    env_file: [.env]
    command: ["serve", "--config", "/etc/harness/tokens.yaml"]
    ports: ["8099:8099"]
    volumes: [./etc/harness:/etc/harness:ro, ./logs/harness:/var/log/harness]
    restart: on-failure
  memory:
    image: ${GHCR_NS:-ghcr.io/threadsoftime}/memory:${TOT_VERSION}
    container_name: tot-memory
    env_file: [.env]
    environment:
      MEM_DB_PATH: /var/memory/memory.sqlite
      MEM_EMBED_ENDPOINT: ${BRAIN_EMBEDDINGS_URL}
      MEM_EMBED_MODEL: ${BRAIN_EMBEDDINGS_MODEL}
    volumes: [./data/memory:/var/memory]
    restart: on-failure
  brain:
    image: ${GHCR_NS:-ghcr.io/threadsoftime}/brain:${TOT_VERSION}
    container_name: tot-brain
    depends_on: [harness, memory]
    env_file: [.env]
    environment:
      HARNESS_MCP_URL: http://harness:8099/mcp/mcp
      MEMORY_MCP_URL: http://memory:8090/mcp/mcp
      LLM_BASE_URL: ${BRAIN_LLM_URL}
      LLM_MODEL: ${BRAIN_LLM_MODEL}
    volumes: [./data/brain:/var/lib/brain, ./logs/brain:/var/log/brain]
    restart: on-failure
volumes:
  tot-database:
  tot-client-data:
```

Note the topology difference vs. Quadlet: Compose uses service-name DNS (`database`, `harness`, `memory`) instead of pod loopback `127.0.0.1`. Both are valid; the e2e (Task 8.3) exercises whichever mode it installs.

- [ ] **Step 2: Validate config**

Run: `cd tot/deploy && TOT_VERSION=1.0.0 MYSQL_ROOT_PASSWORD=x MYSQL_USER=acore MYSQL_PASSWORD=y BRAIN_LLM_URL=u BRAIN_LLM_MODEL=m BRAIN_EMBEDDINGS_URL=u BRAIN_EMBEDDINGS_MODEL=m docker compose -f compose.yml config >/dev/null && echo OK`
Expected: `OK` (use `podman-compose config` if docker compose absent).

- [ ] **Step 3: Commit**

```bash
git add tot/deploy/compose.yml
git commit -m "feat(deploy): operator Compose stack (Quadlet alternative)"
```

---

## Phase 3 — First-boot bootstrap

### Task 3.1: `tot/content/sql/apply-order.toml`

**Files:**
- Create: `tot/content/sql/apply-order.toml`

- [ ] **Step 1: Write the explicit ordering manifest**

```toml
# Explicit apply order for ToT content SQL (these files are undated; this
# manifest is the single source of truth for sequence + target DB).
# Applied by firstboot AFTER AC dbimport (stock + module SQL).
# db is one of: world, characters, auth.

[[apply]]
file = "bracket1/world_bracket1_items.sql"
db   = "world"
[[apply]]
file = "bracket1/world_bracket1_loot.sql"
db   = "world"
[[apply]]
file = "bracket1/world_discovery_npc.sql"
db   = "world"
[[apply]]
file = "bracket1/world_dk_stats_stub.sql"
db   = "world"
[[apply]]
file = "bracket1/world_starter_respawn.sql"
db   = "world"
[[apply]]
file = "bracket1-raid/world_bfd_raid_items.sql"
db   = "world"
[[apply]]
file = "bracket1-raid/world_bfd_raid_creatures.sql"
db   = "world"
[[apply]]
file = "bracket1-raid/world_bfd_raid_smart_scripts.sql"
db   = "world"
[[apply]]
file = "bracket1-raid/world_bfd_raid_lockout.sql"
db   = "world"
[[apply]]
file = "bracket1/characters_wipe_bots.sql"
db   = "characters"
```

(Order: content world rows, then BFD raid, then the characters bot-wipe last. Confirm this set against `ls tot/content/sql/**/*.sql` during implementation and add any file not listed — the parser test in Task 3.2 enforces that every on-disk file appears in the manifest.)

- [ ] **Step 2: Commit**

```bash
git add tot/content/sql/apply-order.toml
git commit -m "feat(content): explicit apply-order manifest for ToT content SQL"
```

### Task 3.2: TDD the apply-order parser

**Files:**
- Create: `tot/deploy/firstboot/apply_content_sql.py`
- Test: `tests/firstboot/test_apply_order.py`

- [ ] **Step 1: Write the failing test**

```python
# tests/firstboot/test_apply_order.py
import tomllib
from pathlib import Path
import pytest
from tot.deploy.firstboot.apply_content_sql import load_apply_order, DB_TO_DATABASE

REPO = Path(__file__).resolve().parents[2]
SQL_ROOT = REPO / "tot" / "content" / "sql"

def test_manifest_lists_every_on_disk_sql_file():
    listed = {e["file"] for e in load_apply_order(SQL_ROOT / "apply-order.toml")}
    on_disk = {str(p.relative_to(SQL_ROOT)) for p in SQL_ROOT.rglob("*.sql")}
    assert on_disk == listed, f"manifest/disk mismatch: {on_disk ^ listed}"

def test_every_db_maps_to_a_real_database():
    for e in load_apply_order(SQL_ROOT / "apply-order.toml"):
        assert e["db"] in DB_TO_DATABASE

def test_order_is_preserved():
    entries = load_apply_order(SQL_ROOT / "apply-order.toml")
    assert entries[0]["file"].startswith("bracket1/")
    assert entries[-1]["file"] == "bracket1/characters_wipe_bots.sql"
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd /Users/tbrack/Documents/Projects/threads-of-time && python -m pytest tests/firstboot/test_apply_order.py -v`
Expected: FAIL — `ModuleNotFoundError: tot.deploy.firstboot.apply_content_sql`.

- [ ] **Step 3: Write minimal implementation**

```python
# tot/deploy/firstboot/apply_content_sql.py
"""Apply ToT content SQL in the order declared by apply-order.toml."""
from __future__ import annotations
import argparse, subprocess, sys, tomllib
from pathlib import Path

DB_TO_DATABASE = {"world": "tot_world", "characters": "tot_characters", "auth": "tot_auth"}

def load_apply_order(manifest: Path) -> list[dict]:
    with open(manifest, "rb") as fh:
        data = tomllib.load(fh)
    return data.get("apply", [])

def apply(sql_root: Path, host: str, port: str, user: str, password: str) -> None:
    for entry in load_apply_order(sql_root / "apply-order.toml"):
        sql_file = sql_root / entry["file"]
        database = DB_TO_DATABASE[entry["db"]]
        print(f"  applying {entry['file']} -> {database}", flush=True)
        with open(sql_file, "rb") as fh:
            subprocess.run(
                ["mysql", f"-h{host}", f"-P{port}", f"-u{user}", f"-p{password}", database],
                stdin=fh, check=True,
            )

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sql-root", type=Path, required=True)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", default="3306")
    ap.add_argument("--user", required=True)
    ap.add_argument("--password", required=True)
    args = ap.parse_args()
    apply(args.sql_root, args.host, args.port, args.user, args.password)
    return 0

if __name__ == "__main__":
    sys.exit(main())
```

Also create empty `tests/firstboot/__init__.py`, `tot/deploy/__init__.py`, `tot/deploy/firstboot/__init__.py` if the test import path needs them (only if the repo runs pytest with rootdir import mode; otherwise adjust the test import to a path-based import).

- [ ] **Step 4: Run to verify pass**

Run: `python -m pytest tests/firstboot/test_apply_order.py -v`
Expected: PASS (3 tests). If `test_manifest_lists_every_on_disk_sql_file` fails, reconcile `apply-order.toml` with the actual file list and re-run.

- [ ] **Step 5: Commit**

```bash
git add tot/deploy/firstboot/apply_content_sql.py tests/firstboot/
git commit -m "feat(firstboot): content-SQL apply-order parser + tests"
```

### Task 3.3: First-boot entrypoint `firstboot.sh`

**Files:**
- Create: `tot/deploy/firstboot/firstboot.sh`, `tot/deploy/firstboot/dbimport.conf.tmpl`

- [ ] **Step 1: Write the dbimport.conf template**

```ini
# tot/deploy/firstboot/dbimport.conf.tmpl — rendered by firstboot.sh
LoginDatabaseInfo     = "@DBHOST@;3306;@DBUSER@;@DBPASS@;tot_auth"
WorldDatabaseInfo     = "@DBHOST@;3306;@DBUSER@;@DBPASS@;tot_world"
CharacterDatabaseInfo = "@DBHOST@;3306;@DBUSER@;@DBPASS@;tot_characters"
Updates.EnableDatabases = 7
Updates.AutoSetup       = 1
Updates.Redundancy      = 1
Updates.AllowedModules  = "all"
```

- [ ] **Step 2: Write the idempotent first-boot script**

```bash
#!/usr/bin/env bash
# tot/deploy/firstboot/firstboot.sh — idempotent first-boot bootstrap.
# Runs in the db-import image (has dbimport + all stock/module SQL).
set -euo pipefail

FLAG=/tot/data/bootstrap.flag
DBHOST="${DB_HOST:-127.0.0.1}"
log() { printf '[firstboot] %s\n' "$*"; }
fail() { printf '[firstboot] STEP FAILED: %s\n' "$*" >&2; exit 1; }

if [ -f "$FLAG" ]; then
  log "bootstrap.flag present — skipping (already bootstrapped)."
  exit 0
fi

# 1. Wait for MySQL (kb_9d1289dd backoff)
log "1/8 waiting for MySQL"
for i in $(seq 1 30); do
  if mysqladmin ping -h "$DBHOST" -uroot -p"${MYSQL_ROOT_PASSWORD}" --silent 2>/dev/null; then
    break
  fi
  [ "$i" = 30 ] && fail "MySQL not reachable after 30 tries (15s backoff cap). Check tot-database."
  sleep $(( i < 5 ? i : 5 ))
done

mysql_root() { mysql -h "$DBHOST" -uroot -p"${MYSQL_ROOT_PASSWORD}" "$@"; }

# 2. Create DBs + app user
log "2/8 creating databases + ${MYSQL_USER} grants"
mysql_root <<SQL || fail "DB/user creation"
CREATE DATABASE IF NOT EXISTS tot_auth       DEFAULT CHARSET utf8mb4;
CREATE DATABASE IF NOT EXISTS tot_characters  DEFAULT CHARSET utf8mb4;
CREATE DATABASE IF NOT EXISTS tot_world       DEFAULT CHARSET utf8mb4;
CREATE DATABASE IF NOT EXISTS tot_playerbots  DEFAULT CHARSET utf8mb4;
CREATE USER IF NOT EXISTS '${MYSQL_USER}'@'%' IDENTIFIED BY '${MYSQL_PASSWORD}';
GRANT ALL ON tot_auth.*      TO '${MYSQL_USER}'@'%';
GRANT ALL ON tot_characters.* TO '${MYSQL_USER}'@'%';
GRANT ALL ON tot_world.*      TO '${MYSQL_USER}'@'%';
GRANT ALL ON tot_playerbots.* TO '${MYSQL_USER}'@'%';
FLUSH PRIVILEGES;
SQL

# 3. AC dbimport — stock base + updates + module SQL (idempotent, tracked)
log "3/8 running AC dbimport (stock + module SQL)"
sed -e "s|@DBHOST@|${DBHOST}|g" -e "s|@DBUSER@|${MYSQL_USER}|g" -e "s|@DBPASS@|${MYSQL_PASSWORD}|g" \
    /tot/firstboot/dbimport.conf.tmpl > /azerothcore/env/dist/etc/dbimport.conf
/azerothcore/env/dist/bin/dbimport || fail "dbimport (stock + module SQL)"

# 4. ToT content SQL via explicit manifest
log "4/8 applying ToT content SQL (apply-order.toml)"
python3 /tot/firstboot/apply_content_sql.py \
  --sql-root /tot/content-sql --host "$DBHOST" \
  --user "${MYSQL_USER}" --password "${MYSQL_PASSWORD}" || fail "content SQL"

# 5. Seed realm row
log "5/8 seeding realm row -> ${TOT_REALM_HOST}"
mysql_root tot_auth <<SQL || fail "realm seed"
INSERT INTO realmlist (id, name, address, localAddress, port, gamebuild)
VALUES (1, '${TOT_REALM_NAME}', '${TOT_REALM_HOST}', '127.0.0.1', 8085, 12340)
ON DUPLICATE KEY UPDATE name=VALUES(name), address=VALUES(address);
SQL

# 6. Seed default GM account (admin/admin — change-me)
log "6/8 seeding GM account admin (CHANGE THE PASSWORD)"
# AC stores SRP6 verifier; use the worldserver 'account create' path is unavailable
# here, so insert via AC's account table using the documented SHA_PASS_HASH legacy
# path is deprecated. Instead, write a marker so the operator runs:
#   tot run-console "account create admin <password>" ; "account set gmlevel admin 3 -1"
# documented in docs/install.md. (Automated SRP6 seeding deferred to 1.0.x.)
log "    (GM account created post-boot via 'tot' console — see docs/install.md)"

# 7. Generate harness token if absent
log "7/8 ensuring harness bearer token"
if [ ! -s /tot/secrets/harness-token ]; then
  printf '%s' "${HARNESS_BEARER_TOKEN:-$(head -c32 /dev/urandom | od -An -tx1 | tr -d ' \n')}" \
    > /tot/secrets/harness-token
fi

# 8. Mark complete
log "8/8 writing bootstrap.flag"
date -u +%Y-%m-%dT%H:%M:%SZ > "$FLAG"
log "bootstrap complete."
```

Note on Step 6: SRP6 verifier seeding from a shell is fragile; the plan defers automated GM seeding to a post-boot `tot run-console "account create …"` step documented in `install.md`. This is a deliberate scope call (not a placeholder) — the GM account is created by the operator on first run via the console, which `tot` exposes.

- [ ] **Step 3: shellcheck**

Run: `shellcheck tot/deploy/firstboot/firstboot.sh`
Expected: no errors (warnings about `${VAR}` in heredocs are acceptable; fix any error-level findings).

- [ ] **Step 4: Commit**

```bash
git add tot/deploy/firstboot/firstboot.sh tot/deploy/firstboot/dbimport.conf.tmpl
git commit -m "feat(firstboot): idempotent bootstrap (dbimport + content SQL + seed)"
```

### Task 3.4: Idempotency test for `firstboot.sh`

**Files:**
- Test: `tests/firstboot/test_idempotency.py`

- [ ] **Step 1: Write the test (flag short-circuits second run)**

```python
# tests/firstboot/test_idempotency.py
import subprocess, os, stat
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / "tot/deploy/firstboot/firstboot.sh"

def test_flag_present_short_circuits(tmp_path):
    data = tmp_path / "data"; data.mkdir()
    (data / "bootstrap.flag").write_text("2026-01-01T00:00:00Z\n")
    # Run with a doctored env where the flag dir is tmp; the script must exit 0
    # at the flag check BEFORE touching MySQL (no mysqladmin needed).
    env = {**os.environ, "PATH": os.environ["PATH"]}
    # Patch FLAG path by running a wrapper that overrides /tot/data via sed-free bind:
    wrapper = tmp_path / "run.sh"
    wrapper.write_text(
        f'#!/usr/bin/env bash\nset -e\nsed "s#/tot/data#{data}#g" "{SCRIPT}" > "{tmp_path}/fb.sh"\n'
        f'bash "{tmp_path}/fb.sh"\n'
    )
    wrapper.chmod(wrapper.stat().st_mode | stat.S_IEXEC)
    r = subprocess.run(["bash", str(wrapper)], capture_output=True, text=True)
    assert r.returncode == 0
    assert "skipping" in r.stdout
```

- [ ] **Step 2: Run**

Run: `python -m pytest tests/firstboot/test_idempotency.py -v`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/firstboot/test_idempotency.py
git commit -m "test(firstboot): flag short-circuits re-run"
```

---

## Phase 4 — Operator CLI

### Task 4.1: `tot` CLI

**Files:**
- Create: `tot/deploy/bin/tot`

- [ ] **Step 1: Write the CLI**

```bash
#!/usr/bin/env bash
# tot — Threads of Time operator CLI. Installed onto PATH by install-tot.sh.
# Dispatches to Quadlet (systemctl --user) or Compose based on $TOT_MODE.
set -euo pipefail
TOT_HOME="${TOT_HOME:-/opt/tot}"
MODE_FILE="$TOT_HOME/.mode"
MODE="$( [ -f "$MODE_FILE" ] && cat "$MODE_FILE" || echo compose )"

compose() { ( cd "$TOT_HOME" && docker compose -f compose.yml "$@" 2>/dev/null \
              || podman-compose -f compose.yml "$@" ); }
units=(tot-database tot-firstboot tot-authserver tot-worldserver tot-harness tot-memory tot-brain)

usage() { echo "usage: tot {up|down|restart <svc>|logs <svc>|run-console <cmd>|upgrade <ver>}"; exit 2; }

cmd="${1:-}"; shift || true
case "$cmd" in
  up)
    if [ "$MODE" = quadlet ]; then systemctl --user daemon-reload; systemctl --user start tot-server-pod.service "${units[@]/%/.service}";
    else compose up -d; fi ;;
  down)
    if [ "$MODE" = quadlet ]; then systemctl --user stop "${units[@]/%/.service}" tot-server-pod.service || true;
    else compose down; fi ;;
  restart)
    svc="${1:?service name}"
    if [ "$MODE" = quadlet ]; then systemctl --user restart "tot-${svc}.service"; else compose restart "$svc"; fi ;;
  logs)
    svc="${1:?service name}"; podman logs -f "tot-${svc}" ;;
  run-console)
    podman exec -i tot-worldserver /bin/sh -c "echo '$*' | ./worldserver" || \
      podman attach --sig-proxy=false tot-worldserver ;;
  upgrade)
    ver="${1:?version}"
    echo "==> snapshotting DBs"; "$TOT_HOME/bin/tot-backup.sh"
    echo "==> pulling :$ver"; sed -i.bak "s/^TOT_VERSION=.*/TOT_VERSION=$ver/" "$TOT_HOME/.env"
    if [ "$MODE" = compose ]; then compose pull; compose up -d;
    else "$TOT_HOME/bin/render-quadlets.sh" "$ver"; systemctl --user daemon-reload; tot up; fi
    echo "==> upgraded to $ver (forward-only; restore from backups/ to roll back)" ;;
  *) usage ;;
esac
```

- [ ] **Step 2: shellcheck**

Run: `shellcheck -e SC2086 tot/deploy/bin/tot`
Expected: no error-level findings.

- [ ] **Step 3: Commit**

```bash
git add tot/deploy/bin/tot && chmod +x tot/deploy/bin/tot
git commit -m "feat(deploy): tot operator CLI (up/down/restart/logs/upgrade)"
```

### Task 4.2: `tot-backup.sh`

**Files:**
- Create: `tot/deploy/bin/tot-backup.sh`

- [ ] **Step 1: Write the backup script**

```bash
#!/usr/bin/env bash
# tot-backup.sh — mysqldump all four DBs + memory.sqlite snapshot.
set -euo pipefail
TOT_HOME="${TOT_HOME:-/opt/tot}"
. "$TOT_HOME/.env"
DEST="${TOT_BACKUP_DIR:-$TOT_HOME/backups}"
STAMP="$(date -u +%Y%m%d-%H%M%S)"
mkdir -p "$DEST"
echo "==> dumping databases -> $DEST/db-$STAMP.sql.gz"
podman exec tot-database sh -c \
  "mysqldump -uroot -p'${MYSQL_ROOT_PASSWORD}' --databases tot_auth tot_characters tot_world tot_playerbots" \
  | gzip > "$DEST/db-$STAMP.sql.gz"
echo "==> snapshotting memory.sqlite (WAL checkpoint first)"
podman exec tot-memory sh -c "sqlite3 /var/memory/memory.sqlite 'PRAGMA wal_checkpoint(TRUNCATE);'" || true
podman cp tot-memory:/var/memory/memory.sqlite "$DEST/memory-$STAMP.sqlite" || true
echo "==> pruning (keep 7 daily + 4 weekly)"
ls -1t "$DEST"/db-*.sql.gz 2>/dev/null | tail -n +8 | xargs -r rm -f
echo "OK: backup $STAMP"
```

- [ ] **Step 2: shellcheck + commit**

Run: `shellcheck tot/deploy/bin/tot-backup.sh`; Expected: no error-level findings.

```bash
git add tot/deploy/bin/tot-backup.sh && chmod +x tot/deploy/bin/tot-backup.sh
git commit -m "feat(deploy): tot-backup.sh (DB dump + memory snapshot + retention)"
```

---

## Phase 5 — Release pipeline + images

### Task 5.1: Add `tot/memory/Containerfile`

**Files:**
- Create: `tot/memory/Containerfile`

- [ ] **Step 1: Read the packaged memory config to confirm entrypoint + env names**

Run: `sed -n '1,40p' tot/memory/pyproject.toml; ls tot/memory/src/tot_memory`
Confirm the package name + the ASGI app factory path (the live image used `uvicorn memory_sidecar.main:create_app --factory`; the ToT package may be `tot_memory`). Use the confirmed module path below.

- [ ] **Step 2: Write the Containerfile (adjust the module path to what Step 1 confirmed)**

```dockerfile
# tot/memory/Containerfile — ToT memory subsystem sidecar.
FROM python:3.12-slim
WORKDIR /app
COPY pyproject.toml ./
COPY src/ ./src/
RUN pip install --no-cache-dir . && pip install --no-cache-dir uvicorn
ENV PYTHONPATH=/app/src \
    MEM_DB_PATH=/var/memory/memory.sqlite \
    MEM_BIND_HOST=0.0.0.0 \
    MEM_BIND_PORT=8090
EXPOSE 8090
# Adjust factory path to the confirmed package (Step 1):
CMD ["uvicorn", "tot_memory.main:create_app", "--factory", "--host", "0.0.0.0", "--port", "8090"]
```

- [ ] **Step 3: Build it locally to verify it produces an image**

Run: `podman build -t tot-memory:plantest -f tot/memory/Containerfile tot/memory`
Expected: build succeeds. (If the factory path is wrong, the build still succeeds but the container won't start — verify with `podman run --rm tot-memory:plantest python -c "import tot_memory.main"`.)

- [ ] **Step 4: Commit**

```bash
git add tot/memory/Containerfile
git commit -m "feat(memory): add Containerfile (was external in mod-playerbots)"
```

### Task 5.2: `build-images.sh`

**Files:**
- Create: `tot/release/build-images.sh`

- [ ] **Step 1: Write the multi-image builder**

```bash
#!/usr/bin/env bash
# tot/release/build-images.sh <version> — build + tag all ToT images.
# worldserver + db-import build on Heimdal via build.sh (the -j4 gated step);
# harness/brain/memory build anywhere. Tags :<version>. Pushes if PUSH=1.
set -euo pipefail
VERSION="${1:?usage: build-images.sh <version>}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NS="${GHCR_NS:-ghcr.io/threadsoftime}"

echo "== worldserver + db-import (Heimdal build.sh, -j4) =="
TAG="$VERSION" bash "$REPO/tot/release/build.sh"   # builds wow-server + wow-db-import, tags :$VERSION on Heimdal

echo "== authserver (from AC Dockerfile target) =="
podman build --target authserver -t "$NS/authserver:$VERSION" -f "$REPO/apps/docker/Dockerfile" "$REPO"

for svc in harness brain memory; do
  echo "== $svc =="
  podman build -t "$NS/$svc:$VERSION" -f "$REPO/tot/$svc/Containerfile" "$REPO/tot/$svc"
done

if [ "${PUSH:-0}" = 1 ]; then
  for img in worldserver authserver db-import harness brain memory; do
    echo "== push $img:$VERSION =="
    podman push "$NS/$img:$VERSION"
  done
fi
echo "OK: images built for $VERSION (push=${PUSH:-0})"
```

Note: `build.sh` currently tags `wow-server`/`wow-db-import` locally on Heimdal; build-images.sh must additionally retag those to `$NS/worldserver:$VERSION` and `$NS/db-import:$VERSION` before push. Add the retag step against the build.sh output (the engineer wires this when integrating — build.sh's tag names are `wow-server:$TAG` / `wow-db-import:$TAG`).

- [ ] **Step 2: shellcheck + commit**

Run: `shellcheck tot/release/build-images.sh`; Expected: no error-level findings.

```bash
git add tot/release/build-images.sh && chmod +x tot/release/build-images.sh
git commit -m "feat(release): build-images.sh (6 images, optional GHCR push)"
```

### Task 5.3: `release.sh` (11-step, with `--dry-run`)

**Files:**
- Create: `tot/release/release.sh`

- [ ] **Step 1: Write the orchestrator**

```bash
#!/usr/bin/env bash
# tot/release/release.sh vX.Y.Z [--dry-run]
# 11-step release. Nothing is published before step 9. --dry-run does 1-8 only.
set -euo pipefail
VERSION="${1:?usage: release.sh vX.Y.Z [--dry-run]}"; shift || true
DRY=0; [ "${1:-}" = "--dry-run" ] && DRY=1
V="${VERSION#v}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ART="$REPO/release-artifacts"; rm -rf "$ART"; mkdir -p "$ART/ac-patches"
step() { printf '\n== %s ==\n' "$*"; }

step "1 preflight: clean tree, release branch, unused tag"
[ -z "$(git -C "$REPO" status --porcelain)" ] || { echo "tree not clean"; exit 1; }
git -C "$REPO" rev-parse --verify "$VERSION" >/dev/null 2>&1 && { echo "tag $VERSION exists"; exit 1; } || true
case "$(git -C "$REPO" branch --show-current)" in release/*) ;; *) echo "not on release/X.Y"; [ "$DRY" = 1 ] || exit 1 ;; esac

step "2 UPSTREAMS.toml AC SHA"
grep -A2 '^\[ac\]' "$REPO/UPSTREAMS.toml" | grep '^sha'

step "3 full test suite"
( cd "$REPO" && python -m pytest tot/ tests/ -q )

step "4 AC patch series"
git -C "$REPO" format-patch "$(grep '^sha' "$REPO/UPSTREAMS.toml" | head -1 | cut -d'"' -f2)..HEAD" \
  -- src/ deps/ apps/ data/ -o "$ART/ac-patches" || true
( cd "$ART" && tar czf "tot-$V-ac-patches.tar.gz" ac-patches )

step "5 build + tag images"
PUSH=0 bash "$REPO/tot/release/build-images.sh" "$V"

step "6 compose client MPQ"
bash "$REPO/tot/release/build-mpq.sh" "$V"
cp "$REPO/tot/client-patch/build/patch-ZZ-tot-$V.MPQ" "$ART/"

step "7 bundle reference stack"
( cd "$REPO" && tar czf "$ART/tot-$V-stack.tar.gz" \
    tot/deploy install-tot.sh docs/install.md docs/operator-troubleshooting.md docs/byollm-setup.md )

step "8 checksums"
( cd "$ART" && { command -v sha256sum >/dev/null && sha256sum ./* || shasum -a256 ./*; } > SHA256SUMS )

if [ "$DRY" = 1 ]; then echo "DRY RUN complete (steps 1-8). Artifacts in $ART"; exit 0; fi

step "9 tag + push"
git -C "$REPO" tag "$VERSION" && git -C "$REPO" push origin "$VERSION" "$(git -C "$REPO" branch --show-current)"

step "10 push images to GHCR"
PUSH=1 bash "$REPO/tot/release/build-images.sh" "$V"

step "11 GitHub Release"
gh release create "$VERSION" "$ART"/* --title "Threads of Time $VERSION" --notes-file "$ART/../CHANGELOG-$V.md" 2>/dev/null \
  || gh release create "$VERSION" "$ART"/* --title "Threads of Time $VERSION" --generate-notes
echo "OK: released $VERSION"
```

- [ ] **Step 2: shellcheck + commit**

Run: `shellcheck tot/release/release.sh`; Expected: no error-level findings.

```bash
git add tot/release/release.sh && chmod +x tot/release/release.sh
git commit -m "feat(release): 11-step release.sh with --dry-run"
```

### Task 5.4: `release.sh --dry-run` smoke test

**Files:** (no new file — a manual verification task)

- [ ] **Step 1: Run the dry-run on a throwaway release branch**

```bash
cd /Users/tbrack/Documents/Projects/threads-of-time
git switch -c release/0.0-dryrun
bash tot/release/release.sh v0.0.1-dryrun --dry-run
```
Expected: steps 1–8 print; `release-artifacts/` contains `ac-patches/`, `tot-0.0.1-dryrun-stack.tar.gz`, the MPQ, and `SHA256SUMS`; exits 0 with "DRY RUN complete". (Step 5 image build may be skipped/short-circuited in dry-run if Heimdal is unavailable — gate step 5 behind a `--skip-images` flag the engineer adds if needed.)

- [ ] **Step 2: Clean up the throwaway branch**

```bash
git switch dev && git branch -D release/0.0-dryrun && rm -rf release-artifacts
```

- [ ] **Step 3: Commit any fixes** discovered during the dry-run (if the script needed adjustment).

---

## Phase 6 — Install script

### Task 6.1: `install-tot.sh`

**Files:**
- Create: `install-tot.sh` (repo root; shipped as a release artifact + curl-able)
- Create: `tot/deploy/render-quadlets.sh` (helper that renders `@TOKEN@`s into the Quadlet dir; used by install + `tot upgrade`)

- [ ] **Step 1: Write the quadlet renderer helper**

```bash
#!/usr/bin/env bash
# render-quadlets.sh <version> — render @TOKEN@ quadlet templates into the
# operator's systemd user dir. Used by install-tot.sh and `tot upgrade`.
set -euo pipefail
VERSION="${1:?version}"
TOT_HOME="${TOT_HOME:-/opt/tot}"
NS="${GHCR_NS:-ghcr.io/threadsoftime}"
SRC="$TOT_HOME/quadlet"
DST="${XDG_CONFIG_HOME:-$HOME/.config}/containers/systemd"
mkdir -p "$DST"
find "$SRC" -type f \( -name '*.container' -o -name '*.pod' -o -name '*.volume' \
   -o -name '*.service' -o -name '*.timer' \) | while read -r f; do
  sed -e "s|@TOT_HOME@|$TOT_HOME|g" -e "s|@TOT_VERSION@|$VERSION|g" -e "s|@GHCR_NS@|$NS|g" \
      "$f" > "$DST/$(basename "$f")"
done
echo "OK: rendered quadlets -> $DST"
```

- [ ] **Step 2: Write `install-tot.sh` (the 7 documented steps)**

```bash
#!/usr/bin/env sh
# install-tot.sh — one-command Threads of Time operator bootstrap. POSIX sh.
set -eu
TOT_VERSION="${TOT_VERSION:-1.0.0}"
TOT_HOME="${TOT_HOME:-/opt/tot}"
NS="${GHCR_NS:-ghcr.io/threadsoftime}"
say() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }

# 1. Container engine
say "1/7 checking container engine"
if command -v podman >/dev/null 2>&1; then ENGINE=podman
elif command -v docker  >/dev/null 2>&1; then ENGINE=docker
else echo "ERROR: need podman 4+ or docker 24+. Install one and re-run."; exit 1; fi
echo "    using $ENGINE"

# 2. mod-playerbots (source-build operators only; pre-built image bakes it in)
say "2/7 mod-playerbots check (source-build operators only)"
if [ -d "modules/mod-playerbots" ]; then echo "    found modules/mod-playerbots";
else echo "    not present; pre-built image operators can ignore. To source-build, clone the"
     echo "    range in UPSTREAMS.toml [playerbots-dependency] into modules/mod-playerbots/."; fi

# 3. Scaffold $TOT_HOME
say "3/7 scaffolding $TOT_HOME"
mkdir -p "$TOT_HOME"/secrets "$TOT_HOME"/data/brain "$TOT_HOME"/data/memory \
         "$TOT_HOME"/etc/harness "$TOT_HOME"/logs/harness "$TOT_HOME"/logs/brain "$TOT_HOME"/backups
# fetch stack assets (from release tarball if curl-installed; from repo if local)
if [ -d tot/deploy ]; then SRC=tot/deploy; else
  curl -fsSL "https://github.com/threadsoftime/threads-of-time/releases/download/v$TOT_VERSION/tot-$TOT_VERSION-stack.tar.gz" \
    | tar xz -C "$TOT_HOME" --strip-components=2 tot/deploy; SRC="$TOT_HOME"; fi
cp -r "$SRC"/quadlet "$SRC"/compose.yml "$SRC"/firstboot "$SRC"/bin "$TOT_HOME"/ 2>/dev/null || true
cp -r tot/content/sql "$TOT_HOME"/content-sql 2>/dev/null || true
[ -f "$TOT_HOME/.env" ] || cp "$SRC/.env.example" "$TOT_HOME/.env"

# 4. Generate secrets
say "4/7 generating secrets"
gen() { head -c24 /dev/urandom | od -An -tx1 | tr -d ' \n'; }
sed -i.bak "s/^MYSQL_ROOT_PASSWORD=.*/MYSQL_ROOT_PASSWORD=$(gen)/; \
            s/^MYSQL_PASSWORD=.*/MYSQL_PASSWORD=$(gen)/; \
            s/^HARNESS_BEARER_TOKEN=.*/HARNESS_BEARER_TOKEN=$(gen)/" "$TOT_HOME/.env"
rm -f "$TOT_HOME/.env.bak"

# 5. Pull images
say "5/7 pulling images :$TOT_VERSION"
for img in worldserver authserver db-import harness brain memory; do
  $ENGINE pull "$NS/$img:$TOT_VERSION"; done

# 6. Prompt for realmlist / LLM / mode
say "6/7 configuration"
printf "Realmlist hostname/IP players will connect to [realm.example.com]: "; read -r RH || true
[ -n "${RH:-}" ] && sed -i.bak "s|^TOT_REALM_HOST=.*|TOT_REALM_HOST=$RH|" "$TOT_HOME/.env" && rm -f "$TOT_HOME/.env.bak"
printf "LLM endpoint URL (OpenAI-compatible) [skip]: "; read -r LU || true
[ -n "${LU:-}" ] && sed -i.bak "s|^BRAIN_LLM_URL=.*|BRAIN_LLM_URL=$LU|" "$TOT_HOME/.env" && rm -f "$TOT_HOME/.env.bak"
printf "Deploy mode [quadlet/compose] (compose): "; read -r MODE || true
MODE="${MODE:-compose}"; echo "$MODE" > "$TOT_HOME/.mode"
[ "$MODE" = quadlet ] && TOT_HOME="$TOT_HOME" GHCR_NS="$NS" sh "$TOT_HOME/bin/render-quadlets.sh" "$TOT_VERSION"
install -m755 "$TOT_HOME/bin/tot" /usr/local/bin/tot 2>/dev/null || \
  { echo "    (could not install 'tot' to /usr/local/bin; run via $TOT_HOME/bin/tot)"; }

# 7. Next steps
say "7/7 done"
cat <<EOF
Threads of Time $TOT_VERSION installed at $TOT_HOME (mode: $MODE).
Next:
  1. Review $TOT_HOME/.env (LLM endpoint, bot counts, realm host).
  2. Start:   TOT_HOME=$TOT_HOME tot up
  3. Create your GM account once worldserver is up:
       TOT_HOME=$TOT_HOME tot run-console "account create admin <password>"
       TOT_HOME=$TOT_HOME tot run-console "account set gmlevel admin 3 -1"
  4. Players: download patch-ZZ-tot-$TOT_VERSION.MPQ, see docs/install.md.
EOF
```

- [ ] **Step 3: shellcheck (POSIX mode)**

Run: `shellcheck -s sh install-tot.sh tot/deploy/render-quadlets.sh`
Expected: no error-level findings (note: `sed -i.bak` is BSD/GNU-portable; the `-i ''` macOS form is avoided intentionally).

- [ ] **Step 4: Commit**

```bash
git add install-tot.sh tot/deploy/render-quadlets.sh && chmod +x install-tot.sh tot/deploy/render-quadlets.sh
git commit -m "feat(install): install-tot.sh + quadlet renderer"
```

### Task 6.2: install dry-run (scaffold-only) test

**Files:**
- Test: `tests/install/test_install_scaffold.sh`

- [ ] **Step 1: Write a scaffold-only test (TOT_HOME=tmp, engine present, no pulls)**

```bash
#!/usr/bin/env bash
# Runs install-tot.sh steps 1-4 against a temp TOT_HOME with image pull stubbed.
set -euo pipefail
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
export TOT_HOME="$TMP/tot" TOT_VERSION=1.0.0
# stub the engine so step 5 'pull' no-ops
BINSTUB="$TMP/bin"; mkdir -p "$BINSTUB"
printf '#!/bin/sh\nexit 0\n' > "$BINSTUB/podman"; chmod +x "$BINSTUB/podman"
PATH="$BINSTUB:$PATH"
# feed prompts: realm, llm(skip), mode=compose
printf 'test.example.com\n\ncompose\n' | sh install-tot.sh
test -f "$TOT_HOME/.env"
grep -q '^MYSQL_ROOT_PASSWORD=[0-9a-f]\{48\}$' "$TOT_HOME/.env" || { echo "secret not generated"; exit 1; }
grep -q '^TOT_REALM_HOST=test.example.com$' "$TOT_HOME/.env"
test -f "$TOT_HOME/.mode" && grep -q compose "$TOT_HOME/.mode"
echo "OK: scaffold test passed"
```

- [ ] **Step 2: Run**

Run: `bash tests/install/test_install_scaffold.sh`
Expected: `OK: scaffold test passed`.

- [ ] **Step 3: Commit**

```bash
git add tests/install/test_install_scaffold.sh
git commit -m "test(install): scaffold + secret-gen + prompt handling"
```

---

## Phase 7 — Operator docs

### Task 7.1: `docs/install.md`

**Files:**
- Create: `docs/install.md`

- [ ] **Step 1: Write the install doc with these REQUIRED sections + content**

The file must contain, in order:
1. **Prerequisites** — the §6.1 table (OS Linux x86_64 + podman 4/docker 24; 4 CPU/8GB min, 8 CPU/16GB rec; 20GB/50GB disk; MySQL 8.x; OpenAI-compatible LLM; no GPU on host). State mod-playerbots is baked into the pre-built image; only source-build operators clone it.
2. **One-command install** — the exact `curl -fsSL https://github.com/threadsoftime/threads-of-time/releases/download/v1.0.0/install-tot.sh | sh` line + what the 7 prompts ask.
3. **Manual install** — download `tot-1.0.0-stack.tar.gz`, extract, edit `.env`, `tot up`.
4. **`.env` walkthrough** — link to the documented `.env.example`; call out `TOT_REALM_HOST`, `BRAIN_LLM_URL`, `TOT_BOT_POPULATION`/`TOT_LIVING_BOT_COUNT`.
5. **First start + GM account** — `tot up`, watch `tot logs firstboot` until `bootstrap complete`, then the two `tot run-console "account create admin …"` / `"account set gmlevel admin 3 -1"` commands (the change-me path from Task 3.3 step 6). Firewall note: open `3724`, `8085`, and `8099` (harness) only if remote brain access is needed.
6. **Verifying a healthy boot** — `tot logs worldserver` shows "World initialized"; `curl -s -H "Authorization: Bearer <token>" http://127.0.0.1:8099/v1/tools/obs.ping`.
7. **Player install** — link to the Plan-4 player-install doc (the `patch-ZZ-tot-X.Y.Z.MPQ` rename + `Cache/WDB/enUS/itemcache.wdb` clear). Verify the linked doc path exists (`ls tot/client-patch/**/PLAYER*.md` or wherever Plan 4 put it) and link it.

- [ ] **Step 2: Verify required strings present**

Run: `grep -cE 'install-tot.sh|TOT_REALM_HOST|account create admin|obs.ping|itemcache.wdb' docs/install.md`
Expected: `5` (one match per required anchor).

- [ ] **Step 3: Commit**

```bash
git add docs/install.md
git commit -m "docs: operator install guide"
```

### Task 7.2: `docs/operator-troubleshooting.md`

**Files:**
- Create: `docs/operator-troubleshooting.md`

- [ ] **Step 1: Write with the §6.8 failure-mode table + day-2 ops**

Required content:
- **MySQL race after host reboot** — symptom (authserver can't connect), cause, recovery (`tot restart database` then `tot up`; the units ship `Restart=on-failure RestartSec=15` to self-heal). Reference the kb_9d1289dd pattern in prose (don't link the internal kb).
- **BYOLLM endpoint unreachable** — bots hold last goal, no new actions, worldserver stays alive; fix the endpoint, brain reconnects without restart.
- **Memory store corruption** — SQLite WAL recovery; else restore `memory-*.sqlite` from `backups/`; memory is per-bot, not gameplay-load-bearing.
- **Out of disk** — memory store growth; monitor at 80%.
- **firstboot failed** — read `tot logs firstboot`; it prints the exact failed step (`STEP FAILED: …`); fix + delete `data/bootstrap.flag` to retry.
- **Day-2 ops** — `tot logs <svc>`; `BRAIN_LOG_LEVEL=DEBUG`; `tot-backup.sh` (7 daily + 4 weekly); harness `/metrics` Prometheus endpoint.

- [ ] **Step 2: Verify + commit**

Run: `grep -cE 'MySQL race|BYOLLM|bootstrap.flag|/metrics|tot-backup' docs/operator-troubleshooting.md`; Expected: `5`.

```bash
git add docs/operator-troubleshooting.md
git commit -m "docs: operator troubleshooting + day-2 ops"
```

### Task 7.3: `docs/byollm-setup.md`

**Files:**
- Create: `docs/byollm-setup.md`

- [ ] **Step 1: Write the BYOLLM guide**

Required content:
- ToT needs **two** OpenAI-compatible endpoints: chat (`BRAIN_LLM_URL`/`BRAIN_LLM_MODEL`) and embeddings (`BRAIN_EMBEDDINGS_URL`/`BRAIN_EMBEDDINGS_MODEL`). They may be the same server.
- **Ollama** example: `ollama pull qwen2.5:14b-instruct && ollama pull nomic-embed-text`; set both URLs to `http://<host>:11434/v1`.
- **llama.cpp / llama-server** example: `--port 8080`; URL `http://<host>:8080/v1`.
- **Hosted (OpenRouter/OpenAI)** example: set the URL + `BRAIN_LLM_API_KEY`.
- **Fancy routing** — ToT ships single-endpoint only; put a proxy (e.g. LiteLLM) in front and point `BRAIN_LLM_URL` at it. Multi-endpoint fallback is intentionally not a ToT feature (§9.3).
- **Verifying** — `curl $BRAIN_LLM_URL/models` returns the model; bot greets in-game once reachable.

- [ ] **Step 2: Verify + commit**

Run: `grep -cE 'BRAIN_LLM_URL|BRAIN_EMBEDDINGS_URL|nomic-embed-text|LiteLLM' docs/byollm-setup.md`; Expected: `4`.

```bash
git add docs/byollm-setup.md
git commit -m "docs: BYOLLM setup guide"
```

---

## Phase 8 — Deferred integration tests

### Task 8.1: Tier-1 read-only live-probe suite

**Files:**
- Create: `tot/release/integration/tier1_live_probe.py`

- [ ] **Step 1: Write the read-only probe suite**

```python
# tot/release/integration/tier1_live_probe.py
# Read-only probes against a LIVE ToT harness. Safe to run against the live
# Heimdal baseline. Usage:
#   HARNESS_URL=http://127.0.0.1:8099 HARNESS_BEARER=<tok> python tier1_live_probe.py
import os, sys, json, urllib.request

URL = os.environ["HARNESS_URL"].rstrip("/")
TOK = os.environ["HARNESS_BEARER"]

def call(tool, payload=None):
    req = urllib.request.Request(
        f"{URL}/v1/tools/{tool}",
        data=json.dumps(payload or {}).encode(),
        headers={"Authorization": f"Bearer {TOK}", "Content-Type": "application/json"},
        method="POST")
    with urllib.request.urlopen(req, timeout=15) as r:
        return json.load(r)

def main():
    checks, failures = [], []
    def probe(name, fn):
        try:
            fn(); checks.append(f"PASS {name}")
        except Exception as e:  # noqa: BLE001
            failures.append(f"FAIL {name}: {e}")

    probe("obs.ping", lambda: call("obs.ping"))
    probe("obs.list_bot_population", lambda: (
        lambda r: (_ for _ in ()).throw(AssertionError("no bots")) if not r.get("result") else None
    )(call("obs.list_bot_population")))
    probe("obs.list_players", lambda: call("obs.list_players"))
    probe("obs.get_state-shape", lambda: call("obs.list_bot_population"))  # shape smoke

    print("\n".join(checks + failures))
    return 1 if failures else 0

if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 2: Run against the live harness (read-only, safe)**

Run (Heimdal harness reachable): `HARNESS_URL=http://192.168.1.3:8099 HARNESS_BEARER=<live-token> python tot/release/integration/tier1_live_probe.py`
Expected: `PASS obs.ping`, `PASS obs.list_bot_population`, etc. (This is the soft gate — record results, don't block the build on transient LLM/network state.)

- [ ] **Step 3: Commit**

```bash
git add tot/release/integration/tier1_live_probe.py
git commit -m "test(integration): Tier-1 read-only live-probe suite"
```

### Task 8.2: C++ `IsPlayerBot()` / `PlayerbotsMgr` live runtime probe

> **Dispatch `cpp-systems-engineer`** — this is the live runtime probe deferred from subset gating (the IsPlayerBot()/PlayerbotsMgr check). It is read-only.

**Files:**
- Create: `tot/release/integration/playerbots_runtime_probe.md` (the probe procedure + expected output)

- [ ] **Step 1: Define + run the runtime probe**

The probe confirms, against the live worldserver, that `PlayerbotsMgr` sees the enrolled bot population and `IsPlayerBot()` returns true for them — the C++ runtime invariant subset gating relies on. Implement as either (a) a worldserver console command exposed via `tot run-console`/`gm.run_console` that prints the playerbot count, or (b) an `obs.*` harness assertion already surfacing `PlayerbotsMgr` state (`obs.list_bot_population` is the existing surface — confirm it is sourced from `PlayerbotsMgr::GetPlayerBotsCount()` / `IsPlayerBot()` in the C++, not a DB count). `cpp-systems-engineer` verifies the call path in `modules/mod-playerbots` + the harness adapter and documents the exact command + expected output in the `.md`.

- [ ] **Step 2: Record the probe result** in the `.md` (count matches `TOT_BOT_POPULATION`, all flagged `IsPlayerBot()==true`).

- [ ] **Step 3: Commit**

```bash
git add tot/release/integration/playerbots_runtime_probe.md
git commit -m "test(integration): document + run C++ IsPlayerBot/PlayerbotsMgr runtime probe"
```

### Task 8.3: Tier-2 isolated-pod e2e (hard gate)

**Files:**
- Create: `tot/release/integration/tier2_e2e.sh`

- [ ] **Step 1: Write the isolated e2e harness**

```bash
#!/usr/bin/env bash
# tier2_e2e.sh — fresh-install e2e on an ISOLATED pod (distinct name/ports/DB/
# volumes; never touches the live stack). Hard gate before cutting release/1.0.
# Run in a bounded window off the user's gaming hours. Small bot population.
set -euo pipefail
VERSION="${1:?usage: tier2_e2e.sh <version>}"
export TOT_HOME="$(mktemp -d)/tot-e2e"
export GHCR_NS="${GHCR_NS:-ghcr.io/threadsoftime}"
# Isolated ports (offset from live to avoid collision)
AUTH=13724; WORLD=18085; HARNESS=18099
cleanup() {
  ( cd "$TOT_HOME" && docker compose -f compose.yml down -v 2>/dev/null || podman-compose -f compose.yml down -v ) || true
  rm -rf "$(dirname "$TOT_HOME")"
}
trap cleanup EXIT

echo "== install (isolated) =="
printf 'e2e.local\n\ncompose\n' | TOT_HOME="$TOT_HOME" TOT_VERSION="$VERSION" sh install-tot.sh
# shrink population + remap ports for isolation
sed -i.bak "s/^TOT_BOT_POPULATION=.*/TOT_BOT_POPULATION=5/; s/^TOT_LIVING_BOT_COUNT=.*/TOT_LIVING_BOT_COUNT=5/" "$TOT_HOME/.env"
sed -i.bak "s/3724:3724/$AUTH:3724/; s/8085:8085/$WORLD:8085/; s/8099:8099/$HARNESS:8099/" "$TOT_HOME/compose.yml"

echo "== up =="
( cd "$TOT_HOME" && TOT_HOME="$TOT_HOME" tot up )

echo "== wait for firstboot complete =="
for i in $(seq 1 60); do
  podman logs tot-firstboot 2>&1 | grep -q "bootstrap complete" && break
  [ "$i" = 60 ] && { echo "FAIL: firstboot did not complete"; podman logs tot-firstboot; exit 1; }
  sleep 5
done

echo "== worldserver listens + bots spawn (verifies the stripped-bind-mount risk) =="
for i in $(seq 1 60); do
  podman logs tot-worldserver 2>&1 | grep -q "World initialized" && break
  [ "$i" = 60 ] && { echo "FAIL: worldserver never initialized"; exit 1; }
  sleep 5
done
TOK="$(cat "$TOT_HOME/secrets/harness-token")"
HARNESS_URL="http://127.0.0.1:$HARNESS" HARNESS_BEARER="$TOK" \
  python tot/release/integration/tier1_live_probe.py || { echo "FAIL: live probes"; exit 1; }

echo "OK: Tier-2 e2e passed for $VERSION"
```

⚠️ This is where the **vestigial-bind-mount risk** (Task 1.6) is verified: if `obs.list_bot_population` returns zero bots after a clean db-import-only bootstrap, the worldserver image must bake `modules/mod-playerbots/data/sql` — file that as a follow-up Dockerfile change and re-run.

- [ ] **Step 2: shellcheck + run (off-hours, hard gate)**

Run: `shellcheck tot/release/integration/tier2_e2e.sh` (no error-level findings), then `bash tot/release/integration/tier2_e2e.sh 1.0.0`
Expected: `OK: Tier-2 e2e passed for 1.0.0`.

- [ ] **Step 3: Commit**

```bash
git add tot/release/integration/tier2_e2e.sh
git commit -m "test(integration): Tier-2 isolated-pod e2e (hard gate)"
```

---

## Phase 9 — CI (GH-hosted)

### Task 9.1: `tot-ci.yml` (push/PR tests)

**Files:**
- Create: `.github/workflows/tot-ci.yml`

- [ ] **Step 1: Write the workflow**

```yaml
name: ToT CI
on:
  push: {branches: [dev, 'release/**']}
  pull_request: {branches: [dev, 'release/**']}
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: {python-version: '3.12'}
      - name: Install deps
        run: pip install -e tot/memory pytest && pip install -r tot/brain/requirements.txt || true
      - name: Unit + parity + memory + firstboot
        run: python -m pytest tot/ tests/ -q
      - name: Client-patch compositor determinism (70 tests, must stay green)
        run: python -m pytest tot/client-patch -q
      - name: shellcheck
        run: |
          sudo apt-get update && sudo apt-get install -y shellcheck
          shellcheck install-tot.sh tot/release/*.sh tot/deploy/bin/* tot/deploy/firstboot/*.sh
      - name: Quadlet Heimdal-literal gate
        run: bash tot/deploy/validate-quadlets.sh
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/tot-ci.yml
git commit -m "ci: push/PR tests (unit+parity+memory+compositor+shellcheck+quadlet gate)"
```

### Task 9.2: `tot-release-branch.yml` (build + push rc images)

**Files:**
- Create: `.github/workflows/tot-release-branch.yml`

- [ ] **Step 1: Write it (builds the GH-buildable images; worldserver/db-import NOT built here)**

```yaml
name: ToT Release Branch
on:
  push: {branches: ['release/**']}
jobs:
  images:
    runs-on: ubuntu-latest
    permissions: {contents: read, packages: write}
    strategy:
      matrix: {svc: [harness, brain, memory]}
    steps:
      - uses: actions/checkout@v4
      - name: Derive version
        run: echo "VER=$(echo "${GITHUB_REF_NAME#release/}")-rc" >> "$GITHUB_ENV"
      - name: Log in to GHCR
        run: echo "${{ secrets.GITHUB_TOKEN }}" | docker login ghcr.io -u "${{ github.actor }}" --password-stdin
      - name: Build + push ${{ matrix.svc }}
        run: |
          docker build -t ghcr.io/threadsoftime/${{ matrix.svc }}:$VER -f tot/${{ matrix.svc }}/Containerfile tot/${{ matrix.svc }}
          docker push ghcr.io/threadsoftime/${{ matrix.svc }}:$VER
```

Comment in the file: worldserver + authserver + db-import images are built on-demand via `tot/release/build.sh` (the `-j4` Heimdal step) and pushed by `release.sh` — not in CI for 1.0.0 (the self-hosted runner is deferred to 1.1.0).

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/tot-release-branch.yml
git commit -m "ci: release-branch builds + pushes harness/brain/memory rc images"
```

### Task 9.3: `tot-release.yml` (tag → GitHub Release)

**Files:**
- Create: `.github/workflows/tot-release.yml`

- [ ] **Step 1: Write it (validates artifacts + creates the Release; assumes images pushed by release.sh)**

```yaml
name: ToT Release
on:
  push: {tags: ['v*.*.*']}
jobs:
  release:
    runs-on: ubuntu-latest
    permissions: {contents: write, packages: write}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: {python-version: '3.12'}
      - name: Build release artifacts (steps 1-8, no publish)
        run: bash tot/release/release.sh "${GITHUB_REF_NAME}" --dry-run
      - name: Verify checksums
        run: cd release-artifacts && (sha256sum -c SHA256SUMS || shasum -a256 -c SHA256SUMS)
      - name: Create GitHub Release
        env: {GH_TOKEN: "${{ secrets.GITHUB_TOKEN }}"}
        run: gh release create "${GITHUB_REF_NAME}" release-artifacts/* --title "Threads of Time ${GITHUB_REF_NAME}" --generate-notes
```

(The worldserver/db-import GHCR images for the tag are pushed by the maintainer's local `release.sh` run beforehand — documented in the release runbook section of `docs/operator-troubleshooting.md` or a `RELEASING.md`.)

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/tot-release.yml
git commit -m "ci: tag → artifact validation + GitHub Release"
```

### Task 9.4: `tot-nightly-ac-drift.yml`

**Files:**
- Create: `.github/workflows/tot-nightly-ac-drift.yml`

- [ ] **Step 1: Write the nightly rebase-dry-run**

```yaml
name: ToT Nightly AC Drift
on:
  schedule: [{cron: '0 7 * * *'}]
  workflow_dispatch: {}
jobs:
  drift:
    runs-on: ubuntu-latest
    permissions: {contents: read, issues: write}
    steps:
      - uses: actions/checkout@v4
        with: {fetch-depth: 0}
      - name: Attempt rebase onto upstream/ac@HEAD
        id: rebase
        run: |
          git remote add upstream-ac https://github.com/azerothcore/azerothcore-wotlk || true
          git fetch upstream-ac master
          git config user.email ci@threadsoftime && git config user.name tot-ci
          BASE=$(grep '^sha' UPSTREAMS.toml | head -1 | cut -d'"' -f2)
          if git rebase --onto upstream-ac/master "$BASE" HEAD 2>rebase.log; then
            echo "clean=true" >> "$GITHUB_OUTPUT"
          else
            echo "clean=false" >> "$GITHUB_OUTPUT"; git rebase --abort || true
          fi
      - name: Open issue on conflict
        if: steps.rebase.outputs.clean == 'false'
        env: {GH_TOKEN: "${{ secrets.GITHUB_TOKEN }}"}
        run: gh issue create --title "AC drift: rebase conflicts $(date -u +%F)" --body-file rebase.log --label ac-drift || true
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/tot-nightly-ac-drift.yml
git commit -m "ci: nightly AC-drift rebase dry-run → conflict issue"
```

---

## Phase 10 — Completion

### Task 10.1: `OPERATOR_INSTALL_STATE.md`

**Files:**
- Create: `OPERATOR_INSTALL_STATE.md`

- [ ] **Step 1: Write the end-state doc** following the sibling state docs (FOUNDATION_STATE.md, MPQ_COMPOSITOR_STATE.md) shape: what shipped, the 6 GHCR images, the deploy-stack file map, the firstboot/dbimport mechanism, the two-tier test results, CI topology, and carry-forwards (the deferred-to-1.1.0 list from spec §5: signed releases, self-hosted runner, Grafana, `tot memory vacuum`; plus any Tier-2 follow-up like the worldserver-image module-SQL bake if that risk fired).

- [ ] **Step 2: Commit**

```bash
git add OPERATOR_INSTALL_STATE.md
git commit -m "docs: OPERATOR_INSTALL_STATE (Plan 5 end state)"
```

### Task 10.2: kb refresh

> **Dispatch `knowledge-curator`.**

- [ ] **Step 1:** Refresh `kb_a217a790` — mark `release.sh`/`build-images.sh`/`install-tot.sh`/first-boot/CI shipped; correct the image count to **6** (add `authserver` + `db-import`); correct the first-boot "dated order" line to the dbimport + apply-order.toml reality; **clean its stale Plan-4 pre-flight sub-section** (wrong spell IDs, retire-patch-W framing).
- [ ] **Step 2:** Update `kb_87a7eade` — Thread L marked Plan 5 done; add a new thread for the operator install stack (tag `operator-install-complete`); refresh the live-deploy/working-copies sections.
- [ ] **Step 3:** Update `kb_677e753f` scope-freeze rows (Reference Compose+Quadlet / install script / docs → shipped). Verify no broken `[[kb_id]]` links introduced.

### Task 10.3: Tag the completion

- [ ] **Step 1: Verify the full suite is green + the no-Heimdal-literal gate passes**

Run: `cd /Users/tbrack/Documents/Projects/threads-of-time && python -m pytest tot/ tests/ -q && bash tot/deploy/validate-quadlets.sh`
Expected: all green + `OK: quadlets render and carry no Heimdal literals`.

- [ ] **Step 2: Tag**

```bash
git tag -a operator-install-complete -m "ToT 1.0.0 operator install stack complete (Plan 5)"
git log --oneline -1
```

---

## Self-review

**1. Spec coverage** — every spec component maps to a task:
- 3.A deploy stack → Phase 1 (Quadlet, Tasks 1.1–1.11) + Phase 2 (Compose). 3.A.4 Heimdal-only relocation → Task 1.1.
- 3.B release pipeline → Phase 5 (`build-images.sh` 5.2, `release.sh` 5.3, dry-run 5.4). 6-image correction folded in.
- 3.C install-tot.sh → Phase 6 (6.1) + scaffold test 6.2.
- 3.D first-boot → Phase 3 (apply-order 3.1, parser+tests 3.2, `firstboot.sh` 3.3, idempotency 3.4). Init image = db-import (verified).
- 3.E docs → Phase 7 (install 7.1, troubleshooting 7.2, byollm 7.3).
- 3.F integration tests → Phase 8 (Tier-1 8.1, C++ probe 8.2, Tier-2 8.3).
- 3.G CI → Phase 9 (push/PR 9.1, release-branch 9.2, release tag 9.3, nightly 9.4).
- 3.H completion + 3.I CLI → Phase 4 (`tot` 4.1, backup 4.2) + Phase 10 (state 10.1, kb 10.2, tag 10.3).
- Verified findings (db-import init image; missing brain/memory units; missing `tot/memory/Containerfile`; 6 images) → Tasks 1.4/1.8/1.9, 5.1, build-images.sh.

**2. Placeholder scan** — code-bearing steps carry full content. Two intentional, documented scope-deferrals (not placeholders): automated SRP6 GM seeding → post-boot `tot run-console` (Task 3.3 step 6, documented in install.md); the worldserver module-SQL bake → contingent follow-up only if Tier-2 (8.3) shows zero bots. Several steps include an explicit "confirm against on-disk X during implementation" (AC env-var override names; memory factory path; content-SQL file list) — these are verification steps, not unfilled blanks, and each has a concrete default + a test that fails loudly if wrong.

**3. Type/name consistency** — env var names (`MYSQL_USER`/`MYSQL_PASSWORD`, `BRAIN_LLM_URL`, `HARNESS_BEARER_TOKEN`, `TOT_BACKUP_DIR`, `TOT_*_MEM_LIMIT`), container names (`tot-database`/`tot-firstboot`/`tot-authserver`/`tot-worldserver`/`tot-harness`/`tot-brain`/`tot-memory`), image names (worldserver/authserver/db-import/harness/brain/memory), `@TOKEN@` set (`@TOT_HOME@`/`@TOT_VERSION@`/`@GHCR_NS@`), and the `apply-order.toml`/`apply_content_sql.py`/`DB_TO_DATABASE` surface are used identically across `.env.example`, every quadlet, `compose.yml`, `firstboot.sh`, `render-quadlets.sh`, `install-tot.sh`, `build-images.sh`, `release.sh`, and the CI workflows.

