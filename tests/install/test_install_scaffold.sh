#!/usr/bin/env bash
# tests/install/test_install_scaffold.sh
#
# Runs install-tot.sh steps 1-4 against a temp TOT_HOME with the container
# engine and image pull stubbed to no-ops. Does NOT pull real images, does
# NOT touch any live server.
#
# Asserts:
#   - .env created + MYSQL_ROOT_PASSWORD generated (48 hex chars)
#   - TOT_REALM_HOST prompt handled
#   - .mode written as "compose"
#   - AC_LOGIN_DATABASE_INFO / AC_WORLD_DATABASE_INFO / AC_CHARACTER_DATABASE_INFO
#     are non-empty and contain the generated MYSQL_PASSWORD
#   - HARNESS_BEARER and MEMORY_BEARER are set and equal to HARNESS_BEARER_TOKEN
#   - $TOT_HOME/etc/harness/tokens.yaml exists and contains the generated token
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

export TOT_HOME="$TMP/tot" TOT_VERSION=1.0.0

# Stub the container engine so step 5 'pull' no-ops
BINSTUB="$TMP/bin"
mkdir -p "$BINSTUB"
printf '#!/bin/sh\nexit 0\n' > "$BINSTUB/podman"
chmod +x "$BINSTUB/podman"
PATH="$BINSTUB:$PATH"

# Feed prompts: realm, llm(skip), mode=compose
# Run from the repo root so install-tot.sh can find tot/deploy + tot/content/sql
# using its relative-path check ([ -d tot/deploy ]).
printf 'test.example.com\n\ncompose\n' | ( cd "$REPO" && sh install-tot.sh )

# --- Basic scaffold checks ---
test -f "$TOT_HOME/.env"         || { echo "FAIL: .env not created"; exit 1; }
test -f "$TOT_HOME/.mode"        || { echo "FAIL: .mode not created"; exit 1; }
grep -q compose "$TOT_HOME/.mode" || { echo "FAIL: .mode not 'compose'"; exit 1; }
grep -q "^TOT_REALM_HOST=test.example.com$" "$TOT_HOME/.env" || { echo "FAIL: TOT_REALM_HOST not set"; exit 1; }

# --- Secret generation: MYSQL_ROOT_PASSWORD = 48 hex chars ---
MYSQL_ROOT_PW="$(grep '^MYSQL_ROOT_PASSWORD=' "$TOT_HOME/.env" | head -1 | cut -d'=' -f2)"
if ! printf '%s' "$MYSQL_ROOT_PW" | grep -qE '^[0-9a-f]{48}$'; then
  echo "FAIL: MYSQL_ROOT_PASSWORD not a 48-hex-char value, got: $MYSQL_ROOT_PW"; exit 1
fi

# --- Extract all the generated secrets ---
MYSQL_PW="$(grep '^MYSQL_PASSWORD=' "$TOT_HOME/.env" | head -1 | cut -d'=' -f2)"
HARNESS_TOKEN="$(grep '^HARNESS_BEARER_TOKEN=' "$TOT_HOME/.env" | head -1 | cut -d'=' -f2)"

if [ -z "$MYSQL_PW" ]; then
  echo "FAIL: MYSQL_PASSWORD is empty"; exit 1
fi
if [ -z "$HARNESS_TOKEN" ]; then
  echo "FAIL: HARNESS_BEARER_TOKEN is empty"; exit 1
fi

# --- AC_*_DATABASE_INFO: non-empty and contain the generated MYSQL_PASSWORD ---
for var in AC_LOGIN_DATABASE_INFO AC_WORLD_DATABASE_INFO AC_CHARACTER_DATABASE_INFO; do
  val="$(grep "^${var}=" "$TOT_HOME/.env" | head -1 | cut -d'=' -f2)"
  if [ -z "$val" ]; then
    echo "FAIL: $var is empty in .env"; exit 1
  fi
  if ! printf '%s' "$val" | grep -qF "$MYSQL_PW"; then
    echo "FAIL: $var does not contain the generated MYSQL_PASSWORD"
    echo "  $var=$val"
    echo "  MYSQL_PASSWORD=$MYSQL_PW"
    exit 1
  fi
  echo "  PASS: $var is non-empty and contains generated password"
done

# --- HARNESS_BEARER and MEMORY_BEARER set and equal to HARNESS_BEARER_TOKEN ---
HARNESS_BEARER="$(grep '^HARNESS_BEARER=' "$TOT_HOME/.env" | head -1 | cut -d'=' -f2)"
MEMORY_BEARER="$(grep '^MEMORY_BEARER=' "$TOT_HOME/.env" | head -1 | cut -d'=' -f2)"

if [ -z "$HARNESS_BEARER" ]; then
  echo "FAIL: HARNESS_BEARER is empty in .env"; exit 1
fi
if [ -z "$MEMORY_BEARER" ]; then
  echo "FAIL: MEMORY_BEARER is empty in .env"; exit 1
fi
if [ "$HARNESS_BEARER" != "$HARNESS_TOKEN" ]; then
  echo "FAIL: HARNESS_BEARER ($HARNESS_BEARER) != HARNESS_BEARER_TOKEN ($HARNESS_TOKEN)"; exit 1
fi
if [ "$MEMORY_BEARER" != "$HARNESS_TOKEN" ]; then
  echo "FAIL: MEMORY_BEARER ($MEMORY_BEARER) != HARNESS_BEARER_TOKEN ($HARNESS_TOKEN)"; exit 1
fi
echo "  PASS: HARNESS_BEARER and MEMORY_BEARER equal HARNESS_BEARER_TOKEN"

# --- tokens.yaml exists and contains the generated HARNESS_BEARER_TOKEN ---
TOKENS_YAML="$TOT_HOME/etc/harness/tokens.yaml"
if [ ! -f "$TOKENS_YAML" ]; then
  echo "FAIL: tokens.yaml not found at $TOKENS_YAML"; exit 1
fi
if ! grep -qF "$HARNESS_TOKEN" "$TOKENS_YAML"; then
  echo "FAIL: tokens.yaml does not contain the generated HARNESS_BEARER_TOKEN"
  echo "  Token expected: $HARNESS_TOKEN"
  echo "  tokens.yaml contents:"
  cat "$TOKENS_YAML"
  exit 1
fi
echo "  PASS: tokens.yaml exists and contains the generated bearer token"

echo "OK: scaffold test passed"
