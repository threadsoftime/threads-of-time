#!/usr/bin/env bash
# tools/harness-daemon/build.sh — build the daemon image on Heimdal.
#
# Usage: ./build.sh [--tag VN]   (default tag: harness-vN where N auto-bumps)
set -euo pipefail

HEIMDAL="heimdal"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TAG="${1:-harness-$(date -u +%Y%m%d-%H%M)}"

SUDO_PW=$(security find-internet-password -s "192.168.1.3" -a "brackin" -w)
trap 'unset SUDO_PW' EXIT

echo "==> rsync tools/harness-daemon to Heimdal"
rsync -az --delete --exclude='__pycache__' --exclude='.venv' --exclude='*.egg-info' \
    --exclude='.pytest_cache' --exclude='dist' --exclude='build' \
    "$REPO_ROOT/tools/harness-daemon/" \
    "$HEIMDAL:/tmp/harness-daemon-build/"

echo "==> podman build localhost/harness-daemon:${TAG}"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman build \
    -t localhost/harness-daemon:${TAG} \
    -f /tmp/harness-daemon-build/Containerfile \
    /tmp/harness-daemon-build"

echo "==> tag :current"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman tag \
    localhost/harness-daemon:${TAG} localhost/harness-daemon:current"

echo "==> images"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman images localhost/harness-daemon"

echo "==> Done. Image: localhost/harness-daemon:${TAG} (also :current)"
