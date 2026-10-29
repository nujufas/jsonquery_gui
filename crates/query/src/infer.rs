//! Making a JSON Schema from a document — what the Tools window's Create
//! Schema does.
//!
//! The document is walked once, and what is seen at each place is summed up:
//! the types of the values, the members of the objects and how many of them
//! have each, what the strings have in common (`text.rs`). The elements of an
//! array are one place, so a list of a thousand orders makes one schema of an
//! order. The schema is written from that: `type`, `properties` and
//! `required`, `items`, and for strings the `format` and the `pattern` of what
//! they were found to be — a date, a uuid, an address, a code of one shape, a
//! list of them with commas between (`kinds.rs`).
//!
//! An object whose keys are data and not names — all of them uuids, dates,
//! numbers… — is a map: its values are one place, under `patternProperties`. So
//! are members whose keys are a name and a number, as `phone1` and `phone2`
//! are.
//!
//! The schema made is then checked against the document, by the validator the
//! Validate page uses, with formats: a `format` or a `pattern` that a value
//! does not satisfy after all (the validator's idea of a format can be
//! stricter) is taken out again, so the schema fits the document it was made
//! from.

mod kinds;
mod text;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use jsonquery_core::lazy::{self, LazyTree};
use jsonquery_core::ValueKind;
use regex::Regex;
use serde_json::{Map, Number, Value};

use crate::schema;
use kinds::{escape, Kind};
use text::{Source, Strings};

/// Values deeper than this are not looked into: the schema allows anything
/// there.
const MAX_DEPTH: usize = 128;

/// The most different keys the objects of one place have their members kept
/// apart for; past that, the values of all of them are one place, as a map's.
const MAX_KEYS: usize = 10_000;

/// An `enum` is of this many different strings at most, each no longer than
/// this.
const ENUM_MOST: usize = text::MAX_VALUES;
const ENUM_LONGEST: usize = 64;

/// How often the walk looks at the cancel flag, in values.
const CANCEL_EVERY: usize = 4096;

/// The draft of JSON Schema the schema is written for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Draft {
    D4,
    D6,
    D7,
    D201909,
    #[default]
    D202012,
}

impl Draft {
    /// The newest first, as a list to pick from has them.
    pub const ALL: [Draft; 5] = [
        Draft::D202012,
        Draft::D201909,
        Draft::D7,
        Draft::D6,
        Draft::D4,
    ];

    /// "Draft 2020-12".
    pub fn name(self) -> &'static str {
        match self {
            Draft::D4 => "Draft 4",
            Draft::D6 => "Draft 6",
            Draft::D7 => "Draft 7",
            Draft::D201909 => "Draft 2019-09",
            Draft::D202012 => "Draft 2020-12",
        }
    }

    /// What `$schema` says.
    pub fn uri(self) -> &'static str {
        match self {
            Draft::D4 => "http://json-schema.org/draft-04/schema#",
            Draft::D6 => "http://json-schema.org/draft-06/schema#",
            Draft::D7 => "http://json-schema.org/draft-07/schema#",
            Draft::D201909 => "https://json-schema.org/draft/2019-09/schema",
            Draft::D202012 => "https://json-schema.org/draft/2020-12/schema",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub draft: Draft,
    /// List, under `required`, the members that every object of a place has.
    pub required: bool,
    /// Allow no members but those seen: `additionalProperties: false`.
    pub closed: bool,
    /// Say what strings were found to be, as a `format` and a `pattern`.
    pub patterns: bool,
    /// A few different strings, some of them again and again, are an `enum`.
    pub enums: bool,
    /// The least and the most seen: `minimum` and `maximum` of numbers,
    /// `minLength` and `maxLength` of strings, `minItems` and `maxItems` of
    /// arrays — and the lengths in the patterns.
    pub bounds: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            draft: Draft::default(),
            required: true,
            closed: false,
            patterns: true,
            enums: false,
            bounds: false,
        }
    }
}

/// A pattern found in the document: what the strings at a place are, or what
/// the keys of an object are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Where, as a path of jq: `.orders[].id`; `[]` is every element of an
    /// array, or every value of a map.
    pub path: String,
    /// What was found, in a few words: "uuid", "comma-separated: email",
    /// "keys: date".
    pub what: String,
    /// The regular expression the schema says they match.
    pub expression: String,
    /// The `format` the schema says they have, if it says one.
    pub format: Option<String>,
    /// It is about the keys of the objects at `path`, not their values.
    pub keys: bool,
    /// The values of the `enum` the strings at `path` were made (a null among
    /// them when there were nulls too): then it is no pattern, and has no
    /// expression. Empty for a pattern.
    pub values: Vec<Value>,
    /// How many times each of `values` was in the document.
    pub times: Vec<usize>,
    /// A JSON Pointer to the schema the expression is in.
    schema_at: String,
}

impl Found {
    /// It is an `enum`, not a pattern.
    pub fn is_enum(&self) -> bool {
        !self.values.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct Created {
    pub schema: Value,
    /// The patterns found, in the order of the schema.
    pub found: Vec<Found>,
    /// How many members of objects the schema names, at every depth.
    pub properties: usize,
    /// What the validator still finds wrong with the document against the schema
    /// (nothing, unless something here is wrong).
    pub problems: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    /// The schema made can't be used to check the document: a fault of this
    /// module. The text says what.
    Schema(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Cancelled => f.write_str("cancelled"),
            Error::Schema(message) => write!(f, "the schema made can't be used: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// Make a schema of `value`.
pub fn create(value: &Value, options: Options, cancel: &AtomicBool) -> Result<Created, Error> {
    let mut root = Node::default();
    Walker { cancel, steps: 0 }.add(&mut root, value, 0)?;

    let mut writer = Writer {
        options,
        found: Vec::new(),
        properties: 0,
    };
    let body = writer.schema(&root, "", "");
    let mut schema = Map::new();
    schema.insert("$schema".to_owned(), Value::from(options.draft.uri()));
    if let Value::Object(body) = body {
        schema.extend(body);
    }
    let mut schema = Value::Object(schema);
    let problems = writer.check(&mut schema, value, cancel)?;
    Ok(Created {
        schema,
        found: writer.found,
        properties: writer.properties,
        problems,
    })
}

/// What was seen at one place of the document.
#[derive(Clone, Debug, Default)]
struct Node {
    seen: usize,
    nulls: usize,
    booleans: usize,
    integers: Numbers,
    /// Numbers with a fraction or an exponent.
    decimals: Numbers,
    strings: Strings,
    arrays: Option<Box<Arrays>>,
    objects: Option<Box<Objects>>,
}

impl Node {
    fn merge(&mut self, other: &Node) {
        self.seen += other.seen;
        self.nulls += other.nulls;
        self.booleans += other.booleans;
        self.integers.merge(&other.integers);
        self.decimals.merge(&other.decimals);
        self.strings.merge(&other.strings);
        if let Some(theirs) = &other.arrays {
            match &mut self.arrays {
                Some(mine) => mine.merge(theirs),
                None => self.arrays = Some(theirs.clone()),
            }
        }
        if let Some(theirs) = &other.objects {
            match &mut self.objects {
                Some(mine) => mine.merge(theirs),
                None => self.objects = Some(theirs.clone()),
            }
        }
    }
}

/// The numbers of a place: how many, the least and the most.
#[derive(Clone, Debug, Default)]
struct Numbers {
    count: usize,
    least: Option<(f64, Number)>,
    most: Option<(f64, Number)>,
}

impl Numbers {
    fn add(&mut self, number: &Number) {
        self.count += 1;
        let Some(x) = number.as_f64() else {
            return;
        };
        if self.least.as_ref().is_none_or(|(least, _)| x < *least) {
            self.least = Some((x, number.clone()));
        }
        if self.most.as_ref().is_none_or(|(most, _)| x > *most) {
            self.most = Some((x, number.clone()));
        }
    }

    fn merge(&mut self, other: &Numbers) {
        self.count += other.count;
        if let Some((x, n)) = &other.least {
            if self.least.as_ref().is_none_or(|(least, _)| x < least) {
                self.least = Some((*x, n.clone()));
            }
        }
        if let Some((x, n)) = &other.most {
            if self.most.as_ref().is_none_or(|(most, _)| x > most) {
                self.most = Some((*x, n.clone()));
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Arrays {
    count: usize,
    fewest: usize,
    most: usize,
    /// Every element of every one.
    items: Node,
}

impl Arrays {
    fn merge(&mut self, other: &Arrays) {
        self.fewest = if self.count == 0 {
            other.fewest
        } else {
            self.fewest.min(other.fewest)
        };
        self.most = self.most.max(other.most);
        self.count += other.count;
        self.items.merge(&other.items);
    }
}

#[derive(Clone, Debug, Default)]
struct Objects {
    count: usize,
    /// In the order their keys were first seen.
    members: Vec<Member>,
    index: HashMap<String, usize>,
    /// The keys: each once when they are kept apart, every time when they are
    /// not (see `rest`).
    keys: Strings,
    /// The values of the members whose keys came after [`MAX_KEYS`] different
    /// ones: too many to keep apart.
    rest: Option<Node>,
}

#[derive(Clone, Debug)]
struct Member {
    key: String,
    /// How many of the objects have it.
    present: usize,
    node: Node,
}

impl Objects {
    fn new() -> Self {
        Self {
            keys: Strings::plain(),
            ..Self::default()
        }
    }

    /// Where the values of `key` go, and a note that one more object has it.
    fn member(&mut self, key: &str) -> &mut Node {
        if let Some(&i) = self.index.get(key) {
            let member = &mut self.members[i];
            member.present += 1;
            return &mut member.node;
        }
        self.keys.add(key);
        if self.members.len() >= MAX_KEYS {
            return self.rest.get_or_insert_with(Node::default);
        }
        self.index.insert(key.to_owned(), self.members.len());
        self.members.push(Member {
            key: key.to_owned(),
            present: 1,
            node: Node::default(),
        });
        let last = self.members.len() - 1;
        &mut self.members[last].node
    }

    fn merge(&mut self, other: &Objects) {
        self.count += other.count;
        for theirs in &other.members {
            match self.index.get(&theirs.key) {
                Some(&i) => {
                    let mine = &mut self.members[i];
                    mine.present += theirs.present;
                    mine.node.merge(&theirs.node);
                }
                None => {
                    self.keys.add(&theirs.key);
                    if self.members.len() >= MAX_KEYS {
                        self.rest
                            .get_or_insert_with(Node::default)
                            .merge(&theirs.node);
                    } else {
                        self.index.insert(theirs.key.clone(), self.members.len());
                        self.members.push(theirs.clone());
                    }
                }
            }
        }
        if let Some(theirs) = &other.rest {
            self.keys.merge(&other.keys);
            self.rest.get_or_insert_with(Node::default).merge(theirs);
        }
    }

    /// The values of every member, as one place.
    fn all_values(&self) -> Node {
        let mut all = self.rest.clone().unwrap_or_default();
        for member in &self.members {
            all.merge(&member.node);
        }
        all
    }
}

/// Walks a document into the [`Node`]s of its places.
struct Walker<'a> {
    cancel: &'a AtomicBool,
    steps: usize,
}

impl Walker<'_> {
    fn add(&mut self, node: &mut Node, value: &Value, depth: usize) -> Result<(), Error> {
        self.steps += 1;
        if self.steps.is_multiple_of(CANCEL_EVERY) && self.cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        node.seen += 1;
        // A place is at one depth, so a place this deep has nothing but such values.
        if depth >= MAX_DEPTH {
            return Ok(());
        }
        match value {
            Value::Null => node.nulls += 1,
            Value::Bool(_) => node.booleans += 1,
            Value::Number(n) if is_integer(n) => node.integers.add(n),
            Value::Number(n) => node.decimals.add(n),
            Value::String(text) => node.strings.add(text),
            Value::Array(elements) => {
                let arrays = node.arrays.get_or_insert_with(Box::default);
                arrays.fewest = if arrays.count == 0 {
                    elements.len()
                } else {
                    arrays.fewest.min(elements.len())
                };
                arrays.most = arrays.most.max(elements.len());
                arrays.count += 1;
                for element in elements {
                    self.add(&mut arrays.items, element, depth + 1)?;
                }
            }
            Value::Object(members) => {
                let objects = node.objects.get_or_insert_with(|| Box::new(Objects::new()));
                objects.count += 1;
                for (key, value) in members {
                    let place = objects.member(key);
                    self.add(place, value, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

/// A number written without a fraction or an exponent. (Its text is kept: the
/// document is read with exact numbers.)
fn is_integer(number: &Number) -> bool {
    !number.to_string().contains(['.', 'e', 'E'])
}

/// The kinds of keys that make an object a map: keys that are data.
fn keys_of_a_map(kind: Kind) -> bool {
    !matches!(
        kind,
        Kind::Duration | Kind::HexColour | Kind::BooleanText | Kind::Slug
    )
}

/// Writes the schema of the places, and keeps the list of what it found.
struct Writer {
    options: Options,
    found: Vec<Found>,
    properties: usize,
}

impl Writer {
    /// The schema of `node`, which is at `path` in the document (as jq says it,
    /// "" for the document) and will be at `at` in the schema (a JSON Pointer).
    fn schema(&mut self, node: &Node, path: &str, at: &str) -> Value {
        let mut out = Map::new();
        if node.seen == 0 {
            return Value::Object(out);
        }
        let (integers, decimals) = (node.integers.count, node.decimals.count);
        let numbers = integers + decimals;
        let mut types = Vec::new();
        for (count, name) in [
            (node.objects.as_ref().map_or(0, |o| o.count), "object"),
            (node.arrays.as_ref().map_or(0, |a| a.count), "array"),
            (node.strings.count, "string"),
            // Whole numbers among others are numbers.
            (if decimals == 0 { integers } else { 0 }, "integer"),
            (decimals, "number"),
            (node.booleans, "boolean"),
            (node.nulls, "null"),
        ] {
            if count > 0 {
                types.push(Value::from(name));
            }
        }
        match types.len() {
            // Too deep to look into.
            0 => return Value::Object(out),
            1 => out.insert("type".to_owned(), types.remove(0)),
            _ => out.insert("type".to_owned(), Value::Array(types)),
        };

        if node.strings.count > 0 {
            self.strings(node, path, at, &mut out);
        }
        if numbers > 0 && self.options.bounds {
            let mut both = node.integers.clone();
            both.merge(&node.decimals);
            if let (Some((_, least)), Some((_, most))) = (both.least, both.most) {
                out.insert("minimum".to_owned(), Value::Number(least));
                out.insert("maximum".to_owned(), Value::Number(most));
            }
        }
        if let Some(objects) = &node.objects {
            self.object(objects, path, at, &mut out);
        }
        if let Some(arrays) = &node.arrays {
            if arrays.items.seen > 0 {
                let items = self.schema(&arrays.items, &every(path), &format!("{at}/items"));
                out.insert("items".to_owned(), items);
            }
            if self.options.bounds {
                out.insert("minItems".to_owned(), Value::from(arrays.fewest));
                out.insert("maxItems".to_owned(), Value::from(arrays.most));
            }
        }
        Value::Object(out)
    }

    /// What the strings of a place are: an `enum`, or a `format` and a
    /// `pattern`; and their lengths.
    fn strings(&mut self, node: &Node, path: &str, at: &str, out: &mut Map<String, Value>) {
        let strings = &node.strings;
        let only_strings = node.seen == strings.count + node.nulls;
        let values = strings
            .few_values(ENUM_LONGEST)
            .filter(|values| values.len() <= ENUM_MOST && strings.count > values.len());
        match values {
            Some(values) if self.options.enums && only_strings => {
                let (mut list, mut times): (Vec<Value>, Vec<usize>) = values
                    .into_iter()
                    .map(|(value, times)| (Value::from(value), times))
                    .unzip();
                if node.nulls > 0 {
                    list.push(Value::Null);
                    times.push(node.nulls);
                }
                out.insert("enum".to_owned(), Value::Array(list.clone()));
                self.found.push(Found {
                    path: shown(path),
                    what: "enum".to_owned(),
                    expression: String::new(),
                    format: None,
                    keys: false,
                    values: list,
                    times,
                    schema_at: at.to_owned(),
                });
            }
            _ if self.options.patterns => {
                if let Some(pattern) = strings.pattern(self.options.bounds, self.options.draft) {
                    if let Some(format) = pattern.format {
                        out.insert("format".to_owned(), Value::from(format));
                    }
                    out.insert(
                        "pattern".to_owned(),
                        Value::from(pattern.expression.clone()),
                    );
                    self.found.push(Found {
                        path: shown(path),
                        what: pattern.what,
                        expression: pattern.expression,
                        format: pattern.format.map(str::to_owned),
                        keys: false,
                        values: Vec::new(),
                        times: Vec::new(),
                        schema_at: at.to_owned(),
                    });
                }
            }
            _ => {}
        }
        if self.options.bounds {
            let (least, most) = strings.lengths();
            out.insert("minLength".to_owned(), Value::from(least));
            out.insert("maxLength".to_owned(), Value::from(most));
        }
    }

    /// The members of the objects of a place: a map's values, or the members
    /// one by one, with those whose keys are a name and a number together.
    fn object(&mut self, objects: &Objects, path: &str, at: &str, out: &mut Map<String, Value>) {
        if let Some(expression) = self.map_keys(objects, path, at) {
            let values = objects.all_values();
            let values_at = format!("{at}/patternProperties/{}", pointer_word(&expression));
            let schema = self.schema(&values, &every(path), &values_at);
            let mut patterns = Map::new();
            patterns.insert(expression, schema);
            out.insert("patternProperties".to_owned(), Value::Object(patterns));
            if self.options.closed {
                out.insert("additionalProperties".to_owned(), Value::Bool(false));
            }
            return;
        }
        if objects.rest.is_some() {
            // Too many keys to name, and nothing in common: any key, whose value is
            // what all of them are.
            let values = objects.all_values();
            let schema = self.schema(&values, &every(path), &format!("{at}/additionalProperties"));
            out.insert("additionalProperties".to_owned(), schema);
            return;
        }

        // Members whose keys are a name and a number: two or more of the same name.
        let mut groups: Vec<(&str, Vec<&Member>)> = Vec::new();
        for member in objects.members.iter().filter(|_| self.options.patterns) {
            if let Some(name) = numbered(&member.key) {
                match groups.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, members)) => members.push(member),
                    None => groups.push((name, vec![member])),
                }
            }
        }
        groups.retain(|(_, members)| members.len() >= 2);
        let grouped = |key: &str| {
            groups
                .iter()
                .any(|(_, members)| members.iter().any(|m| m.key == key))
        };

        let mut properties = Map::new();
        let mut required = Vec::new();
        for member in objects.members.iter().filter(|m| !grouped(&m.key)) {
            let child = self.schema(
                &member.node,
                &step(path, &member.key),
                &format!("{at}/properties/{}", pointer_word(&member.key)),
            );
            properties.insert(member.key.clone(), child);
            if member.present == objects.count {
                required.push(Value::from(member.key.clone()));
            }
        }
        self.properties += properties.len();
        if !properties.is_empty() {
            out.insert("properties".to_owned(), Value::Object(properties));
        }

        let mut patterns = Map::new();
        for (name, members) in &groups {
            let expression = format!("^{}[0-9]+$", escape(name));
            let mut values = Node::default();
            for member in members {
                values.merge(&member.node);
            }
            let shown_keys = if members.len() == 2 {
                format!("{}, {}", members[0].key, members[1].key)
            } else {
                format!("{}, {}…", members[0].key, members[1].key)
            };
            self.found.push(Found {
                path: shown(path),
                what: format!("numbered keys: {shown_keys}"),
                expression: expression.clone(),
                format: None,
                keys: true,
                values: Vec::new(),
                times: Vec::new(),
                schema_at: at.to_owned(),
            });
            let values_at = format!("{at}/patternProperties/{}", pointer_word(&expression));
            let schema = self.schema(&values, &numbered_step(path, name), &values_at);
            patterns.insert(expression, schema);
        }
        if !patterns.is_empty() {
            out.insert("patternProperties".to_owned(), Value::Object(patterns));
        }
        if self.options.closed {
            out.insert("additionalProperties".to_owned(), Value::Bool(false));
        }
        if self.options.required && !required.is_empty() {
            out.insert("required".to_owned(), Value::Array(required));
        }
    }

    /// The expression the keys of the objects match, when they are data — two or
    /// more, all of a kind such as a uuid, a date or a number, or of one shape —
    /// and the objects are maps. Says so in the list of what was found.
    fn map_keys(&mut self, objects: &Objects, path: &str, at: &str) -> Option<String> {
        if !self.options.patterns || objects.members.len() + usize::from(objects.rest.is_some()) < 2
        {
            return None;
        }
        let pattern = objects.keys.pattern(false, self.options.draft)?;
        match pattern.source {
            Source::Kind(kind) if keys_of_a_map(kind) => {}
            Source::Shape => {}
            _ => return None,
        }
        self.found.push(Found {
            path: shown(path),
            what: format!("keys: {}", pattern.what),
            expression: pattern.expression.clone(),
            format: None,
            keys: true,
            values: Vec::new(),
            times: Vec::new(),
            schema_at: at.to_owned(),
        });
        Some(pattern.expression)
    }

    /// Check the document against the schema made of it, and take out a
    /// `format` or a `pattern` that the validator finds a value does not
    /// satisfy. Gives how many problems are left (none, but for a fault here).
    fn check(
        &mut self,
        schema: &mut Value,
        value: &Value,
        cancel: &AtomicBool,
    ) -> Result<usize, Error> {
        let options = schema::Options {
            check_formats: true,
        };
        let validate = |schema: &Value| {
            schema::validate(schema, value, options, cancel).map_err(|e| match e {
                schema::Error::Cancelled => Error::Cancelled,
                schema::Error::Schema(message) => Error::Schema(message),
            })
        };
        // Each round can only take keywords out, and there are only so many.
        loop {
            let report = validate(schema)?;
            let mut mended = false;
            for problem in &report.problems {
                for keyword in ["format", "pattern"] {
                    let Some(at) = problem.schema_path.strip_suffix(&format!("/{keyword}")) else {
                        continue;
                    };
                    if let Some(Value::Object(members)) = schema.pointer_mut(at) {
                        mended |= members.remove(keyword).is_some();
                    }
                    self.forget(at, keyword);
                }
            }
            if !mended {
                return Ok(report.problems.len() + usize::from(report.more));
            }
        }
    }

    /// The schema at `at` no longer says `keyword`.
    fn forget(&mut self, at: &str, keyword: &str) {
        match keyword {
            "format" => {
                for found in &mut self.found {
                    if !found.keys && found.schema_at == at {
                        found.format = None;
                    }
                }
            }
            _ => self.found.retain(|f| f.keys || f.schema_at != at),
        }
    }
}

/// The name of a key that is a name and a number (`phone2`, `line_10`): the
/// name, which has a letter in it.
fn numbered(key: &str) -> Option<&str> {
    let name = key.trim_end_matches(|c: char| c.is_ascii_digit());
    (name.len() < key.len() && name.chars().any(char::is_alphabetic)).then_some(name)
}

/// The path of the member `key` of what is at `path`, as jq says it.
fn step(path: &str, key: &str) -> String {
    let plain = key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain {
        format!("{path}.{key}")
    } else {
        let quoted = serde_json::to_string(key).unwrap_or_default();
        if path.is_empty() {
            format!(".[{quoted}]")
        } else {
            format!("{path}[{quoted}]")
        }
    }
}

/// The path of every element of the array, or every value of the map, at
/// `path`.
fn every(path: &str) -> String {
    if path.is_empty() {
        ".[]".to_owned()
    } else {
        format!("{path}[]")
    }
}

/// The path of the members whose keys are `name` and a number: `.phone<n>`.
fn numbered_step(path: &str, name: &str) -> String {
    step(path, name) + "<n>"
}

/// A path as it is shown: the document itself is `.`.
fn shown(path: &str) -> String {
    if path.is_empty() {
        ".".to_owned()
    } else {
        path.to_owned()
    }
}

/// `word` as a step of a JSON Pointer.
fn pointer_word(word: &str) -> String {
    word.replace('~', "~0").replace('/', "~1")
}

// ---------------------------------------------------------------------------
// Changing a pattern of a schema made here, as a person asks.
// ---------------------------------------------------------------------------

/// What a pattern that was found says in the schema now, as a person has
/// changed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Now {
    /// What was found.
    Found,
    /// An expression of the person's own.
    Expression(String),
    /// An enum of the person's own: some of the values found left out, others
    /// put in.
    Values(Vec<Value>),
    /// No pattern: for strings, a string of any kind (no `pattern`, no
    /// `format`, no `enum`); for keys, any key (their values under
    /// `additionalProperties`).
    Plain,
}

/// The order a schema of this module has its keywords in, which one put back
/// takes its place in.
const KEYWORDS: [&str; 17] = [
    "$schema",
    "type",
    "format",
    "pattern",
    "enum",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "properties",
    "patternProperties",
    "additionalProperties",
    "required",
    "items",
    "minItems",
    "maxItems",
    "$defs",
];

/// The expression `found` has in the schema when it says `now`; none when it
/// is plain, or an enum.
pub fn expression_now<'a>(found: &'a Found, now: &'a Now) -> Option<&'a str> {
    match now {
        Now::Found if found.is_enum() => None,
        Now::Found => Some(&found.expression),
        Now::Expression(expression) => Some(expression),
        Now::Values(_) | Now::Plain => None,
    }
}

/// The values of the enum `found` has in the schema when it says `now`; none
/// when it is plain, or a pattern.
pub fn values_now<'a>(found: &'a Found, now: &'a Now) -> Option<&'a [Value]> {
    match now {
        Now::Found if found.is_enum() => Some(&found.values),
        Now::Values(values) => Some(values),
        _ => None,
    }
}

/// Whether `expression` is one a schema can have as a `pattern` (or a key of
/// `patternProperties`): the validator of the Validate page reads it. The error
/// says why not.
pub fn check_expression(expression: &str) -> Result<(), String> {
    if expression.is_empty() {
        return Err("write a pattern, or make it plain".to_owned());
    }
    let probe = serde_json::json!({ "pattern": expression });
    schema::validate(
        &probe,
        &Value::Null,
        schema::Options::default(),
        &AtomicBool::new(false),
    )
    .map(|_| ())
    .map_err(|e| match e {
        schema::Error::Schema(message) => message,
        schema::Error::Cancelled => "cancelled".to_owned(),
    })
}

/// Change what the pattern `found` says in `schema` from `now` to `to`: put an
/// expression in place of the one there (for an enum, other values), make it
/// plain, or put back what was found (`made` is the schema as it was made,
/// which says what an object of keys allowed beside them). The error says why
/// it could not be made, and nothing is changed then.
pub fn change(
    schema: &mut Value,
    made: &Value,
    found: &Found,
    now: &Now,
    to: &Now,
) -> Result<(), String> {
    if found.is_enum() {
        return change_values(schema, found, to);
    }
    if matches!(to, Now::Values(_)) {
        return Err("A pattern has an expression, not values".to_owned());
    }
    if let Some(expression) = expression_now(found, to) {
        check_expression(expression).map_err(|e| format!("The pattern can't be used: {e}"))?;
    }
    let Some(Value::Object(members)) = schema.pointer_mut(&found.schema_at) else {
        return Err("That place is no longer in the schema".to_owned());
    };
    if found.keys {
        let allowed = made
            .pointer(&found.schema_at)
            .and_then(|m| m.get("additionalProperties"))
            .cloned();
        change_keys(members, found, now, to, allowed)
    } else {
        match to {
            Now::Plain => {
                members.remove("pattern");
                members.remove("format");
            }
            Now::Expression(expression) => put(members, "pattern", Value::from(expression.clone())),
            Now::Values(_) => {}
            Now::Found => {
                put(members, "pattern", Value::from(found.expression.clone()));
                match &found.format {
                    Some(format) => put(members, "format", Value::from(format.clone())),
                    None => {
                        members.remove("format");
                    }
                }
            }
        }
        Ok(())
    }
}

/// [`change`] for an enum: its values, or none.
fn change_values(schema: &mut Value, found: &Found, to: &Now) -> Result<(), String> {
    if matches!(to, Now::Expression(_)) {
        return Err("An enum has values, not a pattern".to_owned());
    }
    let values = values_now(found, to);
    if let Some(values) = values {
        if values.is_empty() {
            return Err("Keep a value at least, or make it a plain string".to_owned());
        }
        if let Some(again) = values
            .iter()
            .enumerate()
            .find_map(|(i, v)| values[..i].contains(v).then_some(v))
        {
            return Err(format!("{again} is in the enum already"));
        }
    }
    let Some(Value::Object(members)) = schema.pointer_mut(&found.schema_at) else {
        return Err("That place is no longer in the schema".to_owned());
    };
    match values {
        Some(values) => put(members, "enum", Value::Array(values.to_vec())),
        None => {
            members.remove("enum");
        }
    }
    Ok(())
}

/// [`change`] for a pattern of keys: a key of `patternProperties` is renamed, or
/// its values become those of any key (`additionalProperties`), or come back.
/// `allowed` is what the object allowed beside them as it was made (`false`, or
/// nothing).
fn change_keys(
    members: &mut Map<String, Value>,
    found: &Found,
    now: &Now,
    to: &Now,
    allowed: Option<Value>,
) -> Result<(), String> {
    let patterns = |members: &Map<String, Value>| {
        members
            .get("patternProperties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    };
    // Where their values' schema is now.
    let values = match expression_now(found, now) {
        Some(key) => patterns(members)
            .get(key)
            .cloned()
            .ok_or("Those keys are no longer in the schema")?,
        None => match members.get("additionalProperties") {
            Some(Value::Object(values)) => Value::Object(values.clone()),
            _ => return Err("Those keys are no longer in the schema".to_owned()),
        },
    };
    let mut table = patterns(members);
    match (expression_now(found, now), expression_now(found, to)) {
        (Some(from), Some(into)) => {
            if from != into && table.contains_key(into) {
                return Err(format!("These keys already have the pattern {into}"));
            }
            table = table
                .into_iter()
                .map(|(key, value)| {
                    if key == from {
                        (into.to_owned(), value)
                    } else {
                        (key, value)
                    }
                })
                .collect();
        }
        (Some(from), None) => {
            if matches!(members.get("additionalProperties"), Some(Value::Object(_))) {
                return Err("Another pattern of these keys is any key already".to_owned());
            }
            table.remove(from);
            put(members, "additionalProperties", values);
        }
        (None, Some(into)) => {
            if table.contains_key(into) {
                return Err(format!("These keys already have the pattern {into}"));
            }
            table.insert(into.to_owned(), values);
            match allowed {
                Some(allowed) => put(members, "additionalProperties", allowed),
                None => {
                    members.remove("additionalProperties");
                }
            }
        }
        (None, None) => {}
    }
    if table.is_empty() {
        members.remove("patternProperties");
    } else {
        put(members, "patternProperties", Value::Object(table));
    }
    Ok(())
}

/// Set `key` of a schema to `value`: where it is, if it is there, or else in its
/// place among the others (see [`KEYWORDS`]).
fn put(members: &mut Map<String, Value>, key: &str, value: Value) {
    if let Some(there) = members.get_mut(key) {
        *there = value;
        return;
    }
    let rank = |k: &str| KEYWORDS.iter().position(|w| *w == k);
    let mine = rank(key).unwrap_or(KEYWORDS.len());
    let mut value = Some(value);
    for (k, v) in std::mem::take(members) {
        if rank(&k).is_some_and(|r| r > mine) {
            if let Some(value) = value.take() {
                members.insert(key.to_owned(), value);
            }
        }
        members.insert(k, v);
    }
    if let Some(value) = value {
        members.insert(key.to_owned(), value);
    }
}

// ---------------------------------------------------------------------------
// Where a pattern is: in the document it was found in, and in the schema.
// ---------------------------------------------------------------------------

/// Where in `document` (the text the schema was made of) what `found` is about
/// is: the strings at its place, or the keys of the objects there that match
/// it. As ranges of bytes, in the order of the text; none for text that is not
/// JSON.
///
/// The place is found the way the schema reaches it, a keyword at a time: a
/// member of `properties`, every element (`items`), the members whose keys
/// match a key of `patternProperties`, every member (`additionalProperties`).
pub fn occurrences(found: &Found, document: &str) -> Vec<Range<usize>> {
    let Ok(tree) = LazyTree::from_vec(document.as_bytes().to_vec()) else {
        return Vec::new();
    };
    let mut nodes = vec![tree.root()];
    let mut steps = schema::steps_of(&found.schema_at);
    while let Some(step) = steps.next() {
        nodes = match step.as_str() {
            "properties" => {
                let Some(key) = steps.next() else {
                    return Vec::new();
                };
                nodes
                    .iter()
                    .filter_map(|node| node.child_by_key(&key))
                    .map(|child| child.node)
                    .collect()
            }
            "patternProperties" => {
                let Some(expression) = steps.next() else {
                    return Vec::new();
                };
                let matching = keys_matching(&expression);
                children(&nodes, ValueKind::Object)
                    .into_iter()
                    .filter(|child| matching(child))
                    .map(|child| child.node)
                    .collect()
            }
            "items" => children(&nodes, ValueKind::Array)
                .into_iter()
                .map(|child| child.node)
                .collect(),
            "additionalProperties" => children(&nodes, ValueKind::Object)
                .into_iter()
                .map(|child| child.node)
                .collect(),
            _ => return Vec::new(),
        };
    }
    if found.keys {
        let matching = keys_matching(&found.expression);
        children(&nodes, ValueKind::Object)
            .into_iter()
            .filter(|child| matching(child))
            .filter_map(|child| child.key_range())
            .collect()
    } else {
        // An enum's values are its strings and nulls.
        let null = found.is_enum() && found.values.contains(&Value::Null);
        nodes
            .iter()
            .filter(|node| {
                node.kind() == ValueKind::String || (null && node.kind() == ValueKind::Null)
            })
            .map(lazy::Node::byte_range)
            .collect()
    }
}

/// Where `found` is in `schema`, the text of the schema made as it is now, when
/// it says `now`: its `pattern` member (an enum's `enum`); for strings made
/// plain, their place;
/// for keys, the key of `patternProperties` that is their expression, or
/// `additionalProperties` when they are any key. As a range of bytes; none when
/// the text is not JSON, or no longer has it.
pub fn place_in_schema(found: &Found, now: &Now, schema: &str) -> Option<Range<usize>> {
    let tree = LazyTree::from_vec(schema.as_bytes().to_vec()).ok()?;
    let member =
        |child: lazy::Child<'_>| Some(child.key_range()?.start..child.node.byte_range().end);
    let mut node = tree.root();
    let mut place = None;
    for word in schema::steps_of(&found.schema_at) {
        let child = node.child_by_key(&word)?;
        node = child.node;
        place = Some(child);
    }
    if !found.keys {
        let keyword = if found.is_enum() {
            values_now(found, now).map(|_| "enum")
        } else {
            expression_now(found, now).map(|_| "pattern")
        };
        return match (keyword, place) {
            (Some(keyword), _) => member(node.child_by_key(keyword)?),
            (None, Some(child)) => member(child),
            (None, None) => Some(node.byte_range()),
        };
    }
    match expression_now(found, now) {
        Some(expression) => node
            .child_by_key("patternProperties")?
            .node
            .child_by_key(expression)?
            .key_range(),
        None => node.child_by_key("additionalProperties")?.key_range(),
    }
}

/// The children of those of `nodes` that are of `kind`.
fn children<'a>(nodes: &[lazy::Node<'a>], kind: ValueKind) -> Vec<lazy::Child<'a>> {
    nodes
        .iter()
        .filter(|node| node.kind() == kind)
        .flat_map(|node| node.children())
        .collect()
}

/// What tells the members whose keys match `expression` (all of them, when it
/// is not an expression this can read).
fn keys_matching(expression: &str) -> impl Fn(&lazy::Child<'_>) -> bool {
    let regex = Regex::new(expression).ok();
    move |child| {
        child.key.is_some_and(|key| {
            regex
                .as_ref()
                .is_none_or(|regex| regex.is_match(&key.to_string()))
        })
    }
}

#[cfg(test)]
mod tests;
