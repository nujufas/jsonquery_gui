#!/usr/bin/env bash
# Build the .snap package from snap/snapcraft.yaml. Requires snapcraft,
# plus either multipass or LXD as its build backend (snapcraft prompts to
# install multipass on first run if neither is set up).
#
#   build/snap.sh                      # amd64 -> dist/jsonquery_gui-<v>-amd64.snap
#   build/snap.sh --build-for arm64    # arm64 (cross-built on amd64) -> ...-arm64.snap
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=./common.sh
source ./common.sh

echo "==> Building snap $VERSION"
(cd "$ROOT_DIR" && snapcraft pack "$@")

BUILT="$(find "$ROOT_DIR" -maxdepth 1 -name 'jsonquery-gui_*.snap' -print -quit)"
if [ -n "$BUILT" ]; then
    # snapcraft names it jsonquery-gui_<version>_<arch>.snap
    ARCH="${BUILT##*_}"
    ARCH="${ARCH%.snap}"
    OUTPUT="$DIST_DIR/$APP_NAME-$VERSION-$ARCH.snap"
    mv "$BUILT" "$OUTPUT"
    echo "==> Wrote $OUTPUT"
else
    echo "snap.sh: no .snap file found after build" >&2
    exit 1
fi
