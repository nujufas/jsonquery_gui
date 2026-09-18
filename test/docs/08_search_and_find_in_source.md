# Search and Find in Source — test requirements

Source: `crates/app/src/app.rs` (Search popup §, Search-results panel §,
Find-in-Source §), `crates/core/src/tree.rs`. These are two distinct
features that happen to share the word "find" — keep them in separate test
groups (`TC-SRCH-0xx` for Search, `TC-SRCH-02x` for Find in Source) to avoid
conflating them, per the source inventory's explicit warning that they're
unrelated.

## A. "Search…" (search-panel dialog + results panel)

### TC-SRCH-001 — Dialog fields and buttons
Priority: P2
Steps: open Search (via context menu or Ctrl+F) on either tree.
Expected: title `Search — {Source|Results}` per which tree; `Find:` field
with hint `text to find…`; `Regex` checkbox (unchecked by default); `Find
All` (disabled while the field is blank) and `Cancel` buttons.

### TC-SRCH-002 — Plain (non-regex) search is case-insensitive substring
match over keys and values
Priority: P1
Steps: search for a substring that matches a key in one place and a string
value in another (mixed case relative to the actual fixture content), with
`Regex` unchecked.
Expected: both the key-match and the value-match appear as hits, regardless
of case; number/bool/null values match against their string form (e.g.
searching `true` matches a boolean `true` value, searching `null` matches a
null value) — include a sub-case for this since it's an easy thing to miss
if search were naively "only string values".

### TC-SRCH-003 — Regex mode switches matching engine; invalid pattern is an
error
Priority: P2
Steps: (a) check `Regex`, search a valid pattern (e.g. an alternation or
anchor); (b) search a syntactically invalid pattern (e.g. `(unclosed`).
Expected: (a) matches per regex semantics against the same key/value texts;
(b) Search-results panel shows red `Search error: {details}` (message is the
underlying regex-crate parse error — treat as "non-empty error shown", not an
exact string match, since the crate's wording isn't part of this app's own
contract).

### TC-SRCH-004 — Results panel: header format, states, and Close behavior
Priority: P2
Steps: run a search that produces hits; observe header. Then run one that
produces none. Then click `Close`.
Expected: header reads `Search results — {Source|Results} "{query}"` (regex
mode appends ` (regex)`); hit count as weak `{N} match(es)` once done, or a
spinner while running; zero-hit case shows weak `No matches found.`
(distinct from the red error case in TC-SRCH-003 — don't conflate "0 matches"
with "search error"); `Close` hides the panel without clearing search state
(re-triggering Ctrl+F or a menu Search… reopens the popup fresh, but the
prior results panel's hidden state vs. cleared state is worth a quick check
if easy to distinguish at implementation time — low priority nuance).

### TC-SRCH-005 — Each hit line's format and click-to-reveal behavior
Priority: P1
Steps: produce a hit, inspect its line, click it.
Expected: line reads `[{Source|Results}]  {jq-style path}   {one-line value
preview}` (preview format matches tree-row rendering rules from
[05_tree_view.md](05_tree_view.md) TC-TREE-001); clicking it reveals/expands/
highlights that node in its owning tree and switches that panel to Tree view
(if it was on Text view).

### TC-SRCH-006 — 5,000-match cap
Priority: P3
Steps: search a term matching more than 5,000 keys/values in a large fixture.
Expected: results stop accumulating at 5,000, with **no** truncation notice
shown (unlike the Text-view 20,000-node budget, which does show a notice —
this is a deliberate asymmetry worth its own assertion so a future "helpful"
UI change that adds a notice here doesn't get treated as a false positive
regression without someone noticing the doc needs updating too).

### TC-SRCH-007 — Search is invalidated by a new load, new query, or Clear
Priority: P3
Steps: open a search results panel with hits showing, then (a) load a new
document, or (b) run a new query, or (c) click `Clear` — three sub-cases.
Expected: in all three, the search results panel/state no longer reflects
the stale search (either cleared or hidden — confirm exact behavior at
implementation time and tighten this assertion accordingly).

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
Steps: with a Find in Source list showing, run a `Search…` over Source.
Expected: the panel is headed `Search results …`, not `Find in Source …`.

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
