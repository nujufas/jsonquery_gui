//! Splits a query into the fragments the query box tints — the automatic
//! counterpart of the tutorial's hand-written `Example::parts`.
//!
//! A *chip* is one step of the query: a member (`.name`), a bracket group
//! (`[0]`, `[?age > 30]`), a call (`select(.age > 30)`), a literal, a
//! variable, or — for JSON Pointer — one `/segment`. Everything between chips
//! (pipes, commas, operators, whitespace) is plain glue, as the `|` between
//! stages is in the tutorial.
//!
//! A group is normally one chip, but one that *contains a function call* is
//! opened up so the call gets its own tint, as the tutorial does for
//! `map(select(.active) | .name)`: the opener (`map(`), each step inside, and
//! the closer (`)`), which shares the opener's tint so the pair reads as one.
//! Chips still never overlap.
//!
//! The query box calls this on every keystroke, so it works on half-typed
//! text: an unclosed string or bracket simply runs to the end, and stray
//! closers are glue. It never fails and always covers the whole query.

use std::ops::Range;

use crate::Kind;

/// jq words that structure an expression rather than operate in it; shown as
/// glue so `if .a then .b else .c end` tints `.a`, `.b` and `.c`, not the
/// keywords.
const JQ_KEYWORDS: [&str; 12] = [
    "and", "or", "as", "if", "then", "elif", "else", "end", "try", "catch", "reduce", "foreach",
];

/// How many groups deep the scanner opens groups up. Deeper groups stay one
/// chip, which bounds both the recursion (the query box calls this on every
/// repaint, on text a user may have pasted) and the re-scanning each level
/// costs; nobody reads tints that far in.
const MAX_OPEN_DEPTH: usize = 32;

/// Splits `query` into consecutive slices that concatenate back to it. Each
/// chip is tagged with its index (0, 1, 2, …, numbered by where it starts);
/// glue is `None` — the same shape as `tutorial::segments`, so both can feed
/// one text-layout routine. The closer of an opened-up group repeats its
/// opener's index, so the two slices are tinted alike.
pub fn segments(kind: Kind, query: &str) -> Vec<(&str, Option<usize>)> {
    let chips = match kind {
        Kind::JsonPointer => pointer_chips(query),
        Kind::Jq | Kind::JsonPath | Kind::JmesPath => path_chips(kind, query),
    };
    let mut out = Vec::with_capacity(chips.len() * 2 + 1);
    let mut cursor = 0;
    for (chip, index) in chips {
        if chip.start > cursor {
            out.push((&query[cursor..chip.start], None));
        }
        out.push((&query[chip.clone()], Some(index)));
        cursor = chip.end;
    }
    if cursor < query.len() {
        out.push((&query[cursor..], None));
    }
    out
}

/// A chip's extent in the query, and the index that picks its tint.
type Chip = (Range<usize>, usize);

/// JSON Pointer: each `/…` up to the next `/` is a step. Spaces inside a
/// segment are part of the key, so only the query's trailing whitespace is
/// left out of the last chip.
fn pointer_chips(query: &str) -> Vec<Chip> {
    let mut chips: Vec<Range<usize>> = Vec::new();
    let mut open: Option<usize> = None;
    for (i, c) in query.char_indices() {
        if c == '/' {
            if let Some(start) = open {
                chips.push(start..i);
            }
            open = Some(i);
        }
    }
    if let Some(start) = open {
        chips.push(start..query.trim_end().len());
    }
    chips.into_iter().enumerate().map(|(i, c)| (c, i)).collect()
}

/// The chips of a jq, JSONPath or JMESPath query, in text order.
fn path_chips(kind: Kind, query: &str) -> Vec<Chip> {
    let mut chips = Chips::default();
    scan(kind, query, 0..query.len(), &mut chips);
    chips.list
}

/// The chips found so far, how many chip indexes have been handed out, and
/// how many opened-up groups the scanner is currently inside.
#[derive(Default)]
struct Chips {
    list: Vec<Chip>,
    next_index: usize,
    depth: usize,
}

impl Chips {
    /// Adds the chip covering `chip`. A group that contains a function call
    /// is opened up instead: its opener (the bracket, with the name, dot or
    /// filter `?` around it), then the chips inside, then its closer — which
    /// reuses the opener's index. A group without one stays a single chip.
    fn push(&mut self, kind: Kind, query: &str, chip: Range<usize>) {
        let index = self.next_index;
        self.next_index += 1;
        let b = query.as_bytes();
        let open = group_open(b, &chip).filter(|_| self.depth < MAX_OPEN_DEPTH);
        if let Some(open) = open {
            let close = matching_close(kind, b, open);
            let is_filter = b[open] == b'[' && b.get(open + 1) == Some(&b'?');
            let body = open + 1 + usize::from(is_filter);
            let inner = body..close.unwrap_or(chip.end).max(body);
            if contains_call(kind, b, inner.clone()) {
                self.list.push((chip.start..body, index));
                self.depth += 1;
                scan(kind, query, inner, self);
                self.depth -= 1;
                // An unclosed group (half-typed text) has no closer yet.
                if let Some(close) = close {
                    self.list.push((close..close + 1, index));
                }
                return;
            }
        }
        self.list.push((chip, index));
    }
}

/// jq, JSONPath and JMESPath share one shape — steps joined by glue — so one
/// scanner covers them, with a few per-dialect switches. It reads `span` of
/// `query` (the whole query, or the inside of an opened-up group). Every
/// structural character is ASCII, so scanning bytes is safe: multi-byte
/// characters only appear inside word runs and strings, and a run always ends
/// on an ASCII byte or at the end, both char boundaries.
fn scan(kind: Kind, query: &str, span: Range<usize>, chips: &mut Chips) {
    let b = query.as_bytes();
    let end = span.end;
    let mut i = span.start;
    while i < end {
        let start = i;
        match b[i] {
            b'"' | b'\'' | b'`' => i = skip_string(kind, b, i),
            b'[' | b'(' | b'{' => i = skip_group(kind, b, i),
            b'.' => i = dot_step(kind, b, i),
            b'$' | b'@' => {
                i += 1;
                i = skip_word(b, i);
            }
            b'*' if kind == Kind::JmesPath => i += 1,
            b'#' if kind == Kind::Jq => {
                i = skip_comment(b, i);
                continue;
            }
            c if c.is_ascii_digit() => i = skip_number(b, i),
            c if is_word(c) => {
                i = skip_word(b, i);
                if b.get(i) == Some(&b'(') {
                    i = skip_group(kind, b, i);
                } else if kind == Kind::Jq && JQ_KEYWORDS.contains(&&query[start..i]) {
                    continue;
                }
            }
            // Pipes, commas, operators, whitespace, stray closers.
            _ => {
                i += 1;
                continue;
            }
        }
        // A step never runs past the group it is inside.
        i = i.min(end);
        chips.push(kind, query, start..i);
    }
}

/// The opening bracket of `chip` when it is a group — a bare `(`, `[` or `{`,
/// a call's parenthesis after its name, or a bracket glued to a dot (`.[0]`).
fn group_open(b: &[u8], chip: &Range<usize>) -> Option<usize> {
    let at = chip.start
        + b[chip.clone()]
            .iter()
            .position(|&c| c != b'.' && !is_word(c))?;
    matches!(b[at], b'(' | b'[' | b'{').then_some(at)
}

/// Whether `span` holds a function call — a name followed at once by `(`.
/// Strings and comments are skipped, and so is a name after a dot: `.name(`
/// is a member followed by a group, not a call.
fn contains_call(kind: Kind, b: &[u8], span: Range<usize>) -> bool {
    let mut i = span.start;
    while i < span.end {
        match b[i] {
            b'"' | b'\'' | b'`' => i = skip_string(kind, b, i),
            b'#' if kind == Kind::Jq => i = skip_comment(b, i),
            b'.' => {
                i += 1;
                if b.get(i) == Some(&b'.') {
                    i += 1;
                }
                i = skip_word(b, i);
            }
            b'$' | b'@' => i = skip_word(b, i + 1),
            c if c.is_ascii_digit() => i = skip_number(b, i),
            c if is_word(c) => {
                i = skip_word(b, i);
                if i < span.end && b[i] == b'(' {
                    return true;
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// A step that starts with `.`: `.` itself, `..` (recursive descent),
/// `.name`, `.name?`, `.*`, `."quoted"`, or a dot glued to a group
/// (`.[0]`, `.[a, b]`, `.{x: y}`).
fn dot_step(kind: Kind, b: &[u8], at: usize) -> usize {
    let mut i = at + 1;
    if b.get(i) == Some(&b'.') {
        i += 1;
    }
    match b.get(i) {
        Some(b'[' | b'{') => skip_group(kind, b, i),
        Some(b'"') => skip_string(kind, b, i),
        Some(b'*') => i + 1,
        Some(&c) if is_word(c) => {
            i = skip_word(b, i);
            if b.get(i) == Some(&b'?') {
                i += 1;
            }
            i
        }
        _ => i,
    }
}

/// From an opening `[`, `(` or `{`, to just past its matching closer —
/// or to the end of the text if it is never closed.
fn skip_group(kind: Kind, b: &[u8], at: usize) -> usize {
    matching_close(kind, b, at).map_or(b.len(), |close| close + 1)
}

/// The position of the closer matching the `[`, `(` or `{` at `at`, or `None`
/// if it is never closed. Strings inside are skipped so a bracket in a string
/// doesn't count.
fn matching_close(kind: Kind, b: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = at;
    while i < b.len() {
        match b[i] {
            b'[' | b'(' | b'{' => depth += 1,
            b']' | b')' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            b'"' | b'\'' | b'`' => {
                i = skip_string(kind, b, i);
                continue;
            }
            b'#' if kind == Kind::Jq => {
                i = skip_comment(b, i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A jq `# comment`: up to (not including) the end of the line.
fn skip_comment(b: &[u8], at: usize) -> usize {
    b[at..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(b.len(), |n| at + n)
}

/// From an opening quote to just past its closing one (or the end of the
/// text). A backslash escapes the next byte; in jq, `\(` opens an
/// interpolation, which is skipped as a group so quotes inside it don't end
/// the string.
fn skip_string(kind: Kind, b: &[u8], at: usize) -> usize {
    let quote = b[at];
    let mut i = at + 1;
    while i < b.len() {
        match b[i] {
            b'\\' if kind == Kind::Jq && b.get(i + 1) == Some(&b'(') => {
                i = skip_group(kind, b, i + 1);
            }
            b'\\' => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    b.len()
}

/// A number: digits, an optional fraction, an optional exponent letter and
/// digits (`30`, `3.14`, `1e5`). A `.` only belongs to it when a digit
/// follows, so `0.name` isn't swallowed.
fn skip_number(b: &[u8], at: usize) -> usize {
    let mut i = skip_word(b, at);
    if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i = skip_word(b, i + 1);
    }
    i
}

fn skip_word(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_word(b[i]) {
        i += 1;
    }
    i
}

/// Identifier characters, including every byte of a non-ASCII character so
/// `.名前` is one step.
fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tutorial;

    /// The chips of `query`, as the text each covers.
    fn chips(kind: Kind, query: &str) -> Vec<&str> {
        segments(kind, query)
            .into_iter()
            .filter_map(|(text, chip)| chip.map(|_| text))
            .collect()
    }

    #[test]
    fn jq_pipeline_tints_each_stage_and_leaves_the_pipes_plain() {
        assert_eq!(
            segments(Kind::Jq, ".members[] | select(.age > 30) | .name"),
            vec![
                (".members", Some(0)),
                ("[]", Some(1)),
                (" | ", None),
                ("select(.age > 30)", Some(2)),
                (" | ", None),
                (".name", Some(3)),
            ]
        );
    }

    #[test]
    fn jq_field_steps_and_indexes_are_separate_chips() {
        assert_eq!(chips(Kind::Jq, ".a.b[0].c"), vec![".a", ".b", "[0]", ".c"]);
        assert_eq!(chips(Kind::Jq, ".langs[1:3]"), vec![".langs", "[1:3]"]);
    }

    #[test]
    fn jq_dot_glued_to_a_bracket_is_one_chip() {
        assert_eq!(chips(Kind::Jq, ".[]"), vec![".[]"]);
        assert_eq!(chips(Kind::Jq, ".[0], .[-1]"), vec![".[0]", ".[-1]"]);
        assert_eq!(chips(Kind::Jq, "."), vec!["."]);
    }

    #[test]
    fn jq_quoted_keys_and_optional_steps() {
        assert_eq!(
            chips(Kind::Jq, r#".["first name"], ."a-b", .x?"#),
            vec![r#".["first name"]"#, r#"."a-b""#, ".x?"]
        );
    }

    #[test]
    fn jq_recursive_descent() {
        assert_eq!(chips(Kind::Jq, ".. | .a?"), vec!["..", ".a?"]);
    }

    #[test]
    fn jq_operators_are_glue_and_operands_are_chips() {
        assert_eq!(
            segments(Kind::Jq, ".age > 30"),
            vec![(".age", Some(0)), (" > ", None), ("30", Some(1))]
        );
        assert_eq!(chips(Kind::Jq, r#".a // "x""#), vec![".a", r#""x""#]);
    }

    #[test]
    fn jq_keywords_are_glue() {
        assert_eq!(
            chips(Kind::Jq, "if .a then .b else .c end"),
            vec![".a", ".b", ".c"]
        );
        assert_eq!(chips(Kind::Jq, ".a and .b"), vec![".a", ".b"]);
        // …but only as whole words.
        assert_eq!(chips(Kind::Jq, "ending"), vec!["ending"]);
        // …and only in jq: JMESPath has no such keywords.
        assert_eq!(chips(Kind::JmesPath, "end"), vec!["end"]);
    }

    #[test]
    fn jq_variables_calls_and_builtins() {
        assert_eq!(
            chips(Kind::Jq, ".[] as $p | $p.name | ascii_upcase"),
            vec![".[]", "$p", "$p", ".name", "ascii_upcase"]
        );
        assert_eq!(
            chips(Kind::Jq, r#"map(select(.a == "(")) | length"#),
            vec!["map(", r#"select(.a == "(")"#, ")", "length"]
        );
    }

    #[test]
    fn a_call_inside_a_call_gets_its_own_chip_and_the_closer_matches_the_opener() {
        assert_eq!(
            segments(Kind::Jq, ".members | map(select(.active) | .name)"),
            vec![
                (".members", Some(0)),
                (" | ", None),
                ("map(", Some(1)),
                ("select(.active)", Some(2)),
                (" | ", None),
                (".name", Some(3)),
                (")", Some(1)),
            ]
        );
        // Deeper nesting opens each level that holds a call.
        assert_eq!(
            segments(Kind::Jq, "map(select(.a | test(\"x\")) | .b)"),
            vec![
                ("map(", Some(0)),
                ("select(", Some(1)),
                (".a", Some(2)),
                (" | ", None),
                ("test(\"x\")", Some(3)),
                (")", Some(1)),
                (" | ", None),
                (".b", Some(4)),
                (")", Some(0)),
            ]
        );
    }

    #[test]
    fn a_group_without_a_call_stays_one_chip() {
        assert_eq!(chips(Kind::Jq, "map(.a | .b)"), vec!["map(.a | .b)"]);
        assert_eq!(chips(Kind::Jq, "(.a | length)"), vec!["(.a | length)"]);
        // A call-shaped string, comment or member is not a call.
        assert_eq!(
            chips(Kind::Jq, r#"select(.a == "f(x)")"#),
            vec![r#"select(.a == "f(x)")"#]
        );
        assert_eq!(
            chips(Kind::Jq, "select(\n  # f(x)\n  .a\n)"),
            vec!["select(\n  # f(x)\n  .a\n)"]
        );
        assert_eq!(chips(Kind::Jq, "select(.a.f(1))"), vec!["select(.a.f(1))"]);
    }

    #[test]
    fn any_bracket_holding_a_call_is_opened_up() {
        assert_eq!(
            chips(Kind::Jq, "[.[] | select(.a)]"),
            vec!["[", ".[]", "select(.a)", "]"]
        );
        assert_eq!(
            chips(Kind::Jq, "(.a | select(.b)) | length"),
            vec!["(", ".a", "select(.b)", ")", "length"]
        );
        assert_eq!(
            chips(Kind::Jq, "{a: length(.x)}"),
            vec!["{", "a", "length(.x)", "}"]
        );
        // A dot glued to the bracket belongs to the opener.
        assert_eq!(
            chips(Kind::Jq, ".[first(.a)]"),
            vec![".[", "first(.a)", "]"]
        );
        assert_eq!(
            chips(Kind::JmesPath, "people[?contains(name, 'a')].name"),
            vec!["people", "[?", "contains(name, 'a')", "]", ".name"]
        );
        assert_eq!(
            chips(Kind::JmesPath, "map(&length(skills), members)"),
            vec!["map(", "length(skills)", "members", ")"]
        );
        assert_eq!(
            chips(Kind::JsonPath, "$.store.book[?length(@.title) > 15].title"),
            vec![
                "$",
                ".store",
                ".book",
                "[?",
                "length(@.title)",
                "15",
                "]",
                ".title"
            ]
        );
    }

    #[test]
    fn absurdly_deep_nesting_neither_overflows_the_stack_nor_loses_text() {
        // Ten thousand groups deep would mean ten thousand recursive calls
        // (and a rescan of the rest of the query at every level) if the depth
        // were unbounded; this test thread has a 2 MB stack.
        for depth in [MAX_OPEN_DEPTH + 5, 10_000] {
            let query = format!("{}f(x){}", "(".repeat(depth), ")".repeat(depth));
            let parts = segments(Kind::Jq, &query);
            let joined: String = parts.iter().map(|(t, _)| *t).collect();
            assert_eq!(joined, query);
            // Opened up to the limit, then the rest is one chip.
            let openers = parts.iter().filter(|(t, _)| *t == "(").count();
            assert_eq!(openers, MAX_OPEN_DEPTH);
            let rest = depth - MAX_OPEN_DEPTH;
            let deepest = format!("{}f(x){}", "(".repeat(rest), ")".repeat(rest));
            assert!(parts.iter().any(|(t, _)| *t == deepest));
        }
    }

    #[test]
    fn a_half_typed_call_inside_a_call_is_opened_up_without_a_closer() {
        assert_eq!(
            segments(Kind::Jq, ".members | map(select(.active)"),
            vec![
                (".members", Some(0)),
                (" | ", None),
                ("map(", Some(1)),
                ("select(.active)", Some(2)),
            ]
        );
        assert_eq!(chips(Kind::Jq, "map(select("), vec!["map(", "select("]);
        assert_eq!(
            chips(Kind::Jq, "map(select(.a == \"x"),
            vec!["map(", "select(.a == \"x"]
        );
        // Until a `(` follows the inner name there is no call yet.
        assert_eq!(chips(Kind::Jq, "map(sel"), vec!["map(sel"]);
    }

    #[test]
    fn jq_array_and_object_construction_are_single_chips() {
        assert_eq!(chips(Kind::Jq, "[.a, .b]"), vec!["[.a, .b]"]);
        assert_eq!(
            chips(Kind::Jq, "{name: .name, n: (.a | length)}"),
            vec!["{name: .name, n: (.a | length)}"]
        );
    }

    #[test]
    fn jq_string_interpolation_may_contain_quotes() {
        let q = r#""hi \(.name + "!")" | length"#;
        assert_eq!(chips(Kind::Jq, q), vec![r#""hi \(.name + "!")""#, "length"]);
    }

    #[test]
    fn jq_numbers_and_format_strings() {
        assert_eq!(chips(Kind::Jq, ".a + 3.14 * 2"), vec![".a", "3.14", "2"]);
        assert_eq!(chips(Kind::Jq, "@csv"), vec!["@csv"]);
    }

    #[test]
    fn jsonpath_steps() {
        assert_eq!(
            chips(Kind::JsonPath, "$.store.bicycle.color"),
            vec!["$", ".store", ".bicycle", ".color"]
        );
        assert_eq!(
            chips(Kind::JsonPath, "$['store']['bicycle']"),
            vec!["$", "['store']", "['bicycle']"]
        );
    }

    #[test]
    fn jsonpath_filters_and_wildcards_and_descent() {
        assert_eq!(
            chips(Kind::JsonPath, "$.store.book[?@.price < 10].title"),
            vec!["$", ".store", ".book", "[?@.price < 10]", ".title"]
        );
        assert_eq!(chips(Kind::JsonPath, "$..price"), vec!["$", "..price"]);
        assert_eq!(chips(Kind::JsonPath, "$.a.*"), vec!["$", ".a", ".*"]);
        assert_eq!(chips(Kind::JsonPath, "$..*"), vec!["$", "..*"]);
        assert_eq!(
            chips(Kind::JsonPath, "$.a[?@.b == ']']"),
            vec!["$", ".a", "[?@.b == ']']"]
        );
    }

    #[test]
    fn jmespath_steps() {
        assert_eq!(
            chips(Kind::JmesPath, "people[?age > `30`].name"),
            vec!["people", "[?age > `30`]", ".name"]
        );
        assert_eq!(chips(Kind::JmesPath, "a.b.c"), vec!["a", ".b", ".c"]);
        assert_eq!(
            chips(Kind::JmesPath, "people[*].name | [0]"),
            vec!["people", "[*]", ".name", "[0]"]
        );
    }

    #[test]
    fn jmespath_calls_literals_and_projections() {
        assert_eq!(
            chips(Kind::JmesPath, "sort_by(people, &age)[0].name"),
            vec!["sort_by(people, &age)", "[0]", ".name"]
        );
        assert_eq!(
            chips(Kind::JmesPath, "a.{x: b, y: c}"),
            vec!["a", ".{x: b, y: c}"]
        );
        assert_eq!(
            chips(Kind::JmesPath, "*.name || 'none' && `[1, 2]`"),
            vec!["*", ".name", "'none'", "`[1, 2]`"]
        );
        // `*` is JMESPath's object projection, but jq's multiplication.
        assert_eq!(chips(Kind::Jq, ".a * .b"), vec![".a", ".b"]);
    }

    #[test]
    fn pointer_segments() {
        assert_eq!(
            segments(Kind::JsonPointer, "/store/book/0"),
            vec![("/store", Some(0)), ("/book", Some(1)), ("/0", Some(2))]
        );
        assert_eq!(
            chips(Kind::JsonPointer, "/a~1b/c d/"),
            vec!["/a~1b", "/c d", "/"]
        );
        // Whitespace outside the segments is glue.
        assert_eq!(
            segments(Kind::JsonPointer, " /a\n"),
            vec![(" ", None), ("/a", Some(0)), ("\n", None)]
        );
    }

    #[test]
    fn empty_and_blank_queries_have_no_chips() {
        for kind in Kind::ALL {
            assert_eq!(segments(kind, ""), vec![]);
            assert_eq!(segments(kind, "  \n "), vec![("  \n ", None)]);
        }
    }

    #[test]
    fn half_typed_queries_run_to_the_end_instead_of_failing() {
        assert_eq!(chips(Kind::Jq, ".a[0"), vec![".a", "[0"]);
        assert_eq!(
            chips(Kind::Jq, ".a | select(.b == \"x"),
            vec![".a", "select(.b == \"x"]
        );
        assert_eq!(chips(Kind::JsonPath, "$.a[?@.b"), vec!["$", ".a", "[?@.b"]);
        assert_eq!(chips(Kind::JmesPath, "a[`{\"k"), vec!["a", "[`{\"k"]);
        // A trailing backslash inside a string must not step past the end.
        assert_eq!(chips(Kind::Jq, "\"a\\"), vec!["\"a\\"]);
        // Stray closers are glue.
        assert_eq!(chips(Kind::Jq, ".a ) ] } .b"), vec![".a", ".b"]);
    }

    #[test]
    fn non_ascii_text_stays_on_char_boundaries() {
        assert_eq!(chips(Kind::Jq, ".名前 | .naïve"), vec![".名前", ".naïve"]);
        assert_eq!(
            chips(Kind::Jq, r#".["é"] | "ü(""#),
            vec![r#".["é"]"#, r#""ü(""#]
        );
        assert_eq!(chips(Kind::JsonPointer, "/é/ü"), vec!["/é", "/ü"]);
        assert_eq!(
            chips(Kind::Jq, ".名前 | map(select(.é) | .ü)"),
            vec![".名前", "map(", "select(.é)", ".ü", ")"]
        );
    }

    #[test]
    fn jq_comments_are_glue_up_to_the_end_of_the_line() {
        assert_eq!(
            chips(Kind::Jq, "# it's the name\n.name # (trailing\n| length"),
            vec![".name", "length"]
        );
        // A `#` inside a string or a group's own text is not special outside jq…
        assert_eq!(chips(Kind::Jq, r#""a#b""#), vec![r#""a#b""#]);
        assert_eq!(chips(Kind::JmesPath, "a#b"), vec!["a", "b"]);
        // …and a comment inside parentheses doesn't end the group early.
        assert_eq!(
            chips(Kind::Jq, "select(\n  # )\n  .a\n) | .b"),
            vec!["select(\n  # )\n  .a\n)", ".b"]
        );
    }

    /// Every slice of every query the tutorial teaches, plus each of its
    /// prefixes (what the box holds while that query is being typed), must
    /// reassemble to the query with consecutive chip indexes (a closer repeats
    /// its opener's) — which also proves no slice ever lands inside a
    /// multi-byte character.
    #[test]
    fn every_tutorial_query_and_prefix_reassembles() {
        let mut queries = 0;
        for kind in Kind::ALL {
            for topic in tutorial::topics(kind) {
                for lesson in topic.lessons {
                    for example in lesson.examples {
                        for end in
                            (0..=example.query.len()).filter(|&e| example.query.is_char_boundary(e))
                        {
                            let query = &example.query[..end];
                            let parts = segments(kind, query);
                            let joined: String = parts.iter().map(|(t, _)| *t).collect();
                            assert_eq!(joined, query, "{kind:?}");
                            let mut next = 0;
                            for index in parts.iter().filter_map(|(_, chip)| *chip) {
                                assert!(index <= next, "{kind:?}: {query}");
                                next = next.max(index + 1);
                            }
                            assert!(parts.iter().all(|(t, _)| !t.is_empty()));
                        }
                        queries += 1;
                        assert!(
                            !chips(kind, example.query).is_empty(),
                            "{kind:?}: {}",
                            example.query
                        );
                    }
                }
            }
        }
        assert!(queries > 100, "only {queries} tutorial queries visited");
    }
}
