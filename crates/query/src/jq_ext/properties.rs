//! Tests that put the filters of this folder against a model rather than against a list:
//! random values, random paths, and what a plain reading of jq's rules says must come out.
//! The list of cases in `jq_ext.rs` says what jq gives for the cases somebody thought of;
//! these look at the rest, and at what must not happen whatever is asked (a loop, a panic).
//! The generator is a fixed sequence, so a failure can be run again, and says which round.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::tests::{gives, run};

/// xorshift64*: a sequence of numbers that is the same every time for the same seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number from 0 up to, not including, `n` (which is not 0).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const NAMES: [&str; 8] = ["a", "b", "c", "d", "x y", "é", "0", ""];

/// Names the generated objects do not have.
const NEW_NAMES: [&str; 3] = ["n1", "n2", "new name"];

fn random_scalar(rng: &mut Rng) -> Value {
    match rng.below(7) {
        0 => Value::Null,
        1 => Value::Bool(rng.chance(50)),
        2 => json!(rng.below(20) as i64 - 5),
        3 => json!(*rng.pick(&["s", "", "abc", "é"])),
        4 => json!(1.5),
        5 => json!(rng.below(1000)),
        _ => json!(-1),
    }
}

fn random_value(rng: &mut Rng, depth: usize) -> Value {
    if depth == 0 || rng.chance(30) {
        return random_scalar(rng);
    }
    if rng.chance(50) {
        Value::Array(
            (0..rng.below(5))
                .map(|_| random_value(rng, depth - 1))
                .collect(),
        )
    } else {
        let mut fields = Map::new();
        for _ in 0..rng.below(5) {
            fields.insert((*rng.pick(&NAMES)).to_owned(), random_value(rng, depth - 1));
        }
        Value::Object(fields)
    }
}

/// The text of a value, keys in their order (`Value`s are equal whatever their keys' order).
fn text(value: &Value) -> String {
    serde_json::to_string(value).unwrap()
}

/// The one output of `filter` over `value`.
#[track_caller]
fn one(value: &Value, filter: &str) -> Value {
    match run(value, filter) {
        Ok(mut outputs) if outputs.len() == 1 => outputs.remove(0),
        other => panic!("`{filter}` over {value} gave {other:?}"),
    }
}

/// `work` on a thread of its own, which has to answer in time: a loop is found out by
/// this and not by the test never ending, and a panic by there being no answer.
fn finishes_within<T: Send + 'static>(
    seconds: u64,
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    crate::spawn_query_thread("test", move || {
        let _ = sender.send(work());
    });
    receiver
        .recv_timeout(Duration::from_secs(seconds))
        .expect("the work ended in time, and without a panic")
}

// ---------------------------------------------------------------- paths

#[derive(Clone, Debug)]
enum Key {
    Name(String),
    Index(i64),
}

impl Key {
    fn json(&self) -> Value {
        match self {
            Key::Name(name) => json!(name),
            Key::Index(at) => json!(at),
        }
    }

    /// As a step of a path in a program: `["a"]`, `[-1]`.
    fn step(&self) -> String {
        match self {
            Key::Name(name) => format!("[{}]", serde_json::to_string(name).unwrap()),
            Key::Index(at) => format!("[{at}]"),
        }
    }
}

/// The keys of a path written as JSON.
fn path_keys(path: &Value) -> Vec<Key> {
    path.as_array()
        .expect("a path is an array")
        .iter()
        .map(|key| match key {
            Value::String(name) => Key::Name(name.clone()),
            other => Key::Index(other.as_i64().expect("an index")),
        })
        .collect()
}

fn path_json(path: &[Key]) -> Value {
    Value::Array(path.iter().map(Key::json).collect())
}

/// The path as a path expression: `.["a"][0]`; `.` for the path to the whole value.
fn path_expr(path: &[Key]) -> String {
    format!(".{}", path.iter().map(Key::step).collect::<String>())
}

/// A path to delete: down keys that are there, and at the end, now and then, a key that is
/// not, with more after it (which deletes nothing). The last item of an array is `-1` in all
/// the paths of a round, or in none: jq looks at a key as it is written, and a place named in
/// two ways (`-1` and `2`) is not found to be the same one, so the second path meets what the
/// first left of it.
fn path_to_delete(rng: &mut Rng, value: &Value, last_is_minus_one: bool) -> Vec<Key> {
    let mut path = Vec::new();
    let mut here = Some(value);
    while path.len() < 4 {
        if !path.is_empty() && rng.chance(35) {
            break;
        }
        match here {
            Some(Value::Array(items)) => {
                let len = items.len();
                let (key, next) = match rng.below(10) {
                    0 => (Key::Index((len + rng.below(3)) as i64), None),
                    _ if len > 0 => {
                        let at = rng.below(len);
                        let written = if last_is_minus_one && at == len - 1 {
                            -1
                        } else {
                            at as i64
                        };
                        (Key::Index(written), Some(&items[at]))
                    }
                    _ => (Key::Index(rng.below(3) as i64), None),
                };
                path.push(key);
                here = next;
            }
            Some(Value::Object(fields)) => {
                let names: Vec<&String> = fields.keys().collect();
                if !names.is_empty() && !rng.chance(10) {
                    let name = names[rng.below(names.len())];
                    path.push(Key::Name(name.clone()));
                    here = fields.get(name);
                } else {
                    path.push(Key::Name("zz".to_owned()));
                    here = None;
                }
            }
            Some(_) => break,
            None => {
                path.push(if rng.chance(50) {
                    Key::Name("q".to_owned())
                } else {
                    Key::Index(rng.below(3) as i64)
                });
            }
        }
    }
    path
}

/// What `paths` leave of `value` when they are all deleted from it as it is (not one after
/// the other), by the plain rule: a path that ends at an item takes it out, whatever else is
/// named inside it; a path through something that is not there takes nothing out.
fn model_delete(value: &Value, paths: &[&[Key]]) -> Value {
    if paths.iter().any(|path| path.is_empty()) {
        return Value::Null;
    }
    match value {
        Value::Array(items) => {
            let len = items.len() as i64;
            let mut whole = vec![false; items.len()];
            let mut inside: BTreeMap<usize, Vec<&[Key]>> = BTreeMap::new();
            for path in paths {
                let Key::Index(at) = &path[0] else { continue };
                let at = if *at < 0 { len + at } else { *at };
                if !(0..len).contains(&at) {
                    continue;
                }
                if path.len() == 1 {
                    whole[at as usize] = true;
                } else {
                    inside.entry(at as usize).or_default().push(&path[1..]);
                }
            }
            Value::Array(
                items
                    .iter()
                    .enumerate()
                    .filter(|(at, _)| !whole[*at])
                    .map(|(at, item)| match inside.get(&at) {
                        Some(rest) => model_delete(item, rest),
                        None => item.clone(),
                    })
                    .collect(),
            )
        }
        Value::Object(fields) => {
            let mut whole = BTreeSet::new();
            let mut inside: BTreeMap<&str, Vec<&[Key]>> = BTreeMap::new();
            for path in paths {
                let Key::Name(name) = &path[0] else { continue };
                if !fields.contains_key(name) {
                    continue;
                }
                if path.len() == 1 {
                    whole.insert(name.as_str());
                } else {
                    inside.entry(name.as_str()).or_default().push(&path[1..]);
                }
            }
            let mut kept = Map::new();
            for (name, item) in fields {
                if whole.contains(name.as_str()) {
                    continue;
                }
                kept.insert(
                    name.clone(),
                    match inside.get(name.as_str()) {
                        Some(rest) => model_delete(item, rest),
                        None => item.clone(),
                    },
                );
            }
            Value::Object(kept)
        }
        other => other.clone(),
    }
}

/// A path to put a value at: down keys that are there, or past the end of an array, or not
/// there yet in an object, or in `null` (which is what is under a key that is not there).
fn path_to_set(rng: &mut Rng, base: &Value) -> Vec<Key> {
    let null = Value::Null;
    let mut path = Vec::new();
    let mut here = base;
    for _ in 0..=rng.below(4) {
        match here {
            Value::Array(items) => {
                let len = items.len();
                let (key, next) = match rng.below(8) {
                    0 => (Key::Index((len + rng.below(4)) as i64), &null),
                    1 if len > 0 => {
                        let at = rng.below(len);
                        (Key::Index(at as i64 - len as i64), &items[at])
                    }
                    _ if len > 0 => {
                        let at = rng.below(len);
                        (Key::Index(at as i64), &items[at])
                    }
                    _ => (Key::Index(rng.below(3) as i64), &null),
                };
                path.push(key);
                here = next;
            }
            Value::Object(fields) => {
                let names: Vec<&String> = fields.keys().collect();
                if !names.is_empty() && !rng.chance(30) {
                    let name = names[rng.below(names.len())];
                    path.push(Key::Name(name.clone()));
                    here = &fields[name];
                } else {
                    path.push(Key::Name((*rng.pick(&NEW_NAMES)).to_owned()));
                    here = &null;
                }
            }
            Value::Null => {
                path.push(if rng.chance(50) {
                    Key::Name((*rng.pick(&NAMES)).to_owned())
                } else {
                    Key::Index(rng.below(4) as i64)
                });
                here = &null;
            }
            // A number, a string: it is what the path ends at, and it is replaced.
            _ => break,
        }
    }
    path
}

/// `base` with `put` at `path`, by the plain rule: an object gets the key (it keeps the
/// place of one it had), an array gets the index (and `null`s up to it, if it was past the
/// end), and `null` is first made the object or the array the step asks for.
fn model_set(base: &Value, path: &[Key], put: &Value) -> Value {
    let Some((key, rest)) = path.split_first() else {
        return put.clone();
    };
    match (base, key) {
        (Value::Object(fields), Key::Name(name)) => {
            let mut fields = fields.clone();
            let inner = fields.get(name).cloned().unwrap_or(Value::Null);
            fields.insert(name.clone(), model_set(&inner, rest, put));
            Value::Object(fields)
        }
        (Value::Null, Key::Name(name)) => {
            let mut fields = Map::new();
            fields.insert(name.clone(), model_set(&Value::Null, rest, put));
            Value::Object(fields)
        }
        (Value::Array(items), Key::Index(at)) => {
            let len = items.len() as i64;
            let at = if *at < 0 { len + at } else { *at };
            assert!(at >= 0, "the generator makes no index before the start");
            let at = at as usize;
            let mut items = items.clone();
            if at >= items.len() {
                items.resize(at + 1, Value::Null);
            }
            let inner = items[at].clone();
            items[at] = model_set(&inner, rest, put);
            Value::Array(items)
        }
        (Value::Null, Key::Index(at)) => {
            assert!(*at >= 0, "the generator makes no index before the start");
            model_set(&Value::Array(Vec::new()), path, put)
        }
        _ => panic!("the generator made a path that cannot be set: {base} at {key:?}"),
    }
}

const ROUNDS: usize = 1500;

// ---------------------------------------------------------------- delpaths and del

#[test]
fn delpaths_agrees_with_a_model_on_random_values_and_paths() {
    let mut rng = Rng::new(1);
    for round in 0..ROUNDS {
        let value = random_value(&mut rng, 3);
        let minus_one = rng.chance(40);
        let paths: Vec<Vec<Key>> = (0..=rng.below(4))
            .map(|_| path_to_delete(&mut rng, &value, minus_one))
            .collect();
        let refs: Vec<&[Key]> = paths.iter().map(Vec::as_slice).collect();
        let program = format!(
            "delpaths({})",
            Value::Array(paths.iter().map(|path| path_json(path)).collect())
        );
        assert_eq!(
            text(&one(&value, &program)),
            text(&model_delete(&value, &refs)),
            "round {round}: `{program}` over {value}"
        );
    }
}

#[test]
fn del_of_path_expressions_agrees_with_the_same_model() {
    let mut rng = Rng::new(2);
    for round in 0..ROUNDS {
        let value = random_value(&mut rng, 3);
        let minus_one = rng.chance(40);
        let paths: Vec<Vec<Key>> = (0..=rng.below(3))
            .map(|_| path_to_delete(&mut rng, &value, minus_one))
            .collect();
        let refs: Vec<&[Key]> = paths.iter().map(Vec::as_slice).collect();
        let exprs: Vec<String> = paths.iter().map(|path| path_expr(path)).collect();
        let program = format!("del({})", exprs.join(", "));
        assert_eq!(
            text(&one(&value, &program)),
            text(&model_delete(&value, &refs)),
            "round {round}: `{program}` over {value}"
        );
    }
}

#[test]
fn the_order_and_repeats_of_the_paths_make_no_difference() {
    let mut rng = Rng::new(3);
    for round in 0..600 {
        let value = random_value(&mut rng, 3);
        let minus_one = rng.chance(40);
        let mut paths: Vec<Value> = (0..=rng.below(5))
            .map(|_| path_json(&path_to_delete(&mut rng, &value, minus_one)))
            .collect();
        let program = |paths: &[Value]| format!("delpaths({})", Value::Array(paths.to_vec()));
        let expected = text(&one(&value, &program(&paths)));
        let mut again = paths.clone();
        again.extend(paths.iter().cloned());
        assert_eq!(
            text(&one(&value, &program(&again))),
            expected,
            "round {round}: repeated"
        );
        paths.reverse();
        assert_eq!(
            text(&one(&value, &program(&paths))),
            expected,
            "round {round}: reversed"
        );
        paths.rotate_left(1);
        assert_eq!(
            text(&one(&value, &program(&paths))),
            expected,
            "round {round}: rotated"
        );
    }
}

#[test]
fn deleting_every_path_leaves_an_empty_container() {
    let mut rng = Rng::new(4);
    for round in 0..400 {
        let value = random_value(&mut rng, 4);
        let expected = match &value {
            Value::Array(_) => json!([]),
            Value::Object(_) => json!({}),
            other => other.clone(),
        };
        assert_eq!(
            one(&value, "delpaths([paths])"),
            expected,
            "round {round}: {value}"
        );
        assert_eq!(
            one(&value, "del(..)"),
            Value::Null,
            "round {round}: {value}"
        );
        assert_eq!(one(&value, "del(.)"), Value::Null, "round {round}: {value}");
    }
}

#[test]
fn deleting_by_a_condition_is_keeping_the_rest() {
    let mut rng = Rng::new(5);
    for round in 0..400 {
        let items: Vec<i64> = (0..rng.below(12)).map(|_| rng.below(10) as i64).collect();
        let array = json!(items);
        let kept: Vec<i64> = items.iter().copied().filter(|n| n % 3 != 0).collect();
        assert_eq!(
            one(&array, "del(.[] | select(. % 3 == 0))"),
            json!(kept),
            "round {round}: {array}"
        );
        let kept: Vec<i64> = items.iter().copied().filter(|n| *n <= 5).collect();
        assert_eq!(
            one(&array, "del(.[] | select(. > 5))"),
            json!(kept),
            "round {round}"
        );
        // the same for the values of an object
        let object: Map<String, Value> = items
            .iter()
            .enumerate()
            .map(|(at, n)| (format!("k{at}"), json!(n)))
            .collect();
        let kept: Map<String, Value> = object
            .iter()
            .filter(|(_, n)| n.as_i64().unwrap() <= 5)
            .map(|(name, n)| (name.clone(), n.clone()))
            .collect();
        assert_eq!(
            text(&one(&Value::Object(object), "del(.[] | select(. > 5))")),
            text(&Value::Object(kept)),
            "round {round}"
        );
    }
}

#[test]
fn deleting_an_item_or_a_slice_cuts_it_out() {
    let mut rng = Rng::new(6);
    for round in 0..600 {
        let items: Vec<i64> = (0..rng.below(8)).map(|_| rng.below(100) as i64).collect();
        let len = items.len() as i64;
        let array = json!(items);

        let at = rng.below(20) as i64 - 10;
        let resolved = if at < 0 { len + at } else { at };
        let mut expected = items.clone();
        if (0..len).contains(&resolved) {
            expected.remove(resolved as usize);
        }
        assert_eq!(
            one(&array, &format!("del(.[{at}])")),
            json!(expected),
            "round {round}"
        );

        // a slice: bounds that are numbers, or left out
        let bound = |rng: &mut Rng| (rng.chance(75)).then(|| rng.below(20) as i64 - 10);
        let (mut from, upto) = (bound(&mut rng), bound(&mut rng));
        if from.is_none() && upto.is_none() {
            from = Some(0);
        }
        let clip = |bound: Option<i64>, default: i64| {
            let mut at = bound.unwrap_or(default);
            if at < 0 {
                at += len;
            }
            at.clamp(0, len)
        };
        let (start, end) = (clip(from, 0), clip(upto, len));
        let mut expected = items.clone();
        if end > start {
            expected.drain(start as usize..end as usize);
        }
        let slice = format!(
            "{}:{}",
            from.map(|n| n.to_string()).unwrap_or_default(),
            upto.map(|n| n.to_string()).unwrap_or_default()
        );
        assert_eq!(
            one(&array, &format!("del(.[{slice}])")),
            json!(expected),
            "round {round}: del(.[{slice}]) over {array}"
        );
    }
}

// ---------------------------------------------------------------- writes

#[test]
fn setpath_and_the_assignments_agree_with_a_model_on_random_values_and_paths() {
    let mut rng = Rng::new(7);
    for round in 0..ROUNDS {
        let base = if rng.chance(25) {
            Value::Null
        } else {
            random_value(&mut rng, 3)
        };
        let path = path_to_set(&mut rng, &base);
        let put = if rng.chance(80) {
            random_scalar(&mut rng)
        } else {
            random_value(&mut rng, 2)
        };
        let expected = text(&model_set(&base, &path, &put));
        let programs = [
            format!("setpath({}; {put})", path_json(&path)),
            format!("{} = {put}", path_expr(&path)),
            format!("{} |= {put}", path_expr(&path)),
        ];
        for program in programs {
            assert_eq!(
                text(&one(&base, &program)),
                expected,
                "round {round}: `{program}` over {base}"
            );
        }
    }
}

#[test]
fn what_was_put_is_found_where_it_was_put_and_gone_when_it_is_deleted() {
    let mut rng = Rng::new(8);
    for round in 0..600 {
        let base = if rng.chance(30) {
            Value::Null
        } else {
            random_value(&mut rng, 3)
        };
        let path = path_to_set(&mut rng, &base);
        if path.is_empty() {
            continue;
        }
        let put = json!({"put": round});
        let path = path_json(&path);
        let set = format!("setpath({path}; {put})");
        assert_eq!(
            one(&base, &format!("{set} | getpath({path})")),
            put,
            "round {round}"
        );
        // deleted again, what is left is what the model says (the next item of an array moves up)
        let keys = path_keys(&path);
        let put_in = model_set(&base, &keys, &put);
        assert_eq!(
            text(&one(&base, &format!("{set} | delpaths([{path}])"))),
            text(&model_delete(&put_in, &[keys.as_slice()])),
            "round {round}"
        );
        // the path is one of the paths of the value (unless it counts from the end)
        if keys
            .iter()
            .all(|key| !matches!(key, Key::Index(at) if *at < 0))
        {
            let listed = one(&base, &format!("{set} | [paths] | index([{path}])"));
            assert!(
                listed.is_number(),
                "round {round}: {path} is not among the paths"
            );
        }
    }
}

#[test]
fn putting_the_same_value_twice_changes_nothing_the_second_time() {
    let mut rng = Rng::new(9);
    for round in 0..400 {
        let base = if rng.chance(25) {
            Value::Null
        } else {
            random_value(&mut rng, 3)
        };
        let path = path_json(&path_to_set(&mut rng, &base));
        let once = format!("setpath({path}; 7)");
        assert_eq!(
            text(&one(&base, &format!("{once} | setpath({path}; 7)"))),
            text(&one(&base, &once)),
            "round {round}"
        );
    }
}

// ---------------------------------------------------------------- from_entries, join, scan, @base64d

#[test]
fn to_entries_and_from_entries_are_inverses() {
    let mut rng = Rng::new(10);
    for round in 0..300 {
        let mut fields = Map::new();
        for _ in 0..rng.below(6) {
            fields.insert((*rng.pick(&NAMES)).to_owned(), random_value(&mut rng, 2));
        }
        let object = Value::Object(fields);
        assert_eq!(
            text(&one(&object, "to_entries | from_entries")),
            text(&object),
            "round {round}"
        );
        assert_eq!(
            text(&one(&object, "with_entries(.)")),
            text(&object),
            "round {round}"
        );
        // the entries in the other spellings give the same object
        for (key, value) in [
            ("key", "value"),
            ("Key", "Value"),
            ("name", "value"),
            ("Name", "Value"),
        ] {
            let program =
                format!("to_entries | map({{{key}: .key, {value}: .value}}) | from_entries");
            assert_eq!(
                text(&one(&object, &program)),
                text(&object),
                "round {round}: {key}"
            );
        }
    }
}

#[test]
fn join_writes_each_item_as_jq_writes_it() {
    let mut rng = Rng::new(11);
    for round in 0..500 {
        let items: Vec<Value> = (0..rng.below(7))
            .map(|_| match rng.below(6) {
                0 => Value::Null,
                1 => Value::Bool(rng.chance(50)),
                2 => json!(rng.below(100)),
                3 => json!(-(rng.below(100) as i64)),
                _ => json!(*rng.pick(&["", "a", "é", "x y", "日本"])),
            })
            .collect();
        let separator = *rng.pick(&[",", "", ", ", "-", "é"]);
        let pieces: Vec<String> = items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect();
        assert_eq!(
            one(&json!(items), &format!("join({})", json!(separator))),
            json!(pieces.join(separator)),
            "round {round}: {items:?} with {separator:?}"
        );
    }
}

#[test]
fn join_is_undone_by_split_for_text_without_the_separator() {
    let mut rng = Rng::new(12);
    for round in 0..300 {
        let items: Vec<String> = (0..1 + rng.below(6))
            .map(|_| (*rng.pick(&["a", "bc", "é", "x y", "日本", "0"])).to_owned())
            .collect();
        assert_eq!(
            one(&json!(items), r#"join(",") | split(",")"#),
            json!(items),
            "round {round}"
        );
    }
}

#[test]
fn scan_gives_the_matches_match_finds_and_test_agrees() {
    let mut rng = Rng::new(13);
    // patterns without a group, then with: (as the text of a jq string)
    let plain = [
        "a", "a+", "[0-9]+", "[a-c]", r"\\w+", "b|c", r"\\d", "x.", "-",
    ];
    let grouped = ["(a)(b)", r"(\\d)(\\w)", "(a|b)c", "(?<n>[0-9])"];
    for round in 0..500 {
        let text_in: String = (0..rng.below(15))
            .map(|_| *rng.pick(&['a', 'a', 'b', 'c', '0', '1', '2', ' ', 'x', 'y', '-']))
            .collect();
        let haystack = json!(text_in);
        let pattern = *rng.pick(&plain);
        let scan = one(&haystack, &format!(r#"[scan("{pattern}")]"#));
        let matched = one(&haystack, &format!(r#"[match("{pattern}"; "g").string]"#));
        assert_eq!(scan, matched, "round {round}: {pattern} in {text_in:?}");
        let found = one(&haystack, &format!(r#"test("{pattern}")"#));
        assert_eq!(
            found,
            json!(!scan.as_array().unwrap().is_empty()),
            "round {round}: {pattern} in {text_in:?}"
        );
        let pattern = *rng.pick(&grouped);
        let groups = one(
            &haystack,
            &format!(r#"[match("{pattern}"; "g") | [.captures[].string]]"#),
        );
        assert_eq!(
            one(&haystack, &format!(r#"[scan("{pattern}")]"#)),
            groups,
            "round {round}: {pattern} in {text_in:?}"
        );
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (at, byte)| {
            bits | (u32::from(*byte) << (16 - 8 * at))
        });
        for at in 0..4 {
            out.push(if at <= chunk.len() {
                char::from(ALPHABET[((bits >> (18 - 6 * at)) & 63) as usize])
            } else {
                '='
            });
        }
    }
    out
}

#[test]
fn base64_text_is_decoded_with_or_without_its_padding() {
    let mut rng = Rng::new(14);
    let pieces = [
        "a", "b", "é", "日", "😀", "\n", "\"", "\\", " ", "0", "/", "~",
    ];
    for round in 0..600 {
        let original: String = (0..rng.below(9)).map(|_| *rng.pick(&pieces)).collect();
        let encoded = base64(original.as_bytes());
        let input = json!(encoded);
        assert_eq!(
            one(&json!(original), "@base64"),
            input,
            "round {round}: encoding"
        );
        assert_eq!(
            one(&input, "@base64d"),
            json!(original),
            "round {round}: {encoded}"
        );
        let unpadded = encoded.trim_end_matches('=');
        assert_eq!(
            one(&json!(unpadded), "@base64d"),
            json!(original),
            "round {round}: {unpadded}"
        );
        assert_eq!(
            one(&json!(original), "@base64 | @base64d"),
            json!(original),
            "round {round}"
        );
        // text after the first `=` is not looked at
        if encoded.ends_with('=') {
            assert_eq!(
                one(&json!(format!("{encoded}!!")), "@base64d"),
                json!(original),
                "round {round}"
            );
        }
    }
}

#[test]
fn random_bytes_decode_to_the_characters_they_are_and_replacements_for_the_rest() {
    let mut rng = Rng::new(15);
    for round in 0..600 {
        let bytes: Vec<u8> = (0..rng.below(20)).map(|_| rng.below(256) as u8).collect();
        let decoded = one(&json!(base64(&bytes).trim_end_matches('=')), "@base64d");
        let decoded = decoded.as_str().expect("text");
        // Valid UTF-8 is read as it is, and the rest leaves a replacement character.
        if let Ok(valid) = std::str::from_utf8(&bytes) {
            assert_eq!(decoded, valid, "round {round}: {bytes:02x?}");
        } else {
            assert!(decoded.contains('\u{fffd}'), "round {round}: {bytes:02x?}");
        }
    }
}

// ---------------------------------------------------------------- whatever is asked: no loop, no panic

/// What a path can be made of when somebody writes whatever: as the text of a program.
const ODD_STEPS: &[&str] = &[
    "0",
    "1",
    "2",
    "5",
    "12",
    "-1",
    "-2",
    "-9",
    "1.5",
    "-0.5",
    "1e18",
    "-1e18",
    "9223372036854775807",
    "-9223372036854775808",
    "null",
    "true",
    "false",
    r#""a""#,
    r#""""#,
    "[]",
    "[1]",
    "{}",
    r#"{"start":1,"end":null}"#,
    r#"{"start":null,"end":2}"#,
    r#"{"start":-2,"end":null}"#,
    r#"{"start":"a"}"#,
    r#"{"start":1.5,"end":3.5}"#,
    r#"{"start":nan,"end":2}"#,
    r#"{"end":nan}"#,
    "nan",
    "infinite",
    "-infinite",
    r#"{"start":-infinite,"end":infinite}"#,
    r#"{"start":9223372036854775807,"end":-9223372036854775808}"#,
];

#[test]
fn delpaths_given_whatever_paths_ends_in_a_value_or_an_error() {
    let mut rng = Rng::new(16);
    for round in 0..300 {
        let value = random_value(&mut rng, 3);
        let paths: Vec<String> = (0..=rng.below(4))
            .map(|_| {
                let steps: Vec<&str> = (0..rng.below(4)).map(|_| *rng.pick(ODD_STEPS)).collect();
                format!("[{}]", steps.join(","))
            })
            .collect();
        let program = format!("delpaths([{}])", paths.join(","));
        let (input, shown) = (value.clone(), program.clone());
        let outcome = finishes_within(30, move || run(&input, &shown));
        // (a value or an error: either is an answer)
        assert!(
            outcome.is_ok() || outcome.is_err(),
            "round {round}: {program} over {value}"
        );
    }
}

#[test]
fn setpath_given_whatever_path_ends_in_a_value_or_an_error() {
    let mut rng = Rng::new(17);
    for round in 0..300 {
        let value = if rng.chance(30) {
            Value::Null
        } else {
            random_value(&mut rng, 3)
        };
        let steps: Vec<&str> = (0..rng.below(5)).map(|_| *rng.pick(ODD_STEPS)).collect();
        let program = format!("setpath([{}]; 1)", steps.join(","));
        let (input, shown) = (value.clone(), program.clone());
        let outcome = finishes_within(30, move || run(&input, &shown));
        assert!(
            outcome.is_ok() || outcome.is_err(),
            "round {round}: {program} over {value}"
        );
    }
}

#[test]
fn a_nan_in_a_path_is_an_error_and_not_a_loop() {
    // (jq stops with a failed assertion; here the group of equal keys of a NaN, which is
    // not equal to itself, was empty, and the loop over the groups did not move on)
    for program in [
        "delpaths([[nan]])",
        "delpaths([[nan],[nan],[1]])",
        "delpaths([[0, nan]])",
        r#"delpaths([[{"start":nan,"end":2}]])"#,
        r#"delpaths([[{"start":0,"end":nan}], [1]])"#,
        "del(.[nan])",
    ] {
        let shown = program.to_owned();
        let outcome = finishes_within(30, move || run(&json!([[1], [2], [3]]), &shown));
        assert!(
            outcome
                .as_ref()
                .is_err_and(|message| message.contains("NaN")),
            "{program} gave {outcome:?}"
        );
    }
    // the other numbers that are not indexes name nothing, and delete nothing
    gives("[1,2,3]", "delpaths([[infinite]])", &["[1,2,3]"]);
    gives("[1,2,3]", "delpaths([[-infinite]])", &["[1,2,3]"]);
    gives(
        "[1,2,3]",
        r#"delpaths([[{"start":-infinite,"end":infinite}]])"#,
        &["[]"],
    );
}

#[test]
fn a_big_document_is_deleted_from_and_written_to_in_a_moment() {
    let items: Vec<Value> = (0..100_000)
        .map(|n| json!({"n": n, "even": n % 2 == 0}))
        .collect();
    let document = json!({"items": items});
    let outcome = finishes_within(120, move || {
        let kept = run(&document, "del(.items[] | select(.even)) | .items | length");
        let renamed = run(&document, ".items |= map(.seen = true) | .items[-1].seen");
        let spread = run(
            &document,
            "del(.items[range(0; 100000; 7)]) | .items | length",
        );
        (kept, renamed, spread)
    });
    assert_eq!(outcome.0, Ok(vec![json!(50_000)]));
    assert_eq!(outcome.1, Ok(vec![json!(true)]));
    assert_eq!(outcome.2, Ok(vec![json!(85_714)]));
}
