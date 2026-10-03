#!/usr/bin/env bash
# Smoke-test the x86_64 AppImage the way the AppImage catalog's CI sees it: in a
# bare Ubuntu 22.04 system with Xvfb and software GL, run as an ordinary user
# without network. Needs Docker. See packaging/appimage/README.md.
#
#   packaging/appimage/catalog-smoke/smoke.sh [path/to/x86_64.AppImage]
#
# Defaults to dist/jsonquery_gui-<version>-x86_64.AppImage. Results, logs and
# screenshots go to dist/appimage-smoke/. Everything runs in containers; nothing
# touches the host's display. It also checks that the AppImage's update
# information is in order, including the .zsync file that has to sit next to it.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"
APPIMAGE="${1:-$ROOT/dist/jsonquery_gui-$VERSION-x86_64.AppImage}"
OUT="$ROOT/dist/appimage-smoke"

# The binary's own glibc floor. The bundled libraries must not raise it.
GLIBC_FLOOR=2.18

if [ ! -f "$APPIMAGE" ]; then
    echo "error: $APPIMAGE not found (build it with build/appimage.sh)" >&2
    exit 1
fi
APPIMAGE="$(realpath "$APPIMAGE")"

FAILED=0
ok() { printf 'PASS  %s\n' "$1"; }
bad() {
    printf 'FAIL  %s\n' "$1"
    FAILED=1
}

rm -rf "$OUT"
mkdir -p "$OUT"
echo "==> Static checks on $(basename "$APPIMAGE")"

# What the catalog calls a "dynamic" runtime: it has a program interpreter or
# needs shared libraries, i.e. it depends on the host's libc (and libfuse2).
if readelf -lW "$APPIMAGE" | grep -q 'Requesting program interpreter' ||
    readelf -dW "$APPIMAGE" 2>/dev/null | grep -q '(NEEDED)'; then
    bad "the runtime is dynamically linked (needs the host's libc and libfuse2)"
else
    ok "the runtime is static"
fi

# --appimage-extract only unpacks the payload; it never starts the app
(cd "$OUT" && "$APPIMAGE" --appimage-extract >/dev/null)
APPDIR="$OUT/squashfs-root"

floor="$(find "$APPDIR/usr" -type f \( -path '*/bin/*' -o -name '*.so*' \) -print0 |
    xargs -0 -r readelf --dyn-syms -W 2>/dev/null |
    grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' | sed 's/GLIBC_//' | sort -uV | tail -n 1 || true)"
if [ -n "$floor" ] && [ "$(printf '%s\n%s\n' "$floor" "$GLIBC_FLOOR" | sort -V | tail -n 1)" = "$GLIBC_FLOOR" ]; then
    ok "glibc floor is $floor (at most $GLIBC_FLOOR)"
else
    bad "glibc floor is '${floor:-unknown}', above the binary's own $GLIBC_FLOOR"
fi

# Update information, read back the way the catalog does: the runtime prints the
# AppImage's .upd_info section and exits without starting the app. Updaters then
# look for the .zsync file named by its pattern on the release, so that has to
# exist, describe this exact file, and match the pattern.
upd="$("$APPIMAGE" --appimage-updateinformation 2>/dev/null | tr -d '\0' |
    sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' || true)"
# Print the value of header $2 of the .zsync file $1: "Key: value" lines up to
# the first blank line, followed by binary block checksums.
zsync_header() { LC_ALL=C sed -n "/^\$/q;s/^$2: //p" "$1"; }
if [[ "$upd" != gh-releases-zsync\|* ]]; then
    bad "no gh-releases-zsync update information is embedded (got '${upd:-nothing}')"
else
    ok "update information is embedded: $upd"
    zsync="$APPIMAGE.zsync"
    if [ ! -f "$zsync" ]; then
        bad "$(basename "$zsync") is missing next to the AppImage (updaters fetch it from the release)"
    else
        if [ "$(zsync_header "$zsync" SHA-1)" = "$(sha1sum "$APPIMAGE" | cut -d' ' -f1)" ] &&
            [ "$(zsync_header "$zsync" Length)" = "$(stat -c %s "$APPIMAGE")" ] &&
            [ "$(zsync_header "$zsync" URL)" = "$(basename "$APPIMAGE")" ]; then
            ok "$(basename "$zsync") describes this AppImage (SHA-1, length and URL)"
        else
            bad "$(basename "$zsync") does not describe this AppImage; rebuild both together"
        fi
        IFS='|' read -r _ _ _ _ pattern <<<"$upd"
        # shellcheck disable=SC2053 # $pattern is a glob on purpose
        if [[ "$(basename "$zsync")" == $pattern ]]; then
            ok "the update information's pattern ($pattern) matches $(basename "$zsync")"
        else
            bad "the update information's pattern ($pattern) does not match $(basename "$zsync")"
        fi
    fi
fi

echo "==> Running it in a bare Ubuntu 22.04 system (the first run builds the images)"
for host in ci desktop bare; do
    docker build -q --build-arg HOST="$host" -t "jsonquery-appimage-smoke:$host" "$HERE" >/dev/null
done

# run ID DESCRIPTION HOST LAUNCH EXPECT [docker args...]
# (EXPECT is "bundled", "host", or "" for no check; see run.sh)
run() {
    local id="$1" desc="$2" host="$3" launch="$4" expect="$5" log
    local args=("$launch" /out)
    shift 5
    [ -z "$expect" ] || args+=("$expect")
    mkdir -p "$OUT/$id"
    if log="$(docker run --rm --network none --user "$(id -u):$(id -g)" -e HOME=/tmp/home \
        -v "$APPDIR:/appdir:ro" -v "$APPIMAGE:/app.AppImage:ro" -v "$OUT/$id:/out" \
        "$@" "jsonquery-appimage-smoke:$host" catalog-smoke-run "${args[@]}" 2>&1)"; then
        ok "$desc"
    else
        bad "$desc"
        printf '        %s\n' "${log//$'\n'/$'\n'        }"
    fi
}

run ci "catalog's runner (libxkbcommon0 only): starts on the bundled libraries" ci /appdir bundled
run desktop "ordinary desktop (all xkbcommon libraries): keeps using the system's" desktop /appdir host
run bare "bare system (no xkbcommon at all): starts on the bundled libraries" bare /appdir bundled
run appimage "the .AppImage file itself, through its embedded runtime (no FUSE)" ci /app.AppImage "" \
    -e APPIMAGE_EXTRACT_AND_RUN=1

echo
if [ "$FAILED" -eq 0 ]; then
    echo "All checks passed. Screenshots: $OUT/*/screenshot.png"
else
    echo "Some checks failed; logs and screenshots are in $OUT" >&2
    exit 1
fi
