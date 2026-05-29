#!/usr/bin/env bash
# tools/harness-daemon/build.sh — build the daemon image on the build host.
#
# Usage: ./build.sh [--tag VN]   (default tag: harness-vN where N auto-bumps)
#
# Required environment (export these before running):
#   BUILD_HOST     — ssh target of the build server (e.g. an SSH config alias)
#   BUILD_SUDO_PW  — sudo password for the build host, OR configure
#                    passwordless sudo on the build host and unset this
set -euo pipefail

BUILD_HOST="${BUILD_HOST:?set BUILD_HOST to your build server}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TAG="${1:-harness-$(date -u +%Y%m%d-%H%M)}"

SUDO_PW="${BUILD_SUDO_PW:?set BUILD_SUDO_PW or configure passwordless sudo on the build host}"
trap 'unset SUDO_PW' EXIT

echo "==> rsync tools/harness-daemon to the build host"
rsync -az --delete --exclude='__pycache__' --exclude='.venv' --exclude='*.egg-info' \
    --exclude='.pytest_cache' --exclude='dist' --exclude='build' \
    "$REPO_ROOT/tools/harness-daemon/" \
    "$BUILD_HOST:/tmp/harness-daemon-build/"

echo "==> podman build localhost/harness-daemon:${TAG}"
ssh "$BUILD_HOST" "echo '$SUDO_PW' | sudo -S podman build \
    -t localhost/harness-daemon:${TAG} \
    -f /tmp/harness-daemon-build/Containerfile \
    /tmp/harness-daemon-build"

echo "==> tag :current"
ssh "$BUILD_HOST" "echo '$SUDO_PW' | sudo -S podman tag \
    localhost/harness-daemon:${TAG} localhost/harness-daemon:current"

echo "==> images"
ssh "$BUILD_HOST" "echo '$SUDO_PW' | sudo -S podman images localhost/harness-daemon"

echo "==> Done. Image: localhost/harness-daemon:${TAG} (also :current)"
