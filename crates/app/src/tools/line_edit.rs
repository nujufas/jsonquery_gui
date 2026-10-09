//! Editing a line of the side-by-side view.
//!
//! A line of the view is a line of a document, pretty-printed (two spaces to a level, one
//! member or element to a line), so where in the document it is can be read from the lines of
//! its column: the openers above it, a level up each, are what holds it, and a line says its
//! own name, or, in an array, how many elements are before it ([`place_of`]). That works the
//! same on both sides, though the right one shows an object's members in the order of the left
//! document: a place is made of names and counts, not of where the line is in the column.
//!
//! What is typed over a line takes the place of the member or the element that was on it
//! ([`parse`]): a value, `"name": value`, nothing (the line goes), or several, apart by commas
//! (they are put in). A line that opens an object or an array has its name changed, and the
//! rest of it kept. What is typed has to be JSON where it is, or is not taken, and the
//! message says what is wrong; and the lines of a document that were cut short in the view
//! (see `MAX_LINE_CHARS`) are not edited, as typing over them would lose what is not shown.
//! The worker makes the change in the document ([`apply`]) as it is — an object's members
//! stay in the order they have there, a number is as it was written — and compares again.

use jsonquery_query::diff::{Side, MAX_LINE_CHARS};
use serde_json::{Map, Value};

/// One step from the root of a document down to a member or an element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Key(String),
    Index(usize),
}

/// What there is to change on a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// All of it: a member or an element that is on one line (a value that is not an
    /// object or an array, or one that is empty).
    Whole,
    /// A line that opens an object or an array that has things in it, with this
    /// character at the end: only the member's name can be changed.
    Opens(char),
}

/// What holds the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Within {
    Object,
    Array,
    /// Nothing: the line is the whole document.
    Root,
}

/// Where a line is in the document it is a line of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// To the member or the element the line is of; none for the whole document.
    pub path: Vec<Step>,
    pub part: Part,
    pub within: Within,
    /// For a member: the names of the other members of its object, which are not to be
    /// given to what is typed.
    pub others: Vec<String>,
}

/// Why a line is not one to edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// It closes an object or an array, or opens one that is not a member's (the document
    /// itself, an element): there is nothing on it that is not the structure of what is
    /// around it.
    Structure,
    /// The view cut it short: what is shown is not all of it.
    Cut,
}

impl Why {
    /// What is said of it, if anything: a line that is structure is plainly not one to edit.
    pub fn said(self) -> Option<&'static str> {
        match self {
            Why::Structure => None,
            Why::Cut => Some(
                "That line is too long to edit here: only its beginning is shown. \
                 The Documents page has all of it",
            ),
        }
    }
}

/// What replaces what was on the line.
#[derive(Clone, Debug, PartialEq)]
pub enum Fragment {
    /// For a member of an object: none, one or several members, in its place.
    Members(Vec<(String, Value)>),
    /// For an element of an array, or the document: none, one or several elements.
    Elements(Vec<Value>),
    /// For the line of an object or an array that has things in it: its name.
    Rename(String),
}

/// A change to one line of a document, as the worker makes it.
#[derive(Clone, Debug)]
pub struct Edit {
    /// The document it is made in.
    pub into: Side,
    pub path: Vec<Step>,
    pub fragment: Fragment,
}

impl Edit {
    /// It takes the line out: nothing was typed over it.
    pub fn deletes(&self) -> bool {
        match &self.fragment {
            Fragment::Members(members) => members.is_empty(),
            Fragment::Elements(items) => items.is_empty(),
            Fragment::Rename(_) => false,
        }
    }
}

/// Whether the view cut `line` short.
pub fn is_cut(line: &str) -> bool {
    line.chars().nth(MAX_LINE_CHARS).is_some()
}

/// The depth of a line (two spaces to a level) and what follows its indentation.
fn split(line: &str) -> Option<(usize, &str)> {
    let text = line.trim_start_matches(' ');
    let spaces = line.len() - text.len();
    spaces.is_multiple_of(2).then_some((spaces / 2, text))
}

/// A line without the comma that follows its value, and without the space at the end.
fn body(text: &str) -> &str {
    let text = text.trim_end();
    text.strip_suffix(',').unwrap_or(text).trim_end()
}

fn closes(body: &str) -> bool {
    body.starts_with(['}', ']'])
}

/// The character a line that opens an object or an array ends with.
fn opener(body: &str) -> Option<char> {
    body.chars().last().filter(|c| matches!(c, '{' | '['))
}

/// The name a member's line begins with: `"name":`.
fn name_of(body: &str) -> Option<String> {
    if !body.starts_with('"') {
        return None;
    }
    let mut names = serde_json::Deserializer::from_str(body).into_iter::<String>();
    let name = names.next()?.ok()?;
    body[names.byte_offset()..]
        .trim_start()
        .starts_with(':')
        .then_some(name)
}

/// Where line `at` of `lines` — the lines of one column of the view, in order — is in its
/// document, or why it is not one to edit.
pub fn place_of(lines: &[&str], at: usize) -> Result<Place, Why> {
    let line = *lines.get(at).ok_or(Why::Structure)?;
    if is_cut(line) {
        return Err(Why::Cut);
    }
    let (depth, text) = split(line).ok_or(Why::Structure)?;
    let own = body(text);
    if closes(own) {
        return Err(Why::Structure);
    }

    // What holds it: the nearest line a level up, from there the nearest one more, up to the
    // opening of the document.
    let mut holders = Vec::with_capacity(depth);
    let mut up = at;
    for level in (0..depth).rev() {
        loop {
            up = up.checked_sub(1).ok_or(Why::Structure)?;
            let (above, _) = split(lines[up]).ok_or(Why::Structure)?;
            if above == level {
                break;
            }
            if above < level {
                return Err(Why::Structure);
            }
        }
        holders.push(up);
    }
    holders.reverse();

    // The step into each of them, and into the line.
    let mut path = Vec::with_capacity(depth);
    let mut within = Within::Root;
    for (level, &line_at) in holders.iter().chain(std::iter::once(&at)).enumerate() {
        if level == 0 {
            continue;
        }
        let holder = lines[holders[level - 1]];
        let (_, holder_text) = split(holder).ok_or(Why::Structure)?;
        within = match opener(body(holder_text)) {
            Some('{') => Within::Object,
            Some(_) => Within::Array,
            None => return Err(Why::Structure),
        };
        let this = lines[line_at];
        if is_cut(this) {
            return Err(Why::Cut);
        }
        let (_, this_text) = split(this).ok_or(Why::Structure)?;
        path.push(match within {
            Within::Object => Step::Key(name_of(body(this_text)).ok_or(Why::Structure)?),
            _ => Step::Index(elements_before(lines, holders[level - 1], line_at, level)),
        });
    }

    let part = match opener(own) {
        None => Part::Whole,
        // The opening of the document, or of an element: no name to change.
        Some(_) if within != Within::Object => return Err(Why::Structure),
        Some(opening) => Part::Opens(opening),
    };
    let others = match (within, holders.last()) {
        (Within::Object, Some(&holder)) => members_but(lines, holder, at, depth),
        _ => Vec::new(),
    };
    Ok(Place {
        path,
        part,
        within,
        others,
    })
}

/// The names of the members of the object that opens on line `holder`, whose members are at
/// `depth`, but the one on line `except`.
fn members_but(lines: &[&str], holder: usize, except: usize, depth: usize) -> Vec<String> {
    let mut names = Vec::new();
    for (at, line) in lines.iter().enumerate().skip(holder + 1) {
        let Some((level, text)) = split(line) else {
            continue;
        };
        if level < depth {
            break;
        }
        if level == depth && at != except && !is_cut(line) {
            names.extend(name_of(body(text)));
        }
    }
    names
}

/// How many elements of the array opened on line `holder` come before line `at`, which is
/// at depth `level`: the lines at that depth between them that do not close one.
fn elements_before(lines: &[&str], holder: usize, at: usize, level: usize) -> usize {
    lines[holder + 1..at]
        .iter()
        .filter(|line| {
            split(line).is_some_and(|(depth, text)| depth == level && !closes(body(text)))
        })
        .count()
}

/// What typing `line` over the line at `place` comes to, or what is wrong with it.
pub fn parse(place: &Place, line: &str) -> Result<Fragment, String> {
    let (_, text) = split(line).unwrap_or((0, line.trim_start()));
    let typed = body(text);
    let indent = line.chars().count() - line.trim_start_matches(' ').chars().count();
    match place.part {
        Part::Whole => match place.within {
            Within::Object => {
                let wrapped = format!("{{{typed}}}");
                let members: Map<String, Value> =
                    serde_json::from_str(&wrapped).map_err(|e| said(&e, typed, 1, indent))?;
                if let Some(name) = members.keys().find(|name| place.others.contains(name)) {
                    return Err(taken(name));
                }
                Ok(Fragment::Members(members.into_iter().collect()))
            }
            Within::Array | Within::Root => {
                let wrapped = format!("[{typed}]");
                let items: Vec<Value> =
                    serde_json::from_str(&wrapped).map_err(|e| said(&e, typed, 1, indent))?;
                if place.within == Within::Root && items.len() != 1 {
                    return Err("The document has to be one value".to_owned());
                }
                Ok(Fragment::Elements(items))
            }
        },
        Part::Opens(opening) => {
            let kind = if opening == '{' {
                "an object"
            } else {
                "an array"
            };
            let keep = format!("This line opens {kind}: only its name can be changed, and it has to end with {opening}");
            let name = typed
                .strip_suffix(opening)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| keep.clone())?;
            let wrapped = format!("{{{name} null}}");
            let members: Map<String, Value> = serde_json::from_str(&wrapped)
                .map_err(|e| said(&e, name, 1, indent))
                .and_then(|members: Map<String, Value>| {
                    if members.len() == 1 {
                        Ok(members)
                    } else {
                        Err(keep.clone())
                    }
                })?;
            let (name, _) = members.into_iter().next().ok_or(keep)?;
            if place.others.contains(&name) {
                return Err(taken(&name));
            }
            Ok(Fragment::Rename(name))
        }
    }
}

/// What is wrong with JSON that was typed, as it is said to the person who typed it: the
/// reason, and the character of the line it is at. `typed` was wrapped with `lead`
/// characters before it, and has `indent` spaces before it on the line.
fn said(error: &serde_json::Error, typed: &str, lead: usize, indent: usize) -> String {
    let text = error.to_string();
    let reason = text
        .rsplit_once(" at line ")
        .map_or(text.as_str(), |(reason, _)| reason);
    // `column` counts bytes from 1; say it in characters of the line.
    let before = error.column().saturating_sub(lead + 1);
    let chars = typed
        .char_indices()
        .take_while(|(at, _)| *at < before)
        .count();
    let at = indent + chars + 1;
    if error.column() <= lead {
        format!("Not valid JSON: {reason}")
    } else {
        format!("Not valid JSON: {reason} (at character {at})")
    }
}

/// `edit` made in `root`: the document with the line changed, or why it can't be.
pub fn apply(mut root: Value, edit: &Edit) -> Result<Value, String> {
    let stale = || "the document is not as it was when it was compared".to_owned();
    let Some((last, parents)) = edit.path.split_last() else {
        return match &edit.fragment {
            Fragment::Elements(items) if items.len() == 1 => Ok(items[0].clone()),
            _ => Err("The document has to be one value".to_owned()),
        };
    };
    let mut node = &mut root;
    for step in parents {
        node = match (step, node) {
            (Step::Key(key), Value::Object(members)) => members.get_mut(key).ok_or_else(stale)?,
            (Step::Index(at), Value::Array(items)) => items.get_mut(*at).ok_or_else(stale)?,
            _ => return Err(stale()),
        };
    }
    match (last, &edit.fragment, node) {
        (Step::Key(key), Fragment::Members(put), Value::Object(members)) => {
            replace(members, key, put)?
        }
        (Step::Key(key), Fragment::Rename(name), Value::Object(members)) => {
            rename(members, key, name)?
        }
        (Step::Index(at), Fragment::Elements(put), Value::Array(items)) => {
            if *at >= items.len() {
                return Err(stale());
            }
            items.splice(*at..*at + 1, put.iter().cloned());
        }
        _ => return Err(stale()),
    }
    Ok(root)
}

fn taken(name: &str) -> String {
    format!(
        "There already is a member called {} here",
        Value::String(name.to_owned())
    )
}

/// Member `key` of `members` replaced by `put`, in its place.
fn replace(
    members: &mut Map<String, Value>,
    key: &str,
    put: &[(String, Value)],
) -> Result<(), String> {
    if !members.contains_key(key) {
        return Err("the document is not as it was when it was compared".to_owned());
    }
    if let Some((name, _)) = put
        .iter()
        .find(|(name, _)| name != key && members.contains_key(name))
    {
        return Err(taken(name));
    }
    for (name, value) in std::mem::take(members) {
        if name == key {
            members.extend(put.iter().cloned());
        } else {
            members.insert(name, value);
        }
    }
    Ok(())
}

/// Member `key` of `members` called `name`, where it is.
fn rename(members: &mut Map<String, Value>, key: &str, name: &str) -> Result<(), String> {
    if !members.contains_key(key) {
        return Err("the document is not as it was when it was compared".to_owned());
    }
    if name != key && members.contains_key(name) {
        return Err(taken(name));
    }
    for (old, value) in std::mem::take(members) {
        members.insert(if old == key { name.to_owned() } else { old }, value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonquery_query::diff::compare;
    use jsonquery_query::reformat::{self, Options};
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    fn render(value: &Value) -> String {
        reformat::render(value, &Options::default())
    }

    fn lines(value: &Value) -> Vec<String> {
        render(value).lines().map(str::to_owned).collect()
    }

    fn borrowed(lines: &[String]) -> Vec<&str> {
        lines.iter().map(String::as_str).collect()
    }

    fn key(name: &str) -> Step {
        Step::Key(name.to_owned())
    }

    fn sample() -> Value {
        json!({
            "name": "Ada",
            "tags": ["a", "b"],
            "address": {"city": "London", "geo": {"lat": 51.5}},
            "empty": [],
            "list": [{"x": 1}, 2, [3]]
        })
    }

    // Where a line is.

    fn put(path: Vec<Step>, part: Part, within: Within, others: &[&str]) -> Place {
        Place {
            path,
            part,
            within,
            others: others.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    #[test]
    fn a_member_is_found_by_its_name_and_an_element_by_how_many_are_before_it() {
        let value = sample();
        let text = lines(&value);
        let lines = borrowed(&text);
        let place = |needle: &str| {
            let at = lines.iter().position(|l| l.trim() == needle).unwrap();
            place_of(&lines, at)
        };
        use Part::{Opens, Whole};
        use Within::{Array, Object};
        assert_eq!(
            place("\"name\": \"Ada\","),
            Ok(put(
                vec![key("name")],
                Whole,
                Object,
                &["tags", "address", "empty", "list"]
            ))
        );
        assert_eq!(
            place("\"b\""),
            Ok(put(vec![key("tags"), Step::Index(1)], Whole, Array, &[]))
        );
        assert_eq!(
            place("\"lat\": 51.5"),
            Ok(put(
                vec![key("address"), key("geo"), key("lat")],
                Whole,
                Object,
                &[]
            ))
        );
        // The others are those of its own object: "city" has "geo" beside it.
        assert_eq!(
            place("\"city\": \"London\","),
            Ok(put(
                vec![key("address"), key("city")],
                Whole,
                Object,
                &["geo"]
            ))
        );
        // An empty object or array is a value on one line.
        assert_eq!(
            place("\"empty\": [],"),
            Ok(put(
                vec![key("empty")],
                Whole,
                Object,
                &["name", "tags", "address", "list"]
            ))
        );
        // A member that has things in it opens on a line of its own, with its name.
        assert_eq!(
            place("\"address\": {"),
            Ok(put(
                vec![key("address")],
                Opens('{'),
                Object,
                &["name", "tags", "empty", "list"]
            ))
        );
        // Elements: the one after an element that has lines of its own is the second.
        assert_eq!(
            place("2,"),
            Ok(put(vec![key("list"), Step::Index(1)], Whole, Array, &[]))
        );
        assert_eq!(
            place("3"),
            Ok(put(
                vec![key("list"), Step::Index(2), Step::Index(0)],
                Whole,
                Array,
                &[]
            ))
        );
    }

    #[test]
    fn what_closes_and_what_has_no_name_is_structure() {
        let value = sample();
        let text = lines(&value);
        let lines = borrowed(&text);
        // The opening of the document, and every line that closes something.
        assert_eq!(place_of(&lines, 0), Err(Why::Structure));
        assert_eq!(place_of(&lines, lines.len() - 1), Err(Why::Structure));
        let closer = lines.iter().position(|l| l.trim() == "},").unwrap();
        assert_eq!(place_of(&lines, closer), Err(Why::Structure));
        // An element that is an object opens with no name.
        let opening = lines.iter().position(|l| l.trim() == "{").unwrap();
        assert_eq!(place_of(&lines, opening), Err(Why::Structure));
        // Past the end.
        assert_eq!(place_of(&lines, lines.len()), Err(Why::Structure));
    }

    #[test]
    fn a_document_that_is_one_value_is_its_one_line() {
        for line in ["5", "[]"] {
            assert_eq!(
                place_of(&[line], 0),
                Ok(put(vec![], Part::Whole, Within::Root, &[]))
            );
        }
    }

    #[test]
    fn a_name_with_a_quote_a_colon_or_an_accent_is_read_whole() {
        let value = json!({"a\"b": 1, "x: y": 2, "é": 3, "": 4, "k\\": 5});
        let text = lines(&value);
        let lines = borrowed(&text);
        let names: Vec<String> = (1..lines.len() - 1)
            .map(|at| match &place_of(&lines, at).unwrap().path[..] {
                [Step::Key(name)] => name.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(names, ["a\"b", "x: y", "é", "", "k\\"]);
    }

    #[test]
    fn a_line_that_was_cut_short_is_not_edited_nor_is_one_under_a_name_that_was() {
        let long = "x".repeat(600);
        let value = json!({"a": long.clone(), "b": 1, long.clone(): {"c": 2}});
        let text: Vec<String> = lines(&value)
            .into_iter()
            .map(|l| {
                // As the view shows them.
                if l.chars().count() > MAX_LINE_CHARS {
                    let mut cut: String = l.chars().take(MAX_LINE_CHARS).collect();
                    cut.push('…');
                    cut
                } else {
                    l
                }
            })
            .collect();
        let lines = borrowed(&text);
        assert_eq!(place_of(&lines, 1), Err(Why::Cut));
        assert!(place_of(&lines, 2).is_ok(), "{:?}", lines[2]);
        // The member whose name is cut, and what is in it.
        assert_eq!(place_of(&lines, 3), Err(Why::Cut));
        assert_eq!(place_of(&lines, 4), Err(Why::Cut));
        assert!(Why::Cut.said().is_some() && Why::Structure.said().is_none());
    }

    // Small generated documents.

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn scalar(rng: &mut Rng) -> Value {
        match rng.below(8) {
            0 => json!(null),
            1 => json!(true),
            2 => json!(rng.below(1000)),
            3 => json!(-1.5),
            4 => json!("text"),
            5 => json!("a, b: {c} [d]"),
            6 => json!("é\"q\""),
            _ => json!("{"),
        }
    }

    fn generate(rng: &mut Rng, depth: u32) -> Value {
        if depth == 0 || rng.below(3) == 0 {
            return scalar(rng);
        }
        if rng.below(2) == 0 {
            Value::Array(
                (0..rng.below(5))
                    .map(|_| generate(rng, depth - 1))
                    .collect(),
            )
        } else {
            let names = ["a", "b", "c", "x y", "q\"", "é", "{", "k: v"];
            let mut members = Map::new();
            for _ in 0..rng.below(5) {
                members.insert(
                    names[rng.below(names.len() as u64) as usize].to_owned(),
                    generate(rng, depth - 1),
                );
            }
            Value::Object(members)
        }
    }

    fn at<'a>(root: &'a Value, path: &[Step]) -> Option<&'a Value> {
        path.iter().try_fold(root, |node, step| match (step, node) {
            (Step::Key(key), Value::Object(members)) => members.get(key),
            (Step::Index(at), Value::Array(items)) => items.get(*at),
            _ => None,
        })
    }

    /// What a member or an element should be on its line, as the view writes it: the
    /// name, if it has one, and then the value on one line — or only the opening of it.
    fn expected(root: &Value, place: &Place) -> String {
        let value = at(root, &place.path).unwrap_or_else(|| panic!("{place:?} is not in {root}"));
        let name = match place.path.last() {
            Some(Step::Key(name)) if place.within == Within::Object => {
                format!("{}: ", Value::String(name.clone()))
            }
            _ => String::new(),
        };
        match place.part {
            Part::Whole => format!("{name}{}", render(value)),
            Part::Opens(opening) => format!("{name}{opening}"),
        }
    }

    /// The lines of a document in the order they are in, each with the line a person would
    /// say it is: the place it should come to, or the reason it should not.
    fn truth(
        value: &Value,
        path: &mut Vec<Step>,
        (within, others): (Within, &[String]),
        out: &mut Vec<Result<Place, Why>>,
    ) {
        let named = matches!(path.last(), Some(Step::Key(_)));
        let whole = |out: &mut Vec<Result<Place, Why>>, path: &Vec<Step>| {
            out.push(Ok(Place {
                path: path.clone(),
                part: Part::Whole,
                within,
                others: others.to_vec(),
            }))
        };
        match value {
            Value::Object(members) if !members.is_empty() => {
                out.push(if named {
                    Ok(Place {
                        path: path.clone(),
                        part: Part::Opens('{'),
                        within,
                        others: others.to_vec(),
                    })
                } else {
                    Err(Why::Structure)
                });
                for (name, member) in members {
                    path.push(Step::Key(name.clone()));
                    let beside: Vec<String> =
                        members.keys().filter(|k| *k != name).cloned().collect();
                    truth(member, path, (Within::Object, &beside), out);
                    path.pop();
                }
                out.push(Err(Why::Structure));
            }
            Value::Array(items) if !items.is_empty() => {
                out.push(if named {
                    Ok(Place {
                        path: path.clone(),
                        part: Part::Opens('['),
                        within,
                        others: others.to_vec(),
                    })
                } else {
                    Err(Why::Structure)
                });
                for (index, item) in items.iter().enumerate() {
                    path.push(Step::Index(index));
                    truth(item, path, (Within::Array, &[]), out);
                    path.pop();
                }
                out.push(Err(Why::Structure));
            }
            _ => whole(out, path),
        }
    }

    #[test]
    fn every_line_of_a_generated_document_is_found_where_it_is() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for case in 0..2_000 {
            let value = generate(&mut rng, 4);
            let text = lines(&value);
            let lines = borrowed(&text);
            let mut wanted = Vec::new();
            truth(&value, &mut Vec::new(), (Within::Root, &[]), &mut wanted);
            assert_eq!(wanted.len(), lines.len(), "case {case}: {value}");
            for (at, want) in wanted.iter().enumerate() {
                let found = place_of(&lines, at);
                assert_eq!(
                    &found, want,
                    "case {case}, line {at} {:?}\n{value}",
                    lines[at]
                );
                if let Ok(place) = &found {
                    // The line is what the document has there.
                    let line = lines[at].trim_start();
                    let line = line.strip_suffix(',').unwrap_or(line);
                    assert_eq!(line, expected(&value, place), "case {case}, line {at}");
                }
            }
        }
    }

    #[test]
    fn the_lines_of_either_column_of_the_view_are_found_in_their_own_document() {
        // The right column shows an object's members in the left one's order, so where a
        // line is in the column is not where it is in the document: its name and its count
        // still are.
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let stop = AtomicBool::new(false);
        let mut found = 0;
        for case in 0..1_500 {
            let before = generate(&mut rng, 3);
            let mut after = before.clone();
            shuffle(&mut rng, &mut after);
            if rng.below(2) == 0 {
                *after_mut(&mut rng, &mut after) = scalar(&mut rng);
            }
            let view = compare(&before, &after, &stop).unwrap().view.unwrap();
            for (side, root) in [(Side::Left, &before), (Side::Right, &after)] {
                let text: Vec<&str> = view
                    .rows
                    .iter()
                    .filter_map(|row| match side {
                        Side::Left => row.left.as_deref(),
                        Side::Right => row.right.as_deref(),
                    })
                    .collect();
                for (line, shown) in text.iter().enumerate() {
                    let Ok(place) = place_of(&text, line) else {
                        continue;
                    };
                    found += 1;
                    let shown = shown.trim_start();
                    let shown = shown.strip_suffix(',').unwrap_or(shown);
                    assert_eq!(
                        shown,
                        expected(root, &place),
                        "case {case}, {side:?} line {line}\n  before {before}\n  after  {after}"
                    );
                    // The others are the other members of its own object, as that document
                    // has them (the column shows exactly those).
                    if let Some((Step::Key(own), parents)) = place.path.split_last() {
                        let Some(Value::Object(object)) = at(root, parents) else {
                            panic!("case {case}: {place:?}")
                        };
                        let mut want: Vec<&String> = object.keys().filter(|k| *k != own).collect();
                        let mut got: Vec<&String> = place.others.iter().collect();
                        want.sort();
                        got.sort();
                        assert_eq!(got, want, "case {case}, {side:?} line {line}");
                    }
                }
            }
        }
        assert!(found > 5_000, "{found} lines were checked");
    }

    /// Put the members of every object in `value` in another order.
    fn shuffle(rng: &mut Rng, value: &mut Value) {
        match value {
            Value::Object(members) => {
                let mut taken: Vec<(String, Value)> = std::mem::take(members).into_iter().collect();
                for at in (1..taken.len()).rev() {
                    taken.swap(at, rng.below(at as u64 + 1) as usize);
                }
                for (name, mut member) in taken {
                    shuffle(rng, &mut member);
                    members.insert(name, member);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| shuffle(rng, item)),
            _ => {}
        }
    }

    /// Some value inside `value`, which may be changed.
    fn after_mut<'a>(rng: &mut Rng, value: &'a mut Value) -> &'a mut Value {
        let len = match value {
            Value::Object(members) => members.len(),
            Value::Array(items) => items.len(),
            _ => 0,
        };
        if len == 0 || rng.below(3) == 0 {
            return value;
        }
        let at = rng.below(len as u64) as usize;
        match value {
            Value::Object(members) => after_mut(rng, members.values_mut().nth(at).unwrap()),
            Value::Array(items) => after_mut(rng, &mut items[at]),
            _ => unreachable!(),
        }
    }

    // What is typed.

    fn whole(within: Within) -> Place {
        Place {
            path: vec![key("a")],
            part: Part::Whole,
            within,
            others: vec!["taken".to_owned()],
        }
    }

    #[test]
    fn a_member_is_typed_with_its_name_and_a_comma_does_not_matter() {
        let place = whole(Within::Object);
        for typed in ["  \"a\": 5,", "\"a\": 5", "    \"a\":5 ,", "\t\"a\" : 5"] {
            assert_eq!(
                parse(&place, typed),
                Ok(Fragment::Members(vec![("a".to_owned(), json!(5))])),
                "{typed:?}"
            );
        }
        // Another name, and a value of several lines, typed on one.
        assert_eq!(
            parse(&place, "  \"b\": {\"c\": [1, 2]}"),
            Ok(Fragment::Members(vec![(
                "b".to_owned(),
                json!({"c": [1, 2]})
            )]))
        );
    }

    #[test]
    fn a_name_that_another_member_of_the_object_has_is_not_taken() {
        // (the other member of `whole` is called "taken")
        let place = whole(Within::Object);
        let said = parse(&place, "  \"taken\": 1").unwrap_err();
        assert_eq!(said, "There already is a member called \"taken\" here");
        // One of several.
        let said = parse(&place, "  \"a\": 1, \"taken\": 2").unwrap_err();
        assert!(said.contains("already is a member"), "{said}");
        // Its own name is no other's.
        assert!(parse(&place, "  \"a\": 1").is_ok());

        let opening = Place {
            path: vec![key("address")],
            part: Part::Opens('{'),
            within: Within::Object,
            others: vec!["taken".to_owned()],
        };
        let said = parse(&opening, "  \"taken\": {").unwrap_err();
        assert!(said.contains("already is a member"), "{said}");
        assert!(parse(&opening, "  \"free\": {").is_ok());
    }

    #[test]
    fn several_members_typed_make_several_and_nothing_typed_makes_none() {
        let place = whole(Within::Object);
        assert_eq!(
            parse(&place, "  \"a\": 1, \"b\": 2,"),
            Ok(Fragment::Members(vec![
                ("a".to_owned(), json!(1)),
                ("b".to_owned(), json!(2))
            ]))
        );
        assert_eq!(parse(&place, "      "), Ok(Fragment::Members(vec![])));
        assert_eq!(parse(&place, ","), Ok(Fragment::Members(vec![])));
    }

    #[test]
    fn an_element_is_typed_as_a_value_and_the_document_has_to_stay_one() {
        let place = Place {
            path: vec![key("a"), Step::Index(1)],
            part: Part::Whole,
            within: Within::Array,
            others: vec![],
        };
        assert_eq!(
            parse(&place, "    7,"),
            Ok(Fragment::Elements(vec![json!(7)]))
        );
        assert_eq!(
            parse(&place, "    \"s\", [], {\"k\": null}"),
            Ok(Fragment::Elements(vec![
                json!("s"),
                json!([]),
                json!({"k": null})
            ]))
        );
        assert_eq!(parse(&place, ""), Ok(Fragment::Elements(vec![])));

        let root = Place {
            path: vec![],
            part: Part::Whole,
            within: Within::Root,
            others: vec![],
        };
        assert_eq!(parse(&root, "6"), Ok(Fragment::Elements(vec![json!(6)])));
        assert!(parse(&root, "").unwrap_err().contains("one value"));
        assert!(parse(&root, "1, 2").unwrap_err().contains("one value"));
    }

    #[test]
    fn what_is_not_json_is_said_with_why_and_where() {
        let place = whole(Within::Object);
        let said = parse(&place, "  \"a\": ").unwrap_err();
        assert!(said.starts_with("Not valid JSON: "), "{said}");
        // A member needs a name, and a name a string.
        for typed in ["  5", "  a: 5", "  \"a\" 5", "  \"a\": 5 6", "  \"a\": [1"] {
            let said = parse(&place, typed).unwrap_err();
            assert!(said.starts_with("Not valid JSON: "), "{typed:?}: {said}");
        }
        // The character is counted on the line, indentation and all.
        let said = parse(&place, "  \"a\": nope").unwrap_err();
        assert!(said.contains("(at character 9)"), "{said}");
        // …in characters, not bytes.
        let said = parse(&place, "é \"a\": nope").unwrap_err();
        assert!(said.contains("character"), "{said}");
        let array = Place {
            path: vec![Step::Index(0)],
            part: Part::Whole,
            within: Within::Array,
            others: vec![],
        };
        let said = parse(&array, "  1 2").unwrap_err();
        assert!(said.starts_with("Not valid JSON: "), "{said}");
    }

    #[test]
    fn a_line_that_opens_something_has_only_its_name_changed() {
        let place = Place {
            path: vec![key("address")],
            part: Part::Opens('{'),
            within: Within::Object,
            others: vec!["taken".to_owned()],
        };
        assert_eq!(
            parse(&place, "  \"home\": {"),
            Ok(Fragment::Rename("home".to_owned()))
        );
        assert_eq!(
            parse(&place, "  \"a b\":{"),
            Ok(Fragment::Rename("a b".to_owned()))
        );
        // The end of it has to stay, and it can't become several, or none.
        for typed in [
            "  \"home\": [",
            "  \"home\"",
            "  {",
            "  \"a\": 1, \"b\": {",
            "",
        ] {
            let said = parse(&place, typed).unwrap_err();
            assert!(said.contains("opens an object"), "{typed:?}: {said}");
        }
        let array = Place {
            path: vec![key("tags")],
            part: Part::Opens('['),
            within: Within::Object,
            others: vec![],
        };
        assert!(parse(&array, "  \"t\": {")
            .unwrap_err()
            .contains("an array"));
        assert_eq!(
            parse(&array, "  \"t\": ["),
            Ok(Fragment::Rename("t".to_owned()))
        );
    }

    // Making the change.

    fn edit(path: Vec<Step>, fragment: Fragment) -> Edit {
        Edit {
            into: Side::Left,
            path,
            fragment,
        }
    }

    #[test]
    fn a_member_is_replaced_where_it_is_with_the_members_around_it_in_order() {
        let root: Value = serde_json::from_str(r#"{"z": 1, "a": 2, "m": 3}"#).unwrap();
        let put = |members: Vec<(&str, Value)>| {
            let fragment = Fragment::Members(
                members
                    .into_iter()
                    .map(|(name, value)| (name.to_owned(), value))
                    .collect(),
            );
            apply(root.clone(), &edit(vec![key("a")], fragment)).unwrap()
        };
        let names =
            |value: &Value| -> Vec<String> { value.as_object().unwrap().keys().cloned().collect() };
        // The same name: the value changes, nothing moves.
        let same = put(vec![("a", json!(9))]);
        assert_eq!(names(&same), ["z", "a", "m"]);
        assert_eq!(same["a"], json!(9));
        // Another name takes its place; several go in a row there.
        assert_eq!(names(&put(vec![("b", json!(0))])), ["z", "b", "m"]);
        assert_eq!(
            names(&put(vec![
                ("a", json!(1)),
                ("n", json!(2)),
                ("o", json!(3))
            ])),
            ["z", "a", "n", "o", "m"]
        );
        // None: it goes.
        assert_eq!(names(&put(vec![])), ["z", "m"]);
        // A name that another member has is not taken.
        let said = apply(
            root.clone(),
            &edit(
                vec![key("a")],
                Fragment::Members(vec![("m".to_owned(), json!(0))]),
            ),
        )
        .unwrap_err();
        assert_eq!(said, "There already is a member called \"m\" here");
    }

    #[test]
    fn a_number_is_kept_as_it_was_written() {
        let root: Value = serde_json::from_str(r#"{"a": 1.50, "b": 1}"#).unwrap();
        let fragment = parse(
            &Place {
                path: vec![key("b")],
                part: Part::Whole,
                within: Within::Object,
                others: vec!["a".to_owned()],
            },
            "  \"b\": 100000000000000000000.250",
        )
        .unwrap();
        let out = apply(root, &edit(vec![key("b")], fragment)).unwrap();
        assert_eq!(
            render(&out),
            "{\n  \"a\": 1.50,\n  \"b\": 100000000000000000000.250\n}"
        );
    }

    #[test]
    fn elements_are_replaced_removed_and_put_in_by_their_count() {
        let root = json!({"l": [10, 20, 30]});
        let item = |at: usize, put: Vec<Value>| {
            apply(
                root.clone(),
                &edit(vec![key("l"), Step::Index(at)], Fragment::Elements(put)),
            )
            .unwrap()["l"]
                .clone()
        };
        assert_eq!(item(1, vec![json!(21)]), json!([10, 21, 30]));
        assert_eq!(item(1, vec![]), json!([10, 30]));
        assert_eq!(item(2, vec![json!(1), json!(2)]), json!([10, 20, 1, 2]));
        assert_eq!(item(0, vec![json!([0])]), json!([[0], 20, 30]));
        // Past the end: the document is not the one that was compared.
        assert!(apply(
            root,
            &edit(vec![key("l"), Step::Index(3)], Fragment::Elements(vec![]))
        )
        .is_err());
    }

    #[test]
    fn the_whole_document_is_replaced_by_one_value() {
        let put = |items: Vec<Value>| apply(json!(1), &edit(vec![], Fragment::Elements(items)));
        assert_eq!(put(vec![json!({"a": 1})]), Ok(json!({"a": 1})));
        assert!(put(vec![]).is_err());
        assert!(put(vec![json!(1), json!(2)]).is_err());
    }

    #[test]
    fn a_member_is_renamed_where_it_is_and_not_to_a_name_that_is_taken() {
        let root: Value = serde_json::from_str(r#"{"z": 1, "a": {"k": 1}, "m": 3}"#).unwrap();
        let out = apply(
            root.clone(),
            &edit(vec![key("a")], Fragment::Rename("b".to_owned())),
        )
        .unwrap();
        assert_eq!(
            render(&out),
            render(&json!({"z": 1, "b": {"k": 1}, "m": 3}))
        );
        assert_eq!(
            out.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["z", "b", "m"]
        );
        // The same name is no change; a taken one is refused.
        assert!(apply(
            root.clone(),
            &edit(vec![key("a")], Fragment::Rename("a".to_owned()))
        )
        .is_ok());
        assert!(apply(
            root,
            &edit(vec![key("a")], Fragment::Rename("m".to_owned()))
        )
        .unwrap_err()
        .contains("already is a member"));
    }

    #[test]
    fn an_edit_of_what_is_not_there_or_of_the_wrong_kind_is_refused() {
        let root = json!({"a": [1], "s": "x"});
        for (path, fragment) in [
            (vec![key("missing")], Fragment::Members(vec![])),
            (vec![key("a"), key("k")], Fragment::Members(vec![])),
            (vec![key("s"), Step::Index(0)], Fragment::Elements(vec![])),
            (vec![key("a")], Fragment::Elements(vec![])),
            (vec![Step::Index(0)], Fragment::Elements(vec![])),
            (vec![key("missing")], Fragment::Rename("n".to_owned())),
        ] {
            let said = apply(root.clone(), &edit(path.clone(), fragment)).unwrap_err();
            assert!(said.contains("not as it was"), "{path:?}: {said}");
        }
    }

    #[test]
    fn what_is_typed_over_a_line_of_a_generated_document_is_what_the_document_has_after() {
        // Over every line that can be edited: type the same value, and the document is as
        // it was; type another, and exactly that member or element changed.
        let mut rng = Rng(0x1234_5678_9abc_def1);
        let mut edited = 0;
        for case in 0..1_500 {
            let value = generate(&mut rng, 3);
            let text = lines(&value);
            let lines = borrowed(&text);
            for (at, shown) in lines.iter().enumerate() {
                let Ok(place) = place_of(&lines, at) else {
                    continue;
                };
                if place.part != Part::Whole {
                    continue;
                }
                edited += 1;
                // The line, as it is.
                let fragment = parse(&place, shown).unwrap();
                let same = apply(value.clone(), &edit(place.path.clone(), fragment)).unwrap();
                assert_eq!(render(&same), render(&value), "case {case}, line {at}");
                // The value of it replaced.
                let name = match place.path.last() {
                    Some(Step::Key(name)) if place.within == Within::Object => {
                        format!("{}: ", Value::String(name.clone()))
                    }
                    _ => String::new(),
                };
                let typed = format!("{name}\"typed\"");
                let fragment = parse(&place, &typed).unwrap();
                let out = apply(value.clone(), &edit(place.path.clone(), fragment)).unwrap();
                assert_eq!(
                    at_path(&out, &place.path),
                    Some(&json!("typed")),
                    "case {case}"
                );
                // Taken out: it is not there any more, and the rest is.
                if place.within == Within::Root {
                    assert!(parse(&place, "").is_err());
                } else {
                    let fragment = parse(&place, "").unwrap();
                    let out = apply(value.clone(), &edit(place.path.clone(), fragment)).unwrap();
                    let mut without = value.clone();
                    remove(&mut without, &place.path);
                    assert_eq!(
                        render(&out),
                        render(&without),
                        "case {case}, line {at} {shown:?}\n{value}"
                    );
                }
            }
        }
        assert!(edited > 3_000, "{edited} lines were edited");
    }

    fn at_path<'a>(root: &'a Value, path: &[Step]) -> Option<&'a Value> {
        at(root, path)
    }

    /// `value` without what is at `path`, which is not the document itself.
    fn remove(value: &mut Value, path: &[Step]) {
        let (last, parents) = path.split_last().unwrap();
        let mut node = value;
        for step in parents {
            node = match (step, node) {
                (Step::Key(key), Value::Object(members)) => members.get_mut(key).unwrap(),
                (Step::Index(at), Value::Array(items)) => &mut items[*at],
                _ => unreachable!(),
            };
        }
        match (last, node) {
            (Step::Key(key), Value::Object(members)) => {
                *members = std::mem::take(members)
                    .into_iter()
                    .filter(|(name, _)| name != key)
                    .collect();
            }
            (Step::Index(at), Value::Array(items)) => {
                items.remove(*at);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn deleting_says_so_for_the_page() {
        let nothing = edit(vec![key("a")], Fragment::Members(vec![]));
        assert!(nothing.deletes());
        assert!(!edit(vec![key("a")], Fragment::Rename("b".to_owned())).deletes());
        assert!(!edit(vec![], Fragment::Elements(vec![json!(1)])).deletes());
    }
}
