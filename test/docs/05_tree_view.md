# Tree view — test requirements

Source: `crates/app/src/tree_view.rs`, `crates/core/src/tree.rs`. Applies
identically to both the Source and Results panels unless noted.

## Test cases

### TC-TREE-001 — Row rendering per value kind
Priority: P1
Steps: load a fixture with at least one of each kind (object, array, string,
number, bool, null) and inspect the tree.
Expected, per kind:
- Object → `{…} ({N} keys)`, weak/gray color.
- Array → `[…] ({N} items)`, weak/gray color.
- String → debug-quoted (e.g. `"hello"`), green.
- Number → literal text, orange.
- Bool → `true`/`false`, blue.
- Null → `null`, weak/gray.
- Object-entry rows are prefixed `"{key}": ` (debug-quoted key); array-entry
  rows are prefixed `{index}: `.
Automation notes: color assertions need pixel-sampling (read RGB at the
glyph's approximate location) rather than OCR, which doesn't report color.
Exact RGB values are in the source inventory (dark/light variants) — assert
against those, sampled in both themes (pairs with TC-WIN-002).

### TC-TREE-002 — Expand/collapse via the arrow, and via double-click
Priority: P1
Steps:
1. Load a doc with a nested object/array. Confirm only the root is expanded
   initially.
2. Click the expand arrow on a collapsed container row.
3. Click it again to collapse.
4. Double-click anywhere else on a (still collapsed after step 3) container
   row.
Expected: arrow glyph is `▸` collapsed, `▾` expanded; steps 2 and 4 both
toggle expansion; a single click elsewhere on a leaf (non-container) row does
nothing observable.

### TC-TREE-003 — Root-only-expanded is the default after every fresh
load/query
Priority: P2
Steps: load a doc, manually expand a few nested nodes, then run a new query
or load a different/second document.
Expected: the new tree (results, or the newly loaded source) starts with only
its root expanded — prior manual expand state does not carry over to a *new*
document or a *new* query's result tree.

### TC-TREE-004 — Streamed results don't disturb existing expand state
Priority: P3
Steps: run a query that streams many results progressively (large fixture),
manually expand one of the earlier-arriving result rows while more are still
streaming in.
Expected: that row's expanded state survives subsequent rows arriving —
expand state is keyed per-path, not reset by each new item.

### TC-TREE-005 — Virtualized scrolling stays responsive and correct at scale
Priority: P2
Steps: load/query a fixture producing several thousand rows once fully
expanded; scroll to the middle and to the end.
Expected: rows render correctly at every scroll position (no blank gaps,
no duplicated rows); this is primarily a "does it stay usable" smoke check
rather than a pixel-perfect assertion — a reasonable automation approach is
sampling row content via OCR at a few scroll positions and checking it's
plausible (non-empty, correctly indented) rather than asserting exact content
at every row.

### TC-TREE-006 — Highlight-and-scroll on reveal (from Search or Find in
Source)
Priority: P2
Covered primarily by [08_search_and_find_in_source.md](08_search_and_find_in_source.md);
listed here to confirm the *visual* contract specifically: the revealed row
gets a translucent selection-colored background, and the view auto-scrolls
so it's centered in the visible area (not just "somewhere on screen") —
worth a dedicated pixel/position check once a Search or Find-in-Source case
triggers a reveal on a node that starts off-screen.
