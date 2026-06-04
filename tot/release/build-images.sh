#!/usr/bin/env bash
# tot/release/build-images.sh <version>
# SPDX-License-Identifier: GPL-2.0-or-later
#
# Build + tag all 6 ToT images:
#   worldserver, authserver, db-import, harness, brain, memory
#
# worldserver + db-import are built on the build host via build.sh (-j4
# gated; the -j4 cap is non-negotiable). This script then RETAGS the
# build.sh output names to the GHCR namespace.
# harness/brain/memory build anywhere (no C++ toolchain needed).
#
# Environment:
#   GHCR_NS   — GHCR namespace (default: ghcr.io/threadsoftime)
#   PUSH      — set to 1 to push all images after build (default: 0)
#
# Usage:
#   TAG=1.0.0 bash build-images.sh 1.0.0          # build only
#   PUSH=1 TAG=1.0.0 bash build-images.sh 1.0.0   # build + push
set -euo pipefail
VERSION="${1:?usage: build-images.sh <version>}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NS="${GHCR_NS:-ghcr.io/threadsoftime}"
PUSH="${PUSH:-0}"

echo "== [1/4] worldserver + db-import (build-host build.sh, -j4) =="
# build.sh tags locally as:
#   wow-server:$VERSION  (the worldserver runtime image)
#   wow-db-import:$VERSION (the AC dbimport image)
# -j4 cap enforced inside build.sh; do NOT pass overriding MAKE_JOBS here.
TAG="${VERSION}" bash "${REPO}/tot/release/build.sh"

# Retag build.sh output names to the operator GHCR namespace.
# This runs on the build host where the images were just built; retag is a
# local metadata operation (no network, no re-build).
echo "== [1b/4] retag worldserver + db-import to ${NS} =="
podman tag "wow-server:${VERSION}"    "${NS}/worldserver:${VERSION}"
podman tag "wow-db-import:${VERSION}" "${NS}/db-import:${VERSION}"

echo "== [2/4] authserver (from AC Dockerfile --target authserver) =="
podman build \
  --target authserver \
  -t "${NS}/authserver:${VERSION}" \
  -f "${REPO}/apps/docker/Dockerfile" \
  "${REPO}"

echo "== [3/4] harness brain memory (Rust -rs crates) =="
# These are the Rust rewrites (live on Heimdal since Phases 16-18). The -rs
# Containerfiles build from the workspace root context (tot/), not tot/${svc}/.
# Image NAMES stay harness/brain/memory so deploy quadlets + compose.yml are unchanged.
for svc in harness brain memory; do
  echo "   -- ${svc} (from ${svc}-rs)"
  podman build \
    -t "${NS}/${svc}:${VERSION}" \
    -f "${REPO}/tot/${svc}-rs/Containerfile" \
    "${REPO}/tot"
done

echo "== [4/4] push=${PUSH} =="
if [ "${PUSH}" = "1" ]; then
  for img in worldserver authserver db-import harness brain memory; do
    echo "   pushing ${NS}/${img}:${VERSION}"
    podman push "${NS}/${img}:${VERSION}"
  done
else
  echo "   (PUSH=0; skipping push. Run with PUSH=1 to push images.)"
fi

echo "OK: images built for ${VERSION} (push=${PUSH})"
