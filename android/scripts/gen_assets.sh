#!/usr/bin/env bash
# Regenerate launcher icons and Play graphics from assets/icon.png (in the container).
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
in_container -- python3 scripts/gen_assets.py
