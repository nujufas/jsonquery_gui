//! jq builtins that jaq lacks, loaded after jaq's own.
//!
//! jaq is not jq, and a few things people paste from the jq manual are not in
//! it. These are added here, in the same two forms jaq's own builtins come in:
//!
//! - **Definitions written in jq** ([`defs`], `jq_ext/prelude.jq`): the
//!   SQL-style `IN`, `INDEX` and `JOIN`, and `fromstream` and `truncate_stream`.
//! - **Native filters** ([`funs`]): `tostream`, and `@csv` and `@tsv`. They are
//!   native for speed (`tostream`) or because the work is byte shuffling that
//!   jq code does badly (`@csv`, `@tsv`).
//!
//! [`crate::run_query_with_vars`] is the one place a jq program is compiled, so
//! the Merge tool's filter gets all of them too.
//!
//! `@csv` and `@tsv` produce *text*: a string per row. What to do with such
//! results — copy them as CSV rather than as a quoted JSON string — is
//! [`crate::output`]'s business.

use jaq_core::load::parse::Def;
use jaq_core::native::Fun;
use jaq_core::DataT;
use jaq_json::Val;

mod tabular;
mod tostream;

/// The definitions written in jq. Like jaq's own they need its native filters
/// loaded as well (`getpath`, `any`, `foreach`, …), and must come after its
/// definitions in the list handed to the loader.
pub fn defs() -> impl Iterator<Item = Def<&'static str>> {
    // Parsed on every call, like jaq's own `defs()`: it takes microseconds.
    jaq_core::load::parse(include_str!("jq_ext/prelude.jq"), |p| p.defs())
        .expect("jq_ext/prelude.jq parses")
        .into_iter()
}

/// The native filters.
pub fn funs<D: for<'a> DataT<V<'a> = Val>>() -> impl Iterator<Item = Fun<D>> {
    let [csv, tsv] = tabular::funs::<D>();
    [csv, tsv, tostream::fun::<D>()].into_iter()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use serde_json::{json, Value};

    use crate::{run_query, QueryEvent};

    /// Every output of `filter` over `input`, or the first error: jq stops at the
    /// first one, and so do these tests.
    fn run(input: &Value, filter: &str) -> Result<Vec<Value>, String> {
        let mut outputs = Vec::new();
        let mut error = None;
        let result = run_query(
            input,
            filter,
            &AtomicBool::new(false),
            |event| match event {
                QueryEvent::Item(value) => outputs.push(value),
                QueryEvent::ItemError(message) => {
                    error.get_or_insert(message);
                }
            },
        );
        match (result, error) {
            (Err(e), _) => Err(e.to_string()),
            (Ok(_), Some(message)) => Err(message),
            (Ok(_), None) => Ok(outputs),
        }
    }

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    /// `filter` over `input` (JSON text) outputs exactly `expected` (JSON text,
    /// one per output), in order. The cases that follow were taken from jq 1.8.1
    /// itself, so that they hold on a machine that has no jq.
    #[track_caller]
    fn gives(input: &str, filter: &str, expected: &[&str]) {
        let expected: Vec<Value> = expected.iter().map(|e| parse(e)).collect();
        assert_eq!(
            run(&parse(input), filter),
            Ok(expected),
            "`{filter}` over {input}"
        );
    }

    /// `filter` over `input` stops with an error, as it does in jq.
    #[track_caller]
    fn fails(input: &str, filter: &str) {
        let outcome = run(&parse(input), filter);
        assert!(outcome.is_err(), "`{filter}` over {input} gave {outcome:?}");
    }

    fn error_of(input: &str, filter: &str) -> String {
        run(&parse(input), filter).expect_err("the filter fails")
    }

    #[test]
    #[rustfmt::skip]
    fn in_asks_whether_a_value_is_among_others() {
        gives(r#"2"#, r#"IN(1,2,3)"#, &[r#"true"#]);
        gives(r#"5"#, r#"IN(1,2,3)"#, &[r#"false"#]);
        gives(r#"[1,2,3,4]"#, r#"[.[] | IN(2,3)]"#, &[r#"[false,true,true,false]"#]);
        gives(r#"[1,2,3]"#, r#"IN(.[]; 2,3)"#, &[r#"true"#]);
        gives(r#"[1,2,3]"#, r#"IN(.[]; 7,8)"#, &[r#"false"#]);
        gives(r#"null"#, r#"IN(empty)"#, &[r#"false"#]);
        gives(r#"{"a":1}"#, r#"IN({"a":1}, {"b":2})"#, &[r#"true"#]);
        gives(r#"{"a":1}"#, r#"IN({"b":2})"#, &[r#"false"#]);
        gives(r#"[1,2]"#, r#"IN([1,2])"#, &[r#"true"#]);
        gives(r#""a""#, r#"IN("a","b")"#, &[r#"true"#]);
        gives(r#"1"#, r#"IN(1.0)"#, &[r#"true"#]);
        gives(r#"null"#, r#"IN(null)"#, &[r#"true"#]);
        gives(r#"5"#, r#"IN(range(100000000))"#, &[r#"true"#]);
        gives(r#"[3,4]"#, r#"IN(.[]; .[])"#, &[r#"true"#]);
        gives(r#"{"members":[{"role":"dev"},{"role":"lead"},{"role":"ops"}]}"#, r#"[.members[] | select(.role | IN("dev","ops"))]"#, &[r#"[{"role":"dev"},{"role":"ops"}]"#]);
    }

    #[test]
    #[rustfmt::skip]
    fn index_makes_an_object_keyed_by_an_expression() {
        gives(r#"[{"id":1,"v":"a"},{"id":2,"v":"b"}]"#, r#"INDEX(.id)"#, &[r#"{"1":{"id":1,"v":"a"},"2":{"id":2,"v":"b"}}"#]);
        gives(r#"[{"id":1,"v":"a"},{"id":2,"v":"b"}]"#, r#"INDEX(.[]; .id)"#, &[r#"{"1":{"id":1,"v":"a"},"2":{"id":2,"v":"b"}}"#]);
        gives(r#"[{"k":"x","v":1},{"k":"x","v":2}]"#, r#"INDEX(.k)"#, &[r#"{"x":{"k":"x","v":2}}"#]);
        gives(r#"[]"#, r#"INDEX(.id)"#, &[r#"{}"#]);
        gives(r#"[{"id":null}]"#, r#"INDEX(.id)"#, &[r#"{"null":{"id":null}}"#]);
        gives(r#"[{"id":true},{"id":false}]"#, r#"INDEX(.id)"#, &[r#"{"true":{"id":true},"false":{"id":false}}"#]);
        gives(r#"[{"id":1.5}]"#, r#"INDEX(.id)"#, &[r#"{"1.5":{"id":1.5}}"#]);
        gives(r#"null"#, r#"INDEX(range(3); .*2)"#, &[r#"{"0":0,"2":1,"4":2}"#]);
        gives(r#"[{"a":"x","b":"y"}]"#, r#"INDEX(.[]; .a, .b)"#, &[r#"{"x":{"a":"x","b":"y"},"y":{"a":"x","b":"y"}}"#]);
        gives(r#"[{"id":1},{"x":2}]"#, r#"INDEX(.id)"#, &[r#"{"1":{"id":1},"null":{"x":2}}"#]);
        gives(r#"[[1,2],[3,4]]"#, r#"INDEX(.[0])"#, &[r#"{"1":[1,2],"3":[3,4]}"#]);
        gives(r#"{"a":{"id":1},"b":{"id":2}}"#, r#"INDEX(.[]; .id)"#, &[r#"{"1":{"id":1},"2":{"id":2}}"#]);
        gives(r#"["a","b","a"]"#, r#"INDEX(.[]; .)"#, &[r#"{"a":"a","b":"b"}"#]);
        gives(r#"[{"id":{"x":1}}]"#, r#"INDEX(.id)"#, &[r#"{"{\"x\":1}":{"id":{"x":1}}}"#]);
        gives(r#"[{"id":"é"}]"#, r#"INDEX(.id)"#, &[r#"{"é":{"id":"é"}}"#]);
        gives(r#"["x"]"#, r#"INDEX(.[]; empty)"#, &[r#"{}"#]);
    }

    #[test]
    #[rustfmt::skip]
    fn join_pairs_each_row_with_its_match_in_a_lookup_object() {
        gives(r#"[1,2]"#, r#"JOIN({"1":"x","2":"y"}; tostring)"#, &[r#"[[1,"x"],[2,"y"]]"#]);
        gives(r#"null"#, r#"[JOIN({"1":"x","2":"y"}; 1,2; tostring)]"#, &[r#"[[1,"x"],[2,"y"]]"#]);
        gives(r#"null"#, r#"[JOIN({"1":"x","2":"y"}; 1,2; tostring; .[1])]"#, &[r#"["x","y"]"#]);
    }

    #[test]
    #[rustfmt::skip]
    fn tostream_gives_the_events_of_a_value() {
        gives(r#"1"#, r#"[tostream]"#, &[r#"[[[],1]]"#]);
        gives(r#"null"#, r#"[tostream]"#, &[r#"[[[],null]]"#]);
        gives(r#""a""#, r#"[tostream]"#, &[r#"[[[],"a"]]"#]);
        gives(r#"[]"#, r#"[tostream]"#, &[r#"[[[],[]]]"#]);
        gives(r#"{}"#, r#"[tostream]"#, &[r#"[[[],{}]]"#]);
        gives(r#"{"a":1}"#, r#"[tostream]"#, &[r#"[[["a"],1],[["a"]]]"#]);
        gives(r#"[1,[2,3]]"#, r#"[tostream]"#, &[r#"[[[0],1],[[1,0],2],[[1,1],3],[[1,1]],[[1]]]"#]);
        gives(r#"{"a":[1,{"b":2}],"c":null}"#, r#"[tostream]"#, &[r#"[[["a",0],1],[["a",1,"b"],2],[["a",1,"b"]],[["a",1]],[["c"],null],[["c"]]]"#]);
        gives(r#"[[]]"#, r#"[tostream]"#, &[r#"[[[0],[]],[[0]]]"#]);
        gives(r#"[{}]"#, r#"[tostream]"#, &[r#"[[[0],{}],[[0]]]"#]);
        gives(r#"{"a":{}}"#, r#"[tostream]"#, &[r#"[[["a"],{}],[["a"]]]"#]);
        gives(r#"{"a":[],"b":{}}"#, r#"[tostream]"#, &[r#"[[["a"],[]],[["b"],{}],[["b"]]]"#]);
        gives(r#"{"a":[1,2,{"b":[true,false,null]}],"z":"s"}"#, r#"[tostream]"#, &[r#"[[["a",0],1],[["a",1],2],[["a",2,"b",0],true],[["a",2,"b",1],false],[["a",2,"b",2],null],[["a",2,"b",2]],[["a",2,"b"]],[["a",2]],[["z"],"s"],[["z"]]]"#]);
        gives(r#"[[[[1]]]]"#, r#"[tostream]"#, &[r#"[[[0,0,0,0],1],[[0,0,0,0]],[[0,0,0]],[[0,0]],[[0]]]"#]);
        gives(r#"{"a":null}"#, r#"[tostream]"#, &[r#"[[["a"],null],[["a"]]]"#]);
        gives(r#"[null]"#, r#"[tostream]"#, &[r#"[[[0],null],[[0]]]"#]);
        gives(r#"{"b":1,"a":2}"#, r#"[tostream]"#, &[r#"[[["b"],1],[["a"],2],[["a"]]]"#]);
        gives(r#"[1,2,3]"#, r#"[limit(2; tostream)]"#, &[r#"[[[0],1],[[1],2]]"#]);
        gives(r#"{"a":[1,2]}"#, r#"[tostream | select(length==2) | .[0] | map(tostring) | join(".")]"#, &[r#"["a.0","a.1"]"#]);
        gives(r#"[1,2,3,4,5]"#, r#"[tostream | select(length == 2) | .[1]] | add"#, &[r#"15"#]);
        gives(r#"{"a":[1,2]}"#, r#"[tostream | select(length == 1)]"#, &[r#"[[["a",1]],[["a"]]]"#]);
        gives(r#"{"a":1,"b":[2,3]}"#, r#"reduce (tostream | select(length==2)) as [$p,$v] ({}; .[$p | map(tostring) | join("/")] = $v)"#, &[r#"{"a":1,"b/0":2,"b/1":3}"#]);
    }

    #[test]
    #[rustfmt::skip]
    fn fromstream_and_truncate_stream_build_values_back_from_events() {
        gives(r#"{"a":[1,{"b":2}],"c":null}"#, r#"fromstream(tostream)"#, &[r#"{"a":[1,{"b":2}],"c":null}"#]);
        gives(r#"[1,[2,3],{"x":[]}]"#, r#"[fromstream(tostream)]"#, &[r#"[[1,[2,3],{"x":[]}]]"#]);
        gives(r#"3"#, r#"[fromstream(tostream)]"#, &[r#"[3]"#]);
        gives(r#"[]"#, r#"[fromstream(tostream)]"#, &[r#"[[]]"#]);
        gives(r#"{}"#, r#"[fromstream(tostream)]"#, &[r#"[{}]"#]);
        gives(r#"null"#, r#"[fromstream(([1,2],{"a":3}) | tostream)]"#, &[r#"[[1,2],{"a":3}]"#]);
        gives(r#"null"#, r#"[1|truncate_stream([[0],1],[[1,0],2],[[1,0]],[[1]])]"#, &[r#"[[[0],2],[[0]]]"#]);
        gives(r#"null"#, r#"fromstream(1|truncate_stream([[0],1],[[1,0],2],[[1,0]],[[1]]))"#, &[r#"[2]"#]);
        gives(r#"{"a":[1,{"b":2}]}"#, r#". as $d | [1 | truncate_stream($d | tostream)]"#, &[r#"[[[0],1],[[1,"b"],2],[[1,"b"]],[[1]]]"#]);
        gives(r#"{"a":[1,{"b":2}]}"#, r#". as $d | [fromstream(1 | truncate_stream($d | tostream))]"#, &[r#"[[1,{"b":2}]]"#]);
        gives(r#"[[1,2,3],[4]]"#, r#"fromstream(tostream)"#, &[r#"[[1,2,3],[4]]"#]);
        gives(r#"[[[1]],[[2,[3]]]]"#, r#"fromstream(tostream)"#, &[r#"[[[1]],[[2,[3]]]]"#]);
        gives(r#"{"a":{"b":{"c":{"d":[1,2,{"e":null}]}}}}"#, r#"fromstream(tostream)"#, &[r#"{"a":{"b":{"c":{"d":[1,2,{"e":null}]}}}}"#]);
        gives(r#"[null,null,[null]]"#, r#"fromstream(tostream)"#, &[r#"[null,null,[null]]"#]);
        gives(r#"{"a":[],"b":{},"c":[[]]}"#, r#"fromstream(tostream)"#, &[r#"{"a":[],"b":{},"c":[[]]}"#]);
        gives(r#""str""#, r#"fromstream(tostream)"#, &[r#""str""#]);
        gives(r#"null"#, r#"fromstream(tostream)"#, &[r#"null"#]);
        gives(r#"true"#, r#"[fromstream(tostream)]"#, &[r#"[true]"#]);
        gives(r#"[1,[2]]"#, r#"[fromstream(tostream, tostream)]"#, &[r#"[[1,[2]],[1,[2]]]"#]);
        gives(r#"null"#, r#"[fromstream([[0],1],[[1],2],[[1]])]"#, &[r#"[[1,2]]"#]);
        gives(r#"null"#, r#"[fromstream([[2],"x"],[[2]])]"#, &[r#"[[null,null,"x"]]"#]);
        gives(r#"null"#, r#"[fromstream([["a","b"],1],[["a","b"]],[["a"]])]"#, &[r#"[{"a":{"b":1}}]"#]);
        gives(r#"null"#, r#"[fromstream([["a"],1],[["b"],2],[["b"]])]"#, &[r#"[{"a":1,"b":2}]"#]);
        gives(r#"null"#, r#"[fromstream(empty)]"#, &[r#"[]"#]);
        gives(r#"{"a":[1,2,3]}"#, r#"[fromstream(1|truncate_stream([[0],1],[[1,0],2],[[1,0]],[[1]]))]"#, &[r#"[[2]]"#]);
        gives(r#"{"users":[{"id":1,"n":"a"},{"id":2,"n":"b"}]}"#, r#". as $d | [fromstream(2 | truncate_stream($d | tostream))]"#, &[r#"[{"id":1,"n":"a"},{"id":2,"n":"b"}]"#]);
        gives(r#"{"users":[{"id":1,"n":"a"},{"id":2,"n":"b"}]}"#, r#". as $d | [fromstream(1 | truncate_stream($d | tostream)) | length]"#, &[r#"[2]"#]);
        gives(r#"{"a":[{"b":1},{"b":2}]}"#, r#"[limit(1; fromstream(tostream))]"#, &[r#"[{"a":[{"b":1},{"b":2}]}]"#]);
        gives(r#"{"big":[1,2,3]}"#, r#"first(fromstream(tostream))"#, &[r#"{"big":[1,2,3]}"#]);
        gives(r#"[[1,2],[3,4]]"#, r#". as $d | [fromstream(1|truncate_stream($d|tostream))]"#, &[r#"[[1,2],[3,4]]"#]);
    }

    #[test]
    #[rustfmt::skip]
    fn csv_writes_an_array_as_a_row() {
        gives(r#"[1,"a b",null,true,false,"x\"y"]"#, r#"@csv"#, &[r#""1,\"a b\",,true,false,\"x\"\"y\"""#]);
        gives(r#"[100000000000000000000,0.1,1.10]"#, r#"@csv"#, &[r#""100000000000000000000,0.1,1.10""#]);
        gives(r#"[0.1,0.2,0.30000000000000004]"#, r#"@csv"#, &[r#""0.1,0.2,0.30000000000000004""#]);
        gives(r#"[1.0,1.50,3.14159265358979323846]"#, r#"@csv"#, &[r#""1.0,1.50,3.14159265358979323846""#]);
        fails(r#"[[1]]"#, r#"@csv"#);
        fails(r#"[{"a":1}]"#, r#"@csv"#);
        fails(r#"{"a":1}"#, r#"@csv"#);
        fails(r#""x""#, r#"@csv"#);
        gives(r#"[]"#, r#"@csv"#, &[r#""""#]);
        gives(r#"["a,b","c"]"#, r#"@csv"#, &[r#""\"a,b\",\"c\"""#]);
        gives(r#"["é","日本"]"#, r#"@csv"#, &[r#""\"é\",\"日本\"""#]);
        gives(r#"[null,null]"#, r#"@csv"#, &[r#"",""#]);
        gives(r#"[[1,"a"],[2,"b"]]"#, r#".[] | @csv"#, &[r#""1,\"a\"""#, r#""2,\"b\"""#]);
        gives(r#"{"a":[1,2],"b":[3]}"#, r#"@csv "\(.a)|\(.b)""#, &[r#""1,2|3""#]);
        gives(r#"["a\"b"]"#, r#"@csv"#, &[r#""\"a\"\"b\"""#]);
        gives(r#"["a\\b"]"#, r#"@csv"#, &[r#""\"a\\b\"""#]);
        gives(r#"[1,[2]]"#, r#"try @csv catch "caught""#, &[r#""caught""#]);
        gives(r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Linus"}]"#, r#".[] | [.id, .name] | @csv"#, &[r#""1,\"Ada\"""#, r#""2,\"Linus\"""#]);
        gives(r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Linus"}]"#, r#"(.[0] | keys_unsorted), (.[] | [.[]]) | @csv"#, &[r#""\"id\",\"name\"""#, r#""1,\"Ada\"""#, r#""2,\"Linus\"""#]);
        gives(r#"["a\nb","c\"d\ne"]"#, r#"@csv"#, &[r#""\"a\nb\",\"c\"\"d\ne\"""#]);
        gives(r#"["line1\r\nline2"]"#, r#"@csv"#, &[r#""\"line1\r\nline2\"""#]);
        gives(r#"["é́","😀"]"#, r#"@csv"#, &[r#""\"é́\",\"😀\"""#]);
        gives(r#"["tab\there"]"#, r#"@csv"#, &[r#""\"tab\there\"""#]);
        gives(r#"[{"n":"Ada, L.","q":"say \"hi\""}]"#, r#".[] | [.n, .q] | @csv"#, &[r#""\"Ada, L.\",\"say \"\"hi\"\"\"""#]);
        gives(r#"[[1,"a,b"],[2,null]]"#, r#".[] | @csv"#, &[r#""1,\"a,b\"""#, r#""2,""#]);
        gives(r#"[[1,"a,b"],[2,null]]"#, r#"map(@csv) | join("\n")"#, &[r#""1,\"a,b\"\n2,""#]);
    }

    #[test]
    #[rustfmt::skip]
    fn tsv_writes_an_array_as_a_row() {
        gives(r#"["a\tb","c\nd","e\\f","g\rh"]"#, r#"@tsv"#, &[r#""a\\tb\tc\\nd\te\\\\f\tg\\rh""#]);
        gives(r#"["é","日本"]"#, r#"@tsv"#, &[r#""é\t日本""#]);
        gives(r#"[true,false,null,1]"#, r#"@tsv"#, &[r#""true\tfalse\t\t1""#]);
        gives(r#"{"a":["x","y"],"b":["p\tq"]}"#, r#"@tsv "\(.a)|\(.b)""#, &[r#""x\ty|p\\tq""#]);
        gives(r#"["\u0000"]"#, r#"@tsv"#, &[r#""\\0""#]);
        gives(r#"[1,{"a":2}]"#, r#"try @tsv catch "caught""#, &[r#""caught""#]);
        gives(r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Linus"}]"#, r#"map([.id,.name]) | .[] | @tsv"#, &[r#""1\tAda""#, r#""2\tLinus""#]);
        gives(r#"[["id","name"],[1,"Ada"]]"#, r#".[] | @tsv"#, &[r#""id\tname""#, r#""1\tAda""#]);
    }

    /// jq spells a number the way its decimal library prints it; jaq prints the
    /// number it was given (`1e3` as `1e+3`, where jq has `1E+3`, and `-0` as `0`),
    /// everywhere, not only in these rows.
    #[test]
    fn numbers_are_spelled_as_jaq_spells_them() {
        gives("[1,2.5,-3,1e3]", "@csv", &[r#""1,2.5,-3,1e+3""#]);
        gives(
            "[1e100,1E-5,12345678901234567890]",
            "@csv",
            &[r#""1e+100,1e-5,12345678901234567890""#],
        );
        gives("[-0,-0.0]", "@csv", &[r#""0,-0.0""#]);
        gives("[1e3]", ".[0]", &["1e+3"]);
    }

    #[test]
    fn errors_are_worded_as_jq_words_them() {
        let say = error_of;
        assert_eq!(
            say(r#"{"a":1}"#, "@csv"),
            r#"object ({"a":1}) cannot be csv-formatted, only an array can be"#
        );
        assert_eq!(
            say(r#"[{"a":1}]"#, "@csv"),
            r#"object ({"a":1}) is not valid in a csv row"#
        );
        assert_eq!(
            say("1", "@csv"),
            "number (1) cannot be csv-formatted, only an array can be"
        );
        assert_eq!(
            say(r#""x""#, "@tsv"),
            r#"string ("x") cannot be tsv-formatted, only an array can be"#
        );
        // jq says "csv row" here too; this says which one it was.
        assert_eq!(
            say("[[1]]", "@tsv"),
            "array ([1]) is not valid in a tsv row"
        );
        assert_eq!(
            say("[[1]]", "@csv"),
            "array ([1]) is not valid in a csv row"
        );
    }

    #[test]
    fn an_error_quotes_only_the_start_of_a_value() {
        assert_eq!(
            error_of(r#"[1,{"b":[1,2,3,4,5,6,7,8,9,10]}]"#, "@csv"),
            r#"object ({"b":[1,2,3,4,5,6,7,8,9,...) is not valid in a csv row"#
        );
        let huge = Value::Array(vec![Value::Array((0..100_000).map(Value::from).collect())]);
        let message = run(&huge, "@csv").expect_err("a nested array is not a field");
        assert!(message.len() < 100, "{message}");
    }

    #[test]
    fn the_message_of_a_failed_row_can_be_caught() {
        gives(
            "[[1]]",
            "try @csv catch .",
            &[r#""array ([1]) is not valid in a csv row""#],
        );
        // Only an array is a row: of these, the number is not, and `@csv?` drops it.
        gives(
            "[1,2]",
            "[.[] | try @tsv catch \"bad\"]",
            &[r#"["bad","bad"]"#],
        );
        gives("[[1],2]", "[.[] | @csv?]", &[r#"["1"]"#]);
    }

    /// jaq prints an error that carries a string as a JSON string; what is shown
    /// is the string.
    #[test]
    fn an_error_that_carries_a_string_is_shown_as_that_string() {
        assert_eq!(error_of("null", r#"error("boom")"#), "boom");
        assert_eq!(error_of("null", r#"error("say \"hi\"")"#), r#"say "hi""#);
        assert_eq!(error_of("null", r#"error({"a":1})"#), r#"{"a":1}"#);
        assert_eq!(error_of("null", "error(1)"), "1");
        assert!(error_of(r#"{"a":1}"#, ".a + {}").starts_with("cannot calculate"));
        gives("null", r#"try error("boom") catch ."#, &[r#""boom""#]);
    }

    #[test]
    fn tostream_stops_when_nothing_more_is_asked_of_it() {
        let big = Value::Array((0..200_000).map(Value::from).collect());
        assert_eq!(
            run(&big, "[limit(3; tostream)]"),
            Ok(vec![parse("[[[0],0],[[1],1],[[2],2]]")])
        );
        assert_eq!(run(&big, "first(tostream)"), Ok(vec![parse("[[0],0]")]));
        // Every item once, and the closing event.
        let events = run(&big, "[tostream] | length").unwrap();
        assert_eq!(events, vec![json!(200_001)]);
    }

    #[test]
    fn a_stream_survives_a_round_trip_through_fromstream() {
        let doc = r#"{"a":[1,{"b":[]},{}],"c":null,"d":{"e":[[],[2,[3]]]},"f":"x","g":[null]}"#;
        gives(doc, "[fromstream(tostream)]", &[&format!("[{doc}]")]);
        gives(
            doc,
            ". as $d | [fromstream($d | tostream)] == [$d]",
            &["true"],
        );
    }

    #[test]
    fn a_users_own_definition_wins() {
        gives("1", "def IN(s): \"mine\"; IN(2)", &[r#""mine""#]);
        gives("1", "def tostream: \"mine\"; tostream", &[r#""mine""#]);
        gives("[1]", "def @csv: \"mine\"; @csv", &[r#""mine""#]);
    }

    #[test]
    fn the_merge_filter_has_them_too() {
        let names = ["a.json".to_owned(), "b.json".to_owned()];
        let merged = crate::merge::merge(
            vec![json!([{"id": "x", "n": 1}]), json!([{"id": "y", "n": 2}])],
            &names,
            "add | INDEX(.id) | map_values(.n)",
            &AtomicBool::new(false),
        )
        .expect("merges");
        assert_eq!(merged.value, json!({"x": 1, "y": 2}));
    }
}
