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
