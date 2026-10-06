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

/// How much of a string a row shows, in bytes: a row is a line, and drawing a
/// string of a few megabytes as one asks the graphics card for more than it will
/// give (the window closes) for nothing anyone could read.
pub const PREVIEW_BYTES: usize = 1024;

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

    /// Every direct child from the `start`th on, as [`iter_children`] gives
    /// them. A node that can find its `start`th child without going past the
    /// ones before it should say so: this is how a tree shows a window onto a
    /// list of millions.
    ///
    /// [`iter_children`]: ValueView::iter_children
    fn iter_children_from(
        &self,
        start: usize,
    ) -> Box<dyn Iterator<Item = (Option<PathSegment>, Self)> + '_>
    where
        Self: Sized,
    {
        Box::new(self.iter_children().skip(start))
    }

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

    fn iter_children_from(
        &self,
        start: usize,
    ) -> Box<dyn Iterator<Item = (Option<PathSegment>, Self)> + '_> {
        // `enumerate().skip()` over a slice goes straight to the `start`th.
        match *self {
            Value::Array(items) => Box::new(
                items
                    .iter()
                    .enumerate()
                    .skip(start)
                    .map(|(i, child)| (Some(PathSegment::Index(i)), child)),
            ),
            Value::Object(map) => Box::new(
                map.iter()
                    .skip(start)
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
            Value::String(s) if s.len() > PREVIEW_BYTES => {
                // Cut where a character is not.
                let mut end = PREVIEW_BYTES;
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                Some(format!("{:?}…", &s[..end]))
            }
            Value::String(s) => Some(format!("{s:?}")),
            Value::Array(_) | Value::Object(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_short_string_is_shown_whole() {
        let v = json!("hello \"world\"\n");
        assert_eq!(
            (&v).scalar_preview().as_deref(),
            Some("\"hello \\\"world\\\"\\n\"")
        );
        let edge = json!("x".repeat(PREVIEW_BYTES));
        assert!((&edge).scalar_preview().unwrap().ends_with("x\""));
    }

    #[test]
    fn a_long_string_is_shown_by_its_beginning() {
        let v = json!(format!("item-{}", "x".repeat(5_000_000)));
        let preview = (&v).scalar_preview().unwrap();
        assert!(preview.starts_with("\"item-xxx") && preview.ends_with("x\"…"));
        assert!(preview.len() <= PREVIEW_BYTES + 8, "{}", preview.len());
    }

    #[test]
    fn a_long_string_is_not_cut_in_the_middle_of_a_character() {
        // Two-byte characters, so that the limit falls inside one when it is odd.
        let v = json!("é".repeat(2_000));
        let preview = (&v).scalar_preview().unwrap();
        assert!(preview.ends_with("é\"…"), "{preview}");
        let v = json!(format!("a{}", "é".repeat(2_000)));
        assert!((&v).scalar_preview().unwrap().ends_with("é\"…"));
    }
}
