#!/usr/bin/env bash
# Open the Android app in an emulator window you can use by hand.
#
#   android/scripts/run-phone.sh  [options] [file.json ...]   a Pixel 3 phone, upright
#   android/scripts/run-tablet.sh [options] [file.json ...]   a Pixel Tablet, landscape
#
# It builds the debug APK if the sources are newer than it, starts the emulator
# in a container (a first boot takes a minute or two), installs the app, puts
# sample JSON files in the Downloads folder (and any file you name), and opens
# the app. The window appears on your desktop: a click is a tap, click-and-drag
# swipes, holding the button is a long press, and your keyboard types into the
# app. The strip beside the window rotates the device and has Back and Home.
#
# Run it again after changing code: it rebuilds, updates the app inside the
# running emulator (its saved data is kept) and reopens it.
#
# Options:
#   --build      rebuild the APK even if it is up to date
#   --no-build   use the APK as it is
#   --fresh      throw the emulator away and boot a clean one
#   --logs       follow the app's log (Ctrl-C to stop)
#   --stop       close this emulator (closing its window does the same)
#   --stop-all   close the phone and the tablet
#
# Needs Docker, /dev/kvm and a desktop session (an X display; a Wayland desktop
# has one through XWayland). Nothing else is installed on the host. The
# emulator draws with software OpenGL, so it is slower than a real device.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

PACKAGE=io.github.nujufas.jsonquery.debug
ACTIVITY=io.github.nujufas.jsonquery.MainActivity
FIXTURES="$ANDROID_DIR/test/resources/fixtures"

usage() { sed -n '2,27p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

device="${1:-}"
case "$device" in
    phone|tablet) shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "usage: run-emulator.sh phone|tablet [options] (or run-phone.sh / run-tablet.sh)" ;;
esac
# One container per device, apart from test/run.sh's headless ones.
NAME="jsonquery-android-live-$device"

running() { docker ps --format '{{.Names}}' | grep -qx "$NAME"; }
# Removes a device's container if there is one, running or exited (its window was
# closed). `docker rm -f` alone succeeds for a container that does not exist.
close_emulator() {
    docker ps -a --format '{{.Names}}' | grep -qx "jsonquery-android-live-$1" || return 1
    docker rm -f "jsonquery-android-live-$1" >/dev/null
}
# adb, inside the container, to its one emulator.
dadb() { docker exec "$NAME" adb -e "$@"; }

build=auto fresh=0 logs=0
files=()
while [ $# -gt 0 ]; do
    case "$1" in
        --build) build=yes; shift ;;
        --no-build) build=no; shift ;;
        --fresh) fresh=1; shift ;;
        --logs) logs=1; shift ;;
        --stop)
            if close_emulator "$device"; then
                log "closed the $device emulator"
            else
                log "the $device emulator was not running"
            fi
            exit 0 ;;
        --stop-all)
            for d in phone tablet; do
                if close_emulator "$d"; then log "closed the $d emulator"; fi
            done
            exit 0 ;;
        -h|--help) usage; exit 0 ;;
        -*) die "unknown option: $1 (see --help)" ;;
        *) [ -f "$1" ] || die "no such file: $1"; files+=("$(realpath "$1")"); shift ;;
    esac
done

if [ "$logs" = 1 ]; then
    running || die "the $device emulator is not running; start it with run-$device.sh"
    pid="$(dadb shell pidof -s "$PACKAGE" | tr -d '\r')"
    [ -n "$pid" ] || die "the app is not running in the $device emulator"
    exec docker exec -t "$NAME" adb -e logcat -v time --pid="$pid"
fi

# --- What the emulator needs from the host -------------------------------------
[ -w /dev/kvm ] || die "/dev/kvm is missing or not writable by you; the emulator needs hardware virtualisation (are you in the 'kvm' group?)"
case "${DISPLAY:-}" in
    :*) ;;
    *) die "no local X display (DISPLAY='${DISPLAY:-}'); run this from a terminal in your desktop session" ;;
esac
dnum="${DISPLAY#:}"; dnum="${dnum%%.*}"
[ -S "/tmp/.X11-unix/X$dnum" ] || die "the X display $DISPLAY has no socket at /tmp/.X11-unix/X$dnum"
# The emulator's window is drawn on your X display: hand the container the
# display's socket and your login cookie (read-only). The hostname matches the
# cookie's, which some sessions key it on.
x_args=(-e "DISPLAY=$DISPLAY" -e QT_X11_NO_MITSHM=1 -v /tmp/.X11-unix:/tmp/.X11-unix)
xauth="${XAUTHORITY:-$HOME/.Xauthority}"
[ -f "$xauth" ] && x_args+=(-e XAUTHORITY=/tmp/.host-xauthority -v "$xauth":/tmp/.host-xauthority:ro)

# --- 1. The APK ----------------------------------------------------------------
version="$(workspace_version)"
apk="$ANDROID_DIR/dist/jsonquery-$version-debug.apk"
sources=("$REPO_ROOT/crates" "$REPO_ROOT/assets" "$REPO_ROOT/Cargo.toml" "$REPO_ROOT/Cargo.lock"
    "$ANDROID_DIR/rust" "$ANDROID_DIR/app/src" "$ANDROID_DIR/Cargo.toml" "$ANDROID_DIR/Cargo.lock"
    "$ANDROID_DIR/version.properties" "$ANDROID_DIR/build.gradle.kts" "$ANDROID_DIR/app/build.gradle.kts"
    "$ANDROID_DIR/settings.gradle.kts" "$ANDROID_DIR/gradle.properties")
case "$build" in
    yes) rebuild=1 ;;
    no)  [ -f "$apk" ] || die "no APK at android/dist/${apk##*/}; run without --no-build"; rebuild=0 ;;
    *)   rebuild=0
         if [ ! -f "$apk" ] || [ -n "$(find "${sources[@]}" -type f -newer "$apk" -print -quit 2>/dev/null)" ]; then
             rebuild=1
         fi ;;
esac
if [ "$rebuild" = 1 ]; then
    "$ANDROID_DIR/scripts/build.sh" apk
else
    log "the APK is up to date (--build forces a rebuild)"
fi

# --- 2. The emulator -----------------------------------------------------------
ensure_emulator_image
[ "$fresh" = 1 ] && docker rm -f "$NAME" >/dev/null 2>&1 || true
started=0
if ! running; then
    docker rm -f "$NAME" >/dev/null 2>&1 || true   # one that has exited (its window was closed)
    log "Starting the $device emulator; its window opens on your screen (a first boot takes a minute or two)..."
    docker run -d --name "$NAME" --device /dev/kvm --hostname "$(hostname)" \
        -e AVD="$device" -e WINDOW=1 "${x_args[@]}" \
        -v "$REPO_ROOT":/work \
        "$EMU_IMAGE" /work/android/test/emulator-entry.sh >/dev/null
    started=1
fi

waited=0
until [ "$(dadb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ]; do
    running || { docker logs --tail 30 "$NAME" >&2; die "the emulator stopped before it finished booting (its log is above)"; }
    [ "$waited" -lt 420 ] || die "the emulator did not finish booting in 7 minutes"
    sleep 3; waited=$((waited + 3))
    [ $((waited % 30)) -ne 0 ] || log "  still booting (${waited}s)..."
done
# Let the launcher settle, so it does not steal the app's first launch.
[ "$started" = 1 ] && sleep 5 || true

# --- 3. The app and something to open with it ---------------------------------
log "Installing the app..."
in_emu_apk="/work/android/dist/${apk##*/}"
if ! out="$(dadb install -r -t "$in_emu_apk" 2>&1)"; then
    # Most likely signed with a different debug key: replace it (this drops the app's saved data).
    log "update-install failed (${out##*$'\n'}); reinstalling"
    dadb uninstall "$PACKAGE" >/dev/null 2>&1 || true
    dadb install -t "$in_emu_apk" >/dev/null || die "could not install the app"
fi

# Into the Downloads folder, where the file picker ("Open File...") looks first.
# (The media scan is what makes the picker's "Recent" list show them at once.)
push_download() {   # <path in the container> <name on the device>
    dadb push "$1" "/sdcard/Download/$2" >/dev/null 2>&1 || die "could not copy $2 to the emulator"
    dadb shell am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE \
        -d "file:///sdcard/Download/$2" >/dev/null 2>&1 || true
}
for f in "$FIXTURES"/*.json; do
    push_download "/work/android/test/resources/fixtures/${f##*/}" "${f##*/}"
done
if [ "${#files[@]}" -gt 0 ]; then
    docker exec "$NAME" mkdir -p /tmp/push
    for f in "${files[@]}"; do
        name="${f##*/}"; name="${name// /_}"
        docker cp "$f" "$NAME:/tmp/push/$name"
        push_download "/tmp/push/$name" "$name"
        log "  Downloads/$name"
    done
fi

dadb shell am force-stop "$PACKAGE"
dadb shell am start -W -n "$PACKAGE/$ACTIVITY" >/dev/null

script="android/scripts/run-$device.sh"
log "The $device emulator is open, with the app running."
log "  click = tap, drag = swipe, hold = long press; your keyboard types into it"
log "  files: Open File... > Downloads has the sample JSON${files[0]:+ and yours}"
log "  changed the code?  run $script again (it rebuilds and reopens the app)"
log "  $script --logs   follow the app's log      --stop   close the emulator"
