#!/usr/bin/env bash
# Build a self-integrating AppImage for Linux.
#
#   build/appimage.sh            # x86_64, cross-built in Docker (see linux_target)
#   build/appimage.sh aarch64    # arm64, likewise
#
# Structure/approach borrowed from a sibling project's build.sh
# (ubuntu_manager/sysmanager/build.sh): the AppRun script registers a
# .desktop file + icon into ~/.local/share on first run (keyed off the
# AppImage runtime's $APPIMAGE variable, which is stable across launches
# unlike the FUSE mount point), so the app shows up in the app menu/taskbar
# and can be pinned there without requiring appimaged or AppImageLauncher.
#
# Two things keep it working on bare systems (a container, the AppImage
# catalog's CI runner), which is where an AppImage meets the fewest assumptions:
#   - the static type2 runtime is embedded, so no libfuse2 is needed to start it;
#   - the few dlopen()ed libraries such a system may lack are bundled as a
#     fallback (fetch_fallback_libs below; AppRun prefers the host's own copy).
# packaging/appimage/ has a smoke test that runs the result in such a system.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=./common.sh
source ./common.sh

# The binary links only libc/libm/libgcc_s: X11, Wayland, xkbcommon and GL are
# dlopen()ed at run time from the host. Desktops have them all, but a minimal
# system may not, and the app then dies at startup -- the AppImage catalog's
# runner has no libxkbcommon-x11, so winit panics with "Library
# libxkbcommon-x11.so could not be loaded" before any window opens. An AppImage
# must not rely on libraries that not every target system has, so we ship the
# ones that were missing as a fallback.
#
# Only the xkbcommon family, deliberately: it is small, it is not on the
# AppImage excludelist, and it is independent of the GPU stack. libX11, libGL,
# libEGL and friends must keep coming from the host (they pair with its
# drivers) and are NOT bundled.
#
# These are Ubuntu 20.04 builds because that is the newest LTS release whose
# copies still reference nothing newer than glibc 2.17 (22.04's libxkbcommon
# needs 2.33), i.e. below this binary's own 2.18 floor, so bundling them does
# not raise the AppImage's glibc requirement. Each is pinned by sha256 (taken
# from the archive's own Packages index) so a changed or truncated download is
# caught. Columns: pool dir, package, version, sha256 amd64, sha256 arm64.
FALLBACK_DEBS=(
    "libx/libxkbcommon libxkbcommon0 0.10.0-1 b704ce1751dd938025b88a84c4054d4f36651661e473efeb3da8ed39aaf05868 3f2620fa08f83a1d139dde4012c49519a96abd58f07a92945e0f16154096bb2a"
    "libx/libxkbcommon libxkbcommon-x11-0 0.10.0-1 929d5e4ee88d6f5f3b09e238ffae60d667e4c9c4731a0210227b092fc578c6ac 087b45873e4852c438a07101f409d417d2263a2cd9c7581bd5f5d9743559e7c3"
    "libx/libxcb libxcb-xkb1 1.14-2 300c205cf611ce1cfae89fb490c7aeec297aea0ccb82ea15d9941078a09882e3 f599fd435d9a2f7296671f444fd8272ab50fb73750a58f0faef81aefcf6edd79"
)

# Unpack $FALLBACK_DEBS for $ARCH into $1, named by SONAME (libxkbcommon.so.0,
# ...). They are a set from one build: libxkbcommon-x11 shares internals with
# libxkbcommon, so pairing the bundled -x11 with a *different* libxkbcommon
# (the catalog's runner has a newer one but no -x11) segfaults. AppRun therefore
# uses the whole directory or none of it. Downloads are cached in
# build/fallback-libs/.
fetch_fallback_libs() {
    local dest="$1" cache="$ROOT_DIR/build/fallback-libs"
    local deb_arch mirror row dir pkg ver sha_amd64 sha_arm64 sha file tmp lib soname
    case "$ARCH" in
    x86_64)
        deb_arch=amd64
        mirror="https://archive.ubuntu.com/ubuntu/pool/main"
        ;;
    aarch64)
        deb_arch=arm64
        mirror="https://ports.ubuntu.com/ubuntu-ports/pool/main"
        ;;
    esac
    mkdir -p "$cache" "$dest"
    for row in "${FALLBACK_DEBS[@]}"; do
        read -r dir pkg ver sha_amd64 sha_arm64 <<<"$row"
        sha="$sha_amd64"
        [ "$deb_arch" = arm64 ] && sha="$sha_arm64"
        file="${pkg}_${ver}_${deb_arch}.deb"
        if ! echo "$sha  $cache/$file" | sha256sum -c --status 2>/dev/null; then
            echo "    downloading $file..."
            curl -fLo "$cache/$file" "$mirror/$dir/$file"
            echo "$sha  $cache/$file" | sha256sum -c --status || {
                echo "error: $file does not match its pinned sha256" >&2
                exit 1
            }
        fi
        tmp="$(mktemp -d)"
        # (the data member is xz in these 20.04 packages; newer Ubuntu uses zstd)
        ar p "$cache/$file" data.tar.xz | tar -xJ -C "$tmp"
        # The SONAME links (libxkbcommon.so.0), not the versioned files behind them
        for lib in "$tmp"/usr/lib/*/lib*.so.[0-9]; do
            soname="$(basename "$lib")"
            cp -L "$lib" "$dest/$soname"
        done
        rm -rf "$tmp"
    done
}

linux_target "${1:-x86_64}"
APPDIR="$ROOT_DIR/build/AppDir"
OUTPUT="$DIST_DIR/$APP_NAME-$VERSION-$ARCH.AppImage"

echo "==> Building $APP_NAME $VERSION for $TARGET"
"${BUILD[@]}" --release --target "$TARGET" -p jsonquery_gui

echo "==> Assembling AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/share/applications"
for size in 16 32 48 64 128 256 512; do
    mkdir -p "$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps"
    cp "$ROOT_DIR/assets/icons/icon-$size.png" \
        "$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps/$APP_NAME.png"
done
mkdir -p "$APPDIR/usr/share/pixmaps"
cp "$ROOT_DIR/assets/icons/icon-256.png" "$APPDIR/usr/share/pixmaps/$APP_NAME.png"

cp "$ROOT_DIR/target/$TARGET/release/$APP_NAME" "$APPDIR/usr/bin/$APP_NAME"

echo "==> Bundling fallback libraries"
fetch_fallback_libs "$APPDIR/usr/lib/fallback"

# AppDir root: appimagetool looks for .DirIcon (a plain file, not a symlink)
# to embed into the AppImage so file managers can show it without appimaged.
cp "$ROOT_DIR/assets/icons/icon-256.png" "$APPDIR/$APP_NAME.png"
cp "$ROOT_DIR/assets/icons/icon-256.png" "$APPDIR/.DirIcon"

cat >"$APPDIR/AppRun" <<APPRUN_EOF
#!/bin/bash
HERE="\$(dirname "\$(readlink -f "\$0")")"

# Self-integrate into the desktop app menu/taskbar on first run, and
# re-integrate if the AppImage has since moved. \$APPIMAGE is set by the
# AppImage type2 runtime to this file's own real path (unlike \$HERE, a
# throwaway FUSE mount point that differs on every launch), so it's stable
# enough to point a launcher's Exec= at.
if [ -n "\$APPIMAGE" ]; then
    DESKTOP_DST="\$HOME/.local/share/applications/$APP_NAME.desktop"
    ICON_DST="\$HOME/.local/share/icons/hicolor/256x256/apps/$APP_NAME.png"
    EXEC_LINE="Exec=\"\$APPIMAGE\""
    if [ ! -f "\$DESKTOP_DST" ] || ! grep -qF "\$EXEC_LINE" "\$DESKTOP_DST" 2>/dev/null; then
        mkdir -p "\$(dirname "\$DESKTOP_DST")" "\$(dirname "\$ICON_DST")"
        cp "\$HERE/$APP_NAME.png" "\$ICON_DST" 2>/dev/null
        sed "s|^Exec=.*|\$EXEC_LINE|" "\$HERE/$APP_NAME.desktop" > "\$DESKTOP_DST"
        command -v update-desktop-database &>/dev/null && update-desktop-database "\$HOME/.local/share/applications" &>/dev/null
        # A pre-existing icon-theme.cache (e.g. left behind by appimaged) has
        # an mtime >= the theme dir's, so GTK trusts it and never sees the
        # icon we just copied. Refresh it, or failing that bump the dir mtime
        # so the stale cache is ignored.
        HICOLOR="\$HOME/.local/share/icons/hicolor"
        if [ -f "\$HICOLOR/icon-theme.cache" ]; then
            gtk-update-icon-cache -f -t --ignore-theme-index "\$HICOLOR" &>/dev/null || touch "\$HICOLOR"
        fi
    fi
fi

# Libraries the app dlopen()s that a minimal system may lack are bundled in
# usr/lib/fallback/. The host's own copies always win: the directory goes on
# the search path only if the host is missing at least one of them, so a system
# that already works never loads any. It is all or nothing because the
# libraries are one matched set (libxkbcommon-x11 must pair with the
# libxkbcommon of the same build; mixing it with the host's other version
# crashes). If the host's libraries can't be listed (no ldconfig) the bundled
# set is used rather than risk the app failing to start.
FALLBACK="\$HERE/usr/lib/fallback"
if [ -d "\$FALLBACK" ]; then
    HOST_LIBS="\$({ /sbin/ldconfig -p || /usr/sbin/ldconfig -p || ldconfig -p; } 2>/dev/null)"
    for LIB in "\$FALLBACK"/*.so.*; do
        if ! grep -qE "(^|[[:space:]])\$(basename "\$LIB") " <<<"\$HOST_LIBS"; then
            export LD_LIBRARY_PATH="\$FALLBACK\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
            break
        fi
    done
fi

exec "\$HERE/usr/bin/$APP_NAME" "\$@"
APPRUN_EOF
chmod +x "$APPDIR/AppRun"

# .desktop entry (must live at AppDir root AND usr/share/applications/).
# StartupWMClass matches APP_ID set via ViewportBuilder::with_app_id() in
# main.rs, so the WM associates the running window with this launcher —
# without that match, "pin to taskbar" after launch doesn't stick.
cat >"$APPDIR/$APP_NAME.desktop" <<DESKTOP_EOF
[Desktop Entry]
Type=Application
Name=jsonquery
GenericName=JSON Query Tool
Comment=Browse and query large JSON files with jq-compatible queries
Exec=$APP_NAME
Icon=$APP_NAME
Categories=Development;Utility;
Terminal=false
StartupWMClass=jsonquery_gui
X-AppImage-Version=$VERSION
DESKTOP_EOF
cp "$APPDIR/$APP_NAME.desktop" "$APPDIR/usr/share/applications/$APP_NAME.desktop"

echo "==> Locating appimagetool"
# appimagetool itself has to run on this host, whatever arch the AppImage is for.
HOST_ARCH="$(uname -m)"
if command -v appimagetool &>/dev/null; then
    APPIMAGETOOL="appimagetool"
else
    TOOL_PATH="$ROOT_DIR/build/appimagetool-$HOST_ARCH.AppImage"
    if [ ! -f "$TOOL_PATH" ]; then
        echo "    downloading appimagetool..."
        curl -Lo "$TOOL_PATH" \
            "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-$HOST_ARCH.AppImage"
        chmod +x "$TOOL_PATH"
    fi
    APPIMAGETOOL="$TOOL_PATH"
fi

# Always embed the maintained static type2 runtime, for the host's arch too.
# The runtime appimagetool carries by default is the old one, linked
# dynamically against the host's libc and needing libfuse2 -- which Ubuntu
# 24.04 and other current distros no longer install by default, so the
# AppImage would not even start there (the AppImage catalog flags it as an
# "old AppImage runtime"). The static one needs neither.
RUNTIME_PATH="$ROOT_DIR/build/runtime-$ARCH"
if [ ! -f "$RUNTIME_PATH" ]; then
    echo "    downloading the $ARCH AppImage runtime..."
    curl -fLo "$RUNTIME_PATH" \
        "https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-$ARCH"
fi
RUNTIME_ARGS=(--runtime-file "$RUNTIME_PATH")

echo "==> Building AppImage"
rm -f "$OUTPUT"
ARCH=$ARCH "$APPIMAGETOOL" "${RUNTIME_ARGS[@]}" "$APPDIR" "$OUTPUT"

echo "==> Wrote $OUTPUT"
