#!/usr/bin/env bash
# Heimdal-local bracket advance script. Runs as root via systemd.
# Distinct from scripts/advance-bracket.sh (which is laptop-driven via SSH).
# Reads MySQL password from /opt/containers/wow/.env, applies SQL, restarts.
set -euo pipefail

N="${1:?bracket number required}"
case "$N" in
  1) MAX_LEVEL=25; XP_RATE=0.25 ;;
  2) MAX_LEVEL=35; XP_RATE=0.30 ;;
  3) MAX_LEVEL=45; XP_RATE=0.35 ;;
  4) MAX_LEVEL=55; XP_RATE=0.40 ;;
  5) MAX_LEVEL=60; XP_RATE=0.50 ;;
  6) MAX_LEVEL=70; XP_RATE=0.75 ;;
  7) MAX_LEVEL=80; XP_RATE=1.00 ;;
  *) echo "Unknown bracket: $N" >&2; exit 1 ;;
esac

source /opt/containers/wow/.env

# Broadcast countdown — write directly to worldserver console via /proc/1/fd/0 of the container.
# This works because the wow-worldserver container has Tty + StdinIsOpen (set via PodmanArgs).
broadcast() {
    podman exec wow-worldserver bash -c "echo '$1' > /proc/1/fd/0" || true
}

broadcast ".announce Server entering Bracket ${N} in 30 seconds — restarting briefly."
sleep 25
broadcast ".announce Bracket ${N} in 5 seconds."
sleep 5

# Apply SQL pack — files are at /opt/containers/wow/sql/bracket${N}/ (synced by deploy.sh).
for f in /opt/containers/wow/sql/bracket${N}/*.sql; do
    [ -f "$f" ] || continue
    name=$(basename "$f")
    case "$name" in
        auth_*)        DB=acore_auth ;;
        characters_*)  DB=acore_characters ;;
        world_*|*)     DB=acore_world ;;
    esac
    podman exec -i wow-database mysql -uroot -p"${MYSQL_ROOT_PASSWORD}" "$DB" < "$f"
done

# Edit configs in place
sed -i \
  -e "s/^MaxPlayerLevel.*/MaxPlayerLevel = $MAX_LEVEL/" \
  -e "s/^Rate.XP.Quest.*/Rate.XP.Quest = $XP_RATE/" \
  -e "s/^Rate.XP.Kill.*/Rate.XP.Kill = $XP_RATE/" \
  -e "s/^Rate.XP.Explore.*/Rate.XP.Explore = $XP_RATE/" \
  /opt/containers/wow/etc/worldserver.conf

sed -i "s/^AiPlayerbot.RandomBotMaxLevel.*/AiPlayerbot.RandomBotMaxLevel = $MAX_LEVEL/" \
    /opt/containers/wow/etc/modules/playerbots.conf

# Restart
systemctl restart wow-worldserver

# Wait for "World Initialized"
DEADLINE=$(( $(date +%s) + 600 ))
while true; do
    journalctl -u wow-worldserver --since "10 min ago" --no-pager 2>/dev/null | grep -q "World Initialized" && break
    [ $(date +%s) -ge "$DEADLINE" ] && { echo "Worldserver did not reach 'World Initialized' within 10 min" >&2; exit 1; }
    sleep 5
done

# Welcome
broadcast ".announce Welcome to Bracket ${N}!"
echo "Bracket ${N} advance complete."
