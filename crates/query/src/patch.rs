//! Patching a JSON document — what the Tools window's Patch JSON does. Two
//! kinds of patch are understood:
//!
//! * **JSON Patch** (RFC 6902): a list of operations — `add`, `remove`,
//!   `replace`, `move`, `copy` and `test` — each naming the place it works on
//!   with a JSON Pointer (RFC 6901). [`apply`] runs them one after the other and
//!   is all or nothing: if one fails the document is left as it was, and the
//!   error says which one. [`crate::diff`] makes patches of this kind.
//! * **JSON Merge Patch** (RFC 7386): a document laid over another — its
//!   members are added or replace the ones with the same name, a `null`
//!   deletes one, and anything that isn't an object replaces what it is laid
//!   over. [`apply_merge`] does it; it can't fail, but it can't say "set this
//!   to null" either.
//!
//! Objects keep the order of their members: a replaced one stays where it
//! was, a new one goes last, and removing one leaves the rest in order.

use serde_json::{Map, Value};

use crate::diff::{equal, summarize};

/// A reference token (one step of a JSON Pointer) with `~` and `/` escaped, as
/// RFC 6901 has it: `a/b` is written `a~1b`, `m~n` `m~0n`.
pub fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn unescape(token: &str) -> Result<String, String> {
    let mut text = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.next() {
                Some('0') => text.push('~'),
                Some('1') => text.push('/'),
                _ => return Err(format!("in \"{token}\", ~ must be followed by 0 or 1")),
            }
        } else {
            text.push(c);
        }
    }
    Ok(text)
}

/// The steps of a JSON Pointer: `""` has none (it is the whole document),
/// `"/a/0"` has `a` and `0`.
pub fn pointer_tokens(pointer: &str) -> Result<Vec<String>, String> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err(format!(
            "\"{pointer}\" is not a JSON Pointer: it is empty or starts with /"
        ));
    };
    rest.split('/').map(unescape).collect()
}

/// Why a patch could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchError {
    /// Which operation, counting from 1; none when the patch as a whole is not
    /// a list of operations.
    pub operation: Option<usize>,
    /// The operation as the patch wrote it, like `replace /a/b`.
    pub what: String,
    pub message: String,
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.operation, self.what.as_str()) {
            (Some(n), "") => write!(f, "operation {n}: {}", self.message),
            (Some(n), what) => write!(f, "operation {n} ({what}): {}", self.message),
            (None, _) => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for PatchError {}

/// Apply the JSON Patch `patch` to `document`. The document is returned
/// patched, or — if any operation fails — not at all.
pub fn apply(mut document: Value, patch: &Value) -> Result<Value, PatchError> {
    let Some(operations) = patch.as_array() else {
        return Err(PatchError {
            operation: None,
            what: String::new(),
            message: "a JSON Patch is an array of operations".to_owned(),
        });
    };
    for (i, operation) in operations.iter().enumerate() {
        run(&mut document, operation).map_err(|(what, message)| PatchError {
            operation: Some(i + 1),
            what,
            message,
        })?;
    }
    Ok(document)
}

/// One operation. The error is the operation as written (as far as it could be
/// read) and what is wrong with it.
fn run(document: &mut Value, operation: &Value) -> Result<(), (String, String)> {
    let bad = |message: &str| (String::new(), message.to_owned());
    let Some(members) = operation.as_object() else {
        return Err(bad(
            "an operation is an object like {\"op\": \"add\", \"path\": \"/a\", \"value\": 1}",
        ));
    };
    let text = |key: &str| match members.get(key) {
        Some(Value::String(text)) => Ok(text.as_str()),
        Some(_) => Err(bad(&format!("\"{key}\" is a string"))),
        None => Err(bad(&format!("it has no \"{key}\""))),
    };
    let name = text("op")?;
    let path = text("path")?;
    let what = format!("{name} {path}");
    let fail = |message: String| (what.clone(), message);

    let value = || {
        members
            .get("value")
            .cloned()
            .ok_or_else(|| fail("it has no \"value\"".to_owned()))
    };
    match name {
        "add" => add(document, path, value()?).map_err(fail),
        "remove" => remove(document, path).map(drop).map_err(fail),
        "replace" => replace(document, path, value()?).map_err(fail),
        "move" => {
            let from = text("from")?;
            if is_inside(path, from) {
                return Err(fail(format!(
                    "{from} cannot be moved into itself ({path} is inside it)"
                )));
            }
            let moved = remove(document, from).map_err(fail)?;
            add(document, path, moved).map_err(fail)
        }
        "copy" => {
            let copied = get(document, text("from")?).map_err(fail)?.clone();
            add(document, path, copied).map_err(fail)
        }
        "test" => {
            let expected = value()?;
            let found = get(document, path).map_err(fail)?;
            if equal(found, &expected) {
                Ok(())
            } else {
                Err(fail(format!(
                    "the value there is {}, not {}",
                    summarize(found),
                    summarize(&expected)
                )))
            }
        }
        other => Err(fail(format!(
            "\"{other}\" is not an operation (add, remove, replace, move, copy and test are)"
        ))),
    }
}

/// Whether `path` names something below `from` (not `from` itself).
fn is_inside(path: &str, from: &str) -> bool {
    path.len() > from.len() && path.starts_with(from) && path.as_bytes()[from.len()] == b'/'
}

/// The place in an array a reference token names: digits only and no leading
/// zero (RFC 6901). `-`, the place after the last element, is not one.
fn array_index(token: &str) -> Option<usize> {
    let plain = !token.is_empty()
        && token.bytes().all(|b| b.is_ascii_digit())
        && (token == "0" || !token.starts_with('0'));
    plain.then(|| token.parse().ok()).flatten()
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn not_an_index(token: &str) -> String {
    format!("\"{token}\" is not a place in an array (digits, without a leading zero)")
}

fn get<'a>(document: &'a Value, pointer: &str) -> Result<&'a Value, String> {
    let mut current = document;
    for token in pointer_tokens(pointer)? {
        current = match current {
            Value::Object(members) => members
                .get(token.as_str())
                .ok_or_else(|| format!("there is no member \"{token}\""))?,
            Value::Array(items) => {
                let index = array_index(&token).ok_or_else(|| not_an_index(&token))?;
                items.get(index).ok_or_else(|| {
                    format!(
                        "there is no element {index} (the array has {})",
                        items.len()
                    )
                })?
            }
            other => return Err(format!("cannot look for \"{token}\" in {}", kind(other))),
        };
    }
    Ok(current)
}

/// The value that holds what the steps `via` lead to.
fn holder<'a>(document: &'a mut Value, via: &[String]) -> Result<&'a mut Value, String> {
    let mut current = document;
    for token in via {
        current = match current {
            Value::Object(members) => members
                .get_mut(token.as_str())
                .ok_or_else(|| format!("there is no member \"{token}\""))?,
            Value::Array(items) => {
                let len = items.len();
                let index = array_index(token).ok_or_else(|| not_an_index(token))?;
                items
                    .get_mut(index)
                    .ok_or_else(|| format!("there is no element {index} (the array has {len})"))?
            }
            other => return Err(format!("cannot look for \"{token}\" in {}", kind(other))),
        };
    }
    Ok(current)
}

fn add(document: &mut Value, pointer: &str, value: Value) -> Result<(), String> {
    let mut via = pointer_tokens(pointer)?;
    let Some(last) = via.pop() else {
        *document = value;
        return Ok(());
    };
    match holder(document, &via)? {
        Value::Object(members) => {
            members.insert(last, value);
            Ok(())
        }
        Value::Array(items) => {
            if last == "-" {
                items.push(value);
                return Ok(());
            }
            let index = array_index(&last).ok_or_else(|| not_an_index(&last))?;
            if index > items.len() {
                return Err(format!(
                    "cannot add at {index}: the array has {} element{}",
                    items.len(),
                    if items.len() == 1 { "" } else { "s" }
                ));
            }
            items.insert(index, value);
            Ok(())
        }
        other => Err(format!("cannot add to {}", kind(other))),
    }
}

fn remove(document: &mut Value, pointer: &str) -> Result<Value, String> {
    let mut via = pointer_tokens(pointer)?;
    let Some(last) = via.pop() else {
        return Err("the whole document cannot be removed".to_owned());
    };
    match holder(document, &via)? {
        Value::Object(members) => members
            .shift_remove(last.as_str())
            .ok_or_else(|| format!("there is no member \"{last}\" to remove")),
        Value::Array(items) => {
            let index = array_index(&last).ok_or_else(|| not_an_index(&last))?;
            if index < items.len() {
                Ok(items.remove(index))
            } else {
                Err(format!(
                    "there is no element {index} to remove (the array has {})",
                    items.len()
                ))
            }
        }
        other => Err(format!("cannot remove from {}", kind(other))),
    }
}

fn replace(document: &mut Value, pointer: &str, value: Value) -> Result<(), String> {
    let mut via = pointer_tokens(pointer)?;
    let Some(last) = via.pop() else {
        *document = value;
        return Ok(());
    };
    match holder(document, &via)? {
        Value::Object(members) => match members.get_mut(last.as_str()) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(format!("there is no member \"{last}\" to replace")),
        },
        Value::Array(items) => {
            let index = array_index(&last).ok_or_else(|| not_an_index(&last))?;
            let len = items.len();
            match items.get_mut(index) {
                Some(slot) => {
                    *slot = value;
                    Ok(())
                }
                None => Err(format!(
                    "there is no element {index} to replace (the array has {len})"
                )),
            }
        }
        other => Err(format!("cannot replace in {}", kind(other))),
    }
}

/// Lay the JSON Merge Patch `patch` over `target` (RFC 7386).
pub fn apply_merge(target: Value, patch: &Value) -> Value {
    let Value::Object(changes) = patch else {
        return patch.clone();
    };
    let mut members = match target {
        Value::Object(members) => members,
        _ => Map::new(),
    };
    for (key, change) in changes {
        if change.is_null() {
            members.shift_remove(key.as_str());
        } else if let Some(slot) = members.get_mut(key.as_str()) {
            let old = std::mem::take(slot);
            *slot = apply_merge(old, change);
        } else {
            members.insert(key.clone(), apply_merge(Value::Null, change));
        }
    }
    Value::Object(members)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use serde_json::json;

    use super::*;
    use crate::diff::diff;

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// `patch` applied to `document` gives `expected` (RFC 6902, appendix A).
    fn works(document: &str, patch: &str, expected: &str) {
        let got = apply(parse(document), &parse(patch))
            .unwrap_or_else(|e| panic!("{patch} on {document}: {e}"));
        assert!(
            equal(&got, &parse(expected)),
            "{patch} on {document}\n  got      {got}\n  expected {expected}"
        );
    }

    fn fails(document: &str, patch: &str) -> PatchError {
        apply(parse(document), &parse(patch))
            .err()
            .unwrap_or_else(|| panic!("{patch} on {document} should fail"))
    }

    #[test]
    fn rfc_6902_appendix_a_examples() {
        // A.1 adding an object member
        works(
            r#"{"foo":"bar"}"#,
            r#"[{"op":"add","path":"/baz","value":"qux"}]"#,
            r#"{"baz":"qux","foo":"bar"}"#,
        );
        // A.2 adding an array element
        works(
            r#"{"foo":["bar","baz"]}"#,
            r#"[{"op":"add","path":"/foo/1","value":"qux"}]"#,
            r#"{"foo":["bar","qux","baz"]}"#,
        );
        // A.3 removing an object member
        works(
            r#"{"baz":"qux","foo":"bar"}"#,
            r#"[{"op":"remove","path":"/baz"}]"#,
            r#"{"foo":"bar"}"#,
        );
        // A.4 removing an array element
        works(
            r#"{"foo":["bar","qux","baz"]}"#,
            r#"[{"op":"remove","path":"/foo/1"}]"#,
            r#"{"foo":["bar","baz"]}"#,
        );
        // A.5 replacing a value
        works(
            r#"{"baz":"qux","foo":"bar"}"#,
            r#"[{"op":"replace","path":"/baz","value":"boo"}]"#,
            r#"{"baz":"boo","foo":"bar"}"#,
        );
        // A.6 moving a value
        works(
            r#"{"foo":{"bar":"baz","waldo":"fred"},"qux":{"corge":"grault"}}"#,
            r#"[{"op":"move","from":"/foo/waldo","path":"/qux/thud"}]"#,
            r#"{"foo":{"bar":"baz"},"qux":{"corge":"grault","thud":"fred"}}"#,
        );
        // A.7 moving an array element
        works(
            r#"{"foo":["all","grass","cows","eat"]}"#,
            r#"[{"op":"move","from":"/foo/1","path":"/foo/3"}]"#,
            r#"{"foo":["all","cows","eat","grass"]}"#,
        );
        // A.8 testing a value: success
        works(
            r#"{"baz":"qux","foo":["a",2,"c"]}"#,
            r#"[{"op":"test","path":"/baz","value":"qux"},{"op":"test","path":"/foo/1","value":2}]"#,
            r#"{"baz":"qux","foo":["a",2,"c"]}"#,
        );
        // A.10 adding a nested member object
        works(
            r#"{"foo":"bar"}"#,
            r#"[{"op":"add","path":"/child","value":{"grandchild":{}}}]"#,
            r#"{"foo":"bar","child":{"grandchild":{}}}"#,
        );
        // A.11 ignoring unrecognized elements
        works(
            r#"{"foo":"bar"}"#,
            r#"[{"op":"add","path":"/baz","value":"qux","xyz":123}]"#,
            r#"{"foo":"bar","baz":"qux"}"#,
        );
        // A.14 ~ escape ordering
        works(
            r#"{"/":9,"~1":10}"#,
            r#"[{"op":"test","path":"/~01","value":10}]"#,
            r#"{"/":9,"~1":10}"#,
        );
        // A.16 adding an array value
        works(
            r#"{"foo":["bar"]}"#,
            r#"[{"op":"add","path":"/foo/-","value":["abc","def"]}]"#,
            r#"{"foo":["bar",["abc","def"]]}"#,
        );
    }

    #[test]
    fn rfc_6902_appendix_a_errors() {
        // A.9 testing a value: error
        let e = fails(
            r#"{"baz":"qux"}"#,
            r#"[{"op":"test","path":"/baz","value":"bar"}]"#,
        );
        assert_eq!(e.operation, Some(1));
        assert!(e.message.contains("\"qux\", not \"bar\""), "{e}");
        // A.12 adding to a nonexistent target
        let e = fails(
            r#"{"foo":"bar"}"#,
            r#"[{"op":"add","path":"/baz/bat","value":"qux"}]"#,
        );
        assert!(e.message.contains("no member \"baz\""), "{e}");
        // A.15 comparing strings and numbers
        fails(
            r#"{"/":9,"~1":10}"#,
            r#"[{"op":"test","path":"/~01","value":"10"}]"#,
        );
    }

    #[test]
    fn a_failure_names_the_operation_and_leaves_nothing_half_done() {
        let document = parse(r#"{"a":1}"#);
        let patch = parse(
            r#"[{"op":"add","path":"/b","value":2},{"op":"replace","path":"/c/d","value":3}]"#,
        );
        let e = apply(document.clone(), &patch).unwrap_err();
        assert_eq!(e.operation, Some(2));
        assert_eq!(e.what, "replace /c/d");
        assert_eq!(
            e.to_string(),
            "operation 2 (replace /c/d): there is no member \"c\""
        );
        // The caller still has its own copy, untouched.
        assert_eq!(document, parse(r#"{"a":1}"#));
    }

    #[test]
    fn a_patch_is_a_list_of_operations() {
        let e = fails(r#"{}"#, r#"{"op":"add","path":"/a","value":1}"#);
        assert_eq!(e.operation, None);
        assert_eq!(e.to_string(), "a JSON Patch is an array of operations");
        works(r#"{"a":1}"#, "[]", r#"{"a":1}"#);
    }

    #[test]
    fn a_badly_written_operation_is_explained() {
        for (patch, wanted) in [
            (r#"[1]"#, "an operation is an object"),
            (r#"[{"path":"/a"}]"#, "no \"op\""),
            (r#"[{"op":"add"}]"#, "no \"path\""),
            (r#"[{"op":"add","path":"/a"}]"#, "no \"value\""),
            (r#"[{"op":"move","path":"/a"}]"#, "no \"from\""),
            (
                r#"[{"op":"add","path":5,"value":1}]"#,
                "\"path\" is a string",
            ),
            (r#"[{"op":"frob","path":"/a"}]"#, "not an operation"),
            (
                r#"[{"op":"add","path":"a","value":1}]"#,
                "not a JSON Pointer",
            ),
            (
                r#"[{"op":"add","path":"/a~2","value":1}]"#,
                "~ must be followed",
            ),
        ] {
            let e = fails(r#"{}"#, patch);
            assert_eq!(e.operation, Some(1), "{patch}");
            assert!(e.message.contains(wanted), "{patch}: {e}");
        }
    }

    #[test]
    fn the_root_can_be_replaced_but_not_removed() {
        works(
            r#"{"a":1}"#,
            r#"[{"op":"replace","path":"","value":[1]}]"#,
            "[1]",
        );
        works(r#"{"a":1}"#, r#"[{"op":"add","path":"","value":5}]"#, "5");
        let e = fails(r#"{"a":1}"#, r#"[{"op":"remove","path":""}]"#);
        assert!(e.message.contains("whole document"), "{e}");
    }

    #[test]
    fn array_places_are_strict() {
        // Past the end, leading zeros, signs, "-" where a place must exist.
        for patch in [
            r#"[{"op":"add","path":"/a/3","value":0}]"#,
            r#"[{"op":"add","path":"/a/01","value":0}]"#,
            r#"[{"op":"add","path":"/a/+1","value":0}]"#,
            r#"[{"op":"remove","path":"/a/2"}]"#,
            r#"[{"op":"remove","path":"/a/-"}]"#,
            r#"[{"op":"replace","path":"/a/-","value":0}]"#,
            r#"[{"op":"replace","path":"/a/2","value":0}]"#,
            r#"[{"op":"test","path":"/a/-","value":0}]"#,
        ] {
            fails(r#"{"a":[1,2]}"#, patch);
        }
        // Right at the end is fine for add.
        works(
            r#"{"a":[1,2]}"#,
            r#"[{"op":"add","path":"/a/2","value":3}]"#,
            r#"{"a":[1,2,3]}"#,
        );
    }

    #[test]
    fn copy_duplicates_and_move_does_not_move_into_itself() {
        works(
            r#"{"a":{"x":1},"b":[]}"#,
            r#"[{"op":"copy","from":"/a","path":"/b/-"}]"#,
            r#"{"a":{"x":1},"b":[{"x":1}]}"#,
        );
        let e = fails(
            r#"{"a":{"x":1}}"#,
            r#"[{"op":"move","from":"/a","path":"/a/y"}]"#,
        );
        assert!(e.message.contains("into itself"), "{e}");
        // A path that merely starts with the same letters is not inside it.
        works(
            r#"{"a":1}"#,
            r#"[{"op":"move","from":"/a","path":"/ab"}]"#,
            r#"{"ab":1}"#,
        );
        // Moving a value onto itself changes nothing.
        works(
            r#"{"a":1}"#,
            r#"[{"op":"move","from":"/a","path":"/a"}]"#,
            r#"{"a":1}"#,
        );
    }

    #[test]
    fn test_compares_numbers_by_value_and_objects_without_order() {
        works(
            r#"{"a":1.0,"o":{"x":1,"y":2}}"#,
            r#"[{"op":"test","path":"/a","value":1},{"op":"test","path":"/o","value":{"y":2,"x":1}}]"#,
            r#"{"a":1.0,"o":{"x":1,"y":2}}"#,
        );
    }

    #[test]
    fn members_keep_their_order() {
        let patched = apply(
            parse(r#"{"a":1,"b":2,"c":3}"#),
            &parse(
                r#"[{"op":"replace","path":"/b","value":20},{"op":"add","path":"/d","value":4},{"op":"remove","path":"/a"}]"#,
            ),
        )
        .unwrap();
        assert_eq!(patched.to_string(), r#"{"b":20,"c":3,"d":4}"#);
    }

    #[test]
    fn numbers_keep_their_digits() {
        let patched = apply(
            parse(r#"{"id":12345678901234567890123}"#),
            &parse(r#"[{"op":"add","path":"/n","value":0.10}]"#),
        )
        .unwrap();
        assert_eq!(
            patched.to_string(),
            r#"{"id":12345678901234567890123,"n":0.10}"#
        );
    }

    #[test]
    fn rfc_7386_appendix_a_examples() {
        for (target, patch, expected) in [
            (r#"{"a":"b"}"#, r#"{"a":"c"}"#, r#"{"a":"c"}"#),
            (r#"{"a":"b"}"#, r#"{"b":"c"}"#, r#"{"a":"b","b":"c"}"#),
            (r#"{"a":"b"}"#, r#"{"a":null}"#, r#"{}"#),
            (r#"{"a":"b","b":"c"}"#, r#"{"a":null}"#, r#"{"b":"c"}"#),
            (r#"{"a":["b"]}"#, r#"{"a":"c"}"#, r#"{"a":"c"}"#),
            (r#"{"a":"c"}"#, r#"{"a":["b"]}"#, r#"{"a":["b"]}"#),
            (
                r#"{"a":{"b":"c"}}"#,
                r#"{"a":{"b":"d","c":null}}"#,
                r#"{"a":{"b":"d"}}"#,
            ),
            (r#"{"a":[{"b":"c"}]}"#, r#"{"a":[1]}"#, r#"{"a":[1]}"#),
            (r#"["a","b"]"#, r#"["c","d"]"#, r#"["c","d"]"#),
            (r#"{"a":"b"}"#, r#"["c"]"#, r#"["c"]"#),
            (r#"{"a":"foo"}"#, r#"null"#, r#"null"#),
            (r#"{"a":"foo"}"#, r#""bar""#, r#""bar""#),
            (r#"{"e":null}"#, r#"{"a":1}"#, r#"{"e":null,"a":1}"#),
            (r#"[1,2]"#, r#"{"a":"b","c":null}"#, r#"{"a":"b"}"#),
            (
                r#"{}"#,
                r#"{"a":{"bb":{"ccc":null}}}"#,
                r#"{"a":{"bb":{}}}"#,
            ),
        ] {
            let got = apply_merge(parse(target), &parse(patch));
            assert!(
                equal(&got, &parse(expected)),
                "{patch} over {target}\n  got      {got}\n  expected {expected}"
            );
        }
    }

    #[test]
    fn a_merge_patch_keeps_the_order_of_what_it_changes() {
        let got = apply_merge(
            parse(r#"{"a":1,"b":2,"c":3}"#),
            &parse(r#"{"b":20,"a":null,"d":4}"#),
        );
        assert_eq!(got.to_string(), r#"{"b":20,"c":3,"d":4}"#);
    }

    #[test]
    fn tokens_and_escapes_agree() {
        assert_eq!(pointer_tokens("").unwrap(), Vec::<String>::new());
        assert_eq!(pointer_tokens("/").unwrap(), [""]);
        assert_eq!(pointer_tokens("/a~1b/m~0n/0").unwrap(), ["a/b", "m~n", "0"]);
        assert!(pointer_tokens("a").is_err());
        assert_eq!(escape("a/b~c"), "a~1b~0c");
        for token in ["", "a", "a/b", "~", "~1", "/~", "~0~1"] {
            assert_eq!(
                pointer_tokens(&format!("/{}", escape(token))).unwrap(),
                [token]
            );
        }
    }

    // Round trips: whatever the difference between two documents, applying the
    // patch that `diff` makes to the first gives the second. The documents are
    // made by a small deterministic generator with few distinct values, so that
    // equal elements, duplicates and reorderings come up all the time.

    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
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
            Value::Array(items) => match rng.below(6) {
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
                4 if items.len() > 1 => {
                    let (a, b) = (
                        rng.below(items.len() as u64) as usize,
                        rng.below(items.len() as u64) as usize,
                    );
                    items.swap(a, b);
                }
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

    fn assert_round_trip(case: usize, before: &Value, after: &Value) {
        let d = diff(before, after, &AtomicBool::new(false)).unwrap();
        let patch = d.patch();
        let patched = apply(before.clone(), &patch).unwrap_or_else(|e| {
            panic!("case {case}: {e}\n  before {before}\n  after  {after}\n  patch  {patch}")
        });
        assert!(
            equal(&patched, after),
            "case {case}\n  before {before}\n  after  {after}\n  patch  {patch}\n  got    {patched}"
        );
        assert_eq!(d.total(), d.operations.len(), "case {case}");
        assert_eq!(d.is_empty(), equal(before, after), "case {case}");
    }

    #[test]
    fn a_patch_made_from_a_difference_turns_the_first_document_into_the_second() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for case in 0..4_000 {
            let before = generate(&mut rng, 3);
            let mut after = before.clone();
            for _ in 0..1 + rng.below(3) {
                mutate(&mut rng, &mut after);
            }
            assert_round_trip(case, &before, &after);
        }
    }

    #[test]
    fn it_works_for_documents_that_have_nothing_in_common() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        for case in 0..2_000 {
            let before = generate(&mut rng, 3);
            let after = generate(&mut rng, 3);
            assert_round_trip(case, &before, &after);
        }
    }

    #[test]
    fn it_works_for_the_awkward_array_cases() {
        for (before, after) in [
            (json!([1, 2, 3]), json!([3, 2, 1])),
            (json!([1, 1, 1]), json!([1, 1])),
            (json!([1, 2, 1, 2]), json!([2, 1, 2, 1])),
            (json!([]), json!([1, 2])),
            (json!([1, 2]), json!([])),
            (json!([[1], [2]]), json!([[2], [1]])),
            (
                json!([{"a": 1}, {"a": 2}]),
                json!([{"a": 2}, {"a": 3}, {"a": 1}]),
            ),
            (json!([1, 2, 3, 4, 5]), json!([5, 1, 2, 3, 4])),
            (json!([1, 2, 3, 4, 5]), json!([2, 3, 4, 5, 6])),
            (json!({"a": [1, 2]}), json!({"a": [2, 1], "b": []})),
            (json!(1), json!([1])),
            (json!([[]]), json!([[], []])),
        ] {
            assert_round_trip(0, &before, &after);
        }
    }

    #[test]
    fn it_works_for_arrays_too_big_to_align() {
        // 4,000 x 4,000 cells is over the limit, so the elements are paired by
        // position; the patch is longer but still right.
        let before: Vec<u32> = (0..4_000).collect();
        let mut after = before.clone();
        after.remove(10);
        after.insert(3_000, 99_999);
        after.reverse();
        assert_round_trip(0, &json!(before), &json!(after));
    }
}
