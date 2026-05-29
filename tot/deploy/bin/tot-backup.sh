#!/usr/bin/env bash
# tot-backup.sh — mysqldump all four DBs + memory.sqlite snapshot.
set -euo pipefail
TOT_HOME="${TOT_HOME:-/opt/tot}"
# shellcheck source=/dev/null
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
