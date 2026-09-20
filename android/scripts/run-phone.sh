#!/usr/bin/env bash
# Open the app on a phone emulator (a Pixel 3) that you can use by hand.
# The work is in run-emulator.sh; run this with --help for the options.
exec "$(dirname "${BASH_SOURCE[0]}")/run-emulator.sh" phone "$@"
