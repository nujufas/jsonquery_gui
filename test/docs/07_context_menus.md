# Right-click context menus — test requirements

Source: `crates/app/src/tree_view.rs` (`context_menu` §). Applies to any row
in either the Source or Results tree; item availability differs slightly by
which tree the row belongs to (noted per case).

## Test cases

### TC-CTX-001 — Menu contents and order, Source tree row
Priority: P1
Steps: right-click any row in the Source tree.
Expected menu, top to bottom: `Save…`, `Copy JSON Path`, then a separator,
then `Search…`. **No** `Find in Source` item (that's Results-only).

### TC-CTX-002 — Menu contents and order, Results tree row
Priority: P1
Steps: right-click any row in the Results tree.
Expected menu, top to bottom: `Save…`, `Copy JSON Path`, `Find in Source`,
then a separator, then `Search…`.

### TC-CTX-003 — "Copy JSON Path" copies the jq-style path to the clipboard
Priority: P1
Steps: right-click a nested row (e.g. a value at `.foo[3]["a-b"]`-shaped
location — pick a fixture with a key needing quoting, to exercise that
formatting rule), click `Copy JSON Path`.
Expected: OS clipboard contains exactly the jq-style path string for that
row, quoting non-identifier keys (e.g. `["a-b"]`) and using plain `[N]` for
array indices.
Automation notes: read the clipboard via `pyperclip` (or equivalent) rather
than OCR — this is an exact-string assertion where OCR would be needlessly
risky for no benefit.

### TC-CTX-004 — "Save…" on a single row saves just that node
Priority: P1
Covered jointly with [09_saving.md](09_saving.md) TC-SAVE-003/004; this entry
exists to confirm the menu item itself opens the native Save dialog
correctly scoped to the row (not the whole document) — see the saving doc
for the file-content and default-filename assertions.

### TC-CTX-005 — "Find in Source", Results row only, success case
Priority: P1
Covered in [08_search_and_find_in_source.md](08_search_and_find_in_source.md)
(TC-SRCH-020+); listed here only to confirm menu placement/availability.

### TC-CTX-006 — "Search…" opens the dialog scoped to the correct tree
Priority: P2
Steps: (a) right-click a Source row → `Search…`; (b) right-click a Results
row → `Search…`.
Expected: dialog title reads `Search — Source` for (a), `Search — Results`
for (b) — confirms scoping is by which tree was clicked, not a global toggle.
Full dialog behavior covered in
[08_search_and_find_in_source.md](08_search_and_find_in_source.md).

### TC-CTX-007 — No context menu exists outside tree rows
Priority: P3
Steps: right-click the query box, a toolbar button, and a panel header.
Expected: no context menu appears in any of these locations (negative
assertion, low priority — include as a quick smoke check rather than a full
per-location test).
