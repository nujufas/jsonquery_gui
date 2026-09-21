#!/bin/bash
# Runs inside the smoke container. Xvfb (amd64) serves X over loopback TCP; the
# arm64 app runs in the arm64 sysroot chroot under qemu-user and connects to it.
set -u
LIMIT=${LIMIT:-540}

mkdir -p /sysroot/dev /sysroot/tmp /sysroot/root && chmod 1777 /sysroot/tmp
mknod -m 666 /sysroot/dev/null c 1 3 2>/dev/null || true
cp /usr/bin/qemu-aarch64-static /sysroot/usr/bin/

Xvfb :99 -screen 0 1280x800x24 -ac -listen tcp > /out/xvfb.log 2>&1 &
sleep 3
export DISPLAY=127.0.0.1:99

env -i HOME=/root PATH=/usr/sbin:/usr/bin:/bin DISPLAY=127.0.0.1:99 \
    LIBGL_ALWAYS_SOFTWARE=1 RUST_BACKTRACE=1 RUST_LOG=warn \
    chroot /sysroot /usr/bin/qemu-aarch64-static /app/jsonquery_gui > /out/app.log 2>&1 &
APP=$!
echo "app pid $APP (chroot/qemu), started $(date +%T)"

start=$(date +%s)
mapped=""
while kill -0 "$APP" 2>/dev/null; do
    el=$(( $(date +%s) - start ))
    [ "$el" -gt "$LIMIT" ] && { echo "TIMEOUT after ${el}s with no mapped window"; break; }
    # first named top-level client window
    win=$(xwininfo -root -tree 2>/dev/null | grep -E '^\s+0x[0-9a-f]+ "[^"]+":' | head -1)
    if [ -n "$win" ]; then
        id=$(echo "$win" | awk '{print $1}')
        if xwininfo -id "$id" 2>/dev/null | grep -q "Map State: IsViewable"; then
            mapped="$win"
            echo "WINDOW MAPPED after ${el}s: $win"
            break
        fi
    fi
    sleep 5
done

if [ -n "$mapped" ]; then
    sleep 25   # let a few emulated frames draw
    import -display "$DISPLAY" -window root /out/gui.png 2>>/out/import.log && echo "screenshot written"
    xwininfo -root -tree 2>/dev/null | grep -E '^\s+0x[0-9a-f]+ "' | head -5
fi

if kill -0 "$APP" 2>/dev/null; then echo "app still running -> alive"; else wait "$APP"; echo "app exited: $?"; fi
kill "$APP" 2>/dev/null
echo "--- app.log (first 25 lines):"; head -25 /out/app.log
