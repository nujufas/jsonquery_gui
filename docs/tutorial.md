# Tutorial

jsonquery ships with a built-in tutorial for all four query dialects — jq,
JSON Pointer, JSONPath and JMESPath. Click the 📖 icon in the toolbar (next
to the 💡 autocomplete toggle) or press `F1`; it opens in its own window so it
can sit beside the main one.

## What you see

- **Four tabs** across the top — **jq** (the default), **JSON Pointer**,
  **JSONPath**, **JMESPath**. Each tab remembers the lesson you were on.
- **A topic tree** on the left: topics (expand/collapse) containing lessons.
  The filter box narrows it by lesson title, summary or example query.
- **The lesson** on the right: a one- or two-sentence summary, one to three
  short examples, and a few *Good to know* tips, with **Previous / Next**
  buttons that walk the whole tab in order.

Each example shows:

| Piece | What it is |
|---|---|
| **Query** | The query text, with each explained fragment on a tinted background. |
| **Result** | The *live* result, computed by the real engine — never a pasted copy. |
| **Explanation** | One row per highlighted fragment, colour-matched to the query. Hover a row to brighten its fragment. |
| **Sample data** | The JSON the query runs on (collapsible; shared by the lesson's examples when they use the same document). |

Related concepts share an example rather than one example per keyword — for
instance `.langs[0], .langs[-1], .langs[1:3]` covers indexing, negative
indexes and slicing in one query, with a row explaining each.

Examples that fail on purpose (a JSON Pointer to a value that isn't there)
say so in the result heading and show the engine's real error message.

## Practising in the main window

Every example has four buttons:

| Button | Effect in the main window |
|---|---|
| **▶ Try it** | Replaces the source document with the example's data, puts the query in the query box, pins the engine, and runs it. |
| **Load query** | Puts the query in the box and pins the engine; keeps whatever data is open. |
| **Load data** | Opens the example's data as the source document; leaves the query alone. |
| **Copy query** | Copies the query text to the clipboard. |

The engine is *pinned* (not left on auto-detect) whenever a query is loaded,
because a plain JMESPath path such as `office.city` is otherwise
auto-detected as jq. Loading brings the main window to the front and shows a
confirmation in the tutorial's status bar. The last lesson of each dialect is
a **cheat sheet** whose rows each have their own ▶.

## Where the content lives

| | |
|---|---|
| `crates/query/src/tutorial/mod.rs` | The content model (`Topic` → `Lesson` → `Example`), the shared sample documents, and the pure helpers: running an example, formatting a result, splitting a query into highlighted segments, and the tiny inline-markup parser. No egui. |
| `crates/query/src/tutorial/{jq,pointer,jsonpath,jmespath}.rs` | One `static TOPICS` per dialect — the lessons themselves. |
| `crates/app/src/tutorial.rs` | The window: tabs, tree, lesson view, and the hand-off to the main window. |
| `crates/app/src/query_highlight.rs` | The chip palette (`part_color`), shared with the main window's query box, which tints the query you type the same way — its steps come from `jsonquery_query::highlight` instead of a lesson's hand-written fragments. |
| `App::apply_tutorial_request` in `app.rs` | Applies a load request. *▶ Try it* sets `run_after_load`, because loading a document is asynchronous and finishing a load cancels any query already running — so the query is started from the `Loaded` event. |

The split mirrors `suggest.rs` / `query_suggest.rs`: pure, unit-tested logic
in the query crate; egui glue in the app crate.

## Adding or editing a lesson

Lessons are plain `const`-constructed data:

```rust
Lesson::new(
    "select & comparisons",
    "`select(condition)` lets a value through only when the condition is true.",
    &[Example::new(
        "Members older than 30",          // one plain-English line
        TEAM,                              // the sample document
        ".members[] | select(.age > 30) | .name",
        &[                                 // fragments, in query order
            (".members[]", "Stream the members."),
            ("select(.age > 30)", "Pass a member on only when the condition is true."),
            ("| .name", "Read the name of those that passed."),
        ],
    )],
)
.tips(&["Comparison operators: `==`, `!=`, `<`, `<=`, `>`, `>=`."])
```

- **Fragments** are matched against the query in order, so each must appear
  after the previous one. Text between fragments is simply not highlighted.
- **Prose** (`summary`, tips, notes) understands `` `code` `` and `*emphasis*`.
  JMESPath's own literals use backticks, so a code span that contains one is
  fenced with doubled backticks: `` `` `30` `` ``.
- Mark an example that is *supposed* to fail with `.failing()`.
- Use `Lesson::cheat_sheet(...)` for a syntax table, or `Lesson::new(title,
  summary, &[])` with `.tips(...)` for a text-only page.
- Avoid characters missing from egui's bundled fonts (see below); `→` and `←`
  are fine in prose — they are rendered specially.

`cargo test -p jsonquery-query tutorial` then checks that every example
**runs as declared** against the real engines, that every highlighted
fragment occurs in its query in order, that all sample data is valid JSON,
and that lesson titles are unique per dialect. To proof-read the prose
("→ `37.5`", "four results") against what the engines actually return:

```sh
cargo test -p jsonquery-query tutorial::tests::dump_every_example -- --ignored --nocapture
```

## Known gaps the lessons work around

These were found by running candidate examples through the real engines, and
are pinned by the `documented_gaps_are_still_real` test so the matching tips
get removed when an upgrade fixes them:

- **jq (jaq)** lacks `@csv`, `@tsv`, `IN`, `INDEX`, `$ENV`, `input`,
  `tostream`, `leaf_paths` and `toarray`, and can't create several missing
  path levels in one assignment (`{} | .a.b.c = 1`). The *jq here vs jq 1.7*
  lesson lists them.
- **JMESPath decimal literals are broken.** `` `9.5` `` evaluates to
  `{"$serde_json::private::Number":"9.5"}`, so ``items[?price > `9.5`]``
  silently returns `[]`. Whole numbers, strings, booleans and `null` work. The
  cause is the same `serde_json` `arbitrary_precision` trap described in
  [Query Engines](query-engines.md), this time on the `jmespath` crate's own
  literal parser rather than on the input document.

## Implementation notes

- **An immediate viewport.** The window is opened with
  `Context::show_viewport_immediate`, so it runs inside the main window's
  frame with plain `&mut` access to its state — no `Arc<Mutex<…>>`. egui
  falls back to an embedded floating window on a backend without native
  multi-window support.
- **Missing glyphs.** egui's bundled *proportional* font has no `→`, `←` or
  `✓` (they draw as empty boxes); its monospace font has the arrows. Prose
  therefore draws arrows through `push_text` in `tutorial.rs`, and the UI
  avoids `✓`. Check a new symbol on screen before relying on it.
- **No `egui::Grid` for wrapped text.** A grid feeds each frame's column
  widths back into the next frame's wrapping; with wrapping labels in its
  cells, rows settled at wildly wrong heights (visible in the cheat sheet).
  The explanation list and cheat sheet use fixed-width rows instead.
- **Testing under Xvfb + fluxbox.** A bare `F1` is swallowed by fluxbox's
  key grabs there (the app sees a phantom `Alt` press and only the release),
  so the shortcut looks broken; it works with no window manager, and on real
  desktops. Click the 📖 icon to exercise the window under the harness.
