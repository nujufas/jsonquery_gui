//! A document that stays on disk: the file is memory-mapped, checked once, and
//! only the places that matter are remembered — where the children of its big
//! containers are — so that any node can be found, and parsed on its own,
//! without the document ever being a tree in memory.
//!
//! A parsed document takes about twelve times the size of its file (Architecture
//! §"Scaling beyond in-memory"); this takes a few bytes per thousand. What it
//! costs instead is that every question is asked of the file, so each answer
//! takes some work: finding the `n`th child of an array of millions is a binary
//! search over its checkpoints and then a walk past at most
//! [`IndexConfig::cp_children`] children, rather than an indexing operation.
//!
//! - [`scan`] reads the bytes once, as strictly as `serde_json` does, and
//!   builds the [`Index`]: for every container with many children or many bytes,
//!   where it ends, how many children it has, and a checkpoint (child number
//!   and byte offset) every so many children or so many bytes.
//! - A [`Node`] is a value of the document: the range of bytes it is, and what
//!   it is. It looks at the file only when asked: for its children (the big
//!   containers by their checkpoints, the small ones by walking them), for
//!   its value, or to be written out. It is a [`ValueView`], so the tree, the
//!   search and the text view work with it as they do with a parsed value.
//!
//! Everything here assumes what `scan` established — that the bytes are JSON —
//! and so never fails on a document; but a file that is changed while it is
//! mapped can show anything, so nothing here panics on bytes that are not. One
//! change is told of rather than shown: a file cut short has its mapping watched
//! (see [`guard`]), so that reading what is gone is zeros and
//! [`LazyTree::damaged`] is true, instead of the end of the process.

mod guard;
mod repeated;
mod scan;
mod search;
mod write;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::{Deref, Range};
use std::sync::{Arc, Mutex, OnceLock};

use memmap2::Mmap;
use serde_json::Value;

pub use scan::{ErrorKind, ScanError};
pub use search::search;
pub use write::{Indent, PrettyLimits, Style, Written};

use crate::tree::{PathSegment, ValueKind};
use crate::view::ValueView;

/// What to tell whoever asks something of a document whose file was cut short
/// while it was open (see [`LazyTree::damaged`]).
pub const CHANGED_WHILE_OPEN: &str = "the file was changed on disk while it was open, so what is \
     read from it can't be trusted: open it again";

/// The place the top level of a document of several values is kept in the
/// index, in the position of the bracket it does not have.
const STREAM_ROOT: usize = usize::MAX;

/// What the index keeps, and how finely.
#[derive(Clone, Copy, Debug)]
pub struct IndexConfig {
    /// A container with this many children has an entry of its own...
    pub min_children: usize,
    /// ... and so has one this many bytes long, which is also how a container
    /// that is small and has no entry is known to take little to walk.
    pub min_bytes: usize,
    /// A checkpoint is kept every this many children...
    pub cp_children: usize,
    /// ... or when this many bytes have gone by without one.
    pub cp_bytes: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            min_children: 256,
            min_bytes: 256 * 1024,
            cp_children: 64,
            cp_bytes: 1024 * 1024,
        }
    }
}

/// A child of a container, and where it begins: the byte of its value for an
/// array, of its key for an object.
#[derive(Clone, Copy, Debug)]
struct Checkpoint {
    child: usize,
    offset: usize,
}

/// A container the index knows about.
#[derive(Clone, Copy, Debug)]
struct Container {
    /// Where its opening bracket is ([`STREAM_ROOT`] for the top level of a
    /// document of several values).
    start: usize,
    /// Just past its closing bracket.
    end: usize,
    count: usize,
    /// Its checkpoints, as a range of `Index::pool`.
    cp_start: usize,
    cp_len: usize,
}

/// What the top level of the document is.
#[derive(Clone, Copy, Debug)]
enum Top {
    /// One value.
    Single {
        start: usize,
        end: usize,
        kind: ValueKind,
    },
    /// None, or several (NDJSON, or JSON values just written one after the
    /// other): an array of them, as far as everything else is concerned.
    Stream,
}

struct Index {
    /// Sorted by `start`.
    containers: Vec<Container>,
    pool: Vec<Checkpoint>,
    /// Where the objects that have a key twice begin, sorted: they are read as a
    /// parsed object is, with each key once.
    repeated: Vec<usize>,
    top: Top,
}

enum Backing {
    Mapped(Mmap),
    Owned(Vec<u8>),
}

impl Deref for Backing {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Backing::Mapped(map) => map,
            Backing::Owned(bytes) => bytes,
        }
    }
}

/// A JSON document that is read from its bytes as it is looked at.
pub struct LazyTree {
    /// Watches the mapping, if there is one. First, so that it is dropped, and
    /// the watch ended, before the mapping goes.
    watch: guard::Watch,
    data: Backing,
    index: Index,
    sample: OnceLock<Value>,
    /// The members of the objects that have a key twice, by where each begins,
    /// made when one is first looked at.
    members: Mutex<HashMap<usize, Arc<[repeated::Member]>>>,
}

impl LazyTree {
    /// Check and index a mapped file. The mapping is kept for as long as the
    /// tree lives.
    pub fn from_mmap(map: Mmap) -> Result<Self, ScanError> {
        // A file that is cut short from here on does not end the process (see
        // [`guard`]); watched from now, so that the scan is covered too.
        let watch = guard::Watch::new(map.as_ptr() as usize, map.len());
        // The scan goes through it once from front to back; what comes after is
        // looking things up here and there.
        #[cfg(unix)]
        let _ = map.advise(memmap2::Advice::Sequential);
        let tree = Self::build(Backing::Mapped(map), IndexConfig::default(), watch)?;
        #[cfg(unix)]
        if let Backing::Mapped(map) = &tree.data {
            let _ = map.advise(memmap2::Advice::Random);
        }
        Ok(tree)
    }

    /// Check and index bytes held in memory.
    pub fn from_vec(bytes: Vec<u8>) -> Result<Self, ScanError> {
        Self::build(
            Backing::Owned(bytes),
            IndexConfig::default(),
            guard::Watch::none(),
        )
    }

    /// [`from_vec`](Self::from_vec) with an index of the given grain, so that a
    /// small document can have the checkpoints a big one would.
    pub fn from_vec_with(bytes: Vec<u8>, cfg: IndexConfig) -> Result<Self, ScanError> {
        Self::build(Backing::Owned(bytes), cfg, guard::Watch::none())
    }

    fn build(data: Backing, cfg: IndexConfig, watch: guard::Watch) -> Result<Self, ScanError> {
        // Bytes that the file did not have make a document that is not JSON, or
        // one that is not the file's: either way, the file is what to blame.
        let scanned = scan::scan(&data, cfg);
        if watch.damaged() {
            return Err(ScanError::file_changed());
        }
        Ok(Self {
            watch,
            data,
            index: scanned?,
            sample: OnceLock::new(),
            members: Mutex::new(HashMap::new()),
        })
    }

    /// Whether the file was cut short while it was open, so that some of what
    /// was read from it is zeros that it never had: what is shown is not to be
    /// trusted, and the file is to be opened again. (The process is not ended by
    /// it; see [`guard`].)
    pub fn damaged(&self) -> bool {
        self.watch.damaged()
    }

    /// Whether the bytes are a mapping of a file (and not held in memory).
    pub fn is_mapped(&self) -> bool {
        matches!(self.data, Backing::Mapped(_))
    }

    /// The document's bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// How many bytes the document is.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// How many values the document has at its top level: one, unless it is a
    /// stream of them (see [`Top`]).
    pub fn top_level_values(&self) -> usize {
        match self.index.top {
            Top::Single { .. } => 1,
            Top::Stream => self
                .container_at(STREAM_ROOT)
                .map_or(0, |container| container.count),
        }
    }

    /// A small value that looks like the document, for what has to be shown
    /// against a value but cannot be shown against all of it: the first few
    /// children of every container, to a few levels, with long strings cut. The
    /// completions of a query box are made from it. It is made when it is first
    /// asked for.
    pub fn sample(&self) -> &Value {
        self.sample
            .get_or_init(|| sample_of(self.root(), 0, &mut SAMPLE_NODES.clone()))
    }

    /// The node that is `range` of the file, which has to be the range of one that
    /// was found: it is told what kind of value it is from its first byte, and
    /// nothing else is looked at. (`None` where there is no value: a range that
    /// is not in the file, as one of a file that has been changed can be.)
    pub fn node_at(&self, range: Range<usize>) -> Option<Node<'_>> {
        if range.start >= range.end || range.end > self.len() {
            return None;
        }
        let kind = kind_of(*self.bytes().get(range.start)?)?;
        Some(Node {
            tree: self,
            start: range.start,
            end: range.end,
            kind,
        })
    }

    /// The bytes the index itself takes, apart from the file.
    pub fn index_bytes(&self) -> usize {
        self.index.containers.len() * std::mem::size_of::<Container>()
            + self.index.pool.len() * std::mem::size_of::<Checkpoint>()
    }

    /// The document's root. A stream of several values (or of none) is an array
    /// of them, as when such a document is parsed.
    pub fn root(&self) -> Node<'_> {
        match self.index.top {
            Top::Single { start, end, kind } => Node {
                tree: self,
                start,
                end,
                kind,
            },
            Top::Stream => Node {
                tree: self,
                start: STREAM_ROOT,
                end: self.len(),
                kind: ValueKind::Array,
            },
        }
    }

    /// The members of the object that begins at `start`, if it has a key twice.
    fn members_at(&self, start: usize) -> Option<Arc<[repeated::Member]>> {
        self.index.repeated.binary_search(&start).ok()?;
        let mut made = self.members.lock().ok()?;
        Some(
            made.entry(start)
                .or_insert_with(|| repeated::members(self, start).into())
                .clone(),
        )
    }

    fn container_at(&self, start: usize) -> Option<&Container> {
        self.index
            .containers
            .binary_search_by(|c| c.start.cmp(&start))
            .ok()
            .map(|at| &self.index.containers[at])
    }

    fn checkpoints(&self, container: &Container) -> &[Checkpoint] {
        self.index
            .pool
            .get(container.cp_start..container.cp_start + container.cp_len)
            .unwrap_or(&[])
    }

    /// Where the first byte past the white space at `from` is.
    fn skip_ws(&self, mut from: usize) -> usize {
        let b = self.bytes();
        while let Some(&c) = b.get(from) {
            if matches!(c, b' ' | b'\n' | b'\t' | b'\r') {
                from += 1;
            } else {
                break;
            }
        }
        from
    }

    /// Just past the string whose opening quote is before `from`.
    fn skip_string(&self, mut from: usize) -> usize {
        let b = self.bytes();
        while let Some(found) = memchr::memchr2(b'"', b'\\', &b[from.min(b.len())..]) {
            from += found;
            if b[from] == b'"' {
                return from + 1;
            }
            // A backslash: the byte after it is part of the escape.
            from += 2;
        }
        b.len()
    }

    /// Just past the value that starts at `from`, a byte that is not white
    /// space. Looks the big containers up and walks the others, which are small.
    fn skip_value(&self, from: usize) -> usize {
        let b = self.bytes();
        match b.get(from) {
            None => from,
            Some(b'"') => self.skip_string(from + 1),
            Some(b'[' | b'{') => match self.container_at(from) {
                Some(container) => container.end,
                None => self.walk_container(from),
            },
            Some(_) => {
                let mut at = from + 1;
                while let Some(&c) = b.get(at) {
                    if matches!(
                        c,
                        b',' | b']'
                            | b'}'
                            | b':'
                            | b' '
                            | b'\n'
                            | b'\t'
                            | b'\r'
                            | b'"'
                            | b'['
                            | b'{'
                    ) {
                        break;
                    }
                    at += 1;
                }
                at
            }
        }
    }

    /// Just past the container whose bracket is at `from`, by counting
    /// brackets.
    fn walk_container(&self, from: usize) -> usize {
        let b = self.bytes();
        let mut depth = 0usize;
        let mut at = from;
        while let Some(&c) = b.get(at) {
            match c {
                b'"' => {
                    at = self.skip_string(at + 1);
                    continue;
                }
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return at + 1;
                    }
                }
                _ => {}
            }
            at += 1;
        }
        b.len()
    }
}

/// A value of a [`LazyTree`]: the bytes it is, and what it is.
#[derive(Clone, Copy)]
pub struct Node<'a> {
    tree: &'a LazyTree,
    start: usize,
    end: usize,
    kind: ValueKind,
}

/// How a container lays out its children.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Array,
    Object,
    /// Values one after the other, to the end of the file.
    Stream,
}

impl<'a> Node<'a> {
    pub fn kind(&self) -> ValueKind {
        self.kind
    }

    /// The bytes of the document it is. (For the root of a stream of values,
    /// all of it.)
    pub fn byte_range(&self) -> Range<usize> {
        if self.start == STREAM_ROOT {
            0..self.end
        } else {
            self.start..self.end
        }
    }

    pub fn byte_len(&self) -> usize {
        let range = self.byte_range();
        range.end - range.start
    }

    /// Its bytes, as they are in the file.
    pub fn raw(&self) -> &'a [u8] {
        self.tree.bytes().get(self.byte_range()).unwrap_or(&[])
    }

    pub fn tree(&self) -> &'a LazyTree {
        self.tree
    }

    fn layout(&self) -> Option<Layout> {
        if self.start == STREAM_ROOT {
            return Some(Layout::Stream);
        }
        match self.kind {
            ValueKind::Array => Some(Layout::Array),
            ValueKind::Object => Some(Layout::Object),
            _ => None,
        }
    }

    fn entry(&self) -> Option<&'a Container> {
        self.tree.container_at(self.start)
    }

    /// The members of an object that has a key twice, each key once.
    fn members(&self) -> Option<Arc<[repeated::Member]>> {
        if self.start == STREAM_ROOT || self.kind != ValueKind::Object {
            return None;
        }
        self.tree.members_at(self.start)
    }

    /// Where its first child begins: the byte of its value for an array, of its
    /// key for an object.
    fn first_child(&self) -> usize {
        let from = if self.start == STREAM_ROOT {
            0
        } else {
            self.start + 1
        };
        self.tree.skip_ws(from)
    }

    /// How many children it has: 0 for what is not a container.
    pub fn child_count(&self) -> usize {
        if self.layout().is_none() {
            return 0;
        }
        if let Some(members) = self.members() {
            return members.len();
        }
        match self.entry() {
            Some(container) => container.count,
            None => self.children().count(),
        }
    }

    /// Its children, in order.
    pub fn children(&self) -> Children<'a> {
        Children {
            tree: self.tree,
            at: self.first_child(),
            layout: self.layout(),
            index: 0,
            members: self.members(),
        }
    }

    /// Its children from the `index`th on: they are found by way of the
    /// checkpoint nearest before it, so this is quick wherever it is.
    pub fn children_from(&self, index: usize) -> Children<'a> {
        let mut children = self.children();
        if children.members.is_some() {
            children.index = index;
            return children;
        }
        if let Some(container) = self.entry() {
            let checkpoints = self.tree.checkpoints(container);
            let before = checkpoints.partition_point(|cp| cp.child <= index);
            if let Some(cp) = before.checked_sub(1).and_then(|at| checkpoints.get(at)) {
                children.at = cp.offset;
                children.index = cp.child;
            }
        }
        while children.index < index {
            if children.next().is_none() {
                break;
            }
        }
        children
    }

    /// The `index`th child.
    pub fn child(&self, index: usize) -> Option<Child<'a>> {
        self.layout()?;
        if self.members().is_none() {
            if let Some(container) = self.entry() {
                if index >= container.count {
                    return None;
                }
            }
        }
        self.children_from(index).next()
    }

    /// The child of an object under `key`.
    pub fn child_by_key(&self, key: &str) -> Option<Child<'a>> {
        if self.layout() != Some(Layout::Object) {
            return None;
        }
        let mut found = None;
        for child in self.children() {
            if child.key.is_some_and(|k| k.is(key)) {
                found = Some(child);
            }
        }
        found
    }

    /// Parse it, if it takes no more than `max_bytes` of the file.
    pub fn to_value(&self, max_bytes: usize) -> Result<Value, MaterializeError> {
        let bytes = self.byte_len();
        if bytes > max_bytes {
            return Err(MaterializeError::TooLarge { bytes });
        }
        if self.start != STREAM_ROOT {
            return serde_json::from_slice(self.raw()).map_err(MaterializeError::Parse);
        }
        // Several values at the top level: an array of them.
        let mut values = Vec::new();
        for value in serde_json::Deserializer::from_slice(self.raw()).into_iter::<Value>() {
            values.push(value.map_err(MaterializeError::Parse)?);
        }
        Ok(Value::Array(values))
    }

    /// The value of a scalar, whatever its size.
    pub fn scalar(&self) -> Option<Value> {
        if self.layout().is_some() {
            return None;
        }
        serde_json::from_slice(self.raw()).ok()
    }

    /// A string's text, whatever its size; `None` for what is not a string.
    pub fn string(&self) -> Option<String> {
        if self.kind != ValueKind::String {
            return None;
        }
        serde_json::from_slice(self.raw()).ok()
    }
}

/// Why a node could not be turned into a value.
#[derive(Debug)]
pub enum MaterializeError {
    TooLarge { bytes: usize },
    Parse(serde_json::Error),
}

impl std::fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaterializeError::TooLarge { bytes } => {
                write!(f, "it is {bytes} bytes, too many to hold in memory")
            }
            MaterializeError::Parse(e) => write!(f, "parsing JSON: {e}"),
        }
    }
}

impl std::error::Error for MaterializeError {}

/// One child of a container.
#[derive(Clone, Copy)]
pub struct Child<'a> {
    /// Its position among its siblings.
    pub index: usize,
    /// Its key, for the child of an object.
    pub key: Option<Key<'a>>,
    pub node: Node<'a>,
}

impl Child<'_> {
    /// How the child is reached from its parent.
    pub fn segment(&self) -> PathSegment {
        match self.key {
            Some(key) => PathSegment::Key(key.to_string()),
            None => PathSegment::Index(self.index),
        }
    }
}

/// An object key, as it is written in the file.
#[derive(Clone, Copy)]
pub struct Key<'a> {
    /// What is between the quotes.
    raw: &'a [u8],
}

impl Key<'_> {
    /// Where in `bytes` (the bytes of the document) what is between its quotes
    /// begins.
    fn offset_in(&self, bytes: &[u8]) -> usize {
        (self.raw.as_ptr() as usize).saturating_sub(bytes.as_ptr() as usize)
    }

    /// How many bytes of the file it is, as it is written.
    fn len(&self) -> usize {
        self.raw.len()
    }

    /// Whether it is `text`.
    pub fn is(&self, text: &str) -> bool {
        if memchr::memchr(b'\\', self.raw).is_none() {
            return self.raw == text.as_bytes();
        }
        self.decoded().is_some_and(|key| key == text)
    }

    pub(super) fn decoded(&self) -> Option<String> {
        let mut quoted = Vec::with_capacity(self.raw.len() + 2);
        quoted.push(b'"');
        quoted.extend_from_slice(self.raw);
        quoted.push(b'"');
        serde_json::from_slice(&quoted).ok()
    }

    pub fn compare(&self, other: &Key<'_>) -> Ordering {
        self.to_string().cmp(&other.to_string())
    }
}

impl std::fmt::Display for Key<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if memchr::memchr(b'\\', self.raw).is_none() {
            if let Ok(text) = std::str::from_utf8(self.raw) {
                return f.write_str(text);
            }
        }
        f.write_str(&self.decoded().unwrap_or_default())
    }
}

/// The children of a container, in order.
pub struct Children<'a> {
    tree: &'a LazyTree,
    /// Where the next one begins (or where the container ends).
    at: usize,
    layout: Option<Layout>,
    index: usize,
    /// For an object that has a key twice: its members, each key once, which are
    /// what is walked.
    members: Option<Arc<[repeated::Member]>>,
}

impl<'a> Iterator for Children<'a> {
    type Item = Child<'a>;

    fn next(&mut self) -> Option<Child<'a>> {
        let layout = self.layout?;
        let b = self.tree.bytes();
        if let Some(members) = &self.members {
            let member = members.get(self.index)?;
            let index = self.index;
            self.index += 1;
            return Some(Child {
                index,
                key: Some(Key {
                    raw: b.get(repeated::key_range(member)).unwrap_or(&[]),
                }),
                node: Node {
                    tree: self.tree,
                    start: member.value.0,
                    end: member.value.1,
                    kind: member.kind,
                },
            });
        }
        let first = *b.get(self.at)?;
        let (key, value_start) = match layout {
            Layout::Array => {
                if first == b']' {
                    return None;
                }
                (None, self.at)
            }
            Layout::Stream => (None, self.at),
            Layout::Object => {
                if first != b'"' {
                    return None;
                }
                let key_end = self.tree.skip_string(self.at + 1);
                let raw = b.get(self.at + 1..key_end.saturating_sub(1)).unwrap_or(&[]);
                let colon = self.tree.skip_ws(key_end);
                let value_start = self.tree.skip_ws(colon + 1);
                (Some(Key { raw }), value_start)
            }
        };
        let kind = kind_of(*b.get(value_start)?)?;
        let value_end = self.tree.skip_value(value_start);

        // On to the next one: past the comma there may be.
        let mut next = self.tree.skip_ws(value_end);
        if b.get(next) == Some(&b',') {
            next = self.tree.skip_ws(next + 1);
        }
        self.at = next;
        let index = self.index;
        self.index += 1;
        Some(Child {
            index,
            key,
            node: Node {
                tree: self.tree,
                start: value_start,
                end: value_end,
                kind,
            },
        })
    }
}

/// How much of a document [`LazyTree::sample`] holds: this many children of
/// each container, this many levels, and this many nodes in all.
const SAMPLE_CHILDREN: usize = 50;
const SAMPLE_DEPTH: usize = 10;
const SAMPLE_NODES: usize = 4000;
const SAMPLE_STRING_BYTES: usize = 200;

fn sample_of(node: Node<'_>, depth: usize, budget: &mut usize) -> Value {
    if *budget == 0 {
        return Value::Null;
    }
    *budget -= 1;
    match node.kind() {
        ValueKind::Array => Value::Array(if depth >= SAMPLE_DEPTH {
            Vec::new()
        } else {
            node.children()
                .take(SAMPLE_CHILDREN)
                .map(|child| sample_of(child.node, depth + 1, budget))
                .collect()
        }),
        ValueKind::Object => Value::Object(if depth >= SAMPLE_DEPTH {
            serde_json::Map::new()
        } else {
            node.children()
                .take(SAMPLE_CHILDREN)
                .map(|child| {
                    let key = child.key.map(|k| k.to_string()).unwrap_or_default();
                    (key, sample_of(child.node, depth + 1, budget))
                })
                .collect()
        }),
        ValueKind::String => {
            let (text, cut) = write::decode_prefix(node.raw(), SAMPLE_STRING_BYTES);
            Value::String(if cut { format!("{text}…") } else { text })
        }
        _ => node.scalar().unwrap_or(Value::Null),
    }
}

/// What a value is, from its first byte.
fn kind_of(first: u8) -> Option<ValueKind> {
    Some(match first {
        b'"' => ValueKind::String,
        b'[' => ValueKind::Array,
        b'{' => ValueKind::Object,
        b't' | b'f' => ValueKind::Bool,
        b'n' => ValueKind::Null,
        b'-' | b'0'..=b'9' => ValueKind::Number,
        _ => return None,
    })
}

impl<'a> ValueView for Node<'a> {
    fn kind(&self) -> ValueKind {
        self.kind
    }

    fn child_count(&self) -> usize {
        Node::child_count(self)
    }

    fn child_by_key(&self, key: &str) -> Option<Self> {
        Node::child_by_key(self, key).map(|child| child.node)
    }

    fn child_at(&self, index: usize) -> Option<Self> {
        if self.kind != ValueKind::Array {
            return None;
        }
        Node::child(self, index).map(|child| child.node)
    }

    fn iter_children(&self) -> Box<dyn Iterator<Item = (Option<PathSegment>, Self)> + '_> {
        Box::new(
            self.children()
                .map(|child| (Some(child.segment()), child.node)),
        )
    }

    fn iter_children_from(
        &self,
        start: usize,
    ) -> Box<dyn Iterator<Item = (Option<PathSegment>, Self)> + '_> {
        Box::new(
            self.children_from(start)
                .map(|child| (Some(child.segment()), child.node)),
        )
    }

    fn scalar_value(&self) -> Option<Value> {
        self.scalar()
    }

    fn scalar_preview(&self) -> Option<String> {
        write::scalar_preview(self)
    }
}

#[cfg(test)]
mod tests;
