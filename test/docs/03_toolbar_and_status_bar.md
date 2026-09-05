# Toolbar and status bar — test requirements

Source: `crates/app/src/app.rs` (top toolbar §, bottom status bar §).
Load/query/save error *text* itself is enumerated in
[11_error_and_edge_cases.md](11_error_and_edge_cases.md); this doc covers the
surrounding chrome — what shows, in what order, under what conditions.

## Test cases

### TC-TOOL-001 — Source label reflects the actual source kind
Priority: P2
Steps: Load a document via (a) file, (b) URL, (c) paste — three sub-cases.
Expected: the read-only, selectable source-label field shows, respectively,
(a) the file's path, (b) the URL, (c) literally `(pasted JSON)`.
Automation notes: field is marked selectable/copyable in code — a good
candidate to verify by selecting-all + copy + clipboard read instead of OCR,
which sidesteps font/OCR risk entirely for this one field.

### TC-TOOL-002 — Byte size is human-readable and updates per document
Priority: P3
Steps: Load fixtures of a few different sizes (e.g. ~100 B, ~50 KB, ~5 MB).
Expected: size renders as `{N} B`, `{N.N} KB`, `{N.N} MB` respectively
(confirm exact `human_bytes` thresholds/rounding during implementation and
adjust expected strings accordingly — don't guess the boundary values here).

### TC-TOOL-003 — NDJSON record-count suffix only appears when there's more
than one top-level value
Priority: P3
Steps: (a) load a single-JSON-value file; (b) load a multi-record NDJSON file.
Expected: (a) no `(N NDJSON records)` suffix at all; (b) suffix present with
the correct count. Duplicates part of TC-OPEN-012 — keep as a quick negative
check here rather than a full re-test.

### TC-TOOL-004 — Status area is mutually exclusive: no-doc / loading / loaded
Priority: P2
Steps: Observe the toolbar status area at each of the three states (use a
slow-loading fixture, e.g. an artificially large file or a deliberately
delayed local fixture server, to catch the loading state).
Expected: exactly one of the three renderings is visible at a time; the
"loading" spinner + `Loading…` never simultaneously shows the no-doc
placeholder or a loaded doc's label.

### TC-TOOL-005 — Status bar: parse time only appears once a doc is loaded
Priority: P3
Steps: Compare status bar contents before vs. after loading a document.
Expected: `Parsed in {duration}` text is absent before load, present after.

### TC-TOOL-006 — Status bar: save-outcome text is mutually exclusive
(error takes priority over stale success)
Priority: P2
Steps: (a) save successfully once; observe `Saved to {path}`. (b) then
attempt a save that fails (e.g. point at an unwritable destination during
implementation, or simulate via a read-only fixture directory); observe.
Expected: (a) weak `Saved to {path}`. (b) red `Save error: {details}`
**replaces** the prior success text — the two never show together.

### TC-TOOL-007 — Status bar: query-outcome line format, with and without
truncation/cancellation, with engine suffix
Priority: P1
Steps: Run a query that returns a small result set with an explicitly
selected engine.
Expected: `Query ran in {elapsed} — {N} result(s) [{label}]` — no
`, {shown} shown (...)` clause (result count is under the 50,000 cap), no
` — cancelled` clause, engine suffix is `[{label}]` (not `[{label} · auto]`)
since the engine was explicitly picked, not auto-detected.
Automation notes: run companion variants for: (a) auto-detected engine → `[{label} ·
auto]` suffix; (b) a result set exceeding 50,000 items → truncation clause
present with the literal number `50000`; (c) a cancelled run → ` — cancelled`
clause present. These four variants share this one status-line format
requirement; don't write four near-duplicate docs, just four data-driven test
cases under this one ID group (TC-TOOL-007a..d) in the implementation.

### TC-TOOL-008 — Status bar: item-error count only appears when jq produces
per-item errors
Priority: P2
Steps: Run a jq query against mixed-type data that causes some items to
error while others succeed (e.g. `.[] | .foo` over an array mixing objects
and non-objects).
Expected: orange text `{N} item error(s) (last: {last_item_error})`; the
successfully-produced items still appear in the results tree alongside this.

### TC-TOOL-009 — "Find in Source" status line: locating spinner vs. final
message
Priority: P3
Covered primarily in [08_search_and_find_in_source.md](08_search_and_find_in_source.md);
listed here only to confirm this line's *placement* (rightmost segment of
the status bar) and that it doesn't interfere with the query-outcome segment
appearing simultaneously in the same bar.
