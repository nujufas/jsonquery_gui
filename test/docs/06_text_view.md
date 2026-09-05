# Text (raw) view — test requirements

Source: `crates/app/src/app.rs` (Text view rendering §, editable-paste §).
Applies to both Source and Results panels; the editable-paste variant applies
to Source only, and only when the document came from paste.

## Test cases

### TC-TXT-001 — Tree/Text toggle switches rendering mode per panel
independently
Priority: P1
Steps: load a doc, run a query so both panels have content. Toggle Source to
Text while leaving Results on Tree; then toggle Results to Text too.
Expected: each panel's Tree/Text state is independent of the other; Text view
shows plain pretty-printed JSON (2-space indent), one uniform text color, no
per-type coloring (unlike Tree view).

### TC-TXT-002 — Long lines are not wrapped; horizontal scroll works
Priority: P3
Steps: load/produce content with a very long single-line string value, view
it in Text mode.
Expected: line is not wrapped; a horizontal scrollbar/gesture reveals the
rest of it.

### TC-TXT-003 — Rendering indicator while a Text view render is pending
Priority: P3
Steps: trigger a Text view render of a large-enough document/result set to
observe the transient state (implementation detail: needs a fixture large
enough that the async render is observable, but under the 20,000-node
budget so this test isn't also exercising TC-TXT-004).
Expected: spinner + `Rendering…` shown while pending, replaced by the actual
content once ready.

### TC-TXT-004 — 20,000-node budget notice, Source and Results wording
differs
Priority: P2
Steps: (a) load a document with more than 20,000 nodes, view its Text mode;
(b) separately, produce a query result with more than 20,000 nodes, view
Results Text mode.
Expected: (a) weak notice `Showing the first 20000 nodes — use Tree view, or
Save… for the full results.` — note the notice's own wording says "results"
even in the Source-panel case; confirm this exact string against the source
at implementation time rather than assuming Source gets a different word.
(b) same notice text for Results. (If Source and Results notice text turn out
identical, only one wording needs asserting — flag during implementation
whether the two differ and correct this test case's expected values
accordingly.)

### TC-TXT-005 — Text view only re-renders when the underlying value changes
Priority: P3
Steps: with a doc's Text view open, interact with something unrelated (e.g.
toggle theme, resize window) without changing the doc or query.
Expected: no `Rendering…` flash occurs — this is a performance/behavioral
nuance more than a user-facing correctness one; low priority, useful mainly
as a regression guard if this ever gets broken.

### TC-TXT-006 — Editable Text view (pasted documents only): Apply re-parses
in place
Priority: P1
Preconditions: a document loaded via paste (not file/URL).
Steps:
1. Switch Source panel to Text view. Confirm weak label `Editable — change
   the JSON below, then apply.` and an `Apply  (Ctrl+Enter)` button (note the
   two spaces in the label — assert it literally if the OCR engine's spacing
   normalization allows, otherwise treat as a known engine limitation and use
   a "contains Apply" check instead).
2. Edit the JSON text (e.g. change a value).
3. Click `Apply`.
Expected: document reloads from the edited text, exactly like a fresh paste
load (same load-error handling if the edit produces invalid JSON — see
TC-TXT-008).

### TC-TXT-007 — Ctrl+Enter also applies, while the editable textarea has
focus
Priority: P2
Steps: same as TC-TXT-006 but press Ctrl+Enter instead of clicking Apply.
Expected: identical result.

### TC-TXT-008 — Editing to invalid JSON and applying surfaces the normal
load error
Priority: P2
Steps: edit the pasted-doc Text view to invalid JSON, click Apply.
Expected: `Load error: parsing JSON: {details}` in the status bar, same as
any other malformed-JSON load (cross-reference TC-OPEN-014).

### TC-TXT-009 — File- and URL-sourced documents' Text view is read-only
Priority: P2
Steps: load a doc via file (or URL), switch to Text view, attempt to type
into it.
Expected: no edit affordance exists at all — no Apply button, and the text
widget does not accept input (confirm it's a plain non-editable label widget,
not just a disabled-looking editable one).

### TC-TXT-010 — Apply button disabled when the edited buffer is blank
Priority: P3
Steps: in the editable Text view, select-all and delete the content.
Expected: `Apply` becomes disabled.
