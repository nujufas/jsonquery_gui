# Saving — test requirements

Source: `crates/app/src/app.rs` (§Saving), `crates/app/src/worker.rs` (write
logic). All saves go through a native `rfd` Save dialog — see
[00_test_strategy.md](00_test_strategy.md) for the dialog-automation approach
these cases depend on. All output is pretty-printed JSON.

## Test cases, one per trigger

### TC-SAVE-001 — Source panel header "Save…": saves the whole document,
default filename
Priority: P1
Steps: (a) load via file, click Source's `Save…`, inspect the dialog's
suggested filename; (b) load via paste, repeat; (c) load via URL, repeat.
Expected default filenames: (a) the source file's own name; (b) `data.json`;
(c) the URL's last path segment, or `data.json` if the URL has none.
Then, completing the save: written file's content is the whole document,
pretty-printed.

### TC-SAVE-002 — Source panel header "Save…" is always available once a
document is loaded (no result-emptiness gating, unlike Results)
Priority: P3
Steps: with a doc loaded and no query ever run, click Source's `Save…`.
Expected: dialog opens normally (this button isn't gated on query state at
all).

### TC-SAVE-003 — Source tree row context menu "Save…": saves just that node,
default filename by kind
Priority: P1
Steps: right-click (a) an object-valued row, (b) an array-element row, (c) a
scalar row with no natural key context (implementation detail: pick a
fixture shape that produces the "else" fallback) → `Save…` on each.
Expected default filenames: (a) `{key}.json`; (b) `item_{index}.json`; (c)
`data.json`. Written content is just that node's value, pretty-printed.

### TC-SAVE-004 — Results tree row context menu "Save…": same per-kind
filename scheme, `results.json` fallback
Priority: P2
Steps: same three sub-cases as TC-SAVE-003, on Results rows instead.
Expected: same `{key}.json` / `item_{index}.json` scheme; fallback default
is `results.json` (not `data.json` — this is the one difference from
TC-SAVE-003's fallback, worth an explicit assertion since it's easy to
copy-paste the wrong fallback string between these two test cases).

### TC-SAVE-005 — Results panel header "Save…": disabled when results are
empty, saves the full (capped) results array otherwise
Priority: P1
Steps: (a) with no query run yet (empty results), inspect the button; (b)
run a query producing results, click `Save…`.
Expected: (a) disabled; (b) dialog opens, default filename `results.json`,
written content is the results array as currently materialized.

### TC-SAVE-006 — Results save only includes the materialized (≤50,000-item)
preview, not a hypothetically larger true result count
Priority: P3
Steps: run a query producing more than 50,000 results (large fixture, see
TC-QRY-062), save via Results header `Save…`.
Expected: written file contains exactly the capped subset (50,000 items),
not the true unbounded count — cross-check the written item count against
the status bar's "shown" number, not its "true total" number.

### TC-SAVE-007 — Save destination unwritable surfaces a save error
Priority: P2
Steps: attempt to save to a path the test process can't write (e.g. a
read-only directory fixture).
Expected: status bar shows red text starting with `Save error: creating
{path}: ` (or `Save error: writing {path}: ` depending on exactly where the
failure occurs — confirm which during implementation and assert the one that
actually triggers for the chosen failure mode).

### TC-SAVE-008 — Saving a row whose value no longer exists (race) reports
the specific error text
Priority: P3
Steps: this is a genuine race condition in the source (a row resolved from a
stale path after the document changed underneath it) — likely only
reproducible with deliberate timing manipulation. **Flag during
implementation**: decide whether this is worth simulating via test-only
timing control, or whether to leave it as a documented-but-unautomated
requirement and rely on code review / the Rust-side error path existing at
all. If skipped, expected text for reference: `Save error: that value is no
longer part of the document`.

### TC-SAVE-009 — Successful save updates the status bar, subsequent load
error clears it
Priority: P3
Steps: save successfully (`Saved to {path}` appears), then trigger any load
(even a failing one).
Expected: the stale `Saved to {path}` text is gone once a new load
outcome (success or error) is present — status bar's save-outcome segment
reflects only the most recent save, and a new document load doesn't leave an
old save confirmation lingering indefinitely. Confirm exact clearing
behavior against the source at implementation time (this specific
interaction wasn't in the primary inventory pass) and adjust expectations if
the real behavior differs.

### TC-SAVE-010 — Save filter is always `JSON` / `.json`
Priority: P3
Steps: open any Save dialog, inspect the file-type filter.
Expected: single filter labeled `JSON`, extension `json`, for every trigger
(no format choice is ever offered).
