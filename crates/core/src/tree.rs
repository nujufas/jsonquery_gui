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

/// Where a results row's value probably came from in the source document —
/// what [`locate`] returns.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SourceMatches {
    /// Candidate locations, best guess first; empty if nothing was found.
    pub paths: Vec<NodePath>,
    /// `None` when every path holds a value equal to the row's. `Some(text)`
    /// when no node did — the value was presumably computed — and `paths` are
    /// instead the hits of a plain [`search`] for `text`.
    pub searched_for: Option<String>,
}

/// Locate a query result back in the source document, for the results
/// panel's "Find in Source". `target` is the clicked row's value; the row sits
/// at `rel` (its own key/index path) below the `nth` item of the query's
/// result stream. `nth` is a position in the results, not in the source, so
/// like `rel` it is only a hint for ordering candidates (see [`rank`]).
///
/// Most queries (selection, filtering, field access) pass values through
/// unchanged, without the query engine having to track provenance for each
/// result, so:
///
/// 1. Look for every node equal to `target` — key and value together: the
///    ones under the same trailing keys/indices as the row are preferred
///    ([`rank`]), so a repeated value is told apart by its key, and an array
///    element by its index.
/// 2. A value the query synthesized (`add`, string interpolation, ...) equals
///    nothing, so the fallback is a plain text [`search`] — for the value if
///    it's a string (it may have been lower-cased, trimmed, sliced, ...), else
///    for the row's key — ranked the same way.
///
/// If both find nothing there is no single "source location" to point at, and
/// an empty result is the honest outcome.
pub fn locate<V: ValueView + Clone>(
    root: V,
    target: &Value,
    nth: usize,
    rel: &[PathSegment],
) -> SourceMatches {
    let mut exact = Vec::new();
    collect_equal(root.clone(), target, &mut Vec::new(), &mut exact);
    if !exact.is_empty() {
        return SourceMatches {
            paths: rank(exact, rel, nth),
            searched_for: None,
        };
    }

    let string_text = match target {
        Value::String(s) => Some(s.trim()),
        _ => None,
    };
    let key_text = match rel.last() {
        Some(PathSegment::Key(k)) => Some(k.as_str()),
        _ => None,
    };
    for text in string_text
        .into_iter()
        .chain(key_text)
        .filter(|t| !t.is_empty())
    {
        // A plain (non-regex) search has no way to fail.
        let hits = search(root.clone(), text, false).unwrap_or_default();
        if !hits.is_empty() {
            return SourceMatches {
                paths: rank(hits, rel, nth),
                searched_for: Some(text.to_string()),
            };
        }
    }
    SourceMatches::default()
}

/// Every node in `root` structurally equal to `target`, depth-first
/// pre-order, capped at [`MAX_SEARCH_MATCHES`].
fn collect_equal<V: ValueView>(
    value: V,
    target: &Value,
    path: &mut NodePath,
    out: &mut Vec<NodePath>,
) {
    if out.len() >= MAX_SEARCH_MATCHES {
        return;
    }
    if structurally_equal(&value, target) {
        // Nothing inside a node equal to `target` can itself equal `target`,
        // so there's no point walking down into it.
        out.push(path.clone());
        return;
    }
    for (child_key, child) in value.iter_children() {
        let child_key = child_key.expect("iter_children yields a key/index for every child");
        path.push(child_key);
        collect_equal(child, target, path, out);
        path.pop();
    }
}

/// Order candidate source locations for one results row, best guess first.
///
/// A candidate that shares more trailing segments with the row's own path
/// (`rel`, below its query output) is better — for a row that is element 2 of
/// a `tags` array, `.tags[2]` beats `.tags[0]` — and only the best-sharing
/// group is kept, so an equal value under a different key drops out whenever
/// one under the same key exists. Within that group a candidate whose
/// innermost array index (above the shared segments) is `nth`, the query
/// output's own position, comes first: the n-th output of a `.users[]`-style
/// iteration is usually `users[n]`. Ties keep document order.
fn rank(paths: Vec<NodePath>, rel: &[PathSegment], nth: usize) -> Vec<NodePath> {
    let shared = |p: &NodePath| {
        p.iter()
            .rev()
            .zip(rel.iter().rev())
            .take_while(|(a, b)| a == b)
            .count()
    };
    let best = paths.iter().map(shared).max().unwrap_or(0);

    let mut ranked: Vec<(bool, NodePath)> = paths
        .into_iter()
        .filter(|p| shared(p) == best)
        .map(|p| {
            let innermost_index = p[..p.len() - best].iter().rev().find_map(|seg| match seg {
                PathSegment::Index(i) => Some(*i),
                PathSegment::Key(_) => None,
            });
            (innermost_index == Some(nth), p)
        })
        .collect();
    // Stable, so document order survives within each half.
    ranked.sort_by_key(|(same_position, _)| !same_position);
    ranked.into_iter().map(|(_, p)| p).collect()
}

/// Whether `value` and `target` describe the same JSON structure — the same
/// equality `serde_json::Value`'s own `PartialEq` gives two owned `Value`s
/// (arrays compare length and order; objects compare as a set of key/value
/// pairs, order-independent), but computed one child at a time through
/// [`ValueView`] instead of requiring `value` to already be a `Value`.
fn structurally_equal<V: ValueView>(value: &V, target: &Value) -> bool {
    // Cheap rejection first: `locate` runs this on every node of the document,
    // and the scalar arm below clones the node's value.
    if value.kind() != ValueKind::of(target) {
        return false;
    }
    match target {
        Value::Array(items) => {
            value.child_count() == items.len()
                && items.iter().enumerate().all(|(i, item)| {
                    value
                        .child_at(i)
                        .is_some_and(|c| structurally_equal(&c, item))
                })
        }
        Value::Object(map) => {
            value.child_count() == map.len()
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

    fn key(k: &str) -> PathSegment {
        PathSegment::Key(k.into())
    }

    fn idx(i: usize) -> PathSegment {
        PathSegment::Index(i)
    }

    #[test]
    fn locate_finds_an_equal_container() {
        let v = json!({"a": [{"name": "Ada"}, {"name": "Alan"}]});
        let found = locate(&v, &json!({"name": "Alan"}), 0, &[]);
        assert_eq!(found.paths, vec![vec![key("a"), idx(1)]]);
        assert_eq!(found.searched_for, None);
        assert_eq!(resolve(&v, &found.paths[0]), Some(&json!({"name": "Alan"})));
    }

    #[test]
    fn locate_returns_nothing_for_a_value_not_present() {
        let v = json!({"a": 1});
        assert_eq!(locate(&v, &json!("nope"), 0, &[]), SourceMatches::default());
        // A computed number has no text worth searching for, and no key.
        assert_eq!(locate(&v, &json!(31), 0, &[]), SourceMatches::default());
    }

    #[test]
    fn locate_lists_every_equal_value_in_document_order() {
        let v = json!({"a": {"city": "Paris"}, "b": [{"city": "Paris"}, {"city": "Rome"}]});
        // Output 9: no candidate sits at array index 9, so nothing is promoted.
        let found = locate(&v, &json!("Paris"), 9, &[]);
        assert_eq!(
            found.paths,
            vec![
                vec![key("a"), key("city")],
                vec![key("b"), idx(0), key("city")]
            ]
        );
    }

    #[test]
    fn locate_puts_the_nth_element_first_for_the_nth_output() {
        let v = json!({"users": [
            {"country": "US"}, {"country": "UK"}, {"country": "US"}, {"country": "US"}
        ]});
        // `.users[].country` yields US, UK, US, US; the row is output 2.
        let found = locate(&v, &json!("US"), 2, &[]);
        assert_eq!(
            found.paths,
            vec![
                vec![key("users"), idx(2), key("country")],
                vec![key("users"), idx(0), key("country")],
                vec![key("users"), idx(3), key("country")],
            ]
        );
    }

    #[test]
    fn locate_prefers_nodes_under_the_same_key() {
        let v = json!({"a": {"name": "x"}, "b": {"alias": "x"}, "c": {"name": "x"}});
        // Row `.name` = "x": the `alias` node has the value but not the key.
        let found = locate(&v, &json!("x"), 0, &[key("name")]);
        assert_eq!(
            found.paths,
            vec![vec![key("a"), key("name")], vec![key("c"), key("name")]]
        );
        assert_eq!(found.searched_for, None);
    }

    #[test]
    fn locate_keeps_value_only_matches_when_no_node_shares_the_key() {
        let v = json!({"a": {"name": "x"}, "b": {"alias": "x"}});
        // The query renamed `name` to `n`.
        let found = locate(&v, &json!("x"), 0, &[key("n")]);
        assert_eq!(
            found.paths,
            vec![vec![key("a"), key("name")], vec![key("b"), key("alias")]]
        );
        assert_eq!(found.searched_for, None);
    }

    #[test]
    fn locate_matches_the_array_index_of_an_element_row() {
        let v = json!({"users": [{"tags": ["a", "b"]}, {"tags": ["b", "a"]}]});
        // Row `.tags[1]` = "b": `users[1].tags[0]` is also "b", but sits at
        // the wrong index.
        let found = locate(&v, &json!("b"), 0, &[key("tags"), idx(1)]);
        assert_eq!(
            found.paths,
            vec![vec![key("users"), idx(0), key("tags"), idx(1)]]
        );
    }

    #[test]
    fn locate_puts_the_nth_output_first_among_nodes_under_the_same_key() {
        let v = json!({
            "hq": {"code": "00100"},
            "depots": [{}, {}, {"code": "75001"}],
            "sites": [
                {"city": "Paris", "zip": "75001"},
                {"city": "Rome", "zip": "00100"},
                {"city": "Lyon", "zip": "75001"},
            ],
        });
        // Row `.zip` of output 2 of `.sites[]`. `depots[2].code` holds the
        // same value at the same index, but under another key, so it is out.
        let found = locate(&v, &json!("75001"), 2, &[key("zip")]);
        assert_eq!(
            found.paths,
            vec![
                vec![key("sites"), idx(2), key("zip")],
                vec![key("sites"), idx(0), key("zip")],
            ]
        );
        // Row `.zip` of output 0 of `.sites[1]`: `hq.code` is out too, which
        // leaves a single exact candidate.
        let found = locate(&v, &json!("00100"), 0, &[key("zip")]);
        assert_eq!(found.paths, vec![vec![key("sites"), idx(1), key("zip")]]);
    }

    #[test]
    fn locate_falls_back_to_a_text_search_for_a_transformed_string() {
        let v = json!({"name": "Alice", "other": 1});
        // `.name | ascii_downcase` -> "alice": no equal node, but the search
        // is case-insensitive.
        let found = locate(&v, &json!("alice"), 0, &[]);
        assert_eq!(found.paths, vec![vec![key("name")]]);
        assert_eq!(found.searched_for.as_deref(), Some("alice"));
    }

    #[test]
    fn locate_falls_back_to_the_key_when_the_string_is_nowhere() {
        let v = json!({"users": [{"name": "Ann"}, {"name": "Bob"}, {"name": "Cy"}]});
        // Row `.name` of output 1 of `.users[] | {name: (.name + "!")}`:
        // "Bob!" is computed, so the search falls through to the key `name` —
        // and the output's position puts `users[1].name` first.
        let found = locate(&v, &json!("Bob!"), 1, &[key("name")]);
        assert_eq!(found.searched_for.as_deref(), Some("name"));
        assert_eq!(
            found.paths,
            vec![
                vec![key("users"), idx(1), key("name")],
                vec![key("users"), idx(0), key("name")],
                vec![key("users"), idx(2), key("name")],
            ]
        );
    }

    #[test]
    fn locate_does_not_search_for_a_blank_string() {
        let v = json!({"a": "x"});
        assert_eq!(locate(&v, &json!("  "), 0, &[]), SourceMatches::default());
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
