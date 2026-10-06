//! Queries over a document in its file against the same queries over the same
//! document as a value: what a walked stage makes has to be what jaq makes.

use std::sync::atomic::{AtomicBool, Ordering};

use jsonquery_core::engine::QueryEngine;
use jsonquery_core::lazy::{IndexConfig, LazyTree};
use serde_json::{json, Value};

use super::*;
use crate::reformat;
use crate::{run_query, QueryEvent as JaqEvent};

type Outputs = Vec<Result<Value, String>>;

/// An index with a checkpoint nearly everywhere, so that small documents are
/// walked the way big ones are.
fn fine() -> IndexConfig {
    IndexConfig {
        min_children: 2,
        min_bytes: 12,
        cp_children: 2,
        cp_bytes: 16,
    }
}

fn tree_of(text: &str) -> LazyTree {
    LazyTree::from_vec_with(text.as_bytes().to_vec(), fine()).unwrap()
}

/// What jaq makes of `q` on the value in `text`.
fn eager(text: &str, q: &str) -> Outputs {
    let input: Value = serde_json::from_str(text).unwrap();
    let mut out = Vec::new();
    run_query(&input, q, &AtomicBool::new(false), |event| {
        out.push(match event {
            JaqEvent::Item(v) => Ok(v),
            JaqEvent::ItemError(e) => Err(e),
        })
    })
    .unwrap();
    out
}

/// What the lazy tree makes of it with `limits`.
fn lazy_with(text: &str, q: &str, limits: Limits) -> Result<Outputs, QueryError> {
    let tree = tree_of(text);
    let mut out = Vec::new();
    run_with(
        Kind::Jq,
        &tree,
        q,
        limits,
        &AtomicBool::new(false),
        &mut |event| {
            out.push(match event {
                QueryEvent::Item(v) => Ok(v),
                QueryEvent::ItemError(e) => Err(e),
            })
        },
    )?;
    Ok(out)
}

/// Limits under which every list and object is too big to be given to jaq, so
/// that every stage that can be walked is.
fn walk_everything() -> Limits {
    Limits {
        materialize_bytes: 0,
        scalar_bytes: usize::MAX / 4,
        result_bytes: usize::MAX / 4,
        collect_bytes: usize::MAX / 4,
        key_bytes: usize::MAX / 4,
        parallel: Parallel::default(),
    }
}

/// `limits`, with the elements of a list shared out to `threads` threads, `chunk`
/// at a time, after the first `head`: even a list of a dozen is.
fn by_threads(limits: Limits, threads: usize, chunk: usize, head: usize) -> Limits {
    Limits {
        parallel: Parallel {
            threads,
            chunk,
            head,
            // Whatever the size of it: that is what the tests are of.
            head_bytes: usize::MAX,
            min_bytes: 0,
            chunk_bytes: usize::MAX,
        },
        ..limits
    }
}

/// The same, with a value that is more than a few bytes still handed to jaq.
fn walk_the_big_ones(bytes: usize) -> Limits {
    Limits {
        materialize_bytes: bytes,
        ..walk_everything()
    }
}

/// An error is an error; what a walked stage says about a node it cannot show
/// in full is its own to say.
fn shape(outputs: Outputs) -> Outputs {
    outputs
        .into_iter()
        .map(|o| o.map_err(|_| String::new()))
        .collect()
}

const DOCUMENTS: &[&str] = &[
    "null",
    "true",
    "0",
    "-3",
    "1.5",
    "\"text\"",
    "\"héllo\"",
    "[]",
    "{}",
    "[1,2,3,4,5]",
    "[[1,2],[3],[]]",
    "[{\"a\":1},{\"a\":2},{\"b\":3},null,5]",
    "{\"a\":1,\"b\":[1,2,3],\"c\":{\"d\":\"x\",\"e\":null}}",
    "{\"a b\":1,\"é\":2,\"\":3}",
    "{\"users\":[{\"name\":\"Ada\",\"tags\":[\"x\",\"y\"]},{\"name\":\"Alan\",\"tags\":[]},{\"name\":\"Cy\"}],\"n\":3}",
    "[[1,[2,[3,[4]]]],{\"k\":[{\"k\":[]}]}]",
    "[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29]",
    "{\"a\":{\"b\":{\"c\":[1,2,{\"d\":4}]}},\"e\":[]}",
    "{\"b\":1,\"a\":2,\"c\":3}",
    "[{\"id\":1,\"k\":\"a\",\"n\":3},{\"id\":2,\"k\":\"b\",\"n\":1},{\"id\":3,\"k\":\"a\",\"n\":2},{\"id\":4,\"k\":null,\"n\":2.5}]",
    "[3,1,2,\"b\",\"a\",null,true,false,[1],{\"a\":1},[0],1.5,-1]",
    "[{\"k\":\"b\",\"n\":1,\"id\":1},{\"k\":\"a\",\"n\":1,\"id\":2},{\"k\":\"b\",\"n\":0,\"id\":3},{\"k\":\"a\",\"n\":1,\"id\":4},{\"k\":\"c\",\"n\":null,\"id\":5}]",
    "[[3,1],[2],[],[1,1,1],[2],{\"a\":[]}]",
    "{\"items\":[{\"n\":1,\"tags\":[\"x\"]},{\"n\":2,\"tags\":[]}],\"meta\":{\"total\":2,\"ok\":true}}",
];

/// Programs made only of what can be walked.
const WALKED: &[&str] = &[
    ".",
    ".a",
    ".a.b",
    ".a.b.c",
    ".b",
    ".[0]",
    ".[1]",
    ".[-1]",
    ".[-2]",
    ".[100]",
    ".[-100]",
    ".[2:4]",
    ".[:2]",
    ".[-2:]",
    ".[3:1]",
    ".[10:20]",
    ".[-100:2]",
    ".[1:3][0]",
    ".[1:3][1:]",
    ".[1:3][]",
    ".[1:3] | length",
    ".[]",
    ".[]?",
    ".a[]",
    ".a[]?",
    ".b[]?",
    ".[].a",
    ".[]?.a?",
    ".[][0]?",
    ".a?.b?",
    ".a?",
    ".[0]?",
    ".[\"a b\"]",
    ".\"é\"",
    ".a[0]",
    ".b[1:]",
    ".users[1].name",
    ".users[].name",
    ".users[].tags[]?",
    ".users | length",
    ".users | .[0]",
    ".users | first",
    ".users | last",
    "length",
    "keys",
    "keys_unsorted",
    "first",
    "last",
    ".[] | length",
    ".a | keys",
    ".[] | keys",
    "map(.a)",
    "map(.a?)",
    "map(.[0]?)",
    "map(length)",
    "map(.a) | length",
    "[.[]]",
    "[.[]?]",
    "[.[] | .a]",
    "[.[]? | .a?]",
    "[.a, .b]",
    "[.[0], .[-1]]",
    ".a, .b",
    ".[0], .[-1]",
    ".[1:3], .[0]",
    "first(.[])",
    "first(.[]?)",
    "limit(2; .[])",
    "limit(2; .[]?)",
    "limit(100; .[])",
    "first(.[] | .a?)",
    "limit(3; .[] | length)",
    "(.a | .b)",
    "(.a), (.b)",
    ".users | map(.name)",
    ".users | map(.tags | length)",
    ".users | map(.tags?[0]?)",
    "[.users[].name]",
    ".users[0], .users[1].name, .n",
    // What depends on the kind of a value.
    "type",
    ".[] | type",
    ".a | type",
    "map(type)",
    "[.[] | type]",
    ".[0] | type",
    "strings",
    "numbers",
    ".[] | numbers",
    ".[] | strings",
    ".[]? | arrays",
    "[.[] | scalars]",
    "[.[] | nulls]",
    "[.[] | booleans]",
    "[.[] | objects]",
    "[.[] | iterables]",
    "[.[] | values]",
    // Everything below.
    "..",
    "[..]",
    ".. | type",
    ".. | numbers",
    "[.. | strings]",
    ".. | objects",
    ".. | arrays",
    "[.. | scalars] | length",
    "[.. | values] | length",
    ".a | ..",
    "[.a | ..]",
    "first(..)",
    "limit(3; ..)",
    ".[] | ..",
    "[.[]? | ..] | length",
    "recurse",
    "[recurse | numbers]",
    // `select`, with what it is run on, whole.
    "select(true)",
    "select(false)",
    "select(null)",
    "select(.a)",
    "select(.[0])",
    "select(.a, .b)",
    "select(.a?)",
    "select(type == \"array\")",
    "select(length > 1)",
    ".[] | select(type == \"number\")",
    ".[] | select(.a?)",
    ".[] | select(.a? == 1)",
    ".[]? | select(length > 1)",
    ".[] | select(.k? == \"a\") | .id",
    "map(select(type == \"number\"))",
    "map(select(.n? > 1))",
    "first(.[] | select(type == \"string\"))",
    ".. | select(type == \"number\")",
    "[.. | select(type == \"object\")] | length",
    "[.. | select(.a? == 1)]",
    // Expressions of parts.
    "{count: length}",
    "{n: length, first: .[0], last: .[-1]}",
    "{a, b}",
    "{a: .a, b: .b}",
    "{a: .a?}",
    "{\"x y\": length}",
    "{(.k? // \"k\"): length}",
    "{a: 1, b: \"x\", c: null}",
    "{a: (.[] | numbers)}",
    "{a: .[]?, b: .[]?}",
    "{a: .[0]?, b: (.[1:] | length?)}",
    "{keys: keys, type: type}",
    "{}",
    "[]",
    "1",
    "\"s\"",
    "null",
    "true",
    "length - 1",
    "length > 3",
    "length == 0",
    ".[0] == .[-1]",
    ".[-1] - .[0]",
    "-length",
    "length * 2",
    "length + length",
    ".a + .b",
    ".a // \"none\"",
    ".a? // \"none\"",
    ".a and .b",
    ".a or .b",
    "length > 1 and length < 100",
    ".[]? + 1",
    ".[]? + .[]?",
    "(.[0], .[1]) + 1",
    ".[] | {a}",
    ".[] | {a: .a?}",
    ".[]? | {n: length}",
    "map({n: length})",
    "map({a: .a?})",
    "[.[] | length > 1]",
    ".users | map({name, n: (.tags? | length?)})",
    ".users[] | {name}",
    ".items[] | {n, t: (.tags | length)}",
    ".users | map(select(.tags? | length > 0)) | length",
    "[.[]? | {id, k}]",
    "{total: .meta.total?, n: (.items | length?)}",
    // What needs a key of every element: put in order, in groups, in a few.
    "sort",
    "sort_by(.)",
    "sort_by(.a?)",
    "sort_by(.n?)",
    "sort_by(.k?)",
    "sort_by(-.n?)",
    "sort_by(.k?, .n?)",
    "sort_by([.k?, .n?])",
    "sort_by(length)",
    "sort_by(type)",
    "sort_by(.n?) | reverse",
    "sort_by(.n?) | .[0]",
    "sort_by(.n?) | .[-1]",
    "sort_by(.n?) | .[1:3]",
    "sort_by(.n?) | .[1:3] | map(.id?)",
    "sort_by(.n?) | map(.id?)",
    "sort_by(.n?) | length",
    "sort_by(.n?)[0]",
    "sort_by(.n?) | first",
    "sort_by(.n?) | last",
    "sort_by(.n?) | .[]",
    "sort_by(.n?) | limit(2; .[])",
    "sort_by(.n?) | .[1:] | .[1:]",
    "sort_by(.n?) | sort_by(.id?)",
    "sort_by(.n?) | keys",
    "sort_by(.n?) | map(select(.k? == \"a\")) | length",
    "sort_by(.id?) | reverse | .[0:2]",
    "group_by(.k?)",
    "group_by(.k?) | length",
    "group_by(.k?) | map(length)",
    "group_by(.k?) | map(.[0].id?)",
    "group_by(.k?) | map({k: .[0].k?, n: length})",
    "group_by(.k?) | map({k: .[0].k?, ids: map(.id?)})",
    "group_by(.n?) | .[0]",
    "group_by(.n?) | .[-1] | length",
    "group_by(.k?)[0]",
    "group_by(.k?) | .[] | length",
    "group_by(.k?) | .[1:]",
    "group_by(.k?) | reverse | map(length)",
    "group_by(.k?) | first | length",
    "group_by(type)",
    "group_by(type) | map({type: (.[0] | type), n: length})",
    "group_by(.)",
    "group_by(length)",
    "group_by(.k?) | sort_by(length)",
    "group_by(.k?) | sort_by(length) | reverse | .[0] | length",
    "group_by(.k?) | min_by(length) | length",
    "group_by(.k?) | max_by(length) | map(.id?)",
    "group_by(.k?) | unique_by(length) | length",
    "unique",
    "unique_by(.k?)",
    "unique_by(.k?) | length",
    "unique_by(.n?) | map(.id?)",
    "unique_by(type)",
    "unique_by(.k?) | .[0]",
    "unique_by(length) | length",
    "min",
    "max",
    "min_by(.n?)",
    "max_by(.n?)",
    "min_by(.k?)",
    "max_by(.k?)",
    "min_by(.n?) | .id?",
    "max_by(.n?) | .id?",
    "min_by(.id?, .n?)",
    "max_by(.n?, .id?)",
    "min_by(length)",
    "max_by(length)",
    "min_by(.)",
    "reverse",
    "reverse | .[0]",
    "reverse | .[1:3]",
    "reverse | reverse",
    "reverse | map(type)",
    "reverse | length",
    "sort | reverse | .[0]",
    "sort_by(.n?) | .[] | .id?",
    "sort_by(.n?) | .[0:2][]",
    "[sort_by(.n?)[] | .id?]",
    "{sorted: (sort_by(.n?) | map(.id?)), n: length}",
    "{first: (sort_by(.id?) | first), last: (sort_by(.id?) | last)}",
    ".[] | select(type == \"object\") | .k?",
    "..|numbers",
];

#[test]
fn what_is_walked_is_what_jaq_makes_of_the_value() {
    for text in DOCUMENTS {
        for q in WALKED {
            let expected = shape(eager(text, q));
            let got = lazy_with(text, q, walk_everything()).unwrap();
            // Every program in the list is walked all the way: none is left to
            // jaq to be given a node that is too big.
            assert!(
                !got.iter()
                    .any(|o| matches!(o, Err(e) if e.contains("needs all of"))),
                "{q}  on  {text}  is not all walked"
            );
            assert_eq!(shape(got), expected, "{q}  on  {text}");
        }
    }
}

#[test]
fn what_is_given_to_jaq_whole_is_what_jaq_makes_of_it() {
    // The same, where nothing is walked: the error texts are jaq's too.
    let everything = Limits {
        materialize_bytes: usize::MAX / 4,
        ..walk_everything()
    };
    for text in DOCUMENTS {
        for q in WALKED {
            assert_eq!(
                lazy_with(text, q, everything).unwrap(),
                eager(text, q),
                "{q}  on  {text}"
            );
        }
    }
}

/// Programs that are walked as far as they can be and finished by jaq.
const MIXED: &[&str] = &[
    ".[] | select(. != null)",
    ".users[] | select(.name | startswith(\"A\")) | .name",
    ".users | map(select(.tags | length > 0)) | length",
    ".users | map(.name) | join(\",\")",
    ".users | map(.tags | length) | add",
    "[.users[] | .name | ascii_upcase]",
    "[.users[] | {n: .name, t: (.tags // [] | length)}]",
    ".users[] | {name, n: (.tags | length)}",
    ".users[] | .name | length",
    ".users[1:] | map(.name)",
    ".users | map(.name) | sort | reverse",
    "map(.a?) | map(select(. != null))",
    "[.[] | numbers] | add",
    "[.[] | numbers] | length",
    ".[] | numbers | . * 2",
    "first(.[] | select(. > 3))",
    "limit(3; .[] | select(. % 2 == 0))",
    ".[:5] | add",
    ".[:5] | map(. * 2)",
    "[.[] | select(. > 10)] | length",
    ".users | length | . + 1",
    ".users[0].tags | map(ascii_upcase)",
    ".a | to_entries",
    ".users | map(.name) | .[1:]",
    "[.users[].name] | length",
    "[limit(2; .users[])] | map(.name)",
    ".users[] | .tags? | length?",
    ".users[]? | .tags[]?",
    ".. | select(type == \"number\") | . + 1",
    "[.. | numbers] | add",
    "[.. | strings] | join(\",\")",
    "[.. | objects | keys[]] | unique",
    "[.. | objects | .a?] | map(select(. != null))",
    "map(select(length > 1)) | length",
    ".[] | {a} | .a",
    "map({n: length}) | map(.n) | add",
    "{n: length} | .n",
    "[.[]? | select(type == \"object\") | {k: (.k? // \"-\")}]",
    ".users | {names: map(.name), n: length} | .names | length",
    "{a: (.users | length), b: (.users | map(.name) | join(\"+\"))}",
    "length - 1 | . * 2",
];

#[test]
fn what_is_walked_and_then_given_to_jaq_is_what_jaq_makes_of_the_value() {
    for text in DOCUMENTS {
        for q in MIXED {
            let expected = shape(eager(text, q));
            for bytes in [0, 8, 30, 60, 120] {
                let got = shape(lazy_with(text, q, walk_the_big_ones(bytes)).unwrap_or_default());
                // A program that needs a whole list that is too big for jaq to be
                // given says so, which is not a thing to compare.
                let refused = lazy_with(text, q, walk_the_big_ones(bytes))
                    .unwrap()
                    .iter()
                    .any(|o| matches!(o, Err(e) if e.contains("needs all of")));
                if refused {
                    continue;
                }
                assert_eq!(got, expected, "{q}  on  {text}  (limit {bytes})");
            }
        }
    }
}

#[test]
fn a_list_too_big_for_a_stage_that_needs_all_of_it_is_an_error_that_says_so() {
    let text = "[1,2,3,4,5,6,7,8,9,10]";
    let out = lazy_with(text, "to_entries", walk_the_big_ones(8)).unwrap();
    assert_eq!(out.len(), 1);
    let message = out[0].as_ref().unwrap_err();
    assert!(
        message.contains("`to_entries` needs all of an array of 10 items"),
        "{message}"
    );
    assert!(message.contains(".[] | …"), "{message}");

    // After a stage that was walked, what it needs is that stage's output.
    let out = lazy_with(text, ".[2:] | add", walk_the_big_ones(8)).unwrap();
    assert!(out[0].as_ref().unwrap_err().contains("`add` needs all of"));
    // And where it fits, it is run.
    assert_eq!(
        lazy_with(text, ".[2:4] | add", walk_the_big_ones(30)).unwrap(),
        vec![Ok(json!(7))]
    );
}

#[test]
fn a_result_that_is_too_big_is_an_error_not_a_value() {
    let text = "{\"big\":[1,2,3,4,5,6,7,8,9,10],\"small\":[1]}";
    let limits = Limits {
        result_bytes: 12,
        ..walk_everything()
    };
    let out = lazy_with(text, ".big", limits).unwrap();
    assert_eq!(out.len(), 1);
    let message = out[0].as_ref().unwrap_err();
    assert!(message.contains("an array of 10 items"), "{message}");
    assert!(message.contains("too big to show"), "{message}");
    assert_eq!(
        lazy_with(text, ".small", limits).unwrap(),
        vec![Ok(json!([1]))]
    );
    // A part of it that is small enough is shown.
    assert_eq!(
        lazy_with(text, ".big[:2]", limits).unwrap(),
        vec![Ok(json!([1, 2]))]
    );
}

#[test]
fn what_is_gathered_is_held_to_a_limit() {
    let text = format!(
        "[{}]",
        (0..200)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let limits = Limits {
        collect_bytes: 700,
        ..walk_everything()
    };
    let out = lazy_with(&text, "[.[]]", limits).unwrap();
    let message = out[0].as_ref().unwrap_err();
    assert!(message.contains("too much to hold in memory"), "{message}");
    let out = lazy_with(&text, "[.[:5][]]", limits).unwrap();
    assert_eq!(out, vec![Ok(json!([0, 1, 2, 3, 4]))]);
}

#[test]
fn a_collection_stops_at_the_first_error_as_it_does_in_jaq() {
    let text = "[{\"a\":1},2,{\"a\":3}]";
    for q in ["[.[] | .a]", "map(.a)", "[.[] | .a] | length"] {
        let expected = shape(eager(text, q));
        assert_eq!(expected.len(), 1, "{q}");
        assert_eq!(
            shape(lazy_with(text, q, walk_everything()).unwrap()),
            expected,
            "{q}"
        );
    }
    // And a stream goes on past one.
    let expected = shape(eager(text, ".[] | .a"));
    assert_eq!(expected.len(), 3);
    assert_eq!(
        shape(lazy_with(text, ".[] | .a", walk_everything()).unwrap()),
        expected
    );
}

#[test]
fn a_stream_that_is_cut_short_is_not_read_to_its_end() {
    // A list of a million, of which `first` and `limit` look at a few: this
    // would take a very long time, or not finish, if it read the rest.
    let mut text = String::from("[");
    for i in 0..300_000 {
        if i > 0 {
            text.push(',');
        }
        text.push_str(&i.to_string());
    }
    text.push(']');
    let started = std::time::Instant::now();
    let out = lazy_with(&text, "first(.[])", Limits::default()).unwrap();
    assert_eq!(out, vec![Ok(json!(0))]);
    let out = lazy_with(&text, "limit(3; .[] | select(. > 100))", Limits::default()).unwrap();
    assert_eq!(out, vec![Ok(json!(101)), Ok(json!(102)), Ok(json!(103))]);
    let out = lazy_with(&text, ".[150000]", Limits::default()).unwrap();
    assert_eq!(out, vec![Ok(json!(150000))]);
    let out = lazy_with(&text, "length", Limits::default()).unwrap();
    assert_eq!(out, vec![Ok(json!(300_000))]);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_query_that_is_stopped_stops() {
    let text = format!(
        "[{}]",
        (0..5000)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let tree = tree_of(&text);
    let cancelled = AtomicBool::new(false);
    let mut seen = 0usize;
    run_with(
        Kind::Jq,
        &tree,
        ".[] | . * 2",
        walk_everything(),
        &cancelled,
        &mut |_| {
            seen += 1;
            if seen == 10 {
                cancelled.store(true, Ordering::Relaxed);
            }
        },
    )
    .unwrap();
    assert_eq!(seen, 10);
}

#[test]
fn a_program_that_is_not_jq_is_refused_in_jaqs_words() {
    let tree = tree_of("[1]");
    let mut ignore = |_: QueryEvent| {};
    let lazy = run(Kind::Jq, &tree, ".[", &AtomicBool::new(false), &mut ignore).unwrap_err();
    let value = crate::JaqEngine
        .run(&json!([1]), ".[", &AtomicBool::new(false), &mut |_| {})
        .unwrap_err();
    assert_eq!(lazy.to_string(), value.to_string());
    assert!(matches!(lazy, QueryError::Parse(_)));
}

#[test]
fn json_pointer_walks_to_its_place() {
    let text = "{\"a\":{\"b\":[10,20,{\"c/d\":1,\"e~f\":2}]},\"01\":3}";
    let tree = tree_of(text);
    let input: Value = serde_json::from_str(text).unwrap();
    for pointer in [
        "",
        "/a",
        "/a/b",
        "/a/b/1",
        "/a/b/2/c~1d",
        "/a/b/2/e~0f",
        "/01",
        "/a/b/01",
        "/a/b/+1",
        "/a/b/3",
        "/a/b/-",
        "/a/x",
        "/a/b/1/z",
        "/",
        "no slash",
    ] {
        let mut lazy = Vec::new();
        let l = run(
            Kind::JsonPointer,
            &tree,
            pointer,
            &AtomicBool::new(false),
            &mut |e| lazy.push(e),
        );
        let mut eager = Vec::new();
        let e = crate::JsonPointerEngine.run(&input, pointer, &AtomicBool::new(false), &mut |ev| {
            eager.push(ev)
        });
        assert_eq!(l.is_ok(), e.is_ok(), "{pointer:?}");
        match (l, e) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{pointer:?}"),
            (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string(), "{pointer:?}"),
            _ => unreachable!(),
        }
        assert_eq!(lazy.len(), eager.len(), "{pointer:?}");
        for (a, b) in lazy.iter().zip(&eager) {
            match (a, b) {
                (QueryEvent::Item(a), QueryEvent::Item(b)) => assert_eq!(a, b, "{pointer:?}"),
                _ => panic!("{pointer:?}"),
            }
        }
    }
}

#[test]
fn json_pointer_to_something_too_big_says_so() {
    let tree = tree_of("{\"a\":[1,2,3,4,5,6,7,8]}");
    let limits = Limits {
        result_bytes: 5,
        ..Limits::default()
    };
    let err = run_with(
        Kind::JsonPointer,
        &tree,
        "/a",
        limits,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(err.to_string().contains("too big to show"), "{err}");
}

/// What the engine of `kind` makes of `q` on the value in `text`.
fn eager_in(kind: Kind, text: &str, q: &str) -> Result<Outputs, String> {
    let input: Value = serde_json::from_str(text).unwrap();
    let mut out = Vec::new();
    kind.engine()
        .run(&input, q, &AtomicBool::new(false), &mut |event| {
            out.push(match event {
                QueryEvent::Item(v) => Ok(v),
                QueryEvent::ItemError(e) => Err(e),
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// What the lazy tree makes of it, run as `kind` with `limits`.
fn lazy_in(kind: Kind, text: &str, q: &str, limits: Limits) -> Result<Outputs, String> {
    let tree = tree_of(text);
    let mut out = Vec::new();
    run_with(
        kind,
        &tree,
        q,
        limits,
        &AtomicBool::new(false),
        &mut |event| {
            out.push(match event {
                QueryEvent::Item(v) => Ok(v),
                QueryEvent::ItemError(e) => Err(e),
            })
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(out)
}

const STORE: &str = r#"{"store":{"book":[
  {"category":"reference","author":"Nigel Rees","title":"Sayings","price":8.95},
  {"category":"fiction","author":"Evelyn Waugh","title":"Sword","price":12.99},
  {"category":"fiction","author":"Herman Melville","title":"Moby","isbn":"0-553","price":8.99},
  {"category":"fiction","author":"Tolkien","title":"Rings","isbn":"0-395","price":22.99}],
  "bicycle":{"color":"red","price":19.95}},"expensive":10}"#;

const JSONPATHS: &[&str] = &[
    "$",
    "$.a",
    "$.a.b",
    "$.a.b.c",
    "$['a']",
    "$['a b']",
    "$.b",
    "$[0]",
    "$[1]",
    "$[-1]",
    "$[-2]",
    "$[100]",
    "$[-100]",
    "$[1:3]",
    "$[:2]",
    "$[-2:]",
    "$[::2]",
    "$[1::2]",
    "$[::-1]",
    "$[3:0:-1]",
    "$[-1:-4:-1]",
    "$[5:2]",
    "$[0,2]",
    "$[0,-1]",
    "$['a','b']",
    "$[*]",
    "$.*",
    "$..*",
    "$..a",
    "$..b",
    "$..[0]",
    "$..[0,1]",
    "$..['a','b']",
    "$[*].a",
    "$[*][0]",
    "$.users",
    "$.users[*].name",
    "$.users[0].tags[*]",
    "$.users[1:].name",
    "$.users[?(@.tags)]",
    "$.users[?(@.name == 'Ada')].tags[*]",
    "$.users[?(@.tags[0] == 'x')].name",
    "$..users[0,1]",
    "$..name",
    "$..tags[*]",
    "$..[?(@.name)]",
    "$[?(@.a)]",
    "$[?(@.a == 2)]",
    "$[?(@ > 3)]",
    "$[?(@ > 3 && @ < 9)]",
    "$[?(@.id)]",
    "$[?(@.k == 'a')].id",
    "$[?(@.n >= 1)].id",
    "$[?(@.n >= 1 || @.k == 'c')].id",
    "$..[?(@.n > 1)]",
    "$.items[?(@.n >= 2)].tags",
    "$.items[*].n",
    "$.items..n",
    "$.meta.*",
    "$.store.book[*].author",
    "$.store.book[?(@.price < 10)].title",
    "$.store.book[?(@.isbn)].title",
    "$.store.book[-1:].title",
    "$.store.book[0,1].price",
    "$.store..price",
    "$..book[2]",
    "$..book[?(@.price < $.expensive)].title",
    "$..price",
    "$.store.*",
    "$..*.price",
    "$.a[",
    "a.b",
    "$..",
    "$.[?(@.x ==)]",
];

#[test]
fn jsonpath_on_a_document_in_its_file_is_what_it_is_on_a_value() {
    let documents: Vec<&str> = DOCUMENTS.iter().copied().chain([STORE]).collect();
    let mut compared = 0;
    for text in documents {
        for q in JSONPATHS {
            let expected = eager_in(Kind::JsonPath, text, q);
            // A filter that reads the root is refused where it would be run on a part of it.
            let reads_the_root = q.contains("[?") && q[1..].contains('$');
            for limits in [
                walk_everything(),
                walk_the_big_ones(8),
                walk_the_big_ones(30),
                walk_the_big_ones(120),
                by_threads(walk_the_big_ones(8), 3, 2, 1),
            ] {
                let got = lazy_in(Kind::JsonPath, text, q, limits);
                if reads_the_root && got.is_err() && expected.is_ok() {
                    continue;
                }
                // A filter is not run on a child that is too big to be given to it.
                let refused = got.as_ref().is_ok_and(|out| {
                    out.iter()
                        .any(|o| matches!(o, Err(e) if e.contains("too big to be tested")))
                });
                if refused {
                    continue;
                }
                assert_eq!(
                    got.map(shape),
                    expected.clone().map(shape),
                    "{q}  on  {text}"
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 6_000, "{compared}");
}

#[test]
fn a_jsonpath_filter_that_reads_the_root_is_refused_not_misread() {
    let tree = tree_of(STORE);
    let q = "$..book[?(@.price < $.expensive)].title";
    let err = run_with(
        Kind::JsonPath,
        &tree,
        q,
        walk_everything(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(err.to_string().contains("from its root"), "{err}");
}

#[test]
fn a_jsonpath_that_is_not_one_is_refused_in_the_words_it_is_for_a_value() {
    for q in ["$.a[", "a.b", "$.[?(@.x ==)]"] {
        let tree = tree_of("[1]");
        let lazy = run_with(
            Kind::JsonPath,
            &tree,
            q,
            walk_everything(),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        let eager = eager_in(Kind::JsonPath, "[1]", q);
        if let Err(eager) = eager {
            assert_eq!(lazy.unwrap_err().to_string(), eager, "{q}");
        }
    }
}

#[test]
fn a_long_list_is_walked_by_jsonpath_as_the_threads_of_a_query_walk_it() {
    let text = format!(
        "{{\"items\":[{}]}}",
        (0..6_000)
            .map(|n| format!("{{\"id\":{n},\"k\":\"{}\"}}", n % 7))
            .collect::<Vec<_>>()
            .join(",")
    );
    let limits = by_threads(Limits::default(), 4, 16, 100);
    for q in [
        "$.items[*].id",
        "$.items[?(@.k == '3')].id",
        "$.items[100:200].id",
        "$..id",
        "$.items[-3:]",
        "$.items[::1000].id",
    ] {
        assert_eq!(
            lazy_in(Kind::JsonPath, &text, q, limits),
            eager_in(Kind::JsonPath, &text, q),
            "{q}"
        );
    }
}

#[test]
fn a_stream_of_values_is_walked_as_the_array_it_is() {
    let text = "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n";
    let input: Value = json!([{"a": 1}, {"a": 2}, {"a": 3}]);
    let tree = tree_of(text);
    for q in [
        ".",
        "length",
        ".[1]",
        ".[].a",
        "map(.a)",
        "[.[] | .a] | add",
        ".[-1].a",
    ] {
        let mut lazy = Vec::new();
        run_with(
            Kind::Jq,
            &tree,
            q,
            walk_everything(),
            &AtomicBool::new(false),
            &mut |e| {
                lazy.push(match e {
                    QueryEvent::Item(v) => Ok(v),
                    QueryEvent::ItemError(e) => Err(e),
                })
            },
        )
        .unwrap();
        let mut eager = Vec::new();
        run_query(&input, q, &AtomicBool::new(false), |e| {
            eager.push(match e {
                JaqEvent::Item(v) => Ok(v),
                JaqEvent::ItemError(e) => Err(e),
            })
        })
        .unwrap();
        assert_eq!(lazy, eager, "{q}");
    }
}

#[test]
fn a_document_of_a_few_megabytes_is_queried_without_being_parsed() {
    // Records of a few fields: some 3 MB, which is a lot more than the 1 MiB
    // that a list may be before it is walked rather than parsed.
    let mut text = String::from("[");
    for i in 0..40_000 {
        if i > 0 {
            text.push(',');
        }
        text.push_str(&format!(
            "{{\"id\":{i},\"name\":\"item {i}\",\"tags\":[\"a\",\"b\"],\"score\":{}.5}}",
            i % 100
        ));
    }
    text.push(']');
    assert!(text.len() > 2_000_000);
    let limits = Limits {
        materialize_bytes: 1024 * 1024,
        ..Limits::default()
    };
    let input: Value = serde_json::from_str(&text).unwrap();

    for q in [
        "length",
        ".[39999].name",
        ".[-1] | .id",
        "map(.id) | add",
        "[.[] | select(.score > 98) | .id] | length",
        ".[] | select(.id == 12345) | .name",
        "first(.[] | select(.name == \"item 777\")) | .id",
        "map(.tags | length) | add",
        "[.[1000:1003][] | .name]",
        ".[500:503] | map(.score)",
        "limit(3; .[] | select(.score == 7) | .id)",
        "sort_by(.score) | .[0] | .id",
        "sort_by(.score) | .[0:3] | map(.id)",
        "sort_by(.name) | .[-1] | .id",
        "sort_by(.score, .id) | .[100:103] | map(.id)",
        "group_by(.score) | length",
        "group_by(.score) | map(length) | add",
        "group_by(.score) | map({score: .[0].score, n: length}) | .[0:3]",
        "group_by(.tags | length) | map(length)",
        "min_by(.score) | .id",
        "max_by(.score) | .id",
        "max_by(.id) | .name",
        "unique_by(.score) | length",
        "unique_by(.score) | map(.id) | .[0:5]",
        "[.[] | .score] | unique | length",
        "reverse | .[0:2] | map(.id)",
        "sort_by(.score) | reverse | .[0].id",
        "{n: length, first: .[0].id, last: .[-1].id}",
        "[.[] | select(.score > 98)] | length",
        "[.. | numbers] | length",
        "[.[0:100][] | {id, name}] | length",
    ] {
        let got = lazy_with(&text, q, limits).unwrap();
        let mut expected = Vec::new();
        run_query(&input, q, &AtomicBool::new(false), |e| {
            expected.push(match e {
                JaqEvent::Item(v) => Ok(v),
                JaqEvent::ItemError(e) => Err(e),
            })
        })
        .unwrap();
        assert_eq!(got, expected, "{q}");
    }

    // What needs all of it is refused, not attempted.
    let out = lazy_with(&text, "to_entries | .[0]", limits).unwrap();
    assert!(out[0]
        .as_ref()
        .unwrap_err()
        .contains("needs all of an array of 40000 items"));
    let small_results = Limits {
        result_bytes: 1024 * 1024,
        ..limits
    };
    let out = lazy_with(&text, ".", small_results).unwrap();
    assert!(out[0].as_ref().unwrap_err().contains("too big to show"));
}

#[test]
fn what_is_walked_by_many_threads_is_what_one_thread_makes() {
    for (threads, chunk, head) in [(3, 2, 0), (2, 1, 3)] {
        for text in DOCUMENTS {
            for q in WALKED {
                let expected = shape(eager(text, q));
                let got = lazy_with(text, q, by_threads(walk_everything(), threads, chunk, head))
                    .unwrap();
                assert_eq!(shape(got), expected, "{q}  on  {text}  ({threads} threads)");
            }
            for q in MIXED {
                for bytes in [0, 60] {
                    let limits = by_threads(walk_the_big_ones(bytes), threads, chunk, head);
                    let got = lazy_with(text, q, limits).unwrap();
                    if got
                        .iter()
                        .any(|o| matches!(o, Err(e) if e.contains("needs all of")))
                    {
                        continue;
                    }
                    assert_eq!(
                        shape(got),
                        shape(eager(text, q)),
                        "{q}  on  {text}  (limit {bytes}, {threads} threads)"
                    );
                }
            }
        }
    }
}

#[test]
fn a_long_list_shared_out_to_threads_comes_back_in_its_own_order() {
    let text = format!(
        "[{}]",
        (0..5_000)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let limits = by_threads(Limits::default(), 4, 16, 100);
    for q in [
        ".[] | select(. % 7 == 0)",
        "[.[] | . * 2] | length",
        ".[] | if . % 1000 == 999 then error(\"x\") else . end",
        "map(select(. > 4990))",
        ".[1000:4000][] | . + 1",
        "first(.[] | select(. > 4000))",
        "limit(5; .[] | select(. % 1000 == 3))",
        ".[] | select(. > 4995), 7",
    ] {
        let expected = eager(&text, q);
        assert_eq!(lazy_with(&text, q, limits).unwrap(), expected, "{q}");
    }
}

#[test]
fn a_query_that_is_stopped_stops_among_threads_too() {
    let text = format!(
        "[{}]",
        (0..50_000)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let tree = tree_of(&text);
    let cancelled = AtomicBool::new(false);
    let mut seen = 0usize;
    run_with(
        Kind::Jq,
        &tree,
        ".[] | . * 2",
        by_threads(walk_everything(), 4, 64, 10),
        &cancelled,
        &mut |_| {
            seen += 1;
            if seen == 1_000 {
                cancelled.store(true, Ordering::Relaxed);
            }
        },
    )
    .unwrap();
    assert_eq!(seen, 1_000);
}

// ---- what needs a key of every element ----------------------------------------

const TIES: &str = r#"[
  {"id":1,"k":"b","n":1}, {"id":2,"k":"a","n":1}, {"id":3,"k":"b","n":0},
  {"id":4,"k":"a","n":1}, {"id":5,"k":"c","n":null}, {"id":6,"k":"a","n":0},
  {"id":7,"k":"b","n":1}, {"id":8,"k":"a","n":null}
]"#;

#[test]
fn elements_with_the_same_key_stay_in_the_order_of_the_file() {
    for q in [
        "sort_by(.n) | map(.id)",
        "sort_by(.k) | map(.id)",
        "sort_by(.k, .n) | map(.id)",
        "sort_by(.n, .k) | map(.id)",
        "group_by(.n) | map(map(.id))",
        "group_by(.k) | map(map(.id))",
        "unique_by(.n) | map(.id)",
        "unique_by(.k) | map(.id)",
        // The first that is smallest and the last that is biggest, as in jaq.
        "min_by(.n) | .id",
        "max_by(.n) | .id",
        "min_by(.k) | .id",
        "max_by(.k) | .id",
        "min_by(.n, .k) | .id",
        "max_by(.n, .k) | .id",
        "reverse | map(.id)",
        "sort_by(.n) | reverse | map(.id)",
    ] {
        let expected = eager(TIES, q);
        for limits in [
            walk_everything(),
            by_threads(walk_everything(), 3, 2, 1),
            by_threads(walk_everything(), 2, 1, 0),
        ] {
            assert_eq!(lazy_with(TIES, q, limits).unwrap(), expected, "{q}");
        }
    }
}

#[test]
fn the_first_element_whose_key_cannot_be_made_is_the_error() {
    let text = r#"[{"a":{"b":1}},{"a":5},{"a":"x"},{"a":{"b":2}},{"a":null},{"a":[1]}]"#;
    for q in [
        "sort_by(.a.b)",
        "group_by(.a.b)",
        "unique_by(.a.b)",
        "min_by(.a.b)",
        "max_by(.a.b)",
        "sort_by(.a.b) | length",
        "sort_by(.a.b?)",
    ] {
        let expected = eager(text, q);
        for limits in [
            walk_everything(),
            by_threads(walk_everything(), 3, 1, 0),
            by_threads(walk_everything(), 4, 2, 2),
        ] {
            assert_eq!(lazy_with(text, q, limits).unwrap(), expected, "{q}");
        }
    }
    // A key that is an error of the program's own, where the elements are given to jaq.
    assert_eq!(
        lazy_with(text, "sort_by(.a | error(\"boom\"))", Limits::default()).unwrap(),
        eager(text, "sort_by(.a | error(\"boom\"))")
    );
    // The list that is not one is the error that jaq makes of it, whole.
    for (doc, q) in [
        ("{\"a\":1}", "sort_by(.)"),
        ("\"abc\"", "reverse"),
        ("null", "group_by(.)"),
        ("5", "min"),
    ] {
        // (What a node too big to show is called is not what jaq calls the value.)
        assert_eq!(
            shape(lazy_with(doc, q, walk_everything()).unwrap()),
            shape(eager(doc, q)),
            "{q}  on  {doc}"
        );
    }
}

#[test]
fn a_long_list_put_in_order_by_threads_is_in_the_order_one_thread_puts_it() {
    // Keys that repeat, of several kinds, in an order that is not any.
    let mut rng = 12345u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let items: Vec<String> = (0..6_000)
        .map(|id| {
            let k = next() % 23;
            let n = match next() % 5 {
                0 => "null".to_owned(),
                1 => format!("\"s{}\"", next() % 9),
                _ => (next() % 40).to_string(),
            };
            format!("{{\"id\":{id},\"k\":{k},\"n\":{n}}}")
        })
        .collect();
    let text = format!("[{}]", items.join(","));
    let limits = by_threads(Limits::default(), 4, 16, 100);
    for q in [
        "sort_by(.k) | map(.id) | .[0:200]",
        "sort_by(.n, .k) | map(.id) | .[-200:]",
        "group_by(.k) | map(length)",
        "group_by(.n) | map({n: .[0].n, ids: (map(.id) | .[0:3])})",
        "unique_by(.n) | map(.id)",
        "min_by(.n) | .id",
        "max_by(.n) | .id",
        "min_by(.k, .id) | .id",
        "sort_by(.k) | reverse | .[0:5] | map(.id)",
        "group_by(.k) | sort_by(length) | map(.[0].k)",
        "group_by(.k) | max_by(length) | length",
    ] {
        assert_eq!(lazy_with(&text, q, limits).unwrap(), eager(&text, q), "{q}");
    }
}

#[test]
fn numbers_that_a_float_cannot_tell_apart_are_ordered_as_they_are() {
    let text = r#"[{"id":9007199254740993},{"id":9007199254740992},{"id":9007199254740994},
                    {"id":9223372036854775807},{"id":9223372036854775806},{"id":1.5},{"id":-2}]"#;
    for q in [
        "sort_by(.id) | map(.id)",
        "max_by(.id) | .id",
        "min_by(.id) | .id",
        "group_by(.id) | length",
        "unique_by(.id) | map(.id)",
    ] {
        assert_eq!(
            lazy_with(text, q, walk_everything()).unwrap(),
            eager(text, q),
            "{q}"
        );
    }
}

#[test]
fn the_keys_of_a_list_are_held_to_a_limit() {
    let text = format!(
        "[{}]",
        (0..300)
            .map(|n| format!("{{\"n\":{}}}", 300 - n))
            .collect::<Vec<_>>()
            .join(",")
    );
    let limits = Limits {
        key_bytes: 2_000,
        ..walk_everything()
    };
    for q in ["sort_by(.n)", "group_by(.n)", "unique_by(.n)"] {
        let out = lazy_with(&text, q, limits).unwrap();
        assert_eq!(out.len(), 1, "{q}");
        let message = out[0].as_ref().unwrap_err();
        assert!(
            message.contains("more than can be put in order here"),
            "{q}: {message}"
        );
        // A refusal is not an error of the program, which `?` would let by.
        let out = lazy_with(&text, &format!("({q})?"), limits).unwrap();
        assert_eq!(out.len(), 1, "({q})?");
        assert!(out[0].is_err(), "({q})?");
    }
    // Looking for the smallest keeps no keys, and so has no such limit.
    let out = lazy_with(&text, "min_by(.n) | .n", limits).unwrap();
    assert_eq!(out, vec![Ok(json!(1))]);
}

#[test]
fn a_list_of_groups_is_put_in_order_but_not_grouped_again() {
    let text = TIES;
    let out = lazy_with(text, "group_by(.k) | group_by(length)", walk_everything()).unwrap();
    assert_eq!(out.len(), 1);
    let message = out[0].as_ref().unwrap_err();
    assert!(message.contains("not grouped again"), "{message}");
    // Put in order, searched and cut, it is.
    for q in [
        "group_by(.k) | sort_by(length) | map(length)",
        "group_by(.k) | sort_by(.[0].n) | map(.[0].id)",
        "group_by(.k) | reverse | map(.[0].id)",
        "group_by(.k) | unique_by(length) | map(length)",
        "group_by(.k) | min_by(length) | map(.id)",
        "group_by(.k) | max_by(length) | map(.id)",
        "group_by(.k) | sort_by(length) | .[1:] | map(length)",
        "group_by(.k) | sort_by(length) | reverse | .[0] | map(.id)",
    ] {
        assert_eq!(
            lazy_with(text, q, walk_everything()).unwrap(),
            eager(text, q),
            "{q}"
        );
    }
}

#[test]
fn what_is_put_in_order_is_a_list_that_can_be_shown_or_refused_as_any_is() {
    let text = TIES;
    let limits = Limits {
        result_bytes: 40,
        ..walk_everything()
    };
    // The whole of it is too big to show; a part of it is not.
    let out = lazy_with(text, "sort_by(.id) | reverse", limits).unwrap();
    assert!(out[0].as_ref().unwrap_err().contains("too big to show"));
    let out = lazy_with(text, "sort_by(.id) | reverse | .[0:2] | map(.id)", limits).unwrap();
    assert_eq!(out, vec![Ok(json!([8, 7]))]);
    // What comes out of it can be what is in the file as it is, byte for byte.
    let out = lazy_with(text, "sort_by(.id) | reverse | first", walk_everything()).unwrap();
    assert_eq!(out, vec![Ok(json!({"id": 8, "k": "a", "n": null}))]);
}

#[test]
fn a_part_that_is_too_big_is_an_error_only_where_it_is_run() {
    let text = "[1,2,3,4,5,6,7,8,9,10]";
    let limits = walk_the_big_ones(8);
    // `add` needs all of the list; `false and …` does not run it, and `true and …`
    // does.
    let out = lazy_with(text, "false and add", limits).unwrap();
    assert_eq!(out, vec![Ok(json!(false))]);
    let out = lazy_with(text, "true and add", limits).unwrap();
    assert_eq!(out.len(), 1);
    assert!(out[0].as_ref().unwrap_err().contains("`add` needs all of"));
    let out = lazy_with(text, "null // add", limits).unwrap();
    assert!(out[0].as_ref().unwrap_err().contains("`add` needs all of"));
    let out = lazy_with(text, "1 // add", limits).unwrap();
    assert_eq!(out, vec![Ok(json!(1))]);
    let out = lazy_with(text, "{a: add}", limits).unwrap();
    assert!(out[0].as_ref().unwrap_err().contains("`add` needs all of"));
    // An error of the program that comes where a refusal might is the program's.
    let out = lazy_with(text, "(.[0] | error(\"mine\")) // 5", Limits::default()).unwrap();
    assert_eq!(out, eager(text, "(.[0] | error(\"mine\")) // 5"));
}

const JMESPATHS: &[&str] = &[
    "@",
    "a",
    "a.b",
    "a.b.c",
    "[0]",
    "[1]",
    "[-1]",
    "[-2]",
    "[100]",
    "[1:3]",
    "[:2]",
    "[-2:]",
    "[::2]",
    "[1::2]",
    "[::-1]",
    "[3:0:-1]",
    "[5:2]",
    "[*]",
    "[*].a",
    "[*].a.b",
    "[].a",
    "[*][0]",
    "[0][0]",
    "[0] || [1]",
    "a || b",
    "a && b",
    "!a",
    "*",
    "*.a",
    "[*].*",
    "a[0]",
    "a[*].b",
    "a[?b > `1`]",
    "[?a == `1`]",
    "[?a]",
    "[?a].a",
    "[?n >= `1`].id",
    "[?k == 'a'].id",
    "[?k == 'a' && n >= `1`].id",
    "[?k == 'a' || k == 'c'].id",
    "[?!k].id",
    "[?length(@) > `1`]",
    "users[*].name",
    "users[0].tags[0]",
    "users[*].tags[]",
    "users[?tags].name",
    "users[?length(tags) > `0`].name",
    "users[1:].name",
    "users[*].{name: name, n: length(tags)}",
    "[*].{x: a}",
    "{n: length(@), first: [0]}",
    "{a: a, b: b}",
    "[a, b]",
    "length(@)",
    "length(users)",
    "length([*])",
    "length(a)",
    "keys(@)",
    "values(@)",
    "sort(@)",
    "reverse(@)",
    "max(@)",
    "sum(@)",
    "type(@)",
    "to_string(@)",
    "[*].a | [0]",
    "[*].a | length(@)",
    "[0:2][0]",
    "items[*].n | [0]",
    "items[?n >= `2`].tags",
    "items[*].tags[]",
    "meta.*",
    "store.book[*].author",
    "store.book[?price < `10`].title",
    "store.book[?isbn].title",
    "store.book[-1:].title",
    "store.book[0].price",
    "store.*.price",
    "sort_by(store.book, &price)[0].title",
    "map(&price, store.book)",
    "max_by(store.book, &price).title",
    "`1`",
    "'raw'",
    "foo.",
    "a[",
    "bogus(@)",
    "[::0]",
];

#[test]
fn jmespath_on_a_document_in_its_file_is_what_it_is_on_a_value() {
    let documents: Vec<&str> = DOCUMENTS.iter().copied().chain([STORE]).collect();
    let mut compared = 0;
    for text in documents {
        for q in JMESPATHS {
            let expected = eager_in(Kind::JmesPath, text, q);
            for limits in [
                walk_everything(),
                walk_the_big_ones(8),
                walk_the_big_ones(30),
                walk_the_big_ones(120),
                by_threads(walk_the_big_ones(8), 3, 2, 1),
            ] {
                let got = lazy_in(Kind::JmesPath, text, q, limits);
                // What needs all of a list that is too big for it is refused: a function of it.
                let refused =
                    matches!(&got, Err(e) if e.contains("too big") || e.contains("needs all of"));
                if refused && expected.is_ok() {
                    continue;
                }
                assert_eq!(got, expected, "{q}  on  {text}  (limit {limits:?})");
                compared += 1;
            }
        }
    }
    assert!(compared > 6_000, "{compared}");
}

#[test]
fn jmespath_walks_a_long_list_as_the_threads_of_a_query_do() {
    let text = format!(
        "{{\"items\":[{}]}}",
        (0..6_000)
            .map(|n| format!("{{\"id\":{n},\"k\":\"{}\",\"t\":[{n},1]}}", n % 7))
            .collect::<Vec<_>>()
            .join(",")
    );
    let limits = by_threads(Limits::default(), 4, 16, 100);
    for q in [
        "items[*].id",
        "items[?k == '3'].id",
        "items[100:200].id",
        "items[::1000].id",
        "items[-3:]",
        "length(items)",
        "items[*].t[]",
        "items[0:3].{id: id, k: k}",
        "{n: length(items), last: items[-1].id}",
    ] {
        assert_eq!(
            lazy_in(Kind::JmesPath, &text, q, limits),
            eager_in(Kind::JmesPath, &text, q),
            "{q}"
        );
    }
}

#[test]
fn what_jmespath_needs_all_of_a_list_for_is_refused_and_says_so() {
    let tree = tree_of(STORE);
    let err = run_with(
        Kind::JmesPath,
        &tree,
        "sort_by(store.book, &price)[0].title",
        Limits {
            result_bytes: 100,
            ..walk_everything()
        },
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(err.to_string().contains("too big"), "{err}");
}

// ---- a document laid out as Format lays it out ---------------------------------

#[test]
fn a_document_in_its_file_is_laid_out_as_format_lays_out_a_value() {
    use jsonquery_core::lazy::{Indent as FileIndent, PrettyLimits, Style};

    let strange = [
        "[\"é\",\"\\u00e9\",\"\\ud83d\\ude00\",\"a\\nb\\t\\u0001\\\"\\\\\\/\",{\"é\":\"ü\",\"\\u0041\":1}]",
        "{\"a\":1.0,\"b\":1E5,\"c\":-0,\"d\":123456789012345678901234567890,\"e\":0.50}",
        "{\"a\":1,\"b\":2,\"a\":3}",
        "[[],{},[[]],{\"a\":{}}]",
        "\"just a string é\"",
        "7",
        "{\"k\":\"\\u007f\\u0080\\u2028\"}",
    ];
    let mut compared = 0;
    for text in DOCUMENTS
        .iter()
        .chain(strange.iter())
        .copied()
        .chain([STORE])
    {
        let value: Value = serde_json::from_str(text).unwrap();
        let tree = tree_of(text);
        for (indent, file_indent) in [
            (reformat::Indent::Spaces(2), FileIndent::Spaces(2)),
            (reformat::Indent::Spaces(4), FileIndent::Spaces(4)),
            (reformat::Indent::Tab, FileIndent::Tab),
            (reformat::Indent::Minified, FileIndent::Minified),
        ] {
            for ascii_only in [false, true] {
                let expected = reformat::render(
                    &value,
                    &reformat::Options {
                        indent,
                        sort_keys: false,
                        ascii_only,
                    },
                );
                let mut out = Vec::new();
                tree.root()
                    .write_styled(
                        &mut out,
                        Style {
                            indent: file_indent,
                            ascii_only,
                        },
                        PrettyLimits::default(),
                    )
                    .unwrap();
                assert_eq!(
                    String::from_utf8(out).unwrap(),
                    expected,
                    "{text}  as  {indent:?}, ascii {ascii_only}"
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 200, "{compared}");
}
