# Architecture

How data flows from an opened file to a rendered, queryable tree.

## 1. Pipeline overview

```
worker thread:
  File (path from drag-drop / picker / paste / URL)
    -> Ingest (memmap2, or the pasted/downloaded bytes directly)
    -> Parse (serde_json, arbitrary_precision for exact number round-tripping)
    -> Query exec (jaq · JSON Pointer · JSONPath · JMESPath)

        | crossbeam-channel: progress + results,
        | cancellable via generation counter
        v

UI thread (egui/eframe):
  Query bar · Source tree (virtualized) · Results tree (virtualized) · Status bar
```

## 2. File ingest

Files opened from disk are memory-mapped via `memmap2` rather than read
into a `Vec<u8>` up front, so opening a large file doesn't pay an eager
copy. Pasted text and URL downloads skip straight to parsing. Either way,
the bytes are parsed into an owned `serde_json::Value` with the
`arbitrary_precision` feature enabled — a 64-bit snowflake-style ID or a
Postgres bigint survives a query byte-for-byte instead of silently rounding
through an `f64` the way naive JSON tooling does.

NDJSON (one JSON value per line) is handled as a shape of JSON input from
the start, not a separate format — the file is treated as one-or-more
top-level values rather than assuming exactly one root, so a log-export-style
file works the same way a single large document does.

## 3. Query engines

`jsonquery_query::QueryEngine` is the trait every dialect implements —
`JaqEngine` (jq, via the embedded [jaq](https://github.com/01mf02/jaq)),
`JsonPointerEngine` (RFC 6901), `JsonPathEngine` (RFC 9535, via
`jsonpath-rust`), and `JmesPathEngine` (via the `jmespath` crate) all sit
behind it. A `Kind::detect(query_text)` best-effort auto-selects a dialect
from the query's own syntax (a leading `/` means JSON Pointer, a leading
`$` means JSONPath, a few JMESPath-only substrings mean JMESPath, everything
else defaults to jq) — or the toolbar's four engine buttons pin one
explicitly. See [Query Engines](query-engines.md) for the full comparison.

Two things fall out of embedding jaq specifically:

- **Exact number round-tripping** — see §2 above; jaq inherits it from the
  same `serde_json` value it evaluates over.
- **Short-circuiting queries** — jaq's evaluator is generator-based, like jq
  itself: every filter yields a lazy stream of outputs, so `first(...)`,
  `limit(n; …)`, and slicing stop pulling as soon as they have enough rather
  than always running to completion.

## 4. Concurrency model

The UI thread never touches the file or the query engines directly — it
sends commands and receives messages over a `crossbeam-channel` pair.

```
UI thread (egui/eframe)  --  Open(path)/Query(text, gen)/Cancel  -->  Worker thread
                          <--  Progress/Rows(gen)/Error         --   (ingest · parse · query)
```

Two things fall out of this cleanly:

- **The window never freezes.** Loading a file, rendering Text view, or
  running a query happens off-thread; the UI keeps redrawing (a spinner,
  progress) using the last message received.
- **Stale work is cheap to discard.** Every query carries a monotonically
  increasing generation number. If the user edits the query field and
  re-submits before the previous run finishes, the UI ignores any late
  results tagged with an old generation.

## 5. GUI layer & the virtualized tree widget

Built on `egui`/`eframe`. The window is a query bar pinned to the top, and
two instances of the same custom tree widget below it — the **source tree**
(the loaded document) and the **results tree** (the last query's output).

Expand/collapse state is *not* stored on the tree itself — it lives in a
side map keyed by node path. Every frame, the widget reads the current
scroll offset and viewport height, computes which row indices are visible
given a fixed row height, resolves and draws only those rows (following
expand state to skip collapsed subtrees), and reserves the correct total
scroll height up front so the scrollbar stays accurate without ever
visiting off-screen rows. The cost of one frame is bounded by *viewport
height*, not document size.

## 6. Results handling

Query results reuse the exact same virtualized tree widget, populated
differently from the source tree:

- **Streamed, not collected.** The query executor pushes result items to
  the UI incrementally as jaq (or another engine) produces them, rather
  than collecting the full output before returning anything.
- **Bounded live preview, with an escape hatch.** The rendered preview caps
  at a configurable node count; an **Expand All** action re-runs the query
  unbounded when a truncated preview isn't enough, and Search…/Save… over
  results transparently defer to that unbounded rerun too.
- **Full export bypasses the cap.** "Save results to file" re-runs the
  query in a streaming write mode straight to disk, so exporting a large
  matched set never requires materializing it all at once in RAM.

## 7. Crate map

| Crate | Role |
|---|---|
| `eframe` / `egui` | Native GUI shell and immediate-mode widget rendering. |
| `egui_extras` | Reference implementation for row virtualization patterns. |
| `memmap2` | Memory-map file-opened input for zero-copy, lazy-paged access. |
| `serde_json` | Parsing and the in-memory value model, with `arbitrary_precision` enabled for exact number round-tripping. |
| `jaq-core` / `jaq-std` / `jaq-json` | Embedded jq-compatible query language and evaluator. |
| `jsonpath-rust` | JSONPath (RFC 9535) engine. |
| `jmespath` | JMESPath engine. |
| `crossbeam-channel` | UI ⇄ worker-thread messaging. |
| `rfd` | Native file dialog behind the toolbar's "…" button, alongside OS-level drag-and-drop (handled directly by egui/winit). |
| `ureq` | Loading a document from a URL. |
| `anyhow` / `thiserror` | Error handling — typed errors at API boundaries, contextual errors in the app layer. |

## 8. Workspace layout

A Cargo workspace with the core data layer and the query engines kept as
separate crates from the GUI, so they can be unit-tested and benchmarked
without pulling in a GUI toolkit:

```
jsonquery/
├── Cargo.toml                # workspace
├── crates/
│   ├── core/                 # file ingest, the tree data layer
│   ├── query/                # the four query engines + suggest.rs (autocomplete) + highlight.rs (query colouring)
│   └── app/                  # eframe app: panels, virtualized tree widget, worker thread
├── docs/                     # this site
└── benches/                  # criterion benchmarks against synthetic large files
```

## Scaling beyond in-memory

Today's pipeline parses the whole document into memory up front (§2), which
is solid for small-to-medium files but means a multi-gigabyte document pays
full parse time and memory at open. Scaling further would mean replacing
that with a memory-mapped, lazily-resolved index (a sparse checkpoint table
over the mapped bytes, so any row resolves in bounded time regardless of
document size) sitting behind the same `QueryEngine` trait — not yet built.
