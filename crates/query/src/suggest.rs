//! Query-box autocomplete: per-dialect keyword/function tables, plus
//! document-aware field-name completion, feeding the "Query:" box's
//! suggestion popup (`crates/app/src/app.rs`). Kept in this crate rather
//! than `app` so the dialect-specific knowledge lives next to [`Kind`] and
//! can be unit-tested without an egui context.
//!
//! # Engine scope
//! [`engines_in_scope`] decides which dialect(s)' keyword candidates to
//! show:
//! - an explicit engine-picker selection always wins outright;
//! - otherwise, an unambiguous leading marker (`/`, `$`, a JMESPath-only
//!   substring, or jq's leading `.`) narrows to that one dialect;
//! - a bare, marker-less path (`foo`, `foo.bar`) is genuinely ambiguous
//!   between jq and JMESPath — the two dialects that share this syntax —
//!   so both are shown;
//! - an empty box has learned nothing yet, so all four are shown.
//!
//! This mirrors [`crate::Kind::detect`]'s markers but does *not* fall back
//! to jq the way `detect` does at query-run time: `detect` must always
//! commit to something to actually run the query, whereas the popup's job
//! is to stay honest about what's still ambiguous.
//!
//! # Document-aware completion
//! [`suggest`] separately tries to parse the query text up to the cursor as
//! a simple field/index path (see [`parse_dotted_path`] and
//! [`parse_pointer_path`]) and, if it parses and a document is loaded,
//! resolves it via [`jsonquery_core::resolve`] to offer the real field
//! names or array indices found there. A query that has moved past a plain
//! path — a pipe, a function call, a filter, ... — falls back to
//! keyword-only suggestions; this is a deliberate scope limit (correctly
//! tracking "where am I" through arbitrary jq/JMESPath expressions is a much
//! bigger undertaking), not an oversight.

use std::ops::Range;

use jsonquery_core::{resolve, PathSegment};
use serde_json::Value;

use crate::Kind;

/// Markers that mean "this is JMESPath" even without JSON Pointer's `/` or
/// JSONPath's `$` — see [`crate::Kind::detect`], which this mirrors.
const JMESPATH_MARKERS: [&str; 4] = ["[?", "&&", "||", "`"];

/// Cap on how many document-derived (field/index) candidates `suggest`
/// returns, so a huge object doesn't dump thousands of rows into the popup.
const DOC_CANDIDATE_CAP: usize = 50;
/// Cap on how many static keyword/function candidates `suggest` returns.
const KEYWORD_CANDIDATE_CAP: usize = 30;

/// One completion candidate offered in the query box's popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Byte range of the query text this candidate replaces if accepted.
    pub replace: Range<usize>,
    /// Text inserted in place of `replace`.
    pub insert: String,
    /// What the popup row shows as its main label.
    pub label: String,
    /// Short one-line detail shown alongside `label` (a function's
    /// signature, or the JSON value found at a suggested field/index).
    pub detail: String,
}

/// Which dialect(s) should supply keyword candidates for `query` (the text
/// typed so far, up to the cursor) — see the module docs' "Engine scope".
pub fn engines_in_scope(explicit: Option<Kind>, query: &str) -> Vec<Kind> {
    if let Some(k) = explicit {
        return vec![k];
    }
    let trimmed = query.trim_start();
    if trimmed.is_empty() {
        return Kind::ALL.to_vec();
    }
    if trimmed.starts_with('/') {
        return vec![Kind::JsonPointer];
    }
    if trimmed.starts_with('$') {
        return vec![Kind::JsonPath];
    }
    if JMESPATH_MARKERS.iter().any(|m| trimmed.contains(m)) {
        return vec![Kind::JmesPath];
    }
    if trimmed.starts_with('.') {
        return vec![Kind::Jq];
    }
    // A bare word/path with none of the above markers: valid syntax in
    // either jq or JMESPath, and not yet distinguishable between them.
    vec![Kind::Jq, Kind::JmesPath]
}

/// Compute completion candidates for `query_text`, with the cursor at byte
/// offset `cursor` (must be a char boundary). `explicit` is the
/// engine-picker's current selection (`None` = auto); `doc_root` is the
/// loaded document's root value, if any, for field/index completion.
pub fn suggest(
    query_text: &str,
    cursor: usize,
    explicit: Option<Kind>,
    doc_root: Option<&Value>,
) -> Vec<Suggestion> {
    let before = &query_text[..cursor];
    if before.is_empty() {
        // Nothing typed yet: no leading-dot-vs-bare-word ambiguity to
        // create false confidence about, and no keyword prefix to filter
        // by, so showing something would just be the entire multi-hundred-
        // entry union of all four dialects' tables. Wait for a first
        // keystroke instead.
        return Vec::new();
    }

    let scope = engines_in_scope(explicit, before);

    let word_start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(cursor);
    let word = &before[word_start..cursor];

    let mut out = doc_candidates(&scope, before, cursor, doc_root);
    out.truncate(DOC_CANDIDATE_CAP);

    // Skip the keyword dump when there's nothing to filter it by *and*
    // we've already got something more specific (real field names) to
    // show — but still fall back to it when there's no document context at
    // all, so `.` in an empty box isn't a dead end.
    if !word.is_empty() || out.is_empty() {
        let mut kw = keyword_candidates(&scope, word, word_start..cursor);
        kw.truncate(KEYWORD_CANDIDATE_CAP);
        out.extend(kw);
    }

    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn keyword_candidates(scope: &[Kind], word: &str, replace: Range<usize>) -> Vec<Suggestion> {
    let tag_engines = scope.len() > 1;
    let word_lower = word.to_lowercase();

    let mut matches: Vec<(u8, &Candidate)> = Vec::new();
    for kind in scope {
        for cand in builtins(*kind) {
            let text_lower = cand.text.to_lowercase();
            let score = if word.is_empty() || text_lower.starts_with(&word_lower) {
                Some(0)
            } else if text_lower.contains(&word_lower) {
                Some(1)
            } else {
                None
            };
            if let Some(score) = score {
                matches.push((score, cand));
            }
        }
    }
    matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.text.cmp(b.1.text)));
    matches.dedup_by(|a, b| a.1.text == b.1.text && a.1.detail == b.1.detail);

    matches
        .into_iter()
        .map(|(_, cand)| Suggestion {
            replace: replace.clone(),
            insert: cand.text.to_string(),
            label: if tag_engines {
                format!("{}  [{}]", cand.text, cand.kind.label())
            } else {
                cand.text.to_string()
            },
            detail: cand.detail.to_string(),
        })
        .collect()
}

/// Document-aware field/index candidates: tries to parse `before` (the
/// whole query up to the cursor) as a simple path in whichever grammar
/// `scope` implies, resolves it against `doc_root`, and lists the resolved
/// container's children that match the trailing partial segment.
fn doc_candidates(
    scope: &[Kind],
    before: &str,
    cursor: usize,
    doc_root: Option<&Value>,
) -> Vec<Suggestion> {
    let Some(root) = doc_root else {
        return Vec::new();
    };

    let parsed = if scope == [Kind::JsonPointer] {
        let trimmed = before.trim_start();
        trimmed
            .starts_with('/')
            .then(|| parse_pointer_path(trimmed))
    } else if scope == [Kind::JsonPath] {
        before
            .trim_start()
            .strip_prefix('$')
            .and_then(parse_dotted_path)
    } else if scope.iter().any(|k| matches!(k, Kind::Jq | Kind::JmesPath)) {
        parse_dotted_path(before)
    } else {
        None
    };
    let Some(parsed) = parsed else {
        return Vec::new();
    };

    let Some(parent) = resolve(root, &parsed.segments) else {
        return Vec::new();
    };

    let partial_start = cursor - parsed.partial.len();
    let partial_lower = parsed.partial.to_lowercase();

    match parent {
        Value::Object(map) => map
            .keys()
            .filter(|k| k.to_lowercase().starts_with(&partial_lower))
            .map(|k| Suggestion {
                replace: partial_start..cursor,
                insert: parsed.format_key(k),
                label: k.clone(),
                detail: value_preview(&map[k]),
            })
            .collect(),
        Value::Array(items) => {
            if parsed.partial.is_empty() || parsed.partial.chars().all(|c| c.is_ascii_digit()) {
                // In the Dotted grammar, a `.` immediately before the
                // partial is just the separator that got us here (e.g.
                // "items." before typing an index) — not part of the index
                // syntax itself, which `format_index` renders as `[N]`.
                // JSONPath and JMESPath both reject a dot directly before a
                // bracket ("items.[0]" is a parse error in both; "items[0]"
                // is required), while jq tolerates either, so it's always
                // safe to fold that separator into the replaced range —
                // *except* when the dot is the very first character of the
                // query (jq's bare root `.`), which is the identity
                // operator, not a separator: dropping it would turn an
                // accepted "0" into a bare "[0]", a jq array-*construction*
                // literal, not "index the root".
                let mut replace_start = partial_start;
                if matches!(parsed.style, PathStyle::Dotted)
                    && partial_start > 1
                    && before.as_bytes().get(partial_start - 1) == Some(&b'.')
                {
                    replace_start -= 1;
                }
                items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i.to_string(), v))
                    .filter(|(i, _)| i.starts_with(&parsed.partial))
                    .map(|(i, v)| Suggestion {
                        replace: replace_start..cursor,
                        insert: parsed.format_index(&i),
                        label: i.clone(),
                        detail: value_preview(v),
                    })
                    .collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn value_preview(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) if s.chars().count() > 40 => {
            format!("{:?}…", s.chars().take(40).collect::<String>())
        }
        Value::String(s) => format!("{s:?}"),
        Value::Array(a) => format!("array[{}]", a.len()),
        Value::Object(o) => format!("object{{{}}}", o.len()),
    }
}

/// Which path grammar a successful [`parse_dotted_path`]/[`parse_pointer_path`]
/// used, so [`doc_candidates`] can format an accepted key back into that
/// same syntax.
enum PathStyle {
    /// jq/JSONPath/JMESPath dotted-and-bracketed style: `.foo["a b"][3]`.
    Dotted,
    /// RFC 6901 JSON Pointer: `/foo/a~1b/3`.
    Pointer,
}

struct ParsedPath {
    /// Completed segments before the trailing partial token.
    segments: Vec<PathSegment>,
    /// The (possibly empty) token currently being typed, used both to
    /// filter candidate children and as the byte span to replace.
    partial: String,
    style: PathStyle,
}

impl ParsedPath {
    fn format_key(&self, key: &str) -> String {
        match self.style {
            PathStyle::Dotted if is_bare_ident(key) => key.to_string(),
            // A non-bare-ident key would need bracket-quoting, which would
            // also require rewriting whatever separator the user already
            // typed before the partial (e.g. a stray `.`) — out of scope
            // for this simple path-completion pass, so such keys are
            // simply not offered as candidates in the dotted grammar (see
            // `doc_candidates`' object branch, which still lists them —
            // accepting one just inserts the bare text, which is only
            // valid if the key happens to be a bare identifier).
            PathStyle::Dotted => key.to_string(),
            PathStyle::Pointer => escape_pointer_segment(key),
        }
    }

    fn format_index(&self, index: &str) -> String {
        match self.style {
            // `[N]` — bracket notation is required in jq, JSONPath, and
            // JMESPath alike; only the *preceding* `.` is ever optional
            // (see the caller in `doc_candidates`, which strips it except
            // at the bare root).
            PathStyle::Dotted => format!("[{index}]"),
            PathStyle::Pointer => index.to_string(),
        }
    }
}

fn is_bare_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn escape_pointer_segment(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

fn unescape_pointer_segment(s: &str) -> String {
    // Per RFC 6901: replace ~1 with / first is wrong order-wise if done
    // naively on an already-escaped string containing literal "~1" text,
    // but the standard two-step ~1->/ then ~0->~ (applied in that order)
    // is exactly what avoids double-unescaping "~01" etc.
    s.replace("~1", "/").replace("~0", "~")
}

/// Parse `s` (the whole JSON Pointer typed so far, starting with `/`) into
/// completed segments plus the trailing partial token being typed. Always
/// succeeds — every pointer prefix is syntactically valid — so unlike
/// [`parse_dotted_path`] this returns a bare `ParsedPath`, not an `Option`.
fn parse_pointer_path(s: &str) -> ParsedPath {
    let stripped = s.strip_prefix('/').unwrap_or(s);
    let mut parts: Vec<&str> = stripped.split('/').collect();
    let partial_raw = parts.pop().unwrap_or("");
    let segments = parts
        .into_iter()
        .map(|p| {
            let unescaped = unescape_pointer_segment(p);
            match unescaped.parse::<usize>() {
                Ok(i) if unescaped == i.to_string() => PathSegment::Index(i),
                _ => PathSegment::Key(unescaped),
            }
        })
        .collect();
    ParsedPath {
        segments,
        partial: unescape_pointer_segment(partial_raw),
        style: PathStyle::Pointer,
    }
}

/// Parse `s` as a simple jq/JSONPath($-stripped)/JMESPath dotted-and-bracket
/// path — `.foo.bar[3]`, `foo.bar[3]`, or (JSONPath, `$` already stripped
/// by the caller) `.foo.bar[3]` — into completed segments plus the trailing
/// partial token. Returns `None` as soon as anything outside that simple
/// grammar shows up (a pipe, a function call, a filter, an open bracket
/// still being typed, ...), which is the caller's signal to fall back to
/// keyword-only suggestions.
fn parse_dotted_path(s: &str) -> Option<ParsedPath> {
    let mut segments = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    let mut prev_was_dot = false;

    while let Some(c) = chars.next() {
        match c {
            '.' => {
                if prev_was_dot {
                    // ".." (recursive descent) isn't a fixed path.
                    return None;
                }
                if !cur.is_empty() {
                    segments.push(PathSegment::Key(std::mem::take(&mut cur)));
                }
                prev_was_dot = true;
                continue;
            }
            '[' => {
                if !cur.is_empty() {
                    segments.push(PathSegment::Key(std::mem::take(&mut cur)));
                }
                let mut inner = String::new();
                let mut closed = false;
                for c2 in chars.by_ref() {
                    if c2 == ']' {
                        closed = true;
                        break;
                    }
                    inner.push(c2);
                }
                // An unclosed `[` (still being typed) isn't a completed
                // segment we can resolve past — bail to keyword mode.
                if !closed {
                    return None;
                }
                let index: usize = inner.parse().ok()?;
                segments.push(PathSegment::Index(index));
            }
            c if c.is_alphanumeric() || c == '_' || c == '-' => {
                cur.push(c);
            }
            _ => return None,
        }
        prev_was_dot = false;
    }

    Some(ParsedPath {
        segments,
        partial: cur,
        style: PathStyle::Dotted,
    })
}

/// One static keyword/function candidate.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    text: &'static str,
    detail: &'static str,
    kind: Kind,
}

const fn c(kind: Kind, text: &'static str, detail: &'static str) -> Candidate {
    Candidate { text, detail, kind }
}

fn builtins(kind: Kind) -> &'static [Candidate] {
    match kind {
        Kind::Jq => JQ_BUILTINS,
        Kind::JsonPointer => JSON_POINTER_HINTS,
        Kind::JsonPath => JSONPATH_BUILTINS,
        Kind::JmesPath => JMESPATH_BUILTINS,
    }
}

// Names below are cross-checked against the actual jaq-core/jaq-std/jaq-json
// source (defs.jq plus native `register`-style calls), not jq's own docs
// alone — jaq does not implement every jq builtin, and this app runs on
// jaq, not on jq itself.
#[rustfmt::skip]
const JQ_BUILTINS: &[Candidate] = &[
    // Core language keywords/syntax.
    c(Kind::Jq, "if", "if COND then A else B end"),
    c(Kind::Jq, "then", "if/then/elif/else/end"),
    c(Kind::Jq, "elif", "if/then/elif/else/end"),
    c(Kind::Jq, "else", "if/then/elif/else/end"),
    c(Kind::Jq, "end", "closes if/then/…, try/catch, reduce, foreach"),
    c(Kind::Jq, "try", "try EXPR catch HANDLER"),
    c(Kind::Jq, "catch", "try/catch"),
    c(Kind::Jq, "def", "def name(params): body;"),
    c(Kind::Jq, "as", "EXPR as $x | …  (bind to a variable)"),
    c(Kind::Jq, "reduce", "reduce EXPR as $x (init; update)"),
    c(Kind::Jq, "foreach", "foreach EXPR as $x (init; update; extract)"),
    c(Kind::Jq, "and", "boolean and"),
    c(Kind::Jq, "or", "boolean or"),
    c(Kind::Jq, "true", "boolean literal"),
    c(Kind::Jq, "false", "boolean literal"),
    c(Kind::Jq, "null", "null literal"),
    c(Kind::Jq, "empty", "produces no output at all"),
    c(Kind::Jq, "error", "error(\"msg\")  — raise an error"),
    c(Kind::Jq, "not", "negate a boolean"),
    c(Kind::Jq, "select", "select(cond)  — keep input if cond is true"),
    c(Kind::Jq, "tostring", "convert a value to its string form"),
    c(Kind::Jq, "tonumber", "parse a string as a number"),
    c(Kind::Jq, "toboolean", "parse a string as a boolean"),
    c(Kind::Jq, "type", "\"null\"/\"boolean\"/\"number\"/\"string\"/\"array\"/\"object\""),
    c(Kind::Jq, "length", "string/array/object length, or number's abs value"),
    c(Kind::Jq, "range", "range(to) or range(from; to)"),
    c(Kind::Jq, "repeat", "repeat(f)  — infinite f, f|f, f|f|f, …"),
    c(Kind::Jq, "recurse", "recurse or recurse(f) or recurse(f; cond)"),
    c(Kind::Jq, "while", "while(cond; update)"),
    c(Kind::Jq, "until", "until(cond; update)"),
    c(Kind::Jq, "paths", "paths or paths(cond)  — all paths into the input"),
    c(Kind::Jq, "getpath", "getpath([\"a\",0])"),
    c(Kind::Jq, "setpath", "setpath([\"a\",0]; value)"),
    c(Kind::Jq, "delpaths", "delpaths([[\"a\",0]])"),
    c(Kind::Jq, "map", "map(f)  — [.[] | f]"),
    c(Kind::Jq, "map_values", "map_values(f)  — .[] |= f"),
    c(Kind::Jq, "walk", "walk(f)  — apply f bottom-up, recursively"),
    c(Kind::Jq, "del", "del(f)  — delete matching paths"),
    c(Kind::Jq, "first", "first item of an array, or first(g)"),
    c(Kind::Jq, "last", "last item of an array, or last(g)"),
    c(Kind::Jq, "nth", "nth(n) or nth(n; g)"),
    c(Kind::Jq, "limit", "limit(n; g)  — first n outputs of g"),
    c(Kind::Jq, "join", "join($sep)"),
    c(Kind::Jq, "combinations", "cartesian product of an array of arrays"),
    c(Kind::Jq, "to_entries", "{a:1} -> [{key:\"a\",value:1}]"),
    c(Kind::Jq, "from_entries", "[{key:\"a\",value:1}] -> {a:1}"),
    c(Kind::Jq, "with_entries", "with_entries(f)  — to_entries|map(f)|from_entries"),
    c(Kind::Jq, "isempty", "isempty(g)  — true if g produces no output"),
    c(Kind::Jq, "all", "all, all(cond), or all(g; cond)"),
    c(Kind::Jq, "any", "any, any(cond), or any(g; cond)"),
    c(Kind::Jq, "keys", "sorted object keys, or array indices"),
    c(Kind::Jq, "values", "select(. != null)"),
    c(Kind::Jq, "has", "has(key)  — object has key / array has index"),
    c(Kind::Jq, "in", "in(xs)  — inverse of has"),
    c(Kind::Jq, "contains", "contains(x)"),
    c(Kind::Jq, "inside", "inside(xs)  — inverse of contains"),
    c(Kind::Jq, "indices", "indices(x)  — all indices/positions of x"),
    c(Kind::Jq, "index", "index(x)  — first index of x"),
    c(Kind::Jq, "rindex", "rindex(x)  — last index of x"),
    c(Kind::Jq, "bsearch", "binary search a sorted array"),
    c(Kind::Jq, "flatten", "flatten or flatten(depth)"),
    c(Kind::Jq, "transpose", "transpose an array of arrays"),
    c(Kind::Jq, "add", "sum/concatenate all items"),
    c(Kind::Jq, "abs", "absolute value"),
    c(Kind::Jq, "min", "minimum item"),
    c(Kind::Jq, "max", "maximum item"),
    c(Kind::Jq, "min_by", "min_by(f)"),
    c(Kind::Jq, "max_by", "max_by(f)"),
    c(Kind::Jq, "sort", "sort ascending"),
    c(Kind::Jq, "sort_by", "sort_by(f)"),
    c(Kind::Jq, "group_by", "group_by(f)"),
    c(Kind::Jq, "unique", "sorted, de-duplicated"),
    c(Kind::Jq, "unique_by", "unique_by(f)"),
    c(Kind::Jq, "reverse", "reverse an array or string"),
    c(Kind::Jq, "arrays", "select(type == \"array\")"),
    c(Kind::Jq, "objects", "select(type == \"object\")"),
    c(Kind::Jq, "iterables", "arrays and objects"),
    c(Kind::Jq, "scalars", "anything but arrays/objects"),
    c(Kind::Jq, "booleans", "select(type == \"boolean\")"),
    c(Kind::Jq, "numbers", "select(type == \"number\")"),
    c(Kind::Jq, "strings", "select(type == \"string\")"),
    c(Kind::Jq, "nulls", "select(. == null)"),
    c(Kind::Jq, "isnan", "true if not-a-number"),
    c(Kind::Jq, "isinfinite", "true if +/- infinity"),
    c(Kind::Jq, "isnormal", "true if a normal float"),
    c(Kind::Jq, "isvalues", "select(. != null)"),
    c(Kind::Jq, "isarray", "true if an array"),
    c(Kind::Jq, "isobject", "true if an object"),
    c(Kind::Jq, "isboolean", "true if a boolean"),
    c(Kind::Jq, "isnumber", "true if a number"),
    c(Kind::Jq, "isstring", "true if a string"),
    c(Kind::Jq, "nan", "not-a-number literal"),
    c(Kind::Jq, "infinite", "infinity literal"),
    c(Kind::Jq, "pick", "pick(pathexp)  — keep only matching paths"),
    c(Kind::Jq, "explode", "string -> array of codepoints"),
    c(Kind::Jq, "implode", "array of codepoints -> string"),
    c(Kind::Jq, "split", "split($sep), or split(re; flags)"),
    c(Kind::Jq, "splits", "splits(re) or splits(re; flags)"),
    c(Kind::Jq, "sub", "sub(re; str) or sub(re; str; flags)"),
    c(Kind::Jq, "gsub", "gsub(re; str) or gsub(re; str; flags)"),
    c(Kind::Jq, "test", "test(re) or test(re; flags)  — regex match?"),
    c(Kind::Jq, "match", "match(re) or match(re; flags)"),
    c(Kind::Jq, "capture", "capture(re) or capture(re; flags)"),
    c(Kind::Jq, "scan", "scan(re) or scan(re; flags)"),
    c(Kind::Jq, "ascii_downcase", "lowercase ASCII letters"),
    c(Kind::Jq, "ascii_upcase", "uppercase ASCII letters"),
    c(Kind::Jq, "startswith", "startswith($s)"),
    c(Kind::Jq, "endswith", "endswith($s)"),
    c(Kind::Jq, "ltrimstr", "ltrimstr($s)  — strip a leading prefix"),
    c(Kind::Jq, "rtrimstr", "rtrimstr($s)  — strip a trailing suffix"),
    c(Kind::Jq, "fromjson", "parse a string as JSON"),
    c(Kind::Jq, "tojson", "serialize a value as a JSON string"),
    c(Kind::Jq, "fromdate", "parse an ISO-8601 date string to epoch seconds"),
    c(Kind::Jq, "todate", "format epoch seconds as an ISO-8601 date string"),
    c(Kind::Jq, "now", "current time, epoch seconds"),
    c(Kind::Jq, "mktime", "broken-down time -> epoch seconds"),
    c(Kind::Jq, "strftime", "strftime(fmt)  — format broken-down time"),
    c(Kind::Jq, "strptime", "strptime(fmt)  — parse into broken-down time"),
    c(Kind::Jq, "env", "environment variables, as an object"),
    c(Kind::Jq, "input", "read the next input value"),
    c(Kind::Jq, "inputs", "read all remaining input values"),
    c(Kind::Jq, "debug", "print the input to stderr, unchanged"),
    c(Kind::Jq, "stderr", "print the input to stderr, unchanged"),
    c(Kind::Jq, "halt", "stop processing immediately"),
    c(Kind::Jq, "halt_error", "halt_error or halt_error(code)"),
];

// jsonpath-rust's parser (RFC 9535 filter-expression functions), plus the
// standard's own path syntax tokens.
#[rustfmt::skip]
const JSONPATH_BUILTINS: &[Candidate] = &[
    c(Kind::JsonPath, "$", "root of the document"),
    c(Kind::JsonPath, "@", "current node, inside a filter"),
    c(Kind::JsonPath, "*", "wildcard — every child"),
    c(Kind::JsonPath, "..", "recursive descent"),
    c(Kind::JsonPath, "length", "length(@)  — string/array/object length"),
    c(Kind::JsonPath, "count", "count(@.foo[*])  — number of nodes matched"),
    c(Kind::JsonPath, "value", "value(@.foo)  — the single node's value"),
    c(Kind::JsonPath, "match", "match(@.foo, \"regex\")  — full-string match"),
    c(Kind::JsonPath, "search", "search(@.foo, \"regex\")  — substring match"),
];

// The exact 26 functions `jmespath::runtime::register_builtin_functions`
// registers, plus the dialect's own syntax tokens.
#[rustfmt::skip]
const JMESPATH_BUILTINS: &[Candidate] = &[
    c(Kind::JmesPath, "abs", "abs(@)  — absolute value"),
    c(Kind::JmesPath, "avg", "avg(@)  — average of an array of numbers"),
    c(Kind::JmesPath, "ceil", "ceil(@)"),
    c(Kind::JmesPath, "contains", "contains(subject, search)"),
    c(Kind::JmesPath, "ends_with", "ends_with(subject, suffix)"),
    c(Kind::JmesPath, "floor", "floor(@)"),
    c(Kind::JmesPath, "join", "join(glue, array)"),
    c(Kind::JmesPath, "keys", "keys(@)  — object's keys"),
    c(Kind::JmesPath, "length", "length(@)  — string/array/object length"),
    c(Kind::JmesPath, "map", "map(&expr, @)"),
    c(Kind::JmesPath, "min", "min(@)"),
    c(Kind::JmesPath, "max", "max(@)"),
    c(Kind::JmesPath, "max_by", "max_by(@, &expr)"),
    c(Kind::JmesPath, "min_by", "min_by(@, &expr)"),
    c(Kind::JmesPath, "merge", "merge(@, ...)  — shallow-merge objects"),
    c(Kind::JmesPath, "not_null", "not_null(@, ...)  — first non-null argument"),
    c(Kind::JmesPath, "reverse", "reverse(@)"),
    c(Kind::JmesPath, "sort", "sort(@)"),
    c(Kind::JmesPath, "sort_by", "sort_by(@, &expr)"),
    c(Kind::JmesPath, "starts_with", "starts_with(subject, prefix)"),
    c(Kind::JmesPath, "sum", "sum(@)"),
    c(Kind::JmesPath, "to_array", "to_array(@)"),
    c(Kind::JmesPath, "to_number", "to_number(@)"),
    c(Kind::JmesPath, "to_string", "to_string(@)"),
    c(Kind::JmesPath, "type", "type(@)"),
    c(Kind::JmesPath, "values", "values(@)  — object's values"),
    c(Kind::JmesPath, "@", "current node"),
    c(Kind::JmesPath, "&&", "logical and"),
    c(Kind::JmesPath, "||", "logical or"),
];

const JSON_POINTER_HINTS: &[Candidate] = &[c(
    Kind::JsonPointer,
    "~1",
    "escaped '/' inside a key (~0 escapes a literal '~')",
)];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scope_explicit_wins_regardless_of_text() {
        assert_eq!(
            engines_in_scope(Some(Kind::JmesPath), "/a/b"),
            vec![Kind::JmesPath]
        );
    }

    #[test]
    fn scope_empty_query_is_all_four() {
        assert_eq!(engines_in_scope(None, ""), Kind::ALL.to_vec());
        assert_eq!(engines_in_scope(None, "   "), Kind::ALL.to_vec());
    }

    #[test]
    fn scope_unambiguous_markers_narrow_to_one() {
        assert_eq!(engines_in_scope(None, "/a/b"), vec![Kind::JsonPointer]);
        assert_eq!(engines_in_scope(None, "$.a.b"), vec![Kind::JsonPath]);
        assert_eq!(
            engines_in_scope(None, "people[?age > `30`]"),
            vec![Kind::JmesPath]
        );
        assert_eq!(engines_in_scope(None, ".a.b"), vec![Kind::Jq]);
    }

    #[test]
    fn scope_bare_word_is_ambiguous_between_jq_and_jmespath() {
        assert_eq!(
            engines_in_scope(None, "foo.bar"),
            vec![Kind::Jq, Kind::JmesPath]
        );
    }

    #[test]
    fn empty_query_yields_no_suggestions() {
        let root = json!({"a": 1});
        assert!(suggest("", 0, None, Some(&root)).is_empty());
    }

    #[test]
    fn dotted_path_suggests_root_object_keys() {
        let root = json!({"apple": 1, "avocado": 2, "banana": 3});
        let items = suggest(".a", 2, None, Some(&root));
        let labels: Vec<_> = items.iter().map(|s| s.label.as_str()).collect();
        assert!(labels.contains(&"apple"));
        assert!(labels.contains(&"avocado"));
        assert!(!labels.contains(&"banana"));
    }

    #[test]
    fn dotted_path_descends_into_nested_object() {
        let root = json!({"store": {"book": [], "bicycle": {}}});
        let items = suggest(".store.b", 8, None, Some(&root));
        let labels: Vec<_> = items.iter().map(|s| s.label.as_str()).collect();
        assert!(labels.contains(&"book"));
        assert!(labels.contains(&"bicycle"));
    }

    #[test]
    fn dotted_path_after_closed_index_still_resolves() {
        let root = json!({"items": [{"name": "a"}, {"name": "b"}]});
        let items = suggest(".items[0].n", 11, None, Some(&root));
        // The resolved field ("name") comes first, ranked ahead of any
        // keyword matching the same "n" prefix (of which jq has several:
        // not, now, nth, null, ...).
        assert_eq!(items[0].label, "name");
        assert_eq!(items[0].detail, "\"a\"");
    }

    #[test]
    fn bare_word_path_also_resolves_for_jmespath_style() {
        let root = json!({"apple": 1, "banana": 2});
        let items = suggest("a", 1, None, Some(&root));
        let labels: Vec<_> = items.iter().map(|s| s.label.as_str()).collect();
        assert!(labels.contains(&"apple"));
        assert!(!labels.contains(&"banana"));
    }

    #[test]
    fn jsonpath_dollar_prefix_resolves() {
        let root = json!({"store": {"book": []}});
        let items = suggest("$.store.b", 9, None, Some(&root));
        let labels: Vec<_> = items.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["book"]);
    }

    #[test]
    fn json_pointer_resolves_and_suggests_array_indices() {
        let root = json!({"items": ["x", "y", "z"]});
        let items = suggest("/items/", 7, None, Some(&root));
        let labels: Vec<_> = items.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["0", "1", "2"]);
    }

    #[test]
    fn json_pointer_resolves_object_keys() {
        let root = json!({"a": {"aa": 1, "ab": 2}, "b": 3});
        let items = suggest("/a/a", 4, None, Some(&root));
        let labels: Vec<_> = items.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["aa", "ab"]);
    }

    #[test]
    fn dotted_array_index_at_bare_root_keeps_the_identity_dot() {
        // jq's bare root "." is the identity operator, not a separator --
        // dropping it would turn an accepted "0" into "[0]" alone, a jq
        // array-*construction* literal, not "index the root".
        let root = json!(["a", "b", "c"]);
        let items = suggest(".", 1, None, Some(&root));
        let zero = items
            .iter()
            .find(|s| s.label == "0")
            .expect("index 0 suggested");
        let mut text = ".".to_string();
        text.replace_range(zero.replace.clone(), &zero.insert);
        assert_eq!(text, ".[0]");
    }

    #[test]
    fn dotted_array_index_after_a_key_drops_the_separator_dot() {
        // "items.[0]" is a syntax error in both JSONPath and JMESPath (only
        // jq tolerates it) -- the separator dot must be folded away so the
        // accepted text is "items[0]" and works everywhere.
        let root = json!({"items": ["a", "b", "c"]});
        let items = suggest(".items.", 7, None, Some(&root));
        let zero = items
            .iter()
            .find(|s| s.label == "0")
            .expect("index 0 suggested");
        let mut text = ".items.".to_string();
        text.replace_range(zero.replace.clone(), &zero.insert);
        assert_eq!(text, ".items[0]");
    }

    #[test]
    fn dotted_array_index_bare_word_style_drops_the_separator_dot() {
        let root = json!({"items": ["a", "b", "c"]});
        let items = suggest("items.", 6, None, Some(&root));
        let zero = items
            .iter()
            .find(|s| s.label == "0")
            .expect("index 0 suggested");
        let mut text = "items.".to_string();
        text.replace_range(zero.replace.clone(), &zero.insert);
        assert_eq!(text, "items[0]");
    }

    #[test]
    fn jsonpath_array_index_drops_the_separator_dot_even_at_root() {
        // Unlike jq, JSONPath rejects a dot directly before a bracket at
        // the root too ("$.[0]" is a parse error; "$[0]" is required).
        let root = json!(["a", "b", "c"]);
        let items = suggest("$.", 2, None, Some(&root));
        let zero = items
            .iter()
            .find(|s| s.label == "0")
            .expect("index 0 suggested");
        let mut text = "$.".to_string();
        text.replace_range(zero.replace.clone(), &zero.insert);
        assert_eq!(text, "$[0]");
    }

    #[test]
    fn recursive_descent_falls_back_to_keywords_only() {
        let root = json!({"apple": 1});
        let items = suggest("..z", 3, None, Some(&root));
        // ".." bails out of path-parsing, so no document-derived
        // suggestion ("apple") leaks through — only (nonexistent, for
        // "z") keyword matches would appear.
        assert!(items.iter().all(|s| s.label != "apple"));
    }

    #[test]
    fn expression_past_a_plain_path_falls_back_to_keywords() {
        let items = suggest(".a | sel", 8, None, None);
        let labels: Vec<_> = items.iter().map(|s| s.label.as_str()).collect();
        assert!(labels.iter().any(|l| l.starts_with("select")));
    }

    #[test]
    fn keyword_candidates_are_tagged_when_scope_has_multiple_engines() {
        let items = suggest("so", 2, None, None);
        assert!(items.iter().any(|s| s.label.contains("[jq]")));
        assert!(items.iter().any(|s| s.label.contains("[JMESPath]")));
    }

    #[test]
    fn explicit_engine_only_offers_that_engines_keywords() {
        let items = suggest("so", 2, Some(Kind::JmesPath), None);
        assert!(!items.is_empty());
        assert!(items.iter().all(|s| !s.label.contains('[')));
        assert!(items.iter().any(|s| s.label == "sort"));
    }
}
