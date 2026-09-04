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
use jsonquery_core::{Document, DocumentSource};
use jsonquery_query::QueryEvent;

/// Cap on a URL download's response body, matching the "a few GB" v1 scale
/// ceiling (Architecture §3) — protects against a malicious or misbehaving
/// server exhausting disk/memory via an unbounded response.
const MAX_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;

pub enum Command {
    OpenFile(PathBuf),
    OpenText(String),
    OpenUrl(String),
    SaveFile {
        doc: Arc<Document>,
        path: PathBuf,
    },
    SaveResults {
        results: serde_json::Value,
        path: PathBuf,
    },
    Query {
        doc: Arc<Document>,
        text: String,
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
                Command::SaveFile { doc, path } => {
                    let result = save_json(&doc.root, &path);
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
                Command::Query {
                    doc,
                    text,
                    gen,
                    cancel,
                } => {
                    run_query(&evt_tx, &doc, &text, gen, &cancel, &wake);
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
    gen: u64,
    cancel: &AtomicBool,
    wake: &impl Fn(),
) {
    let start = Instant::now();
    let result = jsonquery_query::run_query(&doc.root, text, cancel, |event| match event {
        QueryEvent::Item(value) => send(evt_tx, Event::QueryItem { gen, value }, wake),
        QueryEvent::ItemError(error) => send(evt_tx, Event::QueryItemError { gen, error }, wake),
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
