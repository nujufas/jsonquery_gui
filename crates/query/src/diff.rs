//! Comparing two JSON documents — what the Tools window's Diff JSON shows.
//!
//! [`diff`] says what was added, removed and changed on the way from the first
//! document to the second, and as the RFC 6902 JSON Patch that does it (see
//! [`crate::patch`] for applying one).
//!
//! What counts as a difference is what JSON itself says: the members of an
//! object have no order, so a document with its keys sorted is the same
//! document, and numbers are compared by value (`1`, `1.0` and `10e-1` are one
//! number) with exact digits (two 30-digit integers that differ in the last
//! digit are different). Arrays are ordered, but they are *aligned* by content
//! before they are compared, so a value inserted at the start of a long array is
//! one addition rather than a change to every element after it.
//!
//! [`compare`] does the same and also lays the two documents out side by side,
//! line by line, with what differs marked — what a viewer like Beyond Compare
//! shows (see [`view`]).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Number, Value};

use crate::patch::escape;
use crate::reformat::{self, Indent};

mod view;

pub use view::{Block, Mark, Row, SideBySide, TooLong, MAX_ROWS};

/// One of the two documents of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The first document, `before`.
    Left,
    /// The second document, `after`.
    Right,
}

impl Side {
    pub fn other(self) -> Self {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// Changes kept in [`Diff::changes`] — the counts and the patch are complete
/// whatever this is; it only bounds what a window has to list.
pub const MAX_CHANGES: usize = 5_000;

/// Cells of the table that aligns two arrays (the product of their lengths once
/// what they share at both ends is set aside), for one array and for all the
/// arrays of one comparison. Past these the elements are paired by position —
/// still a correct patch, just not a minimal one.
const MAX_CELLS: u64 = 4_000_000;
const TOTAL_CELLS: u64 = 40_000_000;

/// The comparison was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// A value in the second document with nothing to match it in the first.
    Added,
    /// A value in the first document that is gone from the second.
    Removed,
    /// A value that is in both but not the same.
    Changed,
}

/// One difference, as a person reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    /// A JSON Pointer to the value: in the second document for an addition or
    /// a change, in the first for a removal. Empty when the whole document
    /// changed.
    pub path: String,
    /// A few words on the value before (for a removal or a change) and after
    /// (for an addition or a change) — see [`summarize`].
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Diff {
    /// The differences in document order, at most [`MAX_CHANGES`] of them.
    pub changes: Vec<Change>,
    /// How many of each there are in all (not just in `changes`).
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    /// The RFC 6902 operations that turn the first document into the second,
    /// when applied one after the other. There is one for every difference.
    pub operations: Vec<Value>,
}

impl Diff {
    pub fn total(&self) -> usize {
        self.added + self.removed + self.changed
    }

    /// The two documents are the same.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// The operations as a JSON Patch document.
    pub fn patch(&self) -> Value {
        Value::Array(self.operations.clone())
    }
}

/// Compare `before` with `after`. `cancel` is looked at now and then, so a
/// comparison of big arrays can be stopped from another thread.
pub fn diff(before: &Value, after: &Value, cancel: &AtomicBool) -> Result<Diff, Cancelled> {
    Ok(run(before, after, cancel)?.0)
}

/// What [`compare`] makes of two documents: the differences, and the documents
/// laid out side by side with the differences marked (see [`view`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub diff: Diff,
    pub view: Result<SideBySide, TooLong>,
}

/// [`diff`], and the two documents laid out side by side.
pub fn compare(
    before: &Value,
    after: &Value,
    cancel: &AtomicBool,
) -> Result<Comparison, Cancelled> {
    let (diff, edit) = run(before, after, cancel)?;
    let view = match view::build(before, after, edit.as_ref(), cancel) {
        Ok(view) => Ok(view),
        Err(view::Stop::TooLong) => Err(TooLong),
        Err(view::Stop::Cancelled) => return Err(Cancelled),
    };
    Ok(Comparison { diff, view })
}

/// The document `into` with some of the differences made the same as the other
/// document has them: a difference that is a value only the other document has
/// is put in, one that is a value only this one has is taken out, and one that is
/// a value that is not the same in both gets the other document's. Which
/// differences is said by number, as the rows of [`SideBySide`] carry them
/// ([`Row::change`]; [`SideBySide::changes_of`] gives those of a [`Block`]). The
/// documents must be the ones the view was made of.
///
/// This is moving a difference to the left (`into` is [`Side::Left`]: Left takes
/// what Right has) or to the right. The rest of the document is as it was,
/// including the order of an object's members and how its numbers are written.
pub fn take_changes(
    before: &Value,
    after: &Value,
    changes: Range<u32>,
    into: Side,
    cancel: &AtomicBool,
) -> Result<Value, Cancelled> {
    let (_, edit) = run(before, after, cancel)?;
    view::blend(before, after, edit.as_ref(), changes, into, cancel)
}

/// The differences, and the tree they were read from.
fn run<'a>(
    before: &'a Value,
    after: &'a Value,
    cancel: &AtomicBool,
) -> Result<(Diff, Option<Edit<'a>>), Cancelled> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled);
    }
    let mut cx = Cx {
        cancel,
        ticks: 0,
        cells_left: TOTAL_CELLS,
    };
    let mut result = Diff::default();
    let edit = edit(before, after, &mut cx)?;
    if let Some(edit) = &edit {
        changes(edit, &mut String::new(), &mut result);
        operations(edit, &mut String::new(), &mut result.operations);
    }
    Ok((result, edit))
}

/// Whether two documents are the same JSON: objects whatever the order of their
/// members, numbers by value.
pub fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => numbers_equal(x, y),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(key, p)| y.get(key).is_some_and(|q| equal(p, q)))
        }
        _ => false,
    }
}

fn numbers_equal(a: &Number, b: &Number) -> bool {
    let (x, y) = (a.to_string(), b.to_string());
    x == y || matches!((decimal(&x), decimal(&y)), (Some(p), Some(q)) if p == q)
}

/// A number in a normal form, so that the same value is the same whatever its
/// notation: the sign, the significant digits (no zero at either end) and the
/// power of ten they are multiplied by. Zero has no digits and no sign.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Decimal {
    negative: bool,
    digits: String,
    exponent: i64,
}

/// Read a JSON number's text. `None` for anything that isn't one, and for an
/// exponent too large to hold — the caller then compares the text.
fn decimal(text: &str) -> Option<Decimal> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (mantissa, exponent) = match rest.find(['e', 'E']) {
        Some(at) => (&rest[..at], rest[at + 1..].parse::<i64>().ok()?),
        None => (rest, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    if !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }

    let mut exponent = exponent.checked_sub(i64::try_from(frac.len()).ok()?)?;
    let all = format!("{int}{frac}");
    let significant = all.trim_start_matches('0');
    let digits = significant.trim_end_matches('0');
    if digits.is_empty() {
        return Some(Decimal {
            negative: false,
            digits: String::new(),
            exponent: 0,
        });
    }
    exponent = exponent.checked_add(i64::try_from(significant.len() - digits.len()).ok()?)?;
    Some(Decimal {
        negative,
        digits: digits.to_owned(),
        exponent,
    })
}

/// A hash that is the same for values that are [`equal`]: so it ignores the
/// order of an object's members and the notation of a number.
fn hash_value<H: Hasher>(value: &Value, state: &mut H) {
    match value {
        Value::Null => 0u8.hash(state),
        Value::Bool(b) => {
            1u8.hash(state);
            b.hash(state);
        }
        Value::Number(n) => {
            2u8.hash(state);
            let text = n.to_string();
            match decimal(&text) {
                Some(d) => d.hash(state),
                None => text.hash(state),
            }
        }
        Value::String(s) => {
            3u8.hash(state);
            s.hash(state);
        }
        Value::Array(items) => {
            4u8.hash(state);
            items.len().hash(state);
            for item in items {
                hash_value(item, state);
            }
        }
        Value::Object(members) => {
            5u8.hash(state);
            members.len().hash(state);
            // Added up, so that the order the members come in does not matter.
            let mut sum = 0u64;
            for (key, member) in members {
                let mut h = DefaultHasher::new();
                key.hash(&mut h);
                hash_value(member, &mut h);
                sum = sum.wrapping_add(h.finish());
            }
            sum.hash(state);
        }
    }
}

fn fingerprint(value: &Value) -> u64 {
    let mut h = DefaultHasher::new();
    hash_value(value, &mut h);
    h.finish()
}

// What differs, kept as a tree that borrows from the two documents. Two walks
// over it make the list of changes and the patch: they differ in how they name
// the elements of an array (see `Step`).

enum Edit<'a> {
    /// Replace the first value with the second (they are scalars, or of
    /// different kinds).
    Replace(&'a Value, &'a Value),
    /// Both are objects: the members that differ.
    Object(Vec<Member<'a>>),
    /// Both are arrays: what has to be done to the first to make the second.
    Array(Vec<Step<'a>>),
}

struct Member<'a> {
    key: &'a str,
    what: What<'a>,
}

enum What<'a> {
    Added(&'a Value),
    Removed(&'a Value),
    Changed(Edit<'a>),
}

/// One thing to do to the first array, in order of position. `at` is a place in
/// the *first* array (so a patch can be applied last to first, with every index
/// still pointing where it was computed), `index` one in the *second*.
enum Step<'a> {
    /// The element at `at` is not in the second array.
    Remove { at: usize, value: &'a Value },
    /// `value`, the element at `index` of the second array, goes in before the
    /// element at `at` of the first (`at` is the length when it goes last).
    Add {
        at: usize,
        index: usize,
        value: &'a Value,
    },
    /// The element at `at` of the first array is the one at `index` of the
    /// second, changed.
    Modify {
        at: usize,
        index: usize,
        edit: Edit<'a>,
    },
}

struct Cx<'c> {
    cancel: &'c AtomicBool,
    ticks: u32,
    cells_left: u64,
}

impl Cx<'_> {
    fn tick(&mut self) -> Result<(), Cancelled> {
        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks & 0xff == 0 && self.cancel.load(Ordering::Relaxed) {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

/// What turns `before` into `after`; `None` when they are the same.
fn edit<'a>(
    before: &'a Value,
    after: &'a Value,
    cx: &mut Cx<'_>,
) -> Result<Option<Edit<'a>>, Cancelled> {
    cx.tick()?;
    Ok(match (before, after) {
        (Value::Object(old), Value::Object(new)) => {
            let mut members = Vec::new();
            for (key, old_value) in old {
                match new.get(key) {
                    None => members.push(Member {
                        key,
                        what: What::Removed(old_value),
                    }),
                    Some(new_value) => {
                        if let Some(inner) = edit(old_value, new_value, cx)? {
                            members.push(Member {
                                key,
                                what: What::Changed(inner),
                            });
                        }
                    }
                }
            }
            for (key, new_value) in new {
                if !old.contains_key(key) {
                    members.push(Member {
                        key,
                        what: What::Added(new_value),
                    });
                }
            }
            (!members.is_empty()).then_some(Edit::Object(members))
        }
        (Value::Array(old), Value::Array(new)) => {
            let steps = array_steps(old, new, cx)?;
            (!steps.is_empty()).then_some(Edit::Array(steps))
        }
        _ => (!equal(before, after)).then_some(Edit::Replace(before, after)),
    })
}

/// What turns the array `old` into the array `new`.
fn array_steps<'a>(
    old: &'a [Value],
    new: &'a [Value],
    cx: &mut Cx<'_>,
) -> Result<Vec<Step<'a>>, Cancelled> {
    // What both start and end with needs nothing done.
    let prefix = old.iter().zip(new).take_while(|(p, q)| equal(p, q)).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(p, q)| equal(p, q))
        .count();
    let middle_old = &old[prefix..old.len() - suffix];
    let middle_new = &new[prefix..new.len() - suffix];

    // In between, the elements that stay, as places in the middles. The runs
    // of elements between them are what changed.
    let stay = common_elements(middle_old, middle_new, cx)?;
    let mut steps = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (stay_old, stay_new) in stay
        .into_iter()
        .chain(std::iter::once((middle_old.len(), middle_new.len())))
    {
        let removed = &middle_old[i..stay_old];
        let added = &middle_new[j..stay_new];

        // The first elements of each run are taken to be the same element,
        // changed; what is left over in the longer run was removed or added.
        let paired = removed.len().min(added.len());
        for (k, (old_value, new_value)) in removed.iter().zip(added).enumerate() {
            if let Some(inner) = edit(old_value, new_value, cx)? {
                steps.push(Step::Modify {
                    at: prefix + i + k,
                    index: prefix + j + k,
                    edit: inner,
                });
            }
        }
        for (k, value) in removed.iter().enumerate().skip(paired) {
            steps.push(Step::Remove {
                at: prefix + i + k,
                value,
            });
        }
        for (k, value) in added.iter().enumerate().skip(paired) {
            steps.push(Step::Add {
                at: prefix + stay_old,
                index: prefix + j + k,
                value,
            });
        }
        i = stay_old + 1;
        j = stay_new + 1;
    }
    Ok(steps)
}

/// Places `(i, j)` with `old[i]` equal to `new[j]`, rising in both, as many as
/// can be had (a longest common subsequence). When the arrays are too big to
/// align there are none, and the caller pairs elements by position.
fn common_elements(
    old: &[Value],
    new: &[Value],
    cx: &mut Cx<'_>,
) -> Result<Vec<(usize, usize)>, Cancelled> {
    if old.is_empty() || new.is_empty() {
        return Ok(Vec::new());
    }
    let cells = old.len() as u64 * new.len() as u64;
    if cells > MAX_CELLS || cells > cx.cells_left {
        return Ok(Vec::new());
    }
    cx.cells_left -= cells;

    let old_prints: Vec<u64> = old.iter().map(fingerprint).collect();
    let new_prints: Vec<u64> = new.iter().map(fingerprint).collect();
    let same = |i: usize, j: usize| old_prints[i] == new_prints[j] && equal(&old[i], &new[j]);

    // `table[i][j]` is the length of the longest run in common between
    // `old[i..]` and `new[j..]`.
    let width = new.len() + 1;
    let mut table = vec![0u32; (old.len() + 1) * width];
    for i in (0..old.len()).rev() {
        cx.tick()?;
        for j in (0..new.len()).rev() {
            table[i * width + j] = if same(i, j) {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }

    let mut stay = Vec::with_capacity(table[0] as usize);
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if same(i, j) {
            stay.push((i, j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    Ok(stay)
}

/// Add `edit`'s differences to `out.changes`, in document order. `path` is the
/// pointer to the value `edit` is about, and is left as it was found.
fn changes(edit: &Edit<'_>, path: &mut String, out: &mut Diff) {
    match edit {
        Edit::Replace(before, after) => {
            record(out, ChangeKind::Changed, path, Some(*before), Some(*after));
        }
        Edit::Object(members) => {
            for member in members {
                let keep = path.len();
                push_token(path, member.key);
                match &member.what {
                    What::Added(value) => record(out, ChangeKind::Added, path, None, Some(*value)),
                    What::Removed(value) => {
                        record(out, ChangeKind::Removed, path, Some(*value), None)
                    }
                    What::Changed(inner) => changes(inner, path, out),
                }
                path.truncate(keep);
            }
        }
        Edit::Array(steps) => {
            for step in steps {
                let keep = path.len();
                match step {
                    Step::Remove { at, value } => {
                        push_token(path, &at.to_string());
                        record(out, ChangeKind::Removed, path, Some(*value), None);
                    }
                    Step::Add { index, value, .. } => {
                        push_token(path, &index.to_string());
                        record(out, ChangeKind::Added, path, None, Some(*value));
                    }
                    Step::Modify { index, edit, .. } => {
                        push_token(path, &index.to_string());
                        changes(edit, path, out);
                    }
                }
                path.truncate(keep);
            }
        }
    }
}

fn record(
    out: &mut Diff,
    kind: ChangeKind,
    path: &str,
    before: Option<&Value>,
    after: Option<&Value>,
) {
    match kind {
        ChangeKind::Added => out.added += 1,
        ChangeKind::Removed => out.removed += 1,
        ChangeKind::Changed => out.changed += 1,
    }
    if out.changes.len() < MAX_CHANGES {
        out.changes.push(Change {
            kind,
            path: path.to_owned(),
            before: before.map(summarize),
            after: after.map(summarize),
        });
    }
}

/// Add `edit`'s differences to `out` as RFC 6902 operations that can be applied
/// in the order they are in. The elements of an array are dealt with from the
/// last to the first, so that what has been done to one never moves one that
/// is still to come: every index is a place in the array as it was first.
fn operations(edit: &Edit<'_>, path: &mut String, out: &mut Vec<Value>) {
    let op = |name: &str, path: &str, value: Option<&Value>| {
        let mut op = serde_json::Map::new();
        op.insert("op".to_owned(), Value::String(name.to_owned()));
        op.insert("path".to_owned(), Value::String(path.to_owned()));
        if let Some(value) = value {
            op.insert("value".to_owned(), value.clone());
        }
        Value::Object(op)
    };
    match edit {
        Edit::Replace(_, after) => out.push(op("replace", path, Some(*after))),
        Edit::Object(members) => {
            for member in members {
                let keep = path.len();
                push_token(path, member.key);
                match &member.what {
                    What::Added(value) => out.push(op("add", path, Some(*value))),
                    What::Removed(_) => out.push(op("remove", path, None)),
                    What::Changed(inner) => operations(inner, path, out),
                }
                path.truncate(keep);
            }
        }
        Edit::Array(steps) => {
            for step in steps.iter().rev() {
                let keep = path.len();
                match step {
                    Step::Remove { at, .. } => {
                        push_token(path, &at.to_string());
                        out.push(op("remove", path, None));
                    }
                    Step::Add { at, value, .. } => {
                        push_token(path, &at.to_string());
                        out.push(op("add", path, Some(*value)));
                    }
                    Step::Modify { at, edit, .. } => {
                        push_token(path, &at.to_string());
                        operations(edit, path, out);
                    }
                }
                path.truncate(keep);
            }
        }
    }
}

/// Append `/token` to a JSON Pointer.
fn push_token(path: &mut String, token: &str) {
    path.push('/');
    path.push_str(&escape(token));
}

/// A few words on a value, for a list of changes: scalars as they are (a long
/// string cut short), small arrays and objects in full, others by their size.
pub fn summarize(value: &Value) -> String {
    const LIMIT: usize = 60;
    match value {
        Value::String(text) => {
            let mut shown: String = text.chars().take(LIMIT).collect();
            if text.chars().nth(LIMIT).is_some() {
                shown.push('…');
            }
            serde_json::to_string(&shown).unwrap_or(shown)
        }
        Value::Array(items) => {
            small(value).unwrap_or_else(|| format!("[{} item{}]", items.len(), plural(items.len())))
        }
        Value::Object(members) => small(value)
            .unwrap_or_else(|| format!("{{{} key{}}}", members.len(), plural(members.len()))),
        scalar => scalar.to_string(),
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// `value` on one line, if it is a handful of short values.
fn small(value: &Value) -> Option<String> {
    fn within(value: &Value, nodes: &mut usize) -> bool {
        if *nodes == 0 {
            return false;
        }
        *nodes -= 1;
        match value {
            Value::Array(items) => items.iter().all(|v| within(v, nodes)),
            Value::Object(members) => members.values().all(|v| within(v, nodes)),
            _ => true,
        }
    }
    if !within(value, &mut 8) {
        return None;
    }
    let text = reformat::render(
        value,
        &reformat::Options {
            indent: Indent::Minified,
            ..reformat::Options::default()
        },
    );
    (text.chars().count() <= 60).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn run(before: &Value, after: &Value) -> Diff {
        diff(before, after, &AtomicBool::new(false)).unwrap()
    }

    fn listed(d: &Diff) -> Vec<(ChangeKind, &str)> {
        d.changes
            .iter()
            .map(|c| (c.kind, c.path.as_str()))
            .collect()
    }

    #[test]
    fn objects_are_equal_whatever_the_order_of_their_members() {
        assert!(equal(
            &parse(r#"{"a":1,"b":[1,2]}"#),
            &parse(r#"{"b":[1,2],"a":1}"#)
        ));
        assert!(!equal(&parse(r#"{"a":1}"#), &parse(r#"{"a":1,"b":2}"#)));
    }

    #[test]
    fn arrays_are_equal_only_in_the_same_order() {
        assert!(!equal(&json!([1, 2]), &json!([2, 1])));
        assert!(equal(&json!([1, [2]]), &json!([1, [2]])));
    }

    #[test]
    fn numbers_are_equal_by_value() {
        for (a, b) in [
            ("1", "1.0"),
            ("1", "10e-1"),
            ("100", "1E2"),
            ("0.5", "5e-1"),
            ("-0", "0"),
            ("0.0", "0"),
            ("12.50", "12.5"),
        ] {
            assert!(equal(&parse(a), &parse(b)), "{a} vs {b}");
        }
        for (a, b) in [("1", "2"), ("1", "-1"), ("0.1", "0.01"), ("1e2", "1e3")] {
            assert!(!equal(&parse(a), &parse(b)), "{a} vs {b}");
        }
    }

    #[test]
    fn numbers_keep_every_digit() {
        assert!(!equal(
            &parse("123456789012345678901234567890"),
            &parse("123456789012345678901234567891")
        ));
        assert!(equal(
            &parse("123456789012345678901234567890"),
            &parse("1.2345678901234567890123456789e29")
        ));
    }

    #[test]
    fn things_of_different_kinds_are_not_equal() {
        assert!(!equal(&json!(1), &json!("1")));
        assert!(!equal(&json!(null), &json!(false)));
        assert!(!equal(&json!([]), &json!({})));
    }

    #[test]
    fn the_same_documents_have_no_differences() {
        let doc = parse(r#"{"a":[1,{"b":2.5}],"c":"x"}"#);
        let d = run(&doc, &doc.clone());
        assert!(d.is_empty());
        assert_eq!(d.total(), 0);
        assert_eq!(d.patch(), json!([]));
    }

    #[test]
    fn a_document_with_its_keys_in_another_order_is_the_same() {
        assert!(run(&parse(r#"{"a":1,"b":2}"#), &parse(r#"{"b":2,"a":1}"#)).is_empty());
    }

    #[test]
    fn members_that_were_added_removed_and_changed_are_listed_with_their_paths() {
        let before = parse(r#"{"keep":1,"gone":2,"edit":{"x":1,"y":2}}"#);
        let after = parse(r#"{"keep":1,"edit":{"x":1,"y":3},"new":[1]}"#);
        let d = run(&before, &after);
        assert_eq!(
            listed(&d),
            [
                (ChangeKind::Removed, "/gone"),
                (ChangeKind::Changed, "/edit/y"),
                (ChangeKind::Added, "/new"),
            ]
        );
        assert_eq!((d.added, d.removed, d.changed), (1, 1, 1));
        assert_eq!(d.changes[0].before.as_deref(), Some("2"));
        assert_eq!(d.changes[0].after, None);
        assert_eq!(d.changes[1].before.as_deref(), Some("2"));
        assert_eq!(d.changes[1].after.as_deref(), Some("3"));
        assert_eq!(d.changes[2].after.as_deref(), Some("[1]"));
    }

    #[test]
    fn a_number_written_another_way_is_not_a_change() {
        assert!(run(&parse(r#"{"a":1}"#), &parse(r#"{"a":1.0}"#)).is_empty());
    }

    #[test]
    fn slashes_and_tildes_in_keys_are_escaped_in_paths() {
        let d = run(&json!({"a/b": 1, "m~n": 1}), &json!({"a/b": 2, "m~n": 2}));
        assert_eq!(
            listed(&d),
            [
                (ChangeKind::Changed, "/a~1b"),
                (ChangeKind::Changed, "/m~0n")
            ]
        );
        assert_eq!(d.operations[0]["path"], "/a~1b");
    }

    #[test]
    fn a_value_put_in_at_the_start_of_an_array_is_one_addition() {
        let d = run(&json!([1, 2, 3, 4]), &json!([0, 1, 2, 3, 4]));
        assert_eq!(listed(&d), [(ChangeKind::Added, "/0")]);
        assert_eq!(
            d.operations,
            [json!({"op": "add", "path": "/0", "value": 0})]
        );
    }

    #[test]
    fn a_value_taken_out_of_the_middle_of_an_array_is_one_removal() {
        let d = run(&json!(["a", "b", "c", "d"]), &json!(["a", "c", "d"]));
        assert_eq!(listed(&d), [(ChangeKind::Removed, "/1")]);
        assert_eq!(d.operations, [json!({"op": "remove", "path": "/1"})]);
    }

    #[test]
    fn a_changed_element_is_found_inside() {
        let before = json!([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]);
        let after = json!([{"id": 1, "name": "a"}, {"id": 2, "name": "B"}]);
        let d = run(&before, &after);
        assert_eq!(listed(&d), [(ChangeKind::Changed, "/1/name")]);
    }

    #[test]
    fn elements_changed_and_added_together() {
        let before = json!([{"n": 1}, {"n": 2}]);
        let after = json!([{"n": 10}, {"n": 20}, {"n": 30}]);
        let d = run(&before, &after);
        assert_eq!(
            listed(&d),
            [
                (ChangeKind::Changed, "/0/n"),
                (ChangeKind::Changed, "/1/n"),
                (ChangeKind::Added, "/2"),
            ]
        );
        // Last to first: the addition at the end comes before the changes.
        let paths: Vec<_> = d
            .operations
            .iter()
            .map(|o| o["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, ["/2", "/1/n", "/0/n"]);
    }

    #[test]
    fn a_reordered_array_is_made_of_additions_and_removals() {
        let d = run(&json!([1, 2, 3]), &json!([3, 2, 1]));
        assert!(!d.is_empty());
        assert_eq!(d.total(), d.operations.len());
    }

    #[test]
    fn a_value_of_another_kind_replaces_the_whole() {
        let d = run(&json!({"a": 1}), &json!([1]));
        assert_eq!(listed(&d), [(ChangeKind::Changed, "")]);
        assert_eq!(d.changes[0].before.as_deref(), Some("{\"a\":1}"));
        assert_eq!(d.changes[0].after.as_deref(), Some("[1]"));
        assert_eq!(
            d.operations,
            [json!({"op": "replace", "path": "", "value": [1]})]
        );

        let d = run(&json!({"a": "x"}), &json!({"a": 5}));
        assert_eq!(listed(&d), [(ChangeKind::Changed, "/a")]);
    }

    #[test]
    fn summaries_are_short() {
        assert_eq!(summarize(&json!(12)), "12");
        assert_eq!(summarize(&json!(null)), "null");
        assert_eq!(summarize(&json!("hi")), "\"hi\"");
        assert_eq!(summarize(&json!([1, 2])), "[1,2]");
        assert_eq!(summarize(&json!({"a": 1})), "{\"a\":1}");
        assert_eq!(summarize(&json!([])), "[]");

        let long = "x".repeat(100);
        let shown = summarize(&json!(long));
        assert_eq!(shown.chars().count(), 60 + 2 + 1, "{shown}");
        assert!(shown.ends_with("…\""));

        let many: Vec<u32> = (0..50).collect();
        assert_eq!(summarize(&json!(many)), "[50 items]");
        let big: serde_json::Map<String, Value> =
            (0..20).map(|i| (format!("key{i}"), json!(i))).collect();
        assert_eq!(summarize(&Value::Object(big)), "{20 keys}");
        assert_eq!(
            summarize(&json!([[1, [2, [3, [4, [5, [6, [7, [8]]]]]]]]])),
            "[1 item]"
        );
    }

    #[test]
    fn only_so_many_changes_are_listed_but_all_are_counted() {
        let before: Vec<u32> = (0..6_000).collect();
        let after: Vec<u32> = (100_000..106_000).collect();
        let d = run(&json!(before), &json!(after));
        assert_eq!(d.changes.len(), MAX_CHANGES);
        assert_eq!(d.total(), 6_000);
        assert_eq!(d.operations.len(), 6_000);
    }

    #[test]
    fn a_cancelled_comparison_stops() {
        let a = json!([1, 2, 3]);
        let cancel = AtomicBool::new(true);
        assert_eq!(diff(&a, &a, &cancel), Err(Cancelled));
    }

    #[test]
    fn long_identical_arrays_are_quick_to_compare() {
        let items: Vec<u32> = (0..300_000).collect();
        let a = json!(items);
        assert!(run(&a, &a.clone()).is_empty());

        let mut b = items.clone();
        b.insert(150_000, 7);
        let d = run(&a, &json!(b));
        assert_eq!(d.total(), 1);
    }
}
