# jsonquery — architecture & design notes

A native desktop tool for querying and browsing large JSON files — fast to
open, fast to scroll, fast to query. This page covers the problem and goals;
[Architecture](architecture.md) covers how it's built, and
[Query Engines](query-engines.md) covers the supported query dialects.

## The problem

Poking at a large JSON file today means picking one of three bad trade-offs:
the `jq` CLI is fast and expressive but has no visual browser, so you're
re-running commands blind; a browser-based JSON viewer looks great on a 2 MB
file and then locks the tab on a 2 GB one; and general editors like VS Code
will open large files but treat them as text, not structure — no collapsible
tree, no query.

There isn't a tool that combines *"open anything, however large, instantly"*
with *"query it like jq, and see the result as a tree you can actually
scroll."* That's the gap this project fills.

## Goals

- **Open large files without flinching.** Drag-and-drop or file-picker,
  targeting files from a few KB up to multiple GB, without freezing the UI
  while it loads.
- **Query like jq.** A query field at the top of the window, running a
  jq-compatible query language against the loaded document (or JSON
  Pointer, JSONPath, or JMESPath — see [Query Engines](query-engines.md)).
- **Scroll like it's a text file, not a spreadsheet of doom.** Both the
  source tree and the results pane stay smooth — constant frame time —
  regardless of how many nodes exist off-screen.
- **Stay responsive.** Loading, parsing, and querying happen off the UI
  thread; a slow query can be cancelled by simply starting a new one.
- **Ship as one binary.** No runtime install, no bundled browser engine — a
  native window that opens fast.

## Non-goals

- Editing JSON in place (this is a viewer + query tool, not an editor).
- Schema validation / linting.
- Multiple files open at once, tabs, or session management.
- Remote or cloud-hosted files, beyond a plain HTTP(S) URL load.
- Other formats (YAML, CSV) — pure JSON in. NDJSON (one JSON value per line)
  is treated as a shape of JSON input, not a separate format — see
  [Architecture §2](architecture.md#2-file-ingest).

## Guiding principles

- **Never block the UI thread.** File I/O, parsing, and query execution all
  happen on a worker thread; the GUI only ever reads results handed back
  over a channel.
- **Never materialize what you don't need to render.** If a row isn't
  visible, it isn't drawn — see the virtualized tree widget in
  [Architecture §5](architecture.md#5-gui-layer--the-virtualized-tree-widget).
- **Reuse real query engines, don't invent one.** jq syntax is already
  muscle memory for the target user, and JSON Pointer/JSONPath/JMESPath are
  established standards — see [Query Engines](query-engines.md).

## High-level shape

One direction for data, one thread boundary. Everything else in the
architecture doc is a zoom-in on one of these boxes.

```
background worker thread:
  Opened/dropped/pasted file, or a URL
    -> Ingest (mmap, or bytes directly)
    -> Parse (serde_json)
    -> Query engine (jaq · JSON Pointer · JSONPath · JMESPath)

        | commands down, results streamed up — via channel
        v

UI thread (egui/eframe):
  Query bar · Source tree · Results tree
```

Full detail, including the virtualized tree widget and cancellation model,
is in [Architecture](architecture.md).

## Where to go next

- **[Architecture](architecture.md)** — the components, the concurrency
  model, the virtualized tree widget, and the crate/workspace layout.
- **[Query Engines](query-engines.md)** — jq, JSON Pointer, JSONPath, and
  JMESPath, picked via an auto-detecting query-bar toggle, plus the wider
  landscape surveyed for what to add next.
