#!/bin/bash
# Runs INSIDE the catalog-smoke image (see smoke.sh). Mirrors what the AppImage
# catalog's code/worker.sh does once it has an AppImage: Xvfb at 800x600x24,
# start it, give it at least 10 s, poll up to 20 s more for a window, take a
# screenshot.
#
#   catalog-smoke-run LAUNCH OUTDIR [bundled|host]
#
# LAUNCH is an extracted AppDir (its AppRun is started) or an .AppImage file
# (started directly; the image has no FUSE, so set APPIMAGE_EXTRACT_AND_RUN=1).
# For an AppDir, the optional third argument also checks where the app got its
# xkbcommon libraries from: "bundled" (the AppImage's usr/lib/fallback) or
# "host" (the system's own).
set -u
LAUNCH="$1"
OUT="$2"
EXPECT="${3:-}"
export DISPLAY=:99
mkdir -p "$OUT"

Xvfb :99 -screen 0 800x600x24 >/dev/null 2>&1 &
sleep 1

if [ -d "$LAUNCH" ]; then CMD="$LAUNCH/AppRun"; else CMD="$LAUNCH"; fi
"$CMD" >"$OUT/app.log" 2>&1 &
APID=$!
sleep 10
WAIT=0
for WAIT in $(seq 1 20); do
  kill -0 $APID 2>/dev/null || break
  grep -qE '0x.*": \(' <<<"$(timeout 5 xwininfo -tree -root 2>/dev/null || true)" && break
  sleep 1
done
[ "$WAIT" -gt 1 ] && sleep 2

FAIL=""
if ! kill -0 $APID 2>/dev/null; then
  FAIL="the application exited within $((10 + WAIT)) seconds instead of showing a window"
elif ! grep -qE '0x.*": \(' <<<"$(timeout 5 xwininfo -tree -root 2>/dev/null || true)"; then
  FAIL="no window appeared on screen"
fi

# AppRun execs the binary, so $APID is the app and its maps say which copies
# of the libraries it really loaded
if [ -z "$FAIL" ] && [ -d "$LAUNCH" ] && [ -n "$EXPECT" ]; then
  BUNDLED=$(grep -c '/usr/lib/fallback/' /proc/$APID/maps || true)
  case "$EXPECT" in
  bundled) [ "$BUNDLED" -gt 0 ] || FAIL="expected the bundled xkbcommon libraries, but the system's were used" ;;
  host) [ "$BUNDLED" -eq 0 ] || FAIL="expected the system's xkbcommon libraries, but the bundled ones were used" ;;
  esac
fi

if [ -z "$FAIL" ]; then
  icewm >/dev/null 2>&1 &
  sleep 2
  timeout 30 import -window root "$OUT/screenshot.png" 2>/dev/null || true
  # The catalog rejects an empty window; a rendered UI has far more than a few colors
  COLORS=$(identify -format '%k' "$OUT/screenshot.png" 2>/dev/null || echo 0)
  [ "$COLORS" -gt 20 ] || FAIL="the screenshot looks blank ($COLORS colors)"
fi
kill $APID 2>/dev/null || true

if [ -n "$FAIL" ]; then
  echo "FAIL: $FAIL"
  echo "--- application output:"
  cat "$OUT/app.log"
  exit 1
fi
echo "PASS"
