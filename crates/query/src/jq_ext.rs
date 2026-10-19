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

mod json_text;
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
    [csv, tsv, tostream::fun::<D>(), fromjson, tonumber, tojson].into_iter()
}

/// Whether jaq-json's native filter `name` taking `arity` arguments is written again
/// in [`funs`], and is to be left out of the list handed to the compiler: the first
/// filter of a name in that list is the one that is called.
pub fn replaces_fun(name: &str, arity: usize) -> bool {
    json_text::replaces_fun(name, arity)
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
}
