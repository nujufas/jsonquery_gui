#!/usr/bin/env bash
# Runs the jsonquery GUI Robot Framework suite end to end:
# builds the app, sets up the Python venv if needed, starts an isolated Xvfb
# display (real desktops don't work here -- see test/docs/00_test_strategy.md),
# runs the suites, tears everything down, and reports where results landed.
#
# Usage: test/run.sh [robot args...]
#   test/run.sh                          # run everything
#   test/run.sh test/suites/tree_view/    # run one suite
#   test/run.sh --include p1             # run only P1-tagged cases
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$REPO_ROOT/test"
VENV_DIR="$TEST_DIR/.venv"
RESULTS_DIR="$TEST_DIR/results"
XVFB_DISPLAY=":99"
XVFB_SIZE="1280x900x24"

PYENV_PYTHON="$HOME/.pyenv/versions/3.12.3/bin/python3.12"

log() { echo "[run.sh] $*"; }

# -- 1. build the app -----------------------------------------------------
log "Building jsonquery_gui..."
(cd "$REPO_ROOT" && cargo build -p jsonquery_gui)

# -- 2. venv ----------------------------------------------------------------
if [ ! -x "$VENV_DIR/bin/robot" ]; then
    log "Setting up test venv (Python 3.12, see 00_test_strategy.md for why)..."
    if [ -x "$PYENV_PYTHON" ]; then
        "$PYENV_PYTHON" -m venv "$VENV_DIR"
    else
        log "pyenv 3.12.3 not found at $PYENV_PYTHON -- falling back to python3, but"
        log "requirements.txt is only confirmed against 3.12; this may fail to install."
        python3 -m venv "$VENV_DIR"
    fi
    "$VENV_DIR/bin/pip" install --upgrade pip -q
    "$VENV_DIR/bin/pip" install -r "$TEST_DIR/requirements.txt"
fi

# -- 3. system prerequisites (fail fast with a clear message) ---------------
missing=()
for bin in Xvfb fluxbox xdotool tesseract gnome-screenshot xclip; do
    command -v "$bin" >/dev/null 2>&1 || missing+=("$bin")
done
if [ ${#missing[@]} -gt 0 ]; then
    log "Missing required system packages: ${missing[*]}"
    log "Install with: sudo apt install tesseract-ocr wmctrl xclip xvfb fluxbox gnome-screenshot xdotool"
    exit 1
fi

# -- 4. isolated display: Xvfb must exist before Python (and therefore Robot)
#       ever starts, since pyautogui probes DISPLAY at import time -----------
cleanup() {
    log "Tearing down test display..."
    [ -n "${FLUXBOX_PID:-}" ] && kill -9 "$FLUXBOX_PID" 2>/dev/null || true
    if [ "$STARTED_XVFB" = "1" ] && [ -n "${XVFB_PID:-}" ]; then
        kill -9 "$XVFB_PID" 2>/dev/null || true
        rm -f "/tmp/.X${XVFB_DISPLAY#:}-lock"
    fi
}
trap cleanup EXIT

STARTED_XVFB=0
if [ -e "/tmp/.X11-unix/X${XVFB_DISPLAY#:}" ] && ! DISPLAY="$XVFB_DISPLAY" xdotool getdisplaygeometry >/dev/null 2>&1; then
    log "Found a stale Xvfb socket for $XVFB_DISPLAY with nothing listening -- clearing it."
    rm -f "/tmp/.X${XVFB_DISPLAY#:}-lock" "/tmp/.X11-unix/X${XVFB_DISPLAY#:}"
fi
if [ ! -e "/tmp/.X11-unix/X${XVFB_DISPLAY#:}" ]; then
    log "Starting Xvfb on $XVFB_DISPLAY..."
    Xvfb "$XVFB_DISPLAY" -screen 0 "$XVFB_SIZE" -nolisten tcp &
    XVFB_PID=$!
    STARTED_XVFB=1
    sleep 1
else
    log "Xvfb already running on $XVFB_DISPLAY, reusing it."
fi
export DISPLAY="$XVFB_DISPLAY"

# fluxbox: bare Xvfb has no window manager, which silently breaks keyboard
# focus (confirmed -- see 00_test_strategy.md). The Deco:NONE apps rule is
# required for AppLibrary's window-relative coordinates to be accurate;
# AppLibrary writes it itself on Suite Setup, but write it here too so it
# exists before fluxbox's very first launch in a brand new environment.
mkdir -p "$HOME/.fluxbox"
if ! grep -q "class=jsonquery_gui" "$HOME/.fluxbox/apps" 2>/dev/null; then
    printf '[app] (class=jsonquery_gui)\n  [Deco] {NONE}\n[end]\n' >> "$HOME/.fluxbox/apps"
fi
fluxbox >/dev/null 2>&1 &
FLUXBOX_PID=$!
sleep 1

# -- 5. run the suite ---------------------------------------------------------
mkdir -p "$RESULTS_DIR"
TARGET="${1:-$TEST_DIR/suites}"
if [ "$#" -gt 0 ]; then
    shift
fi

log "Running Robot Framework suite(s): $TARGET"
set +e
"$VENV_DIR/bin/robot" \
    --outputdir "$RESULTS_DIR" \
    --pythonpath "$TEST_DIR/resources" \
    "$@" \
    "$TARGET"
RC=$?
set -e

log "Results: $RESULTS_DIR/report.html (summary), $RESULTS_DIR/log.html (detail)"
exit $RC
