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

### TC-SRCH-020 — Success: reveals the structurally-equal node in Source
Priority: P1
Steps: run a query whose result value is identical (deep-equal) to some node
still present in the loaded source document (e.g. a passthrough/select query
with no transformation), right-click that result row → `Find in Source`.
Expected: status bar briefly shows spinner + `Locating in source…`, then the
matching Source node is expanded/scrolled/highlighted and the Source panel
switches to Tree view (if it was on Text view).

### TC-SRCH-021 — Failure: transformed/computed values report "not found"
Priority: P1
Steps: run a query that computes/renames/aggregates a value with no
structurally-equal counterpart in the source (e.g. a jq expression that
constructs a new object), right-click a result row → `Find in Source`.
Expected: status bar ends with weak `Not found in source.` — no error
styling (this is an expected, non-error outcome, not a failure state).

### TC-SRCH-022 — "Find in Source" is unavailable on Source-tree rows
Priority: P2
Duplicates the negative half of TC-CTX-001 — no separate test needed, just
confirmed here for completeness of this feature's requirements.
