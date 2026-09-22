#!/usr/bin/env bash
# Shared config/helpers, sourced by the other scripts in this directory.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
APP_NAME="jsonquery_gui"
VERSION="$(grep -m1 '^version' "$ROOT_DIR/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"

mkdir -p "$DIST_DIR"

# Pick the Linux target for the arch in $1 (x86_64 or aarch64) and set TARGET,
# ARCH and BUILD (the cargo command to run). Both build in Docker via `cross`,
# whose default image (matching the installed `cross` version) links against
# an old glibc (2.23): a *native* build links against the host's own glibc
# instead, and the result then refuses to start on any distro older than the
# machine it was built on — the v0.4.1 x86_64 release learned this the hard
# way (built natively here on glibc 2.43, wouldn't even start on Ubuntu
# 22.04/24.04); aarch64 was cross-built from the start and never had the
# problem.
linux_target() {
    ARCH="$1"
    case "$ARCH" in
    x86_64)
        TARGET="x86_64-unknown-linux-gnu"
        ;;
    aarch64)
        TARGET="aarch64-unknown-linux-gnu"
        ;;
    *)
        echo "error: unknown arch '$ARCH' (expected x86_64 or aarch64)" >&2
        exit 1
        ;;
    esac
    BUILD=(cross build)
    if ! command -v cross >/dev/null 2>&1; then
        echo "==> 'cross' not found; installing (cargo install cross --locked)"
        cargo install cross --locked
    fi
    if ! docker info >/dev/null 2>&1; then
        echo "error: docker is not available/running — 'cross' needs it to build for $ARCH." >&2
        exit 1
    fi
}

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
