//! jsonquery: tests of what it changed in `Val::map_index` and `Val::map_range`, the two
//! functions every assignment (`=`, `|=`, `+=`, `//=`, `del`, `setpath`) goes through.
//! The results are those of jq 1.8.1. Like the other tests in this folder they need the
//! `serde` feature of this crate and all the features of jaq-std.

pub mod common;

use common::give;
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, Compiler, Ctx, Vars};
use jaq_json::Val;
use serde_json::{from_value, json, Value};

/// Every output of `code` over `input`, or the text of the error that ends it.
fn run(input: &Value, code: &str) -> Vec<Result<Val, String>> {
    let arena = Arena::default();
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let loader = Loader::new(defs);
    let modules = loader.load(&arena, File { path: (), code }).unwrap();
    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());
    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .unwrap();
    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    let input: Val = from_value(input.clone()).unwrap();
    filter
        .id
        .run((ctx, input))
        .map(jaq_core::unwrap_valr)
        .map(|item| match item {
            Ok(val) => Ok(val),
            Err(error) => Err(match error.into_val() {
                Val::TStr(text) => String::from_utf8_lossy(&text).into_owned(),
                other => other.to_string(),
            }),
        })
        .collect()
}

/// The message of the error `code` ends in over `input`.
fn error_of(input: Value, code: &str) -> String {
    match run(&input, code).pop() {
        Some(Err(message)) => message,
        other => panic!("`{code}` over {input} did not end in an error: {other:?}"),
    }
}

/// The keys of the object `code` gives over `input`, in the order they are in.
fn keys_of(input: Value, code: &str) -> Vec<String> {
    match run(&input, code).as_slice() {
        [Ok(Val::Obj(fields))] => fields
            .keys()
            .map(|key| match key {
                Val::TStr(text) => String::from_utf8_lossy(text).into_owned(),
                other => other.to_string(),
            })
            .collect(),
        other => panic!("`{code}` over {input} did not give one object: {other:?}"),
    }
}

// ---------------------------------------------------------------- null is made into what is asked for

#[test]
fn a_key_makes_an_object_out_of_null() {
    give(json!(null), ".a = 1", json!({"a": 1}));
    give(json!(null), ".a.b = 1", json!({"a": {"b": 1}}));
    give(json!(null), ".a.b.c = 1", json!({"a": {"b": {"c": 1}}}));
    give(json!(null), r#".["a b"] = 1"#, json!({"a b": 1}));
    give(json!(null), r#"."a-b".c = 1"#, json!({"a-b": {"c": 1}}));
    give(json!({"x": 1}), ".a.b = 1", json!({"x": 1, "a": {"b": 1}}));
    give(json!({"a": null}), ".a.b = 1", json!({"a": {"b": 1}}));
    give(json!({"a": {}}), ".a.b = 1", json!({"a": {"b": 1}}));
}

#[test]
fn an_index_makes_an_array_out_of_null() {
    give(json!(null), ".[0] = 1", json!([1]));
    give(json!(null), ".[2] = 1", json!([null, null, 1]));
    give(json!(null), ".[0].a = 1", json!([{"a": 1}]));
    give(json!(null), ".[1][1] = 1", json!([null, [null, 1]]));
    give(json!(null), ".a[0] = 1", json!({"a": [1]}));
    give(
        json!(null),
        ".a[2].b = 1",
        json!({"a": [null, null, {"b": 1}]}),
    );
}

#[test]
fn what_the_function_gives_is_put_where_the_place_was_made() {
    give(json!(null), ".a |= 5", json!({"a": 5}));
    give(json!(null), ".a += 1", json!({"a": 1}));
    give(json!(null), ".a //= 3", json!({"a": 3}));
    give(json!({"a": false}), ".a //= 3", json!({"a": 3}));
    give(json!(null), ".a |= null", json!({"a": null}));
    give(json!({}), ".a = null", json!({"a": null}));
    give(json!({}), ".a.b |= (. // 0) + 1", json!({"a": {"b": 1}}));
    give(json!(null), "(.a, .b) = 1", json!({"a": 1, "b": 1}));
    give(
        json!(null),
        "(.a, .b.c) = 1",
        json!({"a": 1, "b": {"c": 1}}),
    );
    give(json!(null), ".a = 1 | .b = 2", json!({"a": 1, "b": 2}));
    give(json!({"b": 2}), ".a = .b", json!({"b": 2, "a": 2}));
    give(
        json!([{}, {"a": null}, {"a": {"b": 0}}]),
        "[.[] | .a.b = 1]",
        json!([{"a": {"b": 1}}, {"a": {"b": 1}}, {"a": {"b": 1}}]),
    );
}

#[test]
fn null_stays_null_where_nothing_is_put() {
    give(json!(null), ".a |= empty", json!(null));
    give(json!(null), ".[0] |= empty", json!(null));
    give(json!(null), ".[1:] |= empty", json!(null));
    // (jq changes nothing here, and gives `{"x": 1}`: the place is made all the same)
    give(json!({"x": 1}), ".a.b |= empty", json!({"x": 1, "a": null}));
}

#[test]
fn a_reduce_that_starts_from_null_builds_what_it_is_given() {
    give(
        json!(null),
        "reduce range(3) as $i (null; .[$i] = $i)",
        json!([0, 1, 2]),
    );
    give(
        json!(null),
        "reduce range(5) as $i (null; .[$i * 2] = $i)",
        json!([0, null, 1, null, 2, null, 3, null, 4]),
    );
    give(
        json!(["a", "b", "a"]),
        "reduce .[] as $x (null; .[$x] += 1)",
        json!({"a": 2, "b": 1}),
    );
}

// ---------------------------------------------------------------- arrays are made longer

#[test]
fn an_index_past_the_end_makes_the_array_longer_with_nulls() {
    give(json!([]), ".[0] = 9", json!([9]));
    give(json!([]), ".[1] = 9", json!([null, 9]));
    give(json!([1, 2, 3]), ".[3] = 9", json!([1, 2, 3, 9]));
    give(
        json!([1, 2, 3]),
        ".[5] = 9",
        json!([1, 2, 3, null, null, 9]),
    );
    give(json!([[1], [2]]), ".[2][0] = 9", json!([[1], [2], [9]]));
    give(json!([1, 2]), ".[2].a = 9", json!([1, 2, {"a": 9}]));
    give(
        json!([1]),
        ".[5] |= . + 1",
        json!([1, null, null, null, null, 1]),
    );
    give(
        json!([1]),
        ".[5] //= 7",
        json!([1, null, null, null, null, 7]),
    );
    give(json!([1]), "(.[0], .[3]) = 9", json!([9, null, null, 9]));
    give(json!([1, 2, 3]), ".[3] = 9 | length", json!(4));
}

#[test]
fn nothing_put_past_the_end_leaves_the_array_as_it_is() {
    give(json!([1, 2, 3]), ".[5] |= empty", json!([1, 2, 3]));
    give(json!([]), ".[0] |= empty", json!([]));
}

#[test]
fn a_negative_index_counts_from_the_end() {
    give(json!([1, 2, 3]), ".[-1] = 9", json!([1, 2, 9]));
    give(json!([1, 2, 3]), ".[-3] = 9", json!([9, 2, 3]));
}

#[test]
fn a_negative_index_before_the_start_is_an_error() {
    let before = "Out of bounds negative array index";
    assert_eq!(error_of(json!([1, 2, 3]), ".[-4] = 9"), before);
    assert_eq!(error_of(json!([]), ".[-1] = 9"), before);
    assert_eq!(error_of(json!(null), ".[-1] = 9"), before);
    // (an optional path leaves the value as it is)
    give(json!([1, 2, 3]), ".[-4]? = 9", json!([1, 2, 3]));
}

#[test]
fn an_index_far_past_the_end_is_an_error_and_not_the_end_of_the_memory() {
    let far = "Array index too large";
    assert_eq!(error_of(json!([]), ".[16777216] = 1"), far);
    assert_eq!(error_of(json!([]), ".[1700000000] = 1"), far);
    assert_eq!(error_of(json!(null), ".[9223372036854775807] = 1"), far);
    assert_eq!(error_of(json!(null), ".[1700000000].a = 1"), far);
    give(json!([]), ".[1000000] = 1 | length", json!(1_000_001));
    // nothing is put: nothing is made
    give(json!([1]), ".[1700000000] |= empty", json!([1]));
    give(json!([1]), ".[1700000000]? = 1", json!([1]));
}

// ---------------------------------------------------------------- slices

#[test]
fn a_slice_of_null_is_the_array_put_there() {
    give(json!(null), ".[1:] = [9]", json!([9]));
    give(json!(null), ".[:2] = [9]", json!([9]));
    give(json!(null), ".[1:3] += [9]", json!([9]));
    give(json!(null), ".[1:] = []", json!([]));
    give(json!({"a": 1}), ".b[1:] |= [9]", json!({"a": 1, "b": [9]}));
    give(
        json!(null),
        r#"setpath([2, {"start": 1, "end": 3}, 3]; 2)"#,
        json!([null, null, [null, null, null, 2]]),
    );
}

#[test]
fn a_slice_of_null_is_given_an_array_or_it_is_an_error() {
    assert!(error_of(json!(null), ".[1:] = 5").contains("array"));
    // (an optional path leaves the null)
    give(json!(null), ".[1:]? = 5", json!(null));
}

#[test]
fn slices_of_arrays_are_as_they_were() {
    give(json!([1, 2, 3]), ".[1:] = [9]", json!([1, 9]));
    give(json!([1, 2, 3]), ".[5:] = [9]", json!([1, 2, 3, 9]));
    give(json!([1, 2, 3]), ".[:0] = [9]", json!([9, 1, 2, 3]));
    give(json!([1, 2, 3]), ".[1:] = []", json!([1]));
    give(json!([1, 2, 3]), ".[1:2] |= map(. * 10)", json!([1, 20, 3]));
}

// ---------------------------------------------------------------- what is refused

#[test]
fn null_is_made_into_something_only_for_a_key_a_number_or_a_slice() {
    // a boolean, null or an array is no index: nothing is made of the null, and the error is about it
    // (jq words it "Cannot index null with boolean"; a key of an object would be another error)
    for index in ["true", "null", "[1]"] {
        let message = error_of(json!(null), &format!(".[{index}] = 1"));
        assert_eq!(
            message, "cannot use null as iterable (array or object)",
            "{index}"
        );
    }
    for query in ["setpath([true]; 1)", ".[true] |= 5"] {
        let message = error_of(json!(null), query);
        assert_eq!(
            message, "cannot use null as iterable (array or object)",
            "{query}"
        );
    }
}

#[test]
fn a_number_is_not_a_key_of_an_object() {
    let a = json!({"a": 1});
    assert!(error_of(a.clone(), ".[0] = 1").contains("cannot index"));
    assert!(error_of(a.clone(), "setpath([1]; 1)").contains("cannot index"));
    // an optional path leaves the object as it is, with no key made
    give(a.clone(), ".[0]? = 1", a);
}

#[test]
fn what_is_neither_an_object_nor_an_array_is_refused() {
    for (input, filter) in [
        (json!("x"), ".a = 1"),
        (json!(5), ".a = 1"),
        (json!(true), ".a = 1"),
        (json!({"a": 1}), ".a.b = 1"),
        (json!([1]), ".a = 1"),
        (json!({"a": [1]}), ".a.b = 1"),
        (json!(null), ".[true] = 1"),
        (json!(null), ".[null] = 1"),
        (json!(null), ".[[1]] = 1"),
        (json!(null), ".[] = 1"),
    ] {
        let outcome = run(&input, filter);
        assert!(
            matches!(outcome.as_slice(), [Err(_)]),
            "`{filter}` over {input} gave {outcome:?}"
        );
    }
}

// ---------------------------------------------------------------- key order

#[test]
fn a_key_taken_out_leaves_the_others_in_order() {
    // (`give` compares objects without looking at the order of their keys, and the objects of
    // `json!` are sorted: the programs make their objects, which are kept in the order given)
    let program = |update: &str| format!(r#"{{"d":1,"b":2,"c":3,"a":4}} | {update}"#);
    let keys = |update: &str| keys_of(json!(null), &program(update));
    assert_eq!(keys(".d |= empty"), ["b", "c", "a"]);
    assert_eq!(keys(".b |= empty"), ["d", "c", "a"]);
    assert_eq!(keys(".c |= empty"), ["d", "b", "a"]);
    assert_eq!(keys(".a |= empty"), ["d", "b", "c"]);
    assert_eq!(keys(".e = 5"), ["d", "b", "c", "a", "e"]);
    assert_eq!(keys(".c = 5"), ["d", "b", "c", "a"]);
    assert_eq!(keys(".z.y = 1 | .a = 2"), ["d", "b", "c", "a", "z"]);
    assert_eq!(keys_of(json!(null), ".z.y = 1 | .a = 2"), ["z", "a"]);
}

#[test]
fn updating_every_value_is_as_it_was() {
    give(json!([1, 2, 3]), ".[] |= . + 1", json!([2, 3, 4]));
    give(
        json!({"a": 1, "b": 2}),
        ".[] |= . + 1",
        json!({"a": 2, "b": 3}),
    );
    give(json!([1, 2, 3]), ".[] |= empty", json!([]));
    give(json!([1, 2, 3]), "map_values(select(. > 1))", json!([2, 3]));
    assert!(error_of(json!(null), ".[] = 1").contains("iterable"));
}
