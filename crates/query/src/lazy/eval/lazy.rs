//! What a value is while it is still in its file: a node, a part of a list, or a
//! list that has been put in another order — and how much of it may be made a
//! value.

use std::ops::Range;
use std::sync::Arc;

use jsonquery_core::lazy::Node;
use jsonquery_core::ValueKind;
use serde_json::Value;

use super::{bytes_text, Evaluator, Failure, Item};

/// Where an element of a list is in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub(super) fn of(node: &Node<'_>) -> Self {
        let range = node.byte_range();
        Self {
            start: range.start,
            end: range.end,
        }
    }

    pub(super) fn bytes(self) -> usize {
        self.end - self.start
    }
}

/// Where an element of a list is: its place in the file, or, for a list of
/// groups, which of them it is.
#[derive(Clone, Copy, Debug)]
pub(super) enum Place {
    Node(Span),
    Group(usize),
}

/// How a value that is still in the file is made up.
#[derive(Clone)]
enum Shape {
    /// The node itself.
    Whole,
    /// A run of the elements of the list that the node is.
    Window(Range<usize>),
    /// Some of the elements of a list, in an order of their own (a list that was
    /// sorted, or reversed, or that is one group of what `group_by` made): the
    /// elements are the ones at these places in the file.
    Picked {
        spans: Arc<Vec<Span>>,
        range: Range<usize>,
    },
    /// Lists of those: what `group_by` makes. Group `n` is the elements from
    /// the end of group `n - 1` to `ends[n]`.
    Groups {
        spans: Arc<Vec<Span>>,
        ends: Arc<Vec<usize>>,
        range: Range<usize>,
    },
}

/// A value of the document that has not been made a value: it is a node of the
/// file or some of the elements of one, which are read when they are asked for.
#[derive(Clone)]
pub(in crate::lazy) struct Lazy<'t> {
    /// The node, or for what is not one the list whose elements these are.
    node: Node<'t>,
    shape: Shape,
}

impl<'t> Lazy<'t> {
    pub(in crate::lazy) fn of(node: Node<'t>) -> Self {
        Self {
            node,
            shape: Shape::Whole,
        }
    }

    /// The elements of the list `node` that are at `spans`, in that order.
    pub(super) fn picked(node: Node<'t>, spans: Vec<Span>) -> Self {
        let range = 0..spans.len();
        Self {
            node,
            shape: Shape::Picked {
                spans: Arc::new(spans),
                range,
            },
        }
    }

    /// Lists of the elements at `spans`, the first ending at `ends[0]`, the next
    /// at `ends[1]`, and so on.
    pub(super) fn groups(node: Node<'t>, spans: Vec<Span>, ends: Vec<usize>) -> Self {
        let range = 0..ends.len();
        Self {
            node,
            shape: Shape::Groups {
                spans: Arc::new(spans),
                ends: Arc::new(ends),
                range,
            },
        }
    }

    pub(super) fn kind(&self) -> ValueKind {
        match self.shape {
            Shape::Whole => self.node.kind(),
            _ => ValueKind::Array,
        }
    }

    /// Whether it is a node, and not a part of a list or a list made of those.
    pub(super) fn is_whole(&self) -> bool {
        matches!(self.shape, Shape::Whole)
    }

    /// The node, for what is one.
    pub(super) fn node(&self) -> Node<'t> {
        self.node
    }

    /// How many elements a list has, or children an object.
    pub(super) fn len(&self) -> usize {
        match &self.shape {
            Shape::Whole => self.node.child_count(),
            Shape::Window(range) => range.len(),
            Shape::Picked { range, .. } | Shape::Groups { range, .. } => range.len(),
        }
    }

    /// The element of the file at `span`.
    fn element(&self, span: Span) -> Option<Lazy<'t>> {
        self.node.tree().node_at(span.start..span.end).map(Lazy::of)
    }

    /// The `at`th element of a list.
    pub(super) fn item(&self, at: usize) -> Option<Lazy<'t>> {
        match &self.shape {
            Shape::Whole => {
                if self.node.kind() != ValueKind::Array {
                    return None;
                }
                self.node.child(at).map(|child| Lazy::of(child.node))
            }
            Shape::Window(range) => {
                if at >= range.len() {
                    return None;
                }
                self.node
                    .child(range.start + at)
                    .map(|child| Lazy::of(child.node))
            }
            Shape::Picked { spans, range } => {
                if at >= range.len() {
                    return None;
                }
                self.element(*spans.get(range.start + at)?)
            }
            Shape::Groups { range, .. } => {
                if at >= range.len() {
                    return None;
                }
                Some(self.group(range.start + at))
            }
        }
    }

    /// Group `n` of a list of groups.
    fn group(&self, n: usize) -> Lazy<'t> {
        let Shape::Groups { spans, ends, .. } = &self.shape else {
            return self.clone();
        };
        let start = if n == 0 { 0 } else { ends[n - 1] };
        Lazy {
            node: self.node,
            shape: Shape::Picked {
                spans: spans.clone(),
                range: start..ends[n],
            },
        }
    }

    /// The elements of a list, or the values of an object, from the `skip`th on.
    pub(super) fn items_from(&self, skip: usize) -> Box<dyn Iterator<Item = Lazy<'t>> + 't> {
        match &self.shape {
            Shape::Whole => Box::new(
                self.node
                    .children_from(skip)
                    .map(|child| Lazy::of(child.node)),
            ),
            Shape::Window(range) => Box::new(
                self.node
                    .children_from(range.start + skip)
                    .take(range.len().saturating_sub(skip))
                    .map(|child| Lazy::of(child.node)),
            ),
            Shape::Picked { spans, range } => {
                let (spans, node) = (spans.clone(), self.node);
                Box::new(
                    (range.start + skip..range.end)
                        .filter_map(move |at| node.tree().node_at(spans[at].start..spans[at].end))
                        .map(Lazy::of),
                )
            }
            Shape::Groups { range, .. } => {
                let this = self.clone();
                Box::new((range.start + skip..range.end).map(move |n| this.group(n)))
            }
        }
    }

    /// Whether it is a list of groups (what `group_by` makes).
    pub(super) fn is_groups(&self) -> bool {
        matches!(self.shape, Shape::Groups { .. })
    }

    /// Where each element of the list is, in the order of the list.
    pub(super) fn places(&self) -> Vec<Place> {
        match &self.shape {
            Shape::Groups { range, .. } => (0..range.len()).map(Place::Group).collect(),
            _ => self
                .items_from(0)
                .map(|item| Place::Node(Span::of(&item.node)))
                .collect(),
        }
    }

    /// The element of the list at `place`.
    pub(super) fn at_place(&self, place: Place) -> Option<Lazy<'t>> {
        match place {
            Place::Node(span) => self.element(span),
            Place::Group(at) => self.item(at),
        }
    }

    /// A list of the elements of this one that are at `places`, in that order.
    pub(super) fn reordered(&self, places: Vec<Place>) -> Lazy<'t> {
        match &self.shape {
            Shape::Groups { spans, ends, range } => {
                let mut kept = Vec::new();
                let mut kept_ends = Vec::with_capacity(places.len());
                for place in places {
                    if let Place::Group(at) = place {
                        let n = range.start + at;
                        let start = if n == 0 { 0 } else { ends[n - 1] };
                        kept.extend_from_slice(&spans[start..ends[n]]);
                        kept_ends.push(kept.len());
                    }
                }
                Lazy::groups(self.node, kept, kept_ends)
            }
            _ => Lazy::picked(
                self.node,
                places
                    .into_iter()
                    .filter_map(|place| match place {
                        Place::Node(span) => Some(span),
                        Place::Group(_) => None,
                    })
                    .collect(),
            ),
        }
    }

    /// The elements `from..to` of a list (`from <= to <= len`).
    pub(super) fn slice(&self, from: usize, to: usize) -> Lazy<'t> {
        let shape = match &self.shape {
            Shape::Whole => Shape::Window(from..to),
            Shape::Window(range) => Shape::Window(range.start + from..range.start + to),
            Shape::Picked { spans, range } => Shape::Picked {
                spans: spans.clone(),
                range: range.start + from..range.start + to,
            },
            Shape::Groups { spans, ends, range } => Shape::Groups {
                spans: spans.clone(),
                ends: ends.clone(),
                range: range.start + from..range.start + to,
            },
        };
        Lazy {
            node: self.node,
            shape,
        }
    }

    /// How many bytes of the file it is, a byte more for each element of a list
    /// (the comma), counted only so far as `limit`: the count is over it if it
    /// is more.
    pub(super) fn bytes_up_to(&self, limit: usize) -> usize {
        match &self.shape {
            Shape::Whole => self.node.byte_len(),
            Shape::Window(range) => {
                let mut total = 0usize;
                for child in self.node.children_from(range.start).take(range.len()) {
                    total = total.saturating_add(child.node.byte_len() + 1);
                    if total > limit {
                        break;
                    }
                }
                total
            }
            Shape::Picked { spans, range } => {
                let mut total = 0usize;
                for span in &spans[range.clone()] {
                    total = total.saturating_add(span.bytes() + 1);
                    if total > limit {
                        break;
                    }
                }
                total
            }
            Shape::Groups { range, .. } => {
                let mut total = 0usize;
                for n in range.clone() {
                    total =
                        total.saturating_add(self.group(n).bytes_up_to(limit - total.min(limit)));
                    if total > limit {
                        break;
                    }
                }
                total
            }
        }
    }
}

impl Evaluator<'_> {
    // ---- what is small enough to be a value ----------------------------------

    pub(super) fn is_small(&self, lazy: &Lazy<'_>) -> bool {
        let limit = if lazy.is_whole() && !lazy.kind().is_container() {
            self.limits.scalar_bytes
        } else {
            self.limits.materialize_bytes
        };
        self.fits(lazy, limit)
    }

    /// Whether `lazy` takes `max` bytes of the file or fewer.
    pub(super) fn fits(&self, lazy: &Lazy<'_>, max: usize) -> bool {
        lazy.bytes_up_to(max) <= max
    }

    /// The value of `lazy`, if it takes no more than `max` bytes of the file.
    pub(super) fn materialize(&self, lazy: &Lazy<'_>, max: usize) -> Result<Value, String> {
        if lazy.is_whole() {
            return lazy
                .node
                .to_value(max)
                .map_err(|e| format!("{}: {e}", self.describe(lazy)));
        }
        if max != usize::MAX && !self.fits(lazy, max) {
            return Err(format!(
                "{} is too big to hold in memory",
                self.describe(lazy)
            ));
        }
        let mut items = Vec::with_capacity(lazy.len());
        for item in lazy.items_from(0) {
            items.push(self.materialize(&item, max)?);
        }
        Ok(Value::Array(items))
    }

    /// An item as a value, to be shown or gathered.
    pub(super) fn to_value(&self, item: Item<'_>) -> Result<Value, String> {
        match item {
            Item::Value(value) => Ok(value),
            Item::Lazy(lazy) => {
                if !self.fits(&lazy, self.limits.result_bytes) {
                    return Err(format!(
                        "{} is too big to show (over {}): narrow it with a slice such as \
                         .[0:100], or look at it in the Source tree",
                        self.describe(&lazy),
                        bytes_text(self.limits.result_bytes)
                    ));
                }
                self.materialize(&lazy, usize::MAX)
            }
        }
    }

    /// An item as a value that is a part of an expression.
    pub(super) fn part(&self, item: Item<'_>) -> Result<Value, Failure> {
        match item {
            Item::Value(value) => Ok(value),
            Item::Lazy(lazy) => {
                if !self.fits(&lazy, self.limits.result_bytes) {
                    return Err(Failure::Refusal(format!(
                        "{} is too big to be a part of an expression (over {}): narrow it \
                         with a slice such as .[0:100]",
                        self.describe(&lazy),
                        bytes_text(self.limits.result_bytes)
                    )));
                }
                self.materialize(&lazy, usize::MAX).map_err(Failure::Error)
            }
        }
    }

    /// What `lazy` is, in a few words, for a message.
    pub(super) fn describe(&self, lazy: &Lazy<'_>) -> String {
        let bytes = bytes_text(lazy.bytes_up_to(usize::MAX));
        match lazy.kind() {
            ValueKind::Array => format!("an array of {} items ({bytes})", lazy.len()),
            ValueKind::Object => format!("an object of {} keys ({bytes})", lazy.len()),
            ValueKind::String => format!("a string of {bytes}"),
            ValueKind::Number => "a number".to_owned(),
            ValueKind::Bool => "a boolean".to_owned(),
            ValueKind::Null => "null".to_owned(),
        }
    }

    /// `lazy` as jaq writes a value into an error message: all of it when it is
    /// small, a few words on it when it is not.
    pub(super) fn shown(&self, lazy: &Lazy<'_>) -> String {
        if self.is_small(lazy) {
            if let Ok(value) = self.materialize(lazy, usize::MAX) {
                return crate::to_val(&value).to_string();
            }
        }
        self.describe(lazy)
    }
}
