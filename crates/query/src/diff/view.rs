//! The two documents of a comparison laid out side by side, line by line, the
//! way a file-comparison tool shows two files: what both have is on one line on
//! both sides, what only one has leaves a blank on the other, and what changed
//! is on one line, marked.
//!
//! The lines are the pretty-printed documents (two spaces per level, numbers as
//! they were written), and which lines go together is not found by comparing
//! text but by walking the two documents together with the [`Edit`] tree the
//! patch and the list of changes come from. So what is marked here is exactly
//! what is in the patch — a key that moved is not a difference, `1` and `1.0`
//! are not, a value put in at the start of an array is one added line and not a
//! change to every line after it.
//!
//! Both sides show the members of an object in the order of the *first*
//! document (members only the second has go after the member that precedes them
//! there): the order of an object's members is not a difference, so a document
//! whose keys are in another order lines up with the other instead of showing
//! every line as moved.
//!
//! A document of more than [`MAX_ROWS`] lines is not laid out ([`TooLong`]).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{Map, Value};

use super::{push_token, Edit, What};
use crate::reformat::{self, Options};

/// The most rows a comparison is laid out in. (Each is two short strings; this
/// is a few tens of megabytes at most.)
pub const MAX_ROWS: usize = 200_000;
/// The longest a line is kept: a longer one (a string of a megabyte) is cut and
/// ends with `…`. The patch has the whole value.
const MAX_LINE_CHARS: usize = 400;
const INDENT: &str = "  ";

/// What a row says about its two sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// The same on both sides.
    Same,
    /// A line on both sides, with a different value.
    Changed,
    /// A line only the left side has.
    Removed,
    /// A line only the right side has.
    Added,
}

/// One line of the view: the text on each side, `None` where a side has
/// nothing (a blank in the view).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub mark: Mark,
    pub left: Option<String>,
    pub right: Option<String>,
    /// The JSON Pointer of the change this row is part of (as in
    /// [`super::Change::path`]): none for a row that is the same on both sides.
    pub path: Option<Arc<str>>,
}

/// A run of rows that differ, between rows that don't.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub start: usize,
    /// One past the last row.
    pub end: usize,
    /// `Added` or `Removed` when every row of the block is, else `Changed`.
    pub mark: Mark,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideBySide {
    pub rows: Vec<Row>,
    pub blocks: Vec<Block>,
    /// The longest line, in characters.
    pub width: usize,
    /// How many lines each document has.
    pub left_lines: usize,
    pub right_lines: usize,
}

/// The documents have more lines than [`MAX_ROWS`] allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong;

pub(super) enum Stop {
    TooLong,
    Cancelled,
}

type Done = Result<(), Stop>;

/// Lay out `before` and `after`, which differ as `edit` says (`None` when they
/// are the same).
pub(super) fn build<'a>(
    before: &'a Value,
    after: &'a Value,
    edit: Option<&'a Edit<'a>>,
    cancel: &AtomicBool,
) -> Result<SideBySide, Stop> {
    if line_count(before).max(line_count(after)) > MAX_ROWS {
        return Err(Stop::TooLong);
    }
    let mut layout = Gen {
        rows: Vec::new(),
        cancel,
        width: 0,
        left_lines: 0,
        right_lines: 0,
    };
    layout.pair(
        before,
        after,
        edit,
        Place {
            depth: 0,
            key: None,
            comma: (false, false),
        },
        &mut String::new(),
    )?;
    let blocks = blocks(&layout.rows);
    Ok(SideBySide {
        rows: layout.rows,
        blocks,
        width: layout.width,
        left_lines: layout.left_lines,
        right_lines: layout.right_lines,
    })
}

/// How many lines `value` is when pretty-printed.
fn line_count(value: &Value) -> usize {
    match value {
        Value::Array(items) if !items.is_empty() => 2 + items.iter().map(line_count).sum::<usize>(),
        Value::Object(members) if !members.is_empty() => {
            2 + members.values().map(line_count).sum::<usize>()
        }
        _ => 1,
    }
}

fn blocks(rows: &[Row]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut open: Option<Block> = None;
    for (at, row) in rows.iter().enumerate() {
        if row.mark == Mark::Same {
            blocks.extend(open.take());
        } else if let Some(block) = &mut open {
            block.end = at + 1;
            if block.mark != row.mark {
                block.mark = Mark::Changed;
            }
        } else {
            open = Some(Block {
                start: at,
                end: at + 1,
                mark: row.mark,
            });
        }
    }
    blocks.extend(open);
    blocks
}

/// Where in the document a value is, as the lines of it need to know.
#[derive(Clone, Copy)]
struct Place<'p> {
    depth: usize,
    /// The member's name, for the line to begin with.
    key: Option<&'p str>,
    /// Whether a comma follows the value on the left and on the right (it does
    /// when something follows it on that side).
    comma: (bool, bool),
}

#[derive(Clone, Copy)]
enum Token<'a> {
    Key(&'a str),
    Index(usize),
}

/// One thing in an object or an array: on one side or both.
struct Item<'a> {
    token: Token<'a>,
    left: Option<&'a Value>,
    right: Option<&'a Value>,
    /// How the two differ, when both have it and it is not the same.
    edit: Option<&'a Edit<'a>>,
}

struct Gen<'c> {
    rows: Vec<Row>,
    cancel: &'c AtomicBool,
    width: usize,
    left_lines: usize,
    right_lines: usize,
}

impl<'a> Gen<'_> {
    fn push(
        &mut self,
        mark: Mark,
        left: Option<String>,
        right: Option<String>,
        path: Option<&Arc<str>>,
    ) -> Done {
        if self.rows.len() >= MAX_ROWS {
            return Err(Stop::TooLong);
        }
        if self.rows.len() & 1023 == 0 && self.cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        let (left, right) = (left.map(clip), right.map(clip));
        for (text, lines) in [
            (&left, &mut self.left_lines),
            (&right, &mut self.right_lines),
        ] {
            if let Some(text) = text {
                *lines += 1;
                self.width = self.width.max(text.chars().count());
            }
        }
        self.rows.push(Row {
            mark,
            left,
            right,
            path: path.cloned(),
        });
        Ok(())
    }

    /// The lines of two versions of one value (`edit` says how they differ, if
    /// they do).
    fn pair(
        &mut self,
        before: &'a Value,
        after: &'a Value,
        edit: Option<&'a Edit<'a>>,
        at: Place<'_>,
        path: &mut String,
    ) -> Done {
        match (before, after) {
            (Value::Object(x), Value::Object(y)) if !(x.is_empty() && y.is_empty()) => {
                let empty = (x.is_empty(), y.is_empty());
                self.container(object_items(x, y, edit), ("{", "}"), empty, at, path)
            }
            (Value::Array(x), Value::Array(y)) if !(x.is_empty() && y.is_empty()) => {
                let empty = (x.is_empty(), y.is_empty());
                self.container(array_items(x, y, edit), ("[", "]"), empty, at, path)
            }
            _ => self.leaf(before, after, edit.is_some(), at, path),
        }
    }

    /// An object or an array both sides have: its opening, what is in it, its
    /// closing. A side that has it empty has it on one line (`[]`), which is
    /// the opening, and no closing.
    fn container(
        &mut self,
        items: Vec<Item<'a>>,
        (open, close): (&str, &str),
        empty: (bool, bool),
        at: Place<'_>,
        path: &mut String,
    ) -> Done {
        let pad = INDENT.repeat(at.depth);
        let prefix = at.key.map(member_prefix).unwrap_or_default();
        let comma = |on: bool| if on { "," } else { "" };
        let opening = |empty: bool, comma_after: bool| {
            if empty {
                format!("{pad}{prefix}{open}{close}{}", comma(comma_after))
            } else {
                format!("{pad}{prefix}{open}")
            }
        };
        self.push(
            Mark::Same,
            Some(opening(empty.0, at.comma.0)),
            Some(opening(empty.1, at.comma.1)),
            None,
        )?;

        let last_left = items.iter().rposition(|item| item.left.is_some());
        let last_right = items.iter().rposition(|item| item.right.is_some());
        for (n, item) in items.iter().enumerate() {
            let comma = (
                last_left.is_some_and(|last| n < last),
                last_right.is_some_and(|last| n < last),
            );
            let keep = path.len();
            match item.token {
                Token::Key(key) => push_token(path, key),
                Token::Index(index) => push_token(path, &index.to_string()),
            }
            self.item(item, at.depth + 1, comma, path)?;
            path.truncate(keep);
        }

        let closing = |empty: bool, comma_after: bool| {
            (!empty).then(|| format!("{pad}{close}{}", comma(comma_after)))
        };
        self.push(
            Mark::Same,
            closing(empty.0, at.comma.0),
            closing(empty.1, at.comma.1),
            None,
        )
    }

    fn item(
        &mut self,
        item: &Item<'a>,
        depth: usize,
        comma: (bool, bool),
        path: &mut String,
    ) -> Done {
        let key = match item.token {
            Token::Key(key) => Some(key),
            Token::Index(_) => None,
        };
        match (item.left, item.right) {
            (Some(before), Some(after)) => {
                let at = Place { depth, key, comma };
                self.pair(before, after, item.edit, at, path)
            }
            (Some(value), None) => self.alone(value, true, Place { depth, key, comma }, path),
            (None, Some(value)) => self.alone(value, false, Place { depth, key, comma }, path),
            (None, None) => Ok(()),
        }
    }

    /// The lines of a value only one side has.
    fn alone(&mut self, value: &Value, on_left: bool, at: Place<'_>, path: &str) -> Done {
        if self.rows.len() + line_count(value) > MAX_ROWS {
            return Err(Stop::TooLong);
        }
        let comma = if on_left { at.comma.0 } else { at.comma.1 };
        let path: Arc<str> = path.into();
        for line in lines_of(value, at.depth, at.key, comma) {
            if on_left {
                self.push(Mark::Removed, Some(line), None, Some(&path))?;
            } else {
                self.push(Mark::Added, None, Some(line), Some(&path))?;
            }
        }
        Ok(())
    }

    /// Two values that are not both objects or both arrays (or are both empty):
    /// the same, or a replacement. A replacement can be of several lines (a
    /// number by an object); the lines are matched up from the top, and those
    /// the longer one has over are removed or added.
    fn leaf(
        &mut self,
        before: &Value,
        after: &Value,
        differs: bool,
        at: Place<'_>,
        path: &str,
    ) -> Done {
        let left = lines_of(before, at.depth, at.key, at.comma.0);
        let right = lines_of(after, at.depth, at.key, at.comma.1);
        let path: Option<Arc<str>> = differs.then(|| path.into());
        let (mut left, mut right) = (left.into_iter(), right.into_iter());
        loop {
            let (l, r) = (left.next(), right.next());
            let mark = match (&l, &r, differs) {
                (None, None, _) => return Ok(()),
                (Some(_), None, _) => Mark::Removed,
                (None, Some(_), _) => Mark::Added,
                (Some(_), Some(_), true) => Mark::Changed,
                (Some(_), Some(_), false) => Mark::Same,
            };
            self.push(mark, l, r, path.as_ref())?;
        }
    }
}

/// The members of two objects, those only one has included: in the first one's
/// order, the second one's own members after the member that comes before them.
fn object_items<'a>(
    x: &'a Map<String, Value>,
    y: &'a Map<String, Value>,
    edit: Option<&'a Edit<'a>>,
) -> Vec<Item<'a>> {
    let changed: HashMap<&str, &Edit<'_>> = match edit {
        Some(Edit::Object(members)) => members
            .iter()
            .filter_map(|member| match &member.what {
                What::Changed(inner) => Some((member.key, inner)),
                _ => None,
            })
            .collect(),
        _ => HashMap::new(),
    };

    let mut after: HashMap<Option<&str>, Vec<&str>> = HashMap::new();
    let mut anchor: Option<&str> = None;
    for key in y.keys() {
        if x.contains_key(key) {
            anchor = Some(key);
        } else {
            after.entry(anchor).or_default().push(key);
        }
    }
    let mut keys: Vec<&str> = Vec::with_capacity(x.len() + y.len());
    keys.extend(after.remove(&None).unwrap_or_default());
    for key in x.keys() {
        keys.push(key);
        keys.extend(after.remove(&Some(key.as_str())).unwrap_or_default());
    }

    keys.into_iter()
        .map(|key| Item {
            token: Token::Key(key),
            left: x.get(key),
            right: y.get(key),
            edit: changed.get(key).copied(),
        })
        .collect()
}

/// The elements of two arrays, lined up as `edit` says: elements are taken from
/// the two in step, except where the edit has one removed from the first array
/// or added from the second.
fn array_items<'a>(x: &'a [Value], y: &'a [Value], edit: Option<&'a Edit<'a>>) -> Vec<Item<'a>> {
    use super::Step;

    let Some(Edit::Array(steps)) = edit else {
        // The same array twice: element for element.
        return x
            .iter()
            .zip(y)
            .enumerate()
            .map(|(index, (before, after))| Item {
                token: Token::Index(index),
                left: Some(before),
                right: Some(after),
                edit: None,
            })
            .collect();
    };

    let mut removed = HashSet::new();
    let mut added = HashSet::new();
    let mut modified: HashMap<usize, &Edit<'_>> = HashMap::new();
    for step in steps {
        match step {
            Step::Remove { at, .. } => {
                removed.insert(*at);
            }
            Step::Add { index, .. } => {
                added.insert(*index);
            }
            Step::Modify { at, edit, .. } => {
                modified.insert(*at, edit);
            }
        }
    }

    let (mut i, mut j) = (0, 0);
    let mut items = Vec::new();
    while i < x.len() || j < y.len() {
        // What the first array has that the second doesn't, then what the second
        // has that the first doesn't, then the two in step. (Anything else left
        // over when one array has ended is taken for removed or added: that
        // can't be, but must not loop.)
        if i < x.len() && (removed.contains(&i) || j >= y.len()) {
            items.push(Item {
                token: Token::Index(i),
                left: Some(&x[i]),
                right: None,
                edit: None,
            });
            i += 1;
        } else if j < y.len() && (added.contains(&j) || i >= x.len()) {
            items.push(Item {
                token: Token::Index(j),
                left: None,
                right: Some(&y[j]),
                edit: None,
            });
            j += 1;
        } else {
            items.push(Item {
                token: Token::Index(j),
                left: Some(&x[i]),
                right: Some(&y[j]),
                edit: modified.get(&i).copied(),
            });
            i += 1;
            j += 1;
        }
    }
    items
}

/// `"name": `, which a member's first line begins with.
fn member_prefix(key: &str) -> String {
    format!(
        "{}: ",
        reformat::render(&Value::String(key.to_owned()), &Options::default())
    )
}

/// `value` pretty-printed as lines: indented `depth` levels, the first one
/// beginning with the member's name, if it is a member's value, and the last
/// ending with a comma if one follows.
fn lines_of(value: &Value, depth: usize, key: Option<&str>, comma: bool) -> Vec<String> {
    let text = reformat::render(value, &Options::default());
    let pad = INDENT.repeat(depth);
    let prefix = key.map(member_prefix).unwrap_or_default();
    let mut lines: Vec<String> = text
        .split('\n')
        .enumerate()
        .map(|(n, line)| {
            if n == 0 {
                format!("{pad}{prefix}{line}")
            } else {
                format!("{pad}{line}")
            }
        })
        .collect();
    if comma {
        if let Some(last) = lines.last_mut() {
            last.push(',');
        }
    }
    lines
}

/// A line cut to what is worth showing.
fn clip(text: String) -> String {
    match text.char_indices().nth(MAX_LINE_CHARS) {
        None => text,
        Some((at, _)) => {
            let mut cut = text;
            cut.truncate(at);
            cut.push('…');
            cut
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{compare, equal};
    use super::*;
    use serde_json::json;

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn view(before: &Value, after: &Value) -> SideBySide {
        compare(before, after, &AtomicBool::new(false))
            .unwrap()
            .view
            .unwrap()
    }

    /// The rows as `(mark, left, right)`, with `-` for a blank.
    fn shown(view: &SideBySide) -> Vec<(Mark, String, String)> {
        view.rows
            .iter()
            .map(|r| {
                (
                    r.mark,
                    r.left.clone().unwrap_or_else(|| "-".to_owned()),
                    r.right.clone().unwrap_or_else(|| "-".to_owned()),
                )
            })
            .collect()
    }

    fn row(mark: Mark, left: &str, right: &str) -> (Mark, String, String) {
        (mark, left.to_owned(), right.to_owned())
    }

    fn side(view: &SideBySide, left: bool) -> String {
        view.rows
            .iter()
            .filter_map(|r| if left { &r.left } else { &r.right }.as_deref())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn documents_that_are_the_same_are_one_side_twice_with_nothing_marked() {
        let doc = parse(r#"{"a":[1,{"b":2}],"c":"x"}"#);
        let v = view(&doc, &doc.clone());
        assert!(v
            .rows
            .iter()
            .all(|r| r.mark == Mark::Same && r.path.is_none()));
        assert!(v.blocks.is_empty());
        assert_eq!(side(&v, true), reformat::render(&doc, &Options::default()));
        assert_eq!(side(&v, true), side(&v, false));
        assert_eq!((v.left_lines, v.right_lines), (9, 9));
    }

    #[test]
    fn a_changed_value_is_a_line_on_both_sides_marked_changed() {
        let v = view(&parse(r#"{"a":1,"b":2}"#), &parse(r#"{"a":1,"b":3}"#));
        assert_eq!(
            shown(&v),
            [
                row(Mark::Same, "{", "{"),
                row(Mark::Same, "  \"a\": 1,", "  \"a\": 1,"),
                row(Mark::Changed, "  \"b\": 2", "  \"b\": 3"),
                row(Mark::Same, "}", "}"),
            ]
        );
        assert_eq!(v.rows[2].path.as_deref(), Some("/b"));
        assert_eq!(
            v.blocks,
            [Block {
                start: 2,
                end: 3,
                mark: Mark::Changed
            }]
        );
    }

    #[test]
    fn a_member_one_side_lacks_leaves_a_blank_and_the_commas_follow_each_side() {
        let v = view(&parse(r#"{"a":1,"b":2}"#), &parse(r#"{"a":1}"#));
        assert_eq!(
            shown(&v),
            [
                row(Mark::Same, "{", "{"),
                // "a" is the last member on the right, so no comma there.
                row(Mark::Same, "  \"a\": 1,", "  \"a\": 1"),
                row(Mark::Removed, "  \"b\": 2", "-"),
                row(Mark::Same, "}", "}"),
            ]
        );
        assert_eq!(v.rows[2].path.as_deref(), Some("/b"));

        let v = view(&parse(r#"{"a":1}"#), &parse(r#"{"a":1,"b":[true]}"#));
        assert_eq!(
            shown(&v),
            [
                row(Mark::Same, "{", "{"),
                row(Mark::Same, "  \"a\": 1", "  \"a\": 1,"),
                row(Mark::Added, "-", "  \"b\": ["),
                row(Mark::Added, "-", "    true"),
                row(Mark::Added, "-", "  ]"),
                row(Mark::Same, "}", "}"),
            ]
        );
        assert_eq!(v.blocks.len(), 1);
        assert_eq!(v.blocks[0].mark, Mark::Added);
        assert!(v.rows[2..5].iter().all(|r| r.path.as_deref() == Some("/b")));
    }

    #[test]
    fn a_value_put_in_at_the_start_of_an_array_is_one_added_line() {
        let v = view(&json!([1, 2, 3]), &json!([0, 1, 2, 3]));
        assert_eq!(
            shown(&v),
            [
                row(Mark::Same, "[", "["),
                row(Mark::Added, "-", "  0,"),
                row(Mark::Same, "  1,", "  1,"),
                row(Mark::Same, "  2,", "  2,"),
                row(Mark::Same, "  3", "  3"),
                row(Mark::Same, "]", "]"),
            ]
        );
        assert_eq!(v.rows[1].path.as_deref(), Some("/0"));
    }

    #[test]
    fn an_element_taken_out_of_an_array_is_one_removed_line_with_its_old_place() {
        let v = view(&json!(["a", "b", "c"]), &json!(["a", "c"]));
        assert_eq!(
            shown(&v),
            [
                row(Mark::Same, "[", "["),
                row(Mark::Same, "  \"a\",", "  \"a\","),
                row(Mark::Removed, "  \"b\",", "-"),
                row(Mark::Same, "  \"c\"", "  \"c\""),
                row(Mark::Same, "]", "]"),
            ]
        );
        assert_eq!(v.rows[2].path.as_deref(), Some("/1"));
    }

    #[test]
    fn a_change_deep_inside_is_found_in_its_place() {
        let before = json!({"list": [{"id": 1, "n": "a"}, {"id": 2, "n": "b"}]});
        let after = json!({"list": [{"id": 1, "n": "a"}, {"id": 2, "n": "B"}]});
        let v = view(&before, &after);
        assert_eq!(v.blocks.len(), 1);
        let at = v.blocks[0].start;
        assert_eq!(
            (v.rows[at].left.as_deref(), v.rows[at].right.as_deref()),
            (Some("      \"n\": \"b\""), Some("      \"n\": \"B\""))
        );
        assert_eq!(v.rows[at].path.as_deref(), Some("/list/1/n"));
    }

    #[test]
    fn keys_in_another_order_and_numbers_written_another_way_are_not_differences() {
        let before = parse(r#"{"a": 1, "b": {"x": 1.0, "y": 2}}"#);
        let after = parse(r#"{"b": {"y": 2, "x": 1}, "a": 1}"#);
        let v = view(&before, &after);
        assert!(v.blocks.is_empty(), "{:?}", shown(&v));
        // Both sides are in the first document's order.
        let keys: Vec<_> = v.rows.iter().filter_map(|r| r.right.as_deref()).collect();
        assert_eq!(keys[1], "  \"a\": 1,");
        assert_eq!(
            keys[3], "    \"x\": 1,",
            "the second document's own notation"
        );
        assert_eq!(v.rows[3].left.as_deref(), Some("    \"x\": 1.0,"));
    }

    #[test]
    fn members_only_the_second_document_has_go_after_the_one_before_them() {
        let before = parse(r#"{"a": 1, "c": 3}"#);
        let after = parse(r#"{"a": 1, "b": 2, "c": 3, "d": 4}"#);
        let v = view(&before, &after);
        let right: Vec<_> = v.rows.iter().filter_map(|r| r.right.as_deref()).collect();
        assert_eq!(
            right,
            [
                "{",
                "  \"a\": 1,",
                "  \"b\": 2,",
                "  \"c\": 3,",
                "  \"d\": 4",
                "}"
            ]
        );
        assert_eq!(v.blocks.len(), 2);
    }

    #[test]
    fn a_value_of_another_kind_is_matched_up_from_the_top() {
        let v = view(&parse(r#"{"a": 1}"#), &parse(r#"{"a": {"x": 1, "y": 2}}"#));
        assert_eq!(
            shown(&v),
            [
                row(Mark::Same, "{", "{"),
                row(Mark::Changed, "  \"a\": 1", "  \"a\": {"),
                row(Mark::Added, "-", "    \"x\": 1,"),
                row(Mark::Added, "-", "    \"y\": 2"),
                row(Mark::Added, "-", "  }"),
                row(Mark::Same, "}", "}"),
            ]
        );
        assert_eq!(v.blocks.len(), 1);
        assert_eq!(v.blocks[0].mark, Mark::Changed);
    }

    #[test]
    fn a_whole_document_that_is_replaced_is_one_changed_row() {
        let v = view(&json!(1), &json!("x"));
        assert_eq!(shown(&v), [row(Mark::Changed, "1", "\"x\"")]);
        assert_eq!(v.rows[0].path.as_deref(), Some(""));
    }

    #[test]
    fn an_empty_container_that_gets_members_opens_up_on_the_side_that_has_them() {
        let v = view(&json!({"a": []}), &json!({"a": [1]}));
        assert_eq!(
            shown(&v),
            [
                row(Mark::Same, "{", "{"),
                row(Mark::Same, "  \"a\": []", "  \"a\": ["),
                row(Mark::Added, "-", "    1"),
                row(Mark::Same, "-", "  ]"),
                row(Mark::Same, "}", "}"),
            ]
        );

        let v = view(&json!([{"x": 1}, 2]), &json!([{}, 2]));
        assert_eq!(
            shown(&v),
            [
                row(Mark::Same, "[", "["),
                row(Mark::Same, "  {", "  {},"),
                row(Mark::Removed, "    \"x\": 1", "-"),
                row(Mark::Same, "  },", "-"),
                row(Mark::Same, "  2", "  2"),
                row(Mark::Same, "]", "]"),
            ]
        );
    }

    #[test]
    fn long_lines_are_cut_and_the_width_is_the_longest_kept_line() {
        let long = "x".repeat(1000);
        let v = view(&json!({"a": long}), &json!({"a": "short"}));
        let cut = v.rows[1].left.as_deref().unwrap();
        assert!(cut.ends_with('…'));
        assert_eq!(cut.chars().count(), MAX_LINE_CHARS + 1);
        assert_eq!(v.width, MAX_LINE_CHARS + 1);
    }

    #[test]
    fn adjacent_differences_are_one_block_and_a_mixed_block_is_changed() {
        let v = view(&json!([1, 2, 3, 4]), &json!([1, 9, 3, 4, 5]));
        assert_eq!(v.blocks.len(), 2);
        assert_eq!(v.blocks[0].mark, Mark::Changed);
        assert_eq!(v.blocks[1].mark, Mark::Added);

        let v = view(&json!([1, 2]), &json!([3, 4, 5]));
        assert_eq!(v.blocks.len(), 1, "{:?}", shown(&v));
        assert_eq!(v.blocks[0].mark, Mark::Changed);
        assert_eq!((v.blocks[0].start, v.blocks[0].end), (1, 4));
    }

    #[test]
    fn a_document_with_too_many_lines_is_not_laid_out() {
        let many: Vec<u32> = (0..(MAX_ROWS as u32 + 1)).collect();
        let result = compare(&json!(many), &json!([1]), &AtomicBool::new(false)).unwrap();
        assert_eq!(result.view, Err(TooLong));
        assert_eq!(
            result.diff.total(),
            MAX_ROWS,
            "the patch is there all the same"
        );
    }

    #[test]
    fn two_documents_that_fit_but_not_together_are_not_laid_out_either() {
        let list: Vec<u32> = (0..(MAX_ROWS as u32 * 6 / 10)).collect();
        let (before, after) = (json!({"a": list.clone()}), json!({"b": list}));
        let result = compare(&before, &after, &AtomicBool::new(false)).unwrap();
        assert_eq!(result.view, Err(TooLong));
        assert_eq!(result.diff.total(), 2);
    }

    #[test]
    fn a_cancelled_layout_is_cancelled() {
        let a = json!([1, 2, 3]);
        let cancel = AtomicBool::new(true);
        assert!(compare(&a, &a, &cancel).is_err());
    }

    /// A small deterministic stream of numbers, for building documents.
    struct Dice(u64);

    impl Dice {
        fn next(&mut self, below: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % below
        }

        fn value(&mut self, depth: u32) -> Value {
            match self.next(if depth == 0 { 4 } else { 7 }) {
                0 => Value::Null,
                1 => json!(self.next(4)),
                2 => json!(["a", "b", "c"][self.next(3) as usize]),
                3 => json!(self.next(2) == 0),
                4 => Value::Array((0..self.next(5)).map(|_| self.value(depth - 1)).collect()),
                _ => Value::Object(
                    (0..self.next(5))
                        .map(|_| {
                            (
                                ["k", "l", "m", "n", "o"][self.next(5) as usize].to_owned(),
                                self.value(depth - 1),
                            )
                        })
                        .collect(),
                ),
            }
        }

        /// A copy of `value` with a few things changed.
        fn mutate(&mut self, value: &Value, depth: u32) -> Value {
            if self.next(5) == 0 {
                return self.value(depth);
            }
            match value {
                Value::Array(items) => {
                    let mut out: Vec<Value> = Vec::new();
                    for item in items {
                        match self.next(6) {
                            0 => {}
                            1 => {
                                out.push(self.value(depth.saturating_sub(1)));
                                out.push(item.clone());
                            }
                            _ => out.push(self.mutate(item, depth.saturating_sub(1))),
                        }
                    }
                    Value::Array(out)
                }
                Value::Object(members) => {
                    let mut out = Map::new();
                    for (key, member) in members {
                        if self.next(6) != 0 {
                            out.insert(key.clone(), self.mutate(member, depth.saturating_sub(1)));
                        }
                    }
                    if self.next(3) == 0 {
                        out.insert("z".to_owned(), self.value(depth.saturating_sub(1)));
                    }
                    Value::Object(out)
                }
                other => other.clone(),
            }
        }
    }

    #[test]
    fn each_side_reads_back_as_its_document_and_the_marks_are_the_patchs_changes() {
        let mut dice = Dice(0x9e37_79b9_7f4a_7c15);
        let cancel = AtomicBool::new(false);
        for round in 0..400 {
            let before = dice.value(3);
            let after = dice.mutate(&before, 3);
            let result = compare(&before, &after, &cancel).unwrap();
            let v = result.view.unwrap();

            // The left side is the first document's own text, commas and all.
            assert_eq!(
                side(&v, true),
                reformat::render(&before, &Options::default()),
                "round {round}: {before} -> {after}"
            );
            // The right side is the second document, in the first one's order.
            let right = parse(&side(&v, false));
            assert!(equal(&right, &after), "round {round}: {before} -> {after}");

            // Rows are marked where, and only where, the patch has a change.
            let mut marked: Vec<&str> = Vec::new();
            for row in &v.rows {
                match (&row.path, row.mark) {
                    (None, Mark::Same) => {}
                    (Some(path), mark) if mark != Mark::Same => {
                        if marked.last() != Some(&&**path) {
                            marked.push(path);
                        }
                    }
                    other => panic!("round {round}: a row with {other:?}"),
                }
            }
            let mut listed: Vec<&str> = result
                .diff
                .changes
                .iter()
                .map(|c| c.path.as_str())
                .collect();
            marked.sort_unstable();
            marked.dedup();
            listed.sort_unstable();
            listed.dedup();
            assert_eq!(marked, listed, "round {round}: {before} -> {after}");
            assert_eq!(v.blocks.is_empty(), result.diff.is_empty());
        }
    }
}
