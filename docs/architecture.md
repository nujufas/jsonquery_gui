# Architecture

How data flows from an opened file to a rendered, queryable tree.

## 1. Pipeline overview

```
worker thread:
  File (path from drag-drop / picker / paste / URL)
    -> Ingest (read; from 256 MiB, memmap2; or the pasted/downloaded bytes directly)
    -> Parse (serde_json, arbitrary_precision for exact number round-tripping)
       — or, for a file of 256 MiB or more, Index (a checking scan that notes where
       the big containers' children are; the file stays on disk, see Scaling below)
    -> Query exec (jaq · JSON Pointer · JSONPath · JMESPath)

        | crossbeam-channel: progress + results,
        | cancellable via generation counter
        v

UI thread (egui/eframe):
  Query bar · Source tree (virtualized) · Results tree (virtualized) · Status bar
```

## 2. File ingest

A file opened from disk comes in one of two ways, by its size
(`jsonquery_core::LAZY_THRESHOLD`, 256 MiB, which is where the user's setting
starts: `LoadLimits`, from the app's [settings](settings.md)). A smaller one is read into memory
and parsed into an owned `serde_json::Value` with the `arbitrary_precision`
feature enabled — a 64-bit snowflake-style ID or a Postgres bigint survives a
query byte-for-byte instead of silently rounding through an `f64` the way naive
JSON tooling does. A `Document` then holds the value and nothing of the file.

A larger one is not parsed: the tree it would make takes some twelve to
seventeen times the file, which is more than a machine has to spare for a file of
a gigabyte. It is memory-mapped via `memmap2`, checked once and indexed, and the
`Document` keeps the mapping for as long as it is open (see
"Scaling beyond in-memory" below). That has costs a
read has not: Windows will not replace a file that is mapped, and where a file
can be cut short by another process while it is open (Linux and macOS) the pages
past the cut are gone, which is a SIGBUS for whoever reads one. A handler for
that signal (`lazy/guard.rs`) is all that stands between it and the end of the
process: it replaces the mapping from the page that faulted on by zeros, marks
the document as damaged (`LazyTree::damaged`), and lets the read go on. The
status bar then says that the file was changed on disk, and a query, a save, a
copy and a search are refused, rather than made from zeros. Only a file too big to
parse is kept mapped at all.

What is not a regular file (a named pipe, the app's own standard input as
`/dev/stdin`, a file of `/proc`: they say they are empty however much they hand
over) and what sits on a filesystem that cannot be mapped are read to the end,
whatever their size — and, if that is 256 MiB or more, indexed in the memory they
were read into (the size of the file, not twelve times it). A pipe has no length
to go by and may have no end, so it is read only up to 4 GiB, the ceiling a URL
download has; and a device (`/dev/zero`, a terminal) is refused, as no file of
JSON, where reading it would never end. A pipe that nothing writes to keeps the
worker waiting for a writer, as opening one always has.

Pasted text and a URL download of under 256 MiB skip straight to parsing; a
larger download is streamed into a temporary file (readable by its owner only,
and with no name from the moment it is made: unlinked on Unix, made to be
deleted when it is closed on Windows), which goes through the same loader, so
that a download is indexed as a file is. The result of a merge is kept the same
way when it is that big (below).

NDJSON (one JSON value per line) is handled as a shape of JSON input from
the start, not a separate format — the file is treated as one-or-more
top-level values rather than assuming exactly one root, so a log-export-style
file works the same way a single large document does.

Several files can also be combined into one document: the Tools window's
**Merge JSON** (see [Tools](tools.md)) reads them on the worker thread, runs a
jq filter over them with the same embedded engine (`jq -s` style: the files
slurped into one array), and hands the result to the app as an in-memory
document (`Document::from_value`, `DocumentSource::Merged`). It parses every
file up front, so it is capped at 128 MB of input in total; each file is read the
way a file opened on its own is. A result of 256 MiB or more (the size from which
a file is kept on disk, a setting) is not kept as a tree: it is printed, a piece
at a time, to a temporary file like a big download's, and the document is that
file, mapped and indexed (`worker::keep_merged`), so what the app keeps of it is an
index, not twelve times its size. The merge itself still needs the room for the
files and the value jq makes of them; what the file saves is what is kept
afterwards, in the page and in the main window it is opened in. (Measured, release
build, a result of 49 MB from 25 MB of files: counting what Save… would write takes
0.09 s, printing it 0.09 s and mapping and indexing it 0.05 s, against 1.6 s for the
merge; the tree's memory is freed once the file is written, but a process often keeps
some of what it has freed, so the size of the app in a task manager falls back only in
part, 1.37 GB to 0.91 GB there.)

The window's other tools work the same way: **Format**, **Diff**, **Patch** and
**Validate Schema** are functions over `serde_json::Value` in `crates/query`
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
and `diff::take_selected` (`take_changes` is its one-run form) walks the two
documents again in the same order (the same `object_items` and `array_items`, so
the numbers agree) and builds the one that changes with those differences taken
from the other — any set of them, as runs of numbers, so that an arrow can move a
whole difference and a selection can move some of its lines. The Diff job does
that first and then compares again, and its answer carries the new text for the
page to put back in the document's box (which then keeps the name of the file it
came from, says it is changed, and is written to a file by the Save… of its
column through the same `SaveText` request as any other text).

Typing over a line of either column is the same kind of job (`tools/line_edit.rs`).
Where a line is in its document is read from the lines of its column — two spaces to
a level, so the openers above it, a level up each, are what holds it, a member's line
says its name and an element's place is how many elements come before it — and so
it is a path of names and counts, which is the same in the right document, whose
members the view shows in the left one's order. What is typed is parsed in the
place it goes (a member, an element, the name of a line that opens something) in
the page, so that a mistake is said at the line; the worker (`Job::Diff`'s `edit`)
makes it in the document as it is parsed — members keep their order, numbers their
spelling — and compares again, and the answer, like a move's, carries the new text.

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

jaq is not jq, though, and a few things people paste from the jq manual are not
in it, or mean something else in it. `crates/query/src/jq_ext.rs` adds and changes
them in the two forms jaq's own builtins take: definitions written in jq
(`jq_ext/prelude.jq`: `IN`, `INDEX`, `JOIN`, `fromstream`, `truncate_stream`, and
jq's own `scan`, `join`, `from_entries`, `with_entries`, `del`, `delpaths` and
`pick`) and native Rust filters (`@csv`, `@tsv` and `tostream`, native for speed
and for byte-level work, and the ones below). `run_query_with_vars` — the one place
a jq program is compiled, and the Merge tool's too — chains them after jaq's own.
They were checked against jq 1.8.1's output, and the cases are in the module's
tests, so they hold on a machine without jq. What still differs is the list of
known issues (§10).

A definition in the prelude takes the place of one of jaq's of the same name and
number of arguments, but only for what is written after it: jaq's `with_entries`
calls the `from_entries` that was there when it was written, so it is written again
below the new one. Of jaq's native filters, those that are written again here are
left out of the list the compiler is handed, as the first filter of a name is the
one called, and a definition of jaq-json's (`tonumber`) is left out too, as a
definition is found before a native filter.

- **`fromjson`** (`jq_ext/json_text.rs`) reads exactly one value, as jq's does.
  jaq's read a sequence, and
  on text it could not read answered with an error and tried the same place
  again, for ever: `"abc" | fromjson` and `"abc" | tonumber` never ended, the
  worker thread was lost with them and the memory grew by a gigabyte a second.
  The reader calls itself once for every array inside another, so text nested
  more than 1,000 levels deep (jq: 10,000) is refused before it is read.
- **`tonumber`** is what jq's is, a number made of the parts of one (`+5`, `007`,
  `.5`, `1e3`, `nan`, not `" 5"`), not `fromjson` and a look at the type.
- **`tojson`** stops at the depth jq stops at when it prints, 256 levels, and
  writes `<skipped: too deep>` for what is below.
- **`@base64d`** (`jq_ext/base64.rs`) decodes as jq does: a missing `=` at the end
  is no error (a JWT segment has none, and jaq's refused it), the first `=` ends
  the text, and what is not UTF-8 becomes one U+FFFD for each broken sequence,
  as jq's own reader of UTF-8 takes it.
- **`delpaths`**, and `del` with it (`jq_ext/paths.rs`, called by the prelude as
  `_delpaths`): the paths are sorted, and everything one level names is deleted
  in one pass, as jq does it. jaq deleted them one after the other, so `del(.a[0],
  .a[1])` took the wrong item, `del(.[1,2])` failed on the item that was no longer
  there, and a key taken out moved the last key into its place.

Assignments cannot be mended from outside jaq. `=`, `|=` and the rest call
`map_index` of jaq-json's `Val` for every step of a path, and that is where a
value put into `null` (which is what is under every key that is not there yet), or
past the end of an array, was an error. `third_party/jaq-json` is jaq-json 2.0.3
with that function, and its sibling for slices, changed: `null` becomes the object
or the array that the key or the index asks for, an index past the end makes the
array longer with `null`s (as far as 16,777,215; jq's 536,870,911 would be eight
gigabytes), a key of an object that is not text is an error, and the key that
`|= empty` takes out leaves the others where they are. It is a path dependency of
`crates/query`, and the workspace `exclude`s it, so that formatting, clippy and the
tests of the workspace leave it alone; its `JSONQUERY.md` lists the changes and
says how to compare it with the original.

A program can make a value as deep as it likes, and jaq writes one as text by
recursion, a kilobyte of stack for each level: a thread's 2 MiB ended the process
at about 1,800 levels. So a result is shown down to 127 levels, as deep as a
document can be (`convert::from_val`, which then cuts what is below, as jq does at
256), and the threads that run queries are started with 256 MiB of stack
(`jsonquery_query::spawn_query_thread`: address space, used as it is needed).

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

The Tools are the one satellite that is not always a window. The ⚓ button at the right
end of their row of tabs (`tools::dock_button`) *docks* them: the main window then has a
row of tabs (`App::tab_strip`, with `Tools::main_tabs`) — Json Query, which is the
window as it is without them, and one for each tool — and draws what the tab in front
holds: either Json Query's rows (the source row, the query bar, Source and Results), or
the page of a tool in all the room under the row of tabs (`Tools::docked_page`), with
the main window's status bar saying what the page says (`Tools::status_bar`) and, beside
the ⚙, the 🗗 (`tools::undock_button`) that lets them out again. `Tools`
(`tools.rs`) keeps where they are (`Place`), which tab is in front, and a move that was
asked for during a frame (`request_place`), which is made once the frame is drawn
(`apply_place_request`) for the reason a pane is moved after the frame: the pages'
ids come from `Tools` and not from the window or the main window (`UiBuilder::id`, as
for the panes), so a page must be drawn in one place per frame, and it keeps its
scroll positions and carets across a move. The pages do not know the difference:
what is in front owns the keys and the files dropped on the window
(`handle_shortcuts` leaves Ctrl+Enter, Ctrl+F and Ctrl+S to the page, in the main
window — a pane's window has its own input and is not affected — and
`handle_drag_and_drop` leaves it the files), and whatever puts something in
the Json Query tab — a document the Tools made, a value to show, a lesson of the
tutorial, a list of hits — brings that tab forward (`Tools::show_query`). Where the
Tools are is kept like the sizes of the other panes (`settings/interface.rs`). A
window docked from over the main window un-maximizes itself and brings the main
window forward, as a closed one does.

Expand/collapse state is *not* stored on the tree itself — it lives in a
side map keyed by node path. Every frame, the widget reads the current
scroll offset and viewport height, computes which row indices are visible
given a fixed row height, resolves and draws only those rows (following
expand state to skip collapsed subtrees), and reserves the correct total
scroll height up front so the scrollbar stays accurate without ever
visiting off-screen rows. The cost of one frame is bounded by *viewport
height*, not document size.

The right-click menu of a text is `crates/app/src/text_menu.rs`, because egui gives
a text box its keys (Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+A) and no menu. `attach` gives a
`TextEdit` Cut, Copy, Paste and Select All (Copy and Select All when the text is only
shown); `copy_only` gives a label or a row of a list the one item Copy. Three
things in egui had to be worked around, and a change to the menu has to keep them:
a press of the secondary button puts the caret where it landed and so throws a
selection away (`keep_selection` puts the range back when the press was inside it);
a press on a menu item takes the focus from the box, so Cut and Paste give it back
and then ask the window system for the same event the key makes
(`ViewportCommand::RequestCut`, `RequestPaste`: it is the only way for a menu to read
the clipboard); and a menu opened inside an `egui::Popup` closes that popup, so there
is none in the ⚠ notice. The text views (Results, Source, the previews of the Tools)
are read-only `TextEdit`s (`text_menu::read_only`; a tool's error message is one in the
window's own font, `read_message`), not selectable labels, because a label's selection is
private to egui and cannot be kept through the press either.
The line a Diff view is typed over in keeps its field while its menu is open
(`Editor::menu_took_focus`).

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
- **The output format.** Results are JSON, and Copy to Clipboard, Save… and
  the Text view write JSON, unless the query is a jq program whose last stage is
  `@csv` or `@tsv`. Then each result is a row of text, and a quoted, escaped
  JSON string of it (`"\"Ada\",36"`) is no use to anyone, so those three write
  the rows as they are: Copy on a row copies the row, on the results root a line
  per result; Save… writes a line per result, to a file named `.csv` or `.tsv`
  by default; the Text view shows the same text. The format is read off the
  query when it runs (`OutputFormat::detect` in `crates/query/src/output.rs`):
  only the last stage counts, so `map(@csv) | length` is still JSON. The
  Results header carries a `CSV`/`TSV` note while it applies, and the Tree view
  stays the JSON tree.

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
| `memmap2` | Memory-map a file of 256 MiB or more, which stays on disk and is read as it is looked at. |
| `memchr` | Finding the end of a string in the bytes of a file, quickly. |
| `serde_json` | Parsing and the in-memory value model, with `arbitrary_precision` enabled for exact number round-tripping. |
| `jaq-core` / `jaq-std` / `jaq-json` | Embedded jq-compatible query language and evaluator. |
| `jsonpath-rust` | JSONPath (RFC 9535) engine. |
| `jmespath` | JMESPath engine. |
| `jsonschema` | JSON Schema validation for the Tools window's Validate Schema (drafts 4–2020-12), without its HTTP and file features. |
| `crossbeam-channel` | UI ⇄ worker-thread messaging. |
| `ring` | The SHA-256 of what the workspace keeps and the random bytes of an item's id (already there, under `ureq`'s TLS). |
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

## 9. Settings

What the user can set is `settings.rs` in the app crate, and how it gets to where
it is used:

- **`settings/limits.rs`** — the limits on file sizes: `Limit` (the nine of them,
  with their name in the file, label and explanation, which the window shows as
  a tooltip; `Limit::MAIN` is the one it shows at first, the others are
  `is_advanced`), `FileLimits` (their values;
  the defaults are taken from where each was defined, `jsonquery_core::LoadLimits`
  and `jsonquery_query::lazy::Limits`, so that there is one place a default is
  changed), and how a size is typed and written (`parse_size`, `size_text`).
- **`settings/interface.rs`** — what the window looks like: the theme, whether the
  query box suggests, the size of the window and of the query panel and Source.
- **`settings.rs`** — `Settings` (both), and `Store`, which reads and writes the
  one file, `~/.jsonquery/settings.json` (`JSONQUERY_HOME`), writing only what
  differs from the defaults, and never failing the app: what cannot be read or
  used is said (`Store::notes`) and left as the default; what cannot be written is
  said (`Store::error`) and the settings last for the run.
- **`settings_window.rs`** — the Settings window (a satellite window, like About):
  the one main limit, and the others under a collapsing *Advanced*. It is small
  and says little (tooltips), and is made as tall as the Advanced limits need
  when they are opened, and as it was when they are closed (a `ViewportCommand`
  sent once they have slid fully open or shut; a window embedded in the main
  one, where there is no real window to resize, opens tall enough for them).

`main` reads the settings before the window opens, which is opened as it was left
(`main_window`), and hands them to the app. A limit reaches the code that holds
to it as a parameter and not a global: the worker thread starts with the limits
and takes new ones by `Command::UseLimits` (it loads files, downloads, merges,
runs tool jobs, copies and runs queries by them: `load_with`, `download`,
`merge_files`, `jobs::run`, `root_text`, `lazy::run_with`); the Tools window has
the one it needs on the UI thread (`Tools::use_limit`). The interface is watched
by `App::note_interface` each frame, and written once it has stopped changing for
0.6 s, and from `eframe::App::on_exit`.

## 10. Known issues

The jq programs that do not run here as they run in jq (the engine is jaq, plus
`jq_ext`) are data, `jsonquery_query::known_issues` (`known_issues.rs`, and the list
itself in `known_issues/list.rs`): each issue is a line, the programs that show it with
what jq 1.8.1 gives and what this engine gives, and sometimes a workaround. Three
things are made from it:

- **The About window's *Known issues* section** (`app/issues_view.rs`): a header, under
  the links, that opens the list (the issues that are not `minor`, a line each, with
  the first example in a tooltip). The window is made wider and then as tall as the
  list needs, as the Settings window is for its *Advanced* limits, and goes back when
  the list is closed.
- **`docs/known-issues.md`**: all of them, with every example, written by
  `known_issues::markdown()` (`JSONQUERY_WRITE_DOCS=1 cargo test -p jsonquery_gui
  the_known_issues_page` writes it again; the same test fails when it is out of date).
- **The tests**, which run every example and workaround against the engine, and look at
  the first thousand results of one that never finishes. When an upgrade or a fix makes
  a program behave like jq, its test fails and names the entry to take out of
  `list.rs`.

## 11. Workspace

The user's saved queries, schemas, patches, documents and web addresses are a folder
(`workspace.rs`, and `workspace/` beside it in the app crate; the guide for users is
[Workspace](workspace.md)). It is a real folder, so that other programs can use what is
in it, and so everything that is done to it is written to be safe to do to a folder
that is not only the app's:

- **`workspace/names.rs`** — which file is which kind of item (`.jq`, `.schema.json`,
  `.patch.json`, `.url`… `ItemKind`), the sample that goes with it
  (`<name>.sample.json`), and the names a file system and a person may use
  (`check_name`, `check_item_name`: the characters, the length, the names Windows keeps,
  and a round trip so that a name is read back as the same item). A web address
  (`ItemKind::Url`) never has a sample and never shares one (`ItemKind::takes_sample`):
  `scan` gives it none, and the sample rules of `store.rs` and `transfer.rs` (who keeps
  a sample, which names a copy or an import may take) do not count it.
- **`workspace/address.rs`** — the web address that a `.url` item holds: `web_address`,
  the one rule for what may be an address (used where a file is read, in the window
  that saves, and before anything is loaded), `address_in` (the first `URL` line of a
  Windows Internet Shortcut, or the one line that the file is), `shortcut_text` (what is
  written) and `default_name` (what a new item is called to begin with).
- **`workspace/files.rs`** — `write_atomic` (a file is written beside and renamed into
  place), relative paths (`/` between the parts, on every system), and the guard against
  going through a link (`passes_through_a_link`, `is_inside`).
- **`workspace/ids.rs`** — `sha256_hex` / `sha256_file` and `RandomIds` (UUID v4 from the
  system's random bytes), both from `ring`; `IdSource` is a trait so that a test can have
  ids it can predict.
- **`workspace/index.rs`** — `.jsonquery/index.json`: the id, SHA-256, size and time of
  every item and the SHA-256 of its sample, as it is documented in
  [Workspace](workspace.md#the-id-and-the-sha-256-of-each-item); keys that a later version
  wrote are kept, an unusable file is set aside, a newer one is never written over.
- **`workspace/store.rs`** — `Workspace`: the folder read into a tree of `Folder`s and
  `Item`s (`scan`: what is new gets an id, what was changed keeps it with a new SHA-256,
  what was moved is found again by its SHA-256, what is gone is dropped), and `save`,
  `save_staged`, `create_folder`, `rename`, `move_to`, `duplicate`, `delete`. Every one
  reads the folder again first, and writes the index after.
- **`workspace/transfer.rs`** — `export` (to a folder that is new or empty, with the index
  so that ids and SHA-256s travel) and `import` (into a new folder or among what is there;
  the same SHA-256 is not copied again; an id in use is replaced).

The pane (`workspace_pane.rs`, with `tree.rs`, `drag.rs`, `forms.rs` and `icons.rs` under it) is
drawn at the left of the main window by `App::workspace_panel`, before the rows it
stands beside (the status bar is drawn before it, so that it stops above it); it asks
the app for what it cannot do itself through `Action`s, as the Tools do with their
`Request`s: put a query in the box, put a file in the Tools' schema or patch box, open
a file as the Source, load a web address as the Source (`Action::OpenUrl`, which the
app does as `App::open_url` does for an address typed in the Source field and loaded),
write the Source to a file (the sample of an item: the worker
writes it, to a file in the app's folder in the workspace, which is moved into place
when `Event::Saved` says it is written), keep another folder, show more or less of
the pane. Where it is not shown (`settings::WorkspaceView::Hidden`, the way the app
starts) nothing of it is drawn or read, and the window is as it was.

A row of the list senses clicks and drags (`tree.rs`): the one that is dragged is the
payload of egui's drag and drop (`Carried`, in `drag.rs`), and what is under the pointer
is worked out once a frame from the rectangles of the rows drawn (`Spot`), not from
each row's own response, because egui says that a widget under another still contains
the pointer. `drag.rs` has what can be said without a window — what a drop would do
(`verdict`), the label, when a shut folder opens (`dwell`) and how fast the list scrolls
(`edge_scroll`) — and the few pictures; the move itself is `Workspace::move_to`, which
the menu's *Move to* uses too. The pictures (`icons.rs`) are drawn with shapes on whole
pixels: the folders of the list as one solid body with a tab, the button of the status bar
(and the strip of a folded pane) as a window with its left column filled, the button
that makes a folder as a folder with a plus left out of its body (rectangles around the
gap, so that what shows in it is the button). A message under the list asks for the
window to be drawn again every frame it is shown, because egui forgets, at the next frame,
what an earlier frame asked for, and a window nobody touches is not drawn at all.

The button that shows the pane is the first thing in the status bar, before the status
text (`App::status_bar`). A query is also put in the box from outside the pane: a query
that is carried out of the list and let go over the query pane (`Env::query_area`, the
rectangle the app drew it in last frame) is opened as a double click opens it (`tree.rs`,
`carry`), and a file dropped on a window (`App::handle_drag_and_drop`) whose name ends in
a query's (`workspace::query_language_of`) goes in the box through the same
`App::put_query`. The windows are not told where in them a file is let go (winit 0.30
has no position for a drop), so which pane a dropped file is for is the file's to say.

## Scaling beyond in-memory

A parsed document takes about twelve times the size of its file for records of a
few short fields, and what the file is made of moves that a long way: about
the size of the file for one of long strings, about 25 times it for a flat array
of small numbers (each of which is a string of its own, to keep it exact), 17
times it for records of six fields with a nested one (measured: 1.7 GB for
100 MB). So a file of a gigabyte would want more than the machine has. A file of
256 MiB or more is therefore not parsed. It is kept on disk, memory-mapped, and
what is held in memory is an index of it: `jsonquery_core::lazy`.

**The index.** `LazyTree::from_mmap` reads the bytes once, as strictly as
`serde_json` does — what it accepts `serde_json` accepts, and what it refuses it
refuses, in the same words and with the same line and column (the tests damage
thousands of random documents to check) — so that any part of a document that
opened can be parsed later. While it does, it notes, for every container with at
least 256 children or 256 KiB of bytes, where it ends, how many children it has,
and a checkpoint (child number, byte offset) every 64 children or 1 MiB. Nothing
else is kept: the document is the file. On the 300 MB file of 2.5 million records
used to measure this it takes 0.4 s in a release build (770 MB/s), and the index is
603 KiB; the anonymous memory of the app is the same with the file open as without
(37 MB, then 39), and the rest — the file's pages, which the system gives back when
it needs them — is what has been looked at.

**A node.** A `Node` is one value of the file: the range of bytes it is, and what
it is. Its children are found without parsing it: the `n`th child of a big
container by binary search over its checkpoints and then a walk past fewer than 64
siblings (3 µs in an array of 2.5 million), those of a small one by walking it. It
is a `ValueView`, the trait the tree widget, search and "Find in source" were
written against from the start, so they work on it as on a parsed value. A node can
be parsed on its own (when it is small enough), or written out as the text
`serde_json` would make of it (pretty-printed, whole or cut by nodes, bytes and the
length of a string) straight from the bytes, a piece at a time.

**The tree.** A list of a million rows cannot be drawn, scrolled by a scrollbar
whose positions are 32-bit floats, or even made: so for a document kept on disk a
container with more than 1000 children shows them in runs — `[0 … 999999]`, which
opens into runs of a thousand, which open into the children — the way the developer
tools of a browser show a long array. At most a thousand rows are made for a list,
however long it is, and only the children of the runs that are open are looked at.
"Find in source" and the search hit list open the runs that hold the node they go
to.

**Queries.** jq is the engine for these documents. The program is read for the
stages that only look for something in the file — a path of constant steps
(`.users[0].name`, `.[]`, `.[2:50]`), `length`, `keys`, `type`, `first`, `last`,
`map(…)`, `[…]`, `first(…)`, `limit(n; …)`, `select(…)`, `strings`, `numbers` and
the other filters of a kind, `..`, `E?`, `E1, E2` — and those are walked against
the file, a step at a time, with what each gives and does on a value in memory (the
tests compare the two on hundreds of programs and dozens of documents, errors
included, down to `.[0]` of an object being `null`). What they hand on, an element
at a time, is small enough to be a value, and jaq is given it together with the
rest of the program. So `.[] | select(.level == "error") | .message` reads the
file once, holds one element at a time, and stops early when a `first` or `limit`
has what it wants; `map(.id) | add` and `[.[] | select(…)] | length` gather what the
elements give and nothing more; `.. | strings | select(test("x"))` goes down every
container, whatever its size, to the pieces that jaq is given.

An expression that is made of such parts — `{count: length, first: .[0]}`,
`.[-1] - .[0]`, `length > 1 and .[0] != null` — is taken apart at its operators and
its braces: each part is walked, and what it makes stands in the text of the
expression where it was, which jaq then runs with nothing to read but those. So
what jaq does with a part that makes no output, or many, or an error, is what it
does with the part itself, and a part that is too big to be given to jaq is an
error only where jaq comes to it (`false and add` is `false`).

What needs a key of every element of a list — `sort_by`, `group_by`, `unique_by`,
`min_by`, `max_by`, and `sort`, `unique`, `min`, `max`, `reverse` — is made of two
things that can be held: the key of each element, which is what jaq makes of the
element with the function it is given (by threads, for a long list, and compared as
jaq compares values: the tests compare the order of 160,000 pairs of random values),
and where the element is in the file. A sort keeps neither the elements nor the
file's pages: what it makes is the same list in another order, or a list of lists,
**still in the file**, which the stages after it read from where it is — `sort_by(.n)
| .[0:10]`, `group_by(.k) | map({k: .[0].k, n: length})`, `sort_by(.n) | reverse |
first` — one element at a time. Looking for the smallest or the biggest (`min_by`,
`max_by`) keeps only the best so far. Measured on a 300 MB file of 3 million
records, in a release build: `sort_by(.score) | .[0:3]` 1.4 s with 250 MB of
anonymous memory at the most, `group_by(.k) | length` 2.9 s, per-group totals
sorted and cut 3.6 s, `min_by(.qty)` 0.9 s with no more than the 50 MB it had.

The elements of a long list, or a few heavy ones (the groups of a `group_by`), are
shared out to threads (all the cores but two, at most twelve, in runs of up to 256
elements or 256 KiB, after the first 20,000 elements or 1 MiB, which the thread
that was asked does itself so that a first match near the beginning answers at
once), and what they make is handed on in the order of the list: on the 300 MB file
of 2.5 million records, `map(.id) | length` took 1.3 s and
`.[] | select(.id == 2000000) | .name` 1.1 s, where one thread takes about eight.
What cannot be done is a program that needs the whole of a list in memory as a
value — `add` on the document itself, `to_entries`, `flatten`, `del(…)`, an
update with `|=`: it is an error that says what it needed and what to do instead
(`.[] | …`, `map(…)`, `[…]`, a slice), which `?` and `try` do not take for an error
of the program. The limits: a list or an object of up to 4 MiB of the file is given
to jaq whole, one result may be up to 16 MiB, what `[…]` gathers or the keys of a
sort take up to 1 GiB each, estimated as they are held (a `serde_json` value is 72
bytes, and a number in a list about 100); a result past that is an error, not a
value the results panel would have to hold. JSON Pointer works (it walks to its
place).

**JSONPath and JMESPath** are walked the same way. The query is read by the engine
that runs it for any document (`jsonpath-rust`, `jmespath`), and what it reads is
walked here for as long as the node is too big to be a value: a name, an index, a
wildcard, a slice or a descendant segment (JSONPath), a field, an index, a slice or
a projection (JMESPath) pick the children, a filter is run on each child by itself,
and what is small enough goes to the engine with what is left of the query, which
is run on it as on a value of its own — so what each piece does is what the engine
does with it, and the tests compare the two on 27 documents and 130 queries. A
long list's elements are shared out to threads as a jq program's are:
`$[?(@.qty > 48)].id` over 3 million records takes 0.4 s, `[?qty > `48`].id |
length(@)` 0.6 s. What cannot be done is a function that needs a whole big list
for its argument (`sort_by`, `max`, `sum`, …), and a JSONPath filter that reads the
document from its root (`$`), since the query is run on a part of it as if that were
the root: both are errors that say so. Each result goes through the results panel
as for any document, so what is shown is bounded as it always was (50,000
results, the text view's 20,000 nodes).

**The rest.** The text view shows the first 20,000 nodes, at most 4 MiB, with a
long string cut at 4 KiB. "Save…" writes the document or a row straight from the
file, a piece at a time, which takes no memory; "Copy" takes up to 64 MiB. Search
and "Find in source" walk the whole file (about 200 MB/s). Completion in the query
box is made from a sample of the document — the first 50 children of every
container, to 10 levels — so it offers what the first of a list has. The Tools
window works on a document in memory and refuses these (it already stopped at
128 MB) — but for Format, which lays out a document that is kept as its file as it
writes it, a piece at a time, in any of its styles but with the keys sorted. An object that has a key twice is read as a parsed one is, with the key once,
in the place of its first and holding the value of its last: a pass of its own
finds those objects while the scan indexes the rest (by another thread, so that it
takes no longer), and only they are read any other way than straight from the
file.

**What it costs.** Everything is asked of the file, so each answer takes some work,
where a parsed document answers from memory; and the file has to stay as it is for
as long as the document is open (SIGBUS, and Windows keeping it). Both are the
price of not holding twelve times the file; they are why a file below 256 MiB is
parsed, as it always was. A merged document is several files, so its root would be
a list of per-file segments, each with its own mapping and index — a handle into a
lazy document should say which file it is in, not assume one byte range; merges are
for files of up to 128 MB in all and are parsed.
