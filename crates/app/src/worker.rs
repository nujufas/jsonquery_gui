//! Background worker thread (Architecture §5). The UI thread never touches
//! the filesystem or the query engine directly — it sends [`Command`]s here
//! and receives [`Event`]s back over a `crossbeam-channel` pair, then wakes
//! the egui context so a repaint picks up the new state.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossbeam_channel::{Receiver, Sender};
use jsonquery_core::engine::QueryEvent;
use jsonquery_core::{Document, DocumentSource, NodePath};

/// Cap on a URL download's response body, matching the "a few GB" v1 scale
/// ceiling (Architecture §3) — protects against a malicious or misbehaving
/// server exhausting disk/memory via an unbounded response.
const MAX_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Which tree a `Command::Search` runs over — the loaded source document (via
/// its `Arc<Document>`, so a huge document isn't cloned just to search it) or
/// the current query results (already bounded by the live-preview cap, so a
/// plain clone into an `Arc` is cheap enough).
pub enum SearchRoot {
    Source(Arc<Document>),
    Results(Arc<serde_json::Value>),
}

/// Which panel a `Command::RenderText` is rendering — echoed back on
/// `Event::TextRendered` so the UI thread knows which cache to fill, without
/// having to carry the (potentially large) rendered value back and forth to
/// tell them apart.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TextTargetKind {
    Source,
    Results,
}

/// What `Command::RenderText` renders: the loaded source document, or the
/// current query results (already bounded by `LIVE_PREVIEW_CAP`, so a plain
/// clone here is cheap — same reasoning as `Command::SaveResults`).
pub enum TextTarget {
    Source(Arc<Document>),
    Results(serde_json::Value),
}

impl TextTarget {
    fn kind(&self) -> TextTargetKind {
        match self {
            TextTarget::Source(_) => TextTargetKind::Source,
            TextTarget::Results(_) => TextTargetKind::Results,
        }
    }
}

pub enum Command {
    OpenFile(PathBuf),
    OpenText(String),
    OpenUrl(String),
    SaveFile {
        doc: Arc<Document>,
        /// The value to save: the whole document (`None`), or one node
        /// within it — resolved here rather than on the UI thread, so a row
        /// save doesn't need to clone a potentially huge document just to
        /// pick one branch out of it.
        node_path: Option<NodePath>,
        path: PathBuf,
    },
    SaveResults {
        results: serde_json::Value,
        path: PathBuf,
    },
    /// Look for a value structurally equal to `target` somewhere in `doc`,
    /// for the results panel's "Find in Source" row action.
    FindInSource {
        doc: Arc<Document>,
        target: serde_json::Value,
        gen: u64,
    },
    /// Search a whole tree for `text`, for the "Search…" row action.
    Search {
        root: SearchRoot,
        text: String,
        regex: bool,
        gen: u64,
    },
    /// Pretty-print `target` as text for the "Text" view toggle, bounded to
    /// `node_budget` nodes so a huge source document — or a huge single
    /// query result, e.g. `.` over a multi-GB doc — can't block the UI
    /// thread or blow past a reasonable render size (see `Event::Loading`'s
    /// sibling concern in Architecture §7's "bounded live preview").
    RenderText {
        target: TextTarget,
        node_budget: usize,
        gen: u64,
    },
    Query {
        doc: Arc<Document>,
        text: String,
        /// Which `QueryEngine` to run `text` against — already resolved from
        /// the UI's picker (or its auto-detect fallback) by the caller, so
        /// the worker thread never has to make that judgment call itself.
        engine: jsonquery_query::Kind,
        gen: u64,
        cancel: Arc<AtomicBool>,
    },
}

pub enum Event {
    Loading,
    Loaded(Arc<Document>),
    LoadError(String),
    Saved(PathBuf),
    SaveError(String),
    Found {
        gen: u64,
        path: Option<NodePath>,
    },
    SearchDone {
        gen: u64,
        matches: Vec<NodePath>,
    },
    SearchError {
        gen: u64,
        error: String,
    },
    TextRendered {
        target: TextTargetKind,
        gen: u64,
        text: String,
        truncated: bool,
    },
    QueryItem {
        gen: u64,
        value: serde_json::Value,
    },
    QueryItemError {
        gen: u64,
        error: String,
    },
    QueryDone {
        gen: u64,
        cancelled: bool,
        elapsed: Duration,
    },
    QueryError {
        gen: u64,
        error: String,
    },
}

/// Spawn the worker thread. `wake` is called after every event is sent so
/// the (otherwise idle, redraw-on-demand) egui context repaints promptly.
pub fn spawn(cmd_rx: Receiver<Command>, evt_tx: Sender<Event>, wake: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for cmd in cmd_rx {
            match cmd {
                Command::OpenFile(path) => {
                    send(&evt_tx, Event::Loading, &wake);
                    let result = jsonquery_core::load(&path).map(Arc::new);
                    send_load_result(&evt_tx, result, &wake);
                }
                Command::OpenText(text) => {
                    send(&evt_tx, Event::Loading, &wake);
                    let result = jsonquery_core::load_text(&text).map(Arc::new);
                    send_load_result(&evt_tx, result, &wake);
                }
                Command::OpenUrl(url) => {
                    send(&evt_tx, Event::Loading, &wake);
                    let result = download_to_temp_file(&url).map(Arc::new);
                    send_load_result(&evt_tx, result, &wake);
                }
                Command::SaveFile {
                    doc,
                    node_path,
                    path,
                } => {
                    let result = match &node_path {
                        Some(np) => match jsonquery_core::resolve(&doc.root, np) {
                            Some(v) => save_json(v, &path),
                            None => Err(anyhow::anyhow!(
                                "that value is no longer part of the document"
                            )),
                        },
                        None => save_json(&doc.root, &path),
                    };
                    match result {
                        Ok(()) => send(&evt_tx, Event::Saved(path), &wake),
                        Err(e) => send(&evt_tx, Event::SaveError(format!("{e:#}")), &wake),
                    }
                }
                Command::SaveResults { results, path } => {
                    let result = save_json(&results, &path);
                    match result {
                        Ok(()) => send(&evt_tx, Event::Saved(path), &wake),
                        Err(e) => send(&evt_tx, Event::SaveError(format!("{e:#}")), &wake),
                    }
                }
                Command::FindInSource { doc, target, gen } => {
                    let path = jsonquery_core::find_path(&doc.root, &target);
                    send(&evt_tx, Event::Found { gen, path }, &wake);
                }
                Command::Search {
                    root,
                    text,
                    regex,
                    gen,
                } => {
                    let value = match &root {
                        SearchRoot::Source(doc) => &doc.root,
                        SearchRoot::Results(v) => v.as_ref(),
                    };
                    match jsonquery_core::search(value, &text, regex) {
                        Ok(matches) => send(&evt_tx, Event::SearchDone { gen, matches }, &wake),
                        Err(e) => send(
                            &evt_tx,
                            Event::SearchError {
                                gen,
                                error: format!("{e:#}"),
                            },
                            &wake,
                        ),
                    }
                }
                Command::RenderText {
                    target,
                    node_budget,
                    gen,
                } => {
                    let kind = target.kind();
                    let value = match &target {
                        TextTarget::Source(doc) => &doc.root,
                        TextTarget::Results(v) => v,
                    };
                    let (text, truncated) =
                        jsonquery_core::pretty_print_bounded(value, node_budget);
                    send(
                        &evt_tx,
                        Event::TextRendered {
                            target: kind,
                            gen,
                            text,
                            truncated,
                        },
                        &wake,
                    );
                }
                Command::Query {
                    doc,
                    text,
                    engine,
                    gen,
                    cancel,
                } => {
                    run_query(&evt_tx, &doc, &text, engine, gen, &cancel, &wake);
                }
            }
        }
    });
}

/// Download `url`'s body into a temporary file, then parse it with the same
/// mmap-backed path a locally opened file would use (Architecture §1) —
/// keeping URL downloads on the same footing as local files rather than
/// materializing the whole response in RAM up front.
fn download_to_temp_file(url: &str) -> anyhow::Result<Document> {
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("requesting {url}"))?;

    let mut body = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_BYTES)
        .reader();

    let temp_path = temp_path_for_url(url);
    let mut file = std::fs::File::create(&temp_path)
        .with_context(|| format!("creating temporary file {}", temp_path.display()))?;
    std::io::copy(&mut body, &mut file).with_context(|| format!("downloading {url}"))?;
    drop(file);

    let mut doc =
        jsonquery_core::load(&temp_path).with_context(|| format!("parsing data from {url}"))?;
    doc.source = DocumentSource::Url(url.to_string());
    Ok(doc)
}

/// A temp-dir path derived from `url`'s last path segment, prefixed with the
/// pid and a nanosecond timestamp for uniqueness. The prefix also neutralizes
/// any path-traversal attempt in that segment (e.g. a URL ending in `/..`):
/// whatever it contains becomes one literal filename component, never `/`.
fn temp_path_for_url(url: &str) -> PathBuf {
    let path_part = url.split(['?', '#']).next().unwrap_or(url);
    let file_name = path_part
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("download.json");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "jsonquery_gui-{}-{nanos}-{file_name}",
        std::process::id()
    ))
}

/// Write a value out as pretty-printed JSON — used by both "Save…" buttons,
/// for the loaded source document and for the current query results.
fn save_json(value: &serde_json::Value, path: &Path) -> anyhow::Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), value)
        .with_context(|| format!("writing {}", path.display()))
}

fn send_load_result(
    evt_tx: &Sender<Event>,
    result: anyhow::Result<Arc<Document>>,
    wake: &impl Fn(),
) {
    match result {
        Ok(doc) => send(evt_tx, Event::Loaded(doc), wake),
        Err(e) => send(evt_tx, Event::LoadError(format!("{e:#}")), wake),
    }
}

fn run_query(
    evt_tx: &Sender<Event>,
    doc: &Document,
    text: &str,
    engine: jsonquery_query::Kind,
    gen: u64,
    cancel: &AtomicBool,
    wake: &impl Fn(),
) {
    let start = Instant::now();
    let result = engine
        .engine()
        .run(&doc.root, text, cancel, &mut |event| match event {
            QueryEvent::Item(value) => send(evt_tx, Event::QueryItem { gen, value }, wake),
            QueryEvent::ItemError(error) => {
                send(evt_tx, Event::QueryItemError { gen, error }, wake)
            }
        });

    match result {
        Ok(_count) => send(
            evt_tx,
            Event::QueryDone {
                gen,
                cancelled: cancel.load(Ordering::Relaxed),
                elapsed: start.elapsed(),
            },
            wake,
        ),
        Err(e) => send(
            evt_tx,
            Event::QueryError {
                gen,
                error: e.to_string(),
            },
            wake,
        ),
    }
}

fn send(evt_tx: &Sender<Event>, event: Event, wake: &impl Fn()) {
    // The receiver is dropped only when the app is shutting down; a failed
    // send at that point is expected and safe to ignore.
    let _ = evt_tx.send(event);
    wake();
}
