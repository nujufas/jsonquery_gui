#!/usr/bin/env bash
# Build the toolchain image (JDK, Android SDK/NDK, Rust, cargo-ndk, bundletool).
# Needed once, and again after changing docker/Dockerfile.build.
set -euo pipefail
ANDROID_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_IMAGE="${BUILD_IMAGE:-jsonquery-android-build:1}"
exec docker build -t "$BUILD_IMAGE" -f "$ANDROID_DIR/docker/Dockerfile.build" "$ANDROID_DIR/docker"
