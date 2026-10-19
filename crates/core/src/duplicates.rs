//! Keys that an object has more than once.
//!
//! JSON says the names of an object "should be unique" and leaves it at that: a
//! document with `{"a": 1, "a": 2}` is valid, and what a reader makes of it is its own
//! business. A parsed tree (`serde_json::Value`, which the tools of the app work on) has
//! one member to a key: the one that comes last, at the place of the first. The others are
//! gone from it.
//!
//! [`find`] reads the text of a document that has been parsed and lists what that left out,
//! so that a page can say so: which object, which key and how many times.
//!
//! A tool that has to show every member — Diff, which lays two documents out line by line —
//! reads the text with [`keep_repeats`] instead. Each member after the first of a key is given
//! a key of its own ([`repeated_key`]: `regions` the second time is `regions` and [`MARK`] and
//! `2`), so the tree has all of them, in the order of the text, and the first of each key is
//! compared with the first, the second with the second. [`plain_key`] gives the name back, for
//! whoever writes such a tree out or shows a line of it.

use std::borrow::Cow;
use std::collections::HashMap;

/// How many repeated keys [`DuplicateKeys::listed`] holds, at most: a document can
/// repeat a key in every one of a million objects, and a page cannot show those.
pub const MAX_LISTED: usize = 200;

/// What a key that an object has more than once is given after its name, for the second
/// member and those after it, and then the number of the member. U+FDD0 is a noncharacter,
/// one of those that Unicode sets aside for a program's own use: no document has it in a key.
pub const MARK: char = '\u{fdd0}';

/// The key of member number `member` (from 1) of the keys called `key`: the key itself for
/// the first, and for the others the key, [`MARK`] and the number.
pub fn repeated_key(key: &str, member: usize) -> String {
    if member < 2 {
        key.to_owned()
    } else {
        format!("{key}{MARK}{member}")
    }
}

/// The name of `key`, which may be the key of a repeated member ([`repeated_key`]).
pub fn plain_key(key: &str) -> &str {
    match key.rfind(MARK) {
        Some(at) if is_number(&key[at + MARK.len_utf8()..]) => &key[..at],
        _ => key,
    }
}

/// Which member of its name `key` is: 1 for one that is not marked as a repeat.
pub fn member_of(key: &str) -> usize {
    key.rfind(MARK)
        .map(|at| &key[at + MARK.len_utf8()..])
        .filter(|tail| is_number(tail))
        .and_then(|tail| tail.parse().ok())
        .unwrap_or(1)
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// A key that is more than once in an object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repeated {
    /// Where the object is: a JSON Pointer (RFC 6901), empty for the document
    /// itself. In a file of several values (NDJSON) the first step is the number
    /// of the value, as the array they are made into has it.
    pub path: String,
    /// The key, as the document spells it — with the escapes undone, so that
    /// `"a"` and `"\u0061"` are one key.
    pub key: String,
    /// How many members the object has by that name: two or more.
    pub times: usize,
}

/// What [`find`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DuplicateKeys {
    /// The first [`MAX_LISTED`] of them, in the order in which the second member
    /// of each comes in the text.
    pub listed: Vec<Repeated>,
    /// How many keys are repeated, in all: an object that has one key three times
    /// and another twice counts two.
    pub keys: usize,
    /// How many members were left out of the tree: all but the last of each key,
    /// so a key that is there three times is two of these.
    pub dropped: usize,
}

impl DuplicateKeys {
    pub fn is_empty(&self) -> bool {
        self.keys == 0
    }
}

/// A key of the object that is being read, and how often it came.
struct Entry<'a> {
    key: Cow<'a, [u8]>,
    times: usize,
    /// What the reader of the scan made of the key's second member, when it did (for
    /// [`find`]: where it is in [`DuplicateKeys::listed`]).
    note: Option<usize>,
}

/// The keys of one object. Few are looked up one by one, which is quicker than
/// hashing them; an object with many gets an index.
#[derive(Default)]
struct Keys<'a> {
    entries: Vec<Entry<'a>>,
    index: Option<HashMap<Cow<'a, [u8]>, usize>>,
}

/// From this many keys on an object is indexed.
const LINEAR: usize = 16;

impl<'a> Keys<'a> {
    fn find(&self, key: &[u8]) -> Option<usize> {
        match &self.index {
            Some(index) => index.get(key).copied(),
            None => self.entries.iter().position(|e| e.key == key),
        }
    }

    fn add(&mut self, key: Cow<'a, [u8]>) {
        if let Some(index) = &mut self.index {
            index.insert(key.clone(), self.entries.len());
        }
        self.entries.push(Entry {
            key,
            times: 1,
            note: None,
        });
        if self.index.is_none() && self.entries.len() > LINEAR {
            self.index = Some(
                self.entries
                    .iter()
                    .enumerate()
                    .map(|(at, e)| (e.key.clone(), at))
                    .collect(),
            );
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.index = None;
    }
}

enum Frame<'a> {
    Array {
        /// The place of the element that is being read.
        at: usize,
    },
    Object {
        /// The next string is a key (not a value).
        key_next: bool,
        /// The key whose value is being read, as the bytes between its quotes.
        current: (usize, usize),
        keys: Keys<'a>,
    },
}

/// A member whose key its object has had before, as the scan meets it.
struct Again<'s, 'a> {
    bytes: &'a [u8],
    /// What holds it: the top of this is its object.
    stack: &'s [Frame<'a>],
    /// The key, escapes undone, and where it is in the text (between the quotes).
    key: &'s [u8],
    start: usize,
    end: usize,
    /// Which member of the key it is: 2 for the second, and so on.
    times: usize,
    /// What the scan was told about this key at its second member.
    note: Option<usize>,
    /// The number of the top-level value it is in, from 1.
    top: usize,
}

/// Go through the text of a document (or of several, one after the other) that has been
/// read already, and call `again` for every member whose key its object has had before, in
/// the order of the text. What `again` answers for a key's second member comes back as
/// `note` with each member after it. Gives how many values there are at the top level.
///
/// The text is gone through once and its mistakes are not looked for. What is not JSON
/// gives something, or nothing, but never a panic.
fn scan<'a>(bytes: &'a [u8], again: &mut dyn FnMut(&Again<'_, 'a>) -> Option<usize>) -> usize {
    let mut stack: Vec<Frame<'a>> = Vec::new();
    // Key sets of objects that are done, to be used again (an array of a million
    // objects would otherwise make a million of them).
    let mut spare: Vec<Keys<'a>> = Vec::new();
    // How many values there have been at the top level.
    let mut top = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'{' | b'[' => {
                if stack.is_empty() {
                    top += 1;
                }
                stack.push(if bytes[i] == b'{' {
                    Frame::Object {
                        key_next: true,
                        current: (0, 0),
                        keys: spare.pop().unwrap_or_default(),
                    }
                } else {
                    Frame::Array { at: 0 }
                });
                i += 1;
            }
            b'}' | b']' => {
                if let Some(Frame::Object { mut keys, .. }) = stack.pop() {
                    keys.clear();
                    spare.push(keys);
                }
                i += 1;
            }
            b',' => {
                match stack.last_mut() {
                    Some(Frame::Array { at }) => *at += 1,
                    Some(Frame::Object { key_next, .. }) => *key_next = true,
                    None => {}
                }
                i += 1;
            }
            b':' => {
                if let Some(Frame::Object { key_next, .. }) = stack.last_mut() {
                    *key_next = false;
                }
                i += 1;
            }
            b'"' => {
                let start = i + 1;
                let end = string_end(bytes, start);
                i = end + 1;
                let Some(Frame::Object {
                    key_next: key_next @ true,
                    current,
                    keys,
                }) = stack.last_mut()
                else {
                    if stack.is_empty() {
                        top += 1;
                    }
                    continue;
                };
                *key_next = false;
                *current = (start, end);
                let key = key_bytes(bytes, start, end);
                let Some(at) = keys.find(&key) else {
                    keys.add(key);
                    continue;
                };
                let entry = &mut keys.entries[at];
                entry.times += 1;
                let (times, note) = (entry.times, entry.note);
                let answer = again(&Again {
                    bytes,
                    stack: &stack,
                    key: &key,
                    start,
                    end,
                    times,
                    note,
                    top,
                });
                if times == 2 {
                    if let Some(Frame::Object { keys, .. }) = stack.last_mut() {
                        keys.entries[at].note = answer;
                    }
                }
            }
            _ => {
                if stack.is_empty() {
                    top += 1;
                }
                while i < bytes.len()
                    && !matches!(
                        bytes[i],
                        b' ' | b'\t' | b'\n' | b'\r' | b',' | b':' | b']' | b'}'
                    )
                {
                    i += 1;
                }
            }
        }
    }
    top
}

/// The repeated keys of `bytes`, which are the text of a JSON document (or of
/// several, one after the other) that has been read already.
pub fn find(bytes: &[u8]) -> DuplicateKeys {
    let mut found = DuplicateKeys::default();
    // The number of the top-level value that each of the listed ones is in.
    let mut tops: Vec<usize> = Vec::new();
    let top = scan(bytes, &mut |seen| {
        found.dropped += 1;
        if seen.times == 2 {
            found.keys += 1;
            if found.listed.len() < MAX_LISTED {
                found.listed.push(Repeated {
                    path: pointer(seen.bytes, seen.stack),
                    key: String::from_utf8_lossy(seen.key).into_owned(),
                    times: 2,
                });
                tops.push(seen.top);
                return Some(found.listed.len() - 1);
            }
        } else if let Some(listed) = seen.note {
            found.listed[listed].times = seen.times;
        }
        None
    });
    // Several values in a row are one array: the first step of a path is the one
    // that each is in.
    if top > 1 {
        for (repeated, top) in found.listed.iter_mut().zip(tops) {
            repeated.path = format!("/{}{}", top - 1, repeated.path);
        }
    }
    found
}

/// The text of a document with each member after the first of a repeated key given a key of
/// its own ([`repeated_key`]), so that a tree made from it has them all — and what [`find`]
/// says of the original. The text itself if no key is repeated.
///
/// What comes back is JSON that has no key twice: the first member of each key is as it
/// was, the later ones are `name`, [`MARK`] and the number of the member. A document that
/// is not JSON may come out as another that is not: the mistake is to be looked for in the
/// text that went in.
pub fn keep_repeats(bytes: &[u8]) -> (Cow<'_, [u8]>, DuplicateKeys) {
    let found = find(bytes);
    if found.is_empty() {
        return (Cow::Borrowed(bytes), found);
    }
    // The keys to rewrite: where they are, and what they are to be.
    let mut renamed: Vec<(usize, usize, String)> = Vec::new();
    scan(bytes, &mut |seen| {
        let name = String::from_utf8_lossy(seen.key);
        renamed.push((seen.start, seen.end, repeated_key(&name, seen.times)));
        None
    });
    let mut out = Vec::with_capacity(bytes.len() + renamed.len() * 4);
    let mut from = 0;
    for (start, end, name) in renamed {
        out.extend_from_slice(&bytes[from.min(start)..start]);
        // As JSON writes a string, without the quotes round it.
        let quoted = serde_json::to_string(&name).unwrap_or_default();
        out.extend_from_slice(quoted.trim_matches('"').as_bytes());
        from = end;
    }
    out.extend_from_slice(&bytes[from.min(bytes.len())..]);
    (Cow::Owned(out), found)
}

/// Where the string that starts at `start` ends: the place of its closing quote,
/// or the end of the text.
fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return i,
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// The key that the bytes `start..end` of the text spell, escapes undone: as bytes,
/// which are the same for two keys that are the same (they need not be looked at as
/// text until one is shown).
fn key_bytes(bytes: &[u8], start: usize, end: usize) -> Cow<'_, [u8]> {
    let end = end.min(bytes.len());
    let raw = &bytes[start.min(end)..end];
    if !raw.contains(&b'\\') {
        return Cow::Borrowed(raw);
    }
    // With the quotes, as JSON has it, for the one that knows the escapes.
    let quoted = &bytes[start - 1..(end + 1).min(bytes.len())];
    match serde_json::from_slice::<String>(quoted) {
        Ok(text) => Cow::Owned(text.into_bytes()),
        Err(_) => Cow::Borrowed(raw),
    }
}

/// The JSON Pointer to the object that is the top of `stack`.
fn pointer(bytes: &[u8], stack: &[Frame<'_>]) -> String {
    let mut path = String::new();
    let Some((_, parents)) = stack.split_last() else {
        return path;
    };
    for frame in parents {
        path.push('/');
        match frame {
            Frame::Array { at } => path.push_str(&at.to_string()),
            Frame::Object { current, .. } => {
                let key = key_bytes(bytes, current.0, current.1);
                path.push_str(
                    &String::from_utf8_lossy(&key)
                        .replace('~', "~0")
                        .replace('/', "~1"),
                );
            }
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repeated(path: &str, key: &str, times: usize) -> Repeated {
        Repeated {
            path: path.to_owned(),
            key: key.to_owned(),
            times,
        }
    }

    fn of(text: &str) -> DuplicateKeys {
        find(text.as_bytes())
    }

    #[test]
    fn a_document_without_repeats_has_none() {
        for text in [
            "{}",
            "[]",
            "1",
            "\"a\"",
            "null",
            r#"{"a": 1, "b": 2, "c": {"a": 1, "b": [{"a": 1}, {"a": 2}]}}"#,
            // the same key in objects that are next to each other, or one in the other
            r#"[{"a": 1}, {"a": 2}]"#,
            r#"{"a": {"a": {"a": 1}}}"#,
            "",
            "  \n ",
        ] {
            assert_eq!(of(text), DuplicateKeys::default(), "{text}");
            assert!(of(text).is_empty());
        }
    }

    #[test]
    fn a_key_that_comes_twice_is_listed_with_where_it_is() {
        let found = of(r#"{"a": 1, "b": 2, "a": 3}"#);
        assert_eq!(found.listed, [repeated("", "a", 2)]);
        assert_eq!((found.keys, found.dropped), (1, 1));
        assert!(!found.is_empty());
    }

    #[test]
    fn three_times_is_two_dropped_and_one_key() {
        let found = of(r#"{"a": 1, "a": 2, "a": 3}"#);
        assert_eq!(found.listed, [repeated("", "a", 3)]);
        assert_eq!((found.keys, found.dropped), (1, 2));
    }

    #[test]
    fn the_object_is_given_by_its_path() {
        let found = of(r#"{"x": [0, {"y": {"k": 1, "k": 2}}], "z": {"k": 1}}"#);
        assert_eq!(found.listed, [repeated("/x/1/y", "k", 2)]);
    }

    #[test]
    fn the_keys_of_the_issue_are_found() {
        // The left document of nujufas/jsonquery_gui#1: three of its keys are there
        // three times.
        let text = r#"{
  "service": "orders-api",
  "regions": ["eu-west-1", "us-east-1"],
  "owner": {"team": "payments", "email": "payments@example.test"},
  "deprecated_flag": null,
  "regions": ["eu-west-1", "us-east-1"],
  "owner": {"team": "payments", "email": "payments@example.test"},
  "deprecated_flag": null,
  "regions": ["eu-west-1", "us-east-1"],
  "owner": {"team": "payments", "email": "payments@example.test"},
  "deprecated_flag": null
}"#;
        let found = of(text);
        assert_eq!(
            found.listed,
            [
                repeated("", "regions", 3),
                repeated("", "owner", 3),
                repeated("", "deprecated_flag", 3),
            ]
        );
        assert_eq!((found.keys, found.dropped), (3, 6));
    }

    #[test]
    fn a_key_is_the_same_key_whichever_way_it_is_spelt() {
        let found = of(r#"{"a": 1, "\u0061": 2, "\u00e9": 3, "é": 4, "\ud83d\ude00": 5, "😀": 6}"#);
        assert_eq!(
            found.listed,
            [
                repeated("", "a", 2),
                repeated("", "é", 2),
                repeated("", "😀", 2),
            ]
        );
    }

    #[test]
    fn keys_that_only_look_alike_are_not_repeats() {
        assert!(of(r#"{"a": 1, "A": 2, "a ": 3, " a": 4, "": 5, "\"": 6, "\\": 7}"#).is_empty());
        // An empty key is a key.
        assert_eq!(of(r#"{"": 1, "": 2}"#).listed, [repeated("", "", 2)]);
    }

    #[test]
    fn what_is_in_a_string_is_not_structure() {
        let found = of(
            r#"{"a": "x\", \"a\": 1, {", "b": "}]", "c": [",", "\\"], "d": "\"", "a": 2, "e": "\\\\"}"#,
        );
        assert_eq!(found.listed, [repeated("", "a", 2)]);
        assert_eq!(found.dropped, 1);
        // A value that is the same as a key is not one.
        assert!(of(r#"{"a": "a", "b": "a"}"#).is_empty());
    }

    #[test]
    fn the_steps_of_a_path_are_written_as_json_pointer_has_them() {
        let found = of(r#"{"a/b": {"c~d": [{"k": 1, "k": 2}]}}"#);
        assert_eq!(found.listed, [repeated("/a~1b/c~0d/0", "k", 2)]);
        let found = of(r#"{"\u0061": {"k": 1, "k": 2}}"#);
        assert_eq!(found.listed, [repeated("/a", "k", 2)]);
    }

    #[test]
    fn the_places_in_an_array_are_counted() {
        let found = of(r#"[{"k": 1}, {"k": 1, "k": 2}, [{"j": 1, "j": 2}], 3, {"i": 1, "i": 2}]"#);
        assert_eq!(
            found.listed,
            [
                repeated("/1", "k", 2),
                repeated("/2/0", "j", 2),
                repeated("/4", "i", 2),
            ]
        );
    }

    #[test]
    fn several_values_in_a_row_are_one_array() {
        // As `load_text` makes them: the first step is the number of the value.
        let found = of("{\"a\": 1}\n{\"a\": 1, \"a\": 2}\n[{\"b\": 1, \"b\": 1}]\n7\n\"s\"\n");
        assert_eq!(
            found.listed,
            [repeated("/1", "a", 2), repeated("/2/0", "b", 2)]
        );
        // One value is not wrapped.
        assert_eq!(of("{\"a\": 1, \"a\": 2}\n").listed, [repeated("", "a", 2)]);
        // Nor is a scalar before it counted out of turn.
        let found = of("1 2 {\"a\": 1, \"a\": 2}");
        assert_eq!(found.listed, [repeated("/2", "a", 2)]);
    }

    #[test]
    fn an_object_with_many_keys_is_looked_at_as_well_as_one_with_few() {
        for count in [LINEAR - 1, LINEAR, LINEAR + 1, 1_000] {
            let mut text = String::from("{");
            for n in 0..count {
                text.push_str(&format!("\"k{n}\": {n}, "));
            }
            // Every fifth key again, one of them three times.
            for n in (0..count).step_by(5) {
                text.push_str(&format!("\"k{n}\": 0, "));
            }
            text.push_str("\"k0\": 0}");
            let found = of(&text);
            let again = (0..count).step_by(5).count();
            assert_eq!(found.keys, again, "{count} keys");
            assert_eq!(found.dropped, again + 1, "{count} keys");
            assert_eq!(found.listed[0], repeated("", "k0", 3), "{count} keys");
            assert_eq!(found.listed.len(), again.min(MAX_LISTED), "{count} keys");
        }
    }

    #[test]
    fn every_key_of_an_object_with_many_keys_can_be_the_one_that_is_repeated() {
        // An object of any size, whose every key comes again: each of them is found, and
        // each once (the index is made from the sixteenth key on, and the keys that come
        // before it and after it have to be the same to it).
        for count in [LINEAR - 1, LINEAR, LINEAR + 1, LINEAR + 2, 40, 1_000] {
            let mut text = String::from("{");
            for round in 0..2 {
                for n in 0..count {
                    let sep = if round == 1 && n + 1 == count {
                        ""
                    } else {
                        ", "
                    };
                    text.push_str(&format!("\"k{n}\": {round}{sep}"));
                }
            }
            text.push('}');
            let found = of(&text);
            assert_eq!((found.keys, found.dropped), (count, count), "{count} keys");
            assert_eq!(found.listed.len(), count.min(MAX_LISTED), "{count} keys");
            for (n, repeated) in found.listed.iter().enumerate() {
                assert_eq!(repeated.key, format!("k{n}"), "{count} keys");
                assert_eq!(repeated.times, 2, "{count} keys");
            }
        }
    }

    #[test]
    fn an_object_after_one_with_many_keys_starts_with_none() {
        // (The key sets of objects that are done are used again.)
        let big = |tail: &str| {
            let keys: Vec<String> = (0..40).map(|n| format!("\"k{n}\": {n}")).collect();
            format!("{{{}{tail}}}", keys.join(", "))
        };
        let text = format!("[{}, {}, {}]", big(""), big(", \"k3\": 0"), big(""));
        let found = of(&text);
        assert_eq!(found.listed, [repeated("/1", "k3", 2)]);
        assert_eq!((found.keys, found.dropped), (1, 1));
        let nested = format!(
            "{{\"a\": {}, \"b\": {}, \"c\": {}}}",
            big(""),
            big(""),
            big("")
        );
        assert!(of(&nested).is_empty());
    }

    #[test]
    fn the_list_is_cut_where_a_page_could_not_show_it_and_the_counts_are_not() {
        let mut text = String::from("[");
        for n in 0..(MAX_LISTED + 50) {
            text.push_str(&format!("{{\"k{n}\": 1, \"k{n}\": 2, \"k{n}\": 3}},"));
        }
        text.push_str("{\"last\": 1, \"last\": 2}]");
        let found = of(&text);
        assert_eq!(found.listed.len(), MAX_LISTED);
        assert_eq!(found.keys, MAX_LISTED + 51);
        assert_eq!(found.dropped, 2 * (MAX_LISTED + 50) + 1);
        assert_eq!(found.listed[7], repeated("/7", "k7", 3));
        assert_eq!(
            found.listed.last().unwrap().path,
            format!("/{}", MAX_LISTED - 1)
        );
    }

    #[test]
    fn what_is_not_json_does_not_stop_it() {
        // The text of a document that was read has no such mistakes, but this is
        // handed the bytes, and a page may hand it what it likes.
        for text in [
            "{",
            "}",
            "[[[[",
            "]]]]",
            "{\"a",
            "{\"a\\",
            "{\"a\": 1, \"a",
            ":,:,",
            "\"",
            "\\",
            "{\"a\": 1,, \"a\": 2",
            "{\"a\\u12\": 1, \"a\\u12\": 2}",
        ] {
            let _ = of(text);
        }
        // An escape that is not one is the key as it is written, so two of them are one key.
        let found = of(r#"{"a\u12": 1, "a\u12": 2}"#);
        assert_eq!(found.listed, [repeated("", "a\\u12", 2)]);
        // A byte that is not text.
        let found = find(b"{\"a\xff\": 1, \"a\xff\": 2}");
        assert_eq!(found.keys, 1);
        // Every prefix of a document that has repeats.
        let text = r#"{"a": [1, {"b": "x\"y", "b": 2}], "a": null, "c": 3, "c": 4}"#;
        for end in 0..=text.len() {
            let _ = find(&text.as_bytes()[..end]);
        }
    }

    #[test]
    fn a_deep_document_is_no_trouble() {
        let depth = 100_000;
        let text = format!(
            "{}{{\"k\": 1, \"k\": 2}}{}",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        let found = of(&text);
        assert_eq!((found.keys, found.dropped), (1, 1));
        assert_eq!(found.listed[0].path.len(), 2 * depth);
    }

    fn kept(text: &str) -> (String, DuplicateKeys) {
        let (bytes, found) = keep_repeats(text.as_bytes());
        (String::from_utf8(bytes.into_owned()).unwrap(), found)
    }

    #[test]
    fn a_key_is_marked_with_which_member_of_its_name_it_is() {
        assert_eq!(repeated_key("a", 1), "a");
        assert_eq!(repeated_key("a", 2), "a\u{fdd0}2");
        assert_eq!(repeated_key("", 3), "\u{fdd0}3");
        assert_eq!(plain_key("a\u{fdd0}2"), "a");
        assert_eq!(plain_key("a\u{fdd0}12"), "a");
        assert_eq!(plain_key("a"), "a");
        assert_eq!(member_of("a\u{fdd0}12"), 12);
        assert_eq!(member_of("a"), 1);
        // Not a mark: nothing after it, or something else.
        for odd in ["a\u{fdd0}", "a\u{fdd0}x", "a\u{fdd0}2x", "\u{fdd0}\u{fdd0}"] {
            assert_eq!(plain_key(odd), odd, "{odd:?}");
            assert_eq!(member_of(odd), 1, "{odd:?}");
        }
        // The last mark is the one that counts: a key can have a mark in it.
        assert_eq!(plain_key("a\u{fdd0}2\u{fdd0}3"), "a\u{fdd0}2");
    }

    #[test]
    fn text_without_repeats_is_given_back_as_it_is() {
        for text in ["{}", "[1, 2]", r#"{"a": 1, "b": {"a": 2}}"#, "", "  7 "] {
            let (bytes, found) = keep_repeats(text.as_bytes());
            assert!(matches!(bytes, Cow::Borrowed(_)), "{text}");
            assert_eq!(&*bytes, text.as_bytes());
            assert!(found.is_empty());
        }
    }

    #[test]
    fn a_repeated_member_gets_a_key_of_its_own_and_nothing_else_is_touched() {
        let (text, found) = kept(r#"{"a": 1, "b": 2, "a": 3, "a": 4}"#);
        assert_eq!(
            text,
            "{\"a\": 1, \"b\": 2, \"a\u{fdd0}2\": 3, \"a\u{fdd0}3\": 4}"
        );
        assert_eq!((found.keys, found.dropped), (1, 2));
        let tree: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&str> = tree
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["a", "b", "a\u{fdd0}2", "a\u{fdd0}3"],
            "all of them, in order"
        );
        assert_eq!(tree["a\u{fdd0}3"], 4);
        // Where it is does not matter, nor does how its key is spelt.
        let (text, _) =
            kept(r#"{"x": [{"k": 1}, {"k": 1, "k": 2}], "y": {"k": 1, "k": 2, "k": 3}}"#);
        let tree: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(tree["x"][1]["k\u{fdd0}2"], 2);
        assert_eq!(tree["y"]["k\u{fdd0}3"], 3);
        assert_eq!(tree["x"][0].as_object().unwrap().len(), 1);
    }

    #[test]
    fn a_key_that_is_spelt_another_way_is_marked_too_and_is_written_as_json_writes_it() {
        let (text, found) =
            kept(r#"{"é": 1, "\u00e9": 2, "q\"q": 3, "q\u0022q": 4, "": 5, "": 6}"#);
        assert_eq!(found.keys, 3);
        let tree: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&str> = tree
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["é", "é\u{fdd0}2", "q\"q", "q\"q\u{fdd0}2", "", "\u{fdd0}2"]
        );
    }

    #[test]
    fn the_text_that_is_given_back_has_no_key_twice_at_any_depth() {
        // Whatever the made-up documents repeat, the rewritten text reads back to a tree
        // that has every member the text has.
        for seed in 0..300 {
            let mut maker = Maker {
                state: seed,
                expected: Vec::new(),
                dropped: 0,
            };
            let mut text = String::new();
            maker.value("", 0, &mut text);
            let (kept, found) = kept(&text);
            let tree: serde_json::Value = serde_json::from_str(&kept).unwrap();
            assert_eq!(
                members(&tree),
                members_written(&text),
                "seed {seed}: {text}"
            );
            assert_eq!(found.dropped, maker.dropped, "seed {seed}");
            // And the marks come out of every key that has one: the names are the text's.
            let plain = strip_marks(&tree);
            assert_eq!(members(&plain), members_written(&text), "seed {seed}");
        }
    }

    /// `value` with every key as its name (a marked key is one with a mark and a number).
    fn strip_marks(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(key, value)| {
                        (
                            format!("{}#{}", plain_key(key), member_of(key)),
                            strip_marks(value),
                        )
                    })
                    .collect(),
            ),
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(strip_marks).collect())
            }
            other => other.clone(),
        }
    }

    /// A document made up from a number: objects and arrays with keys from a small
    /// set, so that repeats are common, and what is repeated, written down as it is
    /// made.
    struct Maker {
        state: u64,
        /// What there is, in the order of the second member of each: path, key, times.
        expected: Vec<(String, String, usize)>,
        dropped: usize,
    }

    impl Maker {
        fn next(&mut self, below: u64) -> u64 {
            self.state = self
                .state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.state >> 33) % below
        }

        fn spelt(&mut self, key: &str) -> String {
            // Written plainly, or with an escape for a letter, as the same key.
            match (self.next(3), key.chars().next()) {
                (0, Some(first)) if first.is_ascii_alphabetic() => {
                    format!("\\u{:04x}{}", first as u32, &key[1..])
                }
                _ => key.to_owned(),
            }
        }

        fn value(&mut self, path: &str, depth: usize, out: &mut String) {
            match if depth >= 4 { 3 } else { self.next(4) } {
                0 | 1 => self.object(path, depth, out),
                2 => {
                    out.push('[');
                    for at in 0..self.next(4) {
                        if at > 0 {
                            out.push_str(", ");
                        }
                        self.value(&format!("{path}/{at}"), depth + 1, out);
                    }
                    out.push(']');
                }
                _ => match self.next(4) {
                    0 => out.push_str("\"a, \\\"b\\\": {[\""),
                    1 => out.push_str("null"),
                    2 => out.push_str("-1.5e3"),
                    _ => out.push_str("true"),
                },
            }
        }

        fn object(&mut self, path: &str, depth: usize, out: &mut String) {
            const KEYS: [&str; 5] = ["a", "b", "c/d", "e~f", "g"];
            let mut counts = [0usize; 5];
            let mut second: Vec<usize> = Vec::new();
            out.push('{');
            let members = self.next(8);
            for member in 0..members {
                if member > 0 {
                    out.push_str(", ");
                }
                let which = self.next(KEYS.len() as u64) as usize;
                counts[which] += 1;
                if counts[which] == 2 {
                    second.push(which);
                }
                let spelt = self.spelt(KEYS[which]);
                out.push_str(&format!("\"{spelt}\": "));
                let step = KEYS[which].replace('~', "~0").replace('/', "~1");
                self.value(&format!("{path}/{step}"), depth + 1, out);
            }
            out.push('}');
            // (Not in the order of the text: the test sorts both lists.)
            for which in second {
                self.expected
                    .push((path.to_owned(), KEYS[which].to_owned(), counts[which]));
                self.dropped += counts[which] - 1;
            }
        }
    }

    #[test]
    fn the_repeats_of_a_made_up_document_are_the_ones_it_was_made_with() {
        for seed in 0..400 {
            let mut maker = Maker {
                state: seed,
                expected: Vec::new(),
                dropped: 0,
            };
            let mut text = String::new();
            maker.value("", 0, &mut text);
            let found = of(&text);
            // The tree has what is left of it, one member to a key: it has fewer than
            // the text where, and only where, there are repeats.
            let tree: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(
                members(&tree) < members_written(&text),
                !found.is_empty(),
                "seed {seed}: {text}"
            );
            assert_eq!(found.dropped, maker.dropped, "seed {seed}: {text}");
            assert_eq!(found.keys, maker.expected.len(), "seed {seed}: {text}");
            // The same repeats, whatever order the objects were finished in.
            let mut got: Vec<_> = found
                .listed
                .iter()
                .map(|r| (r.path.clone(), r.key.clone(), r.times))
                .collect();
            let mut want = maker.expected.clone();
            got.sort();
            want.sort();
            assert_eq!(got, want, "seed {seed}: {text}");
        }
    }

    /// How many members the objects of a tree have, in all.
    fn members(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Object(map) => map.len() + map.values().map(members).sum::<usize>(),
            serde_json::Value::Array(items) => items.iter().map(members).sum(),
            _ => 0,
        }
    }

    /// How many members the objects of a text have, in all, counted by a reader that
    /// knows nothing of keys: every string that is followed by a colon.
    fn members_written(text: &str) -> usize {
        let bytes = text.as_bytes();
        let (mut count, mut i) = (0, 0);
        while i < bytes.len() {
            if bytes[i] == b'"' {
                let end = string_end(bytes, i + 1);
                if bytes.get(end + 1) == Some(&b':') {
                    count += 1;
                }
                i = end + 1;
            } else {
                i += 1;
            }
        }
        count
    }

    #[test]
    fn the_listed_keys_are_the_ones_the_tree_lost() {
        // What the tree keeps and what was dropped add up to what was written (here,
        // where nothing that was dropped has members of its own).
        let text = r#"{"a": 1, "b": {"x": 1, "x": 2, "x": 3}, "a": 2}"#;
        let tree: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(tree["a"], 2);
        assert_eq!(tree["b"]["x"], 3);
        let found = of(text);
        assert_eq!(found.dropped + members(&tree), members_written(text));
    }
}
