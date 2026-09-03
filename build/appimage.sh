#!/usr/bin/env bash
# Build a self-integrating AppImage for Linux x86_64.
#
# Structure/approach borrowed from a sibling project's build.sh
# (ubuntu_manager/sysmanager/build.sh): the AppRun script registers a
# .desktop file + icon into ~/.local/share on first run (keyed off the
# AppImage runtime's $APPIMAGE variable, which is stable across launches
# unlike the FUSE mount point), so the app shows up in the app menu/taskbar
# and can be pinned there without requiring appimaged or AppImageLauncher.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
# shellcheck source=./common.sh
source ./common.sh

TARGET="x86_64-unknown-linux-gnu"
ARCH="x86_64"
APPDIR="$ROOT_DIR/build/AppDir"
OUTPUT="$DIST_DIR/$APP_NAME-$VERSION-$ARCH.AppImage"

echo "==> Building $APP_NAME $VERSION for $TARGET"
cargo build --release --target "$TARGET" -p jsonquery

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
StartupWMClass=jsonquery
X-AppImage-Version=$VERSION
DESKTOP_EOF
cp "$APPDIR/$APP_NAME.desktop" "$APPDIR/usr/share/applications/$APP_NAME.desktop"

echo "==> Locating appimagetool"
if command -v appimagetool &>/dev/null; then
    APPIMAGETOOL="appimagetool"
else
    TOOL_PATH="$ROOT_DIR/build/appimagetool-$ARCH.AppImage"
    if [ ! -f "$TOOL_PATH" ]; then
        echo "    downloading appimagetool..."
        curl -Lo "$TOOL_PATH" \
            "https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage"
        chmod +x "$TOOL_PATH"
    fi
    APPIMAGETOOL="$TOOL_PATH"
fi

echo "==> Building AppImage"
rm -f "$OUTPUT"
ARCH=$ARCH "$APPIMAGETOOL" "$APPDIR" "$OUTPUT"

echo "==> Wrote $OUTPUT"
