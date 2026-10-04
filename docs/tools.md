# Tools

The dim 🛠 icon in the toolbar, next to the 📖 tutorial icon, opens the **Tools
window** — a second window with a tab for each of a few small utilities, and the
page of the selected one under them:

| Tool | What it does |
|---|---|
| [Merge JSON](#merge-json) | Combines several files into one document, with a jq filter. |
| [Format JSON](#format-json) | Pretty-prints (2 or 4 spaces, tab), minifies, sorts the keys. |
| [Diff JSON](#diff-json) | Shows two documents side by side with what differs marked, lists the changes, and gives the JSON Patch for it. |
| [Patch JSON](#patch-json) | Applies a JSON Patch (RFC 6902) or a JSON Merge Patch (RFC 7386) to a document. |
| [Validate schema](#validate-schema) | Checks a document against a JSON Schema and lists what does not fit. |

Every page has the same shape, and it is the main window's: the tabs on top (as
its toolbar is), a command row under them with the page's main button and its
options (as the Query row has Run), then two panes side by side — what goes in
on the left, what came out on the right — each with a small title and its buttons
in a header (as Source and Results have), and a status bar at the bottom
that says what came out ("100 B (was 48 B)", "2 added · 1 removed", "Valid") and
what was last done ("Saved to …"). The line between the panes can be dragged.
(Diff is the one page laid out otherwise: see [Diff JSON](#diff-json).)
Buttons, text and spacing are the main window's own, so the two look like one
program, and the window keeps its text short: what a choice does, and the
limits, are in tooltips.

## Putting JSON in

Format, Diff, Patch and Validate take their documents in the same kind of box.
A box has four ways in:

- **type or paste** the JSON into it;
- **drop a file** on the window — it goes to the first empty box of the page,
  or replaces the last one when all are full;
- **Open file…** reads a file into the box;
- **Open document** uses the document that is open in the main window (one
  over the limit below is refused: the box says so).

A file of up to 1 MB is read into the box, where it can be edited. A bigger one
is not shown (a text box cannot edit that much); the box says so and the file
is read when the tool runs. The open document is shared, not copied. **Clear**
empties a box.

Text that holds several top-level values (NDJSON) counts as one array, as it
does when opened on its own, and numbers keep their exact digits throughout.

All of these work **in memory**, so they are meant for documents that are not
very large: what a tool is given may add up to **128 MB**. Past that it says so
and refuses. **Ctrl+Enter** runs the page's main button, and **Cancel** stops a
job that is still running. If the text is not JSON, the error says which box and
where: `Input: parsing JSON: expected value at line 3 column 1`.

## Merge JSON

Combine several JSON files into one document, using jq.

1. Put the files in the list: drop them on the Tools window — or on the main
   window, which offers them to Merge when you drop more than one — or press
   **Add files…**. The order of the list is the order of the merge: ⏶ ⏷ move a
   file and × removes it (they show up when the pointer is on the row),
   **Sort A–Z** orders by name with numbers in order (`part2` before `part10`),
   and **Clear** empties the list (those two appear once there are files).
2. Choose **Merge as** — hover a choice to see what it does — or write your own
   filter in the box under the command row.
3. Press **Merge** (or `Ctrl+Enter`). The pane on the right shows a preview of
   what came out, and the status bar says what it is ("array · 6 items").
4. **Open in main window** makes it the loaded document — to explore, query
   and search it like any other — and **Save…** writes it to a file. Both are
   in the result's header, and usable once there is a result.

### How the filter sees the files

It is `jq -s` over the files: they are read in the order listed and put in one
array, which is the filter's input `.`; `$files` is the array of their names.
The ready-made filters are ordinary jq, shown in the box so they can be edited:

| Merge as | Filter | What it does |
|---|---|---|
| Append arrays | `add` | Joins arrays end to end. Objects work too: keys are combined and a later file wins a clash. |
| Append, sorted and de-duplicated | `add \| unique` | Like the above, then drops repeats (jq's `unique` also sorts). |
| Deep-merge objects | `reduce .[] as $file ({}; . * $file)` | Merges objects key by key, all the way down. |
| Bundle by file name | `[range(0; length) as $i \| {file: $files[$i], data: .[$i]}]` | Keeps each file whole, labelled with its name. |

Any other filter is a custom merge. Some that come up:

```jq
add | group_by(.id) | map(add)                    # records with the same id are combined, later files win
add | unique_by(.id)                              # one record per id, the first seen
[range(0; length) as $i | .[$i][] | . + {from: $files[$i]}]    # tag every record with its file
map(select(type == "array")) | add               # skip the files that are not arrays
```

A filter that gives several outputs (`.[]`) gives them as one array. A filter
with a runaway output (more than a million) is stopped.

## Format JSON

Print a document the way you want it.

- **Indent**: 2 spaces, 4 spaces, a tab, or **Minified** (no white space).
- **Sort keys** puts the keys of every object in order, at every depth. Arrays
  keep their order.
- **ASCII only** writes every character outside ASCII as a `\uXXXX` escape (a
  surrogate pair above U+FFFF), so the text survives anything that only carries
  ASCII.

**Format** (or `Ctrl+Enter`) prints it; the preview shows the start of long
texts, and **Copy** and **Save…** (in the result's header) take all of it (Copy
stops at 16 MB; Save does not). The status bar says how big the result is against
what went in. Changing
an option or the input drops the old result. A save is named after the file the
text came from (`orders.formatted.json`, or `orders.min.json` when minified), so
it does not land on top of it unless you choose that.

Numbers are written as they were read — `1.0`, `0.50` and a 30-digit integer
come out as they went in (an exponent is normalized: `1E5` comes out `1e+5`) —
and key order is kept unless **Sort keys** is on.

## Diff JSON

Compare a **Left** document with a **Right** one, the way a file-comparison
tool such as Beyond Compare does. (They are not called Before and After: two
files that are compared need not be versions of one.) Differences are read from
Left to Right: what only Left has is **Removed**, what only Right has is
**Added**, and the JSON Patch turns Left into Right. The page has four views, as
tabs in the command row:

- **Documents** — the two documents next to each other, to type, paste, drop or
  open (the same box as the other pages). **Compare** (Ctrl+Enter) runs the
  comparison and opens the next view by itself; **Swap** exchanges the two and
  goes back here.
- **Side by side** — the two documents, left and right, line by line, and
  read-only. What both have is on one line on both sides; what only one has
  is **red** on the left (removed) or **green** on the right (added), with a blank
  on the other side; what changed is **amber** on both, with the characters that
  differ picked out. The two sides scroll together (one scroll bar, and a bar
  under them, or Shift+wheel, for long lines), and the strip at the right edge is
  an overview of the whole document with a tick for every difference — click or
  drag it to go there. **⏶** and **⏷** (Alt+Up and Alt+Down) go to the previous and
  next difference, and the status bar says which ("Difference 2 of 5").
  **Differences only** folds what is the same into a line ("… 23 lines are the
  same"), keeping three lines around each difference. Click a difference to copy
  its path. **⏴** and **⏵** move the picked difference into the other document
  (see [Moving a difference](#moving-a-difference)).
- **Changes** — a list of what was **Added**, **Removed** or **Changed**, each
  with the JSON Pointer to it, and the value (or both values) at the right.
  Click a row to copy its path; hover for the whole of a long value.
- **Patch** — the same difference as the RFC 6902 JSON Patch that turns Left
  into Right, one operation to a line (with a space after each colon and
  comma, so a long one wraps).

**Copy patch** and **Save patch…** are at the right of the command row, in every
view, and take the whole patch.

### Moving a difference

A difference can be moved from one document into the other, so that the two come
to agree there — what a file-comparison tool calls copying to the left or to the
right. Pick one in the Side by side view (click its lines, or step to it with
**⏶** and **⏷**) and press **⏴** (Alt+Left) to move it to the left, so that Left
takes what Right has there, or **⏵** (Alt+Right) to move it to the right, so that
Right takes what Left has there. The right-click menu of a difference's lines
has the same two choices and needs nothing picked first.

What is moved is a difference as the view shows it: lines that differ and touch
are one. A value that only one side has is put into the other document (at its
place in an array, after the member before it in an object) or, going the other
way, taken out of it; a value that is not the same on both sides is replaced by
the other side's. Everything else stays as it was.

The document that changed is written back into its box, pretty-printed with two
spaces to a level; its members keep their order and its numbers their spelling,
and the box is no longer called by the file it was read from — the file itself is
never touched. The other document is not changed. The comparison then runs again
and the view goes on to the next difference, so pressing **⏵** over and over
walks through the whole comparison, and a document that was NDJSON comes back as
one array.

A text box can only edit about a megabyte, so a document that comes out bigger
than that is not shown: its box says "Too big to show" and holds it as it is, to
be used when you compare again, until you clear it or put something else in. A
move is limited only by what the tools take in all (128 MB, with the other
document), and says so if it is over.

The lines are the pretty-printed documents (two spaces to a level, numbers as
they were written), and what is marked is exactly what is in the patch. Both
sides show an object's members in the order of the *left* document, because
the order is not a difference: a document whose keys are in another order lines
up with the other instead of showing every line as moved. Lines longer than 400
characters are cut with `…` (the patch has the whole value). A pair of documents
that is over 200,000 lines is too long to lay out: Side by side says so, and
Changes and Patch have the differences all the same.

What counts as a difference is what JSON itself says:

- the members of an object have no order, so a document with its keys sorted is
  the same document;
- numbers are compared by value, with their exact digits: `1`, `1.0` and `10e-1`
  are one number, while two 30-digit integers that differ in the last digit are
  two;
- arrays are ordered, but they are **aligned by content** before they are
  compared, so a value inserted at the start of a long array is one addition
  rather than a change to every element after it.

The list shows at most 5,000 changes (the counts and the patch are complete).
Very large arrays are paired up by position rather than aligned, which gives a
correct but longer patch.

## Patch JSON

Apply a patch to a document. The second box holds the patch, and **Kind** says
which sort it is:

- **Operations (RFC 6902)** — a list like
  `[{"op": "replace", "path": "/a", "value": 1}]`, using `add`, `remove`,
  `replace`, `move`, `copy` and `test`. They run in order and all or nothing: if
  one fails (a path that isn't there, a `test` that doesn't hold) nothing
  changes and the error names it — `operation 3 (replace /a/b): there is no
  member "b"`.
- **Merge patch (RFC 7386)** — a document laid over the other: its members
  replace or add, a `null` deletes one, and anything that isn't an object
  replaces what it is laid over.

The result is previewed. **Open in main window** makes it the loaded document
("(patched)" in the toolbar), **Save…** writes it (named after the document,
`orders.patched.json`), and **Copy** copies it. Objects keep their order: a
replaced member stays where it was, a new one goes last.

## Validate schema

Check the **Document** against the **Schema** (a JSON Schema: drafts 4, 6, 7,
2019-09 and 2020-12, read from `$schema`, or the newest when there is none).

The result is the list of problems. Each row has the JSON Pointer of the value
that is wrong (`(document)` when it is the whole document) and what is wrong
with it; hover for the keyword that said so (`required`, `minimum`…) and where it
is in the schema. The status bar says **Valid**, or how many problems there are
— a thousand at most are listed. **Copy report** copies the list.

- **Check formats** holds strings to their `format` (`email`, `date-time`,
  `uuid`…). The specification leaves that to the validator in recent drafts;
  most people want it, so it is on.
- If the document is the one **open in the main window**, pick a problem and
  **Show in main window** (or double-click it) puts its pointer in the query
  box, under the *Pointer* engine, and runs it — so the offending value is what
  you see. (A pointer is only meaningful in the document it came from, so this
  is off for text and files.)
- A `$ref` has to point inside the schema (`#/$defs/name`, an `$id` or an
  `$anchor` in it). **Nothing is fetched**: a schema is something you paste in,
  and it must not make the app read a URL or a file. A reference to somewhere
  else is reported as an error.
- A schema that is not a valid schema, or refers to something that isn't there,
  is reported with the reason: `The schema can't be used: …`.

The checking is done by the [`jsonschema`](https://crates.io/crates/jsonschema)
crate.

## Limits

- Every tool works **in memory**: what it is given may add up to **128 MB**.
  (Use `jq`, or a tool made for it, for bigger things; DuckDB reads many JSON
  files with SQL.)
- A filter that produces more than a million outputs is stopped.
- **Cancel** stops a job that is still running; a jq filter that spins without
  producing anything can't be interrupted until it yields, as in the query box.
- Drag-and-drop doesn't work on native Wayland (see the README); **Add files…**
  and **Open file…** do.

## Implementation notes

- The logic is in `crates/query`, one small module per tool, with its own unit
  tests: `merge` (jq `-s` through the embedded jaq), `reformat` (the printer),
  `diff` (the comparison and its RFC 6902 operations), `patch` (RFC 6902 and
  RFC 7386, and JSON Pointer) and `schema` (a thin layer over `jsonschema`, built
  without its network and file features). They work on `serde_json::Value` with
  exact numbers.
- The window is `crates/app/src/tools.rs` and `crates/app/src/tools/`: one file
  per page, `operand.rs` for the box JSON goes in, `widgets.rs` for the shared
  look and `jobs.rs` for the work the worker thread does. The jobs run on the
  worker thread (`Command::Tool`, answered by `Event::ToolDone`), like every
  other file operation, each with a generation number and a cancel flag.
- The window is a *deferred* viewport, redrawn by eframe on its own and not as a
  part of the main window's frame (`App::satellite_frame`, see
  [Architecture](architecture.md)). It matters when the window is maximized over
  the main window: GNOME sends a completely covered window no redraw callbacks,
  and a window drawn inside that window's frame would stop responding with it.
  Its frame takes in the worker's answers itself, so a job finishes while the
  main window is covered.
- A merged or patched document is `Document::from_value` with
  `DocumentSource::Merged` or `DocumentSource::Derived`: it has no file to
  reload from, so the Source field is left empty and the toolbar names it
  ("(merged from 3 files)", "(patched)").
- These are plain in-memory values, so they do not use the memory-mapped path a
  single opened file does. When a lazy, memory-mapped backend arrives (see the
  Scaling section in [Architecture](architecture.md)), the root of a merged
  document should be a list of per-file segments rather than one byte range,
  but the tools are for small files and need nothing from it.
