//! The work behind the Format, Diff, Patch and Validate pages. A page collects
//! what it has — documents typed or pasted in, files, the document open in the
//! main window — and its options into a [`Job`]; the worker thread runs it
//! ([`run`]) and answers with an [`Outcome`]. None of it touches the UI.
//!
//! Like Merge, these work in memory and are for documents that are not very
//! large: a job is refused when what it is given adds up to more than
//! [`MAX_TOOL_BYTES`].

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use jsonquery_core::{Content, Document, DocumentSource};
use jsonquery_query::diff::{self, Change, Side, SideBySide, TooLong};
use jsonquery_query::reformat::{self, Indent};
use jsonquery_query::{patch, schema};
use serde_json::Value;

use crate::app::human_bytes;

/// The most a job is given to work on, all its documents together.
pub const MAX_TOOL_BYTES: u64 = 128 * 1024 * 1024;

/// How much of a long text a preview holds: this many lines, and this many
/// bytes, whichever comes first.
const PREVIEW_LINES: usize = 1_500;
const PREVIEW_BYTES: usize = 200_000;

/// Where one of a job's documents comes from.
#[derive(Clone)]
pub enum Input {
    /// JSON typed or pasted into the page.
    Text(String),
    /// A file, read when the job runs — for one too big to show on the page.
    File(PathBuf),
    /// The document open in the main window, as it was when it was picked.
    Document(Arc<Document>),
}

impl Input {
    /// The document, if this is one that is kept as its file (see
    /// `jsonquery_core::LAZY_THRESHOLD`): the open one, or a file that is that big.
    fn kept_on_disk(&self) -> anyhow::Result<Option<Arc<Document>>> {
        match self {
            Input::Text(_) => Ok(None),
            Input::Document(doc) => Ok(doc.is_lazy().then(|| doc.clone())),
            Input::File(path) => {
                if self.size()? < jsonquery_core::LAZY_THRESHOLD {
                    return Ok(None);
                }
                let doc = jsonquery_core::load(path).with_context(|| path.display().to_string())?;
                Ok(doc.is_lazy().then(|| Arc::new(doc)))
            }
        }
    }

    fn size(&self) -> anyhow::Result<u64> {
        Ok(match self {
            Input::Text(text) => text.len() as u64,
            Input::File(path) => std::fs::metadata(path)
                .with_context(|| format!("reading {}", path.display()))?
                .len(),
            Input::Document(doc) => doc.byte_len,
        })
    }

    fn load(&self) -> anyhow::Result<Loaded> {
        match self {
            Input::Text(text) => {
                let doc = jsonquery_core::load_text(text)?;
                let (root, read) = in_memory(doc)?;
                Ok(Loaded {
                    held: Held::Owned(root),
                    bytes: text.len() as u64,
                    read,
                })
            }
            Input::File(path) => {
                let doc = jsonquery_core::load(path).with_context(|| path.display().to_string())?;
                let bytes = doc.byte_len;
                let (root, read) = in_memory(doc).with_context(|| path.display().to_string())?;
                Ok(Loaded {
                    held: Held::Owned(root),
                    bytes,
                    read,
                })
            }
            Input::Document(doc) => {
                if doc.is_lazy() {
                    return Err(too_big_to_work_on(doc.byte_len));
                }
                Ok(Loaded {
                    held: Held::Shared(doc.clone()),
                    bytes: doc.byte_len,
                    read: Duration::ZERO,
                })
            }
        }
    }
}

/// The value of a document that was loaded for a job, which has to be the
/// whole of it in memory: one too big for that (it is kept as its file, read as
/// it is looked at) is not something these can work on.
fn in_memory(doc: Document) -> anyhow::Result<(Value, Duration)> {
    let byte_len = doc.byte_len;
    match doc.content {
        Content::Tree(root) => Ok((root, doc.parse_time)),
        Content::Lazy(_) => Err(too_big_to_work_on(byte_len)),
    }
}

fn too_big_to_work_on(bytes: u64) -> anyhow::Error {
    anyhow::anyhow!(
        "that is {}, too big for this: it works on the whole document in memory",
        human_bytes(bytes)
    )
}

/// A document in memory: parsed for this job, or shared with the main window
/// (which is not copied just to be looked at).
enum Held {
    Owned(Value),
    Shared(Arc<Document>),
}

/// What a shared document is held as if it should turn out to have no tree,
/// which `Input::load` does not let it.
static NOTHING: Value = Value::Null;

impl Held {
    fn value(&self) -> &Value {
        match self {
            Held::Owned(value) => value,
            Held::Shared(doc) => doc.tree().unwrap_or(&NOTHING),
        }
    }

    fn into_owned(self) -> Value {
        match self {
            Held::Owned(value) => value,
            Held::Shared(doc) => doc.tree().cloned().unwrap_or(Value::Null),
        }
    }
}

struct Loaded {
    held: Held,
    bytes: u64,
    /// Reading and parsing it.
    read: Duration,
}

pub enum Job {
    Format {
        input: Input,
        options: reformat::Options,
    },
    /// The patch it makes turns the left document into the right one. With
    /// `take`, some differences are first moved from one document into the
    /// other, and what is left of them is what is compared.
    Diff {
        left: Input,
        right: Input,
        take: Option<Take>,
    },
    Patch {
        document: Input,
        patch: Input,
        /// A JSON Merge Patch (RFC 7386) rather than a list of operations
        /// (RFC 6902).
        merge_patch: bool,
    },
    Validate {
        document: Input,
        schema: Input,
        check_formats: bool,
    },
}

/// Differences to move from one document into the other before comparing: what
/// moving a difference to the left or to the right in the side-by-side view asks
/// for.
#[derive(Clone, Debug)]
pub struct Take {
    /// The numbers of the differences, as the rows of the view carry them (see
    /// `diff::take_changes`).
    pub changes: Range<u32>,
    /// The document that takes them in.
    pub into: Side,
}

pub enum Outcome {
    Format(Formatted),
    Diff(Compared),
    Patch(Patched),
    Validate(Validated),
}

/// The first part of a text, for a box that cannot show all of it.
pub struct Preview {
    pub text: String,
    /// There is more than this.
    pub truncated: bool,
}

impl Preview {
    fn of(text: &str) -> Self {
        let by_lines = text
            .match_indices('\n')
            .nth(PREVIEW_LINES - 1)
            .map_or(text.len(), |(at, _)| at);
        let mut end = by_lines;
        if end > PREVIEW_BYTES {
            end = PREVIEW_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
        }
        Self {
            text: text[..end].to_owned(),
            truncated: end < text.len(),
        }
    }
}

pub struct Formatted {
    /// All of the formatted text, for Copy and Save. (Empty for a document that
    /// is kept on disk, which is written out from there: see `streamed`.)
    pub text: Arc<str>,
    pub preview: Preview,
    /// The size of what went in.
    pub bytes_in: u64,
    pub elapsed: Duration,
    /// For a document too big to be text in memory: what Save writes it from.
    pub streamed: Option<Streamed>,
}

/// A document that is formatted as it is saved, a piece at a time from its file.
#[derive(Clone)]
pub struct Streamed {
    pub doc: Arc<Document>,
    pub options: reformat::Options,
}

pub struct Compared {
    /// The differences in document order — at most `diff::MAX_CHANGES`, though
    /// the counts below are of all of them.
    pub changes: Vec<Change>,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    /// The documents are the same.
    pub equal: bool,
    /// The RFC 6902 patch that turns the first into the second, one operation
    /// to a line.
    pub patch: Arc<str>,
    pub patch_preview: Preview,
    /// The two documents line by line, with what differs marked — or that they
    /// are too long to lay out.
    pub view: Result<SideBySide, TooLong>,
    /// The document a move changed, to put back in its box.
    pub moved: Option<Moved>,
    pub elapsed: Duration,
}

/// A document that a move changed, written out.
pub struct Moved {
    pub into: Side,
    pub text: Arc<str>,
}

impl Compared {
    pub fn total(&self) -> usize {
        self.added + self.removed + self.changed
    }
}

pub struct Patched {
    /// The patched document, ready to open in the main window.
    pub doc: Arc<Document>,
    pub text: Arc<str>,
    pub preview: Preview,
    /// How many operations a JSON Patch had; none for a merge patch.
    pub operations: Option<usize>,
    pub elapsed: Duration,
}

pub struct Validated {
    pub report: schema::Report,
    /// The document that was checked, when it was the one open in the main
    /// window — which is what a problem's path can be shown in.
    pub open: Option<Arc<Document>>,
    pub elapsed: Duration,
}

/// Run `job`. The error is text for the page to show; `"cancelled"` when
/// `cancel` was set.
pub fn run(job: Job, cancel: &AtomicBool) -> Result<Outcome, String> {
    run_job(job, cancel).map_err(|e| format!("{e:#}"))
}

fn run_job(job: Job, cancel: &AtomicBool) -> anyhow::Result<Outcome> {
    let start = Instant::now();
    match job {
        Job::Format { input, options } => {
            // A document too big to be text in memory is written from its file when it
            // is saved; what is made now is the beginning of it.
            if let Some(doc) = input.kept_on_disk().context("Input")? {
                return format_from_disk(doc, options, start, cancel);
            }
            check_size(&[&input])?;
            let loaded = input.load().context("Input")?;
            stop_if_cancelled(cancel)?;
            let text = reformat::render(loaded.held.value(), &options);
            Ok(Outcome::Format(Formatted {
                preview: Preview::of(&text),
                text: text.into(),
                bytes_in: loaded.bytes,
                elapsed: start.elapsed(),
                streamed: None,
            }))
        }
        Job::Diff { left, right, take } => {
            check_size(&[&left, &right])?;
            let first = left.load().context("Left")?;
            let second = right.load().context("Right")?;
            stop_if_cancelled(cancel)?;
            let merged: Value;
            let (mut before, mut after) = (first.held.value(), second.held.value());
            let mut moved = None;
            if let Some(take) = take {
                merged = diff::take_changes(before, after, take.changes, take.into, cancel)
                    .map_err(|_| anyhow::anyhow!("cancelled"))?;
                let text = reformat::render(&merged, &reformat::Options::default());
                let kept = match take.into {
                    Side::Left => second.bytes,
                    Side::Right => first.bytes,
                };
                if text.len() as u64 + kept > MAX_TOOL_BYTES {
                    bail!(
                        "the {} document would be {}, which with the other is over the {} \
                         these tools can take — they work in memory",
                        match take.into {
                            Side::Left => "Left",
                            Side::Right => "Right",
                        },
                        human_bytes(text.len() as u64),
                        human_bytes(MAX_TOOL_BYTES)
                    );
                }
                match take.into {
                    Side::Left => before = &merged,
                    Side::Right => after = &merged,
                }
                moved = Some(Moved {
                    into: take.into,
                    text: text.into(),
                });
            }
            let compared =
                diff::compare(before, after, cancel).map_err(|_| anyhow::anyhow!("cancelled"))?;
            let found = compared.diff;
            let patch = operations_text(&found.operations);
            Ok(Outcome::Diff(Compared {
                equal: found.is_empty(),
                added: found.added,
                removed: found.removed,
                changed: found.changed,
                changes: found.changes,
                patch_preview: Preview::of(&patch),
                patch: patch.into(),
                view: compared.view,
                moved,
                elapsed: start.elapsed(),
            }))
        }
        Job::Patch {
            document,
            patch: patch_input,
            merge_patch,
        } => {
            check_size(&[&document, &patch_input])?;
            let loaded = document.load().context("Document")?;
            let patch_loaded = patch_input.load().context("Patch")?;
            stop_if_cancelled(cancel)?;
            let read = loaded.read;
            let patch_value = patch_loaded.held.value();
            let (value, operations) = if merge_patch {
                (
                    patch::apply_merge(loaded.held.into_owned(), patch_value),
                    None,
                )
            } else {
                let count = patch_value.as_array().map(Vec::len);
                let patched = patch::apply(loaded.held.into_owned(), patch_value)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                (patched, count)
            };
            let text = reformat::render(&value, &reformat::Options::default());
            let preview = Preview::of(&text);
            let doc = Document::from_value(
                value,
                DocumentSource::Derived {
                    label: "(patched)",
                    file_name: "patched.json",
                },
                text.len() as u64,
                read,
            );
            Ok(Outcome::Patch(Patched {
                doc: Arc::new(doc),
                text: text.into(),
                preview,
                operations,
                elapsed: start.elapsed(),
            }))
        }
        Job::Validate {
            document,
            schema: schema_input,
            check_formats,
        } => {
            check_size(&[&document, &schema_input])?;
            let loaded = document.load().context("Document")?;
            let schema_loaded = schema_input.load().context("Schema")?;
            stop_if_cancelled(cancel)?;
            let report = schema::validate(
                schema_loaded.held.value(),
                loaded.held.value(),
                schema::Options { check_formats },
                cancel,
            )
            .map_err(|e| match e {
                schema::Error::Cancelled => anyhow::anyhow!("cancelled"),
                schema::Error::Schema(message) => {
                    anyhow::anyhow!("The schema can't be used: {message}")
                }
            })?;
            let open = match &document {
                Input::Document(doc) => Some(doc.clone()),
                _ => None,
            };
            Ok(Outcome::Validate(Validated {
                report,
                open,
                elapsed: start.elapsed(),
            }))
        }
    }
}

fn stop_if_cancelled(cancel: &AtomicBool) -> anyhow::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("cancelled");
    }
    Ok(())
}

/// Refuse a job whose documents are too big to hold in memory together.
/// How a document kept on disk is laid out when it is written: the options of
/// Format that can be, a piece at a time.
pub fn style_of(options: &reformat::Options) -> jsonquery_core::lazy::Style {
    use jsonquery_core::lazy::{Indent as Written, Style};
    Style {
        indent: match options.indent {
            Indent::Spaces(n) => Written::Spaces(n),
            Indent::Tab => Written::Tab,
            Indent::Minified => Written::Minified,
        },
        ascii_only: options.ascii_only,
    }
}

/// Format for a document that is kept as its file: a preview of the beginning,
/// and what Save writes it from.
fn format_from_disk(
    doc: Arc<Document>,
    options: reformat::Options,
    start: Instant,
    cancel: &AtomicBool,
) -> anyhow::Result<Outcome> {
    if options.sort_keys {
        bail!(
            "that is {}, too big to have its keys sorted: that needs the whole document in \
             memory. It can be indented or minified, as it is written",
            human_bytes(doc.byte_len)
        );
    }
    stop_if_cancelled(cancel)?;
    let Content::Lazy(tree) = &doc.content else {
        bail!("that document is in memory");
    };
    let mut head = Vec::new();
    let written = tree.root().write_styled(
        &mut head,
        style_of(&options),
        jsonquery_core::lazy::PrettyLimits {
            bytes: PREVIEW_BYTES * 2,
            ..Default::default()
        },
    )?;
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut preview = Preview::of(&text);
    preview.truncated |= !written.complete;
    Ok(Outcome::Format(Formatted {
        text: Arc::from(""),
        preview,
        bytes_in: doc.byte_len,
        elapsed: start.elapsed(),
        streamed: Some(Streamed { doc, options }),
    }))
}

fn check_size(inputs: &[&Input]) -> anyhow::Result<()> {
    let mut total = 0u64;
    for input in inputs {
        total += input.size()?;
    }
    if total > MAX_TOOL_BYTES {
        bail!(
            "that is {}, over the {} these tools can take — they work in memory",
            human_bytes(total),
            human_bytes(MAX_TOOL_BYTES)
        );
    }
    Ok(())
}

/// A JSON Patch as text, one operation to a line:
///
/// ```text
/// [
///   {"op": "replace", "path": "/a", "value": 2},
///   {"op": "remove", "path": "/b"}
/// ]
/// ```
///
/// Each operation has a space after its colons and commas — which makes a long
/// one something a window can wrap — and its value, if it has one, in full.
fn operations_text(operations: &[Value]) -> String {
    if operations.is_empty() {
        return "[]".to_owned();
    }
    let lines: Vec<String> = operations
        .iter()
        .map(|op| format!("  {}", operation_text(op)))
        .collect();
    format!("[\n{}\n]", lines.join(",\n"))
}

fn operation_text(operation: &Value) -> String {
    let minified = reformat::Options {
        indent: Indent::Minified,
        ..reformat::Options::default()
    };
    let Value::Object(members) = operation else {
        return reformat::render(operation, &minified);
    };
    let members: Vec<String> = members
        .iter()
        .map(|(key, value)| {
            format!(
                "{}: {}",
                reformat::render(&Value::String(key.clone()), &minified),
                reformat::render(value, &minified)
            )
        })
        .collect();
    format!("{{{}}}", members.join(", "))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;

    fn text(s: &str) -> Input {
        Input::Text(s.to_owned())
    }

    fn go(job: Job) -> Result<Outcome, String> {
        run(job, &AtomicBool::new(false))
    }

    fn formatted(job: Job) -> Formatted {
        match go(job) {
            Ok(Outcome::Format(f)) => f,
            Ok(_) => panic!("not a format outcome"),
            Err(e) => panic!("{e}"),
        }
    }

    /// A document that is indexed rather than parsed, as one of 256 MiB or more is.
    fn lazy_document(name: &str) -> Arc<Document> {
        let path = temp_dir(name).join("big.json");
        std::fs::write(&path, "[1, 2, 3]").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let doc = jsonquery_core::load_open_file(&file, DocumentSource::File(path), 1).unwrap();
        assert!(doc.is_lazy());
        Arc::new(doc)
    }

    #[test]
    fn a_document_kept_on_disk_is_too_big_for_a_tool_and_says_so() {
        // Every tool works on the whole document in memory.
        let err = Input::Document(lazy_document("lazy-input"))
            .load()
            .err()
            .expect("refused");
        let message = format!("{err:#}");
        assert!(
            message.contains("too big for this") && message.contains("in memory"),
            "{message}"
        );

        // (Format is the one that is not refused: see below.)
        let err = go(Job::Diff {
            left: Input::Document(lazy_document("lazy-diff")),
            right: text("[1]"),
            take: None,
        })
        .err()
        .expect("refused");
        assert!(err.contains("too big for this"), "{err}");
    }

    #[test]
    fn a_document_kept_on_disk_is_formatted_from_there_as_it_is_saved() {
        let f = formatted(Job::Format {
            input: Input::Document(lazy_document("lazy-format")),
            options: reformat::Options {
                indent: Indent::Spaces(4),
                sort_keys: false,
                ascii_only: false,
            },
        });
        assert_eq!(&*f.preview.text, "[\n    1,\n    2,\n    3\n]");
        assert!(!f.preview.truncated);
        let streamed = f.streamed.expect("written from the file");
        assert!(streamed.doc.is_lazy());
        assert_eq!(f.bytes_in, 9);

        // The keys of what is not in memory cannot be put in order.
        let err = go(Job::Format {
            input: Input::Document(lazy_document("lazy-sorted")),
            options: reformat::Options {
                sort_keys: true,
                ..reformat::Options::default()
            },
        })
        .err()
        .expect("refused");
        assert!(err.contains("keys sorted"), "{err}");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jsonquery-jobs-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn formatting_text() {
        let f = formatted(Job::Format {
            input: text(r#"{"b":[1,2],"a":{}}"#),
            options: reformat::Options {
                indent: Indent::Spaces(2),
                sort_keys: true,
                ascii_only: false,
            },
        });
        assert_eq!(
            &*f.text,
            "{\n  \"a\": {},\n  \"b\": [\n    1,\n    2\n  ]\n}"
        );
        assert_eq!(f.bytes_in, 18);
        assert!(!f.preview.truncated);
        assert_eq!(f.preview.text, &*f.text);
    }

    #[test]
    fn formatting_a_file_and_the_open_document() {
        let dir = temp_dir("format-file");
        let path = write(&dir, "in.json", r#"[1, 2]"#);
        let minified = reformat::Options {
            indent: Indent::Minified,
            ..reformat::Options::default()
        };
        let f = formatted(Job::Format {
            input: Input::File(path),
            options: minified,
        });
        assert_eq!(&*f.text, "[1,2]");

        let doc = Arc::new(jsonquery_core::load_text(r#"{ "k" : [ true ] }"#).unwrap());
        let f = formatted(Job::Format {
            input: Input::Document(doc),
            options: minified,
        });
        assert_eq!(&*f.text, r#"{"k":[true]}"#);
    }

    #[test]
    fn text_that_is_not_json_is_reported_with_where() {
        let Err(error) = go(Job::Format {
            input: text("{\n  \"a\": \n}"),
            options: reformat::Options::default(),
        }) else {
            panic!("should fail")
        };
        assert!(error.starts_with("Input: "), "{error}");
        assert!(error.contains("line 3"), "{error}");
    }

    #[test]
    fn a_missing_file_is_named() {
        let missing = temp_dir("missing").join("nope.json");
        let Err(error) = go(Job::Format {
            input: Input::File(missing.clone()),
            options: reformat::Options::default(),
        }) else {
            panic!("should fail")
        };
        assert!(error.contains(&missing.display().to_string()), "{error}");
    }

    #[test]
    fn too_much_is_refused_before_anything_is_read() {
        let dir = temp_dir("big");
        let path = dir.join("big.json");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_TOOL_BYTES + 1)
            .unwrap();
        let Err(error) = go(Job::Format {
            input: Input::File(path),
            options: reformat::Options::default(),
        }) else {
            panic!("should fail")
        };
        assert!(error.contains("work in memory"), "{error}");
    }

    #[test]
    fn the_documents_of_one_job_are_counted_together() {
        let dir = temp_dir("together");
        let (a, b) = (dir.join("a.json"), dir.join("b.json"));
        for path in [&a, &b] {
            // Each alone is fine; the two of them are over.
            std::fs::File::create(path)
                .unwrap()
                .set_len(MAX_TOOL_BYTES / 2 + 1)
                .unwrap();
        }
        let Err(error) = go(Job::Diff {
            left: Input::File(a),
            right: Input::File(b),
            take: None,
        }) else {
            panic!("should be refused")
        };
        assert!(error.contains("work in memory"), "{error}");
    }

    #[test]
    fn an_open_document_counts_at_its_size_on_disk() {
        let big = Arc::new(Document::from_value(
            json!([1]),
            DocumentSource::Pasted,
            MAX_TOOL_BYTES + 1,
            Duration::ZERO,
        ));
        let Err(error) = go(Job::Format {
            input: Input::Document(big),
            options: reformat::Options::default(),
        }) else {
            panic!("should be refused")
        };
        assert!(error.contains("work in memory"), "{error}");
    }

    #[test]
    fn a_cancelled_job_says_so() {
        let cancel = AtomicBool::new(true);
        let Err(error) = run(
            Job::Format {
                input: text("[1]"),
                options: reformat::Options::default(),
            },
            &cancel,
        ) else {
            panic!("should be cancelled")
        };
        assert_eq!(error, "cancelled");
    }

    #[test]
    fn comparing_two_documents() {
        let Ok(Outcome::Diff(d)) = go(Job::Diff {
            left: text(r#"{"a":1,"b":[1,2,3]}"#),
            right: text(r#"{"a":2,"b":[1,3],"c":true}"#),
            take: None,
        }) else {
            panic!("should compare")
        };
        assert!(!d.equal);
        assert_eq!((d.added, d.removed, d.changed), (1, 1, 1));
        assert_eq!(d.total(), 3);
        assert_eq!(d.changes.len(), 3);
        let view = d.view.as_ref().expect("short enough to lay out");
        assert_eq!(view.blocks.len(), 3, "a change, a removal and an addition");
        assert_eq!((view.left_lines, view.right_lines), (8, 8));
        assert_eq!(
            &*d.patch,
            "[\n  {\"op\": \"replace\", \"path\": \"/a\", \"value\": 2},\n  \
             {\"op\": \"remove\", \"path\": \"/b/1\"},\n  \
             {\"op\": \"add\", \"path\": \"/c\", \"value\": true}\n]"
        );
    }

    /// A comparison of `left` and `right` that first moves `changes` into `into`.
    fn compared_after_moving(left: &str, right: &str, changes: Range<u32>, into: Side) -> Compared {
        let Ok(Outcome::Diff(d)) = go(Job::Diff {
            left: text(left),
            right: text(right),
            take: Some(Take { changes, into }),
        }) else {
            panic!("should compare")
        };
        d
    }

    #[test]
    fn a_move_changes_one_document_and_compares_again() {
        let (left, right) = (r#"{"a":1,"b":[1,2,3]}"#, r#"{"a":2,"b":[1,3],"c":true}"#);
        // The view numbers the changes a (0), the 2 taken out of b (1) and c (2).
        let d = compared_after_moving(left, right, 0..1, Side::Right);
        let moved = d.moved.expect("the right document was changed");
        assert_eq!(moved.into, Side::Right);
        assert_eq!(
            serde_json::from_str::<Value>(&moved.text).unwrap(),
            json!({"a": 1, "b": [1, 3], "c": true})
        );
        assert!(moved.text.contains("\n  \"b\": ["), "written out in full");
        assert_eq!(
            (d.added, d.removed, d.changed),
            (1, 1, 0),
            "what is left to compare"
        );

        // Into the left one, and the second difference.
        let d = compared_after_moving(left, right, 1..2, Side::Left);
        let moved = d.moved.expect("the left document was changed");
        assert_eq!(moved.into, Side::Left);
        assert_eq!(
            serde_json::from_str::<Value>(&moved.text).unwrap(),
            json!({"a": 1, "b": [1, 3]})
        );
        assert_eq!((d.added, d.removed, d.changed), (1, 0, 1));

        // Everything moved: the two are the same.
        let d = compared_after_moving(left, right, 0..3, Side::Left);
        assert!(d.equal);
        assert_eq!(&*d.patch, "[]");
    }

    #[test]
    fn a_comparison_without_a_move_has_nothing_to_put_back() {
        let Ok(Outcome::Diff(d)) = go(Job::Diff {
            left: text("[1]"),
            right: text("[2]"),
            take: None,
        }) else {
            panic!("should compare")
        };
        assert!(d.moved.is_none());
    }

    #[test]
    fn a_move_whose_text_is_too_big_for_a_box_is_still_made() {
        // The text goes back in a box that holds it without showing it, so a
        // text box's limit does not stop a move.
        let big = format!(
            "[{}]",
            vec!["\"abcdefghijklmnopqrstuvwxyz\""; 50_000].join(",")
        );
        let d = compared_after_moving(&big, "[]", 0..50_000, Side::Right);
        let moved = d.moved.expect("the right document was changed");
        assert_eq!(moved.into, Side::Right);
        assert!(moved.text.len() as u64 > crate::tools::operand::TEXT_LIMIT);
        assert!(d.equal, "the right document has what the left has");
    }

    #[test]
    fn the_same_documents_have_an_empty_patch() {
        let Ok(Outcome::Diff(d)) = go(Job::Diff {
            left: text(r#"{"a":1,"b":2}"#),
            right: text(r#"{"b":2,"a":1}"#),
            take: None,
        }) else {
            panic!("should compare")
        };
        assert!(d.equal);
        assert_eq!(&*d.patch, "[]");
    }

    #[test]
    fn a_bad_document_is_named_by_its_side() {
        let Err(error) = go(Job::Diff {
            left: text("[1]"),
            right: text("[1,"),
            take: None,
        }) else {
            panic!("should fail")
        };
        assert!(error.starts_with("Right: "), "{error}");
    }

    #[test]
    fn patching() {
        let Ok(Outcome::Patch(p)) = go(Job::Patch {
            document: text(r#"{"a":1}"#),
            patch: text(r#"[{"op":"add","path":"/b","value":[2]}]"#),
            merge_patch: false,
        }) else {
            panic!("should patch")
        };
        assert_eq!(p.doc.tree(), Some(&json!({"a": 1, "b": [2]})));
        assert_eq!(p.operations, Some(1));
        assert_eq!(p.doc.source.label(), "(patched)");
        assert_eq!(&*p.text, "{\n  \"a\": 1,\n  \"b\": [\n    2\n  ]\n}");
    }

    #[test]
    fn merge_patching() {
        let Ok(Outcome::Patch(p)) = go(Job::Patch {
            document: text(r#"{"a":1,"b":2}"#),
            patch: text(r#"{"a":null,"c":3}"#),
            merge_patch: true,
        }) else {
            panic!("should patch")
        };
        assert_eq!(p.doc.tree(), Some(&json!({"b": 2, "c": 3})));
        assert_eq!(p.operations, None);
    }

    #[test]
    fn a_failing_operation_is_named() {
        let Err(error) = go(Job::Patch {
            document: text(r#"{"a":1}"#),
            patch: text(r#"[{"op":"remove","path":"/zzz"}]"#),
            merge_patch: false,
        }) else {
            panic!("should fail")
        };
        assert!(error.contains("operation 1 (remove /zzz)"), "{error}");
    }

    #[test]
    fn a_patch_that_is_not_a_list_is_explained() {
        let Err(error) = go(Job::Patch {
            document: text(r#"{"a":1}"#),
            patch: text(r#"{"a":2}"#),
            merge_patch: false,
        }) else {
            panic!("should fail")
        };
        assert!(error.contains("array of operations"), "{error}");
    }

    #[test]
    fn validating() {
        let doc = Arc::new(jsonquery_core::load_text(r#"{"age": -1}"#).unwrap());
        let Ok(Outcome::Validate(v)) = go(Job::Validate {
            document: Input::Document(doc.clone()),
            schema: text(r#"{"properties":{"age":{"minimum":0}},"required":["name"]}"#),
            check_formats: true,
        }) else {
            panic!("should validate")
        };
        assert_eq!(v.report.problems.len(), 2);
        assert!(v.open.is_some_and(|open| Arc::ptr_eq(&open, &doc)));
    }

    #[test]
    fn a_document_that_is_not_the_open_one_has_no_open_document() {
        let Ok(Outcome::Validate(v)) = go(Job::Validate {
            document: text("1"),
            schema: text(r#"{"type":"integer"}"#),
            check_formats: false,
        }) else {
            panic!("should validate")
        };
        assert!(v.report.is_valid());
        assert!(v.open.is_none());
    }

    #[test]
    fn a_schema_that_cannot_be_used_is_explained() {
        let Err(error) = go(Job::Validate {
            document: text("1"),
            schema: text(r#"{"type": 5}"#),
            check_formats: false,
        }) else {
            panic!("should fail")
        };
        assert!(error.starts_with("The schema can't be used: "), "{error}");
    }

    #[test]
    fn a_preview_keeps_the_first_lines() {
        let body: String = (0..2_000).map(|i| format!("line {i}\n")).collect();
        let preview = Preview::of(&body);
        assert!(preview.truncated);
        assert_eq!(preview.text.lines().count(), PREVIEW_LINES);
        assert!(preview
            .text
            .ends_with(&format!("line {}", PREVIEW_LINES - 1)));
    }

    #[test]
    fn a_short_text_is_all_there_is() {
        let preview = Preview::of("a\nb");
        assert_eq!(preview.text, "a\nb");
        assert!(!preview.truncated);
        let preview = Preview::of("");
        assert_eq!(preview.text, "");
        assert!(!preview.truncated);
    }

    #[test]
    fn one_enormous_line_is_cut_on_a_character_boundary() {
        let body = "é".repeat(PREVIEW_BYTES);
        let preview = Preview::of(&body);
        assert!(preview.truncated);
        assert!(preview.text.len() <= PREVIEW_BYTES);
        assert!(preview.text.chars().all(|c| c == 'é'));
    }

    #[test]
    fn operations_are_listed_one_to_a_line() {
        assert_eq!(operations_text(&[]), "[]");
        let ops = [json!({"op": "remove", "path": "/a"})];
        assert_eq!(
            operations_text(&ops),
            "[\n  {\"op\": \"remove\", \"path\": \"/a\"}\n]"
        );
    }

    #[test]
    fn an_operation_keeps_its_value_whole_on_its_line_and_the_text_is_json() {
        let ops = [
            json!({"op": "add", "path": "/a b", "value": {"k": [1, "x y"], "z": null}}),
            json!({"op": "remove", "path": "/q\"uote"}),
        ];
        let text = operations_text(&ops);
        assert_eq!(
            text,
            "[\n  {\"op\": \"add\", \"path\": \"/a b\", \"value\": {\"k\":[1,\"x y\"],\"z\":null}},\n  \
             {\"op\": \"remove\", \"path\": \"/q\\\"uote\"}\n]"
        );
        // Whatever the spaces, it reads back as the same operations.
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back, Value::Array(ops.to_vec()));
    }
}
