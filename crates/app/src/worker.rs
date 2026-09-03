//! Background worker thread (Architecture §5). The UI thread never touches
//! the filesystem or the query engine directly — it sends [`Command`]s here
//! and receives [`Event`]s back over a `crossbeam-channel` pair, then wakes
//! the egui context so a repaint picks up the new state.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use jsonquery_core::Document;
use jsonquery_query::QueryEvent;

pub enum Command {
    OpenFile(PathBuf),
    OpenText(String),
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
    QueryItem { gen: u64, value: serde_json::Value },
    QueryItemError { gen: u64, error: String },
    QueryDone { gen: u64, cancelled: bool, elapsed: Duration },
    QueryError { gen: u64, error: String },
}

/// Spawn the worker thread. `wake` is called after every event is sent so
/// the (otherwise idle, redraw-on-demand) egui context repaints promptly.
pub fn spawn(
    cmd_rx: Receiver<Command>,
    evt_tx: Sender<Event>,
    wake: impl Fn() + Send + 'static,
) {
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
                Command::Query { doc, text, gen, cancel } => {
                    run_query(&evt_tx, &doc, &text, gen, &cancel, &wake);
                }
            }
        }
    });
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
        Err(e) => send(evt_tx, Event::QueryError { gen, error: e.to_string() }, wake),
    }
}

fn send(evt_tx: &Sender<Event>, event: Event, wake: &impl Fn()) {
    // The receiver is dropped only when the app is shutting down; a failed
    // send at that point is expected and safe to ignore.
    let _ = evt_tx.send(event);
    wake();
}
