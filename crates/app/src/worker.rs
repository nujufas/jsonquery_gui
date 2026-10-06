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
use jsonquery_core::lazy::{Node, PrettyLimits};
use jsonquery_core::{
    Content, Document, DocumentSource, NodePath, Root, SourceMatches, ValueKind, ValueView,
};
use jsonquery_query::{merge, output, OutputFormat};

use crate::app::human_bytes;
use crate::settings::FileLimits;
use crate::tools::jobs::{self, Job, Outcome};
use crate::tools::Tool;

// The sizes this thread holds the work to (how big a download may be, how much a
// merge takes, how much "Copy to Clipboard" copies of a very large document) are
// the user's limits on files (`settings.rs`), which it starts with and takes new
// ones of from `Command::UseLimits`.

/// Nodes of a merge result shown as the Tools window's preview.
const MERGE_PREVIEW_NODES: usize = 600;

/// What the text view shows of a very large document: as much text as this, and
/// of a string as much as that, whatever the number of nodes it is asked for.
const TEXT_VIEW_BYTES: usize = 4 * 1024 * 1024;
const TEXT_VIEW_STRING_BYTES: usize = 4096;

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

/// What "Copy to Clipboard" (a row's context menu) serializes to text,
/// mirroring `Command::SaveFile`/`Command::SaveResults`: a node of the
/// loaded source document, resolved here rather than on the UI thread so a
/// row copy doesn't need to clone a potentially huge document just to pick
/// one branch out of it, or an already-resolved value (a results row, small
/// enough to have been cloned on the UI thread already, but — like
/// `Command::RenderText` — still serialized off it, since one item can be
/// arbitrarily large).
pub enum CopyTarget {
    Source {
        doc: Arc<Document>,
        node_path: Option<NodePath>,
    },
    Value(serde_json::Value),
}

pub enum Command {
    /// The limits on files the user has set (or put back): what the commands after
    /// this one go by.
    UseLimits(FileLimits),
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
        /// How to write them: as JSON, or — when the query ends in `@csv` or
        /// `@tsv` — as the rows of text it made, one to a line.
        format: OutputFormat,
    },
    /// "Copy to Clipboard" over one row's context menu — serializes
    /// `target` to pretty-printed JSON text (or, for `format`, the rows of
    /// CSV or TSV a results row holds); the UI thread decides whether it's
    /// small enough to copy straight away or worth asking about first
    /// (`Event::CopyReady` carries the text either way).
    CopyNode {
        target: CopyTarget,
        /// The format of the results a `CopyTarget::Value` comes from (the
        /// source document is always JSON).
        format: OutputFormat,
    },
    /// Work out where a results row came from in `doc`, for the results
    /// panel's "Find in Source" row action: `target` is the row's value, and
    /// the row sits at `rel` below the `nth` item of the query's result
    /// stream (both only used to rank candidates).
    FindInSource {
        doc: Arc<Document>,
        target: serde_json::Value,
        nth: usize,
        rel: NodePath,
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
        /// What the results are written as (the source is always JSON); for
        /// CSV and TSV the "nodes" the budget counts are rows.
        format: OutputFormat,
    },
    /// The Tools window's "Merge JSON": read `paths` (in this order) and run
    /// the jq `filter` over them (`jsonquery_query::merge`). `cancel` stops it.
    Merge {
        paths: Vec<PathBuf>,
        filter: String,
        gen: u64,
        cancel: Arc<AtomicBool>,
    },
    /// One of the Tools window's other jobs: format, diff, patch or validate
    /// (see `tools::jobs`). `cancel` stops it.
    Tool {
        tool: Tool,
        job: Job,
        gen: u64,
        cancel: Arc<AtomicBool>,
    },
    /// Write `text` to `path`, as the Tools window's Save does.
    SaveText {
        text: Arc<str>,
        path: PathBuf,
    },
    /// Write a document that is kept as its file to `path`, laid out as the Tools
    /// window's Format was asked to, a piece at a time.
    SaveFormatted {
        streamed: crate::tools::jobs::Streamed,
        path: PathBuf,
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

/// What one input file turned out to be, for the Tools window's file list.
pub struct MergedFile {
    pub bytes: u64,
    /// "array · 120 items", "object · 3 keys", "number" …
    pub shape: String,
}

/// A finished merge: the merged document (ready to open or save), what went in
/// and a bounded text preview of what came out.
pub struct MergeOutcome {
    pub doc: Arc<Document>,
    pub files: Vec<MergedFile>,
    /// How many outputs the filter produced (more than one are put in an array).
    pub outputs: usize,
    pub preview: String,
    pub preview_truncated: bool,
    /// Reading the files plus running the filter.
    pub elapsed: Duration,
}

pub enum Event {
    Loading,
    Loaded(Arc<Document>),
    LoadError(String),
    Saved(PathBuf),
    SaveError(String),
    /// `Command::Merge` finished (`gen` says which one).
    MergeDone {
        gen: u64,
        result: Result<MergeOutcome, String>,
    },
    /// `Command::Tool` finished (`tool` and `gen` say which one).
    ToolDone {
        tool: Tool,
        gen: u64,
        result: Result<Outcome, String>,
    },
    /// `Command::CopyNode` finished serializing — the UI thread still has
    /// to decide whether to copy it straight to the clipboard or hold it
    /// for a size-warning confirmation first (see `CLIPBOARD_WARN_BYTES`).
    CopyReady(String),
    CopyError(String),
    Found {
        gen: u64,
        matches: SourceMatches,
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
pub fn spawn(
    cmd_rx: Receiver<Command>,
    evt_tx: Sender<Event>,
    limits: FileLimits,
    wake: impl Fn() + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut limits = limits;
        for cmd in cmd_rx {
            match cmd {
                Command::UseLimits(new) => limits = new,
                Command::OpenFile(path) => {
                    send(&evt_tx, Event::Loading, &wake);
                    let result = jsonquery_core::load_with(&path, limits.load()).map(Arc::new);
                    send_load_result(&evt_tx, result, &wake);
                }
                Command::OpenText(text) => {
                    send(&evt_tx, Event::Loading, &wake);
                    let result = jsonquery_core::load_text(&text).map(Arc::new);
                    send_load_result(&evt_tx, result, &wake);
                }
                Command::OpenUrl(url) => {
                    send(&evt_tx, Event::Loading, &wake);
                    let result = download(&url, &std::env::temp_dir(), &limits).map(Arc::new);
                    send_load_result(&evt_tx, result, &wake);
                }
                Command::SaveFile {
                    doc,
                    node_path,
                    path,
                } => {
                    let result = match &node_path {
                        Some(np) => match jsonquery_core::resolve(doc.root(), np) {
                            Some(root) => save_root(root, &path),
                            None => Err(anyhow::anyhow!(
                                "that value is no longer part of the document"
                            )),
                        },
                        None => save_root(doc.root(), &path),
                    };
                    match result {
                        Ok(()) => send(&evt_tx, Event::Saved(path), &wake),
                        Err(e) => send(&evt_tx, Event::SaveError(format!("{e:#}")), &wake),
                    }
                }
                Command::SaveResults {
                    results,
                    path,
                    format,
                } => {
                    let result = if format.is_tabular() {
                        save_rows(&results, &path)
                    } else {
                        save_json(&results, &path)
                    };
                    match result {
                        Ok(()) => send(&evt_tx, Event::Saved(path), &wake),
                        Err(e) => send(&evt_tx, Event::SaveError(format!("{e:#}")), &wake),
                    }
                }
                Command::CopyNode { target, format } => {
                    let result = match &target {
                        CopyTarget::Source { doc, node_path } => match node_path {
                            Some(np) => match jsonquery_core::resolve(doc.root(), np) {
                                Some(root) => {
                                    root_text(root, limits.copy()).context("serializing that value")
                                }
                                None => Err(anyhow::anyhow!(
                                    "that value is no longer part of the document"
                                )),
                            },
                            None => root_text(doc.root(), limits.copy())
                                .context("serializing the document"),
                        },
                        CopyTarget::Value(v) if format.is_tabular() => Ok(output::rows_text(v)),
                        CopyTarget::Value(v) => {
                            serde_json::to_string_pretty(v).context("serializing that value")
                        }
                    };
                    match result {
                        Ok(text) => send(&evt_tx, Event::CopyReady(text), &wake),
                        Err(e) => send(&evt_tx, Event::CopyError(format!("{e:#}")), &wake),
                    }
                }
                Command::FindInSource {
                    doc,
                    target,
                    nth,
                    rel,
                    gen,
                } => {
                    let matches = jsonquery_core::locate(doc.root(), &target, nth, &rel);
                    send(&evt_tx, Event::Found { gen, matches }, &wake);
                }
                Command::Search {
                    root,
                    text,
                    regex,
                    gen,
                } => {
                    let found = match &root {
                        SearchRoot::Source(doc) => match doc.root() {
                            Root::Tree(value) => jsonquery_core::search(value, &text, regex),
                            // Looked at where it is in the file, not made into
                            // strings: some ten times quicker over a gigabyte.
                            Root::Lazy(node) => jsonquery_core::lazy::search(node, &text, regex)
                                .and_then(|matches| {
                                    ensure_unchanged(doc.root())?;
                                    Ok(matches)
                                }),
                        },
                        SearchRoot::Results(v) => jsonquery_core::search(v.as_ref(), &text, regex),
                    };
                    match found {
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
                    format,
                } => {
                    let kind = target.kind();
                    let (text, truncated) = match &target {
                        TextTarget::Source(doc) => bounded_text(doc.root(), node_budget),
                        TextTarget::Results(v) if format.is_tabular() => {
                            output::rows_bounded(v, node_budget)
                        }
                        TextTarget::Results(v) => {
                            jsonquery_core::pretty_print_bounded(v, node_budget)
                        }
                    };
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
                Command::Merge {
                    paths,
                    filter,
                    gen,
                    cancel,
                } => {
                    let result = merge_files(&paths, &filter, &cancel, &limits)
                        .map_err(|e| format!("{e:#}"));
                    send(&evt_tx, Event::MergeDone { gen, result }, &wake);
                }
                Command::Tool {
                    tool,
                    job,
                    gen,
                    cancel,
                } => {
                    let result = jobs::run(job, &cancel, &limits);
                    send(&evt_tx, Event::ToolDone { tool, gen, result }, &wake);
                }
                Command::SaveFormatted { streamed, path } => {
                    match save_formatted(&streamed, &path) {
                        Ok(()) => send(&evt_tx, Event::Saved(path), &wake),
                        Err(e) => send(&evt_tx, Event::SaveError(format!("{e:#}")), &wake),
                    }
                }
                Command::SaveText { text, path } => {
                    let result = std::fs::write(&path, text.as_bytes())
                        .with_context(|| format!("writing {}", path.display()));
                    match result {
                        Ok(()) => send(&evt_tx, Event::Saved(path), &wake),
                        Err(e) => send(&evt_tx, Event::SaveError(format!("{e:#}")), &wake),
                    }
                }
                Command::Query {
                    doc,
                    text,
                    engine,
                    gen,
                    cancel,
                } => {
                    // A query on a file this big runs threads of its own: whatever
                    // goes wrong in one is an answer for this query, not the end of
                    // this thread, which every later command waits on.
                    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let reply = |event| send(&evt_tx, event, &wake);
                        run_query(&reply, &doc, &text, engine, gen, &cancel, limits.query());
                    }));
                    if ran.is_err() {
                        send(
                            &evt_tx,
                            Event::QueryError {
                                gen,
                                error: "the query stopped because of an internal error".to_owned(),
                            },
                            &wake,
                        );
                    }
                }
            }
        }
    });
}

/// Read `paths` and run the merge `filter` over them. The files are read one
/// after the other with the ordinary loader, so a file of several top-level
/// values (NDJSON) counts as one array, as it does when opened on its own.
fn merge_files(
    paths: &[PathBuf],
    filter: &str,
    cancel: &AtomicBool,
    limits: &FileLimits,
) -> anyhow::Result<MergeOutcome> {
    let start = Instant::now();
    let total = paths
        .iter()
        .map(|p| {
            std::fs::metadata(p)
                .map(|m| m.len())
                .with_context(|| format!("reading {}", p.display()))
        })
        .sum::<anyhow::Result<u64>>()?;
    // A merge happens in memory — each file is parsed, then copied into jq's own
    // values, and the result is built on top — so it is for files that are not
    // very large; past this, one at a time (or a tool made for it) is the way.
    if total > limits.tools() {
        anyhow::bail!(
            "these files add up to {}, over the {} that can be merged — a merge happens in \
             memory. Open them one at a time instead.",
            human_bytes(total),
            human_bytes(limits.tools())
        );
    }

    let mut inputs = Vec::with_capacity(paths.len());
    let mut names = Vec::with_capacity(paths.len());
    let mut files = Vec::with_capacity(paths.len());
    let mut read_time = Duration::ZERO;
    for path in paths {
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("cancelled");
        }
        // Parsed, whatever is allowed to be merged, rather than kept on disk from
        // a lower size than that.
        let doc = jsonquery_core::load_with(path, limits.load_for_tools())
            .with_context(|| path.display().to_string())?;
        read_time += doc.parse_time;
        files.push(MergedFile {
            bytes: doc.byte_len,
            shape: describe(doc.root()),
        });
        names.push(path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ));
        // Only the value is kept. A file too big to be one (the sizes above
        // were of what the file systems said, which is nothing for a pipe) is
        // not merged.
        let byte_len = doc.byte_len;
        let Content::Tree(root) = doc.content else {
            anyhow::bail!(
                "{} is {}, too big to merge — a merge happens in memory",
                path.display(),
                human_bytes(byte_len)
            );
        };
        inputs.push(root);
    }

    let merged = merge::merge(inputs, &names, filter, cancel)?;
    let (preview, preview_truncated) =
        jsonquery_core::pretty_print_bounded(&merged.value, MERGE_PREVIEW_NODES);
    let doc = Document::from_value(
        merged.value,
        DocumentSource::Merged(paths.to_vec()),
        total,
        read_time,
    );
    Ok(MergeOutcome {
        doc: Arc::new(doc),
        files,
        outputs: merged.outputs,
        preview,
        preview_truncated,
        elapsed: start.elapsed(),
    })
}

/// A few words on what `root` is, for the merge file list.
pub(crate) fn describe<V: ValueView>(root: V) -> String {
    let count =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match root.kind() {
        ValueKind::Array => format!("array · {}", count(root.child_count(), "item", "items")),
        ValueKind::Object => format!("object · {}", count(root.child_count(), "key", "keys")),
        ValueKind::String => "string".to_owned(),
        ValueKind::Number => "number".to_owned(),
        ValueKind::Bool => "boolean".to_owned(),
        ValueKind::Null => "null".to_owned(),
    }
}

/// Download `url`'s body and load it. A body of fewer bytes than the user keeps a
/// file on disk from (`limits`; 256 MB unless they changed it) is held in memory
/// and parsed from there, as pasted text is, and never touches the disk. A larger
/// one is streamed into a temporary file in `spill_dir` instead of being held in
/// memory whole, and then loaded like a file opened from disk: mapped and
/// indexed, not parsed. A body of more than `limits` lets a download hand over is
/// an error.
///
/// The temporary file has no name from the moment it is made, so nothing of it
/// can be left behind whatever happens, and a lazy document that is mapped from
/// it has it for as long as it lives.
fn download(url: &str, spill_dir: &Path, limits: &FileLimits) -> anyhow::Result<Document> {
    use std::io::{Read, Write};

    let spill_at = limits.keep_on_disk();
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("requesting {url}"))?;

    // (The reader fails a read that comes after as many bytes as it is to let by,
    // which is the one that would have found the end of a body that is just that
    // long: one more is what makes "at most" so.)
    let mut body = response
        .body_mut()
        .with_config()
        .limit(limits.download().saturating_add(1))
        .reader();

    let mut head = Vec::new();
    (&mut body)
        .take(spill_at)
        .read_to_end(&mut head)
        .map_err(|e| download_error(e, url, limits.download()))?;
    if (head.len() as u64) < spill_at {
        return jsonquery_core::load_bytes(&head, DocumentSource::Url(url.to_string()))
            .with_context(|| format!("parsing data from {url}"));
    }

    let mut file = create_spill_file(spill_dir, url)?;
    let written = file.write_all(&head);
    drop(head);
    written.context("writing the temporary file")?;
    std::io::copy(&mut body, &mut file).map_err(|e| download_error(e, url, limits.download()))?;

    jsonquery_core::load_open_file(&file, DocumentSource::Url(url.to_string()), spill_at)
        .with_context(|| format!("parsing data from {url}"))
}

/// What went wrong reading a download's body. One that is over the limit on
/// downloads is said in the terms of that setting, not of the reader that
/// stopped it.
fn download_error(error: std::io::Error, url: &str, max: u64) -> anyhow::Error {
    let over = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<ureq::Error>())
        .is_some_and(|inner| matches!(inner, ureq::Error::BodyExceedsLimit(_)));
    if over {
        anyhow::anyhow!(
            "{url} is more than {}, the most that a download or a pipe may hand over \
             (Settings: Largest download or pipe)",
            human_bytes(max)
        )
    } else {
        anyhow::Error::new(error).context(format!("downloading {url}"))
    }
}

/// Make the file for `url`'s body, which has no name once it is made, and which
/// nobody but its owner can read where there are file modes, as what is
/// downloaded may not be for them. It is one that did not exist, so that nothing
/// already there — a link someone left at a name in a shared temp dir, say — is
/// written through.
///
/// Unlinked as soon as it is open (a file that is deleted lives on for whoever
/// has it open, or mapped); on Windows, which cannot do that, made to be
/// deleted when the last handle to it closes, the mapping's included.
fn create_spill_file(dir: &Path, url: &str) -> anyhow::Result<std::fs::File> {
    let path = temp_path_for_url(dir, url);
    // Read as well as written: it is mapped afterwards.
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
        const SHARE_READ_WRITE_DELETE: u32 = 0x7;
        options
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .share_mode(SHARE_READ_WRITE_DELETE);
    }
    let file = options
        .open(&path)
        .with_context(|| format!("creating temporary file {}", path.display()))?;
    #[cfg(unix)]
    std::fs::remove_file(&path)
        .with_context(|| format!("unlinking temporary file {}", path.display()))?;
    Ok(file)
}

/// A path in `dir` derived from `url`'s last path segment, prefixed with the
/// pid and a nanosecond timestamp for uniqueness. The prefix also neutralizes
/// any path-traversal attempt in that segment (e.g. a URL ending in `/..`):
/// whatever it contains becomes one literal filename component, never `/`.
fn temp_path_for_url(dir: &Path, url: &str) -> PathBuf {
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
    dir.join(format!(
        "jsonquery_gui-{}-{nanos}-{file_name}",
        std::process::id()
    ))
}

/// Write a node out as pretty-printed JSON — the document, or one row of its
/// tree — for the "Save…" buttons. A parsed value is serialized; a node of a very
/// large document is written straight from its file, a piece at a time, so that
/// saving a gigabyte takes no memory.
fn save_root(root: Root<'_>, path: &Path) -> anyhow::Result<()> {
    match root {
        Root::Tree(value) => save_json(value, path),
        Root::Lazy(node) => {
            ensure_unchanged(root)?;
            let saved = save_node(node, path);
            // A file that was cut short while it was being copied is not copied:
            // what was written has zeros where the rest of it was.
            if let Err(e) = ensure_unchanged(root) {
                let _ = std::fs::remove_file(path);
                return Err(e);
            }
            saved
        }
    }
}

/// Write a document that is kept as its file out to `path`, laid out as asked,
/// straight from the file, which takes no memory however big it is.
fn save_formatted(streamed: &crate::tools::jobs::Streamed, path: &Path) -> anyhow::Result<()> {
    let root = streamed.doc.root();
    let Root::Lazy(node) = root else {
        anyhow::bail!("that document is in memory");
    };
    ensure_unchanged(root)?;
    let style = crate::tools::jobs::style_of(&streamed.options);
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
    let written = node
        .write_styled(&mut out, style, PrettyLimits::default())
        .and_then(|_| std::io::Write::flush(&mut out))
        .with_context(|| format!("writing {}", path.display()));
    // A file that was cut short while it was being copied is not copied.
    if let Err(e) = ensure_unchanged(root) {
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    written
}

/// An error if the file a document is kept in was cut short while it was open
/// (see [`jsonquery_core::lazy::LazyTree::damaged`]): what is read from it is
/// then not the file's.
fn ensure_unchanged(root: Root<'_>) -> anyhow::Result<()> {
    match root {
        Root::Lazy(node) if node.tree().damaged() => {
            anyhow::bail!(jsonquery_core::lazy::CHANGED_WHILE_OPEN)
        }
        _ => Ok(()),
    }
}

fn save_node(node: Node<'_>, path: &Path) -> anyhow::Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
    node.write_pretty(&mut out, PrettyLimits::default())
        .and_then(|_| std::io::Write::flush(&mut out))
        .with_context(|| format!("writing {}", path.display()))
}

/// A node as the text "Copy to Clipboard" puts on the clipboard.
fn root_text(root: Root<'_>, max_copy: usize) -> anyhow::Result<String> {
    match root {
        Root::Tree(value) => Ok(serde_json::to_string_pretty(value)?),
        Root::Lazy(node) => {
            if node.byte_len() > max_copy {
                anyhow::bail!(
                    "that value is {}, too big to copy: use Save… to write it to a file",
                    human_bytes(node.byte_len() as u64)
                );
            }
            ensure_unchanged(root)?;
            let text = node.to_pretty_string(PrettyLimits::default()).0;
            ensure_unchanged(root)?;
            Ok(text)
        }
    }
}

/// The text view's rendering of a document, cut after `node_budget` nodes. One
/// that is kept as its file is also cut by size, so that a few nodes that are
/// huge cannot make a text that is.
fn bounded_text(root: Root<'_>, node_budget: usize) -> (String, bool) {
    match root {
        Root::Tree(value) => jsonquery_core::pretty_print_bounded(value, node_budget),
        Root::Lazy(node) => node.to_pretty_string(PrettyLimits {
            nodes: node_budget,
            bytes: TEXT_VIEW_BYTES,
            string_bytes: TEXT_VIEW_STRING_BYTES,
        }),
    }
}

/// Write a value out as pretty-printed JSON — used by both "Save…" buttons,
/// for the loaded source document and for the current query results.
fn save_json(value: &serde_json::Value, path: &Path) -> anyhow::Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), value)
        .with_context(|| format!("writing {}", path.display()))
}

/// Write results as the rows of text a query ending in `@csv` or `@tsv` made,
/// one to a line — what "Save…" writes in place of JSON for those queries.
fn save_rows(value: &serde_json::Value, path: &Path) -> anyhow::Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut out = std::io::BufWriter::new(file);
    output::write_rows(value, &mut out)
        .and_then(|()| std::io::Write::flush(&mut out))
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

/// Run `text` over `doc` and tell the UI what came of it through `reply`.
/// `limits` are what a query on a document kept on disk may hold of it.
fn run_query(
    reply: &impl Fn(Event),
    doc: &Document,
    text: &str,
    engine: jsonquery_query::Kind,
    gen: u64,
    cancel: &AtomicBool,
    limits: jsonquery_query::lazy::Limits,
) {
    let start = Instant::now();
    let mut on_event = |event| match event {
        QueryEvent::Item(value) => reply(Event::QueryItem { gen, value }),
        QueryEvent::ItemError(error) => reply(Event::QueryItemError { gen, error }),
    };
    let result = match &doc.content {
        Content::Tree(value) => engine.engine().run(value, text, cancel, &mut on_event),
        // A document too big to be a value is read from its file as the query
        // goes: the part of the query that can be is walked, and jq is given the
        // rest a piece at a time.
        Content::Lazy(tree) => {
            jsonquery_query::lazy::run_with(engine, tree, text, limits, cancel, &mut on_event)
        }
    };

    match result {
        Ok(_count) => reply(Event::QueryDone {
            gen,
            cancelled: cancel.load(Ordering::Relaxed),
            elapsed: start.elapsed(),
        }),
        Err(e) => reply(Event::QueryError {
            gen,
            error: e.to_string(),
        }),
    }
}

fn send(evt_tx: &Sender<Event>, event: Event, wake: &impl Fn()) {
    // The receiver is dropped only when the app is shutting down; a failed
    // send at that point is expected and safe to ignore.
    let _ = evt_tx.send(event);
    wake();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch_dir::{ScratchDir, ScratchFile};
    use crate::settings::Limit;
    use jsonquery_core::PathSegment;
    use serde_json::Value;

    fn temp_dir(name: &str) -> ScratchDir {
        ScratchDir::new("worker", name)
    }

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn merges_files_in_the_order_given_and_describes_each() {
        let dir = temp_dir("order");
        let a = write(&dir, "a.json", "[1, 2]");
        let b = write(&dir, "b.json", r#"{"k": 1}"#);
        let outcome = merge_files(
            &[b.clone(), a.clone()],
            "$files",
            &AtomicBool::new(false),
            &FileLimits::default(),
        )
        .unwrap();
        assert_eq!(
            outcome.doc.tree(),
            Some(&serde_json::json!(["b.json", "a.json"]))
        );
        let shapes: Vec<_> = outcome.files.iter().map(|f| f.shape.as_str()).collect();
        assert_eq!(shapes, ["object · 1 key", "array · 2 items"]);
        assert_eq!(outcome.doc.source.label(), "(merged from 2 files)");
        assert_eq!(outcome.doc.byte_len, 6 + 8);
    }

    #[test]
    fn a_file_that_is_not_json_is_named_in_the_error() {
        let dir = temp_dir("bad");
        let good = write(&dir, "good.json", "[1]");
        let bad = write(&dir, "bad.json", "[1,");
        let err = merge_files(
            &[good, bad.clone()],
            "add",
            &AtomicBool::new(false),
            &FileLimits::default(),
        )
        .err()
        .expect("a truncated file fails");
        let message = format!("{err:#}");
        assert!(message.contains(&bad.display().to_string()), "{message}");
    }

    #[test]
    fn a_missing_file_is_named_in_the_error() {
        let missing = temp_dir("missing").join("nope.json");
        let err = merge_files(
            std::slice::from_ref(&missing),
            "add",
            &AtomicBool::new(false),
            &FileLimits::default(),
        )
        .err()
        .expect("a missing file fails");
        assert!(format!("{err:#}").contains(&missing.display().to_string()));
    }

    #[test]
    fn more_than_the_limit_is_refused_before_reading_anything() {
        let dir = temp_dir("big");
        let path = dir.join("big.json");
        // Sparse: nothing is written, but the file reports its length.
        std::fs::File::create(&path)
            .unwrap()
            .set_len(FileLimits::default().tools() + 1)
            .unwrap();
        let err = merge_files(
            &[path],
            "add",
            &AtomicBool::new(false),
            &FileLimits::default(),
        )
        .err()
        .expect("over the limit");
        let message = format!("{err:#}");
        assert!(
            message.contains("add up to") && message.contains("one at a time"),
            "{message}"
        );
    }

    #[test]
    fn a_filter_error_comes_back_as_text() {
        let dir = temp_dir("filter");
        let a = write(&dir, "a.json", "[1]");
        let b = write(&dir, "b.json", r#"{"k": 1}"#);
        let err = merge_files(
            &[a, b],
            "add",
            &AtomicBool::new(false),
            &FileLimits::default(),
        )
        .err()
        .expect("an array and an object do not add");
        assert!(format!("{err:#}").contains("cannot calculate"));
    }

    const WAIT: Duration = Duration::from_secs(20);

    fn start_worker() -> (Sender<Command>, Receiver<Event>) {
        start_worker_with(FileLimits::default())
    }

    fn start_worker_with(limits: FileLimits) -> (Sender<Command>, Receiver<Event>) {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (evt_tx, evt_rx) = crossbeam_channel::unbounded();
        spawn(cmd_rx, evt_tx, limits, || {});
        (cmd_tx, evt_rx)
    }

    /// Limits that keep a file on disk from `bytes`, and nothing else changed.
    fn keeping_on_disk_from(bytes: u64) -> FileLimits {
        let mut limits = FileLimits::default();
        limits.set(Limit::KeepOnDisk, bytes);
        limits
    }

    #[test]
    fn saved_text_is_written_as_given_and_reported() {
        let dir = temp_dir("save-text");
        let path = dir.join("out.json");
        let (commands, events) = start_worker();
        commands
            .send(Command::SaveText {
                text: Arc::from("[\n  1\n]"),
                path: path.clone(),
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::Saved(saved) => assert_eq!(saved, path),
            _ => panic!("expected the save to be reported"),
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[\n  1\n]");
    }

    #[test]
    fn saving_text_where_it_cannot_go_is_an_error_naming_the_file() {
        let path = temp_dir("save-text-missing")
            .join("no-such-folder")
            .join("out.json");
        let (commands, events) = start_worker();
        commands
            .send(Command::SaveText {
                text: Arc::from("[]"),
                path: path.clone(),
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::SaveError(message) => {
                assert!(message.contains(&path.display().to_string()), "{message}");
            }
            _ => panic!("expected an error"),
        }
    }

    fn save_results_as(
        results: Value,
        file_name: &str,
        format: OutputFormat,
    ) -> (Event, ScratchFile) {
        let path = temp_dir("save-results").into_file(file_name);
        let (commands, events) = start_worker();
        commands
            .send(Command::SaveResults {
                results,
                path: path.to_path_buf(),
                format,
            })
            .unwrap();
        (events.recv_timeout(WAIT).unwrap(), path)
    }

    #[test]
    fn json_results_are_saved_as_pretty_json() {
        let (event, path) = save_results_as(
            serde_json::json!(["a,b", 1]),
            "results.json",
            OutputFormat::Json,
        );
        assert!(matches!(event, Event::Saved(ref saved) if *saved == *path));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[\n  \"a,b\",\n  1\n]"
        );
    }

    #[test]
    fn csv_results_are_saved_as_csv_rows_not_as_json() {
        let (event, path) = save_results_as(
            serde_json::json!(["\"Ada\",36", "\"Linus, L.\",28"]),
            "results.csv",
            OutputFormat::Csv,
        );
        assert!(matches!(event, Event::Saved(ref saved) if *saved == *path));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "\"Ada\",36\n\"Linus, L.\",28\n"
        );
    }

    #[test]
    fn tsv_results_are_saved_as_tsv_rows() {
        let (event, path) = save_results_as(
            serde_json::json!(["name\tage", "Ada\t36", "a\\tb\tc"]),
            "results.tsv",
            OutputFormat::Tsv,
        );
        assert!(matches!(event, Event::Saved(_)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "name\tage\nAda\t36\na\\tb\tc\n"
        );
    }

    #[test]
    fn one_saved_row_is_one_line() {
        // What "Save…" on a single results row sends: the row itself.
        let (event, path) = save_results_as(
            serde_json::json!("\"Ada\",36"),
            "item_0.csv",
            OutputFormat::Csv,
        );
        assert!(matches!(event, Event::Saved(_)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "\"Ada\",36\n");
    }

    #[test]
    fn rows_that_cannot_be_saved_are_an_error_naming_the_file() {
        let path = temp_dir("save-rows-missing")
            .join("no-such-folder")
            .join("out.csv");
        let (commands, events) = start_worker();
        commands
            .send(Command::SaveResults {
                results: serde_json::json!(["a"]),
                path: path.clone(),
                format: OutputFormat::Csv,
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::SaveError(message) => {
                assert!(message.contains(&path.display().to_string()), "{message}");
            }
            _ => panic!("expected an error"),
        }
    }

    fn copy_text(value: Value, format: OutputFormat) -> String {
        let (commands, events) = start_worker();
        commands
            .send(Command::CopyNode {
                target: CopyTarget::Value(value),
                format,
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::CopyReady(text) => text,
            _ => panic!("expected the text to copy"),
        }
    }

    #[test]
    fn a_csv_row_is_copied_as_the_row() {
        let row = serde_json::json!("\"Ada\",36,\"say \"\"hi\"\"\"");
        assert_eq!(
            copy_text(row.clone(), OutputFormat::Csv),
            "\"Ada\",36,\"say \"\"hi\"\"\""
        );
        // The same string from a JSON query is copied as the JSON it is.
        assert_eq!(
            copy_text(row, OutputFormat::Json),
            "\"\\\"Ada\\\",36,\\\"say \\\"\\\"hi\\\"\\\"\\\"\""
        );
    }

    #[test]
    fn a_tsv_row_is_copied_as_the_row() {
        assert_eq!(
            copy_text(serde_json::json!("Ada\t36\ta\\nb"), OutputFormat::Tsv),
            "Ada\t36\ta\\nb"
        );
    }

    #[test]
    fn all_the_rows_are_copied_a_line_each() {
        let rows = serde_json::json!(["\"Ada\",36", "\"Linus\",28"]);
        assert_eq!(
            copy_text(rows.clone(), OutputFormat::Csv),
            "\"Ada\",36\n\"Linus\",28"
        );
        assert_eq!(
            copy_text(rows, OutputFormat::Json),
            "[\n  \"\\\"Ada\\\",36\",\n  \"\\\"Linus\\\",28\"\n]"
        );
    }

    #[test]
    fn the_text_view_shows_rows_as_rows_and_counts_them_against_its_budget() {
        let render = |format, node_budget| {
            let (commands, events) = start_worker();
            commands
                .send(Command::RenderText {
                    target: TextTarget::Results(serde_json::json!(["a,b", "\"c\"", "d"])),
                    node_budget,
                    gen: 3,
                    format,
                })
                .unwrap();
            match events.recv_timeout(WAIT).unwrap() {
                Event::TextRendered {
                    target,
                    gen,
                    text,
                    truncated,
                } => {
                    assert!(target == TextTargetKind::Results && gen == 3);
                    (text, truncated)
                }
                _ => panic!("expected the rendered text"),
            }
        };
        assert_eq!(
            render(OutputFormat::Csv, 100),
            ("a,b\n\"c\"\nd".to_owned(), false)
        );
        assert_eq!(
            render(OutputFormat::Tsv, 2),
            ("a,b\n\"c\"".to_owned(), true)
        );
        let (json, _) = render(OutputFormat::Json, 100);
        assert!(json.starts_with("[\n"), "{json}");
        assert!(json.contains("\"a,b\""), "{json}");
    }

    #[test]
    fn a_tool_job_is_answered_under_its_tool_and_generation() {
        let (commands, events) = start_worker();
        commands
            .send(Command::Tool {
                tool: Tool::Format,
                job: Job::Format {
                    input: jobs::Input::Text("[1]".to_owned()),
                    options: jsonquery_query::reformat::Options::default(),
                },
                gen: 7,
                cancel: Arc::new(AtomicBool::new(false)),
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::ToolDone { tool, gen, result } => {
                assert_eq!((tool, gen), (Tool::Format, 7));
                match result {
                    Ok(Outcome::Format(formatted)) => assert_eq!(&*formatted.text, "[\n  1\n]"),
                    _ => panic!("expected the formatted text"),
                }
            }
            _ => panic!("expected the job's answer"),
        }
    }

    #[test]
    fn a_tool_job_that_was_cancelled_says_so() {
        let (commands, events) = start_worker();
        commands
            .send(Command::Tool {
                tool: Tool::Diff,
                job: Job::Diff {
                    left: jobs::Input::Text("[1]".to_owned()),
                    right: jobs::Input::Text("[2]".to_owned()),
                    take: None,
                },
                gen: 1,
                cancel: Arc::new(AtomicBool::new(true)),
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::ToolDone { tool, result, .. } => {
                assert_eq!(tool, Tool::Diff);
                assert!(matches!(result, Err(e) if e == "cancelled"));
            }
            _ => panic!("expected the job's answer"),
        }
    }

    /// Serve `body` once, over HTTP, from a port of its own; the URL to fetch it
    /// from.
    fn serve(body: Vec<u8>) -> String {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            // The request, up to the blank line that ends its headers.
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&chunk[..n]),
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream
                .write_all(head.as_bytes())
                .and_then(|()| stream.write_all(&body));
        });
        format!("http://127.0.0.1:{port}/data.json")
    }

    /// `body`: `[1, 2, 3]`, then blanks up to `len` bytes.
    fn padded(len: usize) -> Vec<u8> {
        let mut body = b"[1, 2, 3]".to_vec();
        body.resize(len, b' ');
        body
    }

    fn files_in(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().count()
    }

    /// The value a document stands for, whichever way it is kept.
    fn value_of(doc: &Document) -> serde_json::Value {
        match &doc.content {
            Content::Tree(value) => value.clone(),
            Content::Lazy(tree) => tree.root().to_value(usize::MAX).unwrap(),
        }
    }

    #[test]
    fn a_small_download_is_parsed_from_memory_and_never_touches_the_disk() {
        let url = serve(br#"{"n": [1, 2, 3]}"#.to_vec());
        // A folder that is not there: a temporary file could not be made in it.
        let nowhere = temp_dir("download-small").join("not-there");
        let doc = download(&url, &nowhere, &keeping_on_disk_from(1024)).unwrap();
        assert_eq!(doc.tree(), Some(&serde_json::json!({"n": [1, 2, 3]})));
        assert_eq!(doc.byte_len, 16);
        assert_eq!(doc.source.label(), url);
        assert!(!doc.is_lazy());
    }

    #[test]
    fn an_empty_download_is_an_empty_array() {
        let url = serve(Vec::new());
        let doc = download(
            &url,
            &temp_dir("download-empty"),
            &keeping_on_disk_from(1024),
        )
        .unwrap();
        assert_eq!(doc.tree(), Some(&serde_json::json!([])));
        assert_eq!((doc.byte_len, doc.top_level_values), (0, 0));
    }

    #[test]
    fn a_large_download_is_kept_in_a_file_that_was_never_there_to_see() {
        let dir = temp_dir("download-large");
        let url = serve(padded(5000));
        let doc = download(&url, &dir, &keeping_on_disk_from(1024)).unwrap();
        // Mapped, and indexed rather than parsed, from a file nobody can open.
        assert!(doc.is_lazy() && doc.lazy().unwrap().is_mapped());
        assert_eq!(value_of(&doc), serde_json::json!([1, 2, 3]));
        assert_eq!(doc.byte_len, 5000);
        assert_eq!(doc.source.label(), url);
        // Unlinked while it is open, where a file that is deleted lives on for
        // whoever has it open; Windows deletes it when the last handle to it, the
        // mapping's, is closed.
        #[cfg(unix)]
        assert_eq!(files_in(&dir), 0, "no file, though the document is open");
        // And the mapping is good for as long as the document is.
        assert_eq!(value_of(&doc), serde_json::json!([1, 2, 3]));
        drop(doc);
        assert_eq!(files_in(&dir), 0, "nothing is left once it is closed");
    }

    #[test]
    fn a_large_download_needs_somewhere_to_go() {
        let nowhere = temp_dir("download-nowhere").join("not-there");
        let url = serve(padded(5000));
        let err = download(&url, &nowhere, &keeping_on_disk_from(1024))
            .err()
            .expect("no folder");
        assert!(
            format!("{err:#}").contains("creating temporary file"),
            "{err:#}"
        );
    }

    #[test]
    fn a_large_download_that_is_not_json_leaves_no_file_behind() {
        let dir = temp_dir("download-bad");
        let url = serve(vec![b'x'; 5000]);
        let err = download(&url, &dir, &keeping_on_disk_from(1024))
            .err()
            .expect("not JSON");
        let message = format!("{err:#}");
        assert!(
            message.contains("parsing data from") && message.contains("parsing JSON"),
            "{message}"
        );
        assert_eq!(files_in(&dir), 0);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_a_download_spills_into_is_private_to_the_user_and_has_no_name() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("spill-mode");
        let file = create_spill_file(&dir, "http://example.test/a.json").unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(files_in(&dir), 0, "unlinked as soon as it is open");
        // It can be written, read back and mapped all the same.
        std::io::Write::write_all(&mut &file, b"[1]").unwrap();
        let doc = jsonquery_core::load_open_file(
            &file,
            DocumentSource::Url("http://example.test/a.json".to_owned()),
            1,
        )
        .unwrap();
        assert!(doc.lazy().unwrap().is_mapped());
        assert_eq!(value_of(&doc), serde_json::json!([1]));
    }

    #[test]
    fn the_temporary_file_is_named_after_the_end_of_the_url_inside_the_folder_given() {
        let dir = Path::new("spill");
        for (url, name) in [
            ("http://h.test/a/b/data.json", "data.json"),
            ("http://h.test/data.json?x=1#top", "data.json"),
            ("http://h.test/", "download.json"),
        ] {
            let path = temp_path_for_url(dir, url);
            assert_eq!(path.parent(), Some(dir), "{url}");
            let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(file_name.starts_with("jsonquery_gui-"), "{file_name}");
            assert!(file_name.ends_with(&format!("-{name}")), "{file_name}");
        }
        // A `..` stays a part of one name rather than a step up.
        let path = temp_path_for_url(dir, "http://h.test/..");
        assert_eq!(path.parent(), Some(dir));
    }

    // ---- a document that is kept as its file ------------------------------

    const USERS: &str = r#"{"users": [{"name": "Ada", "tags": ["x", "y"]},
        {"name": "Alan", "tags": []}, {"name": "Cy"}], "n": 3}"#;

    /// A document of `text` that is indexed rather than parsed, as one of 256 MiB
    /// or more is, made from a file of its own. (The file is gone from its folder
    /// when this returns; the document holds it.)
    fn lazy_document(name: &str, text: &str) -> Arc<Document> {
        lazy_document_in(&temp_dir(name), text)
    }

    /// The same, from the file `doc.json` that is made in `dir`.
    fn lazy_document_in(dir: &Path, text: &str) -> Arc<Document> {
        let path = write(dir, "doc.json", text);
        let file = std::fs::File::open(path).unwrap();
        let doc = jsonquery_core::load_open_file(
            &file,
            DocumentSource::Url("http://example.test/doc.json".to_owned()),
            1,
        )
        .unwrap();
        assert!(doc.is_lazy());
        Arc::new(doc)
    }

    fn query_items(doc: &Arc<Document>, text: &str) -> (Vec<Value>, Vec<String>) {
        let (commands, events) = start_worker();
        commands
            .send(Command::Query {
                doc: doc.clone(),
                text: text.to_owned(),
                engine: jsonquery_query::Kind::Jq,
                gen: 1,
                cancel: Arc::new(AtomicBool::new(false)),
            })
            .unwrap();
        let (mut items, mut errors) = (Vec::new(), Vec::new());
        loop {
            match events.recv_timeout(WAIT).unwrap() {
                Event::QueryItem { value, .. } => items.push(value),
                Event::QueryItemError { error, .. } => errors.push(error),
                Event::QueryDone { .. } => return (items, errors),
                Event::QueryError { error, .. } => {
                    errors.push(error);
                    return (items, errors);
                }
                _ => panic!("expected the query's events"),
            }
        }
    }

    #[test]
    fn a_query_over_a_document_kept_as_its_file_gives_what_it_gives_over_a_value() {
        let doc = lazy_document("lazy-query", USERS);
        let (items, errors) = query_items(&doc, ".users[] | select(.tags | length > 0) | .name");
        assert_eq!(items, [serde_json::json!("Ada")]);
        assert!(errors.is_empty(), "{errors:?}");
        let (items, _) = query_items(&doc, ".users | map(.name) | join(\"+\")");
        assert_eq!(items, [serde_json::json!("Ada+Alan+Cy")]);
        let (items, _) = query_items(&doc, ".users | length");
        assert_eq!(items, [serde_json::json!(3)]);
    }

    #[test]
    fn jsonpath_and_jmespath_read_a_document_kept_as_its_file_too() {
        let doc = lazy_document("lazy-jsonpath", USERS);
        for (engine, text, expected) in [
            (
                jsonquery_query::Kind::JsonPath,
                "$.users[?(@.tags[0] == 'x')].name",
                serde_json::json!("Ada"),
            ),
            (
                jsonquery_query::Kind::JmesPath,
                // (An empty list is not true, so Alan is out.)
                "users[?tags].name | length(@)",
                serde_json::json!(1),
            ),
        ] {
            let (commands, events) = start_worker();
            commands
                .send(Command::Query {
                    doc: doc.clone(),
                    text: text.to_owned(),
                    engine,
                    gen: 1,
                    cancel: Arc::new(AtomicBool::new(false)),
                })
                .unwrap();
            match events.recv_timeout(WAIT).unwrap() {
                Event::QueryItem { value, .. } => assert_eq!(value, expected, "{text}"),
                _ => panic!("expected the query's result: {text}"),
            }
        }
    }

    #[test]
    fn a_row_of_a_document_kept_as_its_file_is_saved_as_pretty_json() {
        let doc = lazy_document("lazy-save", USERS);
        let out_dir = temp_dir("lazy-save-out");
        let out = out_dir.join("out.json");
        let (commands, events) = start_worker();
        for (node_path, expected) in [
            (
                Some(vec![
                    PathSegment::Key("users".into()),
                    PathSegment::Index(0),
                ]),
                "{\n  \"name\": \"Ada\",\n  \"tags\": [\n    \"x\",\n    \"y\"\n  ]\n}",
            ),
            (
                None,
                &*serde_json::to_string_pretty(&serde_json::from_str::<Value>(USERS).unwrap())
                    .unwrap(),
            ),
        ] {
            commands
                .send(Command::SaveFile {
                    doc: doc.clone(),
                    node_path,
                    path: out.clone(),
                })
                .unwrap();
            match events.recv_timeout(WAIT).unwrap() {
                Event::Saved(saved) => assert_eq!(saved, out),
                _ => panic!("expected the save to be reported"),
            }
            assert_eq!(std::fs::read_to_string(&out).unwrap(), expected);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn a_document_whose_file_was_cut_short_is_refused_not_shown_as_zeros() {
        let numbers: Vec<String> = (0..40_000).map(|n| n.to_string()).collect();
        let dir = temp_dir("lazy-cut");
        let doc = lazy_document_in(&dir, &format!("[{}]", numbers.join(",")));
        let tree = doc.lazy().unwrap();
        assert!(!tree.damaged());

        // Another program cuts the file short. Reading past the cut, which used to
        // end the whole process, is what finds it out.
        std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("doc.json"))
            .unwrap()
            .set_len(16_384)
            .unwrap();
        assert_eq!(tree.bytes()[tree.len() - 1], 0);
        assert!(tree.damaged());

        // A query says so rather than answering from zeros...
        let (items, errors) = query_items(&doc, "length");
        assert!(items.is_empty(), "{items:?}");
        assert!(errors[0].contains("changed on disk"), "{errors:?}");

        let (commands, events) = start_worker();
        // ... a save writes nothing...
        let out = dir.join("out.json");
        commands
            .send(Command::SaveFile {
                doc: doc.clone(),
                node_path: None,
                path: out.clone(),
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::SaveError(message) => assert!(message.contains("changed on disk"), "{message}"),
            _ => panic!("expected the save to be refused"),
        }
        assert!(!out.exists());

        // ... and neither does a copy, nor a search, put out what is not in the file.
        commands
            .send(Command::CopyNode {
                target: CopyTarget::Source {
                    doc: doc.clone(),
                    node_path: None,
                },
                format: OutputFormat::Json,
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::CopyError(message) => assert!(message.contains("changed on disk"), "{message}"),
            _ => panic!("expected the copy to be refused"),
        }
        commands
            .send(Command::Search {
                root: SearchRoot::Source(doc),
                text: "7".to_owned(),
                regex: false,
                gen: 1,
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::SearchError { error, .. } => {
                assert!(error.contains("changed on disk"), "{error}")
            }
            _ => panic!("expected the search to be refused"),
        }
    }

    #[test]
    fn a_document_kept_as_its_file_is_saved_laid_out_as_format_was_asked_to() {
        use jsonquery_query::reformat::{self, Indent};

        let text = r#"{"a": [1, "é", "\u00e9\n"], "b": {}, "c": [], "d": {"x": 1.50}}"#;
        let doc = lazy_document("lazy-formatted", text);
        let value: Value = serde_json::from_str(text).unwrap();
        let out_dir = temp_dir("lazy-formatted-out");
        let out = out_dir.join("out.json");
        let (commands, events) = start_worker();
        for options in [
            reformat::Options::default(),
            reformat::Options {
                indent: Indent::Minified,
                sort_keys: false,
                ascii_only: true,
            },
            reformat::Options {
                indent: Indent::Tab,
                sort_keys: false,
                ascii_only: false,
            },
        ] {
            commands
                .send(Command::SaveFormatted {
                    streamed: crate::tools::jobs::Streamed {
                        doc: doc.clone(),
                        options,
                    },
                    path: out.clone(),
                })
                .unwrap();
            match events.recv_timeout(WAIT).unwrap() {
                Event::Saved(saved) => assert_eq!(saved, out),
                _ => panic!("expected the save to be reported"),
            }
            assert_eq!(
                std::fs::read_to_string(&out).unwrap(),
                reformat::render(&value, &options),
                "{options:?}"
            );
        }
    }

    #[test]
    fn a_row_of_a_document_kept_as_its_file_is_copied_as_pretty_json() {
        let doc = lazy_document("lazy-copy", USERS);
        let (commands, events) = start_worker();
        commands
            .send(Command::CopyNode {
                target: CopyTarget::Source {
                    doc,
                    node_path: Some(vec![PathSegment::Key("n".into())]),
                },
                format: OutputFormat::Json,
            })
            .unwrap();
        match events.recv_timeout(WAIT).unwrap() {
            Event::CopyReady(text) => assert_eq!(text, "3"),
            _ => panic!("expected the copied text"),
        }
    }

    // ---- the limits on files the user set -----------------------------------

    /// The default limits with these set.
    fn limits_with(set: &[(Limit, u64)]) -> FileLimits {
        let mut limits = FileLimits::default();
        for (limit, bytes) in set {
            limits.set(*limit, *bytes);
        }
        limits
    }

    /// What opening `bytes` bytes of JSON in a file makes of it, by this worker.
    fn open_file_in(commands: &Sender<Command>, events: &Receiver<Event>, bytes: usize) -> bool {
        let dir = temp_dir(&format!("open-by-limits-{bytes}"));
        let mut text = b"[1, 2, 3]".to_vec();
        text.resize(bytes, b' ');
        let path = dir.join(format!("{bytes}.json"));
        std::fs::write(&path, text).unwrap();
        commands.send(Command::OpenFile(path)).unwrap();
        loop {
            match events.recv_timeout(WAIT).unwrap() {
                Event::Loading => {}
                Event::Loaded(doc) => return doc.is_lazy(),
                Event::LoadError(e) => panic!("{e}"),
                _ => panic!("expected the file to load"),
            }
        }
    }

    #[test]
    fn a_file_is_opened_by_the_limit_the_worker_started_with() {
        let (commands, events) = start_worker();
        assert!(!open_file_in(&commands, &events, 5000), "parsed by default");

        let (commands, events) = start_worker_with(limits_with(&[(Limit::KeepOnDisk, 4096)]));
        assert!(
            open_file_in(&commands, &events, 5000),
            "kept on disk from 4 KB"
        );
        assert!(!open_file_in(&commands, &events, 4095), "and not under it");
    }

    #[test]
    fn new_limits_are_what_the_commands_after_them_go_by() {
        let (commands, events) = start_worker();
        assert!(!open_file_in(&commands, &events, 5000));
        commands
            .send(Command::UseLimits(limits_with(&[(
                Limit::KeepOnDisk,
                4096,
            )])))
            .unwrap();
        assert!(open_file_in(&commands, &events, 5000));
        commands
            .send(Command::UseLimits(FileLimits::default()))
            .unwrap();
        assert!(!open_file_in(&commands, &events, 5000), "put back");
    }

    #[test]
    fn the_limit_on_a_copy_is_the_one_the_worker_was_given() {
        let doc = lazy_document("lazy-copy-limit", USERS);
        let copy = |commands: &Sender<Command>, events: &Receiver<Event>, path| {
            commands
                .send(Command::CopyNode {
                    target: CopyTarget::Source {
                        doc: doc.clone(),
                        node_path: path,
                    },
                    format: OutputFormat::Json,
                })
                .unwrap();
            match events.recv_timeout(WAIT).unwrap() {
                Event::CopyReady(text) => Ok(text),
                Event::CopyError(message) => Err(message),
                _ => panic!("expected the copy's answer"),
            }
        };
        // 8 bytes: the number 3 goes, the document does not.
        let (commands, events) = start_worker_with(limits_with(&[(Limit::Copy, 8)]));
        assert_eq!(
            copy(&commands, &events, Some(vec![PathSegment::Key("n".into())])),
            Ok("3".to_owned())
        );
        let refused = copy(&commands, &events, None).expect_err("over the limit");
        assert!(
            refused.contains("too big to copy") && refused.contains("Save"),
            "{refused}"
        );
        // Raised, the same document goes.
        commands
            .send(Command::UseLimits(FileLimits::default()))
            .unwrap();
        assert!(copy(&commands, &events, None).unwrap().contains("\"Alan\""));
    }

    #[test]
    fn a_query_on_a_document_kept_as_its_file_holds_itself_to_the_limits() {
        let doc = lazy_document("lazy-query-limits", USERS);
        let run = |limits: FileLimits| {
            let (commands, events) = start_worker_with(limits);
            commands
                .send(Command::Query {
                    doc: doc.clone(),
                    text: ".users".to_owned(),
                    engine: jsonquery_query::Kind::Jq,
                    gen: 1,
                    cancel: Arc::new(AtomicBool::new(false)),
                })
                .unwrap();
            match events.recv_timeout(WAIT).unwrap() {
                Event::QueryItem { value, .. } => Ok(value),
                Event::QueryItemError { error, .. } | Event::QueryError { error, .. } => Err(error),
                Event::QueryDone { .. } => panic!("it made nothing"),
                _ => panic!("expected the query's events"),
            }
        };
        let users = run(FileLimits::default()).expect("a list of 100 bytes is fine");
        assert_eq!(users.as_array().map(Vec::len), Some(3));

        // A list of more than 16 bytes is too big to be parsed whole, and a result
        // of more than 16 is more than this one may make.
        let refused = run(limits_with(&[
            (Limit::QueryParse, 16),
            (Limit::QueryResult, 16),
        ]))
        .expect_err("too big");
        assert!(
            refused.contains("too big to show") && refused.contains("16 B"),
            "{refused}"
        );
    }

    #[test]
    fn a_download_is_held_to_the_limit_on_what_one_may_hand_over() {
        let url = serve(padded(5000));
        let dir = temp_dir("download-limit");
        let refused = download(&url, &dir, &limits_with(&[(Limit::Download, 2048)]))
            .err()
            .expect("over the limit");
        let message = format!("{refused:#}");
        assert!(
            message.contains("is more than 2.0 KB") && message.contains("Settings"),
            "{message}"
        );
        assert_eq!(files_in(&dir), 0, "and no file is left");

        // One byte under the size of the body, it does not go either…
        let url = serve(padded(5000));
        let refused = download(&url, &dir, &limits_with(&[(Limit::Download, 4999)]))
            .err()
            .expect("one byte over");
        assert!(
            format!("{refused:#}").contains("is more than"),
            "{refused:#}"
        );

        // …at the size of the body, it does.
        let url = serve(padded(5000));
        let doc = download(&url, &dir, &limits_with(&[(Limit::Download, 5000)])).unwrap();
        assert_eq!(doc.byte_len, 5000);
    }

    #[test]
    fn a_download_is_kept_on_disk_from_the_size_the_user_chose() {
        // 5000 bytes: parsed where files are kept on disk from 8 KB…
        let url = serve(padded(5000));
        let doc = download(
            &url,
            &temp_dir("download-from-8k"),
            &limits_with(&[(Limit::KeepOnDisk, 8192)]),
        )
        .unwrap();
        assert!(!doc.is_lazy());
        // …and kept on disk where they are from 4 KB.
        let url = serve(padded(5000));
        let doc = download(
            &url,
            &temp_dir("download-from-4k"),
            &limits_with(&[(Limit::KeepOnDisk, 4096)]),
        )
        .unwrap();
        assert!(doc.is_lazy());
    }

    #[test]
    fn a_merge_takes_what_the_user_lets_the_tools_take_even_from_the_size_files_are_kept_on_disk_from(
    ) {
        let dir = temp_dir("merge-limits");
        let a = write(&dir, "a.json", &format!("[1{}]", " ".repeat(5000)));
        let b = write(&dir, "b.json", "[2]");
        let cancel = AtomicBool::new(false);

        // Over what the tools take: refused before a file is read.
        let small = limits_with(&[(Limit::Tools, 4096)]);
        let refused = merge_files(&[a.clone(), b.clone()], "add", &cancel, &small)
            .err()
            .expect("over the limit");
        let message = format!("{refused:#}");
        assert!(
            message.contains("add up to") && message.contains("over the 4.0 KB"),
            "{message}"
        );

        // Under it, though files are kept on disk from a lower size than that,
        // it is parsed and merged.
        let wide = limits_with(&[(Limit::Tools, 64 * 1024), (Limit::KeepOnDisk, 1024)]);
        let merged = merge_files(&[a, b], "add", &cancel, &wide).unwrap();
        assert_eq!(merged.doc.tree(), Some(&serde_json::json!([1, 2])));
    }

    #[test]
    fn a_tool_job_takes_what_the_user_lets_the_tools_take() {
        let (commands, events) = start_worker_with(limits_with(&[(Limit::Tools, 1024)]));
        let job = |text: &str| Job::Format {
            input: crate::tools::jobs::Input::Text(text.to_owned()),
            options: jsonquery_query::reformat::Options::default(),
        };
        let run = |commands: &Sender<Command>, text: &str| {
            commands
                .send(Command::Tool {
                    tool: Tool::Format,
                    job: job(text),
                    gen: 1,
                    cancel: Arc::new(AtomicBool::new(false)),
                })
                .unwrap();
            match events.recv_timeout(WAIT).unwrap() {
                Event::ToolDone { result, .. } => result.map(|_| ()),
                _ => panic!("expected the job's answer"),
            }
        };
        let long = format!("[1{}]", " ".repeat(2000));
        let refused = run(&commands, &long).expect_err("over 1 KB");
        assert!(refused.contains("over the 1.0 KB"), "{refused}");
        run(&commands, "[1]").unwrap();
        commands
            .send(Command::UseLimits(FileLimits::default()))
            .unwrap();
        run(&commands, &long).unwrap();
    }

    #[test]
    fn a_search_of_a_document_kept_as_its_file_finds_what_one_of_a_value_does() {
        let doc = lazy_document("lazy-search", USERS);
        let parsed: Value = serde_json::from_str(USERS).unwrap();
        let (commands, events) = start_worker();
        for (text, regex) in [
            ("al", false),
            ("^A\\w+$", true),
            ("tags", false),
            ("zzz", false),
        ] {
            commands
                .send(Command::Search {
                    root: SearchRoot::Source(doc.clone()),
                    text: text.to_owned(),
                    regex,
                    gen: 5,
                })
                .unwrap();
            let Event::SearchDone { matches, .. } = events.recv_timeout(WAIT).unwrap() else {
                panic!("expected the search's answer");
            };
            assert_eq!(
                matches,
                jsonquery_core::search(&parsed, text, regex).unwrap(),
                "{text}"
            );
        }
    }

    #[test]
    fn the_text_of_a_document_kept_as_its_file_is_cut_by_nodes_as_that_of_a_value_is() {
        let doc = lazy_document("lazy-text", USERS);
        let parsed: Value = serde_json::from_str(USERS).unwrap();
        let (commands, events) = start_worker();
        for budget in [3, 7, 1000] {
            commands
                .send(Command::RenderText {
                    target: TextTarget::Source(doc.clone()),
                    node_budget: budget,
                    gen: 2,
                    format: OutputFormat::Json,
                })
                .unwrap();
            let Event::TextRendered {
                text, truncated, ..
            } = events.recv_timeout(WAIT).unwrap()
            else {
                panic!("expected the rendered text");
            };
            assert_eq!(
                (text, truncated),
                jsonquery_core::pretty_print_bounded(&parsed, budget),
                "{budget} nodes"
            );
        }
    }

    #[test]
    fn a_result_found_in_a_document_kept_as_its_file_is_located_in_it() {
        let doc = lazy_document("lazy-find", USERS);
        let (commands, events) = start_worker();
        commands
            .send(Command::FindInSource {
                doc,
                target: serde_json::json!("Alan"),
                nth: 1,
                rel: vec![PathSegment::Key("name".into())],
                gen: 9,
            })
            .unwrap();
        let Event::Found { matches, .. } = events.recv_timeout(WAIT).unwrap() else {
            panic!("expected the answer");
        };
        assert_eq!(
            matches.paths,
            [vec![
                PathSegment::Key("users".into()),
                PathSegment::Index(1),
                PathSegment::Key("name".into())
            ]]
        );
    }

    #[test]
    fn a_cancelled_merge_stops_before_reading() {
        let dir = temp_dir("cancel");
        let a = write(&dir, "a.json", "[1]");
        let err = merge_files(&[a], "add", &AtomicBool::new(true), &FileLimits::default())
            .err()
            .expect("cancelled");
        assert_eq!(format!("{err:#}"), "cancelled");
    }
}
