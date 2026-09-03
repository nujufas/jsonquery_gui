#!/usr/bin/env bash
# Shared config/helpers, sourced by the other scripts in this directory.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
APP_NAME="jsonquery_gui"
VERSION="$(grep -m1 '^version' "$ROOT_DIR/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"

mkdir -p "$DIST_DIR"

# Stage $1 (a built binary) plus any repo docs worth shipping into a
# directory named $2, then archive it. $3 is the archive kind: "tar.gz" or
# "zip".
package() {
    local bin_path="$1" pkg_name="$2" kind="$3"
    local stage
    stage="$(mktemp -d)"
    local out="$stage/$pkg_name"
    mkdir -p "$out"

    cp "$bin_path" "$out/"
    [ -f "$ROOT_DIR/README.md" ] && cp "$ROOT_DIR/README.md" "$out/"

    case "$kind" in
    tar.gz)
        tar -C "$stage" -czf "$DIST_DIR/$pkg_name.tar.gz" "$pkg_name"
        echo "==> Wrote $DIST_DIR/$pkg_name.tar.gz"
        ;;
    zip)
        (cd "$stage" && zip -rq "$DIST_DIR/$pkg_name.zip" "$pkg_name")
        echo "==> Wrote $DIST_DIR/$pkg_name.zip"
        ;;
    *)
        echo "package: unknown archive kind '$kind'" >&2
        exit 1
        ;;
    esac

    rm -rf "$stage"
}
