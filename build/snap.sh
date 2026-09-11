#!/usr/bin/env bash
# Build the .snap package from snap/snapcraft.yaml. Requires snapcraft,
# plus either multipass or LXD as its build backend (snapcraft prompts to
# install multipass on first run if neither is set up).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=./common.sh
source ./common.sh

OUTPUT="$DIST_DIR/$APP_NAME-$VERSION-amd64.snap"

echo "==> Building snap $VERSION"
(cd "$ROOT_DIR" && snapcraft pack "$@")

BUILT="$(find "$ROOT_DIR" -maxdepth 1 -name 'jsonquery-gui_*.snap' -print -quit)"
if [ -n "$BUILT" ]; then
    mv "$BUILT" "$OUTPUT"
    echo "==> Wrote $OUTPUT"
else
    echo "snap.sh: no .snap file found after build" >&2
    exit 1
fi
