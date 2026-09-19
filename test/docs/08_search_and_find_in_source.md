# Search and Find in Source — test requirements

Source: `crates/app/src/app.rs` (Search dialog §, bottom hit-list panel §,
Find-in-Source §), `crates/core/src/tree.rs`. These are two distinct features
that happen to share the word "find" (and the bottom panel) — keep them in
separate test groups (`TC-SRCH-00x`, `TC-SRCH-03x`, `TC-SRCH-04x` for Search,
`TC-SRCH-02x` and `TC-SRCH-037` for Find in Source) to avoid conflating them,
per the source inventory's explicit warning that they're unrelated.

## A. "Search…" (the Find dialog)

`Ctrl+F`, or a row's `Search…` context-menu item, opens a small
Notepad++-style Find dialog over the tree it was opened for (Source or
Results — the whole tree, not just the right-clicked row). The **Find field is
focused at once**, so typing needs no click. The dialog stays open and has two
ways to see the matches:

- **`Find`** (or Enter) — Notepad++'s Find Next. It reveals the matches **one at
  a time** (ancestors expanded, scrolled to, highlighted), stepping to the next
  on each press and wrapping from the last back to the first. The first Find
  for a query searches the whole tree on the worker thread; changing the text,
  the `Regex` box or the target tree starts over at the first match.
- **`Find All`** — lists **every** match in the bottom panel, headed
  `Search results — {Source|Results} "{query}"` (plus ` (regex)`), one
  `[Source] {path}   {preview}` line each. Nothing is revealed until a line is
  clicked. It shares its search with Find, so either can follow the other
  without searching again.

A status line under the buttons says where the dialog stands: `N of M` (plus
` — wrapped around to the top` after stepping past the last match) while
stepping, `N matches` after a Find All, red `No matches found.`, red
`Search error: {details}`, or a spinner and `Searching…`. It goes blank as soon
as the text changes. `Esc` or `Cancel` closes the dialog (the panel stays).

When a Find All list is showing and Find steps, the list's highlight follows
the revealed match; clicking a list line makes Find carry on from that match —
as Notepad++'s Find Next carries on from where the caret was left.

### TC-SRCH-001 — Dialog fields and buttons
Priority: P2
Steps: open Search (via context menu or Ctrl+F) on either tree.
Expected: title `Search — {Source|Results}` per which tree; `Find:` field with
hint `text to find…`; `Regex` checkbox (unchecked by default); `Find`,
`Find All` (both disabled while the field is blank) and `Cancel` buttons.

### TC-SRCH-002 — Plain (non-regex) search is case-insensitive substring
match over keys and values
Priority: P1
Steps: search for a substring that matches a key in one place and a string
value in another (mixed case relative to the actual fixture content), with
`Regex` unchecked.
Expected: both the key-match and the value-match are found (`N of M`, the
first revealed and highlighted; Find steps on to the other), regardless of
case; number/bool/null values match against their string form (e.g.
searching `true` matches a boolean `true` value, searching `null` matches a
null value) — include a sub-case for this since it's an easy thing to miss
if search were naively "only string values".

### TC-SRCH-003 — Regex mode switches matching engine; invalid pattern is an
error
Priority: P2
Steps: (a) check `Regex`, search a valid pattern (e.g. an alternation or
anchor); (b) search a syntactically invalid pattern (e.g. `(unclosed`).
Expected: (a) matches per regex semantics against the same key/value texts
(a hit for `^Ali` proves it — no node contains that as plain text); (b) the
dialog's status line shows red `Search error: {details}` (message is the
underlying regex-crate parse error — treat as "non-empty error shown", not an
exact string match, since the crate's wording isn't part of this app's own
contract). Editing the text or the `Regex` box clears the error. (Find All:
TC-SRCH-046.)

### TC-SRCH-004 — Results panel: header format, states, and Close
Priority: P2
Steps: `Find All` for text that occurs; then for text that occurs nowhere; then
click the panel's `Close`.
Expected: header reads `Search results — {Source|Results} "{query}"` (regex
mode appends ` (regex)`); hit count as weak `{N} match(es)`; the zero-hit case
shows weak `No matches found.` (distinct from the red `Search error` of
TC-SRCH-003 — don't conflate "0 matches" with "search error"); `Close` hides
the panel.

### TC-SRCH-005 — Each hit line's format and click-to-reveal behavior
Priority: P1
Steps: `Find All` for a value that occurs once, inspect its line, click it.
Expected: line reads `[{Source|Results}]  {jq-style path}   {one-line value
preview}` (preview format matches tree-row rendering rules from
[05_tree_view.md](05_tree_view.md) TC-TREE-001); clicking it reveals/expands/
highlights that node in its owning tree and switches that panel to Tree view
(if it was on Text view).

### TC-SRCH-006 — 5,000-match cap
Priority: P3
Steps: search a term matching more than 5,000 keys/values in a large fixture.
Expected: Find and Find All see at most the first 5,000 matches (the core
search stops collecting there), with **no** truncation notice shown (unlike
the Text-view 20,000-node budget, which does show a notice — this is a
deliberate asymmetry worth its own assertion so a future "helpful" UI change
that adds a notice here doesn't get treated as a false positive regression
without someone noticing the doc needs updating too).

### TC-SRCH-007 — Search is invalidated by a new load, new query, or Clear
Priority: P3
Steps: complete a search (a Find All list, or `N of M` showing), then (a) load
a new document, or (b) run a new query (Results search), or (c) click `Clear` —
three sub-cases.
Expected: in all three, the completed search is discarded: the panel closes
(TC-SRCH-007c), the status line goes blank (TC-SRCH-007d), and the next Find
searches afresh instead of stepping on through stale matches.

### TC-SRCH-030 — Ctrl+F puts the cursor in the Find field
Priority: P1
Steps: click into a panel, press Ctrl+F, and type without clicking anything.
Expected: the text lands in the Find field (and `Find`/`Find All` un-dim).

### TC-SRCH-031 — Search… from a row's context menu focuses the field too
Priority: P1
Steps: right-click a row → `Search…`, then type without clicking.
Expected: as TC-SRCH-030.

### TC-SRCH-032 — Find steps through the matches one by one, and wraps
Priority: P1
Steps: `people.json`, search `age` (four matches, in document order: the three
`age` keys, then `.[2].role` = "manager"); press Find four more times.
Expected: `1 of 4` … `4 of 4`, each revealing and highlighting exactly that
row (only one row is highlighted at a time); the fifth press wraps to
`1 of 4 — wrapped around to the top`, highlighting the first match again.

### TC-SRCH-033 — Enter repeats Find and leaves the cursor in the field
Priority: P1
Steps: Ctrl+F, type `age`, press Enter twice — no mouse.
Expected: `1 of 4`, then `2 of 4`: the field keeps keyboard focus after each
Find, so Enter goes on finding the next match.

### TC-SRCH-034 — Changing the text starts again at the first match
Priority: P2
Steps: search `age`, Find once more (`2 of 4`), then search `role`.
Expected: `1 of 3` — the first `role` match, not a continuation of the `age`
stepping.

### TC-SRCH-035 — Escape closes the dialog
Priority: P2
Steps: open the dialog, press Esc.
Expected: the dialog closes (Cancel and the title-bar ✕ do too).

### TC-SRCH-036 — Ctrl+F on an already-open dialog selects its text
Priority: P2
Steps: open the dialog, type `zzz`, press Ctrl+F again, type `Alice`, press
Enter.
Expected: `1 of 1` — the second Ctrl+F re-focused the field with its text
selected, so typing replaced it (rather than `zzzAlice`, which matches
nothing). Ctrl+F aimed at the *other* tree instead retargets the dialog and
clears the field.

### TC-SRCH-038 — Enter still finds after ticking the Regex box
Priority: P2
Steps: open the dialog, type `^Ali`, click the `Regex` checkbox, press Enter.
Expected: `1 of 1` — ticking the box hands the keyboard focus back to the
field, so Enter means Find without a click on the field first.

### TC-SRCH-039 — Find with no match is reported in the dialog, not as an error
Priority: P2
Steps: `Find` for text that occurs nowhere.
Expected: red `No matches found.` in the status line; the dialog stays open; no
tree row is highlighted.

### TC-SRCH-040 — Find reveals the match in its tree
Priority: P1
Steps: `Find` for a value that occurs once.
Expected: `1 of 1`; the node is expanded into view, scrolled to and highlighted
in its owning tree.

### TC-SRCH-042 — Find All lists every match and leaves the dialog open
Priority: P1
Steps: `people.json`, `Find All` for `age`.
Expected: the bottom panel lists all four matches in document order; the status
line reads `4 matches`; the dialog stays open; **nothing** is revealed or
highlighted in the tree, and no list entry is selected.

### TC-SRCH-043 — Clicking a Find All entry reveals it; Find carries on from it
Priority: P1
Steps: as TC-SRCH-042, click the third entry (`.[2].age`), then `Find`.
Expected: the entry is highlighted and its row revealed in Source; Find then
reads `4 of 4` (`.[2].role`) — it continued from the entry clicked rather than
starting at `1 of 4`.

### TC-SRCH-044 — Find moves the highlight in a Find All list
Priority: P2
Steps: as TC-SRCH-042, then `Find` twice.
Expected: after the first, the first entry is highlighted (`1 of 4`); after the
second, the second entry is and the first is not — the list and the tree never
disagree about which match is current.

### TC-SRCH-045 — Find All over Results lists Results rows
Priority: P2
Steps: run `.[]` over `people.json`, click into the Results panel, Ctrl+F,
`Find All` for `bob`, click the hit line.
Expected: the dialog is titled `Search — Results`; the hit line is tagged
`[Results]`; clicking it reveals and highlights the row in the Results tree.

### TC-SRCH-046 — Find All with an invalid regex is reported in the dialog
Priority: P2
Steps: `Regex` checked, `Find All` for `(unclosed`.
Expected: red `Search error` in the dialog's status line, as for Find
(TC-SRCH-003); no list opens.

## B. "Find in Source"

Right-click a Results row → `Find in Source` works out, as a best attempt,
where that row came from in the Source document. The lookup
(`jsonquery_core::locate`, unit-tested in `crates/core/src/tree.rs`) matches the
row's own key and value — nothing about the rows around it:

1. **Key and value.** Every node whose value equals the row's is a candidate;
   those reached by the same trailing key/index as the row are preferred, so a
   repeated value is told apart by its key (a `zip` row doesn't match a `code`
   holding the same digits) and an array element by its index. If no node
   shares the key (the query renamed it), the value-only matches are kept.
2. **A text search** (the same matching as `Search…`) if nothing is equal —
   the value was computed. It searches for the value when that is a string
   (a lower-cased or sliced string still hits), else for the row's key.

Candidates are ordered best-first, and among equals the one whose innermost
array index is the row's output position in the results comes first (output 2
of `.users[]` prefers `users[2]`).

Outcome: **no candidate** → status bar `Not found in source.`; **one exact
candidate** → revealed directly, no list; **several exact candidates** → the
bottom panel opens as `Find in Source — {row path}` listing them (`[Source]
{path}   {preview}`), the first entry highlighted and revealed in Source;
**a text-search fallback** → the same panel, but only listed: nothing is
highlighted and Source doesn't move, since the answer is approximate. Clicking
a list entry reveals it and moves the highlight there.

GUI fixtures: `duplicates.json` (`country` "US" twice) and `sites.json` (the zips
`75001` and `00100` each also occur under another key, `depots[2].code` and
`hq.code`, listed before `sites`).

### TC-SRCH-020 — Success: a single exact hit is revealed directly
Priority: P1
Steps: run a query whose result value is identical (deep-equal) to exactly one
node in the loaded source document (e.g. a passthrough/select query with no
transformation), right-click that result row → `Find in Source`.
Expected: status bar briefly shows spinner + `Locating in source…`, then the
matching Source node is expanded/scrolled/highlighted and the Source panel
switches to Tree view (if it was on Text view). No bottom panel opens.

### TC-SRCH-021 — Failure: transformed/computed values report "not found"
Priority: P1
Steps: run a query that computes a value with no equal node in the source and
no string or key worth searching for (e.g. `.[0].name + "!"`), right-click a
result row → `Find in Source`.
Expected: status bar ends with weak `Not found in source.` — no error
styling (this is an expected, non-error outcome, not a failure state).

### TC-SRCH-022 — "Find in Source" is unavailable on Source-tree rows
Priority: P2
Duplicates the negative half of TC-CTX-001 — no separate test needed, just
confirmed here for completeness of this feature's requirements.

### TC-SRCH-023 — Several exact hits: listed, best one selected and revealed
Priority: P1
Steps: `duplicates.json`, query `.users[].country` (US, UK, US); right-click
result row 2 (the second "US") → `Find in Source`.
Expected: the bottom panel opens headed `Find in Source — .[2]` with two
entries, `.users[2].country` first; that entry is drawn highlighted and is
revealed in Source (Source row `users[2].country` highlighted).

### TC-SRCH-024 — Clicking another candidate moves the selection and the reveal
Priority: P1
Steps: as TC-SRCH-023, then click the second entry (`.users[0].country`).
Expected: the first entry loses its highlight, the second gains it, and Source
reveals `users[0].country`.

### TC-SRCH-025 — A transformed string falls back to a text search, listed only
Priority: P1
Steps: `duplicates.json`, query `.users[0].name | ascii_downcase` ("ann");
right-click result row 0 → `Find in Source`.
Expected: nothing equals "ann", so the panel opens with the one text-search hit
(`.users[0].name`); no entry is highlighted and the Source tree has not moved.

### TC-SRCH-026 — A new Search replaces a Find in Source list
Priority: P3
Steps: with a Find in Source list showing, `Find All` over Source.
Expected: the panel is headed `Search results …`, not `Find in Source …` — the
two features share the one bottom panel.

### TC-SRCH-041 — Find leaves a Find in Source list in place
Priority: P3
Steps: with a Find in Source list showing, `Find` over Source.
Expected: the panel is still headed `Find in Source …`; Find reports in its own
dialog (`1 of 1`) and reveals in the tree. Only Find All replaces the list.

### TC-SRCH-027 — A nested key/value row lists its same-key matches, nth selected
Priority: P1
Steps: `sites.json`, query `.sites[]`; expand result row 2 (Lyon) and
right-click its `zip` child (75001) → `Find in Source`.
Expected: exactly the nodes holding 75001 under the key `zip` are listed —
`.sites[2].zip` (the output's own position) then `.sites[0].zip` — the first
highlighted and revealed in Source. `depots[2].code` (same value, same index,
earlier in the document, but another key) is not a candidate.

### TC-SRCH-028 — A nested row's other match can be picked from the list
Priority: P1
Steps: as TC-SRCH-027, then click the second entry.
Expected: the list highlight moves to `.sites[0].zip` and Source reveals it.

### TC-SRCH-029 — A nested row whose key picks out one node jumps straight to it
Priority: P1
Steps: `sites.json`, query `.sites[1]`; expand result row 0 (Rome) and
right-click its `zip` child (00100) → `Find in Source`.
Expected: `sites[1].zip` is revealed directly with no list, although `hq.code`
(listed first) holds the same value — under another key.

### TC-SRCH-037 — The Find in Source panel closes
Priority: P3
Steps: with a Find in Source list showing, click the panel's `Close`.
Expected: the panel disappears.
