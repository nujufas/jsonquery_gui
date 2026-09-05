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

use regex::Regex;
use serde_json::{Map, Value};

use crate::view::ValueView;

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
pub fn flatten_visible<V: ValueView>(root: V, expand: &ExpandState) -> Vec<RowInfo> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    push_node(root, None, &mut path, 0, expand, &mut out);
    out
}

fn push_node<V: ValueView>(
    value: V,
    key: Option<PathSegment>,
    path: &mut NodePath,
    depth: usize,
    expand: &ExpandState,
    out: &mut Vec<RowInfo>,
) {
    let kind = value.kind();
    let child_count = value.child_count();
    let expanded = kind.is_container() && is_expanded(path, expand);

    out.push(RowInfo {
        path: path.clone(),
        depth,
        key: key.clone(),
        kind,
        child_count,
        scalar_preview: value.scalar_preview(),
        expanded,
    });

    if !expanded {
        return;
    }

    for (child_key, child) in value.iter_children() {
        let child_key = child_key.expect("iter_children yields a key/index for every child");
        path.push(child_key.clone());
        push_node(child, Some(child_key), path, depth + 1, expand, out);
        path.pop();
    }
}

/// Resolve a node path back to its value, e.g. for a "copy value" action.
pub fn resolve<V: ValueView>(root: V, path: &NodePath) -> Option<V> {
    let mut cur = root;
    for seg in path {
        cur = match seg {
            PathSegment::Key(k) => cur.child_by_key(k)?,
            PathSegment::Index(i) => cur.child_at(*i)?,
        };
    }
    Some(cur)
}

/// Render a node path as a jq-style path expression, e.g. `.foo[3]["a-b"]` —
/// for the "Copy JSON Path" row action, so it can be pasted straight into the
/// query bar.
pub fn path_string(path: &[PathSegment]) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let mut s = String::new();
    for seg in path {
        match seg {
            PathSegment::Index(i) => {
                s.push('[');
                s.push_str(&i.to_string());
                s.push(']');
            }
            PathSegment::Key(k) if is_bare_ident(k) => {
                s.push('.');
                s.push_str(k);
            }
            PathSegment::Key(k) => {
                s.push('[');
                s.push_str(&serde_json::to_string(k).unwrap_or_else(|_| format!("{k:?}")));
                s.push(']');
            }
        }
    }
    if s.starts_with('.') {
        s
    } else {
        format!(".{s}")
    }
}

fn is_bare_ident(k: &str) -> bool {
    let mut chars = k.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Find the first node in `root` (in depth-first, pre-order traversal) whose
/// value is structurally equal to `target`, and return its path. This is a
/// best-effort way to locate a query result back in the source document:
/// most queries (selection, filtering, field access) pass matched values
/// through unchanged, so an exact-value search finds the right node without
/// the query engine needing to track provenance for every result. Values
/// synthesized by a query (`add`, string interpolation, computed numbers,
/// ...) generally won't be found — there's no single "source location" for
/// those, so no match is a reasonable, honest outcome.
pub fn find_path<V: ValueView>(root: V, target: &Value) -> Option<NodePath> {
    let mut path = Vec::new();
    find_path_rec(root, target, &mut path).then_some(path)
}

fn find_path_rec<V: ValueView>(value: V, target: &Value, path: &mut NodePath) -> bool {
    if structurally_equal(&value, target) {
        return true;
    }
    for (child_key, child) in value.iter_children() {
        let child_key = child_key.expect("iter_children yields a key/index for every child");
        path.push(child_key);
        if find_path_rec(child, target, path) {
            return true;
        }
        path.pop();
    }
    false
}

/// Whether `value` and `target` describe the same JSON structure — the same
/// equality `serde_json::Value`'s own `PartialEq` gives two owned `Value`s
/// (arrays compare length and order; objects compare as a set of key/value
/// pairs, order-independent), but computed one child at a time through
/// [`ValueView`] instead of requiring `value` to already be a `Value`.
fn structurally_equal<V: ValueView>(value: &V, target: &Value) -> bool {
    match target {
        Value::Array(items) => {
            value.kind() == ValueKind::Array
                && value.child_count() == items.len()
                && items.iter().enumerate().all(|(i, item)| {
                    value
                        .child_at(i)
                        .is_some_and(|c| structurally_equal(&c, item))
                })
        }
        Value::Object(map) => {
            value.kind() == ValueKind::Object
                && value.child_count() == map.len()
                && map.iter().all(|(k, v)| {
                    value
                        .child_by_key(k)
                        .is_some_and(|c| structurally_equal(&c, v))
                })
        }
        scalar => value.scalar_value().as_ref() == Some(scalar),
    }
}

/// Cap on how many hits [`search`] collects, protecting memory and the
/// results panel against a search that matches almost everything in a huge
/// document (e.g. an empty pattern).
const MAX_SEARCH_MATCHES: usize = 5_000;

/// Search `root` for every node whose key or scalar value contains `query`
/// (case-insensitively), or — if `use_regex` — matches it as a regular
/// expression, depth-first pre-order, capped at [`MAX_SEARCH_MATCHES`].
/// Backs the tree's "Search…" row-context-menu action, which lists every hit
/// in a Notepad++-style results panel rather than jumping to just one.
pub fn search<V: ValueView>(
    root: V,
    query: &str,
    use_regex: bool,
) -> anyhow::Result<Vec<NodePath>> {
    let is_match: Box<dyn Fn(&str) -> bool> = if use_regex {
        let re = Regex::new(query)?;
        Box::new(move |s: &str| re.is_match(s))
    } else {
        let needle = query.to_lowercase();
        Box::new(move |s: &str| s.to_lowercase().contains(&needle))
    };

    let mut out = Vec::new();
    let mut path = Vec::new();
    search_rec(root, None, is_match.as_ref(), &mut path, &mut out);
    Ok(out)
}

fn search_rec<V: ValueView>(
    value: V,
    key: Option<PathSegment>,
    is_match: &dyn Fn(&str) -> bool,
    path: &mut NodePath,
    out: &mut Vec<NodePath>,
) {
    if out.len() >= MAX_SEARCH_MATCHES {
        return;
    }

    let key_matches = matches!(&key, Some(PathSegment::Key(k)) if is_match(k));
    let value_matches = value
        .scalar_value()
        .is_some_and(|v| scalar_matches(&v, is_match));
    if key_matches || value_matches {
        out.push(path.clone());
    }

    for (child_key, child) in value.iter_children() {
        if out.len() >= MAX_SEARCH_MATCHES {
            break;
        }
        let child_key = child_key.expect("iter_children yields a key/index for every child");
        path.push(child_key.clone());
        search_rec(child, Some(child_key), is_match, path, out);
        path.pop();
    }
}

fn scalar_matches(v: &Value, is_match: &dyn Fn(&str) -> bool) -> bool {
    match v {
        Value::String(s) => is_match(s),
        Value::Number(n) => is_match(&n.to_string()),
        Value::Bool(b) => is_match(if *b { "true" } else { "false" }),
        Value::Null => is_match("null"),
        Value::Array(_) | Value::Object(_) => {
            unreachable!("scalar_value never returns a container")
        }
    }
}

/// Render `value` as indented JSON text (2-space indent, the same shape as
/// `serde_json::to_string_pretty`), but stop once `node_budget` nodes
/// (scalars, arrays, and objects each count as one) have been visited —
/// used by the app's "Text view" so previewing a huge document or a huge
/// query result (e.g. the single-item output of `.` over a multi-GB doc)
/// costs only as much as the budget, never the whole tree. Returns the
/// rendered text and whether it was cut short.
pub fn pretty_print_bounded(value: &Value, node_budget: usize) -> (String, bool) {
    let mut out = String::new();
    let mut budget = node_budget;
    let complete = write_node(value, 0, &mut out, &mut budget);
    (out, !complete)
}

/// Writes one node and returns whether the whole subtree was written; `false`
/// means the budget ran out somewhere inside it, so `out` holds a truncated
/// (not necessarily valid-JSON) prefix.
fn write_node(value: &Value, indent: usize, out: &mut String, budget: &mut usize) -> bool {
    if *budget == 0 {
        out.push('…');
        return false;
    }
    *budget -= 1;
    match value {
        Value::Null => {
            out.push_str("null");
            true
        }
        Value::Bool(b) => {
            out.push_str(if *b { "true" } else { "false" });
            true
        }
        Value::Number(n) => {
            out.push_str(&n.to_string());
            true
        }
        Value::String(s) => {
            out.push_str(&serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string()));
            true
        }
        Value::Array(items) => write_array(items, indent, out, budget),
        Value::Object(map) => write_object(map, indent, out, budget),
    }
}

fn write_array(items: &[Value], indent: usize, out: &mut String, budget: &mut usize) -> bool {
    if items.is_empty() {
        out.push_str("[]");
        return true;
    }
    out.push('[');
    let child_indent = indent + 1;
    let mut complete = true;
    for (i, item) in items.iter().enumerate() {
        out.push('\n');
        push_indent(out, child_indent);
        if !write_node(item, child_indent, out, budget) {
            complete = false;
            break;
        }
        if i + 1 < items.len() {
            out.push(',');
        }
    }
    out.push('\n');
    push_indent(out, indent);
    out.push(']');
    complete
}

fn write_object(
    map: &Map<String, Value>,
    indent: usize,
    out: &mut String,
    budget: &mut usize,
) -> bool {
    if map.is_empty() {
        out.push_str("{}");
        return true;
    }
    out.push('{');
    let child_indent = indent + 1;
    let mut complete = true;
    let len = map.len();
    for (i, (k, v)) in map.iter().enumerate() {
        out.push('\n');
        push_indent(out, child_indent);
        out.push_str(&serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_string()));
        out.push_str(": ");
        if !write_node(v, child_indent, out, budget) {
            complete = false;
            break;
        }
        if i + 1 < len {
            out.push(',');
        }
    }
    out.push('\n');
    push_indent(out, indent);
    out.push('}');
    complete
}

fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
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

    #[test]
    fn path_string_renders_bare_and_quoted_keys() {
        assert_eq!(path_string(&[]), ".");
        assert_eq!(path_string(&[PathSegment::Key("foo".into())]), ".foo");
        assert_eq!(
            path_string(&[PathSegment::Key("foo".into()), PathSegment::Index(3)]),
            ".foo[3]"
        );
        assert_eq!(path_string(&[PathSegment::Index(2)]), ".[2]");
        assert_eq!(path_string(&[PathSegment::Key("a-b".into())]), ".[\"a-b\"]");
    }

    #[test]
    fn find_path_locates_an_equal_value() {
        let v = json!({"a": [{"name": "Ada"}, {"name": "Alan"}]});
        let path = find_path(&v, &json!({"name": "Alan"})).unwrap();
        assert_eq!(
            path,
            vec![PathSegment::Key("a".into()), PathSegment::Index(1)]
        );
        assert_eq!(resolve(&v, &path), Some(&json!({"name": "Alan"})));
    }

    #[test]
    fn find_path_returns_none_for_a_value_not_present() {
        let v = json!({"a": 1});
        assert_eq!(find_path(&v, &json!("nope")), None);
    }

    #[test]
    fn search_matches_scalar_values_case_insensitively() {
        let v = json!({"a": [{"name": "Ada"}, {"name": "Alan"}]});
        let hits = search(&v, "ada", false).unwrap();
        assert_eq!(
            hits,
            vec![vec![
                PathSegment::Key("a".into()),
                PathSegment::Index(0),
                PathSegment::Key("name".into())
            ]]
        );
    }

    #[test]
    fn search_matches_keys_too() {
        let v = json!({"foo_bar": 1, "baz": 2});
        let hits = search(&v, "foo", false).unwrap();
        assert_eq!(hits, vec![vec![PathSegment::Key("foo_bar".into())]]);
    }

    #[test]
    fn search_regex_mode_matches_pattern() {
        let v = json!({"a": "id42", "b": "id-x"});
        let hits = search(&v, r"^id\d+$", true).unwrap();
        assert_eq!(hits, vec![vec![PathSegment::Key("a".into())]]);
    }

    #[test]
    fn search_invalid_regex_is_an_error() {
        let v = json!({"a": 1});
        assert!(search(&v, "(unclosed", true).is_err());
    }

    #[test]
    fn pretty_print_bounded_matches_serde_when_under_budget() {
        let v = json!({"a": [1, 2.5, "x", null, true], "b": {"c": -3}});
        let (text, truncated) = pretty_print_bounded(&v, 1_000);
        assert!(!truncated);
        assert_eq!(text, serde_json::to_string_pretty(&v).unwrap());
    }

    #[test]
    fn pretty_print_bounded_cuts_off_and_reports_truncation() {
        let v = json!([1, 2, 3, 4, 5]);
        // Budget covers the array itself plus its first two elements only.
        let (text, truncated) = pretty_print_bounded(&v, 3);
        assert!(truncated);
        assert!(text.contains('1'));
        assert!(text.contains('2'));
        assert!(!text.contains('3'));
    }

    #[test]
    fn pretty_print_bounded_empty_containers_never_truncate() {
        let v = json!({"a": [], "b": {}});
        // Exactly one budget unit per node: the root object, plus its two
        // (empty, childless) values.
        let (text, truncated) = pretty_print_bounded(&v, 3);
        assert!(!truncated);
        assert_eq!(text, serde_json::to_string_pretty(&v).unwrap());
    }
}
