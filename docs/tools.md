# Tools

The dim 🛠 icon in the toolbar, next to the 📖 tutorial icon, opens the **Tools
window** — a second window with a list of small utilities on the left and the
selected one on the right. Today that is **Merge JSON**; **Format JSON** and
**Diff & merge** are in the list, greyed, for later.

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

The window keeps its text short: what a choice does, what a button does and the
limit on the files' size are in tooltips (hover **Files**, a **Merge as** choice,
the result's title), and the only line of help on the page is the one under the
filter, saying what `.` and `$files` are.

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

A filter that gives several outputs (`.[]`) gives them as one array. A file
that holds several top-level values (NDJSON) counts as one array, as it does
when opened on its own. Numbers keep their exact digits, as they do everywhere
in the app.

### Limits

- A merge happens **in memory** — each file is parsed, copied into jq's values,
  and the result is built — so it is meant for files that are not very large.
  The files may add up to **128 MB** (hover **Files** to see it); past that,
  Merge says so and refuses.
  (Use `jq -s`, or a tool made for it, for those; DuckDB reads many JSON files
  with SQL.)
- A filter that produces more than a million outputs is stopped.
- **Cancel** stops a merge that is still running; a filter that spins without
  producing anything can't be interrupted until it yields, as in the query box.
- Drag-and-drop doesn't work on native Wayland (see the README); **Add files…**
  does.

## Implementation notes

- The merge itself is `jsonquery_query::merge` (`crates/query/src/merge.rs`):
  `merge(inputs, names, filter, cancel)` runs the filter with jaq — through
  `run_query_with_vars`, which gives it the `$files` global — and is unit-tested
  against the real engine. The window is `crates/app/src/tools.rs`; the files
  are read and merged on the worker thread (`Command::Merge`, answered by
  `Event::MergeDone`), like every other file operation.
- A merged document is `Document::from_value` with `DocumentSource::Merged`: it
  has no file to reload from, so the Source field is left empty and the toolbar
  names it ("(merged from 3 files)").
- It is a plain in-memory value, so it does not use the memory-mapped path a
  single opened file does. When a lazy, memory-mapped backend arrives (see the
  Scaling section in [Architecture](architecture.md)), the root of a merged
  document should be a list of per-file segments rather than one byte range,
  but the merge tool is for small files and needs nothing from it.
