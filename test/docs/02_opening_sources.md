# Opening a JSON source — test requirements

Source: `crates/app/src/app.rs` (§Open File/Open URL/paste/drag-drop),
`crates/core/src/document.rs`, `crates/app/src/worker.rs`.

Covers every way a document gets loaded, plus the load-time edge cases
(NDJSON, empty file, malformed JSON, oversized URL download). See
[00_test_strategy.md](00_test_strategy.md) for the native file-dialog
automation approach these Open File cases depend on.

## Preconditions common to this area

- Fresh app launch per test unless a case explicitly says "with a document
  already loaded" (for the Clear/replace cases).
- A small fixture-file directory under (future) `test/resources/fixtures/`
  will hold the JSON files these cases reference — named here descriptively;
  actual filenames are an implementation detail.

## Test cases

### TC-OPEN-001 — Open a valid JSON file via the file dialog
Priority: P1
Steps:
1. Click `Open File…`.
2. In the native dialog, navigate to / type the path of a small valid JSON
   object fixture, confirm.
Expected:
- Toolbar status area shows: the file's path (selectable text), then its
  human-readable byte size (e.g. `512 B`), no NDJSON-record count suffix.
- Status bar shows `Parsed in {duration}`.
- Source panel's Tree view shows the parsed content, root expanded.
- Window title is still exactly `jsonquery` (unchanged by loading — confirms
  the "title never changes" fact from TC-WIN-001 under a loaded-doc state).
Automation notes: dialog interaction per the strategy doc's path-typing
approach; byte-size and duration text via OCR, "contains/matches pattern"
not exact equality (duration is nondeterministic).

### TC-OPEN-002 — Open File dialog filters to JSON-ish extensions
Priority: P2
Steps: Open the `Open File…` dialog and inspect the file-type filter.
Expected: exactly one filter group, labeled `JSON`, matching extensions
`json, ndjson, jsonl, log, txt`.
Automation notes: this is dialog-chrome, not app UI — OCR/image-match the
filter dropdown text.

### TC-OPEN-003 — Open a JSON source via URL
Priority: P1
Preconditions: a small local HTTP server (implementation detail) serving a
known-good JSON fixture, so the test doesn't depend on network access.
Steps:
1. Click `Open URL…`.
2. Type the fixture server's URL into the field.
3. Click `Load` (also verify Enter-to-submit works, as a separate quick case
   or a parametrized variant of this one).
Expected:
- Popup closes; toolbar status area shows the URL as the source label; a
  `Loading…` spinner may appear briefly before the doc renders.
- Same downstream assertions as TC-OPEN-001 once loaded.
Automation notes: race — assert on the post-load state via
`Wait Until Keyword Succeeds`, don't assume the spinner is observable (it may
resolve faster than a screenshot cadence for small fixtures).

### TC-OPEN-004 — Open URL: Load button is disabled on blank input
Priority: P3
Steps: Open the URL popup, leave the field blank, inspect `Load`.
Expected: `Load` is disabled (no click effect); `Cancel` still works and
closes the popup without loading anything.

### TC-OPEN-005 — Open URL: request failure surfaces a load error
Priority: P1
Steps: Open URL popup, submit a URL that will fail to connect (e.g. an
unroutable address or a closed local port).
Expected: status bar shows red text starting with `Load error: requesting
{url}: `.
Automation notes: OCR "starts with" match; the trailing OS/library error text
is not worth asserting exactly.

### TC-OPEN-006 — Open URL: non-JSON response body surfaces a parse error
Priority: P2
Steps: Point Open URL at a fixture server endpoint returning plain text/HTML.
Expected: status bar shows red text starting with `Load error: parsing data
from {url}: parsing JSON: `.

### TC-OPEN-007 — Open URL: oversized download is rejected, not truncated
Priority: P3
Steps: Point Open URL at a fixture endpoint that would exceed the 4 GiB cap.
Expected: status bar shows red `Load error: downloading {url}: ` — the app
must fail, not silently load a truncated 4 GiB document.
Automation notes: this test is expensive to run for real (needs an endpoint
that actually reaches the cap, or a way to shrink the cap for testing).
**Flag during implementation**: consider whether this is worth a real 4 GiB
transfer in CI, versus documenting it as untested-at-full-scale and instead
unit-testing the cap logic directly in Rust (outside this Robot Framework
suite). Marking P3 pending that decision.

### TC-OPEN-008 — Paste JSON auto-loads on Ctrl+V, no button
Priority: P1
Steps:
1. With no document loaded, click into the left-panel paste textarea.
2. Paste (Ctrl+V) valid JSON text (pre-seed the OS clipboard as part of test
   setup).
Expected: document loads immediately, no explicit submit action; toolbar
status area shows `(pasted JSON)` as the source label.
Automation notes: clipboard must be seeded before the paste — use the same
clipboard library as the Copy JSON Path verification
([07_context_menus.md](07_context_menus.md)), just in the write direction.

### TC-OPEN-009 — Paste JSON via Ctrl+Enter while the paste field is focused
Priority: P2
Steps: Type (not paste) valid JSON directly into the paste textarea, then
press Ctrl+Enter while it has focus.
Expected: same load result as TC-OPEN-008.

### TC-OPEN-010 — Drag-and-drop opens a file
Priority: P3 (skip on pure Wayland test hosts — see strategy doc)
Steps: Drag a fixture JSON file from a file manager onto the app window.
Expected: loads exactly as Open File would; only the first file loads if
multiple are dropped simultaneously (separate sub-case).
Automation notes: requires Xorg/XWayland; document the skip condition rather
than treating a Wayland failure here as a real bug.

### TC-OPEN-011 — Drop-hover overlay only appears when no document is loaded
Priority: P3 (same Wayland caveat as TC-OPEN-010)
Steps: (a) with no doc loaded, drag a file over the window without releasing
— observe; (b) with a doc already loaded, repeat.
Expected: (a) shows a full-panel `Drop to open` heading; (b) shows no such
overlay, even though releasing would still replace the document.

### TC-OPEN-012 — NDJSON / concatenated JSON wraps into one array
Priority: P2
Steps: Open a fixture file containing 3 newline-separated top-level JSON
values.
Expected: loads as a single array of 3 elements; toolbar shows
`(3 NDJSON records)` appended after the byte size.

### TC-OPEN-013 — Empty file loads as an empty array, no error
Priority: P2
Steps: Open a 0-byte fixture file.
Expected: loads successfully; Tree view shows an empty array at the root; no
`Load error` text anywhere; byte size shows as the empty case (confirm exact
`human_bytes` rendering for 0, e.g. `0 B`, during implementation).

### TC-OPEN-014 — Malformed JSON surfaces a parse error
Priority: P1
Steps: Open a fixture file with invalid JSON syntax (e.g. a trailing comma or
unquoted key).
Expected: status bar shows red text starting with `Load error: parsing JSON:
`; no document is loaded (Source panel stays in its prior/empty state).

### TC-OPEN-015 — Loading a new source replaces the current one, and cancels
any in-flight query
Priority: P2
Preconditions: a document is loaded and a query is currently running (use a
large-enough fixture / slow-enough query to keep it running momentarily —
implementation detail).
Steps: While the query is running, open a different file via `Open File…`.
Expected: new document loads and replaces the old one; results panel and
search state are cleared; the query **text box** and **engine-picker
selection** are preserved unchanged (this is the one piece of state that
survives a source swap — don't assert it gets cleared).

### TC-OPEN-016 — `Clear` button resets to the no-document state, with two
specific exceptions
Priority: P1
Preconditions: a document is loaded, a query has been run with a non-default
engine explicitly selected, and a search has been performed.
Steps: Click `Clear`.
Expected:
- Reverts to the TC-WIN-003 empty-state placeholder; load/save errors,
  find state, search panel, and both trees/text caches are all cleared.
- Query text box content is **unchanged**.
- Engine-picker selection is **unchanged**.
Automation notes: this pair of "does NOT clear" assertions is easy to get
backwards — write them as explicit positive checks (the text/selection is
still exactly what it was), not just "no error occurred".

### TC-OPEN-017 — `Clear` is disabled with nothing to clear
Priority: P3
Steps: On fresh launch (no doc, not loading, no load error), inspect `Clear`.
Expected: disabled (click has no effect).
