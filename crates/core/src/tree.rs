//! Data-layer support for the virtualized tree widget (Architecture §6).
//!
//! Expand/collapse state lives outside the tree (keyed by [`NodePath`]) so it
//! survives a document/results swap. [`flatten_visible`] turns a `Value` +
//! expand-state into a flat list of [`RowInfo`] — one entry per *visible*
//! row — which the GUI layer then slices by scroll position and draws. This
//! is recomputed only when the data or expand-state actually changes, not
//! per frame; per-frame cost in the GUI is bounded by viewport height, not by
//! how many rows are in this list.

use std::collections::HashSet;

use serde_json::Value;

/// One step of a path into a JSON document: an object key or an array index.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

/// A path from the document root to a node, e.g. `.foo[3].bar`.
pub type NodePath = Vec<PathSegment>;

/// The set of container paths that are currently expanded in a tree widget.
///
/// The root (empty path) is just another entry here rather than a hard-coded
/// default, so it can be toggled like any other row; callers that want a
/// freshly loaded document to start expanded should seed this with `vec![]`
/// (see `new_expanded_at_root`).
pub type ExpandState = HashSet<NodePath>;

/// An [`ExpandState`] with the root pre-expanded, for a freshly loaded
/// document or result set.
pub fn new_expanded_at_root() -> ExpandState {
    let mut s = ExpandState::new();
    s.insert(Vec::new());
    s
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValueKind {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}

impl ValueKind {
    pub fn of(v: &Value) -> Self {
        match v {
            Value::Null => ValueKind::Null,
            Value::Bool(_) => ValueKind::Bool,
            Value::Number(_) => ValueKind::Number,
            Value::String(_) => ValueKind::String,
            Value::Array(_) => ValueKind::Array,
            Value::Object(_) => ValueKind::Object,
        }
    }

    pub fn is_container(self) -> bool {
        matches!(self, ValueKind::Array | ValueKind::Object)
    }
}

/// One visible row of a rendered tree: enough to draw the row without
/// re-walking the document, and enough to resolve back to the full value
/// on demand (click to expand, copy, etc).
pub struct RowInfo {
    pub path: NodePath,
    pub depth: usize,
    /// This row's key/index relative to its parent; `None` for the root.
    pub key: Option<PathSegment>,
    pub kind: ValueKind,
    /// Direct child count for containers; 0 for scalars.
    pub child_count: usize,
    /// Pre-rendered display text for scalar rows.
    pub scalar_preview: Option<String>,
    /// Whether this row is a container that is currently expanded.
    pub expanded: bool,
}

fn is_expanded(path: &NodePath, expand: &ExpandState) -> bool {
    expand.contains(path)
}

/// Flatten `root` into the list of currently-visible rows, given `expand`.
/// Collapsed subtrees are skipped entirely (not walked), not just hidden.
pub fn flatten_visible(root: &Value, expand: &ExpandState) -> Vec<RowInfo> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    push_node(root, None, &mut path, 0, expand, &mut out);
    out
}

fn push_node(
    value: &Value,
    key: Option<PathSegment>,
    path: &mut NodePath,
    depth: usize,
    expand: &ExpandState,
    out: &mut Vec<RowInfo>,
) {
    let kind = ValueKind::of(value);
    let child_count = match value {
        Value::Array(a) => a.len(),
        Value::Object(o) => o.len(),
        _ => 0,
    };
    let expanded = kind.is_container() && is_expanded(path, expand);

    out.push(RowInfo {
        path: path.clone(),
        depth,
        key: key.clone(),
        kind,
        child_count,
        scalar_preview: (!kind.is_container()).then(|| scalar_preview(value)),
        expanded,
    });

    if !expanded {
        return;
    }

    match value {
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                path.push(PathSegment::Index(i));
                push_node(
                    child,
                    Some(PathSegment::Index(i)),
                    path,
                    depth + 1,
                    expand,
                    out,
                );
                path.pop();
            }
        }
        Value::Object(map) => {
            for (k, child) in map.iter() {
                path.push(PathSegment::Key(k.clone()));
                push_node(
                    child,
                    Some(PathSegment::Key(k.clone())),
                    path,
                    depth + 1,
                    expand,
                    out,
                );
                path.pop();
            }
        }
        _ => {}
    }
}

fn scalar_preview(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("{s:?}"),
        _ => unreachable!("scalar_preview called on a container value"),
    }
}

/// Resolve a node path back to its value, e.g. for a "copy value" action.
pub fn resolve<'a>(root: &'a Value, path: &NodePath) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path {
        cur = match (seg, cur) {
            (PathSegment::Key(k), Value::Object(o)) => o.get(k)?,
            (PathSegment::Index(i), Value::Array(a)) => a.get(*i)?,
            _ => return None,
        };
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collapsed_root_shows_only_itself() {
        let v = json!({"a": [1, 2], "b": "x"});
        let expand = ExpandState::new();
        let rows = flatten_visible(&v, &expand);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].expanded);
    }

    #[test]
    fn seeded_root_expands_children_by_default() {
        let v = json!({"a": [1, 2], "b": "x"});
        let expand = new_expanded_at_root();
        let rows = flatten_visible(&v, &expand);
        // root + "a" + "b" — "a"'s children stay collapsed.
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[0].kind, ValueKind::Object);
        assert!(rows[0].expanded);
    }

    #[test]
    fn expanding_a_path_reveals_its_children_only() {
        let v = json!({"a": [1, 2], "b": {"c": 3}});
        let mut expand = new_expanded_at_root();
        expand.insert(vec![PathSegment::Key("a".into())]);
        let rows = flatten_visible(&v, &expand);
        // root, a, a[0], a[1], b  (b's child "c" stays collapsed)
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows[2].path,
            vec![PathSegment::Key("a".into()), PathSegment::Index(0)]
        );
    }

    #[test]
    fn resolve_walks_a_path() {
        let v = json!({"a": [10, 20]});
        let path = vec![PathSegment::Key("a".into()), PathSegment::Index(1)];
        assert_eq!(resolve(&v, &path), Some(&json!(20)));
    }
}
