#!/usr/bin/env bash
# Build and package release binaries for every supported platform.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

./linux.sh
./appimage.sh
./windows.sh

echo
echo "==> Artifacts:"
ls -la ../dist
