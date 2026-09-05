# Error and edge-case states — consolidated index

This is not a new set of test cases — it's a single reference table
consolidating every distinct error/edge-case *string and state* the app can
show, each of which already has an owning test case in one of the other
docs. Use this doc when reviewing coverage ("did we test every error path?")
rather than as a source of new implementation work.

| Situation | Exact text pattern | Owning test case |
|---|---|---|
| Malformed JSON (file/URL/paste) | `Load error: parsing JSON: {details}` | TC-OPEN-014, TC-OPEN-006, TC-TXT-008 |
| File can't be opened/mmap'd | `Load error: opening/memory-mapping/reading metadata for {path}: ...` | *(not yet assigned — needs a fixture that's e.g. permission-denied or a broken symlink; add during implementation)* |
| URL request fails | `Load error: requesting {url}: ...` | TC-OPEN-005 |
| URL body exceeds 4 GiB / download I/O error | `Load error: downloading {url}: ...` | TC-OPEN-007 |
| Downloaded body isn't valid JSON | `Load error: parsing data from {url}: parsing JSON: ...` | TC-OPEN-006 |
| Empty file (0 bytes) | *(no error — loads as empty array)* | TC-OPEN-013 |
| Save destination unwritable | `Save error: creating/writing {path}: ...` | TC-SAVE-007 |
| Saved row no longer exists (race) | `Save error: that value is no longer part of the document` | TC-SAVE-008 (flagged as possibly unautomatable) |
| jq syntax error | `Query error: query syntax error: ...` | TC-QRY-021 |
| jq compile/runtime error | `Query error: query error: ...` | TC-QRY-021 |
| jq per-item error (non-fatal) | `{N} item error(s) (last: {msg})` | TC-QRY-020, TC-TOOL-008 |
| JSON Pointer malformed | `Query error: query syntax error: a JSON pointer must be empty (whole document) or start with '/'` | TC-QRY-031 |
| JSON Pointer not found | `Query error: query error: no value at pointer '{p}'` | TC-QRY-032 |
| JSONPath syntax error | `Query error: query syntax error: ...` | TC-QRY-041 |
| JSONPath no matches | *(not an error — 0 result(s))* | TC-QRY-040 |
| JMESPath syntax error | `Query error: query syntax error: ...` | TC-QRY-051 |
| JMESPath runtime/type error | `Query error: query error: ...` | TC-QRY-051 |
| JMESPath "no match" | *(not an error — evaluates to a single `null` result)* | TC-QRY-050 |
| Invalid search regex | `Search error: {regex crate message}` | TC-SRCH-003 |
| Search with no hits | `No matches found.` | TC-SRCH-004 |
| Find-in-Source, no structural match | `Not found in source.` | TC-SRCH-021 |
| Popup field left blank | primary button stays disabled, no error text | TC-OPEN-004, TC-SRCH-001, TC-TXT-010 |

## Gaps identified while building this index

- **File-open OS errors** (permission denied, broken symlink, deleted-out-
  from-under-you) have no assigned test case yet. Add one under
  [02_opening_sources.md](02_opening_sources.md) during implementation if a
  reliable way to construct such a fixture exists in CI (e.g. `chmod 000` a
  fixture file before the test, restore after) — otherwise document as an
  intentionally-skipped case with the reason recorded in the traceability
  matrix, not silently dropped.
- Every "P3 — flagged during implementation" note scattered through the
  other docs (oversized URL download, save-race, permission errors) is a
  candidate for this same treatment: either implement with a real fixture,
  or explicitly mark skipped-with-reason so the traceability matrix stays
  honest about what's actually covered versus deliberately left out.
