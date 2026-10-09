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
//! Every difference gets a number, in the order the view meets them, and the
//! rows that are part of it carry it ([`Row::change`]). [`blend`] walks the
//! documents the same way and so can take some differences, by number, from one
//! document into the other: that is what moving a difference to the left or to
//! the right does.
//!
//! A document of more than [`MAX_ROWS`] lines is not laid out ([`TooLong`]).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{Map, Value};

use super::{push_token, Cancelled, Edit, Side, What};
use crate::reformat::{self, Options};

/// The most rows a comparison is laid out in. (Each is two short strings; this
/// is a few tens of megabytes at most.)
pub const MAX_ROWS: usize = 200_000;
/// The longest a line is kept: a longer one (a string of a megabyte) is cut and
/// ends with `…`. The patch has the whole value. (A line that is cut is
/// `MAX_LINE_CHARS + 1` characters, the last the `…`.)
pub const MAX_LINE_CHARS: usize = 400;
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
    /// The number of the difference this row is part of, counted 0, 1, 2… in the
    /// order the view meets them (see [`blend`]): the rows of one value that is
    /// added, removed or replaced share one. None for a row that is the same on
    /// both sides.
    pub change: Option<u32>,
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

impl SideBySide {
    /// The numbers of the differences the rows of `block` are part of, as a
    /// range (the rows of a block are in the order of the numbers, which have no
    /// gaps): what [`super::take_changes`] is given to move the block.
    pub fn changes_of(&self, block: &Block) -> Range<u32> {
        let rows = self.rows.get(block.start..block.end).unwrap_or_default();
        let first = rows.iter().find_map(|row| row.change);
        let last = rows.iter().rev().find_map(|row| row.change);
        match (first, last) {
            (Some(first), Some(last)) => first..last + 1,
            _ => 0..0,
        }
    }
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
        changes: 0,
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
    /// How many differences have been met: the number of the next one.
    changes: u32,
}

impl<'a> Gen<'_> {
    fn push(
        &mut self,
        mark: Mark,
        left: Option<String>,
        right: Option<String>,
        path: Option<&Arc<str>>,
        change: Option<u32>,
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
            change,
        });
        Ok(())
    }

    /// The number of the next difference.
    fn next_change(&mut self) -> u32 {
        self.changes += 1;
        self.changes - 1
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
        let change = Some(self.next_change());
        for line in lines_of(value, at.depth, at.key, comma) {
            if on_left {
                self.push(Mark::Removed, Some(line), None, Some(&path), change)?;
            } else {
                self.push(Mark::Added, None, Some(line), Some(&path), change)?;
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
        let change = differs.then(|| self.next_change());
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
            self.push(mark, l, r, path.as_ref(), change)?;
        }
    }
}

/// The document `into` with the differences numbered in `take` made the same as
/// the other document has them (see [`super::take_changes`]).
///
/// It walks the two documents exactly as [`build`] does — the same items in the
/// same order, so that a difference gets the same number here as on the rows of
/// the view — and builds a value instead of rows. What is in neither numbered
/// difference is the document `into`'s own, as it was written (a number
/// `1.0` stays `1.0`, an object's members stay in their order).
pub(super) fn blend(
    before: &Value,
    after: &Value,
    edit: Option<&Edit<'_>>,
    take: &[Range<u32>],
    into: Side,
    cancel: &AtomicBool,
) -> Result<Value, Cancelled> {
    Blend {
        take: runs(take),
        into,
        met: 0,
        ticks: 0,
        cancel,
    }
    .pair(before, after, edit)
}

/// `ranges` as the fewest ranges that hold the same numbers, in order: the empty ones
/// gone, and those that overlap or touch made one.
fn runs(ranges: &[Range<u32>]) -> Vec<Range<u32>> {
    let mut sorted: Vec<Range<u32>> = ranges.iter().filter(|r| r.start < r.end).cloned().collect();
    sorted.sort_unstable_by_key(|r| r.start);
    let mut merged: Vec<Range<u32>> = Vec::with_capacity(sorted.len());
    for range in sorted {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

struct Blend<'c> {
    /// The numbers of the differences to take, as [`runs`] makes them.
    take: Vec<Range<u32>>,
    into: Side,
    /// How many differences have been met: the number of the next one.
    met: u32,
    ticks: u32,
    cancel: &'c AtomicBool,
}

impl<'a> Blend<'_> {
    fn tick(&mut self) -> Result<(), Cancelled> {
        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks & 0x3ff == 0 && self.cancel.load(Ordering::Relaxed) {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    /// Meets a difference: whether it is one of those to take.
    fn meet(&mut self) -> bool {
        let number = self.met;
        self.met += 1;
        let at = self.take.partition_point(|run| run.end <= number);
        self.take.get(at).is_some_and(|run| run.start <= number)
    }

    /// Two versions of one value, as they are in the result. (The conditions are
    /// those of `Gen::pair`.)
    fn pair(
        &mut self,
        before: &'a Value,
        after: &'a Value,
        edit: Option<&'a Edit<'a>>,
    ) -> Result<Value, Cancelled> {
        self.tick()?;
        match (before, after) {
            (Value::Object(x), Value::Object(y)) if !(x.is_empty() && y.is_empty()) => {
                // In the order the view meets them, which numbers them…
                let mut made: HashMap<&str, Value> = HashMap::new();
                for item in object_items(x, y, edit) {
                    if let (Token::Key(key), Some(value)) = (item.token, self.item(&item)?) {
                        made.insert(key, value);
                    }
                }
                // …and in the order of the document that is changed, whose
                // members keep their places; one that is taken in goes after the
                // member before it, as `object_items` puts it.
                let (own, other) = match self.into {
                    Side::Left => (x, y),
                    Side::Right => (y, x),
                };
                let mut members = Map::new();
                for item in object_items(own, other, edit) {
                    if let Token::Key(key) = item.token {
                        if let Some(value) = made.remove(key) {
                            members.insert(key.to_owned(), value);
                        }
                    }
                }
                Ok(Value::Object(members))
            }
            (Value::Array(x), Value::Array(y)) if !(x.is_empty() && y.is_empty()) => {
                let mut elements = Vec::new();
                for item in array_items(x, y, edit) {
                    elements.extend(self.item(&item)?);
                }
                Ok(Value::Array(elements))
            }
            _ => Ok(self.leaf(before, after, edit.is_some())),
        }
    }

    /// One thing in an object or an array, as it is in the result: not there at
    /// all when that is what the difference it is part of comes to.
    fn item(&mut self, item: &Item<'a>) -> Result<Option<Value>, Cancelled> {
        match (item.left, item.right) {
            (Some(before), Some(after)) => self.pair(before, after, item.edit).map(Some),
            (Some(value), None) => Ok(self.alone(value, Side::Left)),
            (None, Some(value)) => Ok(self.alone(value, Side::Right)),
            (None, None) => Ok(None),
        }
    }

    /// A value only the document `on` has. The result has it when the document
    /// that is changed has it and the difference is left, or lacks it and the
    /// difference is taken.
    fn alone(&mut self, value: &Value, on: Side) -> Option<Value> {
        let taken = self.meet();
        ((on == self.into) != taken).then(|| value.clone())
    }

    /// Two values that are not both objects or both arrays: the document that is
    /// changed has its own, unless they differ and the difference is taken.
    fn leaf(&mut self, before: &Value, after: &Value, differs: bool) -> Value {
        let (own, other) = match self.into {
            Side::Left => (before, after),
            Side::Right => (after, before),
        };
        if differs && self.meet() {
            other.clone()
        } else {
            own.clone()
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
#[allow(clippy::single_range_in_vec_init)] // the tests name single ranges of numbers on purpose
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

    // Taking differences from one document into the other.

    use super::super::{diff, take_changes, take_selected};

    /// What `take` does to `before` (or `after`, when `into` is right), taken from
    /// the other.
    fn taken(before: &Value, after: &Value, take: Range<u32>, into: Side) -> Value {
        take_changes(before, after, take, into, &AtomicBool::new(false)).unwrap()
    }

    /// The numbers of the differences of each block of the view, as (first, one
    /// past the last).
    fn numbered(v: &SideBySide) -> Vec<(u32, u32)> {
        v.blocks
            .iter()
            .map(|b| {
                let numbers = v.changes_of(b);
                (numbers.start, numbers.end)
            })
            .collect()
    }

    #[test]
    fn the_differences_are_numbered_in_the_order_of_the_view_and_a_value_is_one() {
        // A changed value, a member only the left has (several lines), and a
        // number put in place of an object (lines that are only on the left).
        let v = view(
            &parse(r#"{"a":1,"s":0,"b":{"x":[1,2]},"c":{"y":1},"d":true}"#),
            &parse(r#"{"a":2,"s":0,"c":5,"d":true}"#),
        );
        let numbers: Vec<Option<u32>> = v.rows.iter().map(|r| r.change).collect();
        assert!(v
            .rows
            .iter()
            .all(|r| r.change.is_some() == (r.mark != Mark::Same)));
        let mut distinct: Vec<u32> = numbers.iter().flatten().copied().collect();
        distinct.dedup();
        assert_eq!(distinct, [0, 1, 2], "no gaps, in order: {numbers:?}");
        // "b" has its lines in one number, "c" (replaced) in another.
        let of = |line: &str| {
            v.rows
                .iter()
                .find(|r| r.left.as_deref().is_some_and(|l| l.contains(line)))
                .and_then(|r| r.change)
        };
        assert_eq!(of("\"a\": 1"), Some(0));
        assert_eq!(of("\"x\""), Some(1));
        assert_eq!(of("\"y\""), Some(2));
        assert_eq!(numbered(&v), [(0, 1), (1, 3)]);
    }

    #[test]
    fn a_changed_value_moves_to_either_side() {
        let (left, right) = (parse(r#"{"a":1,"b":2}"#), parse(r#"{"a":1,"b":3}"#));
        let v = view(&left, &right);
        let block = &v.changes_of(&v.blocks[0]);
        assert_eq!(*block, 0..1);
        assert_eq!(taken(&left, &right, 0..1, Side::Left), right);
        assert_eq!(taken(&left, &right, 0..1, Side::Right), left);
        // Nothing taken: the document as it was.
        assert_eq!(taken(&left, &right, 0..0, Side::Left), left);
        assert_eq!(taken(&left, &right, 1..5, Side::Right), right);
    }

    #[test]
    fn a_member_one_side_lacks_is_put_in_or_taken_out() {
        let (left, right) = (parse(r#"{"a":1,"b":2}"#), parse(r#"{"a":1,"c":3}"#));
        // b is only in the left, c only in the right. The view puts c, which
        // follows a on the right, before b: c is 0 and b is 1, in one block.
        let v = view(&left, &right);
        assert_eq!(numbered(&v), [(0, 2)]);
        // Moving b to the right puts it into the right document…
        assert_eq!(
            taken(&left, &right, 1..2, Side::Right),
            parse(r#"{"a":1,"b":2,"c":3}"#)
        );
        // …moving it to the left takes it out of the left one.
        assert_eq!(taken(&left, &right, 1..2, Side::Left), parse(r#"{"a":1}"#));
        // c the other way round.
        assert_eq!(
            taken(&left, &right, 0..1, Side::Left),
            parse(r#"{"a":1,"b":2,"c":3}"#)
        );
        assert_eq!(taken(&left, &right, 0..1, Side::Right), parse(r#"{"a":1}"#));
    }

    #[test]
    fn one_difference_among_several_moves_and_the_others_stay() {
        let left = parse(r#"{"v":"1","n":3,"f":{"t":false},"gone":null,"r":["a","c"]}"#);
        let right = parse(r#"{"v":"2","n":3,"f":{"t":true,"x":5},"r":["a","b","c"]}"#);
        let v = view(&left, &right);
        // v changed; f/t changed and f/x added (they touch, so one block); gone
        // removed; r/1 added (a line that is the same lies between those two).
        assert_eq!(numbered(&v), [(0, 1), (1, 3), (3, 4), (4, 5)]);
        let into_left = |block: usize| {
            let mut n = taken(&left, &right, v.changes_of(&v.blocks[block]), Side::Left);
            // What is left over, compared with the right document.
            let rest = diff(&n, &right, &AtomicBool::new(false)).unwrap();
            let before = diff(&left, &right, &AtomicBool::new(false)).unwrap();
            assert!(rest.total() < before.total(), "block {block}: {n}");
            std::mem::take(&mut n)
        };
        assert_eq!(
            into_left(0),
            parse(r#"{"v":"2","n":3,"f":{"t":false},"gone":null,"r":["a","c"]}"#)
        );
        assert_eq!(
            into_left(1),
            parse(r#"{"v":"1","n":3,"f":{"t":true,"x":5},"gone":null,"r":["a","c"]}"#)
        );
        assert_eq!(
            into_left(2),
            parse(r#"{"v":"1","n":3,"f":{"t":false},"r":["a","c"]}"#)
        );
        assert_eq!(
            into_left(3),
            parse(r#"{"v":"1","n":3,"f":{"t":false},"gone":null,"r":["a","b","c"]}"#)
        );
        let into_right =
            |block: usize| taken(&left, &right, v.changes_of(&v.blocks[block]), Side::Right);
        assert_eq!(
            into_right(0),
            parse(r#"{"v":"1","n":3,"f":{"t":true,"x":5},"r":["a","b","c"]}"#)
        );
        assert_eq!(
            into_right(2),
            parse(r#"{"v":"2","n":3,"f":{"t":true,"x":5},"gone":null,"r":["a","b","c"]}"#)
        );
        assert_eq!(
            into_right(3),
            parse(r#"{"v":"2","n":3,"f":{"t":true,"x":5},"r":["a","c"]}"#)
        );
    }

    #[test]
    fn an_element_goes_in_or_out_at_its_place_in_the_array() {
        let (left, right) = (json!([1, 2, 3, 4]), json!([1, "x", 2, 3, "y", 4]));
        let v = view(&left, &right);
        assert_eq!(numbered(&v), [(0, 1), (1, 2)]);
        assert_eq!(
            taken(&left, &right, 0..1, Side::Left),
            json!([1, "x", 2, 3, 4])
        );
        assert_eq!(
            taken(&left, &right, 1..2, Side::Left),
            json!([1, 2, 3, "y", 4])
        );
        assert_eq!(
            taken(&left, &right, 1..2, Side::Right),
            json!([1, "x", 2, 3, 4])
        );
        assert_eq!(
            taken(&left, &right, 0..2, Side::Left),
            json!([1, "x", 2, 3, "y", 4])
        );
    }

    #[test]
    fn what_is_not_moved_is_kept_as_it_was_written() {
        // 1.0 and 1 are the same number, so the right one keeps its spelling
        // and the order of its members, whatever the left one has.
        let left = parse(r#"{"a":1,"b":2,"c":3}"#);
        let right = parse(r#"{"c":3.0,"b":9,"a":1.0}"#);
        let moved = taken(&left, &right, 0..1, Side::Right);
        assert_eq!(
            serde_json::to_string(&moved).unwrap(),
            r#"{"c":3.0,"b":2,"a":1.0}"#
        );
        // A member that comes in from the other side goes after the one before it.
        let left = parse(r#"{"a":1,"b":2,"c":3}"#);
        let right = parse(r#"{"c":3,"a":1}"#);
        let moved = taken(&left, &right, 0..1, Side::Right);
        assert_eq!(
            serde_json::to_string(&moved).unwrap(),
            r#"{"c":3,"a":1,"b":2}"#
        );
    }

    #[test]
    fn a_value_of_another_kind_and_the_whole_document_move_too() {
        let (left, right) = (json!({"a": [1, 2]}), json!({"a": 7}));
        assert_eq!(taken(&left, &right, 0..1, Side::Left), right);
        assert_eq!(taken(&left, &right, 0..1, Side::Right), left);
        let (left, right) = (json!([1]), json!({"x": 1}));
        let v = view(&left, &right);
        assert_eq!(numbered(&v), [(0, 1)]);
        assert_eq!(taken(&left, &right, 0..1, Side::Left), right);
        assert_eq!(taken(&left, &right, 0..1, Side::Right), left);
        // An empty container against a full one has the members come one by one.
        let (left, right) = (json!({}), json!({"a": 1, "b": 2}));
        let v = view(&left, &right);
        assert_eq!(numbered(&v), [(0, 2)]);
        assert_eq!(taken(&left, &right, 1..2, Side::Left), json!({"b": 2}));
    }

    #[test]
    fn differences_that_are_not_one_run_are_taken_together_and_the_ones_between_stay() {
        // Three changes that touch, so that the lines of one block are three differences;
        // the second is not taken.
        let (left, right) = (
            parse(r#"{"a":1,"b":2,"c":3,"s":0}"#),
            parse(r#"{"a":9,"b":8,"c":7,"s":0}"#),
        );
        let v = view(&left, &right);
        assert_eq!(numbered(&v), [(0, 3)]);
        let pick = |changes: &[Range<u32>], into| {
            take_selected(&left, &right, changes, into, &AtomicBool::new(false)).unwrap()
        };
        assert_eq!(
            pick(&[0..1, 2..3], Side::Left),
            parse(r#"{"a":9,"b":2,"c":7,"s":0}"#)
        );
        assert_eq!(
            pick(&[0..1, 2..3], Side::Right),
            parse(r#"{"a":1,"b":8,"c":3,"s":0}"#)
        );
        // One line of the three, any of them.
        assert_eq!(
            pick(&[1..2], Side::Left),
            parse(r#"{"a":1,"b":8,"c":3,"s":0}"#)
        );
        assert_eq!(
            pick(&[2..3], Side::Right),
            parse(r#"{"a":9,"b":8,"c":3,"s":0}"#)
        );
        // The order the ranges are given in, and ranges that overlap or touch, make no
        // difference; nor does an empty one.
        assert_eq!(
            pick(&[2..3, 0..1], Side::Left),
            pick(&[0..1, 2..3], Side::Left)
        );
        assert_eq!(pick(&[0..2, 1..3], Side::Left), right);
        assert_eq!(pick(&[0..1, 1..2, 2..3, 7..7], Side::Left), right);
        assert_eq!(pick(&[], Side::Left), left);
        assert_eq!(pick(&[4..4], Side::Right), right);
        // One run is what take_changes does.
        assert_eq!(
            pick(&[0..2], Side::Left),
            taken(&left, &right, 0..2, Side::Left)
        );
    }

    #[test]
    fn ranges_of_numbers_are_made_into_the_fewest_runs() {
        assert_eq!(runs(&[3..5, 0..2, 2..3, 4..9, 11..11]), [0..9]);
        assert_eq!(runs(&[5..6, 1..2]), [1..2, 5..6]);
        assert_eq!(runs(&[0..10, 2..3]), [0..10]);
        assert_eq!(runs(&[3..3, 0..0]), Vec::<Range<u32>>::new());
        assert_eq!(runs(&[]), Vec::<Range<u32>>::new());
    }

    #[test]
    fn a_move_can_be_cancelled() {
        let (left, right) = (json!([1, 2]), json!([1, 3]));
        assert!(take_changes(&left, &right, 0..1, Side::Left, &AtomicBool::new(true)).is_err());
    }

    // Round trips over generated documents, as in the patch tests: with few
    // distinct values, so that equal elements, duplicates and reorderings come
    // up all the time.

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }

    fn generate(rng: &mut Rng, depth: u32) -> Value {
        match rng.below(if depth == 0 { 4 } else { 6 }) {
            0 => json!(rng.below(4)),
            1 => json!(["a", "b", "c"][rng.below(3) as usize]),
            2 => json!(rng.below(2) == 0),
            3 => Value::Null,
            4 => Value::Array(
                (0..rng.below(7))
                    .map(|_| generate(rng, depth - 1))
                    .collect(),
            ),
            _ => {
                let mut members = Map::new();
                for _ in 0..rng.below(4) {
                    members.insert(format!("k{}", rng.below(5)), generate(rng, depth - 1));
                }
                Value::Object(members)
            }
        }
    }

    fn mutate(rng: &mut Rng, value: &mut Value) {
        match value {
            Value::Array(items) => match rng.below(5) {
                0 if !items.is_empty() => {
                    let at = rng.below(items.len() as u64) as usize;
                    items.remove(at);
                }
                1 => {
                    let at = rng.below(items.len() as u64 + 1) as usize;
                    items.insert(at, generate(rng, 1));
                }
                2 if !items.is_empty() => {
                    let at = rng.below(items.len() as u64) as usize;
                    mutate(rng, &mut items[at]);
                }
                3 => items.reverse(),
                _ => {}
            },
            Value::Object(members) => match rng.below(4) {
                0 if !members.is_empty() => {
                    let at = rng.below(members.len() as u64) as usize;
                    let key = members.keys().nth(at).unwrap().clone();
                    members.shift_remove(&key);
                }
                1 => {
                    members.insert(format!("k{}", rng.below(5)), generate(rng, 1));
                }
                2 if !members.is_empty() => {
                    let at = rng.below(members.len() as u64) as usize;
                    let key = members.keys().nth(at).unwrap().clone();
                    mutate(rng, members.get_mut(&key).unwrap());
                }
                _ => {}
            },
            other => *other = generate(rng, 1),
        }
    }

    #[test]
    fn any_selection_of_the_differences_moves_and_leaves_the_documents_closer() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let stop = AtomicBool::new(false);
        for case in 0..3_000 {
            let before = generate(&mut rng, 3);
            let mut after = before.clone();
            for _ in 0..1 + rng.below(4) {
                mutate(&mut rng, &mut after);
            }
            let found = diff(&before, &after, &stop).unwrap();
            let total = found.total() as u32;
            if total == 0 {
                continue;
            }
            let context = format!("case {case}\n  before {before}\n  after  {after}");
            // A few selections of the numbers: each by a mask of which are taken.
            for _ in 0..4 {
                let mask = rng.below(1 << total.min(12)) as u32;
                let runs: Vec<Range<u32>> = (0..total.min(12))
                    .filter(|n| mask & (1 << n) != 0)
                    .map(|n| n..n + 1)
                    .collect();
                for into in [Side::Left, Side::Right] {
                    let moved = take_selected(&before, &after, &runs, into, &stop).unwrap();
                    let (a, b) = match into {
                        Side::Left => (&moved, &after),
                        Side::Right => (&before, &moved),
                    };
                    let rest = diff(a, b, &stop).unwrap().total();
                    if runs.is_empty() {
                        assert!(equal(a, &before) && equal(b, &after), "{context}");
                    } else {
                        assert!(
                            rest < found.total(),
                            "{context}\n  taking {runs:?} into {into:?}: {moved} ({rest} left of {})",
                            found.total()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn moving_differences_between_generated_documents_comes_out_right() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let stop = AtomicBool::new(false);
        for case in 0..4_000 {
            let before = generate(&mut rng, 3);
            let mut after = before.clone();
            for _ in 0..1 + rng.below(3) {
                mutate(&mut rng, &mut after);
            }
            let found = diff(&before, &after, &stop).unwrap();
            let v = view(&before, &after);
            let context = format!("case {case}\n  before {before}\n  after  {after}");

            // The numbers are the rows' own, with none missing, and are as many as
            // the differences.
            let mut seen: Vec<u32> = v.rows.iter().filter_map(|r| r.change).collect();
            seen.dedup();
            assert_eq!(
                seen,
                (0..found.total() as u32).collect::<Vec<_>>(),
                "{context}"
            );

            // All of them, one way or the other, makes the two the same; none
            // changes nothing.
            let all = 0..found.total() as u32;
            let left = taken(&before, &after, all.clone(), Side::Left);
            let right = taken(&before, &after, all, Side::Right);
            assert!(equal(&left, &after), "{context}\n  left  {left}");
            assert!(equal(&right, &before), "{context}\n  right {right}");
            assert_eq!(
                taken(&before, &after, 0..0, Side::Left),
                before,
                "{context}"
            );
            assert_eq!(
                taken(&before, &after, 0..0, Side::Right),
                after,
                "{context}"
            );

            // Each block moved by itself leaves the documents closer than they
            // were, and is a value that can be moved back.
            for block in &v.blocks {
                let numbers = v.changes_of(block);
                for into in [Side::Left, Side::Right] {
                    let moved = taken(&before, &after, numbers.clone(), into);
                    let (a, b) = match into {
                        Side::Left => (&moved, &after),
                        Side::Right => (&before, &moved),
                    };
                    let rest = diff(a, b, &stop).unwrap().total();
                    assert!(
                        rest < found.total(),
                        "{context}\n  block {numbers:?} into {into:?}: {moved} ({rest} left of {})",
                        found.total()
                    );
                }
            }
        }
    }
}
