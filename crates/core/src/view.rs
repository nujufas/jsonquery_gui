//! [`ValueView`]: the engine-agnostic lazy-resolution surface every consumer
//! of a document — the tree widget, search, a query engine adapter — should
//! program against, instead of matching on a concrete value type directly.
//!
//! Today there is exactly one implementor, `&Value` itself (this module),
//! which is a zero-cost pass-through to `serde_json::Value`'s own structure.
//! The point of factoring it out now, before there's a second implementor,
//! is that a future checkpoint-indexed, mmap-backed value (Architecture
//! §2-3's "Phase 2 — Scale") can implement the same trait and drop straight
//! into `crates/core::tree`'s existing algorithms — and into any future
//! query-language adapter — without either needing to change. Nothing here
//! is jaq-specific: this crate has no query-engine dependency at all.

use serde_json::Value;

use crate::tree::{PathSegment, ValueKind};

/// A JSON container/scalar node, resolvable one child at a time. `&Value`
/// implements this directly; a future lazily-resolved, checkpoint-indexed
/// value (backed by a memory-mapped file rather than a fully parsed
/// document) would implement it the same way, so callers never need to know
/// which kind of document they're holding.
pub trait ValueView {
    fn kind(&self) -> ValueKind;

    /// Direct child count for a container; 0 for a scalar.
    fn child_count(&self) -> usize;

    /// Look up a child by object key; `None` if this isn't an object or has
    /// no such key.
    fn child_by_key(&self, key: &str) -> Option<Self>
    where
        Self: Sized;

    /// Look up a child by array index; `None` if this isn't an array or the
    /// index is out of bounds.
    fn child_at(&self, index: usize) -> Option<Self>
    where
        Self: Sized;

    /// Every direct child, in order, paired with the key/index that reaches
    /// it from `self`. Always `Some(..)` for the paired segment — a node
    /// only ever appears here as somebody's child.
    fn iter_children(&self) -> Box<dyn Iterator<Item = (Option<PathSegment>, Self)> + '_>
    where
        Self: Sized;

    /// This node's value, if it's a scalar (anything but an array/object).
    /// Cheap even for a lazily-backed document — a scalar is always a leaf,
    /// so reading one never requires resolving a whole subtree.
    fn scalar_value(&self) -> Option<Value>;

    /// Pre-rendered display text for a scalar row (see `RowInfo`); `None`
    /// for a container.
    fn scalar_preview(&self) -> Option<String>;
}

impl ValueView for &Value {
    fn kind(&self) -> ValueKind {
        ValueKind::of(self)
    }

    fn child_count(&self) -> usize {
        match self {
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            _ => 0,
        }
    }

    fn child_by_key(&self, key: &str) -> Option<Self> {
        match self {
            Value::Object(o) => o.get(key),
            _ => None,
        }
    }

    fn child_at(&self, index: usize) -> Option<Self> {
        match self {
            Value::Array(a) => a.get(index),
            _ => None,
        }
    }

    fn iter_children(&self) -> Box<dyn Iterator<Item = (Option<PathSegment>, Self)> + '_> {
        match *self {
            Value::Array(items) => Box::new(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, child)| (Some(PathSegment::Index(i)), child)),
            ),
            Value::Object(map) => Box::new(
                map.iter()
                    .map(|(k, child)| (Some(PathSegment::Key(k.clone())), child)),
            ),
            _ => Box::new(std::iter::empty()),
        }
    }

    fn scalar_value(&self) -> Option<Value> {
        match self {
            Value::Array(_) | Value::Object(_) => None,
            scalar => Some((*scalar).clone()),
        }
    }

    fn scalar_preview(&self) -> Option<String> {
        match self {
            Value::Null => Some("null".to_string()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Number(n) => Some(n.to_string()),
            Value::String(s) => Some(format!("{s:?}")),
            Value::Array(_) | Value::Object(_) => None,
        }
    }
}
