#!/usr/bin/env bash
# Raise the Android versionCode. Google Play rejects an upload whose
# versionCode isn't higher than every earlier one, so do this before each
# upload (the human-readable version comes from the workspace version in the
# root Cargo.toml and is bumped there).
#
#   android/scripts/bump-version.sh        # +1
#   android/scripts/bump-version.sh 42     # set explicitly
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

current="$(version_code)"
next="${1:-$((current + 1))}"
[[ "$next" =~ ^[0-9]+$ ]] || die "versionCode must be a positive integer, got '$next'"
[ "$next" -gt "$current" ] || die "new versionCode ($next) must be higher than the current one ($current)"
sed -i "s/^versionCode=.*/versionCode=$next/" "$ANDROID_DIR/version.properties"
log "versionCode $current -> $next (versionName $(workspace_version))"
log "Write the release notes for it: android/play/metadata/android/en-US/changelogs/$next.txt"
