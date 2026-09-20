#!/usr/bin/env bash
# Upload a release bundle (and optionally the store listing) to Google Play.
#
#   android/scripts/publish.sh [--track internal|alpha|beta|production] [--status draft|completed]
#                              [--listing] [--dry-run] [path/to/app.aab]
#
# Credentials: a Play Console service-account key, from
#   android/signing/play-service-account.json   or   $PLAY_SERVICE_ACCOUNT_JSON (path or JSON)
# See android/docs/play-store.md. Runs preflight.sh first; --dry-run stops there.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

version="$(workspace_version)"
aab="$ANDROID_DIR/dist/jsonquery-$version.aab"
passthrough=()
while [ $# -gt 0 ]; do
    case "$1" in
        --track|--status) passthrough+=("$1" "$2"); shift 2 ;;
        --listing|--dry-run) passthrough+=("$1"); shift ;;
        -*) die "unknown option: $1" ;;
        *) aab="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"; shift ;;
    esac
done

"$ANDROID_DIR/scripts/preflight.sh" "$aab"

rel() { echo "/work/${1#"$REPO_ROOT"/}"; }
extra=()
key="$ANDROID_DIR/signing/play-service-account.json"
[ -f "$key" ] && extra+=(--credentials "$(rel "$key")")

in_container --env PLAY_SERVICE_ACCOUNT_JSON -- \
    python3 scripts/play_publish.py --aab "$(rel "$aab")" "${extra[@]}" "${passthrough[@]}"
