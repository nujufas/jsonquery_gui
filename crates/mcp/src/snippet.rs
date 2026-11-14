//! The snippet: what an answer gives of a value too big for a page, and the notice that
//! says so, the same in every tool's answer.
//!
//! A model that wants a document asks for all of it as often as not, and a big one
//! would not fit in its context. So no answer holds more than a page: a value bigger
//! than that is given as a snippet of it ([`snip`]): an array its first items, an object
//! its small members whole and a share of what room is left for each big one, a long
//! string its start. Each part left out is written in its place as
//! `{"$omitted": what, "pointer": where}`, and the items (or members) after the last
//! one given as `{"$more": how many, "pointer": where, "offset": from}`.
//!
//! And the answer says so, in its member `snippet` ([`notice`]): that the value is
//! large, and how large; the arrays (and objects) in it that are not given whole, with
//! how many items each has and how many of them are given; and `next`, a call that
//! reads on from where the snippet stops.

use std::cmp::Reverse;

use jsonquery_core::{PathSegment, ValueKind, ValueView};
use serde_json::{json, Map, Value};

use crate::shape::{json_len, pointer_of, summary, whole};

/// The most a [`notice`] takes, in bytes of JSON.
pub const NOTICE_BYTES: usize = 1_400;

/// How much of a long string is given where it is left out, in characters.
const STARTS_CHARS: usize = 200;

/// The least room a part of a snippet is cut down to: given less, it is only said what
/// it is.
const MIN_PART: usize = 100;

/// The room kept at the end of an array or object for the `$more` that may close it.
const MORE_BYTES: usize = 80;

/// How many members of one object a snippet looks at: the rest are in its `$more`.
const MAX_MEMBERS: usize = 200;

/// How many of the arrays, and of the objects, not given whole a notice lists.
const LISTED_ARRAYS: usize = 4;
const LISTED_OBJECTS: usize = 2;

/// What is left of `max_bytes` for a snippet when its notice must fit in them too: at
/// most a third goes to the notice.
pub fn snippet_bytes(max_bytes: usize) -> usize {
    max_bytes - NOTICE_BYTES.min(max_bytes / 3)
}

/// What a snippet leaves out of the value it is made of, noted as it is made: the
/// arrays and objects it does not give whole, and the strings it gives only the start
/// of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cuts {
    /// The outermost first, then the ones inside it, in the order they are in.
    parts: Vec<Part>,
    strings: usize,
    /// Something was left out to keep to the bytes, not only to the `limit` asked.
    bytes: bool,
}

/// An array or object a snippet does not give whole.
#[derive(Clone, Debug, PartialEq)]
struct Part {
    pointer: String,
    array: bool,
    /// How many items (or members) it has.
    count: usize,
    /// How many of them are given.
    given: usize,
    /// The offset reading on starts from.
    next: usize,
}

impl Cuts {
    /// Nothing was left out.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty() && self.strings == 0
    }

    /// Something was left out to keep to the bytes (not only to the `limit` asked).
    pub fn for_bytes(&self) -> bool {
        self.bytes
    }

    /// The bytes ran out: what was not given is left for the next page.
    pub(crate) fn ran_out(&mut self) {
        self.bytes = true;
    }

    /// The array (or object) at `pointer`, of `count` items, is given a page at a time:
    /// this page gives `given` of them, and the next starts at `next`. It is the
    /// outermost of what is left out: reading on starts there.
    pub fn paged(&mut self, pointer: &str, array: bool, count: usize, given: usize, next: usize) {
        self.parts.insert(
            0,
            Part {
                pointer: pointer.to_owned(),
                array,
                count,
                given,
                next,
            },
        );
    }

    /// The call of read_value, on `input`, that reads on from where the snippet stops:
    /// the rest of its outermost part that is not given whole.
    pub fn read_on(&self, input: &Value) -> Option<Value> {
        let part = self.parts.first()?;
        Some(json!({
            "tool": "read_value",
            "arguments": {"input": input, "pointer": part.pointer, "offset": part.next}
        }))
    }

    fn append(&mut self, other: Cuts) {
        self.parts.extend(other.parts);
        self.strings += other.strings;
        self.bytes |= other.bytes;
    }
}

/// `value` whole if it takes no more than `max_bytes` of JSON; if not, a snippet of it
/// that leaves room in them for its notice, and what it leaves out.
pub fn snippet<V: ValueView>(value: &V, max_bytes: usize) -> (Value, Option<Cuts>) {
    let mut room = max_bytes;
    if let Some(all) = whole(value, &mut room) {
        return (all, None);
    }
    let (part, cuts) = cut_to(value, snippet_bytes(max_bytes));
    (part, Some(cuts))
}

/// A snippet of `value` in about `budget` bytes of JSON (all of it, if it fits), and
/// what it leaves out.
pub fn cut_to<V: ValueView>(value: &V, budget: usize) -> (Value, Cuts) {
    let mut cuts = Cuts::default();
    let part = snip(value, "", budget, &mut cuts);
    (part, cuts)
}

/// `value`, at `pointer`, cut down to about `budget` bytes of JSON: whole if it fits; an
/// array its first items, the first of them cut down too if it alone is too big; an
/// object its small members whole, and a share of what room is left for each big one,
/// cut down to it; a long string its start. What it leaves out is noted in `cuts`.
pub(crate) fn snip<V: ValueView>(
    value: &V,
    pointer: &str,
    budget: usize,
    cuts: &mut Cuts,
) -> Value {
    let mut room = budget;
    if let Some(all) = whole(value, &mut room) {
        return all;
    }
    cuts.bytes = true;
    match value.kind() {
        ValueKind::Array => snip_array(value, pointer, budget, cuts),
        ValueKind::Object => snip_object(value, pointer, budget, cuts),
        _ => omitted(value, pointer, cuts),
    }
}

fn snip_array<V: ValueView>(value: &V, pointer: &str, budget: usize, cuts: &mut Cuts) -> Value {
    let at = cuts.parts.len();
    let count = value.child_count();
    // The brackets, and the `$more` that closes it.
    let mut left = budget.saturating_sub(2 + MORE_BYTES + pointer.len());
    let mut items = Vec::new();
    for (i, (_, child)) in value.iter_children().enumerate() {
        let mut room = left.saturating_sub(usize::from(i > 0));
        if let Some(all) = whole(&child, &mut room) {
            left = room;
            items.push(all);
            continue;
        }
        // The first item, too big whole, is given cut down; the snippet stops after it.
        if i == 0 {
            let child_pointer = format!("{pointer}/0");
            items.push(if left >= MIN_PART {
                snip(&child, &child_pointer, left, cuts)
            } else {
                omitted(&child, &child_pointer, cuts)
            });
        }
        break;
    }
    let given = items.len();
    if given < count {
        items.push(more(count - given, "items", pointer, given));
        cuts.parts.insert(
            at,
            Part {
                pointer: pointer.to_owned(),
                array: true,
                count,
                given,
                next: given,
            },
        );
    }
    Value::Array(items)
}

fn snip_object<V: ValueView>(value: &V, pointer: &str, budget: usize, cuts: &mut Cuts) -> Value {
    let at = cuts.parts.len();
    let count = value.child_count();
    let members: Vec<(String, V)> = value
        .iter_children()
        .take(MAX_MEMBERS)
        .filter_map(|(segment, child)| match segment {
            Some(PathSegment::Key(key)) => Some((key, child)),
            _ => None,
        })
        .collect();
    // The braces, and the `$more` that closes it.
    let left = budget.saturating_sub(2 + MORE_BYTES + pointer.len());
    // What each member takes whole, its key too, if no more than all the room.
    let costs: Vec<Option<usize>> = members
        .iter()
        .map(|(key, child)| {
            let mut room = left;
            whole(child, &mut room).map(|_| left - room + json_len(key) + 2)
        })
        .collect();
    // The room each is given: the smallest first, whole while it fits in its share of
    // what is left; the big ones share the rest.
    let mut order: Vec<usize> = (0..members.len()).collect();
    order.sort_by_key(|&i| costs[i].unwrap_or(usize::MAX));
    let mut shares: Vec<Option<usize>> = vec![None; members.len()];
    let mut to_share = left;
    for (n, &i) in order.iter().enumerate() {
        let fair = to_share / (members.len() - n);
        match costs[i] {
            Some(cost) if cost <= fair => to_share -= cost,
            _ => {
                shares[i] = Some(fair);
                to_share -= fair;
            }
        }
    }
    // Made in their order, each in the room it was given, while there is room.
    let mut out = Map::new();
    let mut used = 0;
    for ((key, child), share) in members.into_iter().zip(shares) {
        let key_cost = json_len(&key) + 1 + usize::from(!out.is_empty());
        let child_pointer = format!(
            "{pointer}{}",
            pointer_of(std::slice::from_ref(&PathSegment::Key(key.clone())))
        );
        let room = left.saturating_sub(used + key_cost);
        let mut part_cuts = Cuts::default();
        let part = match share {
            None => snip(&child, &child_pointer, room, &mut part_cuts),
            Some(share) if share.min(room) >= MIN_PART + key_cost => snip(
                &child,
                &child_pointer,
                share.min(room) - key_cost,
                &mut part_cuts,
            ),
            Some(_) => omitted(&child, &child_pointer, &mut part_cuts),
        };
        let cost = key_cost + part.to_string().len();
        if used + cost > left && !out.is_empty() {
            break;
        }
        used += cost;
        cuts.append(part_cuts);
        out.insert(key, part);
    }
    let given = out.len();
    if given < count {
        out.insert(
            "$more".to_owned(),
            more(count - given, "members", pointer, given),
        );
        cuts.parts.insert(
            at,
            Part {
                pointer: pointer.to_owned(),
                array: false,
                count,
                given,
                next: given,
            },
        );
    }
    Value::Object(out)
}

/// What stands for a part left out: what it is, and where; a long string, its start.
pub(crate) fn omitted<V: ValueView>(value: &V, pointer: &str, cuts: &mut Cuts) -> Value {
    cuts.bytes = true;
    let mut out = Map::new();
    match value.kind() {
        ValueKind::String => {
            cuts.strings += 1;
            let preview = value.scalar_preview().unwrap_or_default();
            let start: String = preview.chars().take(STARTS_CHARS).collect();
            out.insert("$omitted".to_owned(), json!("long string"));
            out.insert("starts".to_owned(), Value::String(start));
        }
        kind => {
            out.insert("$omitted".to_owned(), json!(summary(value)));
            if kind.is_container() {
                cuts.parts.push(Part {
                    pointer: pointer.to_owned(),
                    array: kind == ValueKind::Array,
                    count: value.child_count(),
                    given: 0,
                    next: 0,
                });
            }
        }
    }
    out.insert("pointer".to_owned(), json!(pointer));
    Value::Object(out)
}

/// What stands for the items (or members) after the last one given.
fn more(n: usize, what: &str, pointer: &str, offset: usize) -> Value {
    json!({"$more": format!("{n} more {what}"), "pointer": pointer, "offset": offset})
}

/// What a snippet is of, for its [`notice`].
#[derive(Clone, Debug)]
pub struct Of<'a> {
    /// The member of the answer that holds the snippet: "items", "result", "text"…
    pub member: &'a str,
    /// What it is called, short: "people.json", "the results of the query".
    pub name: &'a str,
    /// Where in that the value is (its JSON Pointer): "" for all of it.
    pub pointer: &'a str,
    /// What it is: "array with 120000 items" ([`summary`]).
    pub is: String,
    /// How many bytes of JSON all of it takes, if that is known.
    pub bytes: Option<u64>,
    /// `bytes` is only what is known of it: it takes more (results still coming, or
    /// more than the app keeps).
    pub at_least: bool,
}

/// The `snippet` member of an answer that gives a snippet of a value too big for a page
/// (`page`, the bytes the user set): that the value is large, and how large; the arrays
/// (and objects) in it that are not given whole, with how many items each has and how
/// many are given; and `next`, the call that reads on (none: there is no such call, and
/// the note says what to do instead). `hint`: what more the tool has to say.
pub fn notice(
    of: &Of<'_>,
    cuts: &Cuts,
    next: Option<Value>,
    page: usize,
    hint: Option<&str>,
) -> Value {
    let mut arrays: Vec<&Part> = cuts.parts.iter().filter(|p| p.array).collect();
    let mut objects: Vec<&Part> = cuts.parts.iter().filter(|p| !p.array).collect();
    arrays.sort_by_key(|p| Reverse(p.count));
    objects.sort_by_key(|p| Reverse(p.count));
    let what = if of.pointer.is_empty() {
        of.name.to_owned()
    } else {
        format!("{:?} in {}", of.pointer, of.name)
    };
    let mut note = large_note(&what, of.bytes, of.at_least, page);
    note.push(' ');
    if let Some(biggest) = arrays.first() {
        note.push_str(&format!(
            "It holds arrays: the biggest, at {}, has {} items, {} of them given here.",
            shown_pointer(&biggest.pointer),
            biggest.count,
            biggest.given
        ));
    } else if let Some(biggest) = objects.first() {
        note.push_str(&format!(
            "It holds objects too big to give whole: the biggest, at {}, has {} members, {} \
             of them given here.",
            shown_pointer(&biggest.pointer),
            biggest.count,
            biggest.given
        ));
    } else if cuts.strings > 0 {
        note.push_str(&format!(
            "It holds strings too long to give whole: {} given by their start.",
            cuts.strings
        ));
    }
    note.push_str(&format!(
        " `{}` holds a snippet of it, not all of it. Do not ask for all of it: it would not \
         fit. ",
        of.member
    ));
    if next.is_some() {
        note.push_str("`next` reads on from where the snippet stops; ");
    }
    note.push_str(
        "read_value reads any part by its pointer, a page at a time; run_query picks, filters \
         or counts what you need; describe gives its shape.",
    );
    if let Some(hint) = hint {
        note.push(' ');
        note.push_str(hint);
    }
    let listed = |parts: &[&Part], items: &str| -> Value {
        parts
            .iter()
            .map(|p| {
                json!({"pointer": p.pointer, items: p.count, "given": p.given, "next_offset": p.next})
            })
            .collect()
    };
    let mut out = Map::new();
    out.insert("note".to_owned(), json!(note));
    out.insert("in".to_owned(), json!(of.member));
    out.insert("of".to_owned(), json!(of.name));
    if !of.pointer.is_empty() {
        out.insert("pointer".to_owned(), json!(of.pointer));
    }
    out.insert("is".to_owned(), json!(of.is));
    if let Some(bytes) = of.bytes {
        out.insert("bytes".to_owned(), json!(bytes));
        out.insert("size".to_owned(), json!(size_text(bytes)));
        out.insert("tokens_about".to_owned(), json!(bytes / 4));
        if of.at_least {
            out.insert("at_least".to_owned(), json!(true));
        }
    }
    if !arrays.is_empty() {
        out.insert(
            "arrays".to_owned(),
            listed(&arrays[..arrays.len().min(LISTED_ARRAYS)], "items"),
        );
        if arrays.len() > LISTED_ARRAYS {
            out.insert(
                "more_arrays".to_owned(),
                json!(arrays.len() - LISTED_ARRAYS),
            );
        }
    }
    if !objects.is_empty() {
        out.insert(
            "objects".to_owned(),
            listed(&objects[..objects.len().min(LISTED_OBJECTS)], "members"),
        );
    }
    if cuts.strings > 0 {
        out.insert("strings_cut".to_owned(), json!(cuts.strings));
    }
    if let Some(next) = next {
        out.insert("next".to_owned(), next);
    }
    // The lists are cut short, the longest first, while it is more than it may be.
    let mut answer = Value::Object(out);
    for list in ["objects", "arrays"] {
        while answer.to_string().len() > NOTICE_BYTES {
            match answer.get_mut(list).and_then(Value::as_array_mut) {
                Some(items) if items.len() > 1 => {
                    items.pop();
                }
                _ => break,
            }
        }
    }
    answer
}

/// That a value is large, and how large: "Large: people.json takes 123.5 MB as JSON,
/// about 30.9M tokens: 5145 pages of 24000 bytes." The same words wherever it is said.
pub fn large_note(name: &str, bytes: Option<u64>, at_least: bool, page: usize) -> String {
    match bytes {
        Some(bytes) => format!(
            "Large: {name} takes {}{} as JSON, about {} tokens: {} pages of {page} bytes.",
            if at_least { "more than " } else { "" },
            size_text(bytes),
            about(bytes / 4),
            bytes.div_ceil(page.max(1) as u64)
        ),
        None => format!("Large: {name} takes more than a page ({page} bytes) holds."),
    }
}

/// What an answer that says what a document is says of one too big for a page (none
/// for one that is not): that it is large, how large, and how to read it.
pub fn large_hint(name: &str, bytes: u64, page: usize) -> Option<String> {
    (bytes > page as u64).then(|| {
        format!(
            "{} Do not ask for all of it: read the part you need (read_value, with a pointer) \
             or query it (run_query). What is too big for a page is given as a snippet, said in \
             the answer's `snippet`.",
            large_note(name, Some(bytes), false, page)
        )
    })
}

/// `bytes` as a person reads it: "1.2 GB", "340.0 KB", "12 bytes".
pub fn size_text(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "GB"), (1_000_000, "MB"), (1_000, "KB")];
    for (unit, name) in UNITS {
        if bytes >= unit {
            return format!("{:.1} {name}", bytes as f64 / unit as f64);
        }
    }
    format!("{bytes} bytes")
}

/// A count, roughly: "30.9M", "120K", "900".
fn about(n: u64) -> String {
    match n {
        n if n >= 1_000_000_000 => format!("{:.1}G", n as f64 / 1e9),
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{}K", n / 1_000),
        n => n.to_string(),
    }
}

/// A pointer as the note says it: the top is `""` (the top).
fn shown_pointer(pointer: &str) -> String {
    if pointer.is_empty() {
        "\"\" (the top)".to_owned()
    } else {
        format!("{pointer:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn users(n: usize) -> Value {
        Value::Array(
            (0..n)
                .map(|i| json!({"id": i, "name": format!("user {i}"), "tags": ["a", "b"]}))
                .collect(),
        )
    }

    fn size(value: &Value) -> usize {
        value.to_string().len()
    }

    #[test]
    fn what_fits_is_given_whole_with_no_cuts() {
        let value = json!({"a": [1, 2, 3]});
        assert_eq!(snippet(&&value, 1_000), (value.clone(), None));
    }

    #[test]
    fn a_long_array_gives_its_first_items_and_says_how_many_more() {
        let value = users(10_000);
        let (part, cuts) = snippet(&&value, 4_000);
        let cuts = cuts.expect("a snippet");
        assert!(size(&part) <= snippet_bytes(4_000), "{}", size(&part));
        let items = part.as_array().unwrap();
        let given = items.len() - 1;
        assert!(given > 10, "{given}");
        assert_eq!(items[0], value[0]);
        assert_eq!(items[given - 1], value[given - 1]);
        assert_eq!(
            items[given],
            json!({"$more": format!("{} more items", 10_000 - given), "pointer": "", "offset": given})
        );
        assert!(cuts.for_bytes());
        assert_eq!(
            cuts.parts,
            vec![Part {
                pointer: String::new(),
                array: true,
                count: 10_000,
                given,
                next: given
            }]
        );
        assert_eq!(
            cuts.read_on(&json!("document")),
            Some(
                json!({"tool": "read_value", "arguments": {"input": "document", "pointer": "", "offset": given}})
            )
        );
    }

    #[test]
    fn an_object_gives_its_small_members_whole_and_shares_the_room_among_its_big_ones() {
        let value = json!({
            "users": users(5_000),
            "meta": {"count": 5_000, "source": "test"},
            "orders": users(3_000),
            "version": 2
        });
        let (part, cuts) = snippet(&&value, 6_000);
        let cuts = cuts.expect("a snippet");
        assert!(size(&part) <= snippet_bytes(6_000), "{}", size(&part));
        // The small ones whole, in their order.
        assert_eq!(part["meta"], value["meta"]);
        assert_eq!(part["version"], 2);
        let keys: Vec<&String> = part.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["users", "meta", "orders", "version"]);
        // Each big one its first items, in about the same room.
        let users_given = part["users"].as_array().unwrap().len() - 1;
        let orders_given = part["orders"].as_array().unwrap().len() - 1;
        assert!(
            users_given > 3 && orders_given > 3,
            "{users_given} {orders_given}"
        );
        assert!(
            users_given.abs_diff(orders_given) <= 2,
            "{users_given} {orders_given}"
        );
        assert_eq!(part["users"][users_given]["pointer"], "/users");
        assert_eq!(part["orders"][orders_given]["offset"], orders_given);
        let pointers: Vec<&str> = cuts.parts.iter().map(|p| p.pointer.as_str()).collect();
        assert_eq!(pointers, ["/users", "/orders"]);
    }

    #[test]
    fn arrays_deep_inside_still_give_their_first_items() {
        let value = json!({"data": {"page": {"rows": users(4_000)}}});
        let (part, cuts) = snippet(&&value, 3_000);
        let rows = part["data"]["page"]["rows"].as_array().unwrap();
        assert!(rows.len() > 3, "{part}");
        assert_eq!(rows[0], value["data"]["page"]["rows"][0]);
        assert_eq!(cuts.unwrap().parts[0].pointer, "/data/page/rows");
    }

    #[test]
    fn a_first_item_too_big_is_cut_down_and_the_array_stops_after_it() {
        let value = json!([{"id": 0, "rows": users(2_000)}, {"id": 1}]);
        let (part, cuts) = snippet(&&value, 3_000);
        let cuts = cuts.unwrap();
        assert_eq!(part[0]["id"], 0);
        assert!(part[0]["rows"].as_array().unwrap().len() > 3);
        assert_eq!(part[1]["$more"], "1 more items");
        assert_eq!(part[1]["offset"], 1);
        // The outermost first: reading on starts at item 1.
        let pointers: Vec<&str> = cuts.parts.iter().map(|p| p.pointer.as_str()).collect();
        assert_eq!(pointers, ["", "/0/rows"]);
    }

    #[test]
    fn what_has_no_room_is_said_what_it_is_and_long_strings_give_their_start() {
        let value = json!({"text": "x".repeat(10_000), "n": 1});
        let (part, cuts) = snippet(&&value, 1_500);
        let cuts = cuts.unwrap();
        assert_eq!(part["n"], 1);
        assert_eq!(part["text"]["$omitted"], "long string");
        assert_eq!(part["text"]["pointer"], "/text");
        assert!(part["text"]["starts"].as_str().unwrap().len() <= 202);
        assert_eq!(cuts.strings, 1);
        // An object of many big members: those past the room are in its `$more`.
        let wide = Value::Object((0..300).map(|i| (format!("k{i}"), users(50))).collect());
        let (part, cuts) = snippet(&&wide, 2_000);
        let cuts = cuts.unwrap();
        assert!(size(&part) <= 2_000, "{}", size(&part));
        let more = &part["$more"];
        let given = part.as_object().unwrap().len() - 1;
        assert_eq!(more["offset"], given);
        assert_eq!(more["$more"], format!("{} more members", 300 - given));
        assert_eq!(cuts.parts[0].pointer, "");
        assert!(!cuts.parts[0].array);
    }

    #[test]
    fn the_notice_says_large_arrays_snippet_and_the_call_that_reads_on() {
        let value = json!({"users": users(120_000), "meta": {"v": 1}});
        let (_, cuts) = snippet(&&value, 4_000);
        let cuts = cuts.unwrap();
        let of = Of {
            member: "items",
            name: "people.json",
            pointer: "",
            is: summary(&&value),
            bytes: Some(123_456_789),
            at_least: false,
        };
        let next = cuts.read_on(&json!("document"));
        let notice = notice(
            &of,
            &cuts,
            next.clone(),
            24_000,
            Some("save_file writes it."),
        );
        let note = notice["note"].as_str().unwrap();
        assert!(note.starts_with("Large: people.json takes 123.5 MB as JSON, about 30.9M tokens: 5145 pages of 24000 bytes."), "{note}");
        assert!(
            note.contains("It holds arrays: the biggest, at \"/users\", has 120000 items"),
            "{note}"
        );
        assert!(
            note.contains("`items` holds a snippet of it, not all of it. Do not ask for all of it"),
            "{note}"
        );
        assert!(note.contains("`next` reads on"), "{note}");
        assert!(note.ends_with("save_file writes it."), "{note}");
        assert_eq!(notice["in"], "items");
        assert_eq!(notice["of"], "people.json");
        assert!(notice.get("pointer").is_none());
        assert_eq!(notice["bytes"], 123_456_789);
        assert_eq!(notice["size"], "123.5 MB");
        assert_eq!(notice["tokens_about"], 30_864_197);
        assert_eq!(notice["arrays"][0]["pointer"], "/users");
        assert_eq!(notice["arrays"][0]["items"], 120_000);
        assert_eq!(
            notice["arrays"][0]["next_offset"],
            notice["arrays"][0]["given"]
        );
        assert_eq!(notice["next"], next.unwrap());
        assert!(notice.to_string().len() <= NOTICE_BYTES);
        // Without a size, and without a call that reads on.
        let of = Of {
            bytes: None,
            pointer: "/users",
            ..of
        };
        let notice = super::notice(&of, &cuts, None, 4_000, None);
        let note = notice["note"].as_str().unwrap();
        assert!(
            note.starts_with(
                "Large: \"/users\" in people.json takes more than a page (4000 bytes) holds."
            ),
            "{note}"
        );
        assert_eq!(notice["pointer"], "/users");
        assert!(!note.contains("`next`"), "{note}");
        assert!(notice.get("next").is_none() && notice.get("bytes").is_none());
    }

    #[test]
    fn the_notice_keeps_to_its_bytes_however_many_parts_are_cut() {
        let value = Value::Object(
            (0..150)
                .map(|i| (format!("a rather long key number {i}"), users(40)))
                .collect(),
        );
        let (_, cuts) = snippet(&&value, 200_000 / 40);
        let cuts = cuts.unwrap();
        let of = Of {
            member: "result",
            name: "the patched document",
            pointer: "",
            is: summary(&&value),
            bytes: Some(9_999_999),
            at_least: true,
        };
        let notice = notice(&of, &cuts, cuts.read_on(&json!("last")), 4_000, None);
        assert!(
            notice.to_string().len() <= NOTICE_BYTES,
            "{}",
            notice.to_string().len()
        );
        assert!(notice["note"]
            .as_str()
            .unwrap()
            .contains("takes more than 10.0 MB"));
        assert_eq!(notice["at_least"], true);
        assert!(notice["more_arrays"].as_u64().unwrap() > 0);
    }

    #[test]
    fn a_document_too_big_for_a_page_is_said_to_be_large() {
        assert_eq!(large_hint("a.json", 20_000, 24_000), None);
        let hint = large_hint("big.json", 2_400_000_000, 24_000).unwrap();
        assert!(
            hint.starts_with(
                "Large: big.json takes 2.4 GB as JSON, about 600.0M tokens: 100000 pages"
            ),
            "{hint}"
        );
        assert!(hint.contains("Do not ask for all of it"), "{hint}");
        assert_eq!(about(999), "999");
        assert_eq!(about(120_400), "120K");
        assert_eq!(size_text(1_500), "1.5 KB");
    }
}
