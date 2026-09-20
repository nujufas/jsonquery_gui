#!/usr/bin/env bash
# Build and push the Chocolatey package with the official Chocolatey CLI in a container
# (chocolatey/choco:latest-linux), so nothing is installed on the host.
#
#   choco.sh pack           check the package, then build dist/chocolatey/jsonquery-gui.<ver>.nupkg
#   choco.sh push [--yes]   push that .nupkg to the community repository (asks first)
#
# `push` reads the API key from $CHOCOLATEY_API_KEY (your key is on
# https://community.chocolatey.org/account). It is never written anywhere by this script.
# Every version goes through Chocolatey's automated checks and a moderator, so a push is
# not instantly public; see README.md.
set -euo pipefail

DEV_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PKG_DIR="$(cd "$DEV_DIR/.." && pwd)" # packaging/chocolatey
REPO_ROOT="$(cd "$PKG_DIR/../.." && pwd)"
OUT="$REPO_ROOT/dist/chocolatey"
IMAGE="${CHOCO_IMAGE:-chocolatey/choco:latest-linux}"
SOURCE="https://push.chocolatey.org/"

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
say() { printf '==> %s\n' "$*"; }
pkg_version() { sed -n 's#.*<version>\(.*\)</version>.*#\1#p' "$PKG_DIR"/*.nuspec | head -1; }
pkg_id() { sed -n 's#.*<id>\(.*\)</id>.*#\1#p' "$PKG_DIR"/*.nuspec | head -1; }

cmd_pack() {
    command -v docker >/dev/null || die "docker is not installed"
    say "checking the package"
    "$DEV_DIR/check-package.py" || die "fix the problems above first"
    mkdir -p "$OUT"
    rm -f "$OUT/$(pkg_id)".*.nupkg
    say "choco pack ($IMAGE)"
    # Pack from a copy without dev/, so only the nuspec and tools/ are in the build tree.
    local tmp
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' RETURN
    cp -r "$PKG_DIR"/*.nuspec "$PKG_DIR/tools" "$tmp/"
    docker run --rm -v "$tmp:/pkg" -v "$OUT:/out" -w /pkg "$IMAGE" \
        bash -c 'choco pack --out /out && chown -R '"$(id -u):$(id -g)"' /out' | grep -v -E '^\s*$'
    local nupkg
    nupkg="$OUT/$(pkg_id).$(pkg_version).nupkg"
    [ -f "$nupkg" ] || die "no $nupkg was produced"
    say "built $nupkg ($(du -h "$nupkg" | cut -f1)); contents:"
    unzip -Z1 "$nupkg" | grep -v -E '^(_rels/|package/|\[Content_Types\])' | sed 's/^/  /'
}

cmd_push() {
    local yes=0 nupkg
    [ "${1:-}" = "--yes" ] && yes=1
    nupkg="$OUT/$(pkg_id).$(pkg_version).nupkg"
    [ -f "$nupkg" ] || die "$nupkg does not exist - run: $0 pack"
    [ -n "${CHOCOLATEY_API_KEY:-}" ] || die "set CHOCOLATEY_API_KEY (from https://community.chocolatey.org/account)"
    if [ "$yes" -ne 1 ]; then
        printf 'Push %s to %s ? [y/N] ' "$(basename "$nupkg")" "$SOURCE"
        read -r answer
        [ "$answer" = "y" ] || die "not pushed"
    fi
    # The key goes in through the container's environment, not on a command line.
    docker run --rm -e CHOCOLATEY_API_KEY -v "$OUT:/out:ro" -w /out "$IMAGE" \
        bash -c "choco push '$(basename "$nupkg")' --source '$SOURCE' --api-key \"\$CHOCOLATEY_API_KEY\""
    say "pushed. The package now waits for Chocolatey's automated checks and a moderator:"
    echo "  https://community.chocolatey.org/packages/$(pkg_id)/$(pkg_version)"
}

case "${1:-help}" in
pack) cmd_pack ;;
push) shift && cmd_push "$@" ;;
*)
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "${BASH_SOURCE[0]}"
    [ "${1:-help}" = help ] || exit 2
    ;;
esac
