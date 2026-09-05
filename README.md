# jsonquery gui

A native desktop tool for browsing and querying large JSON files — drag in a
file (or paste JSON directly) and query it with **jq**, **JSON Pointer**,
**JSONPath**, or **JMESPath**. Built in Rust with
[egui](https://github.com/emilk/egui)/[eframe](https://github.com/emilk/egui).

![jsonquery screenshot](docs/images/screenshot.png)

## Built by AI

This project — the code, the architecture docs, and the build tooling — was
built by [Claude](https://claude.com) (Anthropic's AI), working from a series
of prompts by the repo owner. The [architecture proposal and decision
docs](docs/index.html) capture the reasoning behind the design choices along
the way — open them locally in a browser to read them rendered (GitHub shows
`.html` files as source, not as pages).

## Features

- **Four query dialects, one box** — [jq](https://jqlang.github.io/jq/) (via
  embedded [jaq](https://github.com/01mf02/jaq)), JSON Pointer (RFC 6901),
  JSONPath (RFC 9535), and JMESPath. Pick one from the toolbar or let the app
  auto-detect it from what you type.
- **Streamed, cancellable queries** — results appear as jq produces them, so
  `first(...)`/`limit(...)` genuinely stop early; starting a new query aborts
  whatever was still running.
- **Exact number round-tripping** — big integers (snowflake IDs, Postgres
  bigints) survive a query byte-for-byte instead of quietly rounding through
  an `f64`.
- **NDJSON support** — one JSON value per line loads as a single queryable
  document, no separate format to pick.
- **Tree or raw text views** — toggle either the source document or the
  query results between a virtualized, expand/collapse tree and
  plain, selectable/copyable pretty-printed text.
- **Search** — case-insensitive substring or regex search across the source
  or the results tree, with click-to-reveal on a hit.
- **Right-click row menus** — copy a node's path, search from that scope, or
  save just that node to a file.
- **Save to file** — the whole source document, the whole result set, or a
  single node, independently.
- **Load however's convenient** — drag-and-drop, an **Open File…** picker,
  pasting JSON straight into the text area, or loading from a URL.
- **Light and dark themes**, switchable from the toolbar.
- **Keyboard shortcuts** — `Ctrl+Enter` run/apply, `Ctrl+F` search,
  `Ctrl+S` save (both scoped to whichever panel you last clicked).

## Status

**Phase 1** is implemented: full in-memory parsing, four query dialects, and
a virtualized-tree GUI for both the source document and query results. It's
solid for small-to-medium files.

Multi-gigabyte files need Phase 2 (a memory-mapped, lazily-resolved index),
which isn't built yet. See [`docs/decisions.html`](docs/decisions.html) for
the full roadmap.

## Known limitations

- **Drag-and-drop doesn't work on native Wayland** — a gap in
  [`winit`](https://github.com/rust-windowing/winit) (`eframe`'s windowing
  library), which only implements OS-level file drop on Windows, macOS, and
  X11 ([rust-windowing/winit#1881](https://github.com/rust-windowing/winit/issues/1881)).
  **Open File…** and pasting both work fine everywhere. Workaround: run
  under XWayland instead (if `DISPLAY` is set, it's available):
  ```sh
  WAYLAND_DISPLAY= cargo run --release -p jsonquery_gui
  ```

## Getting started

### Download a build

Prebuilt Linux and Windows binaries are attached to each
[GitHub Release](https://github.com/nujufas/jsonquery_gui/releases). You can
also build them yourself with the scripts in [`build/`](build/) — see
[Building](#building) below.

### Build from source

Requires a [Rust toolchain](https://rustup.rs/) (stable).

```sh
git clone https://github.com/nujufas/jsonquery_gui.git
cd jsonquery_gui
cargo run --release -p jsonquery_gui
```

## Usage

1. Get JSON in: drag a file onto the window, use **Open File…**, paste JSON
   into the text area, or load a URL.
2. Pick a query engine (or leave it on auto-detect) and write a query, e.g.
   `.users[] | select(.active) | {name, roles}` for jq, `/users/0` for
   Pointer, `$.users[*].name` for JSONPath, or `users[?active].name` for
   JMESPath.
3. Press **Run** (or `Ctrl+Enter`). Results stream into the right-hand
   panel — toggle it between **Tree** and **Text**, search it, or save it.

## Building

Native release build for the current platform:

```sh
cargo build --release -p jsonquery_gui
# binary at target/release/jsonquery_gui
```

Cross-platform packaged builds live in [`build/`](build/), output to `dist/`:

```sh
build/linux.sh      # native release build -> .tar.gz
build/appimage.sh   # native release build -> self-integrating .AppImage
build/windows.sh    # cross-compiled via `cross`/Docker -> .zip
build/all.sh        # all three, plus a listing of dist/
```

`build/windows.sh` needs a working Docker daemon (cross-compiles inside a
container with the mingw-w64 toolchain already installed). `build/appimage.sh`
downloads `appimagetool` on first use and needs FUSE to run it. On an actual
Windows machine, `build\windows.bat` builds natively instead — same output
layout, just needs a Rust toolchain and PowerShell.

The AppImage is desktop-pinnable out of the box: it self-registers a
`.desktop` entry and icon on first launch (no `appimaged`/AppImageLauncher
required), so right-click → Pin works from the taskbar/dock immediately.

## Development

```sh
cargo test --workspace     # unit tests (core parsing/tree logic, query engines)
cargo clippy --workspace --all-targets
```

The workspace is split into three crates so the non-GUI logic can be tested
without pulling in a GUI toolkit:

- **`crates/core`** — file ingest (mmap + parse) and the virtualized-tree data layer.
- **`crates/query`** — the four query engines, dispatched through a shared `QueryEngine` trait.
- **`crates/app`** — the eframe/egui application itself.

A Robot Framework GUI test suite (screen-driven, OCR-assisted) lives under
[`test/`](test/README.md) — see that README for how to run it.

### Test data

[`scripts/gen_test_data.py`](scripts/gen_test_data.py) generates a large
synthetic JSON or NDJSON file for exercising the app, with unicode and
19-digit integer ids that exceed `f64`'s exact-integer range:

```sh
scripts/gen_test_data.py                                    # ~200k records to test-data/large.json
scripts/gen_test_data.py --target-size 1GB -o test-data/big.json
scripts/gen_test_data.py --format ndjson -n 1000000 -o test-data/events.ndjson
```

Each run also prints a handful of sample queries worth trying against the
file it just generated.

## Architecture

The design — pipeline, indexing strategy, concurrency model, crate layout —
is written up in [`docs/`](docs/index.html):

- [`docs/index.html`](docs/index.html) — problem statement, goals, high-level shape.
- [`docs/architecture.html`](docs/architecture.html) — the full system design.
- [`docs/decisions.html`](docs/decisions.html) — the decisions that shaped the build, open risks, and the roadmap.
- [`docs/query-engines.html`](docs/query-engines.html) — the query-dialect landscape and why these four were picked.

(Open these locally in a browser — GitHub renders `.html` files as source, not as pages.)

## License

[MIT](LICENSE)
