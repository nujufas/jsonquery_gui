# jsonquery

A native desktop tool for browsing and querying large JSON files — drag in a
file (or paste JSON directly), write a [jq](https://jqlang.github.io/jq/)
compatible query, and see the result as a scrollable tree. Built in Rust with
[egui](https://github.com/emilk/egui)/[eframe](https://github.com/emilk/egui).

![jsonquery screenshot](docs/images/screenshot.png)

## Built by AI

This project — the code, the architecture docs, and the build tooling — was
built by [Claude](https://claude.com) (Anthropic's AI), working from a series
of prompts by the repo owner. It's as much an experiment in AI-driven
software development as it is a JSON tool. The [architecture proposal and
decision docs](docs/index.html) capture the reasoning behind the design
choices along the way — open them locally in a browser to read them rendered
(GitHub shows `.html` files as source, not as pages).

## Status

**Phase 1 (MVP)** is implemented: full in-memory parsing, jq-compatible
queries via an embedded [jaq](https://github.com/01mf02/jaq), and a
virtualized-tree GUI for both the source document and query results. It's
solid for small-to-medium files.

Multi-gigabyte files need Phase 2 (a memory-mapped, lazily-resolved index),
which isn't built yet. See [`docs/decisions.html`](docs/decisions.html) for
the full roadmap and the reasoning behind what's built vs. deferred.

## Features

- **Open large-ish files fast** — memory-mapped, no upfront full-file copy.
- **jq-compatible queries** — a real jq implementation ([jaq](https://github.com/01mf02/jaq)) embedded directly, not a reinvented query language.
- **Streamed results** — results are pushed to the UI as jaq produces them, so `first(...)`/`limit(...)` genuinely stop early instead of running to completion in the background.
- **Exact number round-tripping** — big integers (snowflake IDs, Postgres bigints) survive a query byte-for-byte instead of quietly rounding through an `f64`.
- **NDJSON support** — a file with one JSON value per line is treated as a single queryable document, no separate "format" to pick.
- **Cancellable queries** — start a new query and the previous one is aborted, not queued behind it.
- **Drag-and-drop, file picker, or paste** — three ways to get JSON in.

## Getting started

### Download a build

Prebuilt Linux and Windows binaries can be produced with the scripts in
[`build/`](build/) — see [Building](#building) below. (No binary releases are
published yet; build from source in the meantime.)

### Build from source

Requires a [Rust toolchain](https://rustup.rs/) (stable).

```sh
git clone <this repository's URL>
cd jsonquery
cargo run --release -p jsonquery
```

## Usage

1. Open a file (drag-and-drop, the **Open File…** button, or paste raw JSON
   via **Paste JSON…**).
2. Write a query in the bar at the top — plain jq syntax, e.g.:
   ```
   .users[] | select(.active) | {name, roles}
   ```
3. Press **Run** (or `Ctrl+Enter`). Results stream into the right-hand panel
   as a tree, same as the source document on the left.

## Building

Native release build for the current platform:

```sh
cargo build --release -p jsonquery
# binary at target/release/jsonquery
```

Cross-platform packaged builds (tar.gz for Linux, zip for Windows) live in
[`build/`](build/):

```sh
build/linux.sh     # native release build, packaged
build/windows.sh   # cross-compiled via `cross`/Docker, packaged
build/all.sh        # both, plus a listing of dist/
```

`build/windows.sh` needs a working Docker daemon — it cross-compiles inside a
container that already has the mingw-w64 toolchain, so nothing is installed
on the host.

## Development

```sh
cargo test --workspace     # unit tests (core parsing/tree logic, query engine)
cargo clippy --workspace --all-targets
```

The workspace is split into three crates so the non-GUI logic can be tested
and benchmarked without pulling in a GUI toolkit:

- **`crates/core`** — file ingest (mmap + parse) and the virtualized-tree data layer.
- **`crates/query`** — the embedded jaq query engine and its `serde_json::Value ⇄ jaq_json::Val` conversion.
- **`crates/app`** — the eframe/egui application itself.

## Architecture

The design — pipeline, indexing strategy, concurrency model, crate layout —
is written up in [`docs/`](docs/index.html):

- [`docs/index.html`](docs/index.html) — problem statement, goals, high-level shape.
- [`docs/architecture.html`](docs/architecture.html) — the full system design.
- [`docs/decisions.html`](docs/decisions.html) — the decisions that shaped the build, open risks, and the roadmap.

(Open these locally in a browser — GitHub renders `.html` files as source, not as pages.)

## License

[MIT](LICENSE)
