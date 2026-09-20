//! What `.` refers to at the cursor, for a jq query that has moved past a plain
//! path.
//!
//! Field completion follows a path from the document root, which is right for
//! `.members[].na` and wrong the moment the query pipes or opens a call: in
//! `.members | map(select(.` the `.` inside `select` is one *member*, not the
//! root. [`scope_at`] reads the text before the cursor the way jq evaluates
//! it and returns the values that `.` stands for there, so the ordinary path
//! completion can carry on from them.
//!
//! What it models, all of it inside the groups still open at the cursor:
//! - **`|`**: the next stage's input is the previous stage's output — known for
//!   a plain path (`.members[]`, `.address`) and for stages that keep the
//!   shape (`select(…)`, `sort_by(…)`, `map(select(…))`, `first`, …).
//! - **A call's parentheses**: `map`, `sort_by` and their kin run their
//!   argument once per element ([`PER_ELEMENT`]); `select`, `first`, `test` and
//!   the like run it on the input itself ([`SAME_INPUT`]).
//! - **Everything else that opens a group** — bare `( )`, `[ ]`, `{ }` — and
//!   `,`, `;`, `and`, `or`, `if`/`then`/`else`, arithmetic and comparison
//!   operators leave the input alone; `X as $v | …` too.
//!
//! Anything else makes the input *unknown*, and unknown offers nothing: a
//! wrong field list is worse than none. That covers a function that isn't in
//! the tables (it may be one the user defined), a stage that builds something
//! new (`map(.name)`, `length`, `keys`), `reduce`/`foreach`/`def`/`label`,
//! `|=`, and a cursor inside a string or a comment.

use serde_json::Value;

use super::{parse_dotted_path, resolve_steps_from, step_is_valid, Slot, Step, Syntax, NODE_CAP};
use crate::highlight::{is_word, matching_close, skip_comment, skip_number, skip_word};
use crate::Kind;

/// jq functions that run their argument once per element of the input (per
/// value, for an object): in `map(.name)` the `.` is a member.
const PER_ELEMENT: [&str; 7] = [
    "map",
    "map_values",
    "sort_by",
    "group_by",
    "unique_by",
    "min_by",
    "max_by",
];

/// jq functions whose arguments run against the call's own input, so a `.`
/// inside means what it meant outside: `select(.age > 30)` asks about the value
/// it was given.
const SAME_INPUT: [&str; 34] = [
    "select",
    "del",
    "pick",
    "path",
    "first",
    "last",
    "limit",
    "nth",
    "isempty",
    "range",
    "error",
    "debug",
    "add",
    "has",
    "in",
    "contains",
    "inside",
    "index",
    "rindex",
    "indices",
    "join",
    "split",
    "splits",
    "test",
    "match",
    "capture",
    "scan",
    "startswith",
    "endswith",
    "ltrimstr",
    "rtrimstr",
    "getpath",
    "setpath",
    "delpaths",
];

/// How many groups deep the scan follows the cursor. Nobody types past this,
/// and it bounds the recursion on text that was pasted in.
const MAX_DEPTH: usize = 32;

/// The values a `.` refers to, or `None` when that isn't known.
type Inputs<'a> = Option<Vec<&'a Value>>;

/// The term being typed, and what its leading `.` refers to.
pub(super) struct Scope<'a> {
    /// Byte offset in the scanned text where the term starts (its first
    /// non-blank character) — after the last `|`, `,`, `(`, operator, …
    pub term_start: usize,
    /// What the term's leading `.` refers to; never empty.
    pub inputs: Vec<&'a Value>,
}

/// Where the scan stands in one group (or the whole query).
struct Frame<'a> {
    /// What `.` is where the group starts, and again after each `;`.
    group_input: Inputs<'a>,
    /// What `.` is in the pipeline stage being read.
    input: Inputs<'a>,
    /// Where the stage began: just past the last `|` (or the group's start).
    stage_start: usize,
    /// Where the term being typed began: just past the last `|`, `,`, `;`,
    /// operator or opening bracket.
    term_start: usize,
    /// The stage has an `as $x` binding, so the `|` after it hands the *same*
    /// input on rather than the stage's output.
    bound: bool,
}

/// The scope of the term at the end of `text` (the query up to the cursor,
/// leading blanks trimmed), starting from the document root; `None` when it
/// can't be told.
pub(super) fn scope_at<'a>(text: &str, root: &'a Value) -> Option<Scope<'a>> {
    scan(text, 0, Some(vec![root]), 0)
}

/// Read `text` from `start`, where `.` stands for `input`, down to the end of
/// the text — recursing into the group that is still open there, if any.
fn scan<'a>(text: &str, start: usize, input: Inputs<'a>, depth: usize) -> Option<Scope<'a>> {
    if depth > MAX_DEPTH {
        return None;
    }
    let b = text.as_bytes();
    let mut frame = Frame {
        group_input: input.clone(),
        input,
        stage_start: start,
        term_start: start,
        bound: false,
    };
    let mut i = start;
    while i < b.len() {
        match b[i] {
            b'"' => i = string_end(b, i)?,
            b'#' => {
                i = skip_comment(b, i);
                if i >= b.len() {
                    return None; // the cursor is in the comment
                }
            }
            b'(' | b'[' | b'{' => match matching_close(Kind::Jq, b, i) {
                Some(close) => i = close + 1,
                // Never closed: the cursor is inside it.
                None => {
                    // `.a[`, `.a[0`, `.a["na`: the slot that path completion
                    // itself fills in, so part of this term, not a new scope.
                    if b[i] == b'[' && is_slot_prefix(&text[i + 1..]) {
                        break;
                    }
                    let inner = group_input(text, i, &frame.input);
                    return scan(text, i + 1, inner, depth + 1);
                }
            },
            // `|=` is an update, not a pipe: its right side sees each value
            // the left side reaches, which isn't modelled.
            b'|' if b.get(i + 1) == Some(&b'=') => {
                frame.input = None;
                i += 2;
                frame.term_start = i;
            }
            b'|' => {
                if !frame.bound {
                    frame.input = stage_output(&text[frame.stage_start..i], &frame.input);
                }
                frame.bound = false;
                i += 1;
                frame.stage_start = i;
                frame.term_start = i;
            }
            // Each `;`-separated argument starts over from the group's input.
            b';' => {
                frame.input = frame.group_input.clone();
                frame.bound = false;
                i += 1;
                frame.stage_start = i;
                frame.term_start = i;
            }
            // `.*` is another dialect's wildcard: jq has no such step, so it is
            // neither a path step to follow nor a multiplication to restart at.
            b'*' if i > 0 && b[i - 1] == b'.' => i += 1,
            b',' | b':' | b'=' | b'<' | b'>' | b'!' | b'+' | b'*' | b'/' | b'%' => {
                i += 1;
                frame.term_start = i;
            }
            // `.a-b` is one word to the completion; `.a - .b` is an operator.
            b'-' if !(i > 0 && is_word(b[i - 1])) => {
                i += 1;
                frame.term_start = i;
            }
            b'$' | b'@' => i = skip_word(b, i + 1),
            c if c.is_ascii_digit() => i = skip_number(b, i),
            c if is_word(c) => {
                let from = i;
                i = skip_word(b, i);
                // A name after a dot is a field, whatever it is spelled like.
                if from > 0 && b[from - 1] == b'.' {
                    continue;
                }
                match &text[from..i] {
                    "and" | "or" | "if" | "then" | "elif" | "else" | "try" | "catch" => {
                        frame.term_start = i;
                    }
                    "as" => {
                        frame.bound = true;
                        frame.term_start = i;
                    }
                    "reduce" | "foreach" | "def" | "label" | "import" | "include" => return None,
                    _ => {}
                }
            }
            _ => i += 1,
        }
    }

    let term = &text[frame.term_start..];
    Some(Scope {
        term_start: frame.term_start + term.len() - term.trim_start().len(),
        inputs: frame.input?,
    })
}

/// Whether `rest` (what follows an unclosed `[`) is the start of an index or a
/// quoted key — nothing yet, digits, a `-` and digits, or an opening quote.
fn is_slot_prefix(rest: &str) -> bool {
    rest.starts_with('"')
        || rest
            .strip_prefix('-')
            .unwrap_or(rest)
            .bytes()
            .all(|c| c.is_ascii_digit())
}

/// Just past the closing quote of the string that starts at `at`, or `None`
/// when the text ends inside it — including inside a `\(…)` interpolation.
fn string_end(b: &[u8], at: usize) -> Option<usize> {
    let mut i = at + 1;
    while i < b.len() {
        match b[i] {
            b'\\' if b.get(i + 1) == Some(&b'(') => i = matching_close(Kind::Jq, b, i + 1)? + 1,
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// What `.` is inside the still-open bracket at `open`, given that `.` is
/// `input` outside it.
fn group_input<'a>(text: &str, open: usize, input: &Inputs<'a>) -> Inputs<'a> {
    if text.as_bytes()[open] != b'(' {
        // An array literal, an object construction or an index: evaluated
        // against the same input as the text around it.
        return input.clone();
    }
    let b = text.as_bytes();
    let mut name_start = open;
    while name_start > 0 && is_word(b[name_start - 1]) {
        name_start -= 1;
    }
    let name = &text[name_start..open];
    if name.is_empty() {
        return input.clone(); // a bare `( … )`
    }
    if name_start > 0 && matches!(b[name_start - 1], b'.' | b'$') {
        return None; // `.name(` / `$name(` aren't calls
    }
    if PER_ELEMENT.contains(&name) {
        children(input.as_deref()?)
    } else if SAME_INPUT.contains(&name) {
        input.clone()
    } else {
        None
    }
}

/// What a pipeline stage (`stage`, run against `input`) produces, as far as
/// it can be told.
fn stage_output<'a>(stage: &str, input: &Inputs<'a>) -> Inputs<'a> {
    let input = input.as_deref()?;
    let stage = stage.trim();
    if stage == "." {
        return Some(input.to_vec());
    }
    if stage.starts_with('.') {
        let parsed = parse_dotted_path(stage)?;
        if parsed.slot != Slot::Token {
            return None;
        }
        // The token being "typed" is the last key of a finished path here.
        let mut steps = parsed.steps;
        if !parsed.partial.is_empty() {
            steps.push(Step::Key(parsed.partial));
        }
        if !steps.iter().all(|s| step_is_valid(Syntax::Jq, s)) {
            return None;
        }
        let out = resolve_steps_from(input.to_vec(), &steps, Syntax::Jq);
        return (!out.is_empty()).then_some(out);
    }

    let (name, args) = call_parts(stage)?;
    let arrays = input.iter().all(|v| v.is_array());
    match (name, args) {
        // The same values, some of them dropped.
        ("select", Some(_)) => Some(input.to_vec()),
        // The same elements, in another order or with duplicates removed.
        ("sort" | "reverse" | "unique", None) | ("sort_by" | "unique_by", Some(_)) if arrays => {
            Some(input.to_vec())
        }
        ("map", Some(f)) if arrays && only_selects(f) => Some(input.to_vec()),
        // One of the elements.
        ("first" | "last" | "min" | "max", None) | ("min_by" | "max_by", Some(_)) if arrays => {
            children(input)
        }
        _ => None,
    }
}

/// The elements (or, for an object, values) of every value in `input`.
fn children<'a>(input: &[&'a Value]) -> Inputs<'a> {
    let mut out: Vec<&Value> = Vec::new();
    for value in input {
        match value {
            Value::Array(items) => out.extend(items),
            Value::Object(map) => out.extend(map.values()),
            _ => {}
        }
        if out.len() >= NODE_CAP {
            break;
        }
    }
    out.truncate(NODE_CAP);
    (!out.is_empty()).then_some(out)
}

/// `name` or `name(args)` — the whole of `stage` — as (name, args text).
fn call_parts(stage: &str) -> Option<(&str, Option<&str>)> {
    let b = stage.as_bytes();
    let name_end = skip_word(b, 0);
    if name_end == 0 {
        return None;
    }
    let name = &stage[..name_end];
    if name_end == b.len() {
        return Some((name, None));
    }
    let closes_last =
        b[name_end] == b'(' && matching_close(Kind::Jq, b, name_end) == Some(b.len() - 1);
    closes_last.then(|| (name, Some(&stage[name_end + 1..b.len() - 1])))
}

/// Whether `chain` is `.` and `select(…)` stages joined by pipes — something
/// that only drops values and never changes them.
fn only_selects(chain: &str) -> bool {
    pipe_stages(chain).into_iter().all(|stage| {
        let stage = stage.trim();
        stage == "." || matches!(call_parts(stage), Some(("select", Some(_))))
    })
}

/// `text` split at its top-level `|`s: not the ones inside a string or a
/// bracket, and not `|=`.
fn pipe_stages(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let mut stages = Vec::new();
    let mut from = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => i = string_end(b, i).unwrap_or(b.len()),
            b'(' | b'[' | b'{' => i = matching_close(Kind::Jq, b, i).map_or(b.len(), |c| c + 1),
            b'|' if b.get(i + 1) == Some(&b'=') => i += 2,
            b'|' => {
                stages.push(&text[from..i]);
                i += 1;
                from = i;
            }
            _ => i += 1,
        }
    }
    stages.push(&text[from..]);
    stages
}
