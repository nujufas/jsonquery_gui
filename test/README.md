# jsonquery GUI tests (Robot Framework)

**Status: 10 suites, 74 test cases, all passing** (confirmed across multiple
consecutive full-suite runs as of 2026-09-05) — `launch_and_window`,
`opening_sources`, `query_engines`, `toolbar_and_status`, `tree_view`,
`text_view`, `context_menus`, `search`, `keyboard_shortcuts`, `saving`. Run
them with:

```
test/run.sh                              # everything under test/suites/
test/run.sh test/suites/query_engines/   # just one suite
```

First run builds the app, creates a Python venv (pyenv 3.12.3), and checks
for the required system packages (see below); later runs skip the venv setup.
Results land under `test/results/` (`report.html`, `log.html`).

## Start here

Read [docs/00_test_strategy.md](docs/00_test_strategy.md) for the full
architecture rationale, kept up to date through both implementation passes.
The short version:

- **Confirmed**: accessibility-tree automation doesn't work on this app/stack
  (empirically investigated, not assumed).
- **Confirmed**: real-desktop screen capture is blocked on GNOME/Wayland —
  the suite runs against an isolated Xvfb display instead (`test/run.sh`
  handles this), with a minimal window manager (`fluxbox`) alongside it,
  because bare Xvfb silently breaks keyboard focus.
- **Confirmed, and a real limit on scope**: the native Open File and Save
  dialogs hang or fail before showing anything usable, in every environment
  tried, due to an `rfd`-vs-non-GTK-toolkit integration gap. Every test case
  that depends on one of those dialogs completing is marked **Blocked** in
  the traceability matrix, not implemented as a false pass — 11 cases in
  total, all needing an app-side `Cargo.toml` change to ever unblock.
- The OCR- and coordinate-based interaction technique that actually works,
  plus a growing list of sharp, non-obvious gotchas confirmed while building
  this out — worth reading before extending any suite:
  - pyautogui's key name for Enter is `enter`, not `Return`.
  - `pyautogui.hotkey()` needs an explicit `interval` between keys (0.05s) —
    the default (all keys sent essentially at once) is a real source of
    "Ctrl+Enter sometimes doesn't register" flakiness.
  - A region cropped too tightly to one line of text can make Tesseract's
    layout analysis fail outright (pure noise, not just a bad read) — add
    vertical margin rather than cropping exactly to the text.
  - Tesseract, at this font size, sometimes reads "0" as "O" and "jq" as
    "iq", and can insert a stray space around punctuation (e.g. splitting
    "valid.json" into two word-boxes). `AppLibrary._text_contains` has
    permissive fallbacks for the first and third; the second is worked
    around per-assertion.
  - `ui.weak()`-styled (low-contrast) text — placeholder hints, match
    counts, "No matches found." — is confirmed unreliable for OCR even with
    plenty of crop margin. Assert on adjacent normal-contrast content
    instead of chasing it.
  - A right-click, or a click immediately after typing into a field (the
    Search/Open URL dialogs' submit buttons), occasionally doesn't register
    on the first attempt. The shared keywords (`Open Row Context Menu`,
    `Load Via Url`, `Search For`) all retry-and-verify rather than assuming
    one attempt always works.
  - Some panels (confirmed: the search-results panel) size themselves to
    their content rather than staying a fixed height/position — don't
    calibrate a fixed y-coordinate against only one content size; use one
    wide region plus OCR-based lookups (`Click Text In Region`) instead.
  - Pasting only loads anything while the empty-state "Paste JSON here…" box
    is showing — once a document is loaded there's no box left to paste
    into, so Ctrl+V silently does nothing. Loading a *second* document over
    an already-loaded one needs `Load Via Url` instead.

Then [docs/99_traceability_matrix.md](docs/99_traceability_matrix.md) is the
single index of all 107 defined test-case IDs, with per-row status
(`Passing` / `Blocked` / `Not implemented`, each with its specific reason).
The other files under `docs/` are the per-feature-area requirement documents
(`01_launch_and_window.md` through `10_keyboard_shortcuts.md`, plus
`11_error_and_edge_cases.md` as a consolidated cross-reference of every error
string in the app).

## Environment prerequisites

```
sudo apt install tesseract-ocr wmctrl xclip xvfb fluxbox gnome-screenshot xdotool
```

`test/run.sh` checks for these and fails fast with this same command if any
are missing.

## Directory layout

```
test/
  README.md                — this file
  run.sh                    — the single entry point: builds the app, sets up
                              the venv, starts Xvfb+fluxbox, runs the suite(s)
  requirements.txt          — pinned Python test dependencies
  results/                  — Robot's output.xml/log.html/report.html (generated)
  docs/                     — requirements + strategy, see above
  resources/
    AppLibrary.py            — custom Robot Framework library: process
                                lifecycle, window-relative click/type, OCR
                                text read/find/click, pixel-color compare,
                                clipboard, local HTTP fixture server
    keywords.resource         — shared layout-region constants + composite
                                keywords (Load Fixture Via Paste, Load Via
                                Url, Run Query, Search For, Open Row Context
                                Menu, Select Engine, ...) built on AppLibrary
    fixtures/                 — sample JSON files used across suites
      http/                   — files served by the fixture HTTP server for
                                Open URL... tests (valid/invalid/empty/large)
  suites/
    launch_and_window/        — 4 tests: default state, theme, placeholder,
                                no menu bar
    opening_sources/          — 12 tests: paste, NDJSON, malformed JSON,
                                Clear, Open URL (success/failure/non-JSON/
                                empty/disabled-state), Ctrl+Enter paste,
                                replace-while-loaded
    query_engines/             — 22 tests: picker UI, auto-detect for all 4
                                engines, and each engine's own 0-vs-error
                                contract (jq streaming + item errors, Pointer
                                0-or-1, JSONPath 0-is-fine, JMESPath always-1)
    toolbar_and_status/         — 5 tests: source label by kind, byte-size
                                units, NDJSON suffix, parse-time/load-error
                                status-bar behavior
    tree_view/                  — 4 tests: per-kind row rendering/coloring,
                                expand/collapse, default expand state,
                                virtualized scrolling
    text_view/                   — 7 tests: independent Tree/Text toggles,
                                editable-paste Apply (click + Ctrl+Enter),
                                invalid-JSON apply error, read-only file/URL
                                text, blank-buffer disabled state, node budget
    context_menus/               — 5 tests: Source vs. Results row menu
                                contents, Copy JSON Path, Search... scoping,
                                no menu outside rows
    search/                       — 9 tests: dialog fields, case-insensitive
                                substring (incl. bool/null), regex mode +
                                invalid pattern, results header/close,
                                hit-line format + reveal-on-click, Find in
                                Source (success + not-found)
    keyboard_shortcuts/           — 4 tests: Ctrl+F panel scoping, Ctrl+Enter
                                paste (incl. focus-gating), Enter submits
                                Open URL/Search
    saving/                       — 2 tests: Save... buttons' enabled-state
                                behavior (the dialogs themselves are blocked)
```

## What's left

The remaining gaps are all documented individually in the traceability
matrix, not silent — in short: 11 cases genuinely **Blocked** on the native
dialog issue (would need an app-side `Cargo.toml` change — building `rfd`
with the `gtk3` feature instead of the default `xdg-portal` backend — to ever
unblock), and 22 **Not implemented** cases that are either low-value P3s
(drag-and-drop, an oversized-download fixture, a native-text-editing smoke
test) or judged too timing-fragile to assert reliably from outside the
process (a genuine cancel-mid-query race, a sub-frame "Rendering…"
transient, streamed-results-preserve-expand-state). None represent a known
app defect — each is a testing-technique or cost/benefit call, with its
specific reasoning next to it in the matrix.
