#!/usr/bin/env bash
# Runs inside the emulator container (started by run.sh): wait for the device,
# find the APK, run Robot Framework, give the results back to the host user.
set -uo pipefail
cd /work/android/test

apk="$(ls -t /work/android/dist/*-debug.apk 2>/dev/null | head -1)"
[ -n "$apk" ] || { echo "no debug APK in android/dist; run android/scripts/build.sh apk" >&2; exit 2; }

# The store-screenshot suite is not a test: only run it when asked for.
case " $* " in *screenshots*) exclude=() ;; *) exclude=(--exclude screenshots) ;; esac

robot --outputdir results --variable "APK:$apk" --variable "DEVICE:${DEVICE:-phone}" "${exclude[@]}" "$@"
status=$?
chown -R "${HOST_UID:-0}:${HOST_GID:-0}" results /work/android/play 2>/dev/null || true
exit $status
