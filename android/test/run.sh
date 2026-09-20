#!/usr/bin/env bash
# Run the Android Robot Framework suites against a headless emulator in Docker.
#
#   android/test/run.sh [options] [robot arguments | suite paths]
#
#   android/test/run.sh                          # every phone suite
#   android/test/run.sh --device tablet          # every tablet suite (suites-tablet/)
#   android/test/run.sh suites/opening           # one suite (a directory or .robot file)
#   android/test/run.sh --test 'TC-AND-0*'       # any robot option
#   android/test/run.sh --build                  # rebuild the debug APK first
#   android/test/run.sh --fresh                  # restart the emulator (clean device state)
#   android/test/run.sh --stop                   # shut the emulator containers down
#
# The emulator (Android 16, 16 KB pages; --device phone is a Pixel 3, --device
# tablet a Pixel Tablet) boots once, in a container that stays up between runs,
# so repeat runs start in seconds. It needs KVM (/dev/kvm). Results:
# android/test/results/ (log.html has a screenshot of every step). Nothing but
# Docker is used on the host.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/../scripts/lib.sh"

TEST_DIR="$ANDROID_DIR/test"

build=0 fresh=0 device=phone
while [ $# -gt 0 ]; do
    case "$1" in
        --build) build=1; shift ;;
        --fresh) fresh=1; shift ;;
        --device) [ $# -ge 2 ] || die "--device needs phone or tablet"; device="$2"; shift 2 ;;
        --stop)
            stopped=0
            for name in jsonquery-android-emu jsonquery-android-emu-tablet; do
                if docker ps -a --format '{{.Names}}' | grep -qx "$name"; then
                    docker rm -f "$name" >/dev/null && { log "stopped $name"; stopped=1; }
                fi
            done
            [ "$stopped" = 1 ] || log "no emulator was running"
            exit 0 ;;
        -h|--help) sed -n '2,18p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) break ;;
    esac
done

# One container per device profile (both can be up at once; each is a few GB of RAM).
case "$device" in
    phone)  EMU=jsonquery-android-emu;        default_suites=suites ;;
    tablet) EMU=jsonquery-android-emu-tablet; default_suites=suites-tablet ;;
    *) die "unknown --device '$device' (phone or tablet)" ;;
esac

[ -w /dev/kvm ] || die "/dev/kvm is missing or not writable by you; the emulator needs hardware virtualisation (are you in the 'kvm' group?)"

# 1. The APK under test.
version="$(workspace_version)"
apk="$ANDROID_DIR/dist/jsonquery-$version-debug.apk"
if [ "$build" = 1 ] || [ ! -f "$apk" ]; then
    "$ANDROID_DIR/scripts/build.sh" apk
fi

# 2. The emulator image, once.
ensure_emulator_image

# 3. The emulator container, kept up between runs.
[ "$fresh" = 1 ] && docker rm -f "$EMU" >/dev/null 2>&1 || true
if ! docker ps --format '{{.Names}}' | grep -qx "$EMU"; then
    docker rm -f "$EMU" >/dev/null 2>&1 || true
    log "Starting the $device emulator (first boot takes a minute or two)..."
    docker run -d --name "$EMU" --device /dev/kvm -e AVD="$device" \
        -v "$REPO_ROOT":/work \
        "$EMU_IMAGE" /work/android/test/emulator-entry.sh >/dev/null
fi

# 4. Run the suites inside it.
args=("$@")
has_target=0
for a in "${args[@]}"; do
    case "$a" in suites*|*.robot) has_target=1 ;; esac
done
[ "$has_target" = 1 ] || args+=("$default_suites")

mkdir -p "$TEST_DIR/results"
set +e
docker exec -t -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" -e DEVICE="$device" -w /work/android/test \
    "$EMU" /work/android/test/in-emulator.sh "${args[@]}"
status=$?
set -e
log "Results: android/test/results/log.html"
exit $status
