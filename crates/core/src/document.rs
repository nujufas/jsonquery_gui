use std::fs::{File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use memmap2::Mmap;
use serde_json::Value;

use crate::lazy::{LazyTree, Node};
use crate::tree::{PathSegment, ValueKind};
use crate::view::ValueView;

/// A file of at least this many bytes is not parsed into a tree: it is kept
/// where it is, memory-mapped, and read as it is looked at (see
/// [`crate::lazy`]); any other is read and parsed.
///
/// The tree a parse makes takes about twelve times the file for records of a few
/// short fields (17 times, measured, for 100 MB of them), so a file of a few
/// hundred megabytes is already more than a machine has to spare, where the
/// index of a lazy document takes a thousandth. Below it, a parsed document is
/// the better one: everything about it is quick, and none of what a lazy one
/// has to give up (see `docs/architecture.md`) is given up. A mapping has costs
/// a read has not: it needs a regular file on a filesystem that can be mapped, a
/// file truncated by another process while it is mapped makes the document
/// [damaged](LazyTree::damaged) (the reads past the cut are caught, and not the
/// end of the process), and on Windows it keeps the file from being replaced.
pub const LAZY_THRESHOLD: u64 = 256 * 1024 * 1024;

/// The most that is read from what is not a regular file (a pipe, say): it has
/// no length to go by and may have no end. The same "few GB" ceiling as a URL
/// download has.
pub const MAX_STREAM_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// The sizes that decide how a file is brought in. The default is what the app
/// has always done ([`LAZY_THRESHOLD`], [`MAX_STREAM_BYTES`]); the app's settings
/// can change them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadLimits {
    /// A file of at least this many bytes is kept as it is, memory-mapped and
    /// indexed, rather than read and parsed (see [`LAZY_THRESHOLD`]).
    pub lazy_threshold: u64,
    /// The most that is read from what is not a regular file, one byte more of
    /// which is an error (see [`MAX_STREAM_BYTES`]).
    pub stream_bytes: u64,
}

impl Default for LoadLimits {
    fn default() -> Self {
        Self {
            lazy_threshold: LAZY_THRESHOLD,
            stream_bytes: MAX_STREAM_BYTES,
        }
    }
}

/// Where a [`Document`]'s bytes came from.
pub enum DocumentSource {
    File(PathBuf),
    /// JSON typed or pasted directly into the app, rather than opened from disk.
    Pasted,
    /// Downloaded from a URL: held in memory when it is small, in a temporary
    /// file nobody can see when it is not.
    Url(String),
    /// Built by the Tools window's merge from these files, in this order. It
    /// lives in memory, or, when the result is big, in a temporary file nobody
    /// can see, until it is saved.
    Merged(Vec<PathBuf>),
    /// Made by another of the Tools window's tools (a patch applied to a
    /// document). It lives in memory only, until it is saved. `label` is what
    /// the toolbar calls it, `file_name` what a save suggests.
    Derived {
        label: &'static str,
        file_name: &'static str,
    },
}

impl DocumentSource {
    pub fn label(&self) -> String {
        match self {
            DocumentSource::File(p) => p.display().to_string(),
            DocumentSource::Pasted => "(pasted JSON)".to_string(),
            DocumentSource::Url(url) => url.clone(),
            DocumentSource::Merged(files) => match files.len() {
                1 => "(merged from 1 file)".to_string(),
                n => format!("(merged from {n} files)"),
            },
            DocumentSource::Derived { label, .. } => (*label).to_string(),
        }
    }
}

/// What a [`Document`] holds.
pub enum Content {
    /// The parsed value, all of it in memory.
    Tree(Value),
    /// The bytes, indexed: nothing of the value is in memory until it is asked
    /// for. Boxed: the index is nearly four times the size of a value, and
    /// where it is bigger still (as on macOS) clippy says so.
    Lazy(Box<LazyTree>),
}

/// A loaded JSON document: its content plus load/parse timing and provenance,
/// used to populate the status bar.
pub struct Document {
    pub source: DocumentSource,
    pub byte_len: u64,
    /// How long it took to bring the bytes in and parse (or index) them.
    pub parse_time: Duration,
    pub content: Content,
    /// Number of top-level values found in the source (>1 means the file was
    /// NDJSON / concatenated JSON and got wrapped into a single array root).
    pub top_level_values: usize,
}

impl Document {
    /// A document whose value was built in memory (a merge of several files)
    /// rather than parsed from one source. `byte_len` is the size of what it
    /// was built from, and `parse_time` the time that took to read.
    pub fn from_value(
        root: Value,
        source: DocumentSource,
        byte_len: u64,
        parse_time: Duration,
    ) -> Self {
        Self {
            source,
            byte_len,
            parse_time,
            content: Content::Tree(root),
            top_level_values: 1,
        }
    }

    /// The root of the document, whichever it is made of.
    pub fn root(&self) -> Root<'_> {
        match &self.content {
            Content::Tree(value) => Root::Tree(value),
            Content::Lazy(tree) => Root::Lazy(tree.root()),
        }
    }

    /// The parsed value, if the document is one (a lazy one has none: it would
    /// be too big).
    pub fn tree(&self) -> Option<&Value> {
        match &self.content {
            Content::Tree(value) => Some(value),
            Content::Lazy(_) => None,
        }
    }

    /// A value to complete a query against: the document, or, when it is too
    /// big to be one, [`LazyTree::sample`] of it.
    pub fn suggestion_root(&self) -> &Value {
        match &self.content {
            Content::Tree(value) => value,
            Content::Lazy(tree) => tree.sample(),
        }
    }

    pub fn lazy(&self) -> Option<&LazyTree> {
        match &self.content {
            Content::Tree(_) => None,
            Content::Lazy(tree) => Some(tree),
        }
    }

    /// Whether it is kept as the file it came from, indexed, rather than
    /// parsed (see [`LAZY_THRESHOLD`]).
    pub fn is_lazy(&self) -> bool {
        matches!(self.content, Content::Lazy(_))
    }

    fn lazy_from(source: DocumentSource, tree: LazyTree, parse_time: Duration) -> Self {
        Self {
            source,
            byte_len: tree.len() as u64,
            parse_time,
            top_level_values: tree.top_level_values(),
            content: Content::Lazy(Box::new(tree)),
        }
    }
}

/// The root of a [`Document`]: a parsed value or a node of an indexed file.
/// Either is a [`ValueView`], so the tree widget, the search and the rest take
/// this and need not know which.
#[derive(Clone, Copy)]
pub enum Root<'a> {
    Tree(&'a Value),
    Lazy(Node<'a>),
}

impl<'a> ValueView for Root<'a> {
    fn kind(&self) -> ValueKind {
        match self {
            Root::Tree(value) => ValueView::kind(value),
            Root::Lazy(node) => ValueView::kind(node),
        }
    }

    fn child_count(&self) -> usize {
        match self {
            Root::Tree(value) => ValueView::child_count(value),
            Root::Lazy(node) => ValueView::child_count(node),
        }
    }

    fn child_by_key(&self, key: &str) -> Option<Self> {
        match self {
            Root::Tree(value) => value.child_by_key(key).map(Root::Tree),
            Root::Lazy(node) => ValueView::child_by_key(node, key).map(Root::Lazy),
        }
    }

    fn child_at(&self, index: usize) -> Option<Self> {
        match self {
            Root::Tree(value) => value.child_at(index).map(Root::Tree),
            Root::Lazy(node) => ValueView::child_at(node, index).map(Root::Lazy),
        }
    }

    fn iter_children(&self) -> Box<dyn Iterator<Item = (Option<PathSegment>, Self)> + '_> {
        match self {
            Root::Tree(value) => Box::new(
                value
                    .iter_children()
                    .map(|(segment, child)| (segment, Root::Tree(child))),
            ),
            Root::Lazy(node) => Box::new(
                ValueView::iter_children(node).map(|(segment, child)| (segment, Root::Lazy(child))),
            ),
        }
    }

    fn scalar_value(&self) -> Option<Value> {
        match self {
            Root::Tree(value) => value.scalar_value(),
            Root::Lazy(node) => ValueView::scalar_value(node),
        }
    }

    fn scalar_preview(&self) -> Option<String> {
        match self {
            Root::Tree(value) => value.scalar_preview(),
            Root::Lazy(node) => ValueView::scalar_preview(node),
        }
    }
}

/// Load a JSON file in one step: read and parsed into a tree, or, from
/// [`LAZY_THRESHOLD`] bytes, memory-mapped and indexed (see [`crate::lazy`]).
/// Either way the file is checked as JSON, all of it, before this returns.
pub fn load(path: impl AsRef<Path>) -> Result<Document> {
    load_with(path, LoadLimits::default())
}

/// [`load`] with the sizes that decide how the file is brought in given (the
/// app's settings can change them from what [`load`] uses).
pub fn load_with(path: impl AsRef<Path>, limits: LoadLimits) -> Result<Document> {
    load_limited(path.as_ref(), limits, map_file)
}

/// [`load_with`] with the way of mapping a file as a parameter too, so that a
/// mapping that fails can be made to.
fn load_limited(
    path: &Path,
    limits: LoadLimits,
    map: impl FnOnce(&File) -> io::Result<Mmap>,
) -> Result<Document> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    load_file(
        &file,
        DocumentSource::File(path.to_path_buf()),
        &path.display().to_string(),
        limits,
        map,
    )
}

/// [`load`] with the size from which a file is kept lazy and the way of mapping
/// one as parameters, so that both ways in can be tried on a small file.
#[cfg(test)]
fn load_via(
    path: &Path,
    lazy_threshold: u64,
    map: impl FnOnce(&File) -> io::Result<Mmap>,
) -> Result<Document> {
    let limits = LoadLimits {
        lazy_threshold,
        ..LoadLimits::default()
    };
    load_limited(path, limits, map)
}

/// [`load`] for a file that is already open — the temporary file of a download
/// or of a big merge, which has no name — as the document from `source`, kept lazy
/// if it is `lazy_threshold` bytes or more (as a file is, from [`LoadLimits`]).
/// It is read from its start, whatever the position of the handle is: a file
/// that has just been written is at its end.
pub fn load_open_file(
    file: &File,
    source: DocumentSource,
    lazy_threshold: u64,
) -> Result<Document> {
    load_open_via(file, source, lazy_threshold, map_file)
}

/// [`load_open_file`] with the way of mapping a file as a parameter too.
fn load_open_via(
    file: &File,
    source: DocumentSource,
    lazy_threshold: u64,
    map: impl FnOnce(&File) -> io::Result<Mmap>,
) -> Result<Document> {
    // A mapping starts at the start whatever the handle's position is, but a
    // read (the way in for a file that cannot be mapped) goes on from it. What
    // cannot seek is read from where it is.
    let mut handle = file;
    let _ = handle.seek(SeekFrom::Start(0));
    let name = source.label();
    let limits = LoadLimits {
        lazy_threshold,
        ..LoadLimits::default()
    };
    load_file(file, source, &name, limits, map)
}

fn load_file(
    file: &File,
    source: DocumentSource,
    name: &str,
    limits: LoadLimits,
    map: impl FnOnce(&File) -> io::Result<Mmap>,
) -> Result<Document> {
    let lazy_threshold = limits.lazy_threshold;
    let meta = file
        .metadata()
        .with_context(|| format!("reading metadata for {name}"))?;

    // A device is no file of JSON, and reading one is no way to find that out:
    // /dev/zero would be read until memory ran out, a terminal until somebody
    // typed an end of file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let kind = meta.file_type();
        if kind.is_char_device() || kind.is_block_device() {
            return Err(anyhow!("{name} is a device, not a file"));
        }
    }

    let start = Instant::now();

    // A regular file this big is not parsed: it stays where it is, mapped. (A
    // pipe or a file of /proc says it is 0 bytes long however much it will
    // hand over, and a mapping of 0 bytes maps nothing, so neither is mapped.)
    if meta.is_file() && meta.len() > 0 && meta.len() >= lazy_threshold {
        // Some files cannot be mapped (one of /sys, on some FUSE filesystems).
        // It is read like any other, not refused.
        if let Ok(mapping) = map(file) {
            let tree = LazyTree::from_mmap(mapping).context("parsing JSON")?;
            return Ok(Document::lazy_from(source, tree, start.elapsed()));
        }
    }

    let bytes = read_bytes(file, &meta, name, limits.stream_bytes)?;
    if bytes.len() as u64 >= lazy_threshold {
        // Too big to parse into a tree, and not to be mapped: the bytes are
        // kept, which takes one file's worth of memory and no more.
        let tree = LazyTree::from_vec(bytes).context("parsing JSON")?;
        return Ok(Document::lazy_from(source, tree, start.elapsed()));
    }
    let (root, top_level_values) = parse_bytes(&bytes)?;
    Ok(Document {
        source,
        byte_len: bytes.len() as u64,
        parse_time: start.elapsed(),
        content: Content::Tree(root),
        top_level_values,
    })
}

/// Bring a file's bytes in, to the end — or, for what is not a regular file, to
/// `stream_bytes`.
fn read_bytes(file: &File, meta: &Metadata, name: &str, stream_bytes: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    // One byte more than the file says it has, so that a file of just that
    // length is read without its buffer growing to find the end of it. A file
    // too big for memory is an error to report, not a reason to abort.
    let reserve = usize::try_from(meta.len()).map_or(0, |len| len.saturating_add(1));
    bytes
        .try_reserve_exact(reserve)
        .map_err(|_| anyhow!("not enough memory to read {name} ({} bytes)", meta.len()))?;
    // A regular file ends where it says it does; anything else is read only up
    // to the ceiling, and one byte past it shows that there was more.
    let limit = if meta.is_file() {
        u64::MAX
    } else {
        stream_bytes
    };
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {name}"))?;
    if bytes.len() as u64 > limit {
        return Err(anyhow!(
            "{name} hands over more than {limit} bytes, the most that is read from a pipe"
        ));
    }
    Ok(bytes)
}

fn map_file(file: &File) -> io::Result<Mmap> {
    // SAFETY: the mapping is only read. What can still go wrong is another
    // process truncating the file while it is mapped, which for a lazy document
    // is as long as it is open: the pages past its new end are gone, and reading
    // one is a SIGBUS. `LazyTree::from_mmap` watches the mapping for that (see
    // `lazy::guard`), so that the read is zeros and the document says it is
    // damaged, rather than the process ending. That is the price of not holding
    // the file in memory, and why only a file too big to parse is mapped
    // (`LAZY_THRESHOLD`).
    unsafe { Mmap::map(file) }
}

/// Parse bytes that are already in memory (a small download) into a
/// [`Document`] from `source`. Uses the same one-or-more-top-level-values
/// parsing as the file path, so NDJSON behaves the same way.
pub fn load_bytes(bytes: &[u8], source: DocumentSource) -> Result<Document> {
    let start = Instant::now();
    let (root, top_level_values) = parse_bytes(bytes)?;
    let parse_time = start.elapsed();

    Ok(Document {
        source,
        byte_len: bytes.len() as u64,
        parse_time,
        content: Content::Tree(root),
        top_level_values,
    })
}

/// Parse JSON typed or pasted directly into the app (as opposed to opening a
/// file) into a [`Document`]. Uses the same one-or-more-top-level-values
/// parsing as the file path, so pasted NDJSON behaves the same way.
pub fn load_text(text: &str) -> Result<Document> {
    load_bytes(text.as_bytes(), DocumentSource::Pasted)
}

/// Parse JSON from raw bytes, treating the input as one *or more* top-level
/// values (see Architecture §3 "NDJSON and concatenated JSON"). A single
/// value is returned as-is; multiple values (NDJSON, or plain concatenated
/// JSON) are wrapped into one top-level array so the rest of the app only
/// ever deals with one root value.
fn parse_bytes(bytes: &[u8]) -> Result<(Value, usize)> {
    let mut de = serde_json::Deserializer::from_slice(bytes).into_iter::<Value>();

    let Some(first) = de.next() else {
        return Ok((Value::Array(Vec::new()), 0));
    };
    let first = first.context("parsing JSON")?;

    match de.next() {
        None => Ok((first, 1)),
        Some(second) => {
            let mut values = vec![first, second.context("parsing JSON")?];
            for v in de {
                values.push(v.context("parsing JSON")?);
            }
            let count = values.len();
            Ok((Value::Array(values), count))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The value a document stands for, whichever way it is kept.
    fn value_of(doc: &Document) -> Value {
        match &doc.content {
            Content::Tree(value) => value.clone(),
            Content::Lazy(tree) => tree.root().to_value(usize::MAX).unwrap(),
        }
    }

    #[test]
    fn a_built_document_says_where_it_came_from() {
        let files = vec![PathBuf::from("a.json"), PathBuf::from("b.json")];
        let doc = Document::from_value(
            json!([1, 2]),
            DocumentSource::Merged(files),
            12,
            Duration::ZERO,
        );
        assert_eq!(doc.source.label(), "(merged from 2 files)");
        assert_eq!(doc.byte_len, 12);
        assert_eq!(doc.top_level_values, 1);
        assert_eq!(doc.tree(), Some(&json!([1, 2])));
        assert!(!doc.is_lazy() && doc.lazy().is_none());
    }

    #[test]
    fn a_derived_document_names_itself() {
        let source = DocumentSource::Derived {
            label: "(patched)",
            file_name: "patched.json",
        };
        assert_eq!(source.label(), "(patched)");
    }

    #[test]
    fn one_merged_file_is_not_plural() {
        let source = DocumentSource::Merged(vec![PathBuf::from("a.json")]);
        assert_eq!(source.label(), "(merged from 1 file)");
    }

    /// A directory of its own under the temp dir, removed again when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("jsonquery-core-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_size_from_which_a_file_is_kept_lazy_is_256_mib() {
        assert_eq!(LAZY_THRESHOLD, 256 * 1024 * 1024);
    }

    #[test]
    fn a_small_file_is_parsed() {
        let dir = Scratch::new("small");
        let path = dir.write("small.json", r#"{"a": [1, 2]}"#);
        let doc = load(&path).unwrap();
        assert!(!doc.is_lazy());
        assert_eq!(doc.tree(), Some(&json!({"a": [1, 2]})));
        assert_eq!(doc.byte_len, 13);
        assert_eq!(doc.top_level_values, 1);
        assert_eq!(doc.source.label(), path.display().to_string());
    }

    #[test]
    fn the_threshold_is_where_parsing_turns_into_indexing() {
        let dir = Scratch::new("boundary");
        let path = dir.write("a.json", "[1, 2, 3]");
        let lazy = load_via(&path, 9, map_file).unwrap();
        assert!(lazy.is_lazy());
        assert!(lazy.lazy().unwrap().is_mapped());
        assert!(!load_via(&path, 10, map_file).unwrap().is_lazy());
    }

    #[test]
    fn a_lazy_document_has_what_a_parsed_one_has() {
        let dir = Scratch::new("same");
        for (name, text) in [
            (
                "object.json",
                r#"{"b": 1, "a": [true, null, "ü"], "n": 12345678901234567890}"#,
            ),
            ("ndjson.json", "{\"a\":1}\n{\"a\":2}\n[3]\n"),
            ("padded.json", "\n\n  [1, 2]  \n\n"),
            ("scalar.json", "\"just text\""),
        ] {
            let path = dir.write(name, text);
            let len = text.len() as u64;
            let lazy = load_via(&path, len, map_file).unwrap();
            let parsed = load_via(&path, len + 1, map_file).unwrap();
            assert!(lazy.is_lazy() && !parsed.is_lazy(), "{name}");
            assert_eq!(value_of(&lazy), value_of(&parsed), "{name}");
            assert_eq!(lazy.byte_len, len, "{name}");
            assert_eq!(parsed.byte_len, len, "{name}");
            assert_eq!(lazy.top_level_values, parsed.top_level_values, "{name}");
            assert_eq!(
                lazy.root().kind(),
                parsed.root().kind(),
                "{name}: what the root is"
            );
            assert_eq!(
                lazy.root().child_count(),
                parsed.root().child_count(),
                "{name}: how many children it has"
            );
        }
    }

    #[test]
    fn an_empty_file_is_an_empty_array_whatever_the_threshold() {
        let dir = Scratch::new("empty");
        let path = dir.write("empty.json", "");
        for threshold in [1, LAZY_THRESHOLD] {
            let doc = load_via(&path, threshold, map_file).unwrap();
            assert_eq!(doc.tree(), Some(&json!([])));
            assert_eq!((doc.byte_len, doc.top_level_values), (0, 0));
        }
        // Where everything is lazy, the nothing there is is still an array.
        let doc = load_via(&path, 0, map_file).unwrap();
        assert_eq!(value_of(&doc), json!([]));
        assert_eq!((doc.byte_len, doc.top_level_values), (0, 0));
    }

    #[test]
    fn a_file_that_cannot_be_mapped_is_read_and_indexed_instead() {
        let dir = Scratch::new("unmappable");
        let path = dir.write("a.json", "[1, 2]");
        let doc = load_via(&path, 1, |_| Err(io::ErrorKind::Unsupported.into())).unwrap();
        // Kept lazy, as a file this big is, but held in memory: a file's worth,
        // not the twelve of a parsed tree.
        assert!(doc.is_lazy() && !doc.lazy().unwrap().is_mapped());
        assert_eq!(value_of(&doc), json!([1, 2]));
        assert_eq!(doc.byte_len, 6);
    }

    #[test]
    fn a_file_just_written_is_read_from_its_start_mapped_or_not() {
        // The temporary file of a download or of a big merge: written through the
        // handle that then loads it, which is at the end of the file.
        use std::io::Write as _;
        let dir = Scratch::new("written-then-loaded");
        let file = &mut std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(dir.0.join("spill.json"))
            .unwrap();
        file.write_all(b"[1, 2]").unwrap();

        let mapped = load_open_file(file, DocumentSource::Pasted, 1).unwrap();
        assert!(mapped.lazy().unwrap().is_mapped());
        assert_eq!(value_of(&mapped), json!([1, 2]));

        // Not mapped, the bytes are read: from the start, not from the end, which
        // would be an empty array and no error.
        file.seek(SeekFrom::End(0)).unwrap();
        let read = load_open_via(file, DocumentSource::Pasted, 1, |_| {
            Err(io::ErrorKind::Unsupported.into())
        })
        .unwrap();
        assert!(!read.lazy().unwrap().is_mapped());
        assert_eq!(value_of(&read), json!([1, 2]));
        assert_eq!(read.byte_len, 6);
        file.seek(SeekFrom::End(0)).unwrap();
        let parsed = load_open_file(file, DocumentSource::Pasted, u64::MAX).unwrap();
        assert_eq!(parsed.tree(), Some(&json!([1, 2])));
    }

    #[cfg(unix)]
    #[test]
    fn a_named_pipe_is_read_to_its_end() {
        // A path typed into the source field that is a pipe (a named one here,
        // or `/dev/stdin` when the app is started at the end of one): a file that
        // says it is 0 bytes long. It used to load as an empty array.
        let dir = Scratch::new("pipe");
        let path = dir.0.join("pipe.json");
        // std cannot make one, and this crate has no libc.
        let made = std::process::Command::new("mkfifo").arg(&path).status();
        assert!(made.expect("mkfifo runs").success());
        let sender = path.clone();
        let writer =
            std::thread::spawn(move || std::fs::write(sender, br#"{"from": "a pipe"}"#).unwrap());

        let doc = load_via(&path, u64::MAX, map_file).unwrap();
        writer.join().unwrap();
        assert_eq!(doc.tree(), Some(&json!({"from": "a pipe"})));
        assert_eq!(doc.byte_len, 18);
    }

    /// The ceiling of a pipe, where a test has to go past it: a megabyte, not the
    /// gigabytes of [`MAX_STREAM_BYTES`].
    const CEILING: u64 = 1024 * 1024;

    /// What `load_limited` makes of `bytes` written into a named pipe.
    #[cfg(unix)]
    fn load_through_a_pipe(name: &str, bytes: Vec<u8>, limits: LoadLimits) -> Result<Document> {
        let dir = Scratch::new(name);
        let path = dir.0.join("pipe.json");
        // std cannot make one, and this crate has no libc.
        let made = std::process::Command::new("mkfifo").arg(&path).status();
        assert!(made.expect("mkfifo runs").success());
        let sender = path.clone();
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            let mut pipe = std::fs::OpenOptions::new()
                .write(true)
                .open(sender)
                .unwrap();
            // A reader that has had enough closes the pipe: not this thread's error.
            let _ = pipe.write_all(&bytes);
        });
        let result = load_limited(&path, limits, map_file);
        writer.join().unwrap();
        result
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_that_hands_over_a_lot_is_kept_lazy_in_memory_not_mapped() {
        // Its size is only known once it has been read, and it cannot be mapped:
        // the bytes are held, and indexed, rather than parsed into a tree.
        let limits = LoadLimits {
            lazy_threshold: 10,
            ..LoadLimits::default()
        };
        let doc =
            load_through_a_pipe("lazy-pipe", br#"{"from": "a pipe"}"#.to_vec(), limits).unwrap();
        assert!(doc.is_lazy() && !doc.lazy().unwrap().is_mapped());
        assert_eq!(value_of(&doc), json!({"from": "a pipe"}));
        assert_eq!(doc.byte_len, 18);
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_may_give_the_ceiling_and_not_one_byte_more() {
        let blanks = |len: u64| {
            let mut bytes = b"[1]".to_vec();
            bytes.resize(len as usize, b' ');
            bytes
        };
        let limits = LoadLimits {
            lazy_threshold: u64::MAX,
            stream_bytes: CEILING,
        };
        let doc = load_through_a_pipe("at-ceiling", blanks(CEILING), limits).unwrap();
        assert_eq!(doc.tree(), Some(&json!([1])));
        assert_eq!(doc.byte_len, CEILING);

        // Finite, so that a ceiling that stopped working would fail this and not
        // fill the memory of whoever runs it.
        let err = load_through_a_pipe("past-ceiling", blanks(CEILING + 1), limits)
            .err()
            .expect("one byte more than is read");
        let err = format!("{err:#}");
        assert!(err.contains("more than 1048576 bytes"), "{err}");
    }

    #[test]
    fn a_regular_file_is_not_held_to_the_ceiling_of_a_pipe() {
        let dir = Scratch::new("regular-over-ceiling");
        let mut text = b"[1]".to_vec();
        text.resize(CEILING as usize + 100, b' ');
        let path = dir.write("long.json", &text);
        // Read, as a file below the lazy size is; it ends where it says it does.
        let limits = LoadLimits {
            lazy_threshold: u64::MAX,
            stream_bytes: CEILING,
        };
        let doc = load_limited(&path, limits, map_file).unwrap();
        assert_eq!(doc.tree(), Some(&json!([1])));
        assert_eq!(doc.byte_len, CEILING + 100);
    }

    #[test]
    fn the_limits_are_what_the_app_has_always_used_unless_they_are_changed() {
        let limits = LoadLimits::default();
        assert_eq!(limits.lazy_threshold, 256 * 1024 * 1024);
        assert_eq!(limits.stream_bytes, 4 * 1024 * 1024 * 1024);
        assert_eq!(limits.lazy_threshold, LAZY_THRESHOLD);
        assert_eq!(limits.stream_bytes, MAX_STREAM_BYTES);
    }

    #[test]
    fn load_with_keeps_a_file_lazy_from_the_threshold_it_is_given() {
        let dir = Scratch::new("limits-threshold");
        let path = dir.write("a.json", "[1, 2, 3]");
        let at = |lazy_threshold| LoadLimits {
            lazy_threshold,
            ..LoadLimits::default()
        };
        // Nine bytes: lazy from nine, parsed from ten — not from 256 MiB.
        assert!(load_with(&path, at(9)).unwrap().is_lazy());
        assert!(!load_with(&path, at(10)).unwrap().is_lazy());
        assert!(!load(&path).unwrap().is_lazy());
        // A lower size than the default does not need a bigger file, and a
        // higher one keeps a file parsed that the default would have mapped.
        assert!(load_with(&path, at(1)).unwrap().is_lazy());
        assert!(!load_with(&path, at(u64::MAX)).unwrap().is_lazy());
    }

    #[cfg(unix)]
    #[test]
    fn a_device_is_refused_rather_than_read() {
        // /dev/null is empty, so reading it would do no harm; /dev/zero, a
        // terminal, would. All of them are devices.
        let err = format!("{:#}", load("/dev/null").err().expect("a device"));
        assert!(err.contains("/dev/null is a device"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_is_an_error_that_says_what_it_is() {
        let dir = Scratch::new("directory");
        let err = format!("{:#}", load(&dir.0).err().expect("not JSON"));
        assert!(err.contains("Is a directory"), "{err}");
    }

    #[test]
    fn a_missing_file_is_named_in_the_error() {
        let path = Scratch::new("missing").0.join("nope.json");
        let err = format!("{:#}", load(&path).err().expect("no such file"));
        assert!(
            err.contains("opening") && err.contains(&path.display().to_string()),
            "{err}"
        );
    }

    #[test]
    fn text_that_is_not_json_is_the_same_error_either_way_in() {
        let dir = Scratch::new("invalid");
        for (name, text) in [
            ("a.json", "[1,"),
            ("b.json", "{\"a\": tru}"),
            ("c.json", "[1, 2]\n[3, 4"),
            ("d.json", "\"é\n\""),
        ] {
            let path = dir.write(name, text);
            let parsed = format!("{:#}", load_via(&path, u64::MAX, map_file).err().unwrap());
            let lazy = format!("{:#}", load_via(&path, 1, map_file).err().unwrap());
            assert!(parsed.contains("parsing JSON"), "{parsed}");
            assert_eq!(lazy, parsed, "{name}");
        }
    }

    #[test]
    fn a_download_in_a_file_with_no_name_is_loaded_from_the_handle() {
        let dir = Scratch::new("handle");
        let path = dir.write("body.json", r#"{"k": [1, 2, 3]}"#);
        let file = File::open(&path).unwrap();
        let source = || DocumentSource::Url("http://example.test/body.json".to_owned());
        let at = |lazy_threshold| LoadLimits {
            lazy_threshold,
            ..LoadLimits::default()
        };
        let lazy = load_file(&file, source(), "the download", at(4), map_file).unwrap();
        assert!(lazy.is_lazy());
        assert_eq!(value_of(&lazy), json!({"k": [1, 2, 3]}));
        assert_eq!(lazy.source.label(), "http://example.test/body.json");
        let small = load_file(&file, source(), "the download", at(u64::MAX), map_file).unwrap();
        assert_eq!(small.tree(), Some(&json!({"k": [1, 2, 3]})));
    }

    #[test]
    fn several_top_level_values_become_one_array() {
        let doc = load_text("{\"a\":1}\n[2]\n3").unwrap();
        assert_eq!(doc.tree(), Some(&json!([{"a": 1}, [2], 3])));
        assert_eq!(doc.top_level_values, 3);
        assert!(matches!(doc.source, DocumentSource::Pasted));

        let one = load_text(" [1] ").unwrap();
        assert_eq!((one.tree(), one.top_level_values), (Some(&json!([1])), 1));

        let none = load_text("  \n").unwrap();
        assert_eq!((none.tree(), none.top_level_values), (Some(&json!([])), 0));
    }

    #[test]
    fn bytes_already_in_memory_become_a_document_from_the_source_given() {
        let doc = load_bytes(
            br#"{"k": 1}"#,
            DocumentSource::Url("http://example.test/k.json".to_owned()),
        )
        .unwrap();
        assert_eq!(doc.tree(), Some(&json!({"k": 1})));
        assert_eq!(doc.byte_len, 8);
        assert!(!doc.is_lazy());
        assert_eq!(doc.source.label(), "http://example.test/k.json");
    }

    #[test]
    fn the_root_of_either_kind_of_document_is_the_same_to_a_viewer() {
        use crate::tree::{flatten_visible, new_expanded_at_root};

        let dir = Scratch::new("viewer");
        let text = r#"{"users": [{"name": "Ada", "tags": ["x"]}, {"name": "Alan"}], "n": 3}"#;
        let path = dir.write("v.json", text);
        let lazy = load_via(&path, 1, map_file).unwrap();
        let parsed = load_via(&path, u64::MAX, map_file).unwrap();
        let mut expand = new_expanded_at_root();
        expand.insert(vec![PathSegment::Key("users".into())]);
        expand.insert(vec![
            PathSegment::Key("users".into()),
            PathSegment::Index(0),
        ]);
        let rows = |doc: &Document| {
            flatten_visible(doc.root(), &expand)
                .into_iter()
                .map(|r| (r.path, r.depth, r.kind, r.child_count, r.scalar_preview))
                .collect::<Vec<_>>()
        };
        assert_eq!(rows(&lazy), rows(&parsed));
        assert!(!rows(&lazy).is_empty());
    }
}
