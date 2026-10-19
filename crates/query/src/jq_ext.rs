//! jq builtins that jaq lacks, loaded after jaq's own.
//!
//! jaq is not jq, and a few things people paste from the jq manual are not in
//! it. These are added here, in the same two forms jaq's own builtins come in:
//!
//! - **Definitions written in jq** ([`defs`], `jq_ext/prelude.jq`): the
//!   SQL-style `IN`, `INDEX` and `JOIN`, and `fromstream` and `truncate_stream`.
//! - **Native filters** ([`funs`]): `tostream`, and `@csv` and `@tsv`. They are
//!   native for speed (`tostream`) or because the work is byte shuffling that
//!   jq code does badly (`@csv`, `@tsv`). `fromjson`, `tonumber` and `tojson` are
//!   native too, and take the place of jaq's (`json_text.rs` says why): of two
//!   filters of a name the first found is the one called, so [`replaces_fun`] and
//!   [`replaces_def`] name what the compiler is not handed of jaq-json's.
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

mod base64;
#[cfg(test)]
mod corpus;
mod json_text;
mod paths;
#[cfg(test)]
mod properties;
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
    let [fromjson, tonumber, tojson] = json_text::funs::<D>();
    let [delpaths] = paths::funs::<D>();
    let [decode_base64] = base64::funs::<D>();
    [
        csv,
        tsv,
        tostream::fun::<D>(),
        fromjson,
        tonumber,
        tojson,
        delpaths,
        decode_base64,
    ]
    .into_iter()
}

/// Whether the native filter `name` taking `arity` arguments of jaq-json or jaq-std is
/// written again in [`funs`], and is to be left out of the list handed to the compiler:
/// the first filter of a name in that list is the one that is called.
pub fn replaces_fun(name: &str, arity: usize) -> bool {
    json_text::replaces_fun(name, arity) || base64::replaces_fun(name, arity)
}

/// Whether jaq-json's definition (in jq) of `name` is written again in [`funs`], and
/// is to be left out of the definitions: a definition is found before a native filter.
pub fn replaces_def(name: &str) -> bool {
    json_text::replaces_def(name)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use serde_json::{json, Value};

    use crate::{run_query, QueryEvent};

    /// Every output of `filter` over `input`, or the first error: jq stops at the
    /// first one, and so do these tests.
    pub(super) fn run(input: &Value, filter: &str) -> Result<Vec<Value>, String> {
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

    pub(super) fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    /// `work` on a thread made for running programs: the values some tests make are
    /// deeper than the stack of a test's own thread can free.
    fn on_a_query_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = std::sync::mpsc::channel();
        crate::spawn_query_thread("test", move || sender.send(work()).unwrap());
        receiver.recv().expect("the thread ends with an answer")
    }

    /// `filter` over `input` (JSON text) outputs exactly `expected` (JSON text,
    /// one per output), in order. The cases that follow were taken from jq 1.8.1
    /// itself, so that they hold on a machine that has no jq.
    #[track_caller]
    pub(super) fn gives(input: &str, filter: &str, expected: &[&str]) {
        let expected: Vec<Value> = expected.iter().map(|e| parse(e)).collect();
        assert_eq!(
            run(&parse(input), filter),
            Ok(expected),
            "`{filter}` over {input}"
        );
    }

    /// `filter` over `input` stops with an error, as it does in jq.
    #[track_caller]
    pub(super) fn fails(input: &str, filter: &str) {
        let outcome = run(&parse(input), filter);
        assert!(outcome.is_err(), "`{filter}` over {input} gave {outcome:?}");
    }

    pub(super) fn error_of(input: &str, filter: &str) -> String {
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

    /// How many outputs `filter` has over `input` (JSON text), looking at no more than
    /// `most` of them: a stream that never ends is told by there being that many.
    fn outputs_up_to(input: &str, filter: &str, most: usize) -> usize {
        let program = crate::Program::compile(filter, &[]).expect("the filter compiles");
        program
            .run_vals(crate::to_val(&parse(input)), Vec::new())
            .take(most)
            .count()
    }

    /// `fromjson` and `tonumber` over text that is not JSON never ended: each answered
    /// with an error, and again, for ever, with the app's worker thread (the one that
    /// runs every query) busy and its memory growing by a gigabyte a second.
    #[test]
    fn text_that_is_not_json_is_one_error_not_an_endless_stream_of_them() {
        for filter in ["fromjson", "tonumber", "toboolean"] {
            for input in [
                r#""abc""#,
                r#""N/A""#,
                r#""""#,
                r#"" ""#,
                r#""[1,""#,
                r#""{""#,
                r#""tru""#,
                r#""\"a""#,
                r#""<html>""#,
                r#""1 2""#,
                r#""null null""#,
            ] {
                assert_eq!(outputs_up_to(input, filter, 1000), 1, "{input} | {filter}");
            }
        }
        // what was typed in the query box to get round it works too, now
        gives(r#"["1","x","2"]"#, "[.[] | tonumber?]", &["[1,2]"]);
        gives(
            r#"["1","x","2"]"#,
            "[.[] | try tonumber catch -1]",
            &["[1,-1,2]"],
        );
        gives(r#""abc""#, r#"try fromjson catch "bad""#, &[r#""bad""#]);
        gives(r#""abc""#, "(tonumber? // 0)", &["0"]);
    }

    #[test]
    #[rustfmt::skip]
    fn fromjson_reads_one_value_of_json_text() {
        // (what jq 1.8.1 gives for each)
        gives(r#""null""#, "fromjson", &["null"]);
        gives(r#""true""#, "fromjson", &["true"]);
        gives(r#""[1,2]""#, "fromjson", &["[1,2]"]);
        gives(r#""[ 1 , 2 ]""#, "fromjson", &["[1,2]"]);
        gives(r#""{\"a\":[1,{\"b\":null}]}""#, "fromjson", &[r#"{"a":[1,{"b":null}]}"#]);
        gives(r#""{\"a\":1,\"a\":2}""#, "fromjson", &[r#"{"a":2}"#]);
        gives(r#"" 1 ""#, "fromjson", &["1"]);
        gives(r#""\r\n1\r\n""#, "fromjson", &["1"]);
        gives(r#""\"\\u00e9\"""#, "fromjson", &[r#""é""#]);
        gives(r#""\"\\ud83d\\ude00\"""#, "fromjson", &[r#""😀""#]);
        gives(r#""[]""#, "fromjson", &["[]"]);
        gives(r#""{}""#, "fromjson", &["{}"]);
        gives(r#""100000000000000000000""#, "fromjson", &["100000000000000000000"]);
        gives(r#""123456789012345678901234567890""#, "fromjson", &["123456789012345678901234567890"]);
        gives(r#""0.1""#, "fromjson", &["0.1"]);
        gives(r#""1.50""#, "fromjson", &["1.50"]);
        gives(r#""[1,[2,[3]]]""#, "fromjson | flatten", &["[1,2,3]"]);
        gives(r#"["1","[2]"]"#, "map(fromjson)", &["[1,[2]]"]);
        gives(r#""[1]""#, "fromjson | tojson", &[r#""[1]""#]);
        // as many values as there are, not one of them
        gives(r#"[1,{"a":"b"}]"#, "tojson | fromjson", &[r#"[1,{"a":"b"}]"#]);
    }

    /// Where jq fails: no text, two values, half a value, something else.
    #[test]
    fn fromjson_fails_where_jq_fails() {
        for text in [
            "",
            " ",
            "\n",
            "abc",
            "N/A",
            "n/a",
            "hello world",
            "<html>",
            "tru",
            "truee",
            "nul",
            "True",
            "NULL",
            "undefined",
            "None",
            "-",
            "+",
            ".",
            ",",
            ":",
            "[",
            "]",
            "{",
            "}",
            "\"",
            "\\",
            "'a'",
            "{'a':1}",
            "{a:1}",
            "[1,]",
            "[,1]",
            "[1 2]",
            "[1,2",
            "1,2]",
            "{\"a\":1,}",
            "{\"a\"}",
            "{\"a\":}",
            "{\"a\":1 \"b\":2}",
            "\"a",
            "a\"",
            "\"\\q\"",
            "\"\\x41\"",
            "\"\\ud83d\"",
            "\"a\nb\"",
            "0x10",
            "1_000",
            "--1",
            "1-",
            "1e",
            "1e+",
            "e1",
            "1 2",
            "1\n2",
            "[1] [2]",
            "\"a\" \"b\"",
            "null null",
            "true false",
            "[1,2,3]x",
            "x[1,2,3]",
            "٣",
        ] {
            let input = Value::String(text.to_owned());
            assert!(run(&input, "fromjson").is_err(), "{text:?}");
        }
        for input in ["1", "null", "true", "[1]", r#"{"a":1}"#] {
            assert_eq!(outputs_up_to(input, "fromjson", 10), 1, "{input}");
            fails(input, "fromjson");
        }
        assert_eq!(
            error_of("1", "fromjson"),
            "number (1) only strings can be parsed"
        );
        assert_eq!(
            error_of("null", "fromjson"),
            "null (null) only strings can be parsed"
        );
    }

    #[test]
    fn the_error_of_a_fromjson_says_what_text_it_was_about() {
        let message = error_of(r#""abc""#, "fromjson");
        assert!(message.ends_with("(while parsing 'abc')"), "{message}");
        // a long text is quoted from its start, not all of it
        let long = Value::String(format!("x{}", "y".repeat(1_000_000)));
        let message = run(&long, "fromjson").unwrap_err();
        assert!(message.len() < 200, "{}", message.len());
        assert!(message.ends_with("...')"), "{message}");
    }

    #[test]
    fn fromjson_refuses_text_nested_deeper_than_it_can_read() {
        let nest = |n: usize| Value::String(format!("{}{}", "[".repeat(n), "]".repeat(n)));
        assert_eq!(run(&nest(1000), "fromjson | length"), Ok(vec![json!(1)]));
        let message = run(&nest(1001), "fromjson").unwrap_err();
        assert!(
            message.starts_with("Exceeds depth limit for parsing"),
            "{message}"
        );
        // not read at all: it is far more than a stack has room for
        assert!(run(&nest(3_000_000), "fromjson").is_err());
        assert!(run(&Value::String("{\"a\":".repeat(3_000_000)), "fromjson").is_err());
        // brackets inside a string, or a comment, are not a nest
        let in_a_string = Value::String(format!("[\"{}\"]", "[".repeat(3_000_000)));
        assert_eq!(run(&in_a_string, "fromjson | length"), Ok(vec![json!(1)]));
    }

    /// What `tonumber` takes is what jq's takes (compared with jq 1.8.1 on 4,121 strings
    /// made of the parts of a number).
    #[test]
    #[rustfmt::skip]
    fn tonumber_reads_what_jq_reads() {
        gives(r#""1""#, "tonumber", &["1"]);
        gives(r#""-1""#, "tonumber", &["-1"]);
        gives(r#""007""#, "tonumber", &["7"]);
        gives(r#""+5""#, "tonumber", &["5"]);
        gives(r#""5.""#, "tonumber", &["5"]);
        gives(r#"".5""#, "tonumber", &["0.5"]);
        gives(r#""-.5""#, "tonumber", &["-0.5"]);
        gives(r#""1.50""#, "tonumber", &["1.50"]);
        gives(r#""0.1""#, "tonumber", &["0.1"]);
        gives(r#""123456789012345678901234567890""#, "tonumber", &["123456789012345678901234567890"]);
        gives(r#""9223372036854775808""#, "tonumber", &["9223372036854775808"]);
        gives(r#""1e3""#, "tonumber == 1000", &["true"]);
        gives(r#""1E3""#, "tonumber == 1000", &["true"]);
        gives(r#""1.5e-2""#, "tonumber == 0.015", &["true"]);
        gives(r#""1.e2""#, "tonumber == 100", &["true"]);
        gives(r#"".5e1""#, "tonumber == 5", &["true"]);
        gives(r#""nan""#, "tonumber | isnan", &["true"]);
        gives(r#""-NaN""#, "tonumber | isnan", &["true"]);
        gives(r#""Infinity""#, "tonumber == infinite", &["true"]);
        gives(r#""-inf""#, "tonumber == -infinite", &["true"]);
        gives(r#""infinity""#, "tonumber | isinfinite", &["true"]);
        gives("5", "tonumber", &["5"]);
        gives("1.5", "tonumber", &["1.5"]);
        gives(r#"["1","2.5","-3"]"#, "map(tonumber) | add", &["0.5"]);
        gives(r#"{"n":"42"}"#, ".n | tonumber + 1", &["43"]);
        gives(r#""12""#, "tonumber | type", &[r#""number""#]);
        // a number that is a text of a number, to be compared as one
        gives(r#"["10","9","100"]"#, "map(tonumber) | sort", &["[9,10,100]"]);
    }

    #[test]
    fn tonumber_fails_where_jq_fails() {
        for text in [
            "", " ", "-", "+", ".", "-.", "e5", ".e1", "1e", "1e+", "1e1.5", "ee1", "1.5.5", "..5",
            "--1", "+-1", "1-", "0x10", "1_000", "1,5", "1 0", " 1", "1 ", "\t1", "1\n", "\u{a0}1",
            "٣", "１", "−1", "null", "true", "[1]", "{}", "\"1\"", "nanx", "infinit", "abc", "N/A",
            "5%", "$5", "5px", "1/2", "1e1e1",
        ] {
            let input = Value::String(text.to_owned());
            assert!(run(&input, "tonumber").is_err(), "{text:?}");
        }
        for input in ["null", "true", "[1]", r#"{"a":1}"#, "[]"] {
            fails(input, "tonumber");
        }
    }

    /// Worded as jq words it (it cuts the text shorter).
    #[test]
    fn the_error_of_a_tonumber_is_worded_as_jq_words_it() {
        assert_eq!(
            error_of(r#""abc""#, "tonumber"),
            r#"string ("abc") cannot be parsed as a number"#
        );
        assert_eq!(
            error_of("null", "tonumber"),
            "null (null) cannot be parsed as a number"
        );
        assert_eq!(
            error_of("[1]", "tonumber"),
            "array ([1]) cannot be parsed as a number"
        );
        assert_eq!(
            error_of(r#"{"a":1}"#, "tonumber"),
            r#"object ({"a":1}) cannot be parsed as a number"#
        );
        assert_eq!(
            error_of(r#"" 5""#, "tonumber"),
            r#"string (" 5") cannot be parsed as a number"#
        );
        let message = error_of(&format!("\"{}\"", "x".repeat(10_000)), "tonumber");
        assert!(message.len() < 100, "{message}");
    }

    #[test]
    fn toboolean_reads_json_that_is_a_boolean() {
        gives(r#""true""#, "toboolean", &["true"]);
        gives(r#""false""#, "toboolean", &["false"]);
        gives("true", "toboolean", &["true"]);
        fails(r#""abc""#, "toboolean");
        fails(r#""1""#, "toboolean");
        fails(r#""[true]""#, "toboolean");
    }

    #[test]
    #[rustfmt::skip]
    fn tojson_writes_a_value_on_one_line() {
        gives("null", "tojson", &[r#""null""#]);
        gives(r#"[1,"a",null,{"b":[true]}]"#, "tojson", &[r#""[1,\"a\",null,{\"b\":[true]}]""#]);
        gives(r#""é\n""#, "tojson", &[r#""\"é\\n\"""#]);
        gives(r#"{"b":1,"a":2}"#, "tojson", &[r#""{\"b\":1,\"a\":2}""#]);
        gives("100000000000000000000", "tojson", &[r#""100000000000000000000""#]);
        gives("1.50", "tojson", &[r#""1.50""#]);
        gives(r#"[1,[2]]"#, "@json", &[r#""[1,[2]]""#]);
        gives(r#"{"a":[1]}"#, "tojson | fromjson", &[r#"{"a":[1]}"#]);
        gives(r#"{"a":[1]}"#, r#""x\(. | tojson)""#, &[r#""x{\"a\":[1]}""#]);
    }

    /// A program makes a value as deep as it likes, and `tojson` of it stops where jq
    /// does. Run on a thread for the work, for such a value is deeper than the stack of
    /// a test thread can free.
    #[test]
    fn tojson_of_a_value_nested_deeper_than_jq_prints_is_cut_as_jq_cuts_it() {
        let lengths = on_a_query_thread(|| {
            let len = |filter: &str| {
                run(&Value::Null, filter).map(|outputs| outputs[0].as_u64().expect("a length"))
            };
            // (lengths of jq 1.8.1's `tojson`: 256 arrays round a number are all
            // there, 257 are not)
            [
                len("reduce range(256) as $i (1; [.]) | tojson | length"),
                len("reduce range(257) as $i (1; [.]) | tojson | length"),
                len("reduce range(300) as $i (1; [.]) | tojson | length"),
                len("reduce range(5000) as $i ([]; [.]) | tojson | length"),
                len("reduce range(50000) as $i ([]; [.]) | tojson | length"),
                len("reduce range(257) as $i (1; {a: .}) | tojson | length"),
                len("reduce range(50000) as $i (1; {a: .}) | tojson | length"),
                len(r#"reduce range(50000) as $i ([]; [.]) | "\(tojson | length)" | length"#),
            ]
        });
        assert_eq!(
            lengths,
            [
                Ok(513),
                Ok(533),
                Ok(533),
                Ok(533),
                Ok(533),
                Ok(1561),
                Ok(1561),
                Ok(3)
            ]
        );
    }

    #[test]
    fn a_text_is_cut_where_it_is_too_deep_and_says_so() {
        let text = on_a_query_thread(|| {
            run(&Value::Null, "reduce range(1000) as $i (1; [.]) | tojson")
                .map(|outputs| outputs[0].as_str().expect("text").to_owned())
        })
        .unwrap();
        assert!(text.contains("<skipped: too deep>"), "{}", &text[..600]);
        assert!(text.starts_with("[[[[") && text.ends_with("]]]]"));
        assert!(!text.contains("1]"));
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

    /// jq makes the places a value is put into: an object for a key, an array for an index. (jaq's
    /// assignments failed on `null`, which is what is under every key that is not there yet.)
    #[test]
    #[rustfmt::skip]
    fn writing_into_null_or_a_missing_key_makes_the_objects_and_arrays() {
        gives(r#"null"#, r#".a = 1"#, &[r#"{"a":1}"#]);
        gives(r#"null"#, r#".a.b = 1"#, &[r#"{"a":{"b":1}}"#]);
        gives(r#"null"#, r#".a.b.c = 1"#, &[r#"{"a":{"b":{"c":1}}}"#]);
        gives(r#"null"#, r#".[0] = 1"#, &[r#"[1]"#]);
        gives(r#"null"#, r#".[2] = 1"#, &[r#"[null,null,1]"#]);
        gives(r#"null"#, r#".a[0] = 1"#, &[r#"{"a":[1]}"#]);
        gives(r#"null"#, r#".a[2].b = 1"#, &[r#"{"a":[null,null,{"b":1}]}"#]);
        gives(r#"null"#, r#".[0].a = 1"#, &[r#"[{"a":1}]"#]);
        gives(r#"null"#, r#".[1][1] = 1"#, &[r#"[null,[null,1]]"#]);
        gives(r#"{"x":1}"#, r#".a.b = 1"#, &[r#"{"x":1,"a":{"b":1}}"#]);
        gives(r#"{"a":null}"#, r#".a.b = 1"#, &[r#"{"a":{"b":1}}"#]);
        gives(r#"{"a":{}}"#, r#".a.b = 1"#, &[r#"{"a":{"b":1}}"#]);
        gives(r#"{"metadata":{"name":"web"}}"#, r#".metadata.labels.env = "prod""#, &[r#"{"metadata":{"name":"web","labels":{"env":"prod"}}}"#]);
        gives(r#"{"metadata":{}}"#, r#".metadata.annotations["k"] = "v""#, &[r#"{"metadata":{"annotations":{"k":"v"}}}"#]);
        gives(r#"{"name":"a"}"#, r#".counts[.name] += 1"#, &[r#"{"name":"a","counts":{"a":1}}"#]);
        gives(r#"{"name":"a","counts":{"a":2}}"#, r#".counts[.name] += 1"#, &[r#"{"name":"a","counts":{"a":3}}"#]);
        gives(r#"{"a":1}"#, r#".b.c //= 5"#, &[r#"{"a":1,"b":{"c":5}}"#]);
        gives(r#"{"b":{"c":1}}"#, r#".b.c //= 5"#, &[r#"{"b":{"c":1}}"#]);
        gives(r#"{"b":{"c":false}}"#, r#".b.c //= 5"#, &[r#"{"b":{"c":5}}"#]);
        gives(r#"null"#, r#".a += 1"#, &[r#"{"a":1}"#]);
        gives(r#"null"#, r#".a //= 3"#, &[r#"{"a":3}"#]);
        gives(r#"null"#, r#".a |= 5"#, &[r#"{"a":5}"#]);
        gives(r#"null"#, r#".a |= . + 1"#, &[r#"{"a":1}"#]);
        gives(r#"null"#, r#".a = (.a // 0) + 1"#, &[r#"{"a":1}"#]);
        gives(r#"null"#, r#".[0] += 1"#, &[r#"[1]"#]);
        gives(r#"{}"#, r#".a.b |= 1"#, &[r#"{"a":{"b":1}}"#]);
        gives(r#"{}"#, r#".a.b += 1"#, &[r#"{"a":{"b":1}}"#]);
        gives(r#"{}"#, r#".a[0] += 1"#, &[r#"{"a":[1]}"#]);
        gives(r#"null"#, r#"(.a, .b) = 1"#, &[r#"{"a":1,"b":1}"#]);
        gives(r#"null"#, r#"(.a, .b.c) = 1"#, &[r#"{"a":1,"b":{"c":1}}"#]);
        gives(r#"{"b":2}"#, r#".a = .b"#, &[r#"{"b":2,"a":2}"#]);
        gives(r#"{"c":3}"#, r#".a.b = .c"#, &[r#"{"c":3,"a":{"b":3}}"#]);
        gives(r#"null"#, r#".["a b"] = 1"#, &[r#"{"a b":1}"#]);
        gives(r#"null"#, r#"."a-b".c = 1"#, &[r#"{"a-b":{"c":1}}"#]);
        gives(r#"{}"#, r#".spec.template.spec.containers[0].image = "nginx""#, &[r#"{"spec":{"template":{"spec":{"containers":[{"image":"nginx"}]}}}}"#]);
        gives(r#"{}"#, r#".tags += ["new"]"#, &[r#"{"tags":["new"]}"#]);
        gives(r#"null"#, r#".tags += ["new"]"#, &[r#"{"tags":["new"]}"#]);
        gives(r#"{"type":"x"}"#, r#".stats[.type] += 1"#, &[r#"{"type":"x","stats":{"x":1}}"#]);
        gives(r#"[{},{"a":null},{"a":{"b":0}}]"#, r#"[.[] | .a.b = 1]"#, &[r#"[{"a":{"b":1}},{"a":{"b":1}},{"a":{"b":1}}]"#]);
        gives(r#"{"a":1}"#, r#". + {c: 3} | .d.e = 4"#, &[r#"{"a":1,"c":3,"d":{"e":4}}"#]);
        gives(r#"{}"#, r#".a.b.c |= (. // 0) + 1"#, &[r#"{"a":{"b":{"c":1}}}"#]);
        gives(r#"{"a":1}"#, r#".a = null"#, &[r#"{"a":null}"#]);
        gives(r#"{}"#, r#".a = null"#, &[r#"{"a":null}"#]);
        gives(r#"{}"#, r#".a |= null"#, &[r#"{"a":null}"#]);
        gives(r#"null"#, r#"def inc: .count += 1; inc | inc"#, &[r#"{"count":2}"#]);
        gives(r#"null"#, r#"reduce range(3) as $i (null; .[$i] = $i)"#, &[r#"[0,1,2]"#]);
        gives(r#"null"#, r#"reduce range(5) as $i ([]; .[$i * 2] = $i)"#, &[r#"[0,null,1,null,2,null,3,null,4]"#]);
        gives(r#"["a","b","a"]"#, r#"reduce .[] as $x (null; .[$x] += 1)"#, &[r#"{"a":2,"b":1}"#]);
        gives(r#"[{"k":"a","v":1},{"k":"b","v":2},{"k":"a","v":3}]"#, r#"reduce .[] as $e ({}; .[$e.k].n += $e.v)"#, &[r#"{"a":{"n":4},"b":{"n":2}}"#]);
        gives(r#"[{"k":"a","i":1,"v":"x"},{"k":"a","i":0,"v":"y"}]"#, r#"reduce .[] as $x (null; .[$x.k][$x.i] = $x.v)"#, &[r#"{"a":["y","x"]}"#]);
    }

    /// An item put after the end makes the array longer, with `null` in the gap; a slice of `null` is
    /// the array put there.
    #[test]
    #[rustfmt::skip]
    fn writing_past_the_end_of_an_array_makes_it_longer_with_nulls() {
        gives(r#"[]"#, r#".[0] = 9"#, &[r#"[9]"#]);
        gives(r#"[]"#, r#".[1] = 9"#, &[r#"[null,9]"#]);
        gives(r#"[1,2,3]"#, r#".[3] = 9"#, &[r#"[1,2,3,9]"#]);
        gives(r#"[1,2,3]"#, r#".[5] = 9"#, &[r#"[1,2,3,null,null,9]"#]);
        gives(r#"[1,2,3]"#, r#".[-1] = 9"#, &[r#"[1,2,9]"#]);
        gives(r#"[1,2,3]"#, r#".[-3] = 9"#, &[r#"[9,2,3]"#]);
        gives(r#"[1,2]"#, r#".[2] = 9"#, &[r#"[1,2,9]"#]);
        gives(r#"[[1],[2]]"#, r#".[2][0] = 9"#, &[r#"[[1],[2],[9]]"#]);
        gives(r#"[1,2]"#, r#".[2].a = 9"#, &[r#"[1,2,{"a":9}]"#]);
        gives(r#"[1,null]"#, r#".[1].a = 9"#, &[r#"[1,{"a":9}]"#]);
        gives(r#"[1]"#, r#".[5] |= . + 1"#, &[r#"[1,null,null,null,null,1]"#]);
        gives(r#"[1]"#, r#".[5] += 1"#, &[r#"[1,null,null,null,null,1]"#]);
        gives(r#"[1]"#, r#".[5] //= 7"#, &[r#"[1,null,null,null,null,7]"#]);
        gives(r#"[null]"#, r#".[0] //= 7"#, &[r#"[7]"#]);
        gives(r#"[1]"#, r#"(.[0], .[3]) = 9"#, &[r#"[9,null,null,9]"#]);
        gives(r#"[1,2,3]"#, r#".[1:] = [9]"#, &[r#"[1,9]"#]);
        gives(r#"[1,2,3]"#, r#".[5:] = [9]"#, &[r#"[1,2,3,9]"#]);
        gives(r#"[1,2,3]"#, r#".[:0] = [9]"#, &[r#"[9,1,2,3]"#]);
        gives(r#"[1,2,3]"#, r#".[1:] = []"#, &[r#"[1]"#]);
        gives(r#"[1,2,3]"#, r#".[1:2] |= map(. * 10)"#, &[r#"[1,20,3]"#]);
        gives(r#"null"#, r#".[1:] = [9]"#, &[r#"[9]"#]);
        gives(r#"null"#, r#".[:2] = [9]"#, &[r#"[9]"#]);
        gives(r#"null"#, r#".[1:3] += [9]"#, &[r#"[9]"#]);
        gives(r#"null"#, r#".[1:] = []"#, &[r#"[]"#]);
        gives(r#"{"a":1}"#, r#".b[1:] |= [9]"#, &[r#"{"a":1,"b":[9]}"#]);
        gives(r#"[]"#, r#".[100000] = 1 | length"#, &[r#"100001"#]);
        gives(r#"[1]"#, r#".[199999] = 1 | length"#, &[r#"200000"#]);
        gives(r#"[1,2,3]"#, r#".[3] = 9 | length"#, &[r#"4"#]);
        gives(r#"[[1],[2]]"#, r#".[1] = 9"#, &[r#"[[1],9]"#]);
    }

    /// `setpath` goes through `null` and past the end; `pick`, which jq writes with it, then gives an
    /// array for the indexes of an array (jaq's gave an object with the numbers as keys).
    #[test]
    #[rustfmt::skip]
    fn setpath_and_pick_make_the_levels_they_set() {
        gives(r#"null"#, r#"setpath(["a"]; 1)"#, &[r#"{"a":1}"#]);
        gives(r#"null"#, r#"setpath(["a","b"]; 1)"#, &[r#"{"a":{"b":1}}"#]);
        gives(r#"null"#, r#"setpath([0,"a"]; 1)"#, &[r#"[{"a":1}]"#]);
        gives(r#"null"#, r#"setpath(["a",0]; 1)"#, &[r#"{"a":[1]}"#]);
        gives(r#"[0]"#, r#"setpath([2]; 1)"#, &[r#"[0,null,1]"#]);
        gives(r#"null"#, r#"setpath([]; 5)"#, &[r#"5"#]);
        gives(r#"{"x":1}"#, r#"setpath(["a","b"]; 1)"#, &[r#"{"x":1,"a":{"b":1}}"#]);
        gives(r#"null"#, r#"setpath([2, {"start":1,"end":3}, 3]; 2)"#, &[r#"[null,null,[null,null,null,2]]"#]);
        gives(r#"{"a":{"b":1,"c":2},"d":3}"#, r#"pick(.a.b)"#, &[r#"{"a":{"b":1}}"#]);
        gives(r#"[1,2,3]"#, r#"pick(.[0])"#, &[r#"[1]"#]);
        gives(r#"[1,2,3]"#, r#"pick(first)"#, &[r#"[1]"#]);
        gives(r#"[1,2,3,4]"#, r#"pick(.[1], .[3])"#, &[r#"[null,2,null,4]"#]);
        gives(r#"{"a":1,"b":{"c":2,"d":3}}"#, r#"pick(.a, .b.c)"#, &[r#"{"a":1,"b":{"c":2}}"#]);
        gives(r#"{"a":1}"#, r#"pick(.x)"#, &[r#"{"x":null}"#]);
        gives(r#"{"a":[1,2,3]}"#, r#"pick(.a[1])"#, &[r#"{"a":[null,2]}"#]);
        gives(r#"null"#, r#"pick(.a)"#, &[r#"{"a":null}"#]);
        gives(r#"{"a":[1,{"b":2}],"c":null}"#, r#"fromstream(tostream)"#, &[r#"{"a":[1,{"b":2}],"c":null}"#]);
    }

    /// What cannot be written is an error where it is one in jq: through a value that is not an object or
    /// an array, an object at a number, an array at a name, before the start, and so on.
    #[test]
    #[rustfmt::skip]
    fn writing_where_jq_refuses_is_an_error() {
        fails(r#"{"a":1}"#, r#".a.b = 1"#);
        fails(r#"[1]"#, r#".a = 1"#);
        fails(r#""x""#, r#".a = 1"#);
        fails(r#"5"#, r#".a = 1"#);
        fails(r#"true"#, r#".a = 1"#);
        fails(r#"{"a":1}"#, r#".[0] = 1"#);
        fails(r#"{"a":{"x":1}}"#, r#".a[0] = 1"#);
        fails(r#"{"a":"s"}"#, r#".a[0] = 1"#);
        fails(r#"{"a":[1]}"#, r#".a.b = 1"#);
        fails(r#"null"#, r#".[true] = 1"#);
        fails(r#"null"#, r#".[null] = 1"#);
        fails(r#"null"#, r#".[[1]] = 1"#);
        fails(r#"[1,2,3]"#, r#".[-4] = 9"#);
        fails(r#"[]"#, r#".[-1] = 9"#);
        fails(r#"null"#, r#".[-1] = 9"#);
        fails(r#"[]"#, r#".[536870912] = 1"#);
        fails(r#"[1,2,3]"#, r#".[1:] = 5"#);
        fails(r#"null"#, r#".[1:] = 5"#);
        fails(r#"[1,2]"#, r#".[1].a = 9"#);
        fails(r#"{"a":1}"#, r#"setpath(["a","b"]; 1)"#);
        fails(r#"{"a":1}"#, r#"setpath([1]; 1)"#);
        fails(r#"null"#, r#"setpath("a"; 1)"#);
        fails(r#"[1,2]"#, r#"setpath([-3]; 1)"#);
        fails(r#"[1,2,3]"#, r#"pick(.[-1])"#);
        fails(r#"null"#, r#".[] = 1"#);
        fails(r#"null"#, r#".a -= 1"#);
    }

    /// `|= empty` takes the place away where there is one, and does nothing where there is none.
    #[test]
    #[rustfmt::skip]
    fn an_update_that_gives_nothing_changes_nothing() {
        gives(r#"[1,2,3]"#, r#".[5] |= empty"#, &[r#"[1,2,3]"#]);
        gives(r#"[]"#, r#".[0] |= empty"#, &[r#"[]"#]);
        gives(r#"[1,2,3]"#, r#".[1] |= empty"#, &[r#"[1,3]"#]);
        gives(r#"null"#, r#".a |= empty"#, &[r#"null"#]);
        gives(r#"null"#, r#".[1:] |= empty"#, &[r#"null"#]);
        gives(r#"[1,2,3]"#, r#"(.[] | select(. > 1)) |= empty"#, &[r#"[1]"#]);
        gives(r#"[1,2,3]"#, r#".[] |= empty"#, &[r#"[]"#]);
        gives(r#"{"a":1,"b":2}"#, r#".[] |= empty"#, &[r#"{}"#]);
        gives(r#"{"a":1}"#, r#"map_values(empty)"#, &[r#"{}"#]);
        gives(r#"[1,2,3]"#, r#"map_values(select(. > 1))"#, &[r#"[2,3]"#]);
        gives(r#"{"a":1,"b":2,"c":3}"#, r#"map_values(select(. > 1))"#, &[r#"{"b":2,"c":3}"#]);
        gives(r#"{"a":[1,2,3]}"#, r#".a[1:] |= empty"#, &[r#"{"a":[1]}"#]);
    }

    /// Every path is looked for in the value as it was, not as the paths before it left it (jaq deleted
    /// them one after the other, and `del(.a[0], .a[1])` took the wrong item). The cases are what jq 1.8.1
    /// gives.
    #[test]
    #[rustfmt::skip]
    fn del_and_delpaths_take_all_the_paths_in_one_go() {
        gives(r#"[1,2,3]"#, r#"del(.[1,2])"#, &[r#"[1]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[0,2])"#, &[r#"[2]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[1,1])"#, &[r#"[1,3]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[5])"#, &[r#"[1,2,3]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[-1])"#, &[r#"[1,2]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[-5])"#, &[r#"[1,2,3]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[0], .[0])"#, &[r#"[2,3]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[2], .[0])"#, &[r#"[2]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[])"#, &[r#"[]"#]);
        gives(r#"{"a":1,"b":2}"#, r#"del(.[])"#, &[r#"{}"#]);
        gives(r#"[1,2,3]"#, r#"del(.[] | select(. > 1))"#, &[r#"[1]"#]);
        gives(r#"[1,2,3,2]"#, r#"del(.[] | select(. == 2))"#, &[r#"[1,3]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[] | select(. > 5))"#, &[r#"[1,2,3]"#]);
        gives(r#"{"a":1,"b":2,"c":3}"#, r#"del(.a)"#, &[r#"{"b":2,"c":3}"#]);
        gives(r#"{"a":1,"b":2,"c":3}"#, r#"del(.b)"#, &[r#"{"a":1,"c":3}"#]);
        gives(r#"{"a":1,"b":2,"c":3}"#, r#"del(.c)"#, &[r#"{"a":1,"b":2}"#]);
        gives(r#"{"a":1,"b":2,"c":3}"#, r#"del(.a, .c)"#, &[r#"{"b":2}"#]);
        gives(r#"{"a":1}"#, r#"del(.x)"#, &[r#"{"a":1}"#]);
        gives(r#"{"a":{"b":1,"c":2}}"#, r#"del(.a.b)"#, &[r#"{"a":{"c":2}}"#]);
        gives(r#"{"x":1}"#, r#"del(.a.b)"#, &[r#"{"x":1}"#]);
        gives(r#"{"a":null}"#, r#"del(.a.b)"#, &[r#"{"a":null}"#]);
        gives(r#"{"a":[1,2,3]}"#, r#"del(.a[0])"#, &[r#"{"a":[2,3]}"#]);
        gives(r#"{"a":[1,2,3]}"#, r#"del(.a[0], .a[1])"#, &[r#"{"a":[3]}"#]);
        gives(r#"{"a":[1,2,3]}"#, r#"del(.a[1], .a[0])"#, &[r#"{"a":[3]}"#]);
        gives(r#"{"a":[1,2,3]}"#, r#"del(.a[0,1])"#, &[r#"{"a":[3]}"#]);
        gives(r#"[{"a":1,"b":2}]"#, r#"del(.[0].a)"#, &[r#"[{"b":2}]"#]);
        gives(r#"[{"a":1,"b":2},{"a":3}]"#, r#"del(.[].a)"#, &[r#"[{"b":2},{}]"#]);
        gives(r#"[[1,2,3],[4,5,6]]"#, r#"del(.[][0])"#, &[r#"[[2,3],[5,6]]"#]);
        gives(r#"[1,[1,2],{"a":1}]"#, r#"del(.. | select(. == 1))"#, &[r#"[[2],{}]"#]);
        gives(r#"[1]"#, r#"del(..)"#, &[r#"null"#]);
        gives(r#"[1]"#, r#"del(.)"#, &[r#"null"#]);
        gives(r#"[1]"#, r#"del(empty)"#, &[r#"[1]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[1:])"#, &[r#"[1]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[:1])"#, &[r#"[2,3]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[1:2])"#, &[r#"[1,3]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[-2:])"#, &[r#"[1]"#]);
        gives(r#"[1,2,3]"#, r#"del(.[1:], .[0])"#, &[r#"[]"#]);
        gives(r#"[1,2,3,4]"#, r#"del(.[1:3][0])"#, &[r#"[1,3,4]"#]);
        gives(r#"[1]"#, r#"del(.[5:])"#, &[r#"[1]"#]);
        gives(r#"[1,2,3]"#, r#"del(first(.[] | select(. > 1)))"#, &[r#"[1,3]"#]);
        gives(r#"[1,2,3]"#, r#"del(limit(2; .[]))"#, &[r#"[3]"#]);
        gives(r#"null"#, r#"del(.a)"#, &[r#"null"#]);
        gives(r#"{"a b":1,"c":2}"#, r#"del(."a b")"#, &[r#"{"c":2}"#]);
        gives(r#"{"a":1,"b":2,"c":3}"#, r#"del(.["a","b"])"#, &[r#"{"c":3}"#]);
        gives(r#"{"tmp":1,"a":{"tmp":2,"b":3}}"#, r#"del(.. | .tmp?)"#, &[r#"{"a":{"b":3}}"#]);
        gives(r#"[{"id":1},{"id":2},{"id":3}]"#, r#"del(.[] | select(.id == 2))"#, &[r#"[{"id":1},{"id":3}]"#]);
        gives(r#"{"items":[{"done":true},{"done":false}]}"#, r#"del(.items[] | select(.done))"#, &[r#"{"items":[{"done":false}]}"#]);
        gives(r#"{"x":1,"y":{"x":2,"z":3}}"#, r#"walk(if type == "object" then del(.x) else . end)"#, &[r#"{"y":{"z":3}}"#]);
        gives(r#"[1,2,3,4]"#, r#"del(.[0:2], .[1])"#, &[r#"[3,4]"#]);
        gives(r#"[1,2,3,4]"#, r#"del(.[1:2], .[1:3])"#, &[r#"[1,4]"#]);
        gives(r#"{"a":1,"b":2}"#, r#"del(.a, .a)"#, &[r#"{"b":2}"#]);
        gives(r#"{"a":{"b":1},"c":2}"#, r#"del(.a.b, .a)"#, &[r#"{"c":2}"#]);
        gives(r#"{"a":{"b":1},"c":2}"#, r#"del(.a, .a.b)"#, &[r#"{"c":2}"#]);
        gives(r#"{"a":[0,{"b":1},2]}"#, r#"del(.a[1], .a[1].b)"#, &[r#"{"a":[0,2]}"#]);
        gives(r#"[1,2,3]"#, r#"delpaths([[0],[0]])"#, &[r#"[2,3]"#]);
        gives(r#"[1,2,3]"#, r#"delpaths([[0],[1]])"#, &[r#"[3]"#]);
        gives(r#"{"a":1,"b":2,"c":3}"#, r#"delpaths([["a"],["b"]])"#, &[r#"{"c":3}"#]);
        gives(r#"[1]"#, r#"delpaths([])"#, &[r#"[1]"#]);
        gives(r#"[1]"#, r#"delpaths([[]])"#, &[r#"null"#]);
        gives(r#"[{"a":1,"b":2}]"#, r#"delpaths([[0,"a"]])"#, &[r#"[{"b":2}]"#]);
        gives(r#"{"a":[1,2,3]}"#, r#"delpaths([["a",0],["a",1]])"#, &[r#"{"a":[3]}"#]);
        gives(r#"null"#, r#"delpaths([["a"]])"#, &[r#"null"#]);
        gives(r#"{"a":[1,2,3],"b":1,"c":2}"#, r#"delpaths([["a",0],["a",1],["b"]])"#, &[r#"{"a":[3],"c":2}"#]);
        gives(r#"{"a":{"b":1}}"#, r#"delpaths([["a"],["a","b"]])"#, &[r#"{}"#]);
        gives(r#"[1,2,3]"#, r#"delpaths([[-1],[0]])"#, &[r#"[2]"#]);
        gives(r#"[]"#, r#"delpaths([[0]])"#, &[r#"[]"#]);
        gives(r#"[1,2,3]"#, r#"delpaths([[0],[0],[1]])"#, &[r#"[3]"#]);
        gives(r#"{"a":1,"b":{"c":2,"d":"x"}}"#, r#"delpaths([paths(type == "number")])"#, &[r#"{"b":{"d":"x"}}"#]);
        gives(r#"[1,2,3]"#, r#"delpaths([[{"start":1,"end":null}]])"#, &[r#"[1]"#]);
        gives(r#"[1,2,3]"#, r#"delpaths([[1.5]])"#, &[r#"[1,3]"#]);
        gives(r#"[1,2,3]"#, r#"delpaths([[-1.5]])"#, &[r#"[1,2]"#]);
        gives(r#"[1,2,3]"#, r#"delpaths([[-0.5]])"#, &[r#"[1,2,3]"#]);
        gives(r#"[1,2,3,4,5]"#, r#"delpaths([[{"start":1.2,"end":3.5}]])"#, &[r#"[1,5]"#]);
        gives(r#"[1,2,3,4,5]"#, r#"delpaths([[{"start":-2.5,"end":null}]])"#, &[r#"[1,2]"#]);
        gives(r#"[1,2,3,4,5]"#, r#"delpaths([[{"start":3,"end":1}]])"#, &[r#"[1,2,3,4,5]"#]);
        gives(r#"[1,2,3]"#, r#"delpaths([[{"start":null,"end":null}]])"#, &[r#"[]"#]);
        gives(r#"[1,2,3,4,5]"#, r#"delpaths([[{"start":-9,"end":2}]])"#, &[r#"[3,4,5]"#]);
        gives(r#"[1,2,3,4,5]"#, r#"delpaths([[{"start":2,"end":99}]])"#, &[r#"[1,2]"#]);
        gives(r#"[[1,2,3],[4]]"#, r#"delpaths([[0,{"start":1,"end":null}],[1,0]])"#, &[r#"[[1],[]]"#]);
        // A place written two ways (`-1` and `1`) is not found to be the same one: jq looks at a key
        // as it is written, so the second path meets what the first has left, and so does this.
        gives(r#"["abc",[[null,true]]]"#, r#"delpaths([[0],[-1,0,0],[0],[1,0,0]])"#, &[r#"[[[]]]"#]);
        gives(r#"[[1.5,1.5,1.5,null]]"#, r#"del(.[-1][1], .[0][-4])"#, &[r#"[[1.5,1.5,null]]"#]);
        gives(r#"[[1.5,1.5,1.5,null]]"#, r#"del(.[0][1], .[0][-4])"#, &[r#"[[1.5,null]]"#]);
    }

    /// Where jq fails.
    #[test]
    #[rustfmt::skip]
    fn del_and_delpaths_refuse_what_jq_refuses() {
        fails(r#"{"a":1}"#, r#"del(.a.b)"#);
        fails(r#"[1]"#, r#"del(.a)"#);
        fails(r#"5"#, r#"del(.a)"#);
        fails(r#""abc""#, r#"del(.[0])"#);
        fails(r#""x""#, r#"del(.a)"#);
        fails(r#"[1]"#, r#"delpaths(1)"#);
        fails(r#"[1]"#, r#"delpaths([1])"#);
        fails(r#"{"a":1}"#, r#"delpaths([["a","b"]])"#);
        fails(r#"{"a":1}"#, r#"delpaths([[0]])"#);
        fails(r#"[1,2]"#, r#"delpaths([[[1]]])"#);
        fails(r#"[1,2]"#, r#"delpaths([["a"]])"#);
        fails(r#"{"a":1}"#, r#"delpaths([[true]])"#);
        fails(r#"{"a":1}"#, r#"delpaths([["a", 1]])"#);
        fails(r#"[1,2,3]"#, r#"delpaths([[{"start":"a","end":1}]])"#);
    }

    /// `scan` is `match` with the `g` flag: every match, as text, or as the array of its groups when
    /// the pattern has some. (Left out: a group that took no part is `null` in jq and not there in jaq's
    /// matches, and a pattern that can match nothing finds one match fewer: the regex engine's.)
    #[test]
    #[rustfmt::skip]
    fn scan_finds_every_match_and_the_groups_of_each() {
        gives(r#""a1b22c333""#, r#"[scan("[0-9]+")]"#, &[r#"["1","22","333"]"#]);
        gives(r#""ab ab""#, r#"[scan("(a)(b)")]"#, &[r#"[["a","b"],["a","b"]]"#]);
        gives(r#""aA""#, r#"[scan("a"; "i")]"#, &[r#"["a","A"]"#]);
        // without flags the case matters, and a flag that is not "i" does not change that
        gives(r#""aAa""#, r#"[scan("a")]"#, &[r#"["a","a"]"#]);
        gives(r#""aAa""#, r#"[scan("A")]"#, &[r#"["A"]"#]);
        gives(r#""aAa""#, r#"[scan("a"; "i")]"#, &[r#"["a","A","a"]"#]);
        gives(r#""aAa""#, r#"[scan("a"; "x")]"#, &[r#"["a","a"]"#]);
        gives(r#""bbb""#, r#"[scan("a")]"#, &[r#"[]"#]);
        gives(r#""abc""#, r#"[scan("")]"#, &[r#"["","","",""]"#]);
        gives(r#""hello world""#, r#"[scan("\\w+")]"#, &[r#"["hello","world"]"#]);
        gives(r#""ab cd""#, r#"[scan("[a-z]+"; "g")]"#, &[r#"["ab","cd"]"#]);
        gives(r#""xxxx""#, r#"[scan("x")] | length"#, &[r#"4"#]);
        gives(r#""a1b2""#, r#"scan("\\d")"#, &[r#""1""#, r#""2""#]);
        gives(r#""1a 2b""#, r#"[scan("(\\d)(\\w)")]"#, &[r#"[["1","a"],["2","b"]]"#]);
        gives(r#""1 2""#, r#"[scan("(?<n>\\d)")]"#, &[r#"[["1"],["2"]]"#]);
        gives(r#""aa""#, r#"[scan("a"; null)]"#, &[r#"["a","a"]"#]);
        gives(r#""aaa""#, r#"[scan("^a")]"#, &[r#"["a"]"#]);
        gives(r#""ab cd""#, r#"[scan("\\b\\w")]"#, &[r#"["a","c"]"#]);
        gives(r#""éé""#, r#"[scan("é")]"#, &[r#"["é","é"]"#]);
        gives(r#""a1b22c333""#, r#"[scan("[0-9]+")] | map(tonumber)"#, &[r#"[1,22,333]"#]);
        gives(r#""cost $12.50 and $3.99""#, r#"[scan("\\$\\d+\\.\\d{2}")]"#, &[r#"["$12.50","$3.99"]"#]);
        gives(r#"["a1","b22c3","x"]"#, r#"[.[] | [scan("\\d+")] | length]"#, &[r#"[1,2,0]"#]);
        gives(r#""a1b22""#, r#"first(scan("\\d+"))"#, &[r#""1""#]);
        gives(r#""a1b22""#, r#"[limit(1; scan("\\d+"))]"#, &[r#"["1"]"#]);
        gives(r#""aa b aaa""#, r#"[scan("a+")]"#, &[r#"["aa","aaa"]"#]);
        gives(r#""1 22""#, r#"[scan("\\d+"; "")]"#, &[r#"["1","22"]"#]);
        gives(r#""a1b""#, r#"[scan("(\\w)(\\d)")]"#, &[r#"[["a","1"]]"#]);
        gives(r#""abcabc""#, r#"[scan("a|b|c")]"#, &[r#"["a","b","c","a","b","c"]"#]);
        gives(r#""ac bd""#, r#"[scan("(a|b)(c|d)")]"#, &[r#"[["a","c"],["b","d"]]"#]);
        gives(r#""ab 12 cd""#, r#"[scan("[[:alpha:]]+")]"#, &[r#"["ab","cd"]"#]);
        fails(r#"5"#, r#"scan("a")"#);
        fails(r#"null"#, r#"[scan("a")]"#);
        fails(r#""a""#, r#"[scan(1)]"#);
    }

    /// `join` makes text of numbers and booleans, `null` is nothing, and an array or an object is an
    /// error (jaq made text of all of them, `null` too).
    #[test]
    #[rustfmt::skip]
    fn join_writes_null_as_nothing_and_refuses_arrays_and_objects() {
        gives(r#"["a",null,"b"]"#, r#"join(",")"#, &[r#""a,,b""#]);
        gives(r#"[1,true,false,null,"x"]"#, r#"join(",")"#, &[r#""1,true,false,,x""#]);
        gives(r#"[]"#, r#"join(",")"#, &[r#""""#]);
        gives(r#"["a"]"#, r#"join(",")"#, &[r#""a""#]);
        gives(r#"[null]"#, r#"join(",")"#, &[r#""""#]);
        gives(r#"[null,null]"#, r#"join(",")"#, &[r#"",""#]);
        gives(r#"{"a":"x","b":"y"}"#, r#"join(", ")"#, &[r#""x, y""#]);
        gives(r#"["a","b"]"#, r#"join(null)"#, &[r#""ab""#]);
        gives(r#"["a","b"]"#, r#"join("")"#, &[r#""ab""#]);
        gives(r#"[1.5,3,0.1]"#, r#"join("-")"#, &[r#""1.5-3-0.1""#]);
        gives(r#"["a","b"]"#, r#"join("\n")"#, &[r#""a\nb""#]);
        gives(r#"["é","日本"]"#, r#"join(",")"#, &[r#""é,日本""#]);
        gives(r#"[["a","b"],["c"]]"#, r#"[.[] | join("/")]"#, &[r#"["a/b","c"]"#]);
        gives(r#"[1,2,3]"#, r#"join(",")"#, &[r#""1,2,3""#]);
        gives(r#"[true,false]"#, r#"join(", ")"#, &[r#""true, false""#]);
        gives(r#"["a","b"]"#, r#"join(",", ";")"#, &[r#""a,b""#, r#""a;b""#]);
        gives(r#"[{"name":"a"},{"name":null},{"name":"c"}]"#, r#"map(.name) | join(", ")"#, &[r#""a, , c""#]);
        gives(r#""a,b,c""#, r#"split(",") | join(";")"#, &[r#""a;b;c""#]);
        gives(r#"["a",1,null,false,"b"]"#, r#"join(",")"#, &[r#""a,1,,false,b""#]);
        gives(r#"["a",null,"b"]"#, r#"join(", ") | length"#, &[r#"6"#]);
        gives(r#"null"#, r#"[range(20000) | tostring] | join(",") | length"#, &[r#"108889"#]);
        gives(r#"[1,"a b"]"#, r#"@sh"#, &[r#""1 'a b'""#]);
        fails(r#"[[1]]"#, r#"join(",")"#);
        fails(r#"[{"a":1}]"#, r#"join(",")"#);
        fails(r#"["a",["b"]]"#, r#"join(",")"#);
        fails(r#""abc""#, r#"join(",")"#);
        fails(r#"null"#, r#"join(",")"#);
        fails(r#"5"#, r#"join(",")"#);
        fails(r#"["a","b"]"#, r#"join(1)"#);
        fails(r#"[[]]"#, r#"join(",")"#);
        fails(r#"[{}]"#, r#"join(",")"#);
        fails(r#"["a","b"]"#, r#"join(true)"#);
    }

    /// `key`, `Key`, `name` or `Name` for the key, `value` or `Value` for the value; a key that is not
    /// text is an error (jaq made it a key all the same, and showed the object as text).
    #[test]
    #[rustfmt::skip]
    fn from_entries_takes_the_key_from_the_names_jq_takes_it_from() {
        gives(r#"[{"key":"a","value":1}]"#, r#"from_entries"#, &[r#"{"a":1}"#]);
        gives(r#"[{"Key":"a","Value":1}]"#, r#"from_entries"#, &[r#"{"a":1}"#]);
        gives(r#"[{"name":"a","value":1}]"#, r#"from_entries"#, &[r#"{"a":1}"#]);
        gives(r#"[{"Name":"a","Value":1}]"#, r#"from_entries"#, &[r#"{"a":1}"#]);
        gives(r#"[{"key":"a"}]"#, r#"from_entries"#, &[r#"{"a":null}"#]);
        gives(r#"[{"key":"a","value":null}]"#, r#"from_entries"#, &[r#"{"a":null}"#]);
        gives(r#"[{"key":"a","Value":2}]"#, r#"from_entries"#, &[r#"{"a":2}"#]);
        gives(r#"[{"key":"a","value":false}]"#, r#"from_entries"#, &[r#"{"a":false}"#]);
        gives(r#"[]"#, r#"from_entries"#, &[r#"{}"#]);
        gives(r#"{}"#, r#"from_entries"#, &[r#"{}"#]);
        gives(r#"{"x":{"key":"a","value":1}}"#, r#"from_entries"#, &[r#"{"a":1}"#]);
        gives(r#"[{"key":"a","value":1},{"key":"a","value":2}]"#, r#"from_entries"#, &[r#"{"a":2}"#]);
        gives(r#"[{"key":"b","value":1},{"key":"a","value":2}]"#, r#"from_entries"#, &[r#"{"b":1,"a":2}"#]);
        gives(r#"[{"key":"a","value":1,"extra":2}]"#, r#"from_entries"#, &[r#"{"a":1}"#]);
        gives(r#"[{"key":"","value":1}]"#, r#"from_entries"#, &[r#"{"":1}"#]);
        gives(r#"[{"key":"é","value":1}]"#, r#"from_entries"#, &[r#"{"é":1}"#]);
        gives(r#"[{"Key":"env","Value":"prod"},{"Key":"team","Value":"x"}]"#, r#"from_entries"#, &[r#"{"env":"prod","team":"x"}"#]);
        gives(r#"[{"name":"A","value":"1"},{"name":"B","value":"2"}]"#, r#"from_entries"#, &[r#"{"A":"1","B":"2"}"#]);
        gives(r#"[{"key":"a","value":1,"Value":2}]"#, r#"from_entries"#, &[r#"{"a":1}"#]);
        gives(r#"[{"key":"a","Key":"b","value":1}]"#, r#"from_entries"#, &[r#"{"a":1}"#]);
        gives(r#"[{"name":"n","Name":"N","value":1}]"#, r#"from_entries"#, &[r#"{"n":1}"#]);
        gives(r#"[{"key":"a","value":1},{"name":"b","value":2},{"Key":"c","Value":3}]"#, r#"from_entries"#, &[r#"{"a":1,"b":2,"c":3}"#]);
        gives(r#"[{"key":"b","value":1},{"key":"a","value":2}]"#, r#"from_entries | keys"#, &[r#"["a","b"]"#]);
        gives(r#"{"a":1,"b":2}"#, r#"with_entries(.value += 1)"#, &[r#"{"a":2,"b":3}"#]);
        gives(r#"{"a":1,"b":2,"c":3}"#, r#"with_entries(select(.value > 1))"#, &[r#"{"b":2,"c":3}"#]);
        gives(r#"{"a":1,"b":2}"#, r#"with_entries(.key |= ascii_upcase)"#, &[r#"{"A":1,"B":2}"#]);
        gives(r#"{"a":1}"#, r#"with_entries(.key = .key + "x")"#, &[r#"{"ax":1}"#]);
        gives(r#"{"a":"x"}"#, r#"with_entries({key: .value, value: .key})"#, &[r#"{"x":"a"}"#]);
        gives(r#"{"a":1}"#, r#"with_entries(.Key = .key | del(.key))"#, &[r#"{"a":1}"#]);
        gives(r#"{"a":1}"#, r#"with_entries(.name = .key | del(.key))"#, &[r#"{"a":1}"#]);
        gives(r#"{"a":1}"#, r#"with_entries(empty)"#, &[r#"{}"#]);
        gives(r#"{"b":1,"a":2}"#, r#"to_entries | from_entries"#, &[r#"{"b":1,"a":2}"#]);
        gives(r#"{"a":null,"b":2}"#, r#"(to_entries | map(select(.value != null)) | from_entries)"#, &[r#"{"b":2}"#]);
        gives(r#"null"#, r#"[range(5000) | {key: "k\(.)", value: .}] | from_entries | length"#, &[r#"5000"#]);
        gives(r#"null"#, r#"[range(20000) | {name: "k\(.)", value: .}] | from_entries | length"#, &[r#"20000"#]);
        fails(r#"[{"k":"a","v":1}]"#, r#"from_entries"#);
        fails(r#"[{"key":1,"value":2}]"#, r#"from_entries"#);
        fails(r#"[{"key":null,"value":2}]"#, r#"from_entries"#);
        fails(r#"[{"key":false,"value":2}]"#, r#"from_entries"#);
        fails(r#"[{"key":true,"value":2}]"#, r#"from_entries"#);
        fails(r#"[{"value":2}]"#, r#"from_entries"#);
        fails(r#"[["a",1]]"#, r#"from_entries"#);
        fails(r#"["a"]"#, r#"from_entries"#);
        fails(r#"[null]"#, r#"from_entries"#);
        fails(r#"null"#, r#"from_entries"#);
        fails(r#"[{"key":0,"value":1}]"#, r#"from_entries"#);
        fails(r#"[{"key":[1],"value":1}]"#, r#"from_entries"#);
        fails(r#"[{"key":{"a":1},"value":1}]"#, r#"from_entries"#);
        fails(r#"[1]"#, r#"from_entries"#);
        fails(r#"[1,2]"#, r#"with_entries(.)"#);
    }

    /// A missing `=` at the end is no error (a JWT segment has none), the first `=` ends the text,
    /// and only a lone character left over is. What is not UTF-8 becomes U+FFFD the way jq's own reader
    /// of UTF-8 takes it.
    #[test]
    #[rustfmt::skip]
    fn base64d_decodes_what_jq_decodes() {
        gives(r#""aGVsbG8""#, r#"@base64d"#, &[r#""hello""#]);
        gives(r#""aGVsbG8=""#, r#"@base64d"#, &[r#""hello""#]);
        gives(r#""aGVsbG8==""#, r#"@base64d"#, &[r#""hello""#]);
        gives(r#""aGVsbG8=xx""#, r#"@base64d"#, &[r#""hello""#]);
        gives(r#""ab""#, r#"@base64d"#, &[r#""i""#]);
        gives(r#""abc""#, r#"@base64d"#, &[r#""i�""#]);
        gives(r#""abcd""#, r#"@base64d"#, &[r#""i�\u001d""#]);
        gives(r#""""#, r#"@base64d"#, &[r#""""#]);
        gives(r#""YQ""#, r#"@base64d"#, &[r#""a""#]);
        gives(r#""YQ==""#, r#"@base64d"#, &[r#""a""#]);
        gives(r#""YQ=""#, r#"@base64d"#, &[r#""a""#]);
        gives(r#""+/+/""#, r#"@base64d"#, &[r#""���""#]);
        gives(r#""/w==""#, r#"@base64d"#, &[r#""�""#]);
        gives(r#""AA==""#, r#"@base64d"#, &[r#""\u0000""#]);
        gives(r#""4pyT""#, r#"@base64d"#, &[r#""✓""#]);
        gives(r#""4py""#, r#"@base64d"#, &[r#""�""#]);
        gives(r#"123"#, r#"@base64d"#, &[r#""�m""#]);
        gives(r#"null"#, r#"@base64d"#, &[r#""��""#]);
        gives(r#""héllo wörld""#, r#"@base64 | @base64d"#, &[r#""héllo wörld""#]);
        gives(r#""aGk""#, r#"@base64d "x\(.)y""#, &[r#""xhiy""#]);
        gives(r#"["YQ","Yg=="]"#, r#"[.[] | @base64d]"#, &[r#"["a","b"]"#]);
        gives(r#""eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4ifQ.sig""#, r#"split(".")[1] | @base64d | fromjson"#, &[r#"{"sub":"1234567890","name":"John"}"#]);
        gives(r#""aGVsbG8""#, r#"@base64d | length"#, &[r#"5"#]);
        gives(r#""aGVsbG8gd29ybGQ""#, r#"@base64d"#, &[r#""hello world""#]);
        gives(r#""SGVsbG8sIFdvcmxkIQ""#, r#"@base64d"#, &[r#""Hello, World!""#]);
        gives(r#""=aGVsbG8""#, r#"@base64d"#, &[r#""""#]);
        gives(r#""aGVsbG8====""#, r#"@base64d"#, &[r#""hello""#]);
        gives(r#""Zm9v""#, r#"@base64d"#, &[r#""foo""#]);
        gives(r#""Zm9vYg""#, r#"@base64d"#, &[r#""foob""#]);
        gives(r#""Zm9vYmE""#, r#"@base64d"#, &[r#""fooba""#]);
        gives(r#""Zm9vYmFy""#, r#"@base64d"#, &[r#""foobar""#]);
        gives(r#""Zg""#, r#"@base64d"#, &[r#""f""#]);
        gives(r#""Zm8""#, r#"@base64d"#, &[r#""fo""#]);
        gives(r#""=""#, r#"@base64d"#, &[r#""""#]);
        gives(r#""gA==""#, r#"@base64d"#, &[r#""�""#]);
        gives(r#""wA==""#, r#"@base64d"#, &[r#""�""#]);
        gives(r#""/+8=""#, r#"@base64d"#, &[r#""��""#]);
        gives(r#""8J+YgA""#, r#"@base64d"#, &[r#""😀""#]);
        gives(r#""8J+Y""#, r#"@base64d"#, &[r#""�""#]);
        gives(r#""/w==""#, r#"@base64d | explode"#, &[r#"[65533]"#]);
        gives(r#""/w==""#, r#"@base64d | utf8bytelength"#, &[r#"3"#]);
        gives(r#""4pyT""#, r#"@base64d | length"#, &[r#"1"#]);
        gives(r#""SEVMTE8""#, r#"@base64d | ascii_downcase"#, &[r#""hello""#]);
        gives(r#""aGVsbG8""#, r#"@base64d | test("hello")"#, &[r#"true"#]);
        gives(r#""aGVsbG8""#, r#"@base64d | @base64"#, &[r#""aGVsbG8=""#]);
        fails(r#""a""#, r#"@base64d"#);
        fails(r#""Y""#, r#"@base64d"#);
        fails(r#""Y===""#, r#"@base64d"#);
        fails(r#""/""#, r#"@base64d"#);
        fails(r#""aGVs\nbG8=""#, r#"@base64d"#);
        fails(r#""aGVs bG8=""#, r#"@base64d"#);
        fails(r#""-_""#, r#"@base64d"#);
        fails(r#""a-b_""#, r#"@base64d"#);
        fails(r#"[1]"#, r#"@base64d"#);
        fails(r#"{"a":1}"#, r#"@base64d"#);
        fails(r#""é""#, r#"@base64d"#);
        fails(r#""aGVsbG8\t""#, r#"@base64d"#);
        fails(r#"" aGVsbG8""#, r#"@base64d"#);
        fails(r#""日本""#, r#"@base64d"#);
    }

    /// The keys that are left are in the order they were in: jaq took a key out by moving the last one
    /// into its place. (`gives` compares objects without looking at the order, so this looks at the text.)
    #[test]
    fn deleting_a_key_leaves_the_others_in_their_order() {
        let text = |input: &str, filter: &str| {
            serde_json::to_string(&run(&parse(input), filter).expect("runs")[0]).unwrap()
        };
        assert_eq!(
            text(r#"{"a":1,"b":2,"c":3}"#, "del(.a)"),
            r#"{"b":2,"c":3}"#
        );
        assert_eq!(
            text(r#"{"a":1,"b":2,"c":3}"#, ".a |= empty"),
            r#"{"b":2,"c":3}"#
        );
        assert_eq!(
            text(r#"{"a":1,"b":2,"c":3,"d":4}"#, "del(.a, .c)"),
            r#"{"b":2,"d":4}"#
        );
        assert_eq!(
            text(r#"{"a":1,"b":2,"c":3}"#, "delpaths([[\"a\"], [\"b\"]])"),
            r#"{"c":3}"#
        );
        assert_eq!(
            text(
                r#"{"a":1,"b":2,"c":3}"#,
                r#"with_entries(select(.key != "b"))"#
            ),
            r#"{"a":1,"c":3}"#
        );
        assert_eq!(text(r#"{"c":1,"a":2}"#, ".b = 3"), r#"{"c":1,"a":2,"b":3}"#);
        assert_eq!(text("null", ".z.y = 1 | .a = 2"), r#"{"z":{"y":1},"a":2}"#);
        assert_eq!(
            text(
                r#"[{"Key":"b","Value":1},{"Key":"a","Value":2}]"#,
                "from_entries"
            ),
            r#"{"b":1,"a":2}"#
        );
    }

    /// Deleting, and putting a value in, take a time that grows with the size of the value and not with
    /// its square (a hundred thousand items would not end, otherwise).
    #[test]
    fn deleting_and_writing_stay_fast_on_big_values() {
        let started = std::time::Instant::now();
        gives(
            "null",
            "[range(200000)] | del(.[] | select(. % 2 == 0)) | length",
            &["100000"],
        );
        gives(
            "null",
            "[range(200000)] | del(.[range(0; 200000; 3)]) | length",
            &["133333"],
        );
        gives(
            "null",
            r#"[range(100000) | {key: "k\(.)", value: .}] | from_entries | del(.k0, .k1, .k99999) | length"#,
            &["99997"],
        );
        gives(
            "null",
            "reduce range(50000) as $i (null; .[$i] = $i) | length",
            &["50000"],
        );
        gives(
            "null",
            r#"reduce range(50000) as $i (null; .["k\($i)"].n = $i) | length"#,
            &["50000"],
        );
        gives(
            "null",
            r#"[range(100000) | tostring] | join(",") | length"#,
            &["588889"],
        );
        gives(
            "null",
            r#"[range(50000) | {key: "k\(.)", value: .}] | from_entries | length"#,
            &["50000"],
        );
        gives(
            "null",
            r#""a1" * 20000 | [scan("\\d")] | length"#,
            &["20000"],
        );
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_secs(60), "took {took:?}");
    }

    /// An index meant as a key (an id, a timestamp) must not use up the memory by making an array that
    /// long: jq allows 536,870,911 items, which is eight gigabytes, here the limit is 16,777,215.
    #[test]
    fn an_item_put_too_far_past_the_end_is_an_error_and_not_the_end_of_the_memory() {
        gives("[]", ".[1000000] = 1 | length", &["1000001"]);
        for index in [
            "16777216",
            "20000000",
            "1700000000",
            "536870912",
            "9223372036854775807",
        ] {
            let message = error_of("[]", &format!(".[{index}] = 1"));
            assert_eq!(message, "Array index too large", "{index}");
        }
        assert_eq!(
            error_of("null", ".[1700000000].a = 1"),
            "Array index too large"
        );
        // nothing is put, so nothing is made
        gives("[1]", ".[1700000000] |= empty", &["[1]"]);
        gives(
            "[1]",
            "try (.[1700000000] = 1) catch .",
            &[r#""Array index too large""#],
        );
    }
}
