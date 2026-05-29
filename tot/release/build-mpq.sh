#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Build the unified ToT client MPQ (Stage A + Stage B) + determinism gate.
# Usage: tot/release/build-mpq.sh <version>   (default 0.0.0-dev)
set -euo pipefail

VERSION="${1:-0.0.0-dev}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CP="$REPO/tot/client-patch"
OUT="$CP/build/patch-ZZ-tot-$VERSION.MPQ"

echo "== Stage A: compose AddOn =="
python3 "$CP/compose-tot-addon.py"

echo "== Stage B: pack MPQ ($VERSION) =="
python3 "$CP/pack-mpq.py" --version "$VERSION" --out "$OUT"

echo "== Determinism gate =="
python3 "$CP/pack-mpq.py" --version "$VERSION" --check

echo "== sha256 =="
if command -v sha256sum >/dev/null; then sha256sum "$OUT"; else shasum -a 256 "$OUT"; fi
echo "OK: $OUT"
