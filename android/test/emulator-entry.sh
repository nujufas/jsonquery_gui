#!/usr/bin/env bash
# PID 1 of the emulator container: boot an AVD (headless unless WINDOW=1) and stay up.
#
#   AVD=phone    (default) the Pixel 3 that is baked into the image
#   AVD=tablet   a Pixel Tablet (2560x1600, 320 dpi = 1280x800 dp), made here
#                from the same system image on first start
#
# Other devices are made here, not in the image: one costs a few KB, where a
# new line in the Dockerfile means a rebuild of a 9 GB image.
set -euo pipefail

AVD="${AVD:-phone}"
SYSTEM_IMAGE="system-images;android-36;google_apis_ps16k;x86_64"

# `key=value` in the AVD's config.ini, replacing any earlier line for the key
# (avdmanager writes its own; the emulator does not promise which one wins).
set_avd_option() {
    local file="$ANDROID_AVD_HOME/$AVD.avd/config.ini" key="$1" value="$2"
    sed -i "/^${key//./\\.}=/d" "$file"
    echo "$key=$value" >> "$file"
}

if [ ! -d "$ANDROID_AVD_HOME/$AVD.avd" ]; then
    case "$AVD" in
        tablet) device=pixel_tablet ;;
        *) echo "unknown AVD profile '$AVD' (phone, tablet)" >&2; exit 2 ;;
    esac
    echo no | avdmanager create avd --force --name "$AVD" --package "$SYSTEM_IMAGE" --device "$device"
    set_avd_option hw.gpu.enabled yes
    set_avd_option hw.gpu.mode swiftshader_indirect
    set_avd_option hw.ramSize 4096
    set_avd_option hw.keyboard yes
    set_avd_option showDeviceFrame no
    set_avd_option disk.dataPartition.size 4G
fi

adb start-server
# The test runs are headless. WINDOW=1 (android/scripts/run-phone.sh and
# run-tablet.sh) opens the emulator's own window on the X display the container
# was given, so a person can use it.
window=(-no-window)
[ "${WINDOW:-0}" = 1 ] && window=()
# swiftshader_indirect: software OpenGL ES, so no GPU is needed.
exec emulator -avd "$AVD" "${window[@]}" -no-audio -no-boot-anim -no-snapshot -no-metrics \
    -gpu swiftshader_indirect -accel on -camera-back none -camera-front none
