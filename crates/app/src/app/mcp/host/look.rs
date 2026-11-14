//! What looks at a document out of sight, for the assistant — any document: the one
//! open in the app, the results the user sees, a file or a JSON item of the workspace
//! (loaded for the assistant alone: the window keeps its own), JSON given in the call,
//! or what the assistant's last `run_query` or tool of the Tools gave ("last", kept
//! here) — with nothing of it shown in the window. And `document_info`: facts about a
//! document without its data, for one too big to read.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use jsonquery_core::{Document, DocumentSource, Root, ValueView};
use jsonquery_mcp::shape::{self, pointer_of, resolve_pointer};
use jsonquery_mcp::snippet::{self, Cuts, Of};
use jsonquery_mcp::Input;
use serde_json::{json, Map, Value};

use super::{locked, AppHost};

/// How long a file may take to load for a tool that has no `timeout_ms`.
pub(super) const LOAD_TIME: Duration = Duration::from_secs(60);

/// How long `document_info` may count, once the document is there.
const COUNT_TIME: Duration = Duration::from_secs(10);

/// The most of its results `run_query` keeps as "last", in bytes of JSON.
pub(super) const KEEP_BYTES: usize = 16 * 1024 * 1024;

/// How many members of an object `document_info` names, the biggest first.
const MEMBERS: usize = 10;

const NOTHING_KEPT: &str =
    "nothing is kept as \"last\" yet: run_query, and the tools of the Tools that make a value \
     (apply_patch, merge_documents, create_schema, diff_documents with as_patch), keep it";

/// What the assistant's last `run_query`, or tool of the Tools, gave.
pub(super) struct Kept {
    value: Arc<Value>,
    /// What it is: "the results of run_query \".a\"".
    what: String,
}

/// A document a tool looks at.
pub(super) enum Viewed {
    /// One the app holds: the open one, or one loaded from a file for the assistant.
    Doc(Arc<Document>),
    Value(Arc<Value>),
}

/// `$body`, with `$root` the root of the [`Viewed`] `$viewed`: a document's (a
/// [`Root`]) or a value's (a `&Value`), each a `ValueView`.
macro_rules! on_view {
    ($viewed:expr, $root:ident => $body:expr) => {
        match $viewed {
            Viewed::Doc(doc) => {
                let $root = doc.root();
                $body
            }
            Viewed::Value(value) => {
                let $root = &**value;
                $body
            }
        }
    };
}
pub(super) use on_view;

/// How many bytes of JSON a value takes: as it is in its file, for a part of a
/// document kept on disk; written compactly, for one in memory.
pub(super) trait JsonBytes {
    fn json_bytes(&self) -> usize;
}

impl JsonBytes for Root<'_> {
    fn json_bytes(&self) -> usize {
        match self {
            Root::Tree(value) => json_size(value),
            Root::Lazy(node) => node.byte_len(),
        }
    }
}

impl JsonBytes for &Value {
    fn json_bytes(&self) -> usize {
        json_size(self)
    }
}

/// How long `value` is, written compactly, without writing it anywhere.
pub(super) fn json_size(value: &Value) -> usize {
    json_size_upto(value, usize::MAX).unwrap_or(usize::MAX)
}

/// How long `value` is, written compactly, counted no further than `cap` bytes: none if
/// it is longer. For a value held under the app's lock, which the window waits for.
pub(super) fn json_size_upto(value: &Value, cap: usize) -> Option<usize> {
    struct Count(usize, usize);
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            if self.0 > self.1 {
                return Err(io::Error::other("longer"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0, cap);
    serde_json::to_writer(&mut count, value).ok()?;
    Some(count.0)
}

/// `bytes` as a person reads it: "1.2 GB", "340.0 KB", "12 bytes".
fn size_of(bytes: u64) -> String {
    snippet::size_text(bytes)
}

/// What a document (or anything else a tool names) is called in what an answer says:
/// a file by its name, without its folder; anything else as it is.
pub(super) fn short_name(name: &str) -> String {
    let path = std::path::Path::new(name);
    match path.file_name() {
        Some(file) if path.is_absolute() => file.to_string_lossy().into_owned(),
        _ => name.to_owned(),
    }
}

/// The member of a page of a value (`shape::read`) that holds what it gives.
pub(super) fn page_member<V: ValueView>(value: &V) -> &'static str {
    match value.kind() {
        jsonquery_core::ValueKind::Array => "items",
        jsonquery_core::ValueKind::Object => "members",
        _ => "value",
    }
}

/// `time` as RFC 3339, in UTC: "2026-11-11T09:30:00Z".
fn utc(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (year, month, day) = crate::workspace_pane::civil_from_days(secs.div_euclid(86_400));
    let of_day = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60
    )
}

/// What a document is, beside its value: where it is from, how big, how it is held.
fn facts_of(doc: &Document, out: &mut Map<String, Value>) {
    let source = match &doc.source {
        DocumentSource::File(path) => {
            out.insert("file".to_owned(), json!(path.display().to_string()));
            if let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) {
                out.insert("modified".to_owned(), json!(utc(modified)));
            }
            "file"
        }
        DocumentSource::Url(url) => {
            out.insert("url".to_owned(), json!(url));
            "web address"
        }
        DocumentSource::Pasted => "pasted",
        DocumentSource::Merged(files) => {
            out.insert("files".to_owned(), json!(files.len()));
            "merged"
        }
        DocumentSource::Derived { .. } => "made by a tool",
    };
    out.insert("source".to_owned(), json!(source));
    out.insert("bytes".to_owned(), json!(doc.byte_len));
    out.insert("size".to_owned(), json!(size_of(doc.byte_len)));
    out.insert(
        "format".to_owned(),
        json!(if doc.top_level_values > 1 {
            format!(
                "NDJSON: {} values one after the other, taken as one array",
                doc.top_level_values
            )
        } else {
            "JSON".to_owned()
        }),
    );
    if doc.is_lazy() {
        out.insert(
            "kept_on_disk".to_owned(),
            json!("too big to load: read from its file as it is looked at"),
        );
    }
    out.insert(
        "loaded_in_ms".to_owned(),
        json!(doc.parse_time.as_millis() as u64),
    );
    if !doc.duplicate_keys.is_empty() {
        out.insert(
            "repeated_keys".to_owned(),
            json!(format!(
                "{} keys are in an object more than once: the last of each counts",
                doc.duplicate_keys.keys
            )),
        );
    }
}

impl AppHost {
    /// The page the user set for the answers, in bytes.
    pub(super) fn page(&self) -> usize {
        self.with_app(|app| app.mcp.page())
            .unwrap_or(jsonquery_mcp::DEFAULT_PAGE)
    }

    /// The `snippet` of an answer that gives a snippet of `of` (see
    /// `jsonquery_mcp::snippet`), with the call that reads on (`next`).
    pub(super) fn notice(
        &self,
        of: &Of<'_>,
        cuts: &Cuts,
        next: Option<Value>,
        hint: Option<&str>,
    ) -> Value {
        snippet::notice(of, cuts, next, self.page(), hint)
    }

    /// `value`, made by a tool and kept as "last" (`name` says what it is), whole if it
    /// fits in `max_bytes`; if not, a snippet of it, and the notice of the answer's
    /// `member` that holds it, which says to read on in "last".
    pub(super) fn fitted(
        &self,
        value: &Value,
        max_bytes: usize,
        member: &str,
        name: &str,
    ) -> (Value, Option<Value>) {
        let (part, cuts) = snippet::snippet(&value, max_bytes);
        let notice = cuts.map(|cuts| {
            let of = Of {
                member,
                name,
                pointer: "",
                is: shape::summary(&value),
                bytes: Some(json_size(value) as u64),
                at_least: false,
            };
            self.notice(&of, &cuts, cuts.read_on(&json!("last")), None)
        });
        (part, notice)
    }

    /// Keep `value` as "last": what the assistant's `what` gave.
    pub(super) fn keep(&self, value: Value, what: String) {
        *locked(&self.last) = Some(Kept {
            value: Arc::new(value),
            what,
        });
    }

    /// What is kept as "last", and what it is.
    pub(super) fn kept(&self) -> Result<(Arc<Value>, String), String> {
        locked(&self.last)
            .as_ref()
            .map(|kept| (kept.value.clone(), kept.what.clone()))
            .ok_or_else(|| NOTHING_KEPT.to_owned())
    }

    /// The document `input` names, and what to call it. A file (or an item of the
    /// workspace) is loaded for the assistant alone, by `deadline`.
    pub(super) fn viewed(
        &self,
        input: &Input,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<(Viewed, String), String> {
        let loaded = |path: &std::path::Path| {
            let limits = self.with_app(|app| app.settings.limits)?;
            self.load_file(path, limits, cancel, deadline)
        };
        Ok(match input {
            Input::Document => {
                let (doc, _) = self.source()?;
                let name = doc.source.label();
                (Viewed::Doc(doc), name)
            }
            Input::Results => (
                Viewed::Value(Arc::new(self.with_app(|app| app.results.clone())?)),
                "the results".to_owned(),
            ),
            Input::Last => {
                let (value, what) = self.kept()?;
                (Viewed::Value(value), what)
            }
            Input::Json(value) => (
                Viewed::Value(Arc::new(value.clone())),
                "the JSON given".to_owned(),
            ),
            Input::Text(text) => (
                Viewed::Value(Arc::new(
                    serde_json::from_str(text)
                        .map_err(|e| format!("the text given is not JSON: {e}"))?,
                )),
                "the text given".to_owned(),
            ),
            Input::File(path) => {
                let doc = loaded(path)?;
                let name = doc.source.label();
                (Viewed::Doc(doc), name)
            }
            Input::Workspace(rel) => (
                Viewed::Doc(loaded(&self.workspace_json(rel)?)?),
                rel.clone(),
            ),
        })
    }

    /// `document_info`: facts about the value at `pointer` of `input`, without its
    /// data, kept to about `max_bytes`.
    pub(super) fn document_info(
        &self,
        input: &Input,
        pointer: &str,
        max_bytes: usize,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let (viewed, name) = self.viewed(input, cancel, Instant::now() + LOAD_TIME)?;
        let page = self.with_app(|app| app.mcp.page())?;
        let mut out = Map::new();
        out.insert("of".to_owned(), json!(name));
        if let Viewed::Doc(doc) = &viewed {
            facts_of(doc, &mut out);
        }
        let deadline = Instant::now() + COUNT_TIME;
        let stop = || cancel.load(Ordering::Relaxed) || Instant::now() >= deadline;
        let whole_doc = match &viewed {
            Viewed::Doc(doc) if pointer.is_empty() => Some(doc.byte_len),
            _ => None,
        };
        on_view!(&viewed, root => {
            let (value, path) = resolve_pointer(root, pointer)?;
            if !path.is_empty() {
                out.insert("pointer".to_owned(), json!(pointer_of(&path)));
            }
            out.insert("is".to_owned(), json!(shape::summary(&value)));
            let bytes = whole_doc.unwrap_or_else(|| value.json_bytes() as u64);
            if whole_doc.is_none() {
                // A part of a document beside the document's own size; or a value's size.
                let (bytes_key, size_key) = if out.contains_key("bytes") {
                    ("part_bytes", "part_size")
                } else {
                    ("bytes", "size")
                };
                out.insert(bytes_key.to_owned(), json!(bytes));
                out.insert(size_key.to_owned(), json!(size_of(bytes)));
            }
            let tally = shape::tally(value, &path, &stop);
            out.insert("values".to_owned(), tally.counts());
            out.insert("depth".to_owned(), json!(tally.depth));
            if !tally.complete {
                out.insert(
                    "counted".to_owned(),
                    json!(format!(
                        "only the first {} values: there was no time for more (give a pointer \
                         to count a part)",
                        tally.values()
                    )),
                );
            }
            let biggest = |list: &[(String, usize)], what: &str| -> Value {
                list.iter()
                    .map(|(pointer, n)| json!({"pointer": pointer, what: n}))
                    .collect()
            };
            if !tally.longest_arrays.is_empty() {
                out.insert(
                    "longest_arrays".to_owned(),
                    biggest(&tally.longest_arrays, "items"),
                );
            }
            if !tally.widest_objects.is_empty() {
                out.insert(
                    "widest_objects".to_owned(),
                    biggest(&tally.widest_objects, "members"),
                );
            }
            // Where the bulk of it is: its biggest members, or how big its items are.
            match value.kind() {
                jsonquery_core::ValueKind::Object => {
                    let mut members: Vec<(String, usize, String)> = Vec::new();
                    for (segment, child) in value.iter_children() {
                        if stop() {
                            break;
                        }
                        if let Some(jsonquery_core::PathSegment::Key(key)) = segment {
                            members.push((key, child.json_bytes(), shape::summary(&child)));
                        }
                    }
                    members.sort_by_key(|member| std::cmp::Reverse(member.1));
                    members.truncate(MEMBERS);
                    let listed: Vec<Value> = members
                        .into_iter()
                        .map(|(key, bytes, is)| json!({"key": key, "bytes": bytes, "is": is}))
                        .collect();
                    out.insert("biggest_members".to_owned(), Value::Array(listed));
                }
                jsonquery_core::ValueKind::Array if value.child_count() > 0 => {
                    out.insert(
                        "bytes_per_item".to_owned(),
                        json!(bytes / value.child_count() as u64),
                    );
                }
                _ => {}
            }
            let tokens = bytes / 4;
            out.insert(
                "to_read_whole".to_owned(),
                json!({
                    "tokens_about": tokens,
                    "pages": bytes.div_ceil(page as u64),
                    "page_bytes": page,
                }),
            );
            let what = if path.is_empty() {
                short_name(&name)
            } else {
                format!("{:?} in {}", pointer_of(&path), short_name(&name))
            };
            if let Some(large) = snippet::large_hint(&what, bytes, page) {
                out.insert("large".to_owned(), json!(large));
            }
            Ok::<(), String>(())
        })?;
        // The lists go, the longest first, while it is more than a page.
        let mut answer = Value::Object(out);
        for list in ["biggest_members", "widest_objects", "longest_arrays"] {
            while answer.to_string().len() > max_bytes {
                let Some(items) = answer.get_mut(list).and_then(Value::as_array_mut) else {
                    break;
                };
                if items.pop().is_none() {
                    break;
                }
            }
        }
        Ok(answer)
    }
}
