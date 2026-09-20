#!/usr/bin/env bash
# Retake the phone screenshots for the Google Play listing from the real app
# on the emulator (android/test/suites/store_screenshots), then check them
# against Play's rules.
#
#   android/scripts/store-screenshots.sh
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

rm -f "$ANDROID_DIR"/play/metadata/android/en-US/images/phoneScreenshots/*.png
"$ANDROID_DIR/test/run.sh" --include screenshots suites/store_screenshots
in_container -- python3 scripts/play_publish.py --check-listing-only
log "Screenshots are in android/play/metadata/android/en-US/images/phoneScreenshots/ -- have a look before committing."
