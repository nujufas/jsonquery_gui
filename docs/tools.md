# Tools

The dim 🛠 icon in the toolbar, next to the 📖 tutorial icon, opens the **Tools
window** — a second window with a list of small utilities on the left and the
selected one on the right:

| Tool | What it does |
|---|---|
| [Merge JSON](#merge-json) | Combines several files into one document, with a jq filter. |
| [Format JSON](#format-json) | Pretty-prints (2 or 4 spaces, tab), minifies, sorts the keys. |
| [Diff JSON](#diff-json) | Says what was added, removed and changed between two documents, and gives the JSON Patch for it. |
| [Patch JSON](#patch-json) | Applies a JSON Patch (RFC 6902) or a JSON Merge Patch (RFC 7386) to a document. |
| [Validate schema](#validate-schema) | Checks a document against a JSON Schema and lists what does not fit. |

Every page has the same shape: a heading over two halves, with what goes in on
the left and what came out on the right. Each half has a title row, a box that
takes all the height there is, and its buttons pinned at the bottom. The window
keeps its text short: what a choice does, and the limits, are in tooltips.

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
   filter in the box under it.
3. Press **Merge** (or `Ctrl+Enter`). The box on the right shows a preview of
   what came out, and its title says what it is ("array · 6 items").
4. **Open in main window** makes it the loaded document — to explore, query
   and search it like any other — and **Save…** writes it to a file. Both are
   under the preview, and usable once there is a result.

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
texts, and **Copy** and **Save…** take all of it (Copy stops at 16 MB; Save does
not). The title of the result says how big it is against what went in. Changing
an option or the input drops the old result. A save is named after the file the
text came from (`orders.formatted.json`, or `orders.min.json` when minified), so
it does not land on top of it unless you choose that.

Numbers are written as they were read — `1.0`, `0.50` and a 30-digit integer
come out as they went in (an exponent is normalized: `1E5` comes out `1e+5`) —
and key order is kept unless **Sort keys** is on.

## Diff JSON

Compare **Before** with **After**.

The result is a list of **Changes**: what was **Added**, **Removed** or
**Changed**, each with the JSON Pointer to it, and the value (or both values) at
the right. Click a row to copy its path; hover for the whole of a long value. The
**Patch** tab shows the same difference as the RFC 6902 JSON Patch that turns
Before into After, one operation to a line (with a space after each colon and
comma, so a long one wraps), and **Save patch…** and **Copy patch** take it. **Swap** exchanges the two boxes.

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
is in the schema. The title says **Valid**, or how many problems there are —
a thousand at most are listed. **Copy report** copies the list.

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
- A merged or patched document is `Document::from_value` with
  `DocumentSource::Merged` or `DocumentSource::Derived`: it has no file to
  reload from, so the Source field is left empty and the toolbar names it
  ("(merged from 3 files)", "(patched)").
- These are plain in-memory values, so they do not use the memory-mapped path a
  single opened file does. When a lazy, memory-mapped backend arrives (see the
  Scaling section in [Architecture](architecture.md)), the root of a merged
  document should be a list of per-file segments rather than one byte range,
  but the tools are for small files and need nothing from it.
