#!/usr/bin/env bash
# Build and package the native Linux x86_64 release binary.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=./common.sh
source ./common.sh

TARGET="x86_64-unknown-linux-gnu"

echo "==> Building $APP_NAME $VERSION for $TARGET"
cargo build --release --target "$TARGET" -p jsonquery_gui

package \
    "$ROOT_DIR/target/$TARGET/release/$APP_NAME" \
    "$APP_NAME-$VERSION-linux-x86_64" \
    tar.gz
