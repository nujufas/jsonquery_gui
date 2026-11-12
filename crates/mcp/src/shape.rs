//! What the tools give of a value, kept to a size an assistant can take in: the value
//! at a JSON Pointer, a page of it ([`read`], [`page`]), and its shape without its data
//! ([`describe`]).
//!
//! They work on anything that is a [`ValueView`]: a value in memory, or a document
//! that is kept as its file and read as it is looked at — which can be gigabytes, so
//! nothing here walks more of a value than the answer needs.

use std::collections::HashMap;

use jsonquery_core::{NodePath, PathSegment, ValueKind, ValueView};
use serde_json::{json, Map, Value};

/// How many values [`describe`] looks at in all, however big the value is.
const DESCRIBE_BUDGET: usize = 50_000;
/// How many keys of one object it names; the rest are counted.
const MAX_KEYS: usize = 60;
/// How many example strings it gives of a place.
const MAX_EXAMPLES: usize = 3;
/// How long an example string or a preview may be, in characters.
const EXAMPLE_CHARS: usize = 60;
const PREVIEW_CHARS: usize = 200;

/// The value at `pointer` (RFC 6901) below `root`, and its path.
pub fn resolve_pointer<V: ValueView + Copy>(
    root: V,
    pointer: &str,
) -> Result<(V, NodePath), String> {
    if pointer.is_empty() {
        return Ok((root, Vec::new()));
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err(format!(
            "\"{pointer}\" is not a JSON Pointer: it starts with \"/\""
        ));
    };
    let mut at = root;
    let mut path = Vec::new();
    for raw in rest.split('/') {
        let token = unescape(raw).ok_or_else(|| {
            format!("\"{pointer}\" is not a JSON Pointer: \"~\" is written \"~0\"")
        })?;
        let found = match at.kind() {
            ValueKind::Object => at
                .child_by_key(&token)
                .map(|child| (child, PathSegment::Key(token.clone()))),
            ValueKind::Array => index_of(&token)
                .and_then(|i| at.child_at(i).map(|child| (child, PathSegment::Index(i)))),
            _ => None,
        };
        let Some((child, segment)) = found else {
            let what = match at.kind() {
                ValueKind::Object => format!("has no member \"{token}\""),
                ValueKind::Array => format!(
                    "has no item \"{token}\" (it has {} items, from 0)",
                    at.child_count()
                ),
                kind => format!("is {}, with nothing inside", kind_name(kind)),
            };
            return Err(format!(
                "nothing at \"{pointer}\": \"{}\" {what}",
                pointer_of(&path)
            ));
        };
        at = child;
        path.push(segment);
    }
    Ok((at, path))
}

/// The JSON Pointer of `path`.
pub fn pointer_of(path: &[PathSegment]) -> String {
    let mut out = String::new();
    for segment in path {
        out.push('/');
        match segment {
            PathSegment::Key(key) => out.push_str(&key.replace('~', "~0").replace('/', "~1")),
            PathSegment::Index(i) => out.push_str(&i.to_string()),
        }
    }
    out
}

fn unescape(token: &str) -> Option<String> {
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.next() {
                Some('0') => out.push('~'),
                Some('1') => out.push('/'),
                _ => return None,
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// An array index as a pointer writes it: digits, without a leading zero.
fn index_of(token: &str) -> Option<usize> {
    if token.is_empty()
        || !token.bytes().all(|b| b.is_ascii_digit())
        || (token.len() > 1 && token.starts_with('0'))
    {
        return None;
    }
    token.parse().ok()
}

pub fn kind_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Null => "null",
        ValueKind::Bool => "boolean",
        ValueKind::Number => "number",
        ValueKind::String => "string",
        ValueKind::Array => "array",
        ValueKind::Object => "object",
    }
}

/// What a value is, in a few words: "object with 3 members", "string".
pub fn summary<V: ValueView>(value: &V) -> String {
    match value.kind() {
        ValueKind::Array => count_of(value.child_count(), "item"),
        ValueKind::Object => count_of(value.child_count(), "member"),
        kind => kind_name(kind).to_owned(),
    }
}

fn count_of(n: usize, what: &str) -> String {
    let container = if what == "item" { "array" } else { "object" };
    if n == 1 {
        format!("{container} with 1 {what}")
    } else {
        format!("{container} with {n} {what}s")
    }
}

/// `value` as JSON, if it takes no more than `budget` bytes written compactly; the
/// budget is what is left of it. Walks no more of the value than that.
fn whole<V: ValueView>(value: &V, budget: &mut usize) -> Option<Value> {
    fn take(budget: &mut usize, n: usize) -> Option<()> {
        *budget = budget.checked_sub(n)?;
        Some(())
    }
    match value.kind() {
        ValueKind::Array => {
            take(budget, 2)?;
            let mut items = Vec::new();
            for (i, (_, child)) in value.iter_children().enumerate() {
                if i > 0 {
                    take(budget, 1)?;
                }
                items.push(whole(&child, budget)?);
            }
            Some(Value::Array(items))
        }
        ValueKind::Object => {
            take(budget, 2)?;
            let mut members = Map::new();
            for (i, (segment, child)) in value.iter_children().enumerate() {
                if i > 0 {
                    take(budget, 1)?;
                }
                let Some(PathSegment::Key(key)) = segment else {
                    continue;
                };
                take(budget, json_len(&key) + 1)?;
                let child = whole(&child, budget)?;
                members.insert(key, child);
            }
            Some(Value::Object(members))
        }
        _ => {
            let scalar = value.scalar_value()?;
            take(budget, scalar.to_string().len())?;
            Some(scalar)
        }
    }
}

fn json_len(text: &str) -> usize {
    Value::String(text.to_owned()).to_string().len()
}

/// What stands for a part that is too big to show: what it is, and where.
fn omitted<V: ValueView>(value: &V, pointer: &str) -> Value {
    let mut out = Map::new();
    let what = match value.kind() {
        ValueKind::String => {
            let preview = value.scalar_preview().unwrap_or_default();
            let start: String = preview.chars().take(PREVIEW_CHARS).collect();
            out.insert("starts".to_owned(), Value::String(start));
            "long string".to_owned()
        }
        _ => summary(value),
    };
    out.insert("$omitted".to_owned(), Value::String(what));
    out.insert("pointer".to_owned(), Value::String(pointer.to_owned()));
    Value::Object(out)
}

/// `value` cut down to about `budget` bytes: what fits whole is shown whole, each part
/// that does not is [`omitted`], and what is left over once the budget runs out is
/// counted.
fn brief<V: ValueView>(value: &V, pointer: &str, budget: usize) -> Value {
    let mut left = budget;
    if let Some(all) = whole(value, &mut left) {
        return all;
    }
    if !value.kind().is_container() {
        return omitted(value, pointer);
    }
    let mut left = budget;
    let mut items = Vec::new();
    let mut members = Map::new();
    let mut shown = 0;
    for (segment, child) in value.iter_children() {
        let Some(segment) = segment else { continue };
        let child_pointer = format!("{pointer}{}", pointer_of(std::slice::from_ref(&segment)));
        let mut room = left;
        let shown_child =
            whole(&child, &mut room).unwrap_or_else(|| omitted(&child, &child_pointer));
        let cost = shown_child.to_string().len() + 1;
        if cost > left && shown > 0 {
            break;
        }
        left = left.saturating_sub(cost);
        match segment {
            PathSegment::Key(key) => {
                members.insert(key, shown_child);
            }
            PathSegment::Index(_) => items.push(shown_child),
        }
        shown += 1;
    }
    let more = value.child_count().saturating_sub(shown);
    if value.kind() == ValueKind::Array {
        if more > 0 {
            items.push(json!({"$more": format!("{more} more items"), "pointer": pointer}));
        }
        Value::Array(items)
    } else {
        if more > 0 {
            members.insert(
                "$more".to_owned(),
                json!(format!(
                    "{more} more members: read {pointer:?} with an offset"
                )),
            );
        }
        Value::Object(members)
    }
}

/// The value at `path` (whose pointer is that of `path`), a page at a time: an array's
/// items or an object's members from `offset`, at most `limit` of them and about
/// `max_bytes` of JSON; a scalar whole, or the start of a string too long for that.
pub fn read<V: ValueView + Copy>(
    value: V,
    path: &NodePath,
    offset: usize,
    limit: usize,
    max_bytes: usize,
) -> Value {
    let pointer = pointer_of(path);
    let mut out = Map::new();
    out.insert("pointer".to_owned(), Value::String(pointer.clone()));
    out.insert(
        "kind".to_owned(),
        Value::String(kind_name(value.kind()).to_owned()),
    );
    if !value.kind().is_container() {
        out.insert("value".to_owned(), brief(&value, &pointer, max_bytes));
        return Value::Object(out);
    }
    let size = value.child_count();
    out.insert("size".to_owned(), json!(size));
    out.insert("offset".to_owned(), json!(offset));
    let mut items = Vec::new();
    let mut members = Map::new();
    let mut left = max_bytes;
    let mut next = None;
    for (n, (segment, child)) in value.iter_children_from(offset).take(limit).enumerate() {
        let Some(segment) = segment else { continue };
        let child_pointer = format!("{pointer}{}", pointer_of(std::slice::from_ref(&segment)));
        let mut room = left;
        let shown = match whole(&child, &mut room) {
            Some(all) => {
                left = room.saturating_sub(1);
                all
            }
            // The first is shown as much as fits; after it, the page ends here.
            None if n == 0 => {
                left = 0;
                next = Some(offset + 1);
                brief(&child, &child_pointer, max_bytes)
            }
            None => {
                next = Some(offset + n);
                break;
            }
        };
        match segment {
            PathSegment::Key(key) => {
                let key_cost = json_len(&key) + 1;
                left = left.saturating_sub(key_cost);
                members.insert(key, shown);
            }
            PathSegment::Index(_) => items.push(shown),
        }
        if next.is_some() {
            break;
        }
    }
    let shown = items.len() + members.len();
    let next = next.or_else(|| (offset + shown < size).then_some(offset + shown));
    if value.kind() == ValueKind::Array {
        out.insert("items".to_owned(), Value::Array(items));
    } else {
        out.insert("members".to_owned(), Value::Object(members));
    }
    if let Some(next) = next {
        out.insert("next_offset".to_owned(), json!(next));
    }
    Value::Object(out)
}

/// A page of `values` (the items of a query's results, the first of which is item
/// `first`): from item `offset`, at most `limit` of them and about `max_bytes` of
/// JSON. Gives the page and where the next one starts, if the page ended before
/// `values` did.
pub fn page(
    values: &[Value],
    first: usize,
    offset: usize,
    limit: usize,
    max_bytes: usize,
) -> (Vec<Value>, Option<usize>) {
    let mut out = Vec::new();
    let mut left = max_bytes;
    let start = offset.saturating_sub(first);
    for (n, value) in values.iter().skip(start).take(limit).enumerate() {
        let mut room = left;
        match whole(&value, &mut room) {
            Some(all) => {
                left = room.saturating_sub(1);
                out.push(all);
            }
            None if n == 0 => {
                out.push(brief(&value, &format!("/{}", offset), max_bytes));
                return (out, Some(offset + 1));
            }
            None => return (out, Some(offset + n)),
        }
    }
    let end = offset + out.len();
    (out, (end < first + values.len()).then_some(end))
}

/// The shape of `value`: what kind it is, and for a container what is in it, `depth`
/// levels down, from `sample` items of each long array (the first half of them, and
/// the rest spread over the array); with the range of numbers and a few example
/// strings. Fewer levels, if that many take more than about `max_bytes` to say.
pub fn describe<V: ValueView + Copy>(
    value: V,
    depth: usize,
    sample: usize,
    max_bytes: usize,
) -> Value {
    let asked = depth.max(1);
    let mut depth = asked;
    loop {
        let mut budget = DESCRIBE_BUDGET;
        let mut shape = Shape::default();
        shape.add(value, depth, sample.max(1), &mut budget);
        let mut out = shape.to_json();
        let text = out.to_string();
        // One level always fits: what is inside is not said.
        if depth > 1 && text.len() > max_bytes {
            depth -= 1;
            continue;
        }
        if let Value::Object(map) = &mut out {
            if text.contains("\"deeper\":true") {
                map.insert(
                    "deeper_parts".to_owned(),
                    json!(
                        "\"deeper\": true marks where it was not looked inside: describe \
                           that part by its pointer"
                    ),
                );
            }
            if budget == 0 {
                map.insert(
                    "note".to_owned(),
                    json!(format!(
                        "described from the first {DESCRIBE_BUDGET} values looked at; describe \
                         a part of it (with `pointer`) for more"
                    )),
                );
            }
            if depth < asked {
                map.insert(
                    "depth".to_owned(),
                    json!(format!(
                        "{depth} levels: {asked} did not fit in a page; describe a part (with \
                         `pointer`) to see further down"
                    )),
                );
            }
        }
        return out;
    }
}

/// What is known of the values found at one place.
#[derive(Default)]
struct Shape {
    /// How many values of each kind (in the order of `KINDS`).
    kinds: [usize; 6],
    examples: Vec<String>,
    number_range: Option<(f64, f64)>,
    fractions: bool,
    lengths: Option<(usize, usize)>,
    items: Option<Box<Shape>>,
    /// Items of arrays that were looked at, and that there were.
    items_seen: usize,
    items_there: usize,
    keys: Vec<(String, Shape)>,
    key_index: HashMap<String, usize>,
    /// Keys not named, past `MAX_KEYS`.
    more_keys: usize,
    objects: usize,
    /// Members of objects that were looked at, and that there were.
    members_seen: usize,
    members_there: usize,
    /// What is inside was not looked at: the depth ran out.
    deeper: bool,
}

const KINDS: [ValueKind; 6] = [
    ValueKind::Null,
    ValueKind::Bool,
    ValueKind::Number,
    ValueKind::String,
    ValueKind::Array,
    ValueKind::Object,
];

impl Shape {
    fn add<V: ValueView + Copy>(
        &mut self,
        value: V,
        depth: usize,
        sample: usize,
        budget: &mut usize,
    ) {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let kind = value.kind();
        let slot = KINDS.iter().position(|k| *k == kind).unwrap_or(0);
        self.kinds[slot] += 1;
        let inner = (sample / 4).max(10);
        match kind {
            ValueKind::Number => {
                if let Some(Value::Number(n)) = value.scalar_value() {
                    if !(n.is_i64() || n.is_u64()) {
                        self.fractions = true;
                    }
                    if let Some(f) = n.as_f64() {
                        let (lo, hi) = self.number_range.get_or_insert((f, f));
                        *lo = lo.min(f);
                        *hi = hi.max(f);
                    }
                }
            }
            ValueKind::String => {
                if self.examples.len() < MAX_EXAMPLES {
                    let text = match value.scalar_value() {
                        Some(Value::String(s)) => s,
                        _ => value.scalar_preview().unwrap_or_default(),
                    };
                    let mut example: String = text.chars().take(EXAMPLE_CHARS).collect();
                    if example.len() < text.len() {
                        example.push('…');
                    }
                    if !self.examples.contains(&example) {
                        self.examples.push(example);
                    }
                }
            }
            ValueKind::Array => {
                let n = value.child_count();
                let (lo, hi) = self.lengths.get_or_insert((n, n));
                *lo = (*lo).min(n);
                *hi = (*hi).max(n);
                self.items_there += n;
                if depth <= 1 {
                    self.deeper |= n > 0;
                    return;
                }
                let items = self.items.get_or_insert_with(Box::default);
                for index in sample_indices(n, sample) {
                    if *budget == 0 {
                        break;
                    }
                    if let Some(child) = value.child_at(index) {
                        self.items_seen += 1;
                        items.add(child, depth - 1, inner, budget);
                    }
                }
            }
            ValueKind::Object => {
                self.objects += 1;
                let n = value.child_count();
                self.members_there += n;
                if depth <= 1 {
                    self.deeper |= n > 0;
                    return;
                }
                for (segment, child) in value.iter_children().take(sample.max(MAX_KEYS)) {
                    if *budget == 0 {
                        break;
                    }
                    let Some(PathSegment::Key(key)) = segment else {
                        continue;
                    };
                    self.members_seen += 1;
                    let at = match self.key_index.get(&key) {
                        Some(at) => *at,
                        None if self.keys.len() < MAX_KEYS => {
                            self.key_index.insert(key.clone(), self.keys.len());
                            self.keys.push((key, Shape::default()));
                            self.keys.len() - 1
                        }
                        None => {
                            self.more_keys += 1;
                            continue;
                        }
                    };
                    self.keys[at].1.add(child, depth - 1, inner, budget);
                }
            }
            ValueKind::Null | ValueKind::Bool => {}
        }
    }

    fn seen(&self) -> usize {
        self.kinds.iter().sum()
    }

    fn to_json(&self) -> Value {
        let mut out = Map::new();
        let kinds: Vec<(ValueKind, usize)> = KINDS
            .iter()
            .zip(self.kinds)
            .filter(|(_, n)| *n > 0)
            .map(|(k, n)| (*k, n))
            .collect();
        match kinds.as_slice() {
            [] => {}
            [(kind, _)] => {
                out.insert("type".to_owned(), json!(kind_name(*kind)));
            }
            several => {
                let mut types = Map::new();
                for (kind, n) in several {
                    types.insert(kind_name(*kind).to_owned(), json!(n));
                }
                out.insert("types".to_owned(), Value::Object(types));
            }
        }
        if let Some((lo, hi)) = self.lengths {
            out.insert(
                "length".to_owned(),
                if lo == hi {
                    json!(lo)
                } else {
                    json!({"min": lo, "max": hi})
                },
            );
        }
        if let Some((lo, hi)) = self.number_range {
            let number = |f: f64| {
                if !self.fractions && f.fract() == 0.0 && f.abs() < 9.0e15 {
                    json!(f as i64)
                } else {
                    json!(f)
                }
            };
            out.insert("range".to_owned(), json!([number(lo), number(hi)]));
            if !self.fractions {
                out.insert("integers".to_owned(), json!(true));
            }
        }
        if !self.examples.is_empty() {
            out.insert("examples".to_owned(), json!(self.examples));
        }
        if let Some(items) = self.items.as_ref().filter(|i| i.seen() > 0) {
            if self.items_seen < self.items_there {
                out.insert(
                    "sampled".to_owned(),
                    json!(format!("{} of {} items", self.items_seen, self.items_there)),
                );
            }
            out.insert("items".to_owned(), items.to_json());
        }
        if !self.keys.is_empty() {
            let mut keys = Map::new();
            for (key, shape) in &self.keys {
                let mut shown = shape.to_json();
                let present = shape.seen();
                if present < self.objects && self.members_seen >= self.members_there {
                    if let Value::Object(map) = &mut shown {
                        map.insert(
                            "in".to_owned(),
                            json!(format!("{present} of {}", self.objects)),
                        );
                    }
                }
                keys.insert(key.clone(), shown);
            }
            out.insert("keys".to_owned(), Value::Object(keys));
            if self.more_keys > 0 {
                out.insert("more_keys".to_owned(), json!(self.more_keys));
            }
            if self.members_seen < self.members_there {
                out.insert(
                    "members_sampled".to_owned(),
                    json!(format!(
                        "{} of {} members",
                        self.members_seen, self.members_there
                    )),
                );
            }
        }
        if self.deeper {
            out.insert("deeper".to_owned(), json!(true));
        }
        Value::Object(out)
    }
}

/// What a walk over all of a value found ([`tally`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tally {
    /// How many values of each kind, in the order of `KINDS` (null, boolean, number,
    /// string, array, object).
    pub kinds: [usize; 6],
    /// How many levels down the deepest value is (0: the value has nothing inside).
    pub depth: usize,
    /// The longest arrays and the widest objects, the biggest first: their pointers,
    /// and how many items or members each has.
    pub longest_arrays: Vec<(String, usize)>,
    pub widest_objects: Vec<(String, usize)>,
    /// All of it was looked at: the walk was not stopped.
    pub complete: bool,
}

impl Tally {
    pub fn values(&self) -> usize {
        self.kinds.iter().sum()
    }

    /// The counts, by kind, as the assistant is given them.
    pub fn counts(&self) -> Value {
        let mut out = Map::new();
        out.insert("all".to_owned(), json!(self.values()));
        for (kind, n) in KINDS.iter().zip(self.kinds) {
            let name = match kind {
                ValueKind::Null => "nulls",
                ValueKind::Bool => "booleans",
                ValueKind::Number => "numbers",
                ValueKind::String => "strings",
                ValueKind::Array => "arrays",
                ValueKind::Object => "objects",
            };
            out.insert(name.to_owned(), json!(n));
        }
        Value::Object(out)
    }
}

/// How many of the longest arrays and the widest objects a tally keeps.
const BIGGEST: usize = 5;
/// How deep a tally goes (a value nested deeper is counted, not looked inside).
const TALLY_DEPTH: usize = 1_000;

/// Walk all of `value` (at `path`), counting what is in it, until `stop` says to stop
/// (it is asked every few thousand values).
pub fn tally<V: ValueView + Copy>(value: V, path: &NodePath, stop: &dyn Fn() -> bool) -> Tally {
    let mut tally = Tally {
        complete: true,
        ..Tally::default()
    };
    let mut at = path.clone();
    walk(value, &mut at, 0, &mut tally, stop);
    tally
}

fn walk<V: ValueView + Copy>(
    value: V,
    at: &mut NodePath,
    depth: usize,
    tally: &mut Tally,
    stop: &dyn Fn() -> bool,
) {
    if tally.values() % 4_096 == 4_095 && stop() {
        tally.complete = false;
    }
    if !tally.complete {
        return;
    }
    let kind = value.kind();
    let slot = KINDS.iter().position(|k| *k == kind).unwrap_or(0);
    tally.kinds[slot] += 1;
    tally.depth = tally.depth.max(depth);
    let biggest = match kind {
        ValueKind::Array => &mut tally.longest_arrays,
        ValueKind::Object => &mut tally.widest_objects,
        _ => return,
    };
    let n = value.child_count();
    if n > 0 && (biggest.len() < BIGGEST || biggest.last().is_some_and(|(_, m)| n > *m)) {
        biggest.push((pointer_of(at), n));
        biggest.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        biggest.truncate(BIGGEST);
    }
    if depth >= TALLY_DEPTH {
        return;
    }
    for (segment, child) in value.iter_children() {
        let Some(segment) = segment else { continue };
        at.push(segment);
        walk(child, at, depth + 1, tally, stop);
        at.pop();
        if !tally.complete {
            return;
        }
    }
}

/// Which items of an array of `n` to look at: all, when they are no more than
/// `sample`; else the first half of the sample, and the rest spread over the others.
fn sample_indices(n: usize, sample: usize) -> Vec<usize> {
    if n <= sample {
        return (0..n).collect();
    }
    let first = sample / 2;
    let spread = sample - first;
    let mut out: Vec<usize> = (0..first).collect();
    let rest = n - first;
    for k in 0..spread {
        out.push(first + k * rest / spread);
    }
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonquery_core::{load_with, LoadLimits, Root};

    fn people() -> Value {
        json!({
            "users": [
                {"id": 1, "name": "Ann", "tags": ["a", "b"], "email": "ann@example.com"},
                {"id": 2, "name": "Bo", "tags": [], "email": null},
                {"id": 3, "name": "Cy", "tags": ["c"]}
            ],
            "meta": {"count": 3, "a/b": true, "m~n": 1.5}
        })
    }

    #[test]
    fn a_pointer_finds_its_value_and_says_where_it_went_wrong() {
        let doc = people();
        let (value, path) = resolve_pointer(&doc, "/users/1/name").unwrap();
        assert_eq!(value, &json!("Bo"));
        assert_eq!(pointer_of(&path), "/users/1/name");
        let (value, path) = resolve_pointer(&doc, "/meta/a~1b").unwrap();
        assert_eq!(value, &json!(true));
        assert_eq!(pointer_of(&path), "/meta/a~1b");
        assert_eq!(resolve_pointer(&doc, "/meta/m~0n").unwrap().0, &json!(1.5));
        assert_eq!(resolve_pointer(&doc, "").unwrap().0, &doc);
        let error = resolve_pointer(&doc, "/users/7").unwrap_err();
        assert!(
            error.contains("\"/users\"") && error.contains("3 items"),
            "{error}"
        );
        assert!(resolve_pointer(&doc, "/users/01").is_err());
        assert!(resolve_pointer(&doc, "/users/-").is_err());
        assert!(resolve_pointer(&doc, "/meta/~2").is_err());
        let error = resolve_pointer(&doc, "/meta/count/x").unwrap_err();
        assert!(error.contains("is number"), "{error}");
    }

    #[test]
    fn read_gives_a_page_and_where_the_next_starts() {
        let doc = people();
        let (users, path) = resolve_pointer(&doc, "/users").unwrap();
        let page = read(users, &path, 0, 2, 10_000);
        assert_eq!(page["size"], 3);
        assert_eq!(page["items"].as_array().unwrap().len(), 2);
        assert_eq!(page["next_offset"], 2);
        let last = read(users, &path, 2, 2, 10_000);
        assert_eq!(last["items"][0]["name"], "Cy");
        assert!(last.get("next_offset").is_none());
    }

    #[test]
    fn read_stops_a_page_at_the_bytes_and_cuts_down_the_first_item_only() {
        let big: Vec<Value> = (0..50)
            .map(|i| json!({"i": i, "text": "x".repeat(400)}))
            .collect();
        let doc = Value::Array(big);
        let page = read(&doc, &Vec::new(), 0, 1000, 2_000);
        let items = page["items"].as_array().unwrap();
        assert!(items.len() >= 3 && items.len() < 6, "{}", items.len());
        assert_eq!(page["next_offset"], items.len());
        // A first item bigger than the page: the parts that fit, and pointers to the rest.
        let doc = json!([{"small": 1, "huge": "y".repeat(5_000)}, 2]);
        let page = read(&doc, &Vec::new(), 0, 10, 1_000);
        assert_eq!(page["items"][0]["small"], 1);
        assert_eq!(page["items"][0]["huge"]["pointer"], "/0/huge");
        assert_eq!(page["items"][0]["huge"]["$omitted"], "long string");
        assert_eq!(page["next_offset"], 1);
    }

    #[test]
    fn read_of_an_object_pages_its_members_in_order() {
        let doc = json!({"a": 1, "b": 2, "c": 3});
        let page = read(&doc, &Vec::new(), 1, 1, 1_000);
        assert_eq!(page["members"], json!({"b": 2}));
        assert_eq!(page["next_offset"], 2);
        let scalar = read(&json!("hi"), &Vec::new(), 0, 1, 1_000);
        assert_eq!(scalar["value"], "hi");
        assert_eq!(scalar["kind"], "string");
    }

    #[test]
    fn page_cuts_results_by_count_and_bytes() {
        let values: Vec<Value> = (0..100).map(|i| json!({"n": i})).collect();
        let (items, next) = page(&values, 0, 10, 5, 10_000);
        assert_eq!(items, values[10..15].to_vec());
        assert_eq!(next, Some(15));
        let (items, next) = page(&values, 0, 95, 50, 10_000);
        assert_eq!(items.len(), 5);
        assert_eq!(next, None);
        let (items, next) = page(&values, 0, 0, 100, 500);
        assert!(items.len() < 100 && next == Some(items.len()));
        // Results that start further on (the window of a run that began at an offset).
        let (items, _) = page(&values[40..60], 40, 45, 2, 10_000);
        assert_eq!(items, values[45..47].to_vec());
    }

    #[test]
    fn describe_tells_keys_kinds_and_what_is_sometimes_missing() {
        let doc = people();
        let shape = describe(&doc, 4, 200, 100_000);
        let user = &shape["keys"]["users"]["items"];
        assert_eq!(shape["keys"]["users"]["length"], 3);
        assert_eq!(user["keys"]["id"]["type"], "number");
        assert_eq!(user["keys"]["id"]["range"], json!([1, 3]));
        assert_eq!(user["keys"]["name"]["examples"], json!(["Ann", "Bo", "Cy"]));
        assert_eq!(
            user["keys"]["email"]["types"],
            json!({"null": 1, "string": 1})
        );
        assert_eq!(user["keys"]["email"]["in"], "2 of 3");
        assert_eq!(user["keys"]["tags"]["length"], json!({"min": 0, "max": 2}));
        assert_eq!(
            shape["keys"]["meta"]["keys"]["m~n"]["range"],
            json!([1.5, 1.5])
        );
    }

    #[test]
    fn describe_samples_long_arrays_and_stops_at_its_depth() {
        let doc = Value::Array(
            (0..10_000)
                .map(|i| json!({"i": i, "v": {"w": [i]}}))
                .collect(),
        );
        let shape = describe(&doc, 3, 100, 100_000);
        assert_eq!(shape["length"], 10_000);
        assert_eq!(shape["sampled"], "100 of 10000 items");
        assert!(shape["items"]["keys"]["i"]["range"][1].as_i64().unwrap() > 9_000);
        assert_eq!(shape["items"]["keys"]["v"]["deeper"], true);
        assert!(shape["deeper_parts"]
            .as_str()
            .unwrap()
            .contains("by its pointer"));
    }

    #[test]
    fn describe_goes_fewer_levels_down_when_more_would_not_fit_a_page() {
        let doc = Value::Object(
            (0..50)
                .map(|k| {
                    (
                        format!("key{k}"),
                        json!({"a": {"b": {"c": [k, "some text"]}}}),
                    )
                })
                .collect(),
        );
        let whole = describe(&doc, 4, 200, 1_000_000);
        let small = describe(&doc, 4, 200, 3_000);
        assert!(whole.to_string().len() > 3_000);
        assert!(whole.get("depth").is_none());
        assert!(
            small.to_string().len() <= 3_200,
            "{}",
            small.to_string().len()
        );
        assert!(
            small["depth"].as_str().unwrap().contains("did not fit"),
            "{small}"
        );
        assert_eq!(small["keys"]["key7"]["type"], "object");
    }

    #[test]
    fn a_tally_counts_every_value_and_names_the_biggest_containers() {
        let doc = json!({"a": [1, 2, 3, {"b": [null, true]}], "c": {"d": "x", "e": 1.5, "f": {}}});
        let tally = tally(&doc, &Vec::new(), &|| false);
        assert!(tally.complete);
        // The root, a, 1, 2, 3, {b}, b, null, true, c, d, e, f.
        assert_eq!(tally.values(), 13);
        assert_eq!(tally.depth, 4);
        assert_eq!(
            tally.longest_arrays,
            vec![("/a".to_owned(), 4), ("/a/3/b".to_owned(), 2)]
        );
        assert_eq!(tally.widest_objects[0], ("/c".to_owned(), 3));
        assert_eq!(tally.counts()["numbers"], 4);
        assert_eq!(tally.counts()["objects"], 4);
        // A part, by its path: its pointers are the document's.
        let path = vec![PathSegment::Key("a".to_owned())];
        let part = super::tally(&doc["a"], &path, &|| false);
        assert_eq!(part.longest_arrays[0], ("/a".to_owned(), 4));
        // Stopped: what was counted until then.
        let big = Value::Array((0..100_000).map(|i| json!([i])).collect());
        let stopped = super::tally(&big, &Vec::new(), &|| true);
        assert!(!stopped.complete);
        assert!(stopped.values() < 10_000, "{}", stopped.values());
    }

    #[test]
    fn sample_indices_cover_the_start_and_spread_over_the_rest() {
        assert_eq!(sample_indices(3, 10), vec![0, 1, 2]);
        let picked = sample_indices(1_000, 10);
        assert_eq!(picked.len(), 10);
        assert_eq!(&picked[..5], &[0, 1, 2, 3, 4]);
        assert!(*picked.last().unwrap() > 800);
    }

    #[test]
    fn a_document_kept_as_its_file_is_read_and_described_the_same_way() {
        let dir = std::env::temp_dir().join(format!("jsonquery-mcp-shape-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("big.json");
        let doc = Value::Array(
            (0..2_000)
                .map(|i| json!({"id": i, "name": format!("n{i}")}))
                .collect(),
        );
        std::fs::write(&file, doc.to_string()).unwrap();
        let limits = LoadLimits {
            lazy_threshold: 1,
            ..LoadLimits::default()
        };
        let loaded = load_with(&file, limits).unwrap();
        assert!(loaded.is_lazy());
        let root: Root<'_> = loaded.root();
        let (item, path) = resolve_pointer(root, "/1500/name").unwrap();
        assert_eq!(item.scalar_value(), Some(json!("n1500")));
        assert_eq!(pointer_of(&path), "/1500/name");
        let page = read(root, &Vec::new(), 1990, 20, 10_000);
        assert_eq!(page["items"].as_array().unwrap().len(), 10);
        assert_eq!(page["items"][9]["id"], 1999);
        let shape = describe(root, 3, 50, 100_000);
        assert_eq!(shape["length"], 2000);
        assert_eq!(shape["items"]["keys"]["id"]["integers"], true);
        drop(loaded);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
