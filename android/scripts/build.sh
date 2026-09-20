#!/usr/bin/env bash
# Build the Android app in the toolchain container.
#
#   android/scripts/build.sh apk   [options]   debug APK -> android/dist/
#   android/scripts/build.sh aab   [options]   release bundle for Google Play -> android/dist/
#   android/scripts/build.sh check             rustfmt, clippy and Android lint; no artifacts
#
# Options:
#   --abis a,b,c     ABIs to build the Rust library for. Default: x86_64 for
#                    `apk` (an emulator), arm64-v8a,armeabi-v7a,x86_64 for `aab`.
#                    (arm64-v8a is what phones run.)
#   --profile p      Cargo profile (android/Cargo.toml): `ci` (quick to build,
#                    default for `apk`) or `release` (default for `aab`).
#
# `aab` signs with your upload key if one is configured (see README.md,
# "Signing"); otherwise the bundle is unsigned, fine to inspect but not for Play.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

command="${1:-}"
[ -n "$command" ] || { sed -n '2,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 2; }
shift

abis="" profile=""
while [ $# -gt 0 ]; do
    case "$1" in
        --abis) abis="$2"; shift 2 ;;
        --profile) profile="$2"; shift 2 ;;
        *) die "unknown option: $1" ;;
    esac
done

version="$(workspace_version)"
code="$(version_code)"
mkdir -p "$ANDROID_DIR/dist"

case "$command" in
    apk)
        abis="${abis:-x86_64}"; profile="${profile:-ci}"
        log "Building debug APK $version ($code) for $abis, Rust profile $profile"
        in_container -- ./gradlew assembleDebug -PrustAbis="$abis" -PrustProfile="$profile"
        cp "$ANDROID_DIR/app/build/outputs/apk/debug/app-debug.apk" "$ANDROID_DIR/dist/jsonquery-$version-debug.apk"
        log "-> android/dist/jsonquery-$version-debug.apk"
        ;;
    aab)
        abis="${abis:-arm64-v8a,armeabi-v7a,x86_64}"; profile="${profile:-release}"
        log "Building release bundle $version ($code) for $abis, Rust profile $profile"
        in_container --env ANDROID_KEYSTORE_FILE --env ANDROID_KEYSTORE_PASSWORD \
            --env ANDROID_KEY_ALIAS --env ANDROID_KEY_PASSWORD -- \
            ./gradlew bundleRelease -PrustAbis="$abis" -PrustProfile="$profile"
        cp "$ANDROID_DIR/app/build/outputs/bundle/release/app-release.aab" "$ANDROID_DIR/dist/jsonquery-$version.aab"
        # (The native symbol tables are inside the bundle itself, under
        # BUNDLE-METADATA/com.android.tools.build.debugsymbols; Play reads them there.)
        log "-> android/dist/jsonquery-$version.aab"
        log "Next: android/scripts/preflight.sh"
        ;;
    check)
        # android/rust is outside the root workspace, so the desktop CI never lints it.
        in_container -- bash -c 'cargo fmt -p jsonquery_android -- --check \
            && cargo ndk -t x86_64 --platform 24 clippy -p jsonquery_android -- -D warnings \
            && ./gradlew lintDebug -PrustAbis=x86_64 -PrustProfile=ci'
        ;;
    *)
        die "unknown command: $command (apk | aab | check)"
        ;;
esac
