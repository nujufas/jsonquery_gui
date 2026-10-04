//! Printing a JSON value as text in the layout asked for — what the Tools
//! window's Format JSON does: pretty-print with two or four spaces or a tab,
//! minify, sort the keys, and write non-ASCII characters as escapes.
//!
//! It is its own small printer rather than `serde_json::to_string_pretty`
//! because of those last two options and the tab, which that has no way to
//! ask for; for the common layouts the text is the same as serde_json's. A
//! number is written as the text it was read from (the document keeps exact
//! digits, see `crates/core`), so `1.0`, `0.50` and a 30-digit integer come out
//! as they went in. (The one change is to an exponent, which serde_json reads
//! in a normal form: `1E5` comes out as `1e+5`.)

use std::fmt::Write as _;

use serde_json::Value;

/// How each level of a pretty-printed document is indented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    /// This many spaces per level.
    Spaces(u8),
    Tab,
    /// No white space at all: `{"a":[1,2]}`.
    Minified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub indent: Indent,
    /// Write every object's members in key order (by code point), however
    /// deep. Arrays keep their order.
    pub sort_keys: bool,
    /// Write every character outside ASCII as `\uXXXX` (a surrogate pair above
    /// U+FFFF), so the text is safe to pass through anything that only
    /// carries ASCII.
    pub ascii_only: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            indent: Indent::Spaces(2),
            sort_keys: false,
            ascii_only: false,
        }
    }
}

/// `value` as text. There is no newline after it.
pub fn render(value: &Value, options: &Options) -> String {
    let mut out = String::new();
    Writer {
        options,
        out: &mut out,
    }
    .value(value, 0);
    out
}

struct Writer<'a> {
    options: &'a Options,
    out: &'a mut String,
}

impl Writer<'_> {
    fn value(&mut self, value: &Value, depth: usize) {
        match value {
            Value::Null => self.out.push_str("null"),
            Value::Bool(true) => self.out.push_str("true"),
            Value::Bool(false) => self.out.push_str("false"),
            Value::Number(n) => {
                let _ = write!(self.out, "{n}");
            }
            Value::String(s) => self.string(s),
            Value::Array(items) => self.array(items, depth),
            Value::Object(members) => self.object(members, depth),
        }
    }

    fn array(&mut self, items: &[Value], depth: usize) {
        if items.is_empty() {
            self.out.push_str("[]");
            return;
        }
        self.out.push('[');
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.out.push(',');
            }
            self.line(depth + 1);
            self.value(item, depth + 1);
        }
        self.line(depth);
        self.out.push(']');
    }

    fn object(&mut self, members: &serde_json::Map<String, Value>, depth: usize) {
        if members.is_empty() {
            self.out.push_str("{}");
            return;
        }
        let mut entries: Vec<(&String, &Value)> = members.iter().collect();
        if self.options.sort_keys {
            entries.sort_by(|a, b| a.0.cmp(b.0));
        }
        self.out.push('{');
        for (i, (key, value)) in entries.into_iter().enumerate() {
            if i > 0 {
                self.out.push(',');
            }
            self.line(depth + 1);
            self.string(key);
            self.out.push(':');
            if self.options.indent != Indent::Minified {
                self.out.push(' ');
            }
            self.value(value, depth + 1);
        }
        self.line(depth);
        self.out.push('}');
    }

    /// A line break and the indentation for `depth` (nothing when minified).
    fn line(&mut self, depth: usize) {
        match self.options.indent {
            Indent::Minified => {}
            Indent::Spaces(n) => {
                self.out.push('\n');
                self.out
                    .extend(std::iter::repeat_n(' ', usize::from(n) * depth));
            }
            Indent::Tab => {
                self.out.push('\n');
                self.out.extend(std::iter::repeat_n('\t', depth));
            }
        }
    }

    fn string(&mut self, text: &str) {
        self.out.push('"');
        for c in text.chars() {
            match c {
                '"' => self.out.push_str("\\\""),
                '\\' => self.out.push_str("\\\\"),
                '\n' => self.out.push_str("\\n"),
                '\r' => self.out.push_str("\\r"),
                '\t' => self.out.push_str("\\t"),
                '\u{08}' => self.out.push_str("\\b"),
                '\u{0c}' => self.out.push_str("\\f"),
                c if u32::from(c) < 0x20 => {
                    let _ = write!(self.out, "\\u{:04x}", u32::from(c));
                }
                c if self.options.ascii_only && !c.is_ascii() => {
                    let mut units = [0u16; 2];
                    for unit in c.encode_utf16(&mut units) {
                        let _ = write!(self.out, "\\u{unit:04x}");
                    }
                }
                c => self.out.push(c),
            }
        }
        self.out.push('"');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn render_with(value: &Value, indent: Indent, sort_keys: bool, ascii_only: bool) -> String {
        render(
            value,
            &Options {
                indent,
                sort_keys,
                ascii_only,
            },
        )
    }

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn two_spaces_is_what_serde_json_prints() {
        let value = json!({"a": [1, 2, {"b": null}], "c": "x", "d": {}, "e": []});
        assert_eq!(
            render(&value, &Options::default()),
            serde_json::to_string_pretty(&value).unwrap()
        );
    }

    #[test]
    fn four_spaces_and_tabs_indent_each_level() {
        let value = json!({"a": [1, {"b": true}]});
        assert_eq!(
            render_with(&value, Indent::Spaces(4), false, false),
            "{\n    \"a\": [\n        1,\n        {\n            \"b\": true\n        }\n    ]\n}"
        );
        assert_eq!(
            render_with(&value, Indent::Tab, false, false),
            "{\n\t\"a\": [\n\t\t1,\n\t\t{\n\t\t\t\"b\": true\n\t\t}\n\t]\n}"
        );
    }

    #[test]
    fn minified_has_no_white_space_outside_strings() {
        let value = json!({"a": [1, 2, {"b": "x y"}], "c": {}});
        assert_eq!(
            render_with(&value, Indent::Minified, false, false),
            r#"{"a":[1,2,{"b":"x y"}],"c":{}}"#
        );
    }

    #[test]
    fn empty_containers_stay_on_one_line() {
        assert_eq!(render(&json!({}), &Options::default()), "{}");
        assert_eq!(render(&json!([]), &Options::default()), "[]");
        assert_eq!(
            render(&json!({"a": [], "b": {}}), &Options::default()),
            "{\n  \"a\": [],\n  \"b\": {}\n}"
        );
    }

    #[test]
    fn a_scalar_document_is_just_the_scalar() {
        assert_eq!(render(&json!("hi"), &Options::default()), "\"hi\"");
        assert_eq!(render(&json!(null), &Options::default()), "null");
        assert_eq!(render(&json!(12), &Options::default()), "12");
    }

    #[test]
    fn keys_are_sorted_at_every_depth_and_arrays_keep_their_order() {
        let value = parse(r#"{"b": {"z": 1, "a": 2}, "a": [{"y": 1, "x": 2}, 3, 1], "C": 0}"#);
        assert_eq!(
            render_with(&value, Indent::Minified, true, false),
            r#"{"C":0,"a":[{"x":2,"y":1},3,1],"b":{"a":2,"z":1}}"#
        );
    }

    #[test]
    fn without_sorting_the_keys_keep_the_order_they_were_read_in() {
        let value = parse(r#"{"b": 1, "a": 2}"#);
        assert_eq!(
            render_with(&value, Indent::Minified, false, false),
            r#"{"b":1,"a":2}"#
        );
    }

    #[test]
    fn non_ascii_is_escaped_on_request() {
        let value = json!({"é": "naïve 😀"});
        assert_eq!(
            render_with(&value, Indent::Minified, false, true),
            "{\"\\u00e9\":\"na\\u00efve \\ud83d\\ude00\"}"
        );
        assert_eq!(
            render_with(&value, Indent::Minified, false, false),
            "{\"é\":\"naïve 😀\"}"
        );
    }

    #[test]
    fn escaped_text_reads_back_as_the_same_value() {
        let value = json!({"k\"ey": "line\nbreak\ttab \\ back \u{1} \u{7f} é 😀"});
        for ascii_only in [false, true] {
            let text = render_with(&value, Indent::Spaces(2), false, ascii_only);
            assert_eq!(parse(&text), value, "{text}");
        }
    }

    #[test]
    fn control_characters_are_escaped() {
        let value = json!("a\u{1}b\u{1f}c\u{8}\u{c}");
        assert_eq!(
            render(&value, &Options::default()),
            r#""a\u0001b\u001fc\b\f""#
        );
    }

    #[test]
    fn numbers_keep_the_text_they_were_read_with() {
        let text = r#"{"a":1.0,"b":12345678901234567890123,"c":-0.50,"d":0.1e-3,"e":1e+5}"#;
        let value = parse(text);
        assert_eq!(render_with(&value, Indent::Minified, false, false), text);
    }

    #[test]
    fn an_exponent_comes_out_in_the_form_serde_json_reads_it_in() {
        let value = parse("[1E5, 2e5, 3E-2]");
        assert_eq!(
            render_with(&value, Indent::Minified, false, false),
            "[1e+5,2e+5,3e-2]"
        );
    }

    #[test]
    fn printing_then_reading_gives_the_document_back() {
        let value = parse(
            r#"{"users":[{"id":1,"tags":["a","b"],"ok":true,"n":null},{"id":2.5,"tags":[]}],"z":{}}"#,
        );
        for indent in [
            Indent::Spaces(2),
            Indent::Spaces(4),
            Indent::Tab,
            Indent::Minified,
        ] {
            for sort_keys in [false, true] {
                let text = render_with(&value, indent, sort_keys, true);
                let back = parse(&text);
                // `==` on objects ignores order, so sorted output still matches.
                assert_eq!(back, value, "{indent:?} sort={sort_keys}\n{text}");
            }
        }
    }
}
