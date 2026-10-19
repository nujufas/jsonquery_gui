//! Conversion between the document's `serde_json::Value` and jaq's own `Val`.
//!
//! jaq-json's `Val` is jaq's native value representation (see
//! `jaq_json::Val`), distinct from `serde_json::Value`. Converting through it
//! rather than writing a custom `ValT` adapter is exactly what Phase 1 of the
//! roadmap calls for ("jaq embedded directly against owned Values — no
//! adapter needed yet"); the lazy `Arc<Mmap>`-backed adapter sketched in
//! Architecture §4 is Phase 2 work, once the working set stops fitting
//! comfortably in memory.
//!
//! Numbers go through [`jaq_core::ValT::from_num`], which stores the
//! original literal text for anything that isn't a plain machine integer —
//! this is what preserves e.g. a 64-bit snowflake ID through a query
//! untouched instead of rounding it through an f64.

use jaq_core::ValT;
use jaq_json::Val;
use serde_json::Value;

pub fn to_val(v: &Value) -> Val {
    match v {
        Value::Null => Val::Null,
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => Val::from_num(&n.to_string()).unwrap_or(Val::Null),
        Value::String(s) => Val::utf8_str(s.clone().into_bytes()),
        Value::Array(items) => items.iter().map(to_val).collect(),
        Value::Object(map) => Val::obj(
            map.iter()
                .map(|(k, v)| (Val::utf8_str(k.clone().into_bytes()), to_val(v)))
                .collect(),
        ),
    }
}

/// How many arrays and objects a result can be nested in one another: as many as
/// `serde_json` reads, which is also what a document can be, so that whatever walks
/// a value by recursion has met values this deep before. A result nested deeper is
/// cut where it gets too deep: [`TOO_DEEP`] takes the place of what is below (jq
/// does the same at 256 levels).
pub const MAX_DEPTH: usize = 127;

/// What takes the place of a value that is nested too deep, to be shown or printed.
/// jq prints the same words.
pub const TOO_DEEP: &str = "<skipped: too deep>";

/// Whether `v` is nested more than `limit` levels deep: `1` is 0 levels deep, `[1]` is
/// 1, `[[1]]` is 2, and `[[]]` is deeper than 1 and not deeper than 2. It looks no
/// further down than the limit, so it is safe to ask of a value of any depth.
pub(crate) fn nests_deeper_than(v: &Val, limit: usize) -> bool {
    match v {
        Val::Arr(items) => limit == 0 || items.iter().any(|x| nests_deeper_than(x, limit - 1)),
        Val::Obj(entries) => {
            limit == 0 || entries.values().any(|x| nests_deeper_than(x, limit - 1))
        }
        _ => false,
    }
}

/// Convert a query-result `Val` back into a `serde_json::Value` for display,
/// by writing it through jaq's own JSON-compatible `Display` impl and
/// re-parsing it with `arbitrary_precision` so number literals still survive
/// exactly. Standard JSON-in/JSON-out queries always round-trip cleanly here;
/// the fallback only matters for jq extensions (e.g. binary strings) that
/// can't be represented as plain JSON, which don't arise from filters over
/// ordinary JSON input.
///
/// A value nested more than [`MAX_DEPTH`] levels deep is not written as text first
/// (writing and reading are done by recursion, and a program can make a value as
/// deep as it likes, far deeper than the stack takes): it is cut at that depth.
pub fn from_val(v: &Val) -> Value {
    if nests_deeper_than(v, MAX_DEPTH) {
        return cut(v, 0);
    }
    through_text(v)
}

/// `v` as a value, each array or object at `level` or deeper than [`MAX_DEPTH`]
/// replaced by [`TOO_DEEP`]. It nests no deeper than that itself.
fn cut(v: &Val, level: usize) -> Value {
    match v {
        Val::Arr(_) | Val::Obj(_) if level >= MAX_DEPTH => Value::String(TOO_DEEP.to_owned()),
        Val::Arr(items) => Value::Array(items.iter().map(|x| cut(x, level + 1)).collect()),
        Val::Obj(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, x)| (key_text(key), cut(x, level + 1)))
                .collect(),
        ),
        leaf => through_text(leaf),
    }
}

/// An object's key as the name it has in a document: the text of a string, and
/// what a key that is not one is written as.
fn key_text(key: &Val) -> String {
    match key {
        Val::TStr(text) => String::from_utf8_lossy(text).into_owned(),
        other => other.to_string(),
    }
}

/// `v` written as text and read again, or the text itself where it is not JSON.
fn through_text(v: &Val) -> Value {
    let text = v.to_string();
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_common_shapes() {
        let v = json!({"a": [1, 2.5, "x", null, true], "b": {"c": -3}});
        assert_eq!(from_val(&to_val(&v)), v);
    }

    #[test]
    fn preserves_big_integer_literal() {
        let v = json!({"id": 9223372036854775807i64});
        assert_eq!(from_val(&to_val(&v)), v);

        // Bigger than any primitive integer type serde_json exposes directly;
        // arbitrary_precision keeps this as exact literal text end to end.
        let big = json!({"id": serde_json::Number::from_string_unchecked(
            "123456789012345678901234567890".to_string()
        )});
        assert_eq!(from_val(&to_val(&big)), big);
    }

    /// `n` arrays, one in the next, round `core`.
    fn nested(n: usize, core: Val) -> Val {
        (0..n).fold(core, |inner, _| std::iter::once(inner).collect())
    }

    /// `n` objects, one in the next, round `core`.
    fn nested_objects(n: usize, core: Val) -> Val {
        (0..n).fold(core, |inner, _| {
            Val::obj(
                [(Val::utf8_str(b"a".to_vec()), inner)]
                    .into_iter()
                    .collect(),
            )
        })
    }

    /// How many arrays and objects there are one inside the next, going into the first
    /// of each, and what is inside the last. (A value this deep is not walked by
    /// recursion here, nor dropped by it: that is the thing being tested.)
    fn dive(shown: &Value) -> (usize, &Value) {
        let (mut depth, mut level) = (0, shown);
        loop {
            level = match level {
                Value::Array(items) if !items.is_empty() => &items[0],
                Value::Object(entries) if !entries.is_empty() => entries.values().next().unwrap(),
                _ => return (depth, level),
            };
            depth += 1;
        }
    }

    /// `work` on a thread made for running programs: the values these tests make are
    /// deeper than the stack of a test's own thread can drop.
    fn on_a_query_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = std::sync::mpsc::channel();
        crate::spawn_query_thread("test", move || sender.send(work()).unwrap());
        receiver.recv().expect("the thread ends with an answer")
    }

    #[test]
    fn a_result_as_deep_as_a_document_can_be_is_shown_in_full() {
        let shown = from_val(&nested(MAX_DEPTH, Val::from(1isize)));
        assert_eq!(dive(&shown), (MAX_DEPTH, &json!(1)));
        // and it is the document the loader reads
        let text = format!("{}1{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), shown);
        let shown = from_val(&nested_objects(MAX_DEPTH, Val::Null));
        assert_eq!(dive(&shown), (MAX_DEPTH, &json!(null)));
    }

    #[test]
    fn a_result_nested_deeper_is_cut_where_it_gets_too_deep() {
        let results = on_a_query_thread(|| {
            [MAX_DEPTH + 1, 200, 1000, 5000, 100_000].map(|n| {
                (
                    from_val(&nested(n, Val::from(1isize))),
                    from_val(&nested_objects(n, Val::from(1isize))),
                    // the same, with the core a container too
                    from_val(&nested(n, Val::Arr(Default::default()))),
                )
            })
        });
        for (arrays, objects, empty) in &results {
            assert_eq!(dive(arrays), (MAX_DEPTH, &json!(TOO_DEEP)));
            assert_eq!(dive(objects), (MAX_DEPTH, &json!(TOO_DEEP)));
            assert_eq!(dive(empty), (MAX_DEPTH, &json!(TOO_DEEP)));
        }
        // one more level than there is room for is the least that is cut
        let shown = from_val(&nested(MAX_DEPTH + 1, Val::from(1isize)));
        assert_eq!(dive(&shown).1, &json!(TOO_DEEP));
        let shown = from_val(&nested(MAX_DEPTH, Val::Arr(Default::default())));
        assert_eq!(dive(&shown).1, &json!(TOO_DEEP));
    }

    #[test]
    fn what_is_not_deep_stays_when_a_deep_part_is_cut() {
        // (a value is not `Send`: it is made on the thread that shows it)
        let shown = on_a_query_thread(|| {
            let value = [
                Val::from(1isize),
                nested(500, Val::Null),
                Val::utf8_str(b"text".to_vec()),
                nested(2, Val::from(2isize)),
            ]
            .into_iter()
            .collect::<Val>();
            from_val(&value)
        });
        let items = shown.as_array().unwrap();
        assert_eq!(items[0], json!(1));
        assert_eq!(items[2], json!("text"));
        assert_eq!(items[3], json!([[2]]));
        assert_eq!(dive(&items[1]), (MAX_DEPTH - 1, &json!(TOO_DEEP)));
        // a scalar that is no JSON is the text it is written as, in a cut value too
        let shown = on_a_query_thread(|| {
            let value = [Val::from(f64::NAN), nested(300, Val::Null)]
                .into_iter()
                .collect::<Val>();
            from_val(&value)
        });
        assert_eq!(shown[0], json!("NaN"));
        assert_eq!(dive(&shown[1]), (MAX_DEPTH - 1, &json!(TOO_DEEP)));
    }

    #[test]
    fn a_value_is_nested_deeper_than_a_limit_if_it_has_more_levels_than_that() {
        let deeper = |n, limit| nests_deeper_than(&nested(n, Val::Null), limit);
        assert!(!nests_deeper_than(&Val::Null, 0));
        assert!(nests_deeper_than(&nested(1, Val::Null), 0));
        assert!(!deeper(1, 1));
        assert!(deeper(2, 1));
        assert!(!deeper(127, 127));
        assert!(deeper(128, 127));
        assert!(deeper(5000, 127));
        // it is the deepest part that counts, wherever it is
        let value = [Val::Null, nested(3, Val::Null), nested(1, Val::Null)]
            .into_iter()
            .collect::<Val>();
        assert!(!nests_deeper_than(&value, 4));
        assert!(nests_deeper_than(&value, 3));
        assert!(nests_deeper_than(&nested_objects(3, Val::Null), 2));
        assert!(!nests_deeper_than(&nested_objects(3, Val::Null), 3));
    }
}
