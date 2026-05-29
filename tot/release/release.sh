#!/usr/bin/env bash
# tot/release/release.sh vX.Y.Z [--dry-run] [--skip-images]
# SPDX-License-Identifier: GPL-2.0-or-later
#
# 11-step ToT release orchestrator.
#
# Steps 1-8 are safe locally (no publishes).
# Steps 9-11 push to GitHub + GHCR — only run in non-dry-run mode.
#
# Flags:
#   --dry-run      Run steps 1-8 only; exit 0 with a summary.
#   --skip-images  Skip step 5 (image build). Use when Heimdal is
#                  unavailable (the -j4 worldserver build cannot run
#                  locally). Implied by --dry-run on hosts without podman.
#
# Usage:
#   bash release.sh v1.0.0                                # full release
#   bash release.sh v0.0.1-dryrun --dry-run --skip-images # local smoke
set -euo pipefail

VERSION="${1:?usage: release.sh vX.Y.Z [--dry-run] [--skip-images]}"
shift || true

DRY=0
SKIP_IMAGES=0
for arg in "$@"; do
  case "$arg" in
    --dry-run)      DRY=1 ;;
    --skip-images)  SKIP_IMAGES=1 ;;
    *) echo "unknown flag: $arg" >&2; exit 2 ;;
  esac
done

V="${VERSION#v}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ART="${REPO}/release-artifacts"

# Clean + recreate artifact dir (use mv-to-trash, never rm -rf in prod;
# this directory is ephemeral build output, not source).
[ -d "${ART}" ] && mv "${ART}" "${ART}.prev-$(date +%Y%m%d-%H%M%S)" 2>/dev/null || true
mkdir -p "${ART}/ac-patches"

step() { printf '\n== step %s ==\n' "$*"; }

# ---------------------------------------------------------------------------
step "1/11 preflight: clean tree, release branch, unused tag"
DIRTY="$(git -C "${REPO}" status --porcelain)"
if [ -n "${DIRTY}" ]; then
  echo "ERROR: working tree not clean:"
  echo "${DIRTY}"
  exit 1
fi

# Tag must not already exist
if git -C "${REPO}" rev-parse --verify "${VERSION}" >/dev/null 2>&1; then
  echo "ERROR: tag ${VERSION} already exists"
  exit 1
fi

# Must be on a release/* branch (or dry-run)
CURRENT_BRANCH="$(git -C "${REPO}" branch --show-current)"
case "${CURRENT_BRANCH}" in
  release/*)
    echo "  branch: ${CURRENT_BRANCH} (ok)"
    ;;
  *)
    echo "  WARNING: not on release/* branch (current: ${CURRENT_BRANCH})"
    if [ "${DRY}" = "0" ]; then
      echo "ERROR: full release requires a release/* branch"
      exit 1
    fi
    echo "  (continuing in --dry-run mode)"
    ;;
esac

# ---------------------------------------------------------------------------
step "2/11 UPSTREAMS.toml AC SHA"
grep -A2 '^\[ac\]' "${REPO}/UPSTREAMS.toml" | grep '^sha'

# ---------------------------------------------------------------------------
step "3/11 full test suite"
( cd "${REPO}" && python3 -m pytest tot/ tests/ -q )

# ---------------------------------------------------------------------------
step "4/11 AC patch series"
AC_SHA="$(grep '^sha' "${REPO}/UPSTREAMS.toml" | head -1 | cut -d'"' -f2)"
git -C "${REPO}" format-patch "${AC_SHA}..HEAD" \
  -- src/ deps/ apps/ data/ \
  -o "${ART}/ac-patches" || true
( cd "${ART}" && tar czf "tot-${V}-ac-patches.tar.gz" ac-patches )
PATCH_COUNT="$(find "${ART}/ac-patches" -maxdepth 1 -name '*.patch' | wc -l | tr -d ' ')"
echo "  patches: ${PATCH_COUNT} files"

# ---------------------------------------------------------------------------
step "5/11 build + tag images"
if [ "${SKIP_IMAGES}" = "1" ]; then
  echo "  SKIPPED (--skip-images). Images must be built separately on Heimdal"
  echo "  via: PUSH=0 TAG=${V} bash tot/release/build-images.sh ${V}"
else
  PUSH=0 bash "${REPO}/tot/release/build-images.sh" "${V}"
fi

# ---------------------------------------------------------------------------
step "6/11 compose client MPQ"
if bash "${REPO}/tot/release/build-mpq.sh" "${V}" 2>/dev/null; then
  MPQ="${REPO}/tot/client-patch/build/patch-ZZ-tot-${V}.MPQ"
  if [ -f "${MPQ}" ]; then
    cp "${MPQ}" "${ART}/"
    echo "  MPQ: patch-ZZ-tot-${V}.MPQ"
  else
    echo "  WARNING: build-mpq.sh exited 0 but MPQ not found at ${MPQ}"
  fi
else
  echo "  WARNING: build-mpq.sh failed (StormLib/deps missing?). MPQ deferred."
  echo "  DEFERRED: step 6 (MPQ build). Rerun on a host with StormLib installed."
fi

# ---------------------------------------------------------------------------
step "7/11 bundle reference stack"
STACK_EXTRAS=""
for f in docs/install.md docs/operator-troubleshooting.md docs/byollm-setup.md; do
  [ -f "${REPO}/${f}" ] && STACK_EXTRAS="${STACK_EXTRAS} ${f}"
done
# shellcheck disable=SC2086
( cd "${REPO}" && tar czf "${ART}/tot-${V}-stack.tar.gz" \
    tot/deploy install-tot.sh ${STACK_EXTRAS} )
echo "  stack bundle: tot-${V}-stack.tar.gz"

# ---------------------------------------------------------------------------
step "8/11 checksums"
( cd "${ART}" && {
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum ./*
    else
      shasum -a 256 ./*
    fi
  } > SHA256SUMS )
echo "  SHA256SUMS written ($(grep -c '' "${ART}/SHA256SUMS") entries)"

# ---------------------------------------------------------------------------
if [ "${DRY}" = "1" ]; then
  echo ""
  echo "DRY RUN complete (steps 1-8). Artifacts in ${ART}"
  echo "Skipped: step 9 (tag+push), step 10 (push images), step 11 (GH Release)"
  [ "${SKIP_IMAGES}" = "1" ] && echo "Note: step 5 (image build) was also skipped (--skip-images)."
  exit 0
fi

# ---------------------------------------------------------------------------
step "9/11 tag + push"
git -C "${REPO}" tag "${VERSION}"
git -C "${REPO}" push origin "${VERSION}" "${CURRENT_BRANCH}"

# ---------------------------------------------------------------------------
step "10/11 push images to GHCR"
PUSH=1 bash "${REPO}/tot/release/build-images.sh" "${V}"

# ---------------------------------------------------------------------------
step "11/11 GitHub Release"
NOTES_FILE="${REPO}/CHANGELOG-${V}.md"
if [ -f "${NOTES_FILE}" ]; then
  gh release create "${VERSION}" "${ART}"/* \
    --title "Threads of Time ${VERSION}" \
    --notes-file "${NOTES_FILE}"
else
  gh release create "${VERSION}" "${ART}"/* \
    --title "Threads of Time ${VERSION}" \
    --generate-notes
fi

echo ""
echo "OK: released ${VERSION}"
