# Threads of Time — Operator Troubleshooting + Day-2 Ops

---

## Failure modes

### MySQL race after host reboot

**Symptom:** `tot-authserver` exits immediately after a host reboot or full stack restart.
Logs show something like `Can't connect to MySQL server` or `Access denied`.

**Cause:** `tot-authserver` started before `tot-database` completed its MySQL
initialization handshake. This is a well-known AC race on pod/compose restarts —
the database health probe catches it in steady state but the initial host-reboot
boot order can still race.

**Recovery:**

```sh
# Wait for the database to be healthy, then restart authserver:
tot restart database
# Give MySQL ~15 seconds to finish init, then:
tot up
```

The `tot-authserver` and `tot-worldserver` units both ship `Restart=on-failure`
with a backoff, so a transient race usually self-heals within one or two retries.
If it does not after ~60 seconds, force the restart manually as above.

**Prevention:** for Quadlet operators, the `[Unit] After=tot-firstboot.service`
dependency chain means authserver waits on firstboot which waits on database
healthy. For Compose operators, `depends_on: database: condition: service_healthy`
gates the authserver on MySQL readiness.

---

### BYOLLM endpoint unreachable

**Symptom:** Bots hold their last goal and take no new actions. Worldserver, harness,
and all gameplay remain fully alive — this failure is isolated to the AI decision
layer. `tot logs brain` shows repeated connection errors to `BRAIN_LLM_URL`.

**Cause:** The LLM endpoint is down or the URL/model changed.

**Recovery:**

1. Confirm the endpoint: `curl $BRAIN_LLM_URL/models` should return a JSON model list.
2. Fix the endpoint in `$TOT_HOME/.env` (`BRAIN_LLM_URL` / `BRAIN_LLM_MODEL`).
3. Restart the brain: `tot restart brain`

The brain reconnects automatically once the endpoint is reachable. No worldserver
or harness restart is needed. See `docs/byollm-setup.md` for endpoint setup details.

---

### Memory store corruption

**Symptom:** `tot logs memory` shows SQLite errors, or `obs.list_bot_population`
returns bots with no memory state.

**Memory is per-bot and not gameplay-load-bearing** — a corrupted memory store
does not take down worldserver or harness.

**Recovery (WAL recovery first):**

```sh
# Attempt WAL checkpoint (recovers most corruption):
podman exec tot-memory sqlite3 /var/memory/memory.sqlite 'PRAGMA wal_checkpoint(TRUNCATE);'
tot restart memory
```

**Recovery (restore from backup):**

```sh
# List available snapshots:
ls $TOT_HOME/backups/memory-*.sqlite

# Stop memory, replace the DB, restart:
tot restart memory  # stop
podman cp $TOT_HOME/backups/memory-<stamp>.sqlite tot-memory:/var/memory/memory.sqlite
tot restart memory
```

Bots lose memories accumulated since the backup snapshot, but gameplay is unaffected.

---

### Out of disk

**Symptom:** MySQL or worldserver logs show write errors; `df -h` shows the volume
or host partition at 100%.

**Primary cause:** Memory store growth (sqlite WAL + journal files), or backup
accumulation in `$TOT_HOME/backups/`.

**Recovery:**

```sh
# Check disk usage:
df -h $TOT_HOME

# Prune old backups (tot-backup.sh retains 7 daily by default):
ls -lth $TOT_HOME/backups/ | head -20
# Manually remove old files if needed

# WAL checkpoint to reclaim SQLite space:
podman exec tot-memory sqlite3 /var/memory/memory.sqlite 'PRAGMA wal_checkpoint(TRUNCATE);'
```

**Prevention:** Monitor disk at 80% of the volume. The `$TOT_WORLDSERVER_MEM_LIMIT`,
`$TOT_BRAIN_MEM_LIMIT`, `$TOT_MEMORY_MEM_LIMIT`, and `$TOT_HARNESS_MEM_LIMIT` env
vars cap container RSS. Log volume growth is bounded by podman's `--log-opt max-size`
(default 10 MB per container).

---

### firstboot failed

**Symptom:** `tot-authserver` or `tot-worldserver` refuse to start, or start but
show empty databases. `tot logs firstboot` shows `[firstboot] STEP FAILED: <step>`.

**Recovery:**

```sh
# 1. Read the exact failed step:
tot logs firstboot

# 2. Fix the underlying cause (e.g. wrong MYSQL_ROOT_PASSWORD in .env,
#    LLM endpoint wrong for content SQL, etc.)

# 3. Delete the bootstrap flag to allow a retry:
rm $TOT_HOME/data/bootstrap.flag

# 4. Restart firstboot (Compose):
docker compose -f $TOT_HOME/compose.yml run --rm firstboot
# or (Quadlet):
systemctl --user start tot-firstboot.service
```

The firstboot script is idempotent for all steps up to the flag write — retrying
after a partial run is safe.

---

## Day-2 ops

### Viewing logs

```sh
tot logs worldserver      # worldserver (tailing)
tot logs harness          # harness daemon
tot logs brain            # decision brain
tot logs memory           # memory sidecar
tot logs firstboot        # init container (historical only)
```

### Debug logging

Set `BRAIN_LOG_LEVEL=DEBUG` in `$TOT_HOME/.env` and restart:

```sh
sed -i "s/^BRAIN_LOG_LEVEL=.*/BRAIN_LOG_LEVEL=DEBUG/" $TOT_HOME/.env
tot restart brain
```

Revert to `INFO` when done — `DEBUG` produces verbose JSONL output.

### Backups

`tot-backup.sh` runs nightly at 04:00 (UTC) via the backup timer. It dumps all
four databases to `$TOT_BACKUP_DIR/db-<stamp>.sql.gz` and snapshots the memory
SQLite file. Retention: 7 daily backups kept automatically.

To trigger a manual backup:

```sh
TOT_HOME=/opt/tot bash /opt/tot/bin/tot-backup.sh
```

### Harness metrics

The harness daemon exposes a Prometheus-compatible `/metrics` endpoint (port `8099`):

```sh
curl -H "Authorization: Bearer <token>" http://127.0.0.1:8099/metrics
```

Scrape this with your Prometheus/Grafana stack to track tool call rates, error
rates, and bot session counts.

### Upgrading

```sh
tot upgrade 1.0.1
```

This: snapshots databases, pulls new images, restarts the stack. Forward-only —
to roll back, restore from `$TOT_HOME/backups/` and re-deploy the old version tag.
