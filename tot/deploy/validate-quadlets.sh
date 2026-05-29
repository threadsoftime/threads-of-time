#!/usr/bin/env bash
# Renders @TOKEN@s with throwaway values and asserts no Heimdal literal survives.
# Fix (I3+M1): grep $TMP (rendered output), not $DIR (source); extend glob to *.service + *.timer.
set -euo pipefail
DIR="$(cd "$(dirname "$0")/quadlet" && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
for f in "$DIR"/*.{container,pod,volume,service,timer} "$DIR"/wow-stack/*.{container,service,timer}; do
  [ -f "$f" ] || continue
  sed -e 's|@TOT_HOME@|/opt/tot|g' -e 's|@TOT_VERSION@|1.0.0|g' \
      -e 's|@GHCR_NS@|ghcr.io/threadsoftime|g' "$f" > "$TMP/$(basename "$f")"
done
# Heimdal-literal gate — grep $TMP (rendered files), not $DIR (source)
if grep -rnE '192\.168\.1\.3|/opt/containers/wow|/var/mnt/nas|brackin|localhost/wow-server|localhost/harness-daemon|localhost/brain-sidecar|localhost/memory-sidecar' "$TMP"; then
  echo "FAIL: Heimdal-specific literal found in rendered quadlets"; exit 1
fi
echo "OK: quadlets render and carry no Heimdal literals"
