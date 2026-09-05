# Query bar and query engines — test requirements

Source: `crates/app/src/app.rs` (query bar §), `crates/query/src/lib.rs`,
`jq.rs`, `json_pointer.rs`, `jsonpath.rs`, `jmespath_engine.rs`.

This is the app's core value proposition — correctness here matters more than
anywhere else in the suite. Exact error-message text is cross-referenced in
[11_error_and_edge_cases.md](11_error_and_edge_cases.md); this doc owns the
*behavioral* contract per engine (what counts as 0 results vs. an error, etc.)
and the picker/run/cancel mechanics.

## Fixture data note (implementation detail, flagged here since it affects
every case below)

Most cases need a small, shared, well-known JSON fixture with objects, arrays,
strings, numbers, booleans, and null, so query expressions in this doc's
"Steps" can be written concretely at implementation time rather than as
placeholders. Recommend one canonical fixture (e.g. a small list-of-people
object, matching the flavor of the example expressions already used in the
app's own hover-tooltips and hint text — see engine picker tooltips below)
reused across TC-QRY-*.

## Engine picker mechanics

### TC-QRY-001 — Picker shows exactly 4 engines, in order, with correct
tooltips
Priority: P2
Steps: Load a document, inspect the engine picker row.
Expected: left-to-right: weak `Engine:` label, then toggle buttons `jq`,
`Pointer`, `JSONPath`, `JMESPath`, in that exact order. Hovering each shows:
- jq: `jq — e.g. .[] | select(.age > 21) | .name`
- Pointer: `JSON Pointer (RFC 6901) — e.g. /store/book/0`
- JSONPath: `JSONPath (RFC 9535) — e.g. $.store.book[*].author`
- JMESPath: `JMESPath — e.g. people[?age > \`30\`].age`

### TC-QRY-002 — Selecting an engine explicitly, then re-clicking deselects
back to auto
Priority: P1
Steps:
1. Click `JSONPath` (becomes visibly selected/highlighted).
2. Run a query with a jq-style expression (e.g. `.[]`) while `JSONPath` is
   still selected.
3. Click `JSONPath` again (deselecting it).
4. Run the same query again.
Expected:
- Step 2: query is forced through the JSONPath engine regardless of syntax —
  since `.[]` is not valid JSONPath, expect a syntax error, status bar engine
  suffix `[JSONPath]` (explicit, no `· auto`).
- Step 4: with no engine explicitly selected, auto-detect applies (`.` prefix
  → jq), query succeeds, engine suffix `[jq · auto]`.

### TC-QRY-003 — Query text box hint text shown only when empty
Priority: P3
Steps: Load a doc, inspect the query box before typing anything.
Expected: hint text `e.g. .[] | select(.age > 21) | .name` on line 1, then
`(auto-detects jq / JSON Pointer / JSONPath / JMESPath — or pick one at top
right)` on line 2; hint disappears once any character is typed, including a
single space.

## Auto-detection (`Kind::detect`, no engine explicitly picked)

### TC-QRY-010 — Auto-detect: empty query or leading `.` → jq
Priority: P1
Steps: with no engine selected, run with an empty query box, then separately
with `.name`.
Expected: both run through jq; result suffix `[jq · auto]` in both cases;
empty query returns the whole document as the single result.

### TC-QRY-011 — Auto-detect: leading `/` → JSON Pointer
Priority: P1
Steps: run `/name` (or a path valid for the fixture) with no engine selected.
Expected: routed to Pointer engine, suffix `[Pointer · auto]`.

### TC-QRY-012 — Auto-detect: leading `$` → JSONPath
Priority: P1
Steps: run `$.name` with no engine selected.
Expected: routed to JSONPath, suffix `[JSONPath · auto]`.

### TC-QRY-013 — Auto-detect: contains `[?`, `&&`, `||`, or backtick → JMESPath
Priority: P1
Steps: four sub-cases, one per trigger substring, e.g. `people[?age > \`30\`]`,
an expression using `&&`, one using `||`, and one using a raw backtick
literal.
Expected: all four route to JMESPath, suffix `[JMESPath · auto]`.

### TC-QRY-014 — Auto-detect: none of the above (bare identifier path) falls
back to jq
Priority: P2
Steps: run a bare path like `name` (no leading `.`, `/`, `$`, and none of the
JMESPath trigger substrings) with no engine selected.
Expected: routed to jq (jq accepts bare-looking paths as invalid syntax or as
a valid jq expression depending on content — confirm the specific fixture
expression's actual jq validity during implementation and phrase the expected
result, success or syntax error, accordingly; the requirement being tested
here is only *which engine* handles it, not the outcome).

## Per-engine correctness and error semantics

### TC-QRY-020 — jq: streams multiple results, per-item errors don't abort
the run
Priority: P1
Steps: with `jq` explicitly selected, run `.[] | .foo` against a fixture
array mixing objects (some with a `.foo` key, some without, and some
non-object items).
Expected: items with `.foo` present stream into the results tree; the status
bar shows `{N} item error(s) (last: {msg})` in orange for the failing items;
the run still completes and reports a final elapsed time (it does not treat
per-item failures as a fatal query error).

### TC-QRY-021 — jq: parse error vs. compile error message prefixes
Priority: P2
Steps: (a) run visibly malformed jq syntax (e.g. unbalanced `|(`); (b) run
syntactically valid but semantically invalid jq (a real compile-time error
case — confirm a concrete example against the embedded `jaq` engine during
implementation).
Expected: (a) `Query error: query syntax error: {details}`; (b) `Query error:
query error: {details}`.

### TC-QRY-022 — jq: Cancel stops the run; a brief settle window is expected
Priority: P2
Steps: run a jq query designed to take a noticeable amount of time (e.g. over
a large fixture with an expensive expression), click `Cancel` shortly after
it starts.
Expected: button/spinner state flips to not-running promptly; final status
bar text includes ` — cancelled`; result count may continue to tick up
briefly after the click before settling (this is expected, per the strategy
doc's race note) — assert on the *final* settled state via
`Wait Until Keyword Succeeds`, not the instant of the click.

### TC-QRY-030 — JSON Pointer: exactly 0 or 1 result, never streamed/partial
Priority: P1
Steps: with `Pointer` selected, run a pointer that resolves to a value.
Expected: exactly one result item.

### TC-QRY-031 — JSON Pointer: malformed pointer is a syntax error
Priority: P1
Steps: run a pointer not starting with `/` and not empty (e.g. `foo`).
Expected: `Query error: query syntax error: a JSON pointer must be empty
(whole document) or start with '/'`.

### TC-QRY-032 — JSON Pointer: well-formed but unresolvable pointer is a
query error, not "0 results"
Priority: P1
Steps: run `/does/not/exist` against the fixture.
Expected: `Query error: query error: no value at pointer '/does/not/exist'`
— explicitly **not** a plain `0 result(s)` outcome. This distinction (query
error vs. empty result) is the main behavioral risk for this engine — write
it as an explicit negative assertion (status bar is red/error-styled, not the
neutral result-count styling).

### TC-QRY-033 — JSON Pointer: empty pointer returns the whole document
Priority: P2
Steps: run `` (empty string) with `Pointer` explicitly selected (note: an
empty query with *no* engine selected auto-detects to jq per TC-QRY-010 —
this case requires the engine to be explicitly forced to Pointer to actually
exercise this path).
Expected: one result, identical to the whole loaded document.

### TC-QRY-040 — JSONPath: 0 matches is not an error
Priority: P1
Steps: run a syntactically valid JSONPath expression against the fixture that
matches nothing (e.g. a path into a key that doesn't exist).
Expected: neutral (non-error) status bar text `... — 0 result(s) ...`; no red
error text.

### TC-QRY-041 — JSONPath: multiple matches stream, and malformed syntax is
a parse error
Priority: P1
Steps: (a) run a JSONPath wildcard expression matching several elements; (b)
run malformed JSONPath syntax.
Expected: (a) N results in the tree, N in the status count; (b) `Query error:
query syntax error: {details}`.

### TC-QRY-050 — JMESPath: always exactly 1 result, even a "no match" that
evaluates to null
Priority: P1
Steps: run a JMESPath expression that would conceptually "find nothing" but
is valid JMESPath (e.g. indexing into a nonexistent key, which JMESPath
evaluates to `null` rather than raising an error).
Expected: exactly 1 result item, whose value is `null` — **not** `0
result(s)` and **not** an error. This is the most counter-intuitive
behavioral contract in the whole engine set relative to the other three —
give it its own explicit test rather than folding it into a generic
"per-engine smoke test".

### TC-QRY-051 — JMESPath: parse error vs. runtime/type error message
prefixes
Priority: P2
Steps: (a) malformed JMESPath syntax; (b) a type error at evaluation time
(e.g. applying a numeric filter expression to a string field — confirm a
concrete example during implementation).
Expected: (a) `Query error: query syntax error: {details}`; (b) `Query error:
query error: {details}`.

## Run mechanics

### TC-QRY-060 — Run button disabled with no document loaded
Priority: P2
Steps: on fresh launch (no doc), inspect the Run button.
Expected: disabled.

### TC-QRY-061 — Ctrl+Enter runs the query regardless of which control has
focus (global shortcut)
Priority: P2
Covered jointly with [10_keyboard_shortcuts.md](10_keyboard_shortcuts.md)
TC-KEY-001; listed here as a cross-reference since it's specifically a query
bar behavior, not duplicated as a separate test case.

### TC-QRY-062 — 50,000-item live preview cap: status bar reflects true count
vs. shown count
Priority: P3
Steps: run a query producing more than 50,000 result items against a large
fixture (implementation detail: needs a suitably large/generated fixture —
see also [16, large file handling] discussion carried in the strategy doc).
Expected: status bar shows the true total count and a `, {shown} shown (live
preview capped at 50000)` clause with the literal number 50000; results tree
only contains the capped subset; `Save…` on Results saves only the capped
subset (cross-reference [09_saving.md](09_saving.md)).
