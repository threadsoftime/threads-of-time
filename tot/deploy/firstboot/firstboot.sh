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
i=0
while [ "$i" -lt 30 ]; do
  if mysqladmin ping -h "$DBHOST" -uroot -p"${MYSQL_ROOT_PASSWORD}" --silent 2>/dev/null; then
    break
  fi
  i=$(( i + 1 ))
  if [ "$i" -eq 30 ]; then
    fail "MySQL not reachable after 30 tries (15s backoff cap). Check tot-database."
  fi
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

# 6. GM account note (automated SRP6 seeding deferred to 1.0.x)
log "6/8 seeding GM account admin (CHANGE THE PASSWORD)"
# AC SRP6 verifier seeding from a shell is fragile; the operator runs:
#   tot run-console "account create admin <password>"
#   tot run-console "account set gmlevel admin 3 -1"
# documented in docs/install.md.
log "    (GM account created post-boot via 'tot' console — see docs/install.md)"

# 7. Generate harness token if absent
log "7/8 ensuring harness bearer token"
if [ ! -s /tot/secrets/harness-token ]; then
  token="${HARNESS_BEARER_TOKEN:-}"
  if [ -z "$token" ]; then
    token="$(head -c32 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  fi
  printf '%s' "$token" > /tot/secrets/harness-token
fi

# 8. Mark complete
log "8/8 writing bootstrap.flag"
date -u +%Y-%m-%dT%H:%M:%SZ > "$FLAG"
log "bootstrap complete."
