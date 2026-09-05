# Keyboard shortcuts — test requirements

Source: `crates/app/src/app.rs` (`note_panel_click`, and the four Ctrl-key
handlers scattered through the query bar, paste area, Text view, and the
global key-handling block). This doc is deliberately cross-cutting: most
shortcuts just trigger a flow that's already fully specified elsewhere, so
each entry below states only the shortcut-specific nuance (focus-gating,
which-panel-is-targeted) and cross-references the full behavioral spec
rather than re-describing it.

## Panel-focus tracking (prerequisite for TC-KEY-002/003)

The app tracks which panel (Source or Results) last received a mouse-down,
defaulting to Source. Ctrl+F and Ctrl+S act on whichever panel currently
"has focus" by this definition — clicking anywhere in a panel's rectangle
(not necessarily a specific widget) counts.

### TC-KEY-000 — Clicking a panel updates its focus for subsequent Ctrl+F/S
Priority: P2
Steps: click somewhere inside the Results panel (e.g. an empty area of its
tree), then press Ctrl+F.
Expected: the Search dialog opens titled `Search — Results` (not Source),
confirming the click changed focus tracking. Pairs with TC-KEY-002 below,
which covers the reverse.

## Test cases

### TC-KEY-001 — Ctrl+Enter runs the query, globally, regardless of focus
Priority: P1
Steps: with a document loaded and a valid query typed, click focus into some
unrelated widget (not the query box itself, not the paste/text areas — e.g.
click the tree view), then press Ctrl+Enter.
Expected: the query runs, identical to clicking `Run` — this shortcut is
**not** focus-gated, unlike the other three below. Full run-outcome
assertions are [04_query_bar_and_engines.md](04_query_bar_and_engines.md).

### TC-KEY-002 — Ctrl+F opens Search for the currently-focused panel; no-op
if Source is focused with nothing loaded
Priority: P2
Steps: (a) default focus (Source), no document loaded, press Ctrl+F; (b) with
a document loaded, Source focused, press Ctrl+F; (c) Results focused (per
TC-KEY-000), press Ctrl+F.
Expected: (a) nothing happens (no dialog opens — since Source has no doc to
search); (b) Search dialog opens titled `Search — Source`; (c) titled
`Search — Results`. Full dialog behavior is
[08_search_and_find_in_source.md](08_search_and_find_in_source.md).

### TC-KEY-003 — Ctrl+S saves the focused panel's "whole panel" save target
Priority: P2
Steps: (a) Source focused, doc loaded, press Ctrl+S; (b) Results focused,
results non-empty, press Ctrl+S; (c) Results focused, results empty, press
Ctrl+S.
Expected: (a) equivalent to clicking Source's `Save…` header button; (b)
equivalent to clicking Results' `Save…` header button; (c) no dialog opens
(mirrors that button's disabled state — cross-reference
[09_saving.md](09_saving.md) TC-SAVE-005).

### TC-KEY-004 — Ctrl+Enter loads pasted JSON while the paste textarea has
focus
Priority: P2
Cross-reference [02_opening_sources.md](02_opening_sources.md) TC-OPEN-009;
no separate test needed beyond confirming this is genuinely focus-gated
(i.e. Ctrl+Enter with focus elsewhere does *not* also try to load whatever
is currently sitting in the paste textarea — a quick negative check worth
adding as part of implementing TC-OPEN-009 rather than a standalone case).

### TC-KEY-005 — Ctrl+Enter applies edits while the editable pasted-Text-view
textarea has focus
Priority: P2
Cross-reference [06_text_view.md](06_text_view.md) TC-TXT-007. Same
focus-gating note as TC-KEY-004 applies — worth the same quick negative
check alongside it, not a standalone case.

### TC-KEY-006 — Enter submits the Open URL and Search popups
Priority: P3
Cross-reference [02_opening_sources.md](02_opening_sources.md) TC-OPEN-003
and [08_search_and_find_in_source.md](08_search_and_find_in_source.md)
TC-SRCH-001+ — implement as a parametrized "press Enter instead of clicking
the primary button" variant of those existing cases rather than a new case.

### TC-KEY-007 — Standard text-editing keys behave natively in every text
field
Priority: P3
Steps: in the query box (or any text field), use Ctrl+A, Ctrl+C, Ctrl+V,
Ctrl+X, and arrow-key navigation.
Expected: standard OS text-field behavior, no app-specific override or
interception. Low priority — egui's built-in `TextEdit` is trusted upstream
behavior; a single smoke test across one representative field is sufficient,
not a full matrix across every field in the app.
