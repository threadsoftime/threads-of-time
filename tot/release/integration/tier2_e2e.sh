#!/usr/bin/env bash
# tot/release/integration/tier2_e2e.sh — isolated-pod end-to-end test.
#
# RUN ONLY off the user's gaming hours; requires GHCR images present
# (release.sh must have pushed :<version> first).
#
# This test spins up a COMPLETELY ISOLATED stack in a temp directory with
# offset ports (13724 / 18085 / 18099) and a 5-bot population.  It never
# touches the live Heimdal stack.  Hard gate: must pass before cutting
# a release/1.0 tag.
#
# VESTIGIAL-BIND-MOUNT RISK (Task 1.6):
#   The live Heimdal worldserver mounts source/modules at runtime.  The
#   generalized worldserver unit (Task 1.6) strips that bind-mount entirely;
#   mod-playerbots runs only from what is baked into the image.  This e2e
#   intentionally verifies that bots actually spawn after a clean
#   db-import-only bootstrap (no source/modules bind-mount).
#
#   If obs.list_bot_population returns zero bots after "World initialized",
#   the worldserver image must bake modules/mod-playerbots/data/sql —
#   file that as a follow-up Dockerfile change and re-run this test.
#
# Usage:
#   bash tot/release/integration/tier2_e2e.sh <version>
#
# Requirements:
#   - podman or docker + docker-compose / podman-compose on PATH
#   - GHCR images ghcr.io/threadsoftime/{worldserver,authserver,db-import,
#     harness,brain,memory}:<version> already pushed
#   - python3 on PATH (runs tier1_live_probe.py)
#   - install-tot.sh present in the repo root (CWD when this script runs)
set -euo pipefail

VERSION="${1:?usage: tier2_e2e.sh <version>}"

# Isolated ports — offset from live (3724/8085/8099) to avoid collision.
AUTH_PORT=13724
WORLD_PORT=18085
HARNESS_PORT=18099

# Temp install root — completely separate from the live $TOT_HOME.
_E2E_TMP="$(mktemp -d)"
export TOT_HOME="${_E2E_TMP}/tot-e2e"
export GHCR_NS="${GHCR_NS:-ghcr.io/threadsoftime}"

# Script directory (for finding tier1_live_probe.py alongside this file).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---------------------------------------------------------------------------
# Cleanup trap — tears down the isolated stack + removes temp dir on any exit.
# Uses 'down -v' to remove named volumes created during the e2e run.
# Calls compose directly (not 'tot down') so -v is honoured.
# ---------------------------------------------------------------------------
cleanup() {
  local exit_code=$?
  echo "== cleanup (exit=$exit_code) =="
  if [ -f "${TOT_HOME}/compose.yml" ]; then
    ( cd "${TOT_HOME}" && \
      docker compose -f compose.yml down -v 2>/dev/null || \
      podman-compose -f compose.yml down -v 2>/dev/null || \
      true )
  fi
  rm -rf "${_E2E_TMP}"
}
trap cleanup EXIT

echo "== install (isolated) — TOT_HOME=${TOT_HOME} =="
# Feed prompts: realm=e2e.local, LLM URL=(skip), mode=compose.
printf 'e2e.local\n\ncompose\n' | \
  TOT_HOME="${TOT_HOME}" TOT_VERSION="${VERSION}" sh install-tot.sh

# Shrink bot population for the isolated test (5 bots is enough to confirm
# the vestigial-bind-mount risk is not a blocker).
sed -i.bak \
  "s/^TOT_BOT_POPULATION=.*/TOT_BOT_POPULATION=5/; \
   s/^TOT_LIVING_BOT_COUNT=.*/TOT_LIVING_BOT_COUNT=5/" \
  "${TOT_HOME}/.env"
rm -f "${TOT_HOME}/.env.bak"

# Remap published ports in the installed compose.yml so this isolated stack
# does not collide with the live server.
sed -i.bak \
  "s/3724:3724/${AUTH_PORT}:3724/; \
   s/8085:8085/${WORLD_PORT}:8085/; \
   s/8099:8099/${HARNESS_PORT}:8099/" \
  "${TOT_HOME}/compose.yml"
rm -f "${TOT_HOME}/compose.yml.bak"

echo "== up =="
# 'tot up' in compose mode: cd $TOT_HOME && docker compose -f compose.yml up -d
# TOT_HOME must be exported so the CLI finds the stack (it reads $TOT_HOME/.mode).
TOT_HOME="${TOT_HOME}" tot up

# ---------------------------------------------------------------------------
# Wait for tot-firstboot to complete (up to 5 minutes = 60 * 5s polls).
# 'tot logs firstboot' calls: podman logs -f "tot-firstboot"
# We call podman logs directly (without -f) for a pollable non-blocking read.
# ---------------------------------------------------------------------------
echo "== wait for firstboot (tot-firstboot) =="
_FIRSTBOOT_DONE=0
for _i in $(seq 1 60); do
  if podman logs tot-firstboot 2>&1 | grep -q "bootstrap complete"; then
    _FIRSTBOOT_DONE=1
    break
  fi
  echo "  [${_i}/60] waiting for tot-firstboot bootstrap complete..."
  sleep 5
done
if [ "${_FIRSTBOOT_DONE}" -eq 0 ]; then
  echo "FAIL: tot-firstboot did not complete within 5 minutes"
  podman logs tot-firstboot 2>&1 || true
  exit 1
fi
echo "  firstboot complete."

# ---------------------------------------------------------------------------
# Wait for worldserver to initialize (up to 5 minutes).
# Greps tot-worldserver logs for "World initialized".
# ---------------------------------------------------------------------------
echo "== wait for worldserver (tot-worldserver) =="
_WORLD_DONE=0
for _i in $(seq 1 60); do
  if podman logs tot-worldserver 2>&1 | grep -q "World initialized"; then
    _WORLD_DONE=1
    break
  fi
  echo "  [${_i}/60] waiting for tot-worldserver World initialized..."
  sleep 5
done
if [ "${_WORLD_DONE}" -eq 0 ]; then
  echo "FAIL: tot-worldserver never printed 'World initialized'"
  podman logs tot-worldserver 2>&1 | tail -50 || true
  exit 1
fi
echo "  worldserver initialized."

# ---------------------------------------------------------------------------
# Read the harness bearer token.
#
# TOKEN SOURCE: $TOT_HOME/secrets/harness-token
#   firstboot.sh step 7 writes the operator token here (reading
#   HARNESS_BEARER_TOKEN from the .env injected into the container).
#   install-tot.sh step 4 generates HARNESS_BEARER_TOKEN into $TOT_HOME/.env;
#   firstboot picks it up via EnvironmentFile and writes the same value to
#   /tot/secrets/harness-token (mounted as $TOT_HOME/secrets).
#   This path is the canonical single source after firstboot completes.
#
# NOTE: $TOT_HOME/.env also carries HARNESS_BEARER_TOKEN as a fallback if
#   firstboot did not yet flush secrets/harness-token (e.g. very fast exit).
# ---------------------------------------------------------------------------
echo "== reading harness token =="
_HARNESS_TOKEN=""
if [ -s "${TOT_HOME}/secrets/harness-token" ]; then
  _HARNESS_TOKEN="$(cat "${TOT_HOME}/secrets/harness-token")"
  echo "  token source: ${TOT_HOME}/secrets/harness-token"
else
  # Fallback: grep from .env (install-tot.sh wrote it there)
  _HARNESS_TOKEN="$(grep '^HARNESS_BEARER_TOKEN=' "${TOT_HOME}/.env" | head -1 | cut -d'=' -f2)"
  echo "  token source: ${TOT_HOME}/.env (secrets/harness-token absent — fallback)"
fi
if [ -z "${_HARNESS_TOKEN}" ]; then
  echo "FAIL: could not read harness bearer token from either source"
  exit 1
fi

# ---------------------------------------------------------------------------
# Tier-1 live probes against the isolated harness.
#
# VESTIGIAL-BIND-MOUNT RISK: obs.list_bot_population checks that bots
# actually spawned.  If it returns an empty result, mod-playerbots data/sql
# is not being applied from the image — follow-up: bake the SQL into the
# worldserver image.
# ---------------------------------------------------------------------------
echo "== running Tier-1 probes against isolated harness (port ${HARNESS_PORT}) =="
HARNESS_URL="http://127.0.0.1:${HARNESS_PORT}" \
HARNESS_BEARER="${_HARNESS_TOKEN}" \
  python3 "${SCRIPT_DIR}/tier1_live_probe.py" || {
    echo "FAIL: Tier-1 live probes failed — see output above."
    echo "      If obs.list_bot_population shows zero bots, the vestigial-bind-mount"
    echo "      risk (Task 1.6) has fired. File a follow-up to bake mod-playerbots"
    echo "      data/sql into the worldserver image."
    exit 1
  }

echo ""
echo "OK: Tier-2 e2e passed for ${VERSION}"
