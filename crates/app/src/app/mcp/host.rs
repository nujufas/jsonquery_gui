//! What the MCP server's tools do in the app.
//!
//! The server answers on threads of its own. What only looks (`run_query`,
//! `describe`, `read_value`, `search`) runs on a worker of its own too — a second one,
//! the same as the app's, so that the assistant's work never waits behind the user's
//! and the user's never behind the assistant's — over the document the app already
//! holds, never loaded twice.
//!
//! What changes what the app shows is done to the app itself, under its lock, as a
//! click would do it, and the windows are asked to show it. A call that waits for the
//! app (a document to load, a query to run) does not wait for the app's frames, which
//! a hidden or covered window does not get: it takes in what the app's worker has said
//! itself, as a frame would.

mod compute;
mod keep;
mod look;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use jsonquery_core::{resolve, Document, DocumentSource, NodePath, PathSegment, ValueView};
use jsonquery_mcp::shape::{self, pointer_of, resolve_pointer};
use jsonquery_mcp::{Call, Host, Input, Language, Target};
use jsonquery_query::{Kind, OutputFormat};
use serde_json::{json, Map, Value};

use super::super::{expand_home, lock, wake_windows, App, PanelKind};
use crate::settings::FileLimits;
use crate::tools::Place;
use crate::worker::{self, Command, Event, SearchRoot};
use crate::workspace::{address_in, Folder, Item, ItemKind, Workspace};
use crate::workspace_pane::QUERY_LIMIT;

use self::look::{json_size, on_view, Kept, Viewed, KEEP_BYTES, LOAD_TIME};

/// How often a call that waits for the app looks again.
const POLL: Duration = Duration::from_millis(20);
/// How long a search may take.
const SEARCH_TIME: Duration = Duration::from_secs(30);
/// How long a preview of a value found by a search may be, in characters.
const PREVIEW_CHARS: usize = 120;
/// The most matches of a search the worker finds (`jsonquery_core::search`).
const MAX_SEARCH_MATCHES: usize = 5_000;

const NO_DOCUMENT: &str = "no document is open in the app: give the tool a file as its input, \
                           open one with open_document (both need Edit), or ask the user to";

pub(super) struct AppHost {
    app: Weak<Mutex<App>>,
    ctx: egui::Context,
    worker: Mutex<Worker>,
    /// The file a tool last looked at, as it was loaded for the assistant.
    file: Mutex<Option<LoadedFile>>,
    /// What the assistant's last run_query, or tool of the Tools, gave ("last").
    last: Mutex<Option<Kept>>,
}

/// The worker that does what only looks.
struct Worker {
    commands: Sender<Command>,
    events: Receiver<Event>,
    gen: u64,
}

struct LoadedFile {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
    doc: Arc<Document>,
}

/// How a job of the worker ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
    Done,
    Cancelled,
    TimedOut,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AppHost {
    pub(super) fn new(app: Weak<Mutex<App>>, ctx: egui::Context, limits: FileLimits) -> Self {
        let (commands, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, events) = crossbeam_channel::unbounded();
        worker::spawn(command_rx, event_tx, limits, || {});
        AppHost {
            app,
            ctx,
            worker: Mutex::new(Worker {
                commands,
                events,
                gen: 0,
            }),
            file: Mutex::new(None),
            last: Mutex::new(None),
        }
    }

    /// Do `f` with the app, once it has taken in what its worker has said.
    fn with_app<T>(&self, f: impl FnOnce(&mut App) -> T) -> Result<T, String> {
        let app = self
            .app
            .upgrade()
            .ok_or_else(|| "JsonQuery is closing".to_owned())?;
        let mut app = lock(&app);
        if !app.evt_rx.is_empty() {
            app.drain_events(&self.ctx);
            wake_windows(&self.ctx);
        }
        Ok(f(&mut app))
    }

    /// Ask every window to show what was changed.
    fn shown(&self) {
        wake_windows(&self.ctx);
    }

    /// Look at the app until `check` says what came of it, the call is cancelled, or
    /// `deadline` passes (`None`).
    fn wait<T>(
        &self,
        cancel: &AtomicBool,
        deadline: Instant,
        mut check: impl FnMut(&mut App) -> Option<T>,
    ) -> Result<Option<T>, String> {
        loop {
            if let Some(answer) = self.with_app(&mut check)? {
                return Ok(Some(answer));
            }
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".to_owned());
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(POLL);
        }
    }

    /// Have the worker do `command` (made with the job's number) under `limits`, and
    /// give what it says to `take` until `take` says the job is done.
    fn job(
        &self,
        limits: FileLimits,
        command: impl FnOnce(u64) -> Command,
        cancel: &AtomicBool,
        deadline: Instant,
        mut take: impl FnMut(u64, Event) -> bool,
    ) -> Result<Wait, String> {
        let mut worker = locked(&self.worker);
        // What an earlier job that was given up on said since is of no use.
        while worker.events.try_recv().is_ok() {}
        worker.gen += 1;
        let gen = worker.gen;
        let gone = || "the app's worker has stopped".to_owned();
        worker
            .commands
            .send(Command::UseLimits(limits))
            .map_err(|_| gone())?;
        worker.commands.send(command(gen)).map_err(|_| gone())?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(Wait::Cancelled);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(Wait::TimedOut);
            }
            match worker.events.recv_timeout(left.min(POLL * 5)) {
                Ok(event) => {
                    if take(gen, event) {
                        return Ok(Wait::Done);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(gone()),
            }
        }
    }

    /// The document the app has open, and the limits on files it goes by.
    fn source(&self) -> Result<(Arc<Document>, FileLimits), String> {
        let (doc, limits) = self.with_app(|app| (app.doc.clone(), app.settings.limits))?;
        Ok((doc.ok_or(NO_DOCUMENT)?, limits))
    }

    /// The JSON file at `path`, loaded by the worker (or as it was, if it has not
    /// changed since `run_query` last ran on it).
    fn load_file(
        &self,
        path: &Path,
        limits: FileLimits,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Arc<Document>, String> {
        let path = full_path(path)?;
        let meta = std::fs::metadata(&path)
            .map_err(|e| format!("{} could not be read: {e}", path.display()))?;
        let modified = meta.modified().ok();
        if let Some(kept) = locked(&self.file).as_ref() {
            if kept.path == path && kept.modified == modified && kept.len == meta.len() {
                return Ok(kept.doc.clone());
            }
        }
        let mut loaded = None;
        let wait = self.job(
            limits,
            |_| Command::OpenFile(path.clone()),
            cancel,
            deadline,
            |_, event| match event {
                Event::Loaded(doc) if matches!(&doc.source, DocumentSource::File(p) if *p == path) => {
                    loaded = Some(Ok(doc));
                    true
                }
                Event::LoadError(e) => {
                    loaded = Some(Err(e));
                    true
                }
                _ => false,
            },
        )?;
        match (wait, loaded) {
            (_, Some(Ok(doc))) => {
                *locked(&self.file) = Some(LoadedFile {
                    path,
                    modified,
                    len: meta.len(),
                    doc: doc.clone(),
                });
                Ok(doc)
            }
            (_, Some(Err(e))) => Err(e),
            (Wait::Cancelled, None) => Err("cancelled".to_owned()),
            _ => Err(format!(
                "{} is still loading: give a larger timeout_ms",
                path.display()
            )),
        }
    }

    fn state(&self) -> Result<Value, String> {
        self.with_app(state_of)
    }

    #[allow(clippy::too_many_arguments)]
    fn describe(
        &self,
        input: &Input,
        pointer: &str,
        depth: usize,
        sample: usize,
        max_bytes: usize,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let (viewed, _) = self.viewed(input, cancel, Instant::now() + LOAD_TIME)?;
        on_view!(&viewed, root => {
            let (value, path) = resolve_pointer(root, pointer)?;
            Ok(with_pointer(shape::describe(value, depth, sample, max_bytes), &path))
        })
    }

    fn read_value(
        &self,
        input: &Input,
        pointer: &str,
        offset: usize,
        limit: usize,
        max_bytes: usize,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let (viewed, _) = self.viewed(input, cancel, Instant::now() + LOAD_TIME)?;
        on_view!(&viewed, root => {
            let (value, path) = resolve_pointer(root, pointer)?;
            Ok(shape::read(value, &path, offset, limit, max_bytes))
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn search(
        &self,
        input: &Input,
        text: String,
        regex: bool,
        offset: usize,
        limit: usize,
        max_bytes: usize,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let (viewed, name) = self.viewed(input, cancel, Instant::now() + LOAD_TIME)?;
        let limits = self.with_app(|app| app.settings.limits)?;
        let root = match &viewed {
            Viewed::Doc(doc) => SearchRoot::Source(doc.clone()),
            Viewed::Value(value) => SearchRoot::Results(value.clone()),
        };
        let mut found = None;
        let wait = self.job(
            limits,
            |gen| Command::Search {
                root,
                text,
                regex,
                gen,
            },
            cancel,
            Instant::now() + SEARCH_TIME,
            |gen, event| match event {
                Event::SearchDone { gen: g, matches } if g == gen => {
                    found = Some(Ok(matches));
                    true
                }
                Event::SearchError { gen: g, error } if g == gen => {
                    found = Some(Err(error));
                    true
                }
                _ => false,
            },
        )?;
        let matches = match (wait, found) {
            (_, Some(found)) => found?,
            (Wait::Cancelled, None) => return Err("cancelled".to_owned()),
            _ => return Err("the search took too long: look for something rarer".to_owned()),
        };
        let shown: Vec<Value> = on_view!(&viewed, root => matches
            .iter()
            .skip(offset)
            .take(limit)
            .map(|path| found_at(resolve(root, path), path))
            .collect());
        let (page, _) = shape::page(&shown, offset, offset, limit, max_bytes);
        let end = offset + page.len();
        let mut out = Map::new();
        out.insert("of".to_owned(), json!(name));
        out.insert("count".to_owned(), json!(matches.len()));
        if matches.len() >= MAX_SEARCH_MATCHES {
            out.insert(
                "note".to_owned(),
                json!(format!("the search stops at {MAX_SEARCH_MATCHES} matches")),
            );
        }
        out.insert("offset".to_owned(), json!(offset));
        out.insert("matches".to_owned(), Value::Array(page));
        if end < matches.len() {
            out.insert("next_offset".to_owned(), json!(end));
        }
        Ok(Value::Object(out))
    }

    #[allow(clippy::too_many_arguments)]
    fn run_query(
        &self,
        query: String,
        language: Option<Language>,
        input: &Input,
        offset: usize,
        limit: usize,
        max_bytes: usize,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let deadline = Instant::now() + timeout;
        let (viewed, name) = self.viewed(input, cancel, deadline)?;
        let limits = self.with_app(|app| app.settings.limits)?;
        let doc = match viewed {
            Viewed::Doc(doc) => doc,
            Viewed::Value(value) => Arc::new(Document::from_value(
                Arc::unwrap_or_clone(value),
                DocumentSource::Pasted,
                0,
                Duration::ZERO,
            )),
        };
        let engine = language.unwrap_or_else(|| Kind::detect(&query));
        let stop = Arc::new(AtomicBool::new(false));
        let mut kept = Vec::new();
        // All of them, to keep as "last", while they are not too many.
        let mut all = Some(Vec::new());
        let mut all_bytes = 0usize;
        let mut count = 0usize;
        let mut item_errors = 0usize;
        let mut first_item_error = None;
        let mut error = None;
        let mut finished: Option<(bool, Duration)> = None;
        let started = Instant::now();
        let wait = self.job(
            limits,
            |gen| Command::Query {
                doc,
                text: query.clone(),
                engine,
                gen,
                cancel: stop.clone(),
            },
            cancel,
            deadline,
            |gen, event| match event {
                Event::QueryItem { gen: g, value } if g == gen => {
                    if let Some(list) = &mut all {
                        all_bytes += json_size(&value) + 1;
                        if all_bytes > KEEP_BYTES {
                            all = None;
                        } else {
                            list.push(value.clone());
                        }
                    }
                    if count >= offset && kept.len() < limit {
                        kept.push(value);
                    }
                    count += 1;
                    false
                }
                Event::QueryItemError { gen: g, error } if g == gen => {
                    item_errors += 1;
                    first_item_error.get_or_insert(error);
                    false
                }
                Event::QueryDone {
                    gen: g,
                    cancelled,
                    elapsed,
                } if g == gen => {
                    finished = Some((cancelled, elapsed));
                    true
                }
                Event::QueryError { gen: g, error: e } if g == gen => {
                    error = Some(e);
                    true
                }
                _ => false,
            },
        )?;
        if wait != Wait::Done {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(error) = error {
            return Err(error);
        }
        if wait == Wait::Cancelled {
            return Err("cancelled".to_owned());
        }
        let complete = matches!(finished, Some((false, _)));
        let (items, next) = shape::page(&kept, offset, offset, limit, max_bytes);
        let end = offset + items.len();
        let next = next.or_else(|| (end < count).then_some(end));
        let mut out = Map::new();
        out.insert("of".to_owned(), json!(name));
        out.insert("language".to_owned(), json!(language_key(engine)));
        out.insert("count".to_owned(), json!(count));
        out.insert("complete".to_owned(), json!(complete));
        out.insert("offset".to_owned(), json!(offset));
        out.insert("items".to_owned(), Value::Array(items));
        if let Some(next) = next {
            out.insert("next_offset".to_owned(), json!(next));
        }
        if item_errors > 0 {
            out.insert("item_errors".to_owned(), json!(item_errors));
            out.insert("first_item_error".to_owned(), json!(first_item_error));
        }
        let elapsed = finished.map_or_else(|| started.elapsed(), |(_, e)| e);
        out.insert("elapsed_ms".to_owned(), json!(elapsed.as_millis() as u64));
        if wait == Wait::TimedOut {
            out.insert(
                "note".to_owned(),
                json!(format!(
                    "stopped after {} ms, so `count` is only what came before: narrow the \
                     query, or give a larger timeout_ms",
                    timeout.as_millis()
                )),
            );
        }
        if OutputFormat::detect(engine, &query) != OutputFormat::Json {
            out.insert(
                "format".to_owned(),
                json!("each item is a row of text (the query ends in @csv or @tsv)"),
            );
        }
        match all.filter(|_| complete) {
            Some(all) => {
                self.keep(
                    Value::Array(all),
                    format!("the results of run_query {query:?} on {name}"),
                );
                out.insert("last".to_owned(), json!("kept: give \"last\" as an input"));
            }
            None => {
                out.insert(
                    "last".to_owned(),
                    json!(if complete {
                        format!("not kept: more than {} MB", KEEP_BYTES >> 20)
                    } else {
                        "not kept: the query did not finish".to_owned()
                    }),
                );
            }
        }
        Ok(Value::Object(out))
    }

    fn results(&self, offset: usize, limit: usize, max_bytes: usize) -> Result<Value, String> {
        self.with_app(|app| results_of(app, offset, limit, max_bytes))
    }

    fn open_document(
        &self,
        path: Option<PathBuf>,
        text: Option<String>,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let path = path.map(|p| full_path(&p)).transpose()?;
        let before = self.with_app(|app| {
            let before = app.doc.as_ref().map(Arc::as_ptr);
            app.tools.show_query();
            app.load_error = None;
            match (path, text) {
                (Some(path), _) => app.open_file(path),
                (None, Some(text)) => app.open_text(text),
                (None, None) => {}
            }
            before
        })?;
        self.shown();
        self.loaded_since(before, timeout, cancel)
    }

    /// Wait for the document asked for to have loaded (one that is not `before`), and
    /// say what it is.
    fn loaded_since(
        &self,
        before: Option<*const Document>,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let opened = self.wait(cancel, Instant::now() + timeout, |app| {
            if let Some(e) = &app.load_error {
                return Some(Err(e.clone()));
            }
            if app.loading {
                return None;
            }
            match &app.doc {
                Some(doc) if Some(Arc::as_ptr(doc)) != before => Some(Ok(document_of(doc))),
                _ => None,
            }
        })?;
        match opened {
            Some(Ok(document)) => Ok(json!({"opened": true, "document": document})),
            Some(Err(e)) => Err(e),
            None => Ok(json!({
                "opened": false,
                "note": "it is still loading: call get_state in a moment"
            })),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn set_query(
        &self,
        query: String,
        language: Option<Language>,
        run: bool,
        limit: usize,
        max_bytes: usize,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let gen = self.with_app(|app| {
            app.tools.show_query();
            app.query_engine = language;
            app.query_text = query;
            app.query_follow_cursor = true;
            app.query_suggest.close();
            if run && app.doc.is_some() {
                app.run_query();
                Some(app.query_gen)
            } else {
                None
            }
        })?;
        self.shown();
        let Some(gen) = gen else {
            return Ok(if run {
                json!({"set": true, "ran": false, "note": NO_DOCUMENT})
            } else {
                json!({"set": true, "ran": false})
            });
        };
        let ran = self.wait(cancel, Instant::now() + timeout, |app| {
            if app.query_gen != gen {
                return Some(Err(
                    "the query was set and run, then another query was run in the app \
                     before it finished (the user's, perhaps)"
                        .to_owned(),
                ));
            }
            if app.query_running {
                return None;
            }
            Some(match &app.query_error {
                Some(e) => Err(format!(
                    "the query is in the query box, and running it gave an error: {e}"
                )),
                None => Ok(results_of(app, 0, limit, max_bytes)),
            })
        })?;
        match ran {
            Some(answer) => answer,
            None => self.with_app(|app| {
                let mut out = results_of(app, 0, limit, max_bytes);
                out["note"] =
                    json!("it is still running in the app: get_results gives what it has later");
                out
            }),
        }
    }

    fn reveal(&self, target: Target, pointer: &str) -> Result<Value, String> {
        let shown = self.with_app(|app| {
            let (panel, path, what) = match target {
                Target::Source => {
                    let doc = app.doc.clone().ok_or(NO_DOCUMENT)?;
                    let (value, path) = resolve_pointer(doc.root(), pointer)?;
                    (PanelKind::Source, path, shape::summary(&value))
                }
                Target::Results => {
                    let (value, path) = resolve_pointer(&app.results, pointer)?;
                    (PanelKind::Results, path, shape::summary(&value))
                }
            };
            app.tools.show_query();
            app.reveal_in_tree(panel, path.clone());
            Ok::<_, String>(json!({
                "revealed": pointer_of(&path),
                "in": target.key(),
                "value": what
            }))
        })??;
        self.shown();
        Ok(shown)
    }

    fn workspace_list(
        &self,
        folder: Option<String>,
        offset: usize,
        max_bytes: usize,
    ) -> Result<Value, String> {
        self.with_app(|app| {
            let root = app.workspace_pane.root().map(Path::to_path_buf);
            let Some(ws) = app.workspace_pane.workspace() else {
                return Ok(json!({"folder": null, "items": [],
                                 "note": "the app has no workspace folder"}));
            };
            let start = match &folder {
                Some(path) => ws.tree().find_folder(path.trim_matches('/')).ok_or_else(|| {
                    format!("the workspace has no folder \"{path}\"; workspace_list without one lists them all")
                })?,
                None => ws.tree(),
            };
            let mut items = Vec::new();
            let mut folders = Vec::new();
            list_folder(start, &mut items, &mut folders);
            let mut out = Map::new();
            out.insert(
                "folder".to_owned(),
                json!(root.map(|r| r.display().to_string())),
            );
            // The folders on the first page, in a quarter of it at most.
            if offset == 0 {
                let folders: Vec<Value> = folders.into_iter().map(Value::String).collect();
                let (shown, _) = shape::page(&folders, 0, 0, usize::MAX, max_bytes / 4);
                if shown.len() < folders.len() {
                    out.insert(
                        "more_folders".to_owned(),
                        json!(folders.len() - shown.len()),
                    );
                }
                out.insert("folders".to_owned(), Value::Array(shown));
            }
            let used = Value::Object(out.clone()).to_string().len();
            let (page, next) = shape::page(
                &items,
                0,
                offset,
                usize::MAX,
                max_bytes.saturating_sub(used).max(500),
            );
            out.insert("count".to_owned(), json!(items.len()));
            out.insert("offset".to_owned(), json!(offset));
            out.insert("items".to_owned(), Value::Array(page));
            if let Some(next) = next {
                out.insert("next_offset".to_owned(), json!(next));
            }
            Ok(Value::Object(out))
        })?
    }

    fn workspace_read(&self, path: &str, sample: bool, max_bytes: usize) -> Result<Value, String> {
        self.with_app(|app| {
            let ws = app
                .workspace_pane
                .workspace()
                .ok_or("the app has no workspace folder")?;
            let item = ws.item(path).cloned().ok_or_else(|| {
                format!("the workspace has no item \"{path}\": workspace_list gives their paths")
            })?;
            let mut out = item_of(&item);
            let limit = max_bytes as u64;
            match item.kind {
                ItemKind::Query(_) => {
                    out["text"] = json!(ws.read_text(&item.path, QUERY_LIMIT)?);
                }
                ItemKind::Url => {
                    let text = ws.read_text(&item.path, QUERY_LIMIT)?;
                    out["url"] = json!(address_in(&text));
                }
                ItemKind::Schema | ItemKind::Patch | ItemKind::Document => {
                    out["content"] = file_content(ws, &item.path, item.bytes, limit)?;
                }
            }
            if sample {
                out["sample"] = match &item.sample {
                    Some(s) => file_content(ws, &s.path, s.bytes, limit)?,
                    None => Value::Null,
                };
            }
            Ok(out)
        })?
    }

    fn workspace_open(
        &self,
        path: &str,
        sample: bool,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let ctx = self.ctx.clone();
        self.with_app(|app| {
            // The workspace is read if it was not yet.
            let ws = app
                .workspace_pane
                .workspace()
                .ok_or("the app has no workspace folder")?;
            // Loading a web address would send a request from this computer to wherever
            // the assistant was led to (by what a document says, say): the user's to do.
            if ws.item(path).is_some_and(|item| item.kind == ItemKind::Url) {
                return Err(format!(
                    "\"{path}\" is a web address: the server does not load addresses. \
                     workspace_read gives it; the user can open it in the Workspace"
                ));
            }
            let action = app
                .workspace_pane
                .item_action(path, sample)?
                .ok_or_else(|| {
                    format!(
                        "the workspace has no item \"{path}\": workspace_list gives their paths"
                    )
                })?;
            app.workspace_pane.act(action);
            app.apply_workspace_actions(&ctx);
            Ok::<_, String>(())
        })??;
        self.shown();
        // What it opened has loaded, and what it ran has run.
        self.wait(cancel, Instant::now() + timeout, |app| {
            (!app.loading && !app.query_running && !app.run_after_load).then_some(())
        })?;
        self.with_app(|app| {
            let mut out = state_of(app);
            out["opened"] = json!(path);
            if let Some(e) = &app.load_error {
                out["error"] = json!(e);
            }
            out
        })
    }
}

impl Host for AppHost {
    fn call(&self, call: Call, cancel: &Arc<AtomicBool>) -> Result<Value, String> {
        match call {
            Call::State => self.state(),
            Call::Info {
                input,
                pointer,
                max_bytes,
            } => self.document_info(&input, &pointer, max_bytes, cancel),
            Call::Describe {
                input,
                pointer,
                depth,
                sample,
                max_bytes,
            } => self.describe(&input, &pointer, depth, sample, max_bytes, cancel),
            Call::ReadValue {
                input,
                pointer,
                offset,
                limit,
                max_bytes,
            } => self.read_value(&input, &pointer, offset, limit, max_bytes, cancel),
            Call::Search {
                input,
                text,
                regex,
                offset,
                limit,
                max_bytes,
            } => self.search(&input, text, regex, offset, limit, max_bytes, cancel),
            Call::RunQuery {
                query,
                language,
                input,
                offset,
                limit,
                max_bytes,
                timeout,
            } => self.run_query(
                query, language, &input, offset, limit, max_bytes, timeout, cancel,
            ),
            Call::Results {
                offset,
                limit,
                max_bytes,
            } => self.results(offset, limit, max_bytes),
            Call::OpenDocument {
                path,
                text,
                timeout,
            } => self.open_document(path, text, timeout, cancel),
            Call::SetQuery {
                query,
                language,
                run,
                limit,
                max_bytes,
                timeout,
            } => self.set_query(query, language, run, limit, max_bytes, timeout, cancel),
            Call::Reveal { target, pointer } => self.reveal(target, &pointer),
            Call::WorkspaceList {
                folder,
                offset,
                max_bytes,
            } => self.workspace_list(folder, offset, max_bytes),
            Call::WorkspaceRead {
                path,
                sample,
                max_bytes,
            } => self.workspace_read(&path, sample, max_bytes),
            Call::WorkspaceOpen {
                path,
                sample,
                timeout,
            } => self.workspace_open(&path, sample, timeout, cancel),
            Call::Format {
                input,
                options,
                max_bytes,
            } => self.format_json(&input, options, max_bytes, cancel),
            Call::Diff {
                left,
                right,
                offset,
                limit,
                as_patch,
                max_bytes,
                timeout,
            } => self.diff_documents(
                &left, &right, offset, limit, as_patch, max_bytes, timeout, cancel,
            ),
            Call::Patch {
                input,
                patch,
                merge_patch,
                max_bytes,
            } => self.apply_patch(&input, &patch, merge_patch, max_bytes, cancel),
            Call::Merge {
                inputs,
                filter,
                max_bytes,
                timeout,
            } => self.merge_documents(&inputs, &filter, max_bytes, timeout, cancel),
            Call::Validate {
                input,
                schema,
                check_formats,
                offset,
                limit,
                max_bytes,
                timeout,
            } => self.validate_json(
                &input,
                &schema,
                check_formats,
                offset,
                limit,
                max_bytes,
                timeout,
                cancel,
            ),
            Call::CreateSchema {
                input,
                options,
                max_bytes,
                timeout,
            } => self.create_schema(&input, options, max_bytes, timeout, cancel),
            Call::ShowInTool { page, inputs, run } => self.show_in_tool(&page, &inputs, run),
            Call::WorkspaceSave {
                folder,
                name,
                kind,
                content,
                sample,
                replace,
            } => self.workspace_save(
                &folder,
                &name,
                kind,
                &content,
                sample.as_ref(),
                replace,
                cancel,
            ),
            Call::WorkspaceNewFolder { path } => self.workspace_new_folder(&path),
            Call::WorkspaceMove { path, to, name } => {
                self.workspace_move(&path, to.as_deref(), name.as_deref())
            }
            Call::WorkspaceDelete { path } => self.workspace_delete(&path, cancel),
            Call::SaveFile {
                input,
                path,
                indent,
                replace,
            } => self.save_file(&input, &path, indent, replace, cancel),
            Call::OpenUrl { url, timeout } => self.open_url(&url, timeout, cancel),
            // Answered by the server itself.
            Call::Help { .. } => Err("query_help is answered by the server".to_owned()),
        }
    }
}

fn language_key(language: Language) -> &'static str {
    jsonquery_mcp::language_key(language)
}

/// A path the app can open: a full one (`~` is the home).
fn full_path(path: &Path) -> Result<PathBuf, String> {
    let text = path.to_string_lossy();
    if text.contains("://") {
        return Err("give a file's path: web addresses are not opened from here".to_owned());
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let path = expand_home(&text, home.map(PathBuf::from));
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "\"{}\" is not a full path: give one from the root (or from ~)",
            path.display()
        ))
    }
}

fn with_pointer(mut shape: Value, path: &NodePath) -> Value {
    if let Value::Object(map) = &mut shape {
        let mut out = Map::new();
        out.insert("pointer".to_owned(), json!(pointer_of(path)));
        out.append(map);
        return Value::Object(out);
    }
    shape
}

/// What a search found at `path`: where, under what key, and what it is.
fn found_at<V: ValueView>(node: Option<V>, path: &NodePath) -> Value {
    let mut out = Map::new();
    out.insert("pointer".to_owned(), json!(pointer_of(path)));
    if let Some(PathSegment::Key(key)) = path.last() {
        out.insert("key".to_owned(), json!(key));
    }
    if let Some(node) = node {
        let preview = match node.scalar_preview() {
            Some(preview) if !node.kind().is_container() => {
                let mut cut: String = preview.chars().take(PREVIEW_CHARS).collect();
                if cut.len() < preview.len() {
                    cut.push('…');
                }
                cut
            }
            _ => shape::summary(&node),
        };
        out.insert("value".to_owned(), json!(preview));
    }
    Value::Object(out)
}

/// What the app has open, for `get_state`.
fn state_of(app: &mut App) -> Value {
    let engine = app
        .query_engine
        .unwrap_or_else(|| Kind::detect(&app.query_text));
    let front = if app.tools.page_in_front() {
        app.tools.current_tool().label()
    } else {
        crate::tools::QUERY_TAB
    };
    let tools = match (app.tools.place(), app.tools.in_window()) {
        (Place::Docked, _) => "docked as tabs of the main window",
        (Place::Window, true) => "open in a window of their own",
        (Place::Window, false) => "closed",
    };
    let workspace_folder = app.workspace_pane.root().map(|r| r.display().to_string());
    let workspace_items = app
        .workspace_pane
        .workspace()
        .map(|ws| ws.tree().item_count());
    let document = match &app.doc {
        Some(doc) => document_of(doc),
        None => Value::Null,
    };
    json!({
        "app": format!("JsonQuery {}", env!("CARGO_PKG_VERSION")),
        "document": document,
        "loading": app.loading,
        "load_error": app.load_error,
        "query": {
            "text": app.query_text,
            "language": language_key(engine),
            "language_chosen": app.query_engine.is_some(),
            "running": app.query_running,
            "error": app.query_error,
            "results": app.last_query_elapsed.is_some().then(|| json!({
                "count": app.results_count_so_far,
                "kept": app.results.as_array().map_or(0, Vec::len),
                "item_errors": app.results_item_errors,
                "cancelled": app.last_query_cancelled,
                "elapsed_ms": app.last_query_elapsed.map(|e| e.as_millis() as u64),
            })),
        },
        "in_front": front,
        "tools": tools,
        "workspace": {"folder": workspace_folder, "items": workspace_items},
        "access": app.mcp.access().key(),
    })
}

/// What a document is, without its data.
fn document_of(doc: &Document) -> Value {
    let (source, place) = match &doc.source {
        DocumentSource::File(path) => ("file", json!(path.display().to_string())),
        DocumentSource::Url(url) => ("web address", json!(url)),
        DocumentSource::Pasted => ("pasted", Value::Null),
        DocumentSource::Merged(files) => (
            "merged",
            json!(files
                .iter()
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()),
        ),
        DocumentSource::Derived { label, .. } => ("made by a tool", json!(label)),
    };
    let mut out = Map::new();
    out.insert("source".to_owned(), json!(source));
    if !place.is_null() {
        out.insert(
            if source == "merged" { "files" } else { "from" }.to_owned(),
            place,
        );
    }
    out.insert("bytes".to_owned(), json!(doc.byte_len));
    out.insert("top".to_owned(), json!(shape::summary(&doc.root())));
    if doc.top_level_values > 1 {
        out.insert(
            "note".to_owned(),
            json!(format!(
                "{} values one after the other (NDJSON): the app takes them as one array",
                doc.top_level_values
            )),
        );
    }
    if doc.is_lazy() {
        out.insert(
            "kept_on_disk".to_owned(),
            json!(
                "too big to load: it is read from its file as it is looked at, so a few \
                   jq functions that need all of it at once may not work"
            ),
        );
    }
    Value::Object(out)
}

/// A page of the results the user sees, with what they are the results of.
fn results_of(app: &App, offset: usize, limit: usize, max_bytes: usize) -> Value {
    let engine = app
        .query_engine
        .unwrap_or_else(|| Kind::detect(&app.query_text));
    let kept: &[Value] = app.results.as_array().map_or(&[], Vec::as_slice);
    let (items, next) = shape::page(kept, 0, offset, limit, max_bytes);
    let mut out = Map::new();
    out.insert("query".to_owned(), json!(app.query_text));
    out.insert("language".to_owned(), json!(language_key(engine)));
    out.insert("running".to_owned(), json!(app.query_running));
    if let Some(e) = &app.query_error {
        out.insert("error".to_owned(), json!(e));
    }
    out.insert("count".to_owned(), json!(app.results_count_so_far));
    out.insert("offset".to_owned(), json!(offset));
    let end = offset + items.len();
    out.insert("items".to_owned(), Value::Array(items));
    if let Some(next) = next.or_else(|| (end < kept.len()).then_some(end)) {
        out.insert("next_offset".to_owned(), json!(next));
    }
    if app.results_truncated {
        out.insert(
            "note".to_owned(),
            json!(format!(
                "the app keeps the first {} of them to show; run_query reads any of them",
                kept.len()
            )),
        );
    }
    if app.results_item_errors > 0 {
        out.insert("item_errors".to_owned(), json!(app.results_item_errors));
        out.insert("last_item_error".to_owned(), json!(app.last_item_error));
    }
    if let Some(elapsed) = app.last_query_elapsed {
        out.insert("elapsed_ms".to_owned(), json!(elapsed.as_millis() as u64));
    }
    Value::Object(out)
}

fn kind_of(kind: ItemKind) -> (&'static str, Option<&'static str>) {
    match kind {
        ItemKind::Query(engine) => ("query", Some(language_key(engine))),
        ItemKind::Schema => ("schema", None),
        ItemKind::Patch => ("patch", None),
        ItemKind::Document => ("document", None),
        ItemKind::Url => ("web address", None),
    }
}

fn item_of(item: &Item) -> Value {
    let (kind, language) = kind_of(item.kind);
    let mut out = json!({
        "path": item.path,
        "name": item.name,
        "kind": kind,
        "bytes": item.bytes,
    });
    if let Some(language) = language {
        out["language"] = json!(language);
    }
    if let Some(sample) = &item.sample {
        out["sample_bytes"] = json!(sample.bytes);
    }
    out
}

fn list_folder(folder: &Folder, items: &mut Vec<Value>, folders: &mut Vec<String>) {
    if !folder.path.is_empty() {
        folders.push(folder.path.clone());
    }
    items.extend(folder.items.iter().map(item_of));
    for sub in &folder.folders {
        list_folder(sub, items, folders);
    }
}

/// What a file of the workspace holds: as JSON if it is, as text if not; or, if it is
/// more than `limit` bytes, where it is.
fn file_content(ws: &Workspace, rel: &str, bytes: u64, limit: u64) -> Result<Value, String> {
    if bytes > limit {
        return Ok(json!({
            "$omitted": format!("{bytes} bytes, more than max_bytes"),
            "file": ws.abs(rel).display().to_string(),
        }));
    }
    let text = ws.read_text(rel, limit)?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}
