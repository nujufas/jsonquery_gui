#!/usr/bin/env bash
# Cross-compile and package the Windows x86_64 release binary.
#
# There's no native Windows toolchain on this host, so this uses `cross`
# (https://github.com/cross-rs/cross), which runs the build inside a Docker
# image that already has a matching mingw-w64 toolchain — nothing is
# installed on the host itself. Requires a working `docker` daemon.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=./common.sh
source ./common.sh

TARGET="x86_64-pc-windows-gnu"

if ! command -v cross >/dev/null 2>&1; then
    echo "==> 'cross' not found; installing (cargo install cross --locked)"
    cargo install cross --locked
fi

if ! docker info >/dev/null 2>&1; then
    echo "error: docker is not available/running — 'cross' needs it to build for Windows." >&2
    exit 1
fi

echo "==> Building $APP_NAME $VERSION for $TARGET (via cross + Docker)"
cross build --release --target "$TARGET" -p jsonquery

package \
    "$ROOT_DIR/target/$TARGET/release/$APP_NAME.exe" \
    "$APP_NAME-$VERSION-windows-x86_64" \
    zip
