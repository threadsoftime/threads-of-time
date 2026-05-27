#!/usr/bin/env bash
# tot/release/build.sh — build the Threads of Time worldserver image.
#
# Lineage: adapted from azerothcore-heimdal:image/build.sh. The key
# difference: AzerothCore + mod-playerbots source now lives IN this repo
# (src/, modules/, deps/, apps/, etc.), so there is no clone step and no
# patch-overlay rsync. The script rsyncs the working tree onto Heimdal,
# runs the AC Dockerfile to build the worldserver + db-import images, and
# tags them.
#
# Discipline preserved from the source script:
#   - rsync to Heimdal (build runs there, not on the laptop)
#   - -j4 cap (kb_adbafeda — never raise even when "just this once")
#   - worldserver binary mtime cross-check after build (kb_f974fc65 —
#     BUILD OK from the script has been historically unreliable; cross-
#     check is mandatory before declaring success)
#   - image bake + :current retag
#
# Full release pipeline (release.sh — GHCR push, GitHub Release creation,
# checksums, signed artifacts) is deferred to Plan 6.
#
# Run from laptop. Requires `ssh heimdal` key auth and the brackin sudo
# password in macOS keychain.
set -euo pipefail

HEIMDAL="heimdal"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TAG="${TAG:-tot-$(date -u +%Y%m%d)}"
BUILD_DIR="/tmp/tot-build"

SUDO_PW=$(security find-internet-password -s "192.168.1.3" -a "brackin" -w)
on_exit() { unset SUDO_PW; }
trap on_exit EXIT

step() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }

step "Preparing build directory on Heimdal"
ssh "$HEIMDAL" "rm -rf ${BUILD_DIR} && mkdir -p ${BUILD_DIR}"

step "Rsyncing repo working tree to Heimdal (${BUILD_DIR})"
# Exclude build artifacts, virtualenvs, and the deploy/release run-state
# directories (Heimdal doesn't need them to compile the worldserver).
# NOTE: .git is INCLUDED — apps/docker/Dockerfile bind-mounts source=.git for
# AC's git-revision build metadata. Excluding it broke the Dockerfile bind.
rsync -a --delete \
    --exclude='__pycache__' \
    --exclude='.venv' \
    --exclude='.pytest_cache' \
    --exclude='*.egg-info' \
    --exclude='node_modules' \
    --exclude='*.log' \
    --exclude='/var/' \
    --exclude='/env/' \
    "$REPO_ROOT/" "$HEIMDAL:${BUILD_DIR}/"

step "Populating mod-playerbots into build tree (build-time dependency, not vendored)"
# mod-playerbots is an external operator-installed dependency at RUNTIME, but
# mod-harness-bridge + (future) mod-agenticbots glue both need its headers at
# BUILD TIME (PlayerbotAI*, GET_PLAYERBOT_AI macro, Bot/LlmAgent/* headers).
# Per spec §3.6 + §6.1: operators source-building must clone mod-playerbots
# into modules/ before building; pre-built containers (the primary distribution
# path) have it baked in by ToT's CI. Here on Heimdal we rsync from the
# existing install at /opt/containers/wow/source/modules/mod-playerbots/.
# Falls back to git clone if the local install isn't present.
ssh "$HEIMDAL" "
    set -e
    if [ -d /opt/containers/wow/source/modules/mod-playerbots ]; then
        echo 'Using existing mod-playerbots install at /opt/containers/wow/source/modules/mod-playerbots'
        rsync -a --delete --exclude='.git' \\
            /opt/containers/wow/source/modules/mod-playerbots/ \\
            ${BUILD_DIR}/modules/mod-playerbots/
    else
        echo 'Local mod-playerbots install not found; cloning upstream'
        git clone --depth=1 https://github.com/liyunfan1223/mod-playerbots.git \\
            ${BUILD_DIR}/modules/mod-playerbots
    fi
    ls ${BUILD_DIR}/modules/mod-playerbots/src/ | head -5
"

step "Pinning libmysqlclient to 8.0.45-0ubuntu0.22.04.1 in AC Dockerfile (kb_57b453cd Step 2.5)"
# Stock upstream Dockerfile installs libmysqlclient21 unpinned, which apt
# resolves to whatever Ubuntu 22.04 ships at build time. The acore/ac-wotlk-*
# images on Docker Hub were built against 8.0.35; our runtime worldserver
# container links against 8.0.45 from the host's mod-playerbots build, and
# the ABI mismatch crash-loops the worldserver with ACE00046 on startup.
# Pin both the dev headers (build stage) and the runtime .so (runtime stage)
# to 8.0.45-0ubuntu0.22.04.1 so the entire image is internally consistent.
#
# If upstream changes either line, the verification grep below fails loudly
# and the build aborts before we burn ~40 min on a poisoned image. When that
# happens, re-recon the upstream Dockerfile and update the sed patterns here.
ssh "$HEIMDAL" "
    DF=${BUILD_DIR}/apps/docker/Dockerfile;
    # Runtime stage: pin libmysqlclient21.
    sed -i 's|^      libmysqlclient21 libreadline8 \\\\\$|      libmysqlclient21=8.0.45-0ubuntu0.22.04.1 libreadline8 \\\\|' \"\$DF\";
    # Build stage: pin libmysqlclient-dev alongside default-libmysqlclient-dev.
    sed -i 's|^        default-libmysqlclient-dev libboost-all-dev|        default-libmysqlclient-dev libmysqlclient-dev=8.0.45-0ubuntu0.22.04.1 libboost-all-dev|' \"\$DF\";
    # Verify both pins landed. If either grep fails, abort the build now.
    grep -nF 'libmysqlclient21=8.0.45-0ubuntu0.22.04.1' \"\$DF\" \\
        || { echo 'PIN FAILED: libmysqlclient21 anchor did not match — upstream Dockerfile changed?'; exit 1; }
    grep -nF 'libmysqlclient-dev=8.0.45-0ubuntu0.22.04.1' \"\$DF\" \\
        || { echo 'PIN FAILED: libmysqlclient-dev anchor did not match — upstream Dockerfile changed?'; exit 1; }
"

step "Building worldserver image (~30-40 min on first run)"
# IMPORTANT: -j4 cap (kb_adbafeda). Do NOT raise this even when asked to
# "just this once max it out." The always-on baseline (worldserver + 100
# bots + llama-server-rocm) is already heavy; -j8+ here has triggered
# OOMs and the GPU thermal CTF (kb_f9bf8ed1).
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman build \
    --target worldserver \
    --build-arg MAKE_JOBS=4 \
    -f ${BUILD_DIR}/apps/docker/Dockerfile \
    -t wow-server:${TAG} \
    ${BUILD_DIR}"

step "Building db-import image (cached layers — fast)"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman build \
    --target db-import \
    --build-arg MAKE_JOBS=4 \
    -f ${BUILD_DIR}/apps/docker/Dockerfile \
    -t wow-db-import:${TAG} \
    ${BUILD_DIR}"

step "Cross-checking worldserver binary mtime (kb_f974fc65)"
# BUILD OK from podman build has been historically unreliable — caches can
# resurrect a stale binary. Confirm the binary inside the freshly-tagged
# image was actually written within the last 60 minutes. If older, fail
# loudly before we tag :current.
BUILD_AGE=$(ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman run --rm \
    --entrypoint /bin/sh localhost/wow-server:${TAG} \
    -c 'stat -c %Y /azerothcore/env/dist/bin/worldserver'")
NOW=$(date +%s)
AGE_MIN=$(( (NOW - BUILD_AGE) / 60 ))
echo "  worldserver binary age: ${AGE_MIN} minutes"
if (( AGE_MIN > 60 )); then
    echo "  FAILED: worldserver binary is ${AGE_MIN}min old — build cache resurrected a stale binary."
    echo "  Refusing to tag :current. Re-run with --no-cache or investigate the build layers."
    exit 1
fi

step "Tagging as :current"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman tag wow-server:${TAG} wow-server:current"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman tag wow-db-import:${TAG} wow-db-import:current"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman images wow-server"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman images wow-db-import"

step "Provisioning module .conf files on host bind-mount"
# The Quadlet bind-mounts /opt/containers/wow/etc → /azerothcore/env/dist/etc,
# which SHADOWS the runtime image's etc/modules/ directory. AC's install step
# inside the image puts each module's .conf.dist at /azerothcore/env/dist/etc/modules/,
# but at runtime the worldserver looks at the bind-mount and never sees them.
# Worldserver reads NAME.conf (not NAME.conf.dist), so we must seed both:
#   - .conf.dist  : always refresh from image (canonical defaults)
#   - .conf       : seed from .conf.dist ONLY if missing (preserve user edits)
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S bash -c '
    TMP=\$(mktemp -d)
    podman run --rm --entrypoint /bin/bash localhost/wow-server:current \
        -c \"tar -cf - -C /azerothcore/env/dist/etc/modules .\" > \${TMP}/modules.tar
    mkdir -p \${TMP}/extract
    tar -xf \${TMP}/modules.tar -C \${TMP}/extract
    mkdir -p /opt/containers/wow/etc/modules
    for dist in \${TMP}/extract/*.conf.dist; do
        [[ -f \"\$dist\" ]] || continue
        name=\$(basename \"\$dist\")
        cp -f \"\$dist\" /opt/containers/wow/etc/modules/\"\$name\"
        target=/opt/containers/wow/etc/modules/\"\${name%.dist}\"
        if [[ ! -f \"\$target\" ]]; then
            cp \"\$dist\" \"\$target\"
            echo \"  seeded \${name%.dist} from \$name\"
        fi
    done
    chmod 644 /opt/containers/wow/etc/modules/*.conf* 2>/dev/null || true
    rm -rf \${TMP}
'"

step "Syncing modules + src trees to /opt/containers/wow/source/ (runtime + incremental builds)"
# mod-playerbots auto-populates its DB at worldserver startup by sourcing
# SQL files from /azerothcore/modules/mod-playerbots/data/sql/. The
# --target worldserver image doesn't include modules data, so we persist
# the build tree for the worldserver Quadlet to bind-mount.
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S bash -c '
    mkdir -p /opt/containers/wow/source &&
    rsync -a --delete ${BUILD_DIR}/modules/ /opt/containers/wow/source/modules/ &&
    rsync -a --delete ${BUILD_DIR}/src/ /opt/containers/wow/source/src/ &&
    chown -R 1000:1000 /opt/containers/wow/source/modules /opt/containers/wow/source/src
'"

step "Smoke test: worldserver binary present"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman run --rm --entrypoint /bin/ls localhost/wow-server:current /azerothcore/env/dist/bin/"

step "Smoke test: modules linked in"
ssh "$HEIMDAL" "echo '$SUDO_PW' | sudo -S podman run --rm --entrypoint /bin/sh localhost/wow-server:current -c 'strings /azerothcore/env/dist/bin/worldserver 2>/dev/null | grep -i -E \"playerbot|progression|eluna|autobalance|bracket|warforged|harness\" | sort -u | head -20'"

step "Done. Image: wow-server:${TAG} (also tagged :current)"
