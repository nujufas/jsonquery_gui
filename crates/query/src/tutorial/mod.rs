//! Content and helpers for the in-app tutorial window: for each query
//! dialect ([`Kind`]), a tree of [`Topic`]s holding [`Lesson`]s, each with a
//! few short [`Example`]s.
//!
//! This is pure data plus small pure helpers — no egui — so it can be
//! unit-tested against the real engines. That is the point of the tests
//! below: every example's query is *run* (the window shows the live result,
//! it never shows a hand-copied one), every highlighted fragment must occur
//! in its query, and every sample document must be valid JSON, so a lesson
//! can't silently rot when an engine is upgraded or a query is edited.
//!
//! The egui side lives in `crates/app/src/tutorial.rs`; the split mirrors
//! [`crate::suggest`] / `query_suggest.rs`.

use std::ops::Range;
use std::sync::atomic::AtomicBool;

use jsonquery_core::engine::QueryEvent;
use serde_json::Value;

use crate::Kind;

mod jmespath;
mod jq;
mod jsonpath;
mod pointer;

/// One highlighted fragment of an example's query plus what it does, e.g.
/// `(".age > 30", "keep members older than 30")`. Fragments are matched
/// against the query *in order* (see [`segments`]), so a fragment that
/// appears twice is found at its first occurrence after the previous one.
pub type Part = (&'static str, &'static str);

/// A single query over a single sample document, with a breakdown of what
/// each piece of the query means.
#[derive(Clone, Copy)]
pub struct Example {
    /// One plain-English line: what this example accomplishes.
    pub caption: &'static str,
    /// The sample document, as pretty JSON text (shown as-is in the window
    /// and loaded verbatim by "Load data").
    pub data: &'static str,
    pub query: &'static str,
    /// Fragment-by-fragment explanation, in the order the fragments occur.
    pub parts: &'static [Part],
    /// The example is *meant* to fail, to show what the error looks like
    /// (e.g. a JSON Pointer to a value that isn't there).
    pub fails: bool,
}

impl Example {
    pub const fn new(
        caption: &'static str,
        data: &'static str,
        query: &'static str,
        parts: &'static [Part],
    ) -> Self {
        Self {
            caption,
            data,
            query,
            parts,
            fails: false,
        }
    }

    /// Marks an example as one that is expected to produce an error.
    pub const fn failing(mut self) -> Self {
        self.fails = true;
        self
    }
}

/// A compact "syntax → meaning" reference table over one sample document,
/// for the last lesson of each dialect.
#[derive(Clone, Copy)]
pub struct CheatSheet {
    pub data: &'static str,
    pub rows: &'static [(&'static str, &'static str)],
}

/// One page of the tutorial: a short explanation, one to three examples
/// (or a cheat sheet), and a few tips. Text-only lessons carry just tips.
#[derive(Clone, Copy)]
pub struct Lesson {
    pub title: &'static str,
    /// One or two sentences. May contain `` `code` `` (see [`parse_inline`]).
    pub summary: &'static str,
    pub examples: &'static [Example],
    /// Short "good to know" bullets, same inline markup as `summary`.
    pub tips: &'static [&'static str],
    pub cheat_sheet: Option<CheatSheet>,
}

impl Lesson {
    pub const fn new(
        title: &'static str,
        summary: &'static str,
        examples: &'static [Example],
    ) -> Self {
        Self {
            title,
            summary,
            examples,
            tips: &[],
            cheat_sheet: None,
        }
    }

    pub const fn tips(mut self, tips: &'static [&'static str]) -> Self {
        self.tips = tips;
        self
    }

    pub const fn cheat_sheet(
        title: &'static str,
        summary: &'static str,
        data: &'static str,
        rows: &'static [(&'static str, &'static str)],
    ) -> Self {
        Self {
            title,
            summary,
            examples: &[],
            tips: &[],
            cheat_sheet: Some(CheatSheet { data, rows }),
        }
    }
}

/// A group of related lessons — one expandable node in the left-hand tree.
#[derive(Clone, Copy)]
pub struct Topic {
    pub title: &'static str,
    pub lessons: &'static [Lesson],
}

impl Topic {
    pub const fn new(title: &'static str, lessons: &'static [Lesson]) -> Self {
        Self { title, lessons }
    }
}

/// The topic tree for one dialect.
pub fn topics(kind: Kind) -> &'static [Topic] {
    match kind {
        Kind::Jq => jq::TOPICS,
        Kind::JsonPointer => pointer::TOPICS,
        Kind::JsonPath => jsonpath::TOPICS,
        Kind::JmesPath => jmespath::TOPICS,
    }
}

/// Tab title (the engine picker's own [`Kind::label`] says just "Pointer").
pub fn title(kind: Kind) -> &'static str {
    match kind {
        Kind::Jq => "jq",
        Kind::JsonPointer => "JSON Pointer",
        Kind::JsonPath => "JSONPath",
        Kind::JmesPath => "JMESPath",
    }
}

/// One line describing the dialect, shown under the tabs.
pub fn tagline(kind: Kind) -> &'static str {
    match kind {
        Kind::Jq => "A filter language: a program is a pipeline that streams JSON values through small filters.",
        Kind::JsonPointer => "RFC 6901 — a plain string path that names exactly one value.",
        Kind::JsonPath => "RFC 9535 — XPath-style queries that return a list of every match.",
        Kind::JmesPath => "Projections, filters and built-in functions — the language behind the AWS CLI's --query.",
    }
}

/// Whether `lesson` mentions `needle` (already lower-cased) anywhere a
/// reader might search for it: title, summary, or any example query.
pub fn lesson_matches(lesson: &Lesson, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hit = |s: &str| s.to_lowercase().contains(needle);
    hit(lesson.title)
        || hit(lesson.summary)
        || lesson
            .examples
            .iter()
            .any(|e| hit(e.query) || hit(e.caption))
        || lesson
            .cheat_sheet
            .is_some_and(|c| c.rows.iter().any(|(q, d)| hit(q) || hit(d)))
}

/// What running an example produced, in the shape the window shows it.
#[derive(Debug, Default, Clone)]
pub struct Outcome {
    pub items: Vec<Value>,
    /// Per-item evaluation errors (jq can fail on some inputs mid-stream).
    pub item_errors: Vec<String>,
    /// The query itself failed (syntax error, or a JSON Pointer that names
    /// nothing).
    pub error: Option<String>,
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        self.error.is_none() && self.item_errors.is_empty()
    }
}

/// Runs `query` against the JSON text `data` with `kind`'s real engine.
pub fn run(kind: Kind, data: &str, query: &str) -> Outcome {
    let mut out = Outcome::default();
    let input: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            out.error = Some(format!("the sample data isn't valid JSON: {e}"));
            return out;
        }
    };
    let cancelled = AtomicBool::new(false);
    let result = kind
        .engine()
        .run(&input, query, &cancelled, &mut |event| match event {
            QueryEvent::Item(v) => out.items.push(v),
            QueryEvent::ItemError(e) => out.item_errors.push(e),
        });
    if let Err(e) = result {
        out.error = Some(e.to_string());
    }
    out
}

/// One result value as display text. Short values stay on one line; a long
/// array puts one element per line (each compact if it fits — a list of
/// records reads far better that way than fully pretty-printed); anything
/// else long is pretty-printed.
pub fn format_value(value: &Value) -> String {
    const LINE: usize = 88;
    let compact = serde_json::to_string(value).unwrap_or_default();
    if compact.chars().count() <= LINE {
        return compact;
    }
    match value {
        Value::Array(items) => {
            let lines: Vec<String> = items
                .iter()
                .map(|item| {
                    format_value(item)
                        .lines()
                        .map(|l| format!("  {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .collect();
            format!("[\n{}\n]", lines.join(",\n"))
        }
        _ => serde_json::to_string_pretty(value).unwrap_or(compact),
    }
}

/// Splits `query` into consecutive slices, tagging each slice that one of
/// `parts` explains with that part's index. Fragments are searched for in
/// order, each starting where the previous one ended; text between them is
/// returned untagged, so the pieces always concatenate back to `query`. A
/// fragment that can't be found is skipped (the content tests forbid that).
///
/// A fragment that opens a bracket it doesn't close (`map(`, `[`, `{name`)
/// also tints the bracket that closes it, under the same index, so the pair
/// reads as one — unless another fragment already covers that closer.
pub fn segments<'q>(query: &'q str, parts: &[Part]) -> Vec<(&'q str, Option<usize>)> {
    let mut tinted: Vec<(Range<usize>, usize)> = Vec::new();
    let mut cursor = 0;
    for (i, (fragment, _)) in parts.iter().enumerate() {
        if fragment.is_empty() {
            continue;
        }
        let Some(offset) = query[cursor..].find(fragment) else {
            continue;
        };
        let start = cursor + offset;
        tinted.push((start..start + fragment.len(), i));
        cursor = start + fragment.len();
    }
    let closers: Vec<(Range<usize>, usize)> = tinted
        .iter()
        .flat_map(|(fragment, i)| {
            closers_of(query.as_bytes(), fragment.clone())
                .into_iter()
                .map(move |at| (at..at + 1, *i))
        })
        .filter(|(closer, _)| !tinted.iter().any(|(t, _)| t.contains(&closer.start)))
        .collect();
    tinted.extend(closers);
    tinted.sort_by_key(|(range, _)| range.start);

    let mut out = Vec::new();
    let mut cursor = 0;
    for (range, i) in tinted {
        if range.start > cursor {
            out.push((&query[cursor..range.start], None));
        }
        out.push((&query[range.clone()], Some(i)));
        cursor = range.end;
    }
    if cursor < query.len() {
        out.push((&query[cursor..], None));
    }
    out
}

/// The positions in `b` of the brackets that close the ones `fragment` opens
/// and leaves open — none for a fragment whose brackets balance. Quoted text
/// doesn't count, so a `)` in a string is not a closer.
fn closers_of(b: &[u8], fragment: Range<usize>) -> Vec<usize> {
    let mut unclosed = 0usize;
    let mut i = fragment.start;
    while i < fragment.end {
        match b[i] {
            b'(' | b'[' | b'{' => unclosed += 1,
            b')' | b']' | b'}' => unclosed = unclosed.saturating_sub(1),
            b'"' | b'\'' | b'`' => {
                i = skip_quoted(b, i, fragment.end);
                continue;
            }
            _ => {}
        }
        i += 1;
    }

    let mut closers = Vec::new();
    let mut nested = 0usize;
    let mut i = fragment.end;
    while unclosed > 0 && i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => nested += 1,
            b')' | b']' | b'}' if nested > 0 => nested -= 1,
            b')' | b']' | b'}' => {
                closers.push(i);
                unclosed -= 1;
            }
            b'"' | b'\'' | b'`' => {
                i = skip_quoted(b, i, b.len());
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    closers
}

/// From an opening quote to just past its closing one, or to `end`.
fn skip_quoted(b: &[u8], at: usize, end: usize) -> usize {
    let quote = b[at];
    let mut i = at + 1;
    while i < end {
        match b[i] {
            b'\\' => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    end
}

/// A run of prose from [`parse_inline`].
#[derive(Debug, PartialEq, Eq)]
pub enum Span<'a> {
    Text(&'a str),
    Code(&'a str),
    /// `*emphasis*`, shown in italics.
    Emph(&'a str),
}

/// Splits prose into plain text, inline code and emphasis, markdown-style:
/// a span between single backticks is code, and `*between asterisks*` is
/// emphasis. JMESPath's own literals use backticks, so a code span may
/// instead be fenced by *double* backticks (and its content trimmed), which
/// lets it contain single ones: ``` `` `30` `` ``` is the code `` `30` ``.
/// Backticks or asterisks that don't close stay in the text; an asterisk
/// only opens emphasis at a word start and next to non-space text, so a
/// bare `*` wildcard in prose is left alone.
pub fn parse_inline(text: &str) -> Vec<Span<'_>> {
    let bytes = text.as_bytes();
    let run_len = |at: usize| bytes[at..].iter().take_while(|&&b| b == b'`').count();

    let mut spans = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            let at_word_start = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let opens = at_word_start
                && bytes
                    .get(i + 1)
                    .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'*');
            let close = opens
                .then(|| {
                    (i + 2..bytes.len())
                        .find(|&j| bytes[j] == b'*' && !bytes[j - 1].is_ascii_whitespace())
                })
                .flatten()
                .filter(|&j| !bytes[i + 1..j].contains(&b'`'));
            if let Some(close) = close {
                if text_start < i {
                    spans.push(Span::Text(&text[text_start..i]));
                }
                spans.push(Span::Emph(&text[i + 1..close]));
                i = close + 1;
                text_start = i;
            } else {
                i += 1;
            }
            continue;
        }
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let open = run_len(i);
        let body_start = i + open;

        let mut j = body_start;
        let mut close = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let n = run_len(j);
                if n == open {
                    close = Some(j);
                    break;
                }
                j += n;
            } else {
                j += 1;
            }
        }

        match close {
            Some(close) => {
                if text_start < i {
                    spans.push(Span::Text(&text[text_start..i]));
                }
                let body = &text[body_start..close];
                spans.push(Span::Code(if open > 1 { body.trim() } else { body }));
                i = close + open;
                text_start = i;
            }
            None => i = body_start,
        }
    }
    if text_start < text.len() {
        spans.push(Span::Text(&text[text_start..]));
    }
    spans
}

// ---------------------------------------------------------------------------
// Sample documents shared across lessons. A dialect's lessons deliberately
// reuse one or two documents so a reader learns the data once, and jq and
// JMESPath share `TEAM` so the same question can be compared across the two.
// ---------------------------------------------------------------------------

/// jq and JMESPath: a small team. Has a `null` e-mail (Linus) and a missing
/// one (Grace) to exercise default/missing-value handling.
const TEAM: &str = r#"{
  "team": "Platform",
  "office": { "city": "Berlin", "floor": 3 },
  "members": [
    { "name": "Ada", "age": 36, "role": "lead", "active": true,
      "email": "ada@example.com", "skills": ["rust", "sql"] },
    { "name": "Linus", "age": 28, "role": "dev", "active": true,
      "email": null, "skills": ["c", "shell"] },
    { "name": "Grace", "age": 45, "role": "dev", "active": false,
      "skills": ["cobol"] },
    { "name": "Alan", "age": 41, "role": "qa", "active": true,
      "email": "alan@example.com", "skills": [] }
  ]
}"#;

/// JSONPath: the bookstore from the JSONPath specification.
const STORE: &str = r#"{
  "store": {
    "book": [
      { "title": "Sayings of the Century", "author": "Nigel Rees",
        "category": "reference", "price": 8.95 },
      { "title": "Sword of Honour", "author": "Evelyn Waugh",
        "category": "fiction", "price": 12.99 },
      { "title": "Moby Dick", "author": "Herman Melville",
        "category": "fiction", "price": 8.99, "isbn": "0-553-21311-3" },
      { "title": "The Lord of the Rings", "author": "J. R. R. Tolkien",
        "category": "fiction", "price": 22.99, "isbn": "0-395-19395-8" }
    ],
    "bicycle": { "color": "red", "price": 19.95 }
  }
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every example and cheat-sheet row of every dialect, with where it lives.
    fn all_queries() -> Vec<(Kind, String, &'static str, &'static str, bool)> {
        let mut out = Vec::new();
        for kind in Kind::ALL {
            for topic in topics(kind) {
                for lesson in topic.lessons {
                    let at = format!("{} › {} › {}", title(kind), topic.title, lesson.title);
                    for e in lesson.examples {
                        out.push((kind, at.clone(), e.data, e.query, e.fails));
                    }
                    if let Some(sheet) = lesson.cheat_sheet {
                        for (query, _) in sheet.rows {
                            out.push((kind, at.clone(), sheet.data, query, false));
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn every_example_runs_as_declared() {
        for (kind, at, data, query, fails) in all_queries() {
            let outcome = run(kind, data, query);
            if fails {
                assert!(
                    outcome.error.is_some(),
                    "{at}: `{query}` is marked failing but succeeded"
                );
            } else {
                assert!(
                    outcome.is_ok(),
                    "{at}: `{query}` failed: {:?} {:?}",
                    outcome.error,
                    outcome.item_errors
                );
            }
        }
    }

    #[test]
    fn every_highlighted_fragment_occurs_in_its_query_in_order() {
        for kind in Kind::ALL {
            for topic in topics(kind) {
                for lesson in topic.lessons {
                    for e in lesson.examples {
                        let mut cursor = 0;
                        for (fragment, note) in e.parts {
                            assert!(!fragment.is_empty(), "{}: empty fragment", lesson.title);
                            assert!(
                                !note.is_empty(),
                                "{}: `{fragment}` has no note",
                                lesson.title
                            );
                            let found = e.query[cursor..].find(fragment).unwrap_or_else(|| {
                                panic!(
                                    "{} › {}: fragment `{fragment}` not found in order in `{}`",
                                    title(kind),
                                    lesson.title,
                                    e.query
                                )
                            });
                            cursor += found + fragment.len();
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sample_documents_are_valid_json() {
        let mut docs = vec![TEAM, STORE];
        for kind in Kind::ALL {
            for topic in topics(kind) {
                for lesson in topic.lessons {
                    docs.extend(lesson.examples.iter().map(|e| e.data));
                    docs.extend(lesson.cheat_sheet.map(|c| c.data));
                }
            }
        }
        for doc in docs {
            serde_json::from_str::<Value>(doc)
                .unwrap_or_else(|e| panic!("invalid sample JSON ({e}):\n{doc}"));
        }
    }

    #[test]
    fn every_lesson_has_something_to_show_and_titles_are_unique() {
        for kind in Kind::ALL {
            assert!(!topics(kind).is_empty(), "{} has no topics", title(kind));
            let mut seen = std::collections::HashSet::new();
            for topic in topics(kind) {
                assert!(!topic.lessons.is_empty(), "{}: empty topic", topic.title);
                for lesson in topic.lessons {
                    assert!(
                        seen.insert(lesson.title),
                        "{}: duplicate lesson title `{}`",
                        title(kind),
                        lesson.title
                    );
                    assert!(!lesson.summary.is_empty(), "{}: no summary", lesson.title);
                    assert!(
                        !lesson.examples.is_empty()
                            || lesson.cheat_sheet.is_some()
                            || !lesson.tips.is_empty(),
                        "{}: lesson has no content",
                        lesson.title
                    );
                }
            }
        }
    }

    /// `cargo test -p jsonquery-query tutorial -- --ignored --nocapture`
    /// prints every example's live result, for proof-reading the prose
    /// ("→ 37.5", "four results") against what the engines really return.
    #[test]
    #[ignore = "prints every example's live result for manual review"]
    fn dump_every_example() {
        let (mut lessons, mut examples) = (0, 0);
        for kind in Kind::ALL {
            println!("\n################ {}", title(kind));
            for topic in topics(kind) {
                for lesson in topic.lessons {
                    lessons += 1;
                    println!("\n== {} › {}", topic.title, lesson.title);
                    for e in lesson.examples {
                        examples += 1;
                        let out = run(kind, e.data, e.query);
                        println!("  {}\n    {}", e.caption, e.query);
                        match &out.error {
                            Some(err) => println!("    ERROR: {err}"),
                            None => {
                                println!("    {} result(s)", out.items.len());
                                for v in &out.items {
                                    let s = format_value(v);
                                    println!(
                                        "      {}",
                                        s.lines().collect::<Vec<_>>().join("\n      ")
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        println!("\n{lessons} lessons, {examples} examples");
    }

    #[test]
    fn jq_is_the_first_tab() {
        assert_eq!(Kind::ALL[0], Kind::Jq);
    }

    #[test]
    fn segments_reassemble_the_query_and_tag_the_parts() {
        let query = ".members[] | select(.age > 30) | .name";
        let parts: &[Part] = &[
            (".members[]", "a"),
            ("select(.age > 30)", "b"),
            (".name", "c"),
        ];
        let segs = segments(query, parts);
        assert_eq!(segs.iter().map(|(s, _)| *s).collect::<String>(), query);
        assert_eq!(
            segs.iter().filter_map(|(_, i)| *i).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(segs[1], (" | ", None));
    }

    #[test]
    fn a_fragment_that_opens_a_bracket_also_tints_the_closer() {
        // The closer is tinted with its opener's index, not left plain.
        let segs = segments(
            ".members | map(select(.active) | .name)",
            &[("map(", "a"), ("select(.active)", "b"), ("| .name", "c")],
        );
        assert_eq!(
            segs,
            vec![
                (".members | ", None),
                ("map(", Some(0)),
                ("select(.active)", Some(1)),
                (" ", None),
                ("| .name", Some(2)),
                (")", Some(0)),
            ]
        );
        // Every bracket the fragment opens is closed, however deep the text
        // between, and a `)` inside a string is not a closer.
        let segs = segments(r#"a(b(")") | c)"#, &[("a(", "x")]);
        assert_eq!(segs.last(), Some(&(")", Some(0))));
        assert_eq!(segs.iter().filter(|(_, i)| i.is_some()).count(), 2);
        let segs = segments("f(g(x))", &[("f(g(", "x")]);
        assert_eq!(
            segs,
            vec![
                ("f(g(", Some(0)),
                ("x", None),
                (")", Some(0)),
                (")", Some(0))
            ]
        );
    }

    #[test]
    fn a_closer_another_fragment_already_covers_keeps_that_fragments_tint() {
        let segs = segments("[.a]", &[("[", "open"), (".a", "body"), ("]", "close")]);
        assert_eq!(segs, vec![("[", Some(0)), (".a", Some(1)), ("]", Some(2))]);
    }

    #[test]
    fn balanced_fragments_and_missing_closers_add_nothing() {
        let segs = segments("f(a) | g(b)", &[("f(a)", "x")]);
        assert_eq!(segs.iter().filter(|(_, i)| i.is_some()).count(), 1);
        // Half a query: the closer isn't there yet.
        let segs = segments("map(.a", &[("map(", "x")]);
        assert_eq!(segs, vec![("map(", Some(0)), (".a", None)]);
    }

    #[test]
    fn segments_find_repeated_fragments_in_order() {
        let segs = segments(".a, .a", &[(".a", "x"), (".a", "y")]);
        assert_eq!(segs, vec![(".a", Some(0)), (", ", None), (".a", Some(1))]);
    }

    #[test]
    fn inline_code_spans() {
        assert_eq!(
            parse_inline("use `.name` here"),
            vec![Span::Text("use "), Span::Code(".name"), Span::Text(" here")]
        );
        // Double backticks let a span contain JMESPath's backtick literals.
        assert_eq!(
            parse_inline("numbers are `` `30` `` literals"),
            vec![
                Span::Text("numbers are "),
                Span::Code("`30`"),
                Span::Text(" literals")
            ]
        );
        // An unclosed backtick stays literal text.
        assert_eq!(parse_inline("a ` b"), vec![Span::Text("a ` b")]);
        assert_eq!(parse_inline("plain"), vec![Span::Text("plain")]);
    }

    #[test]
    fn inline_emphasis_leaves_stray_asterisks_alone() {
        assert_eq!(
            parse_inline("a *filter* here"),
            vec![Span::Text("a "), Span::Emph("filter"), Span::Text(" here")]
        );
        // Asterisks inside code are code, not emphasis.
        assert_eq!(
            parse_inline("`[*]` and `.*` both"),
            vec![
                Span::Code("[*]"),
                Span::Text(" and "),
                Span::Code(".*"),
                Span::Text(" both")
            ]
        );
        // A lone or space-adjacent asterisk stays text.
        assert_eq!(parse_inline("2 * 3 * 4"), vec![Span::Text("2 * 3 * 4")]);
        assert_eq!(parse_inline("a * b"), vec![Span::Text("a * b")]);
        // Mid-word asterisks (e.g. `a*b*c`) aren't emphasis.
        assert_eq!(parse_inline("a*b*c"), vec![Span::Text("a*b*c")]);
    }

    #[test]
    fn values_format_compactly_unless_long() {
        use serde_json::json;
        assert_eq!(format_value(&json!({"a": [1, 2]})), r#"{"a":[1,2]}"#);

        // A long object is pretty-printed…
        let long = json!({"text": "x".repeat(100)});
        assert!(format_value(&long).contains('\n'));

        // …but a long array of short records is one record per line.
        let records = json!([
            {"name": "Ada", "role": "lead", "skills": ["rust", "sql"]},
            {"name": "Linus", "role": "dev", "skills": ["c", "shell"]},
        ]);
        assert_eq!(
            format_value(&records),
            "[\n  {\"name\":\"Ada\",\"role\":\"lead\",\"skills\":[\"rust\",\"sql\"]},\n  \
             {\"name\":\"Linus\",\"role\":\"dev\",\"skills\":[\"c\",\"shell\"]}\n]"
        );
    }

    #[test]
    fn filter_matches_titles_queries_and_cheat_rows() {
        let lesson = topics(Kind::Jq)[0].lessons[0];
        assert!(lesson_matches(&lesson, ""));
        assert!(lesson_matches(&lesson, &lesson.title.to_lowercase()));
        assert!(!lesson_matches(&lesson, "zzz-no-such-text"));
    }

    /// The tutorial tells readers certain things don't work in this build.
    /// If an engine upgrade makes one of them work, this fails so the
    /// matching tip gets removed instead of quietly going stale.
    #[test]
    fn documented_gaps_are_still_real() {
        let team = |kind, query| run(kind, TEAM, query);
        for query in [
            r#".members[] | [.name] | @csv"#,
            r#".members[] | [.name] | @tsv"#,
            r#".members[] | select(.role | IN("dev"))"#,
            r#".members | INDEX(.name)"#,
            r#"$ENV | type"#,
            r#"[.members[0] | tostream]"#,
            r#"[.members[0] | leaf_paths]"#,
            r#".members | toarray"#,
            r#"{} | .a.b.c = 1"#,
        ] {
            assert!(
                !team(Kind::Jq, query).is_ok(),
                "`{query}` now works — update the \"jq here vs jq 1.7\" lesson"
            );
        }

        // JMESPath decimal backtick literals come back mangled (the
        // `arbitrary_precision` feature of serde_json changes how the
        // jmespath crate's own literal parser sees a float).
        let prices = r#"{"items": [{"n": "a", "price": 8.95}, {"n": "b", "price": 22.5}]}"#;
        let out = run(Kind::JmesPath, prices, "items[?price > `9.5`].n");
        assert_eq!(
            out.items,
            vec![serde_json::json!([])],
            "decimal JMESPath literals work now — drop the tip in the JMESPath \
             \"Filters & literals\" lesson"
        );
    }
}
