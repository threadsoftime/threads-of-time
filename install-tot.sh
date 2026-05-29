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
elif command -v docker >/dev/null 2>&1; then ENGINE=docker
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
MYSQL_ROOT_PW="$(gen)"
MYSQL_PW="$(gen)"
HARNESS_TOKEN="$(gen)"
sed -i.bak "s/^MYSQL_ROOT_PASSWORD=.*/MYSQL_ROOT_PASSWORD=$MYSQL_ROOT_PW/" "$TOT_HOME/.env"
sed -i.bak "s/^MYSQL_PASSWORD=.*/MYSQL_PASSWORD=$MYSQL_PW/" "$TOT_HOME/.env"
sed -i.bak "s/^HARNESS_BEARER_TOKEN=.*/HARNESS_BEARER_TOKEN=$HARNESS_TOKEN/" "$TOT_HOME/.env"
rm -f "$TOT_HOME/.env.bak"

# Read MYSQL_USER from the .env (default acore)
MYSQL_USER="$(grep '^MYSQL_USER=' "$TOT_HOME/.env" | head -1 | cut -d'=' -f2)"
MYSQL_USER="${MYSQL_USER:-acore}"

# 4a. Resolve AC_*_DATABASE_INFO using the just-generated password.
#     Quadlet cannot interpolate ${VAR} in EnvironmentFile directives;
#     install-tot.sh writes the final resolved strings so Quadlet units
#     that read EnvironmentFile=.../.env get a literal value.
#     Compose overrides these via its own environment: block and ignores these.
DB_HOST="127.0.0.1"
sed -i.bak "s|^AC_LOGIN_DATABASE_INFO=.*|AC_LOGIN_DATABASE_INFO=${DB_HOST};3306;${MYSQL_USER};${MYSQL_PW};tot_auth|" "$TOT_HOME/.env"
sed -i.bak "s|^AC_WORLD_DATABASE_INFO=.*|AC_WORLD_DATABASE_INFO=${DB_HOST};3306;${MYSQL_USER};${MYSQL_PW};tot_world|" "$TOT_HOME/.env"
sed -i.bak "s|^AC_CHARACTER_DATABASE_INFO=.*|AC_CHARACTER_DATABASE_INFO=${DB_HOST};3306;${MYSQL_USER};${MYSQL_PW};tot_characters|" "$TOT_HOME/.env"
rm -f "$TOT_HOME/.env.bak"

# 4b. Set bearer aliases. brain_sidecar/settings.py reads HARNESS_BEARER and
#     MEMORY_BEARER; install-tot.sh sets them equal to HARNESS_BEARER_TOKEN
#     so the single operator token model works without manual duplication.
sed -i.bak "s/^HARNESS_BEARER=.*/HARNESS_BEARER=$HARNESS_TOKEN/" "$TOT_HOME/.env"
sed -i.bak "s/^MEMORY_BEARER=.*/MEMORY_BEARER=$HARNESS_TOKEN/" "$TOT_HOME/.env"
rm -f "$TOT_HOME/.env.bak"

# 4c. (tokens.yaml is written in step 6 after mode is known — URL depends on mode.)

# 5. Pull images
say "5/7 pulling images :$TOT_VERSION"
if [ "${SKIP_PULL:-0}" = "1" ]; then
  echo "    SKIP_PULL=1 — skipping image pull (local images assumed present)"
else
  for img in worldserver authserver db-import harness brain memory; do
    $ENGINE pull "$NS/$img:$TOT_VERSION"; done
fi

# 6. Prompt for realmlist / LLM / mode
say "6/7 configuration"
printf "Realmlist hostname/IP players will connect to [realm.example.com]: "; read -r RH || true
[ -n "${RH:-}" ] && sed -i.bak "s|^TOT_REALM_HOST=.*|TOT_REALM_HOST=$RH|" "$TOT_HOME/.env" && rm -f "$TOT_HOME/.env.bak"
printf "LLM endpoint URL (OpenAI-compatible) [skip]: "; read -r LU || true
[ -n "${LU:-}" ] && sed -i.bak "s|^BRAIN_LLM_URL=.*|BRAIN_LLM_URL=$LU|" "$TOT_HOME/.env" && rm -f "$TOT_HOME/.env.bak"
printf "Deploy mode [quadlet/compose] (compose): "; read -r MODE || true
MODE="${MODE:-compose}"; echo "$MODE" > "$TOT_HOME/.mode"
[ "$MODE" = quadlet ] && env TOT_HOME="$TOT_HOME" GHCR_NS="$NS" sh "$TOT_HOME/bin/render-quadlets.sh" "$TOT_VERSION"
install -m755 "$TOT_HOME/bin/tot" /usr/local/bin/tot 2>/dev/null || \
  { echo "    (could not install 'tot' to /usr/local/bin; run via $TOT_HOME/bin/tot)"; }

# 6a. Seed harness tokens.yaml (deferred to here because URL depends on MODE).
#     Schema: top-level ac_bridge_url / audit_path / listen_address /
#     max_augmented_bots / tokens list.  Each token entry requires:
#     token (str), identity (str), scope (list[str]).
#     mod-harness-bridge listens on port 8091 (not 8080).
#     URL is mode-aware:
#       Quadlet (pod, shared netns) → http://127.0.0.1:8091 (shared network namespace)
#       Compose (separate containers) → http://worldserver:8091 (service DNS name)
HARNESS_TOKENS_YAML="$TOT_HOME/etc/harness/tokens.yaml"
if [ "$MODE" = "quadlet" ]; then
  _AC_BRIDGE_URL="http://127.0.0.1:8091"
else
  _AC_BRIDGE_URL="http://worldserver:8091"
fi
cat > "$HARNESS_TOKENS_YAML" << YAML
# Harness daemon token config — generated by install-tot.sh.
# See tot/harness/src/harness_daemon/config.py for the full schema.
# To mint additional tokens: harness-daemon mint-token --identity <id> --scope <scope>
ac_bridge_url: "${_AC_BRIDGE_URL}"
audit_path: "/var/log/harness/daemon.jsonl"
listen_address: "0.0.0.0:8099"
max_augmented_bots: 7

tokens:
  - token: "$HARNESS_TOKEN"
    identity: "operator"
    scope:
      - "obs.*"
      - "bot.*"
      - "gm.*"
      - "memory.*"
    note: "operator master token — generated by install-tot.sh"
YAML

# 7. Next steps
say "7/7 done"
cat << EOF
Threads of Time $TOT_VERSION installed at $TOT_HOME (mode: $MODE).
Next:
  1. Review $TOT_HOME/.env (LLM endpoint, bot counts, realm host).
  2. Start:   TOT_HOME=$TOT_HOME tot up
  3. Create your GM account once worldserver is up:
       TOT_HOME=$TOT_HOME tot run-console "account create admin <password>"
       TOT_HOME=$TOT_HOME tot run-console "account set gmlevel admin 3 -1"
  4. Players: download patch-ZZ-tot-$TOT_VERSION.MPQ, see docs/install.md.
EOF
