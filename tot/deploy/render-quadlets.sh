#!/usr/bin/env bash
# render-quadlets.sh <version> — render @TOKEN@ quadlet templates into the
# operator's systemd user dir. Used by install-tot.sh and `tot upgrade`.
set -euo pipefail
VERSION="${1:?version}"
TOT_HOME="${TOT_HOME:-/opt/tot}"
NS="${GHCR_NS:-ghcr.io/threadsoftime}"
SRC="$TOT_HOME/quadlet"
DST="${XDG_CONFIG_HOME:-$HOME/.config}/containers/systemd"
mkdir -p "$DST"
find "$SRC" -type f \( -name '*.container' -o -name '*.pod' -o -name '*.volume' \
   -o -name '*.service' -o -name '*.timer' \) | while read -r f; do
  sed -e "s|@TOT_HOME@|$TOT_HOME|g" -e "s|@TOT_VERSION@|$VERSION|g" -e "s|@GHCR_NS@|$NS|g" \
      "$f" > "$DST/$(basename "$f")"
done
echo "OK: rendered quadlets -> $DST"
