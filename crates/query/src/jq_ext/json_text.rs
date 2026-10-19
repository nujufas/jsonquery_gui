//! `fromjson`, `tonumber` and `tojson`: JSON as text and back, in place of jaq's.
//!
//! **`fromjson`.** jaq reads the text as a *sequence* of values, and on text it cannot
//! read it answers with an error and then tries the same place again, for ever:
//! `"abc" | fromjson` and `"abc" | tonumber` (which is `fromjson` in jaq) never ended,
//! used up memory as they went, and took the worker thread, which every query waits
//! on, with them. Here the text is exactly one value, as in jq (`"1 2" | fromjson` is an
//! error there, and in jaq it was two outputs), and the first thing that cannot be
//! read is the error that ends it. The reader is jaq's own, so what it takes (a few
//! things more than JSON, like `#` comments and `NaN`) is what it took.
//!
//! The reader calls itself for every array or object inside another, and a text can
//! open as many as it has room for, so the depth is looked at first, without
//! reading: jq refuses text nested more than 10,000 levels deep, and this does at
//! [`MAX_PARSE_DEPTH`], which even a small stack (a megabyte) has room for.
//!
//! **`tonumber`** is not `fromjson` here, but what jq's is: a string made of what a
//! number is made of (`+5`, `007`, `.5`, `5.`, `1e3`, `nan`, `-Infinity`), and
//! nothing else, not a space. jaq's took any JSON that is a number, which is less
//! (`+5` and `.5` were errors) and more (`" 5"` was 5).
//!
//! **`tojson`** stops at the depth jq stops at when it prints, 256 levels, and writes
//! [`TOO_DEEP`] there, the way jq does. Below that it is jaq's own writer, which calls
//! itself for every level and ends the process (with a stack of 2 MiB, at about 1,800
//! levels) for a value a program has made deeper than it can write.

use jaq_core::native::{bome, v, Filter, Fun};
use jaq_core::{DataT, RunPtr, ValR, ValT as _};
use jaq_json::write::{self, Pp};
use jaq_json::{read, Error, Num, Val};

use super::tabular::describe;
use crate::convert::{nests_deeper_than, TOO_DEEP};

/// How deep JSON text can be nested for `fromjson`. (jq: 10,000.)
const MAX_PARSE_DEPTH: usize = 1000;

/// The level `tojson` writes down to, the one jq prints down to. A value at a deeper
/// level is written as [`TOO_DEEP`].
const MAX_PRINT_DEPTH: usize = 256;

/// How many characters of a text an error about it quotes.
const QUOTE_CHARS: usize = 60;

/// Whether jaq-json's native filter `name` taking `arity` arguments is one of those
/// written again here, and so left out.
pub(super) fn replaces_fun(name: &str, arity: usize) -> bool {
    arity == 0 && matches!(name, "fromjson" | "tojson")
}

/// Whether jaq-json's definition (in jq) of `name` is one of those written again
/// here: a definition is found before a native filter of the same name.
pub(super) fn replaces_def(name: &str) -> bool {
    name == "tonumber"
}

pub(super) fn funs<D: for<'a> DataT<V<'a> = Val>>() -> [Fun<D>; 3] {
    let fromjson: Filter<RunPtr<D>> = ("fromjson", v(0), |cv| bome(fromjson(&cv.1)));
    let tonumber: Filter<RunPtr<D>> = ("tonumber", v(0), |cv| bome(tonumber(&cv.1)));
    let tojson: Filter<RunPtr<D>> = ("tojson", v(0), |cv| bome(Ok(Val::utf8_str(tojson(&cv.1)))));
    [fromjson, tonumber, tojson].map(jaq_core::native::run::<D>)
}

fn fromjson(val: &Val) -> ValR<Val> {
    let Val::TStr(text) = val else {
        return Err(Error::str(format!(
            "{} only strings can be parsed",
            describe(val)
        )));
    };
    let text: &[u8] = text;
    parse(text)
        .map_err(|reason| Error::str(format!("{reason} (while parsing '{}')", start_of(text))))
}

/// The one value `text` is, or why it is none.
fn parse(text: &[u8]) -> Result<Val, String> {
    if text_nests_deeper_than(text, MAX_PARSE_DEPTH) {
        return Err(format!(
            "Exceeds depth limit for parsing ({MAX_PARSE_DEPTH} levels)"
        ));
    }
    read::parse_single(text).map_err(|e| e.to_string())
}

/// The start of `text`, for an error to quote: all of it if it is short.
fn start_of(text: &[u8]) -> String {
    // Four bytes make a character, at most; the rest of a long text is not looked at.
    let head = String::from_utf8_lossy(&text[..text.len().min(4 * QUOTE_CHARS)]);
    let mut chars = head.chars();
    let mut quoted: String = chars.by_ref().take(QUOTE_CHARS).collect();
    if chars.next().is_some() || text.len() > 4 * QUOTE_CHARS {
        quoted.push_str("...");
    }
    quoted
}

/// Whether `text` opens arrays and objects more than `limit` deep, judged without
/// reading it: brackets inside strings and `#` comments (which the reader allows
/// wherever it allows a space) are not counted, and what is not JSON is not looked
/// at, because the reader will say so.
fn text_nests_deeper_than(text: &[u8], limit: usize) -> bool {
    let (mut depth, mut in_string, mut escaped, mut in_comment) = (0usize, false, false, false);
    for &byte in text {
        if in_comment {
            in_comment = byte != b'\n';
        } else if in_string {
            match (escaped, byte) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'#' => in_comment = true,
                b'[' | b'{' => {
                    depth += 1;
                    if depth > limit {
                        return true;
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    false
}

fn tonumber(val: &Val) -> ValR<Val> {
    let number = match val {
        Val::Num(_) => return Ok(val.clone()),
        Val::TStr(text) => std::str::from_utf8(text).ok().and_then(number),
        _ => None,
    };
    number.ok_or_else(|| Error::str(format!("{} cannot be parsed as a number", describe(val))))
}

/// The number `text` is, if it is one the way jq's `tonumber` takes it: an optional
/// sign, digits with a point and digits after it (either may be left out, not both),
/// an exponent (`e` or `E`, an optional sign, digits); or `nan`, `inf` or `infinity`,
/// in either case, after the sign.
fn number(text: &str) -> Option<Val> {
    let (negative, rest) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    if rest.eq_ignore_ascii_case("nan") {
        return Some(Val::Num(Num::Float(f64::NAN)));
    }
    if rest.eq_ignore_ascii_case("inf") || rest.eq_ignore_ascii_case("infinity") {
        return Some(Val::Num(Num::Float(if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        })));
    }
    let bytes = rest.as_bytes();
    let digits_from = |at: usize| {
        bytes[at.min(bytes.len())..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    };
    let whole = digits_from(0);
    let mut end = whole;
    let mut fraction = 0;
    if bytes.get(end) == Some(&b'.') {
        fraction = digits_from(end + 1);
        end += 1 + fraction;
    }
    if whole == 0 && fraction == 0 {
        return None;
    }
    let exponent_at = end;
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
        let exponent = digits_from(end + 1 + sign);
        if exponent == 0 {
            return None;
        }
        end += 1 + sign + exponent;
    }
    if end != bytes.len() {
        return None;
    }
    // As jaq reads a number out of a document: no `+`, no zeros in front, and a point
    // only where there are digits behind it.
    let mut written = String::with_capacity(text.len() + 1);
    if negative {
        written.push('-');
    }
    let whole_digits = rest[..whole].trim_start_matches('0');
    written.push_str(if whole_digits.is_empty() {
        "0"
    } else {
        whole_digits
    });
    if fraction > 0 {
        written.push('.');
        written.push_str(&rest[whole + 1..whole + 1 + fraction]);
    }
    written.push_str(&rest[exponent_at..]);
    Val::from_num(&written).ok()
}

fn tojson(val: &Val) -> Vec<u8> {
    let mut out = Vec::new();
    if nests_deeper_than(val, MAX_PRINT_DEPTH) {
        write_down_to_the_limit(val, 0, &mut out);
    } else {
        write_json(val, &mut out);
    }
    out
}

/// `val` the way jaq's `tojson` writes it: on one line, with no spaces.
fn write_json(val: &Val, out: &mut Vec<u8>) {
    write::write(out, &Pp::default(), 0, val).expect("a Vec can be written to");
}

/// `val`, which is at `level`, as jaq writes it (no spaces, no new lines) except that a
/// value at a level deeper than [`MAX_PRINT_DEPTH`] is [`TOO_DEEP`]. It calls itself
/// no more often than that, so a value of any depth can be written.
fn write_down_to_the_limit(val: &Val, level: usize, out: &mut Vec<u8>) {
    if level > MAX_PRINT_DEPTH {
        out.extend_from_slice(TOO_DEEP.as_bytes());
        return;
    }
    match val {
        Val::Arr(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_down_to_the_limit(item, level + 1, out);
            }
            out.push(b']');
        }
        Val::Obj(entries) => {
            out.push(b'{');
            for (i, (key, item)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_json(key, out);
                out.push(b':');
                write_down_to_the_limit(item, level + 1, out);
            }
            out.push(b'}');
        }
        leaf => write_json(leaf, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{from_val, spawn_query_thread, to_val};
    use serde_json::json;

    /// `n` arrays, one in the next, round `core`.
    fn nested(n: usize, core: Val) -> Val {
        (0..n).fold(core, |inner, _| std::iter::once(inner).collect())
    }

    /// `work` on a thread made for running programs: the values these tests make are
    /// deeper than the stack of a test's own thread can drop.
    fn on_a_query_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = std::sync::mpsc::channel();
        spawn_query_thread("test", move || sender.send(work()).unwrap());
        receiver.recv().expect("the thread ends with an answer")
    }

    fn written(val: &Val) -> String {
        String::from_utf8(tojson(val)).unwrap()
    }

    fn is_nan(val: Option<Val>) -> bool {
        matches!(val, Some(Val::Num(Num::Float(x))) if x.is_nan())
    }

    #[test]
    fn a_number_is_what_jqs_decimal_library_reads() {
        let read = |text: &str| number(text).map(|v| v.to_string());
        // (taken from jq 1.8.1's `tonumber`)
        for (text, shown) in [
            ("1", "1"),
            ("-1", "-1"),
            ("0", "0"),
            ("1.0", "1.0"),
            ("1.50", "1.50"),
            ("01", "1"),
            ("007", "7"),
            ("00", "0"),
            ("+1", "1"),
            ("+0", "0"),
            ("1.", "1"),
            (".5", "0.5"),
            ("-.5", "-0.5"),
            ("+.5", "0.5"),
            ("00.5", "0.5"),
            ("0.50", "0.50"),
            ("5.5", "5.5"),
            ("1e3", "1e3"),
            ("1E3", "1E3"),
            ("1e+3", "1e+3"),
            ("1e-3", "1e-3"),
            ("1.e2", "1e2"),
            (".5e2", "0.5e2"),
            ("1e00", "1e00"),
            ("100000000000000000000", "100000000000000000000"),
            (
                "123456789012345678901234567890",
                "123456789012345678901234567890",
            ),
            (
                "-123456789012345678901234567890",
                "-123456789012345678901234567890",
            ),
        ] {
            assert_eq!(read(text).as_deref(), Some(shown), "{text:?}");
        }
        for text in [
            "",
            " ",
            "-",
            "+",
            ".",
            "-.",
            "+.",
            "e",
            "e1",
            ".e1",
            "1e",
            "1e+",
            "1e-",
            "1e1.5",
            "1e1e1",
            "ee1",
            "1.5.5",
            "0.0.0",
            "..5",
            ".5.",
            "--1",
            "+-1",
            "-+1",
            "1-",
            "1+",
            "0x10",
            "0b1",
            "1_000",
            "1,5",
            "1 0",
            " 1",
            "1 ",
            "\t1",
            "1\n",
            "\u{a0}1",
            "1\u{a0}",
            "٣",
            "１",
            "−1",
            "1f",
            "null",
            "true",
            "false",
            "[1]",
            "{}",
            "\"1\"",
            "i",
            "in",
            "na",
            "nanx",
            "infinit",
            "infinityx",
            "--inf",
            "+-nan",
        ] {
            assert!(number(text).is_none(), "{text:?}");
        }
        for text in ["nan", "NaN", "NAN", "nAn", "-nan", "+NaN"] {
            assert!(is_nan(number(text)), "{text:?}");
        }
        for (text, shown) in [
            ("inf", "Infinity"),
            ("Inf", "Infinity"),
            ("INFINITY", "Infinity"),
            ("infinity", "Infinity"),
            ("+Infinity", "Infinity"),
            ("-inf", "-Infinity"),
            ("-Infinity", "-Infinity"),
        ] {
            assert_eq!(read(text).as_deref(), Some(shown), "{text:?}");
        }
    }

    #[test]
    fn a_number_keeps_every_digit_it_was_given() {
        // Not rounded through a float, as a number out of a document is not.
        assert_eq!(
            number("0.30000000000000004000").unwrap().to_string(),
            "0.30000000000000004000"
        );
        assert_eq!(
            number("9223372036854775808").unwrap().to_string(),
            "9223372036854775808"
        );
        assert_eq!(
            number("-9223372036854775809").unwrap().to_string(),
            "-9223372036854775809"
        );
    }

    #[test]
    fn json_text_nests_as_deep_as_the_brackets_outside_strings_and_comments_say() {
        let nest = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(!text_nests_deeper_than(nest(1000).as_bytes(), 1000));
        assert!(text_nests_deeper_than(nest(1001).as_bytes(), 1000));
        assert!(text_nests_deeper_than(nest(1_000_000).as_bytes(), 1000));
        assert!(text_nests_deeper_than(&b"[".repeat(1001), 1000));
        // objects count, and so do the two kinds together
        assert!(text_nests_deeper_than("{\"a\":".repeat(3).as_bytes(), 2));
        assert!(!text_nests_deeper_than("{\"a\":".repeat(3).as_bytes(), 3));
        assert!(text_nests_deeper_than(b"[{[", 2));
        // brackets inside a string do not, whatever is escaped in it
        assert!(!text_nests_deeper_than(
            format!("[\"{}\"]", "[".repeat(5000)).as_bytes(),
            3
        ));
        assert!(!text_nests_deeper_than(
            br#"["\"[[[[[[[[", "\\", "[[[[[["]"#,
            3
        ));
        // nor does a comment, up to the end of its line, and what follows is counted
        assert!(!text_nests_deeper_than(
            format!("# {}\n[1]", "[".repeat(5000)).as_bytes(),
            3
        ));
        assert!(text_nests_deeper_than(
            format!("# {}\n{}", "[", "[".repeat(5)).as_bytes(),
            3
        ));
        // a quote in a comment opens no string, a `#` in a string no comment
        assert!(text_nests_deeper_than(
            format!("# \"\n{}", "[".repeat(5)).as_bytes(),
            3
        ));
        assert!(text_nests_deeper_than(
            format!("\"#\"{}", "[".repeat(5)).as_bytes(),
            3
        ));
        // a closing bracket with nothing open is no room made
        assert!(!text_nests_deeper_than(b"]]]]]]]][", 3));
    }

    #[test]
    fn text_that_cannot_be_read_ends_in_one_error() {
        // Every string over these characters, up to five long: none of them goes on
        // for ever (this test not ending is the failure), each is a value or an error.
        let alphabet = b"[]{}\",:0-.eE+tfnulaNIy# \\\n";
        let mut text = Vec::new();
        let mut count = 0usize;
        fn go(text: &mut Vec<u8>, alphabet: &[u8], left: usize, count: &mut usize) {
            let _ = parse(text);
            *count += 1;
            if left > 0 {
                for &byte in alphabet {
                    text.push(byte);
                    go(text, alphabet, left - 1, count);
                    text.pop();
                }
            }
        }
        go(&mut text, alphabet, 4, &mut count);
        assert_eq!(count, (0..=4).map(|n| alphabet.len().pow(n)).sum::<usize>());
    }

    #[test]
    fn the_start_of_a_text_is_quoted_not_all_of_it() {
        assert_eq!(start_of(b"abc"), "abc");
        assert_eq!(start_of(&[b'x'; 60]), "x".repeat(60));
        assert_eq!(start_of(&[b'x'; 61]), format!("{}...", "x".repeat(60)));
        assert_eq!(
            start_of(&b"x".repeat(10_000_000)),
            format!("{}...", "x".repeat(60))
        );
        // as many characters as it says, not bytes, and no half of a character
        let wide = "é".repeat(100);
        assert_eq!(start_of(wide.as_bytes()), format!("{}...", "é".repeat(60)));
        assert_eq!(start_of(&[0xff, b'a']), "\u{fffd}a");
    }

    #[test]
    fn tojson_writes_a_shallow_value_as_jaq_does() {
        for value in [
            json!(null),
            json!(true),
            json!(1),
            json!(-1.5),
            json!("a\"b\\c\u{1}\n\té😀\u{7f}"),
            json!([]),
            json!({}),
            json!([1, [2, [3, []]], {"a": {"b": [null]}}]),
            json!({"b": 1, "a": [1, 2, {"c": "d"}]}),
            json!(123456789012345678901234567890u128),
        ] {
            let val = to_val(&value);
            assert_eq!(written(&val), val.to_string(), "{value}");
        }
    }

    #[test]
    fn tojson_stops_where_jq_stops() {
        // (the lengths are those of jq 1.8.1's `tojson`)
        let len = |n, core| written(&nested(n, core)).len();
        assert_eq!(len(256, Val::from(1isize)), 513); // 256 arrays round a 1: all there
        assert_eq!(len(257, Val::from(1isize)), 533); // the 1 is at level 257: it is not
        assert_eq!(len(300, Val::from(1isize)), 533);
        assert_eq!(len(256, Val::Null), 516); // level 256 is the last: a `null` there is
        let text = written(&nested(258, Val::Null));
        assert_eq!(text.len(), 533);
        assert_eq!(
            text,
            format!("{}{TOO_DEEP}{}", "[".repeat(257), "]".repeat(257))
        );
        // objects, and the keys down to the level
        let object = (0..258).fold(Val::from(1isize), |inner, _| {
            Val::obj(
                [(Val::utf8_str(b"a".to_vec()), inner)]
                    .into_iter()
                    .collect(),
            )
        });
        let text = written(&object);
        assert_eq!(text.len(), 1561);
        assert!(text.ends_with(&format!("{{\"a\":{TOO_DEEP}}}{}", "}".repeat(256))));
    }

    #[test]
    fn a_value_far_deeper_than_the_stack_takes_is_written_and_shown_all_the_same() {
        let (text, shown) = on_a_query_thread(|| {
            let deep = nested(100_000, Val::Null);
            let text = written(&deep);
            let shown = from_val(&deep);
            (text, shown)
        });
        assert_eq!(text.len(), 533);
        let mut depth = 0;
        let mut level = &shown;
        while let serde_json::Value::Array(items) = level {
            depth += 1;
            level = &items[0];
        }
        assert_eq!(depth, crate::convert::MAX_DEPTH);
        assert_eq!(level, &json!(TOO_DEEP));
    }
}
