#!/usr/bin/env bash
# Shared helpers for the android/ scripts. Source it; don't run it.
#
# ANDROID_USER_HOME matters: Java finds the user's home from the passwd entry
# (not $HOME), so without it every throw-away container would generate its own
# debug keystore, and no debug build could update-install over the last one.
#
# Everything that needs a compiler, the Android SDK or Rust runs inside the
# toolchain container (docker/Dockerfile.build), so the host needs nothing but
# Docker. The repo is mounted at /work, and the container runs as you, so the
# files it writes are yours. Caches live in android/.build (safe to delete).

ANDROID_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$ANDROID_DIR/.." && pwd)"
BUILD_IMAGE="${BUILD_IMAGE:-jsonquery-android-build:1}"
CACHE_DIR="$ANDROID_DIR/.build"
EMU_IMAGE="${EMU_IMAGE:-jsonquery-android-emulator:2}"

log() { echo "[android] $*" >&2; }
die() { log "error: $*"; exit 1; }

command -v docker >/dev/null 2>&1 || die "Docker is required (nothing else is installed on the host)."

# The emulator image (docker/Dockerfile.emulator): what test/run.sh and
# run-phone.sh / run-tablet.sh boot the app in.
ensure_emulator_image() {
    if ! docker image inspect "$EMU_IMAGE" >/dev/null 2>&1; then
        log "Building the emulator image $EMU_IMAGE (~9 GB, once)..."
        docker build -t "$EMU_IMAGE" -f "$ANDROID_DIR/docker/Dockerfile.emulator" "$ANDROID_DIR/docker"
    fi
}

ensure_image() {
    if ! docker image inspect "$BUILD_IMAGE" >/dev/null 2>&1; then
        log "Toolchain image $BUILD_IMAGE not found; building it (~7 GB, once)..."
        "$ANDROID_DIR/scripts/image.sh"
    fi
}

# Run a command in the toolchain container, from android/.
#   in_container [--env NAME]... -- command args...
# Each --env NAME passes the host's $NAME through if it is set.
in_container() {
    local env_args=()
    while [ "${1:-}" = "--env" ]; do
        shift
        if [ -n "${!1:-}" ]; then env_args+=(-e "$1"); fi
        shift
    done
    [ "${1:-}" = "--" ] && shift

    ensure_image
    mkdir -p "$CACHE_DIR"/{home,cargo,gradle,target}
    local tty_args=()
    [ -t 0 ] && [ -t 1 ] && tty_args=(-t)

    # -i: forward stdin, so `in_container -- bash -s <<EOF ... EOF` works.
    docker run --rm -i "${tty_args[@]}" \
        --user "$(id -u):$(id -g)" \
        -e HOME=/work/android/.build/home \
        -e ANDROID_USER_HOME=/work/android/.build/home/.android \
        -e CARGO_HOME=/work/android/.build/cargo \
        -e CARGO_TARGET_DIR=/work/android/.build/target \
        -e GRADLE_USER_HOME=/work/android/.build/gradle \
        "${env_args[@]}" \
        -v "$REPO_ROOT":/work \
        -w /work/android \
        "$BUILD_IMAGE" "$@"
}

# The version everything is released as: the workspace version in the root Cargo.toml.
workspace_version() {
    awk '/^\[workspace.package\]/ {s=1; next} s && /^version[[:space:]]*=/ {gsub(/[" ]/, "", $3); print $3; exit}' \
        "$REPO_ROOT/Cargo.toml"
}

version_code() {
    awk -F= '/^versionCode=/ {print $2}' "$ANDROID_DIR/version.properties"
}
