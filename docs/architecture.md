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

Several files can also be combined into one document: the Tools window's
**Merge JSON** (see [Tools](tools.md)) reads them on the worker thread, runs a
jq filter over them with the same embedded engine (`jq -s` style: the files
slurped into one array), and hands the result to the app as an in-memory
document (`Document::from_value`, `DocumentSource::Merged`). It parses every
file up front, so it is capped at 128 MB of input in total and is separate from
the memory-mapped path above.

The window's other tools work the same way: **Format**, **Diff**, **Patch** and
**Validate schema** are functions over `serde_json::Value` in `crates/query`
(`reformat`, `diff` — which also lays the two documents out side by side from
the same edit tree the patch comes from (`diff/view.rs`), so what is marked is
what is in the patch — `patch` — RFC 6902 and RFC 7386 — and `schema`, a thin
layer over the `jsonschema` crate built without its network and file features,
so a `$ref` can't make the app fetch anything). The app runs them on the worker
thread as one kind of job (`Command::Tool`, answered by `Event::ToolDone`), with
the open document shared rather than copied; a patched document opens as
`DocumentSource::Derived`.

Moving a difference in the side-by-side view to the other document is the same
kind of job: every difference gets a number as the view lays it out (`Row::change`),
and `diff::take_changes` walks the two documents again in the same order (the
same `object_items` and `array_items`, so the numbers agree) and builds the one
that changes with those differences taken from the other. The Diff job does that
first and then compares again, and its answer carries the new text for the page
to put back in the document's box.

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

The query bar is as tall as the user drags it, and the query box scrolls
inside it. egui stores a resizable panel's size as the rect of its *content*,
so a box that grew with a long query would set the panel's height too (and
undo every drag back to it); the box is therefore a vertical scroll area that
fills whatever room the panel has, with its border drawn fixed around it.

The Source and Results panes (and the boxes of the Tools window) each begin with
the same header (`crates/app/src/pane_header.rs`): a small title, what goes after
it, and the buttons pinned at the right, with nothing drawn under it — the panel's
edge above is the only line. Each of the three panes (`Query`, `Source`,
`Results`) can also be popped out into a window of its own
(`crates/app/src/dock.rs` holds the state and the layout decisions; `app.rs`
draws). The toolbar's source field (with …, Load and Clear) stays in the main
window, so the window of a popped-out Source pane has a row of its own on top with
the same field and buttons (`App::source_field`, one text in `source_input`). A popped-out pane is an egui *deferred
viewport*: eframe redraws its window by itself and calls back into the app, which
`Shared` keeps behind a lock (`Arc<Mutex<App>>`) so that the window's frame and
the main window's frame both have plain `&mut App` access. It has to be deferred
rather than immediate (drawn inside the main window's frame): a Wayland compositor
sends no redraw callbacks to a window that is completely covered (GNOME does), so
a pane maximized over the main window would otherwise stop responding together
with the window under it. Hence a window's frame does what the main window's frame
would have done for it — drains the worker's events, applies the dock changes it
asked for — and a docked pane's window, which only the main window's next frame
can end, minimizes itself if that frame does not come. Where there are no real
windows (embedded viewports, and the headless layout tests) the pane is drawn as
an immediate viewport instead. The other windows — the Tools window, the tutorial
and About (`crates/app/src/app/satellites.rs`) — are deferred viewports for the
same reason and run the same kind of frame (`App::satellite_frame`: the worker's
events, the requests the window makes of the app, and — a closed window being
ended only by the main window's next frame — un-maximizing a window that is closed
from over the main window and minimizing one that is not dropped); the worker's
wake-up asks each of them for a frame too. Two
rules keep the drawing sound: a pane's widget ids come from the pane
(`UiBuilder::id`), not from the panel or window around it, so scroll positions
and text cursors survive a move; and because those ids are global, a pane must be
drawn in exactly one place per frame — so dock changes requested during a frame
are applied after it, never in the middle of it. Every window handles its own
keyboard input (`handle_shortcuts`), and a dialog is drawn in the window of the
pane it belongs to.

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

The same worker-offload discipline extends to single-row actions on either
tree: right-click **Copy to Clipboard** or **Save…** resolves and serializes
just that one node on the worker thread, so picking one branch out of a huge
document never requires cloning the whole thing first. Above a size
threshold, Copy to Clipboard asks for confirmation before putting that much
text on the clipboard.

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
| `jsonschema` | JSON Schema validation for the Tools window's Validate schema (drafts 4–2020-12), without its HTTP and file features. |
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
│   ├── query/                # the four query engines + suggest.rs (autocomplete) + highlight.rs (query colouring) + the Tools window's logic (merge, reformat, diff, patch, schema)
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
document size) sitting behind the same `QueryEngine` trait — not yet built. One thing to keep
in mind when it is: a merged document (above) is several files, so its root is a
list of per-file segments, each with its own mapping and index — a handle into
a lazy document should say which file it is in, not assume one byte range.
