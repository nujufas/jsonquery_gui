#!/usr/bin/env bash
# Build and package the Linux release binary.
#
#   build/linux.sh            # x86_64, built natively
#   build/linux.sh aarch64    # arm64, cross-built in Docker (see linux_target)
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=./common.sh
source ./common.sh

linux_target "${1:-x86_64}"

echo "==> Building $APP_NAME $VERSION for $TARGET"
"${BUILD[@]}" --release --target "$TARGET" -p jsonquery_gui

package \
    "$ROOT_DIR/target/$TARGET/release/$APP_NAME" \
    "$APP_NAME-$VERSION-linux-$ARCH" \
    tar.gz
