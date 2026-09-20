#!/usr/bin/env bash
# Run a disposable Windows 11 in Docker so the winget manifest can be tested on a
# real Windows. Uses https://github.com/dockur/windows (QEMU/KVM inside a container,
# browser viewer included), so nothing but the image lands on the host.
#
#   windows-vm.sh up [--dry-run]   create/start the VM (first boot downloads the ISO, ~7 GB,
#                                  and installs Windows unattended - roughly 20-30 minutes)
#   windows-vm.sh status           container state, guest address, does Windows answer on RDP
#   windows-vm.sh screenshot [f]   save the VM's screen to a PNG (default ./winvm-screen.png)
#   windows-vm.sh stage [CHANNEL] [--refresh]
#                                  copy what a channel's Windows test needs into the share:
#                                    winget (default)  the manifests, test script and a current winget
#                                                      (--refresh re-downloads winget)
#                                    scoop             the Scoop manifest and its test script
#                                    choco             the .nupkg from dist/chocolatey (packaging/
#                                                      chocolatey/dev/choco.sh pack) and its test script
#                                    msix              the test-signed .msix + .cer from dist/msix
#                                                      (packaging/msix/README.md) and its test script
#   windows-vm.sh results [CHANNEL] print the last result a test script wrote (winget|scoop|choco|msix;
#                                  no channel = whichever test ran last)
#   windows-vm.sh stop             shut Windows down cleanly (`up` resumes it)
#   windows-vm.sh down             remove the container (the VM disk in $WINVM_DIR/storage stays)
#
# Environment (defaults in brackets):
#   WINVM_DIR [~/windows-vm]   storage/ = the VM disk, shared/ = the Windows-side "Data" share
#   WINVM_NAME [windows-winget-test]   WINVM_VERSION [11e = Windows 11 Enterprise Evaluation]
#   WINVM_RAM [8G]  WINVM_CORES [4]  WINVM_DISK [64G, sparse]  WINVM_IMAGE [docker.io/dockurr/windows]
#   WINVM_WEB_PORT [8006]  WINVM_RDP_PORT [3389]     (both published on 127.0.0.1 only)
#   WINVM_WINGET_TAG [latest]  a winget-cli release tag to stage instead, e.g. v1.29.290
#
# Needs docker and read/write access to /dev/kvm (group `kvm`). See README.md here.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WINGET_DIR="$(cd "$TOOLS_DIR/.." && pwd)" # packaging/winget: the manifests live here
PKG_ROOT="$(cd "$WINGET_DIR/.." && pwd)"  # packaging/: scoop/ and chocolatey/ live beside winget/
REPO_ROOT="$(cd "$PKG_ROOT/.." && pwd)"

NAME="${WINVM_NAME:-windows-winget-test}"
DIR="${WINVM_DIR:-$HOME/windows-vm}"
VERSION="${WINVM_VERSION:-11e}"
RAM="${WINVM_RAM:-8G}"
CORES="${WINVM_CORES:-4}"
DISK="${WINVM_DISK:-64G}"
IMAGE="${WINVM_IMAGE:-docker.io/dockurr/windows}"
WEB_PORT="${WINVM_WEB_PORT:-8006}"
RDP_PORT="${WINVM_RDP_PORT:-3389}"

say() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
usage() { awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "${BASH_SOURCE[0]}"; }
strip_ansi() { sed 's/\x1b\[[0-9;]*m//g'; }
to_crlf() { sed -e 's/\r$//' -e 's/$/\r/' "$1" >"$2"; } # Windows-side files want CRLF

container_exists() { docker ps -a --format '{{.Names}}' | grep -qx "$NAME"; }
container_running() { docker ps --format '{{.Names}}' | grep -qx "$NAME"; }
need_running() { container_running || die "container '$NAME' is not running (start it with: $0 up)"; }
show_urls() {
    echo "Windows viewer: http://127.0.0.1:$WEB_PORT   (RDP: 127.0.0.1:$RDP_PORT, user Docker, password admin)"
}
# dockur logs "Guest: 172.30.0.2 (Windows) | Mode: NAT ..." once the network is set up.
guest_ip() {
    docker logs "$NAME" 2>&1 | strip_ansi | grep -o 'Guest: [0-9.]*' | tail -1 | cut -d' ' -f2 || true
}

cmd_up() {
    local dry=0
    [ "${1:-}" = "--dry-run" ] && dry=1

    local run=(docker run -d --name "$NAME"
        -e "VERSION=$VERSION" -e "RAM_SIZE=$RAM" -e "CPU_CORES=$CORES" -e "DISK_SIZE=$DISK"
        -p "127.0.0.1:$WEB_PORT:8006" -p "127.0.0.1:$RDP_PORT:3389/tcp" -p "127.0.0.1:$RDP_PORT:3389/udp"
        --device=/dev/kvm --device=/dev/net/tun --cap-add NET_ADMIN
        -v "$DIR/storage:/storage" -v "$DIR/shared:/shared"
        --stop-timeout 120 "$IMAGE")
    if [ "$dry" -eq 1 ]; then
        printf '%q ' "${run[@]}"
        echo
        return
    fi

    docker info >/dev/null 2>&1 || die "docker is not available"
    if container_running; then
        say "'$NAME' is already running"
        show_urls
        return
    fi
    if container_exists; then
        say "starting the existing container '$NAME'"
        docker start "$NAME" >/dev/null
        show_urls
        return
    fi
    { [ -r /dev/kvm ] && [ -w /dev/kvm ]; } ||
        die "/dev/kvm is missing or not accessible (needs hardware virtualization and the 'kvm' group)"
    mkdir -p "$DIR/storage" "$DIR/shared"
    local avail
    avail="$(df --output=avail -BG "$DIR" | tail -1 | tr -dc '0-9')"
    [ "${avail:-0}" -ge 30 ] || say "warning: only ${avail}G free under $DIR (the VM disk can grow to $DISK)"

    "${run[@]}" >/dev/null
    say "created '$NAME' ($VERSION, ${RAM} RAM, $CORES cores, disk in $DIR/storage)"
    show_urls
    echo "First boot downloads Windows and installs it unattended; follow it in the viewer,"
    echo "or with: $0 status   /   $0 screenshot"
}

cmd_status() {
    container_exists || {
        say "no container '$NAME' (create it with: $0 up)"
        return 0
    }
    docker ps -a --filter "name=^${NAME}\$" --format 'container:  {{.Names}} | {{.Status}} | {{.Ports}}'
    container_running || return 0
    local ip
    ip="$(guest_ip)"
    echo "guest:      ${ip:-no address yet (still downloading / preparing)}"
    if [ -n "$ip" ] && docker exec "$NAME" bash -c "timeout 3 bash -c 'exec 3<>/dev/tcp/$ip/3389'" 2>/dev/null; then
        echo "RDP:        answering - Windows is up (the desktop may still be finishing first-run setup: $0 screenshot)"
    else
        echo "RDP:        not answering yet"
    fi
    echo "last log lines:"
    docker logs --tail 4 "$NAME" 2>&1 | strip_ansi | sed '/^$/d; s/^/  /'
    printf 'disk used:  %s (of %s)\n' "$(du -sh "$DIR/storage" 2>/dev/null | cut -f1)" "$DISK"
}

# The QEMU monitor is a unix socket inside the container; `screendump` writes a PPM.
cmd_screenshot() {
    need_running
    local out="${1:-winvm-screen.png}" tmp
    tmp="$(mktemp -d)"
    docker exec "$NAME" bash -c 'rm -f /tmp/screen.ppm; (echo "screendump /tmp/screen.ppm"; sleep 2) | nc -U -w 4 /run/shm/monitor.sock >/dev/null 2>&1'
    docker cp "$NAME:/tmp/screen.ppm" "$tmp/screen.ppm" >/dev/null 2>&1 || {
        rm -rf "$tmp"
        die "could not read the screen (is the VM booted yet?)"
    }
    if python3 -c 'import PIL' 2>/dev/null; then
        python3 -c 'import sys; from PIL import Image; Image.open(sys.argv[1]).save(sys.argv[2])' "$tmp/screen.ppm" "$out"
    else
        out="${out%.png}.ppm"
        cp "$tmp/screen.ppm" "$out"
        say "Pillow is not installed, so this is the raw PPM (pip install pillow for PNG)"
    fi
    rm -rf "$tmp"
    say "screen saved to $out"
}

# The ISO's inbox winget (1.6 on the 2024 build) is far too old for manifest schema 1.12,
# so ship a current App Installer bundle for the test script to install from the share.
stage_winget() {
    local d="$DIR/shared/winget" base bundle="Microsoft.DesktopAppInstaller_8wekyb3d8bbwe.msixbundle"
    if [ -n "${WINVM_WINGET_TAG:-}" ]; then
        base="https://github.com/microsoft/winget-cli/releases/download/$WINVM_WINGET_TAG"
    else
        base="https://github.com/microsoft/winget-cli/releases/latest/download"
    fi
    mkdir -p "$d"
    if [ ! -f "$d/$bundle" ] || [ "$REFRESH" -eq 1 ]; then
        say "downloading winget ($bundle)"
        curl -fL --progress-bar -o "$d/$bundle.part" "$base/$bundle" || die "download failed: $base/$bundle"
        mv "$d/$bundle.part" "$d/$bundle"
    fi
    if [ ! -d "$d/deps/x64" ] || [ "$REFRESH" -eq 1 ]; then
        say "downloading winget dependencies"
        curl -fL --progress-bar -o "$d/deps.zip" "$base/DesktopAppInstaller_Dependencies.zip" ||
            die "download failed: $base/DesktopAppInstaller_Dependencies.zip"
        rm -rf "$d/deps"
        unzip -q -o "$d/deps.zip" 'x64/*' -d "$d/deps"
        rm -f "$d/deps.zip"
    fi
}

# Copy a channel's test script (.ps1 + .cmd) into the share as CRLF. Windows PowerShell 5.1
# reads BOM-less files as ANSI, so they must stay ASCII.
stage_scripts() { # <folder holding the scripts> <base name>: the .ps1 and/or .cmd of that name
    local f found=0
    for f in "$1/$2.ps1" "$1/$2.cmd"; do
        [ -f "$f" ] || continue
        found=1
        ! grep -q -P '[^\x00-\x7F]' "$f" || die "non-ASCII characters in $f"
        to_crlf "$f" "$DIR/shared/$(basename "$f")"
    done
    [ "$found" -eq 1 ] || die "no $2.ps1 or $2.cmd in $1"
}

stage_channel_winget() {
    local id dest f
    id="$(sed -n 's/^PackageIdentifier:[[:space:]]*//p' "$WINGET_DIR"/*.installer.yaml | tr -d '\r' | head -1)"
    [ -n "$id" ] || die "no PackageIdentifier in $WINGET_DIR/*.installer.yaml"
    stage_scripts "$TOOLS_DIR" test-winget-manifest
    dest="$DIR/shared/$id"
    mkdir -p "$dest"
    rm -f "$dest"/*.yaml
    for f in "$WINGET_DIR"/*.yaml; do to_crlf "$f" "$dest/$(basename "$f")"; done # CRLF, as in winget-pkgs
    stage_winget
}

stage_channel_scoop() {
    local dest="$DIR/shared/scoop"
    stage_scripts "$PKG_ROOT/scoop/tools" test-scoop-manifest
    stage_scripts "$PKG_ROOT/scoop/tools" test-scoop-bucket
    mkdir -p "$dest"
    to_crlf "$PKG_ROOT/scoop/jsonquery-gui.json" "$dest/jsonquery-gui.json" # CRLF, as a bucket checkout has it
}

stage_channel_choco() {
    local dest="$DIR/shared/chocolatey" nupkg
    # shellcheck disable=SC2012 # the file names are ours (id.version.nupkg), so plain ls is fine
    nupkg="$(ls -1t "$REPO_ROOT"/dist/chocolatey/*.nupkg 2>/dev/null | head -1 || true)"
    [ -n "$nupkg" ] || die "no .nupkg in $REPO_ROOT/dist/chocolatey - run: packaging/chocolatey/dev/choco.sh pack"
    stage_scripts "$PKG_ROOT/chocolatey/dev" test-choco-package
    mkdir -p "$dest"
    rm -f "$dest"/*.nupkg
    cp "$nupkg" "$dest/"
}

stage_channel_msix() {
    local dest="$DIR/shared/msix" pkg cer="$REPO_ROOT/dist/msix/jsonquery-gui-test.cer"
    # shellcheck disable=SC2012 # the file names are ours (jsonquery-gui_<ver>_x64-test.msix), so plain ls is fine
    pkg="$(ls -1t "$REPO_ROOT"/dist/msix/*-test.msix 2>/dev/null | head -1 || true)"
    { [ -n "$pkg" ] && [ -f "$cer" ]; } ||
        die "no test package in $REPO_ROOT/dist/msix - run the MSIX workflow, then: gh run download <run id> -n msix-<tag> -D dist/msix"
    stage_scripts "$PKG_ROOT/msix/tools" test-msix-package
    mkdir -p "$dest"
    rm -f "$dest"/*.msix "$dest"/*.cer
    cp "$pkg" "$cer" "$dest/"
}

cmd_stage() {
    local channel=winget
    case "${1:-}" in
    winget | scoop | choco | msix) channel="$1" && shift ;;
    chocolatey) channel=choco && shift ;;
    esac
    REFRESH=0
    [ "${1:-}" = "--refresh" ] && REFRESH=1
    mkdir -p "$DIR/shared"
    "stage_channel_$channel"

    say "staged '$channel' in $DIR/shared (Windows: \\\\host.lan\\Data, also the Shared folder on the desktop)"
    (cd "$DIR/shared" && find . -maxdepth 2 -type f -not -path './logs/*' -not -path './winget/*' | sort | sed 's/^/  /')
    case "$channel" in
    choco) echo "Next: double-click test-choco-package.cmd in Windows, then: $0 results choco" ;;
    msix) echo "Next: double-click test-msix-package.cmd in Windows (a screenshot lands in logs/msix-launch.png), then: $0 results msix" ;;
    scoop) echo "Next: double-click test-scoop-manifest.cmd in Windows, then: $0 results scoop" ;;
    *) echo "Next: double-click test-winget-manifest.cmd in Windows, then: $0 results winget" ;;
    esac
}

cmd_results() {
    local name=result
    case "${1:-}" in
    winget | scoop | choco | msix) name="$1" ;;
    chocolatey) name=choco ;;
    "") ;;
    *) die "unknown channel '$1' (winget, scoop, choco or msix)" ;;
    esac
    local r="$DIR/shared/logs/latest-$name.txt"
    [ -f "$r" ] || die "no result at $r yet - run the test script in Windows first"
    echo "(written $(date -r "$r" '+%F %T'))"
    tr -d '\r' <"$r"
    echo "logs in $DIR/shared/logs:"
    # shellcheck disable=SC2012 # the file names are ours (timestamped), so plain ls is fine
    ls -1t "$DIR/shared/logs" | head -8 | sed 's/^/  /'
}

case "${1:-help}" in
up) shift && cmd_up "$@" ;;
status) cmd_status ;;
screenshot) shift && cmd_screenshot "$@" ;;
stage) shift && cmd_stage "$@" ;;
results) shift && cmd_results "$@" ;;
stop) docker stop -t 120 "$NAME" ;;
down)
    docker rm -f "$NAME"
    echo "The VM disk is still in $DIR/storage (delete it to reclaim the space)."
    ;;
help | -h | --help) usage ;;
*)
    usage >&2
    exit 2
    ;;
esac
