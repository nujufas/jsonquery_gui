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
//! follows it through the loaded document to offer the real field names or
//! array indices found there. A query that has moved past a plain path — a
//! pipe, a function call, a filter, ... — falls back to keyword-only
//! suggestions; this is a deliberate scope limit (correctly tracking "where
//! am I" through arbitrary jq/JMESPath expressions is a much bigger
//! undertaking), not an oversight.
//!
//! "Plain path" includes the steps that fan out or index from the end:
//! `[]`, `[*]`, `.*`, slices and negative indices. After a wildcard the
//! cursor is inside *many* values at once (`.members[].` — every member), so
//! the candidates are the union of their children, drawn from the first few
//! hundred (`NODE_CAP`); which dialect has which spelling is `step_is_valid`.
//!
//! # Accepted text must be valid in the dialect
//! Where a candidate goes and how it's spelled depends on the syntax around
//! it, so each document-derived candidate is formatted for the query's
//! dialect rather than splicing the bare name in:
//! - a `.` separator is added when nothing separates the key from what
//!   precedes it (`.a[0]` + `name` -> `.a[0].name`, `$` + `name` ->
//!   `$.name`), and dropped when the candidate is bracketed (`.a["b c"]`);
//! - keys that aren't plain identifiers are quoted the way the dialect
//!   spells it: `.["a b"]` in jq, `$["a b"]` in JSONPath, `."a b"` in
//!   JMESPath (JSON Pointer just escapes `~` and `/`);
//! - a typed `[` completes with the array's indices (or, in jq/JSONPath, the
//!   object's quoted keys), swallowing a closing `]` that's already there.
//!
//! Static keywords are likewise only offered where a name could actually
//! start: never right after a `.` (that's a field, `.first` is not the
//! builtin), after a closing bracket, or inside a string literal.

use std::collections::HashSet;
use std::ops::Range;

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
/// Cap on how many values a wildcard step (`[]`, `[*]`, `.*`, a slice) keeps
/// while resolving a path for completion — enough to see the shape of a
/// long array's elements without walking every one on each keystroke.
const NODE_CAP: usize = 200;

/// One completion candidate offered in the query box's popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Byte range of the query text this candidate replaces if accepted. It
    /// starts at the token being completed (or earlier, to fold away a
    /// separator or reach a `[`) and ends at the cursor — or just past it, to
    /// take in a closing `]` the user already has.
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
    if before.trim().is_empty() {
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

    let mut out = doc_candidates(&scope, query_text, cursor, doc_root);
    out.truncate(DOC_CANDIDATE_CAP);

    // Typing the `~` of a JSON Pointer escape is the one place its `~0`/`~1`
    // spellings are worth offering.
    if scope.contains(&Kind::JsonPointer) && before.ends_with('~') {
        out.extend(pointer_escapes(cursor));
    }

    if keywords_wanted(before, word, word_start, !out.is_empty()) {
        let mut kw = keyword_candidates(&scope, word, word_start..cursor);
        kw.truncate(KEYWORD_CANDIDATE_CAP);
        out.extend(kw);
    }

    out
}

/// Whether static keyword/function names make sense at the cursor, where
/// `word` (starting at byte `word_start` of `before`) is the identifier being
/// typed and `have_doc_candidates` says something more specific is already
/// on offer.
fn keywords_wanted(before: &str, word: &str, word_start: usize, have_doc_candidates: bool) -> bool {
    if inside_string_literal(before) {
        return false;
    }
    let prev = before[..word_start].chars().next_back();
    // Right after a `.` a name is a field (`.first` reads the field "first",
    // it doesn't call the builtin); after a closing bracket/quote no name
    // can start at all (`.a[0]abs`); and after `$`/`@` it's a variable or
    // current-node reference, not a keyword.
    if matches!(
        prev,
        Some('.' | ']' | ')' | '}' | '"' | '\'' | '`' | '$' | '@')
    ) {
        return false;
    }
    if word.is_empty() {
        // Nothing to filter by, so listing every keyword is only a fallback
        // for when there's nothing more specific — and only where a fresh
        // expression begins (after whitespace, a pipe, an opening paren or
        // brace, a separator). Not after a `[` or an operator like `-`, where
        // what belongs is an index or a key (`[-1]`), or nothing yet.
        let expression_starts = prev.is_some_and(|c| c.is_whitespace() || "|(,;:{".contains(c));
        return !have_doc_candidates && expression_starts;
    }
    true
}

/// Whether `text` ends inside an unterminated string (`"…`), JMESPath raw
/// string (`'…`) or JMESPath literal (`` `… ``).
fn inside_string_literal(text: &str) -> bool {
    let mut open: Option<char> = None;
    let mut escaped = false;
    for c in text.chars() {
        match open {
            Some(quote) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    open = None;
                }
            }
            None if matches!(c, '"' | '\'' | '`') => open = Some(c),
            None => {}
        }
    }
    open.is_some()
}

/// The `~0` / `~1` escape spellings, offered in place of a `~` just typed.
fn pointer_escapes(cursor: usize) -> [Suggestion; 2] {
    [
        ("~0", "escapes a literal '~' inside a key"),
        ("~1", "escapes a literal '/' inside a key"),
    ]
    .map(|(text, detail)| Suggestion {
        replace: cursor - 1..cursor,
        insert: text.to_string(),
        label: text.to_string(),
        detail: detail.to_string(),
    })
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

/// Document-aware field/index candidates: tries to parse the query up to the
/// cursor as a simple path in whichever grammar `scope` implies, resolves it
/// against `doc_root`, and lists the resolved containers' children that match
/// the token being typed — spelled for the query's dialect (see [`Syntax`]),
/// so that accepting one yields a valid path, not just the right name.
fn doc_candidates(
    scope: &[Kind],
    query_text: &str,
    cursor: usize,
    doc_root: Option<&Value>,
) -> Vec<Suggestion> {
    let Some(root) = doc_root else {
        return Vec::new();
    };
    let before = &query_text[..cursor];
    let body_start = before.len() - before.trim_start().len();

    let Some((syntax, parsed)) = parse_for_scope(scope, &before[body_start..]) else {
        return Vec::new();
    };
    let parents = resolve_steps(root, &parsed.steps, syntax);
    if parents.is_empty() {
        return Vec::new();
    }

    let site = Site {
        syntax,
        parents,
        parsed,
        before,
        after: &query_text[cursor..],
        cursor,
        body_start,
    };
    match site.parsed.slot {
        Slot::Token => site.token_candidates(),
        Slot::Bracket { quote } => site.bracket_candidates(quote),
    }
}

/// Where the cursor sits inside a resolved path, shared by the two candidate
/// builders below.
struct Site<'a> {
    syntax: Syntax,
    /// The values the path typed so far reaches — the containers whose
    /// children are being completed. Exactly one, unless a wildcard
    /// (`[]`, `[*]`, `.*`, a slice) fanned the path out over many.
    parents: Vec<&'a Value>,
    parsed: ParsedPath,
    /// The query text before / after the cursor.
    before: &'a str,
    after: &'a str,
    cursor: usize,
    /// Byte offset of the query's first non-whitespace character.
    body_start: usize,
}

impl Site<'_> {
    /// Completing a bare token: `.foo`, `foo`, `/foo`, or an empty one right
    /// after a `.`/`]`.
    fn token_candidates(&self) -> Vec<Suggestion> {
        let partial = &self.parsed.partial;
        let partial_start = self.cursor - self.parsed.partial_len;
        let prev = partial_start
            .checked_sub(1)
            .and_then(|i| self.before.as_bytes().get(i));
        let lead = match prev {
            Some(b'.') => Lead::Dot {
                root: self.syntax == Syntax::Jq && partial_start - 1 == self.body_start,
            },
            Some(b']' | b'"' | b'*') => Lead::Joint,
            Some(b'$') if self.syntax == Syntax::JsonPath => Lead::Joint,
            _ => Lead::Start,
        };

        let keys = matching_keys(&self.parents, partial)
            .into_iter()
            .map(|(key, value)| {
                let (start, insert) = key_edit(self.syntax, lead, key, partial_start);
                Suggestion {
                    replace: start..self.cursor,
                    insert,
                    label: key.to_string(),
                    detail: value_preview(value),
                }
            });

        // A bare digit token that isn't after a `.`/`]`/`$` is just a number
        // being typed (`1 + 2`), not an index into anything.
        let is_index_position = self.syntax == Syntax::Pointer || lead != Lead::Start;
        let indices = if is_index_position && partial.chars().all(|c| c.is_ascii_digit()) {
            matching_indices(&self.parents, partial)
        } else {
            Vec::new()
        }
        .into_iter()
        .map(|(index, value)| {
            let (start, insert) = index_edit(self.syntax, lead, &index, partial_start);
            Suggestion {
                replace: start..self.cursor,
                insert,
                label: index,
                detail: value_preview(value),
            }
        });

        keys.chain(indices).collect()
    }

    /// Completing inside an unclosed `[`: array indices, or — where the
    /// dialect has bracketed keys (jq, JSONPath) — quoted object keys. Each
    /// candidate is the whole `[…]`, replacing from the `[` and swallowing a
    /// `]` (and closing quote) that's already there.
    fn bracket_candidates(&self, quote: Option<char>) -> Vec<Suggestion> {
        let partial = &self.parsed.partial;
        let bracket_start = self.cursor - self.parsed.partial_len - 1;
        // jq reads a `[` at the very start of a query as an array *literal*
        // (`[0]` is the one-element array, not "index 0"), so indexing the
        // root there needs its leading identity dot: `.[0]`.
        let root_dot = if self.syntax == Syntax::Jq && bracket_start == self.body_start {
            "."
        } else {
            ""
        };
        let replace = bracket_start..self.cursor + closing_len(self.after, quote);

        let indices = if quote.is_none() && partial.chars().all(|c| c.is_ascii_digit()) {
            matching_indices(&self.parents, partial)
        } else {
            Vec::new()
        }
        .into_iter()
        .map(|(index, value)| Suggestion {
            replace: replace.clone(),
            insert: format!("{root_dot}[{index}]"),
            label: index,
            detail: value_preview(value),
        });

        // JMESPath has no bracketed keys (`foo["a"]` is a syntax error
        // there), so an object in brackets has nothing to offer it.
        let bracketed_keys = matches!(self.syntax, Syntax::Jq | Syntax::JsonPath)
            && (quote.is_some() || partial.is_empty());
        let keys = if bracketed_keys {
            matching_keys(&self.parents, partial)
        } else {
            Vec::new()
        }
        .into_iter()
        .map(|(key, value)| Suggestion {
            replace: replace.clone(),
            insert: format!("{root_dot}[{}]", json_quote(key)),
            label: key.to_string(),
            detail: value_preview(value),
        });

        keys.chain(indices).collect()
    }
}

/// Every value `steps` reaches from `root` — more than one once a wildcard
/// fans the path out over an array's elements (or an object's values), and
/// none if some step doesn't apply. Bounded by [`NODE_CAP`] per step, so a
/// wildcard over a huge array only samples its first elements.
fn resolve_steps<'a>(root: &'a Value, steps: &[Step], syntax: Syntax) -> Vec<&'a Value> {
    let mut nodes = vec![root];
    for step in steps {
        let mut next: Vec<&Value> = Vec::new();
        for node in &nodes {
            match (step, node) {
                (Step::Key(key), Value::Object(map)) => next.extend(map.get(key)),
                (Step::Index(i), Value::Array(items)) => next.extend(items.get(*i)),
                (Step::FromEnd(n), Value::Array(items)) => {
                    // `checked_sub` covers "further back than the array is
                    // long"; `[-0]` lands one past the end, and `get` misses.
                    if let Some(i) = items.len().checked_sub(*n) {
                        next.extend(items.get(i));
                    }
                }
                (Step::Each(kind), _) => each_child(*kind, syntax, node, &mut next),
                (Step::Slice(from, to), Value::Array(items)) => {
                    next.extend(&items[slice_range(items.len(), *from, *to)]);
                }
                _ => {}
            }
            if next.len() >= NODE_CAP {
                break;
            }
        }
        next.truncate(NODE_CAP);
        if next.is_empty() {
            return next;
        }
        nodes = next;
    }
    nodes
}

/// Push what a wildcard of spelling `kind` selects from `node` into `out`.
/// jq's `[]` and JSONPath's `[*]`/`.*` take an array's elements *and* an
/// object's values; JMESPath is stricter — `[]`/`[*]` only apply to arrays,
/// `.*` only to objects — and its `[]` also flattens one level of nested
/// arrays.
fn each_child<'a>(kind: Each, syntax: Syntax, node: &'a Value, out: &mut Vec<&'a Value>) {
    let jmespath = syntax == Syntax::JmesPath;
    match node {
        Value::Array(items) if !(jmespath && kind == Each::DotStar) => {
            for item in items {
                match item {
                    Value::Array(inner) if jmespath && kind == Each::Brackets => out.extend(inner),
                    _ => out.push(item),
                }
            }
        }
        Value::Object(map) if !(jmespath && kind != Each::DotStar) => out.extend(map.values()),
        _ => {}
    }
}

/// The elements a slice `[from:to]` of an array of `len` selects. Negative
/// bounds count from the end; out-of-range ones clamp, as in every dialect.
fn slice_range(len: usize, from: Option<i64>, to: Option<i64>) -> Range<usize> {
    let bound = |v: i64| {
        let magnitude = usize::try_from(v.unsigned_abs()).unwrap_or(usize::MAX);
        if v < 0 {
            len.saturating_sub(magnitude)
        } else {
            magnitude.min(len)
        }
    };
    let start = from.map_or(0, bound);
    let end = to.map_or(len, bound);
    start.min(end)..end
}

/// Object keys across `parents` that start with `partial` (case-insensitive),
/// each once in first-seen order, paired with the value first found under it.
fn matching_keys<'a>(parents: &[&'a Value], partial: &str) -> Vec<(&'a str, &'a Value)> {
    let partial_lower = partial.to_lowercase();
    let mut seen = HashSet::new();
    parents
        .iter()
        .filter_map(|parent| parent.as_object())
        .flat_map(|map| map.iter())
        .filter(|(key, _)| key.to_lowercase().starts_with(&partial_lower) && seen.insert(*key))
        .take(DOC_CANDIDATE_CAP)
        .map(|(key, value)| (key.as_str(), value))
        .collect()
}

/// Array indices across `parents` that start with `partial`, paired with the
/// value first found at each — so over several arrays, every index up to the
/// longest one's length.
fn matching_indices<'a>(parents: &[&'a Value], partial: &str) -> Vec<(String, &'a Value)> {
    let arrays: Vec<&'a Vec<Value>> = parents.iter().filter_map(|p| p.as_array()).collect();
    let longest = arrays.iter().map(|items| items.len()).max().unwrap_or(0);
    (0..longest)
        .map(|i| (i.to_string(), i))
        .filter(|(label, _)| label.starts_with(partial))
        .take(DOC_CANDIDATE_CAP)
        .filter_map(|(label, i)| {
            let first_value = arrays.iter().find_map(|items| items.get(i))?;
            Some((label, first_value))
        })
        .collect()
}

/// How many bytes right after the cursor already belong to the `[…]` being
/// completed — an optional closing `quote`, then an optional `]` — so
/// accepting a candidate (which supplies its own) doesn't leave duplicates.
fn closing_len(after: &str, quote: Option<char>) -> usize {
    let mut len = 0;
    if let Some(q) = quote {
        if after.starts_with(q) {
            len += q.len_utf8();
        }
    }
    if after[len..].starts_with(']') {
        len += 1;
    }
    len
}

/// Where and how to splice in object key `key`, given the token being
/// completed starts at `partial_start`: the start of the byte range to
/// replace (it always ends at the cursor) and the text to put there.
fn key_edit(syntax: Syntax, lead: Lead, key: &str, partial_start: usize) -> (usize, String) {
    if syntax == Syntax::Pointer {
        return (partial_start, escape_pointer_segment(key));
    }
    // Nothing separates the key from a preceding `]`, closing quote, or
    // JSONPath's `$` yet, so a `.`-separated segment has to bring its own.
    let dot = if lead == Lead::Joint { "." } else { "" };
    if is_bare_ident(key) {
        return (partial_start, format!("{dot}{key}"));
    }
    let quoted = json_quote(key);
    if syntax == Syntax::JmesPath {
        // A quoted identifier is an ordinary `.`-separated segment.
        return (partial_start, format!("{dot}{quoted}"));
    }
    // jq and JSONPath spell it as a bracket segment, which can't follow a
    // separator `.` (JSONPath rejects `$.a.["k"]`, and jq's own tolerance
    // doesn't cover every version) — so fold that separator away. Except
    // jq's *root* dot, which is the identity operator: `.["k"]` needs it.
    let start = match lead {
        Lead::Dot { root: false } => partial_start - 1,
        _ => partial_start,
    };
    (start, format!("[{quoted}]"))
}

/// The array-index counterpart of [`key_edit`]: `[N]` bracket notation is
/// required in jq, JSONPath and JMESPath alike (`.0` is a syntax error
/// everywhere), and follows the same separator rules as a bracketed key.
fn index_edit(syntax: Syntax, lead: Lead, index: &str, partial_start: usize) -> (usize, String) {
    if syntax == Syntax::Pointer {
        return (partial_start, index.to_string());
    }
    let start = match lead {
        Lead::Dot { root: false } => partial_start - 1,
        _ => partial_start,
    };
    (start, format!("[{index}]"))
}

/// `s` as a double-quoted, JSON-escaped string — the quoted-key spelling jq,
/// JSONPath and JMESPath all accept.
fn json_quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("{s:?}"))
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

/// Which dialect's spelling an accepted candidate has to use — decides how
/// keys that aren't plain identifiers are quoted and where `.` separators
/// go. Not the same as [`Kind`]: a query's `Kind` can still be ambiguous
/// (jq or JMESPath) where the path text typed so far already pins the
/// spelling down (see [`parse_for_scope`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Syntax {
    Jq,
    JsonPath,
    JmesPath,
    /// RFC 6901 JSON Pointer: `/foo/a~1b/3`.
    Pointer,
}

/// What directly precedes the token being completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lead {
    /// Nothing: the token starts the path (a bare JMESPath word).
    Start,
    /// A `.` separator. `root` marks jq's identity dot at the very start of
    /// the query (`.`, `.foo`), which — unlike every other separator — is
    /// part of the path syntax rather than something that can be dropped
    /// before a bracket.
    Dot { root: bool },
    /// A `]`, a closing quote, or JSONPath's `$`: a segment just ended with
    /// no separator after it, so a `.`-style key must add one while a
    /// bracketed index/key needs none.
    Joint,
}

/// Where inside the path the cursor's token sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// A bare token: `.foo`, `foo`, `/foo`.
    Token,
    /// Right after an unclosed `[` (`[`, `[1`, `["na`); `quote` is the quote
    /// character already typed after it, if any.
    Bracket { quote: Option<char> },
}

/// One completed step of the path typed so far.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Key(String),
    Index(usize),
    /// `[-N]`: the N-th element from the end.
    FromEnd(usize),
    /// A wildcard: every element of an array / value of an object.
    Each(Each),
    /// `[from:to]`; either bound may be missing, and negative ones count
    /// from the end. (A stepped slice, `[::2]`, isn't modelled.)
    Slice(Option<i64>, Option<i64>),
}

/// How a wildcard step was spelled — which decides which dialects accept it
/// (see [`step_is_valid`]) and what it selects (see `each_child`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Each {
    /// `[]`: jq's iterator, JMESPath's flatten.
    Brackets,
    /// `[*]`: JMESPath's and JSONPath's wildcard.
    Star,
    /// `.*`: JMESPath's and JSONPath's object-value wildcard.
    DotStar,
}

/// Whether `syntax` has this step *and* one whose result the completion
/// model can follow. jq has neither `[*]` nor `.*` (there `.*` is a
/// multiplication), and its `.[1:3]` yields a fresh array where JMESPath's and
/// JSONPath's slices select elements — a difference not modelled, so a jq
/// slice ends completion. JSONPath has no `[]`.
fn step_is_valid(syntax: Syntax, step: &Step) -> bool {
    !matches!(
        (syntax, step),
        (
            Syntax::Jq,
            Step::Each(Each::Star | Each::DotStar) | Step::Slice(..)
        ) | (Syntax::JsonPath, Step::Each(Each::Brackets))
    )
}

struct ParsedPath {
    /// Completed steps before the token being typed.
    steps: Vec<Step>,
    /// The (possibly empty) token being typed — with any opening quote
    /// stripped and any JSON Pointer escapes resolved — used to filter
    /// candidate children.
    partial: String,
    /// How many bytes the token occupies in the query text as typed. Can
    /// differ from `partial.len()` (an escaped `~1` is two bytes for one
    /// `/`), and locates where the replaced range starts.
    partial_len: usize,
    slot: Slot,
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

/// Parse `body` (the query so far, leading whitespace trimmed) as a simple
/// path in the grammar `scope` implies, along with the [`Syntax`] accepted
/// candidates must be spelled in. `None` if it isn't a plain path in that
/// grammar, which is the caller's signal to offer no document candidates.
fn parse_for_scope(scope: &[Kind], body: &str) -> Option<(Syntax, ParsedPath)> {
    if body.is_empty() {
        return None;
    }
    if scope == [Kind::JsonPointer] {
        return body
            .starts_with('/')
            .then(|| (Syntax::Pointer, parse_pointer_path(body)));
    }
    if scope == [Kind::JsonPath] {
        let rest = body.strip_prefix('$')?;
        return parse_dotted_path(rest)
            .filter(|p| p.steps.iter().all(|s| step_is_valid(Syntax::JsonPath, s)))
            .map(|p| (Syntax::JsonPath, p));
    }

    // jq and JMESPath share the dotted grammar but not its details: a
    // leading `.` only means something in jq (JMESPath has no identity dot),
    // and jq has no bare `foo.bar` paths (that start is JMESPath's). A `[`
    // being typed at the very start is ambiguous — jq array literal or
    // JMESPath index — so prefer jq, as `Kind::detect` does, unless jq is
    // ruled out. Once that first bracket is closed, though, `[0].name` is
    // only a path in JMESPath.
    let jq = scope.contains(&Kind::Jq);
    let jmes = scope.contains(&Kind::JmesPath);
    let parsed = parse_dotted_path(body)?;
    let syntax = if body.starts_with('.') {
        jq.then_some(Syntax::Jq)?
    } else if jq
        && body.starts_with('[')
        && parsed.steps.is_empty()
        && matches!(parsed.slot, Slot::Bracket { .. })
    {
        Syntax::Jq
    } else if jmes {
        Syntax::JmesPath
    } else {
        return None;
    };
    parsed
        .steps
        .iter()
        .all(|s| step_is_valid(syntax, s))
        .then_some((syntax, parsed))
}

/// Parse `s` (the whole JSON Pointer typed so far, starting with `/`) into
/// completed segments plus the trailing partial token being typed. Always
/// succeeds — every pointer prefix is syntactically valid — so unlike
/// [`parse_dotted_path`] this returns a bare `ParsedPath`, not an `Option`.
fn parse_pointer_path(s: &str) -> ParsedPath {
    let stripped = s.strip_prefix('/').unwrap_or(s);
    let mut parts: Vec<&str> = stripped.split('/').collect();
    let partial_raw = parts.pop().unwrap_or("");
    let steps = parts
        .into_iter()
        .map(|p| {
            let unescaped = unescape_pointer_segment(p);
            match unescaped.parse::<usize>() {
                Ok(i) if unescaped == i.to_string() => Step::Index(i),
                _ => Step::Key(unescaped),
            }
        })
        .collect();
    ParsedPath {
        steps,
        partial: unescape_pointer_segment(partial_raw),
        partial_len: partial_raw.len(),
        slot: Slot::Token,
    }
}

/// What `[…]` at the current position of a dotted path turned out to be.
enum BracketScan {
    /// A complete `[N]` / `["key"]` / `[]` / `[1:3]`: the step it names, and
    /// how many bytes of the text after the `[` it took (through the `]`).
    Closed(Step, usize),
    /// Still being typed (no closing `]` yet): the quote already typed after
    /// the `[`, if any, and the text after it.
    Open {
        quote: Option<char>,
        partial: String,
    },
    /// Something a plain path can't resolve past — a filter `?(…)`, a stepped
    /// slice, an expression.
    Other,
}

/// Classify the text right after a `[` (all the way to the end of the query
/// up to the cursor).
fn scan_bracket(rest: &str) -> BracketScan {
    match rest.chars().next() {
        Some(q @ ('"' | '\'')) => match scan_quoted(&rest[1..], q) {
            Some((raw, used)) => {
                let after = &rest[1 + used..];
                if after.starts_with(']') {
                    match unescape_quoted(&raw, q) {
                        Some(key) => BracketScan::Closed(Step::Key(key), 1 + used + 1),
                        None => BracketScan::Other,
                    }
                } else if after.is_empty() {
                    // `["name"` — the whole key typed, `]` still to come.
                    BracketScan::Open {
                        quote: Some(q),
                        partial: raw,
                    }
                } else {
                    BracketScan::Other
                }
            }
            None => BracketScan::Open {
                quote: Some(q),
                partial: rest[1..].to_string(),
            },
        },
        _ => match rest.find(']') {
            Some(end) => match parse_bracket_step(rest[..end].trim()) {
                Some(step) => BracketScan::Closed(step, end + 1),
                None => BracketScan::Other,
            },
            None => BracketScan::Open {
                quote: None,
                partial: rest.to_string(),
            },
        },
    }
}

/// The step named by the unquoted contents of a closed `[…]`: an index
/// (`3`, `-1`), a wildcard (`` / `*`) or a slice (`1:3`, `:2`, `-2:`).
fn parse_bracket_step(inner: &str) -> Option<Step> {
    match inner {
        "" => return Some(Step::Each(Each::Brackets)),
        "*" => return Some(Step::Each(Each::Star)),
        _ => {}
    }
    if let Ok(index) = inner.parse::<usize>() {
        return Some(Step::Index(index));
    }
    if let Some(back) = inner
        .strip_prefix('-')
        .and_then(|n| n.parse::<usize>().ok())
    {
        return Some(Step::FromEnd(back));
    }
    // A bound is absent (`[:2]`), or an integer. A second `:` (a step) lands
    // in `to` and fails to parse, which rejects the whole slice.
    let bound = |text: &str| match text.trim() {
        "" => Some(None),
        n => n.parse().ok().map(Some),
    };
    let (from, to) = inner.split_once(':')?;
    Some(Step::Slice(bound(from)?, bound(to)?))
}

/// Scan a quoted token whose opening `quote` is already consumed (so `rest`
/// starts with its contents): returns the raw, still-escaped contents and
/// the bytes taken through the closing quote, or `None` if it never closes.
fn scan_quoted(rest: &str, quote: char) -> Option<(String, usize)> {
    let mut raw = String::new();
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            return Some((raw, i + c.len_utf8()));
        }
        raw.push(c);
    }
    None
}

/// Resolve the escapes in the raw contents of a quoted token: JSON string
/// escapes for `"…"`, and just `\'` / `\\` for JSONPath's `'…'`.
fn unescape_quoted(raw: &str, quote: char) -> Option<String> {
    if quote == '"' {
        return serde_json::from_str::<String>(&format!("\"{raw}\"")).ok();
    }
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' { chars.next()? } else { c });
    }
    Some(out)
}

/// Parse `s` as a simple jq/JSONPath($-stripped)/JMESPath dotted-and-bracket
/// path — `.foo.bar[3]`, `foo.bar[3]`, `.a["b c"]`, `a."b c"`, `a[-1]`,
/// `a[].b`, `a[*].b`, `a.*.b`, `a[1:3].b`, or (JSONPath, `$` already stripped
/// by the caller) `.foo.bar[3]` — into completed steps plus the token being
/// typed at its end (a bare word, or the inside of an unclosed `[`). Returns
/// `None` as soon as anything outside that simple grammar shows up (a pipe, a
/// function call, a filter, ...), which is the caller's signal to offer no
/// document candidates. Which of these steps a given *dialect* accepts is
/// [`step_is_valid`]'s call, not this parser's.
fn parse_dotted_path(s: &str) -> Option<ParsedPath> {
    let mut steps = Vec::new();
    let mut cur = String::new();
    let mut pos = 0;
    let mut prev_was_dot = false;
    let mut at_start = true;

    while let Some(c) = s[pos..].chars().next() {
        let next = pos + c.len_utf8();
        match c {
            '.' => {
                if prev_was_dot {
                    // ".." (recursive descent) isn't a fixed path.
                    return None;
                }
                if !cur.is_empty() {
                    steps.push(Step::Key(std::mem::take(&mut cur)));
                }
                prev_was_dot = true;
                at_start = false;
                pos = next;
                continue;
            }
            '[' => {
                if !cur.is_empty() {
                    steps.push(Step::Key(std::mem::take(&mut cur)));
                }
                match scan_bracket(&s[next..]) {
                    BracketScan::Closed(step, used) => {
                        steps.push(step);
                        pos = next + used;
                    }
                    // An unclosed `[` is the token being typed — everything
                    // after it, to the end of the text.
                    BracketScan::Open { quote, partial } => {
                        return Some(ParsedPath {
                            steps,
                            partial,
                            partial_len: s.len() - next,
                            slot: Slot::Bracket { quote },
                        });
                    }
                    BracketScan::Other => return None,
                }
            }
            // A quoted identifier: JMESPath's `foo."a b"` / `"a b"`, or
            // jq's `.foo."a b"`.
            '"' if cur.is_empty() && (prev_was_dot || at_start) => {
                let (raw, used) = scan_quoted(&s[next..], '"')?;
                steps.push(Step::Key(unescape_quoted(&raw, '"')?));
                pos = next + used;
            }
            // `.*`, the object-value wildcard.
            '*' if cur.is_empty() && prev_was_dot => {
                steps.push(Step::Each(Each::DotStar));
                pos = next;
            }
            c if c.is_alphanumeric() || c == '_' || c == '-' => {
                cur.push(c);
                pos = next;
            }
            _ => return None,
        }
        prev_was_dot = false;
        at_start = false;
    }

    Some(ParsedPath {
        steps,
        partial_len: cur.len(),
        partial: cur,
        slot: Slot::Token,
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
        // No keywords: the only Pointer syntax to complete beyond the
        // document's own keys is the `~` escape, see `pointer_escapes`.
        Kind::JsonPointer => &[],
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

    // ---- helpers ---------------------------------------------------------

    /// Marks the cursor inside a test query where it isn't at the end.
    const CURSOR: char = '‸';

    /// Split a test query at its [`CURSOR`] marker (at the end if absent).
    fn split_cursor(marked: &str) -> (String, usize) {
        match marked.find(CURSOR) {
            Some(i) => (marked.replace(CURSOR, ""), i),
            None => (marked.to_string(), marked.len()),
        }
    }

    /// Run `suggest` on a query whose cursor is marked with [`CURSOR`].
    fn suggest_at(marked: &str, explicit: Option<Kind>, doc: &Value) -> (String, Vec<Suggestion>) {
        let (text, cursor) = split_cursor(marked);
        let items = suggest(&text, cursor, explicit, Some(doc));
        (text, items)
    }

    /// `text` after splicing in `s`, as the query box would.
    fn accept(text: &str, s: &Suggestion) -> String {
        let mut t = text.to_string();
        t.replace_range(s.replace.clone(), &s.insert);
        t
    }

    fn labels(items: &[Suggestion]) -> Vec<&str> {
        items.iter().map(|s| s.label.as_str()).collect()
    }

    /// Query text after accepting the candidate labelled `label`.
    fn accepting(marked: &str, explicit: Option<Kind>, doc: &Value, label: &str) -> String {
        let (text, items) = suggest_at(marked, explicit, doc);
        let s = items
            .iter()
            .find(|s| s.label == label)
            .unwrap_or_else(|| panic!("{marked:?}: no {label:?} in {:?}", labels(&items)));
        accept(&text, s)
    }

    /// A document with everything the completions have to spell correctly:
    /// arrays of objects, nested arrays, and keys that aren't identifiers.
    fn awkward_doc() -> Value {
        json!({
            "members": [
                {"name": "Ada", "age": 36, "skills": ["rust", "go"]},
                {"name": "Bo", "age": 20, "skills": []}
            ],
            "matrix": [[1, 2], [3, 4]],
            "first name": "Zed",
            "my-key": 1,
            "1st": 2,
            "名前": 3,
            "a.b": 4,
            "quo\"te": 5,
            "name": "root",
            "x": {"first name": 9, "plain": 10},
            "list": [{"first name": 1}]
        })
    }

    // ---- `[`: indices and bracketed keys ---------------------------------

    #[test]
    fn open_bracket_on_a_root_array_offers_its_indices_and_no_keywords() {
        let arr = json!(["a", "b", "c"]);
        let (_, items) = suggest_at("[", Some(Kind::Jq), &arr);
        assert_eq!(labels(&items), ["0", "1", "2"]);
        assert_eq!(items[1].detail, "\"b\"");
    }

    #[test]
    fn open_bracket_at_query_start_indexes_the_root_with_a_leading_dot_in_jq() {
        // In jq a `[` at the very start is an array *literal*: accepting a
        // bare `[1]` would silently return `[1]`, not element 1.
        let arr = json!(["a", "b", "c"]);
        assert_eq!(accepting("[", Some(Kind::Jq), &arr, "1"), ".[1]");
        // Auto scope can't tell jq from JMESPath yet, and `Kind::detect`
        // runs unmarked text as jq — so the jq spelling is the safe one.
        assert_eq!(accepting("[", None, &arr, "1"), ".[1]");
    }

    #[test]
    fn open_bracket_at_query_start_has_no_dot_in_jmespath_or_jsonpath() {
        let arr = json!(["a", "b", "c"]);
        assert_eq!(accepting("[", Some(Kind::JmesPath), &arr, "1"), "[1]");
        assert_eq!(accepting("$[", Some(Kind::JsonPath), &arr, "1"), "$[1]");
    }

    #[test]
    fn open_bracket_after_a_path_completes_the_index_in_every_dialect() {
        let doc = awkward_doc();
        assert_eq!(accepting(".members[", None, &doc, "1"), ".members[1]");
        assert_eq!(accepting("$.members[", None, &doc, "1"), "$.members[1]");
        assert_eq!(
            accepting("members[", Some(Kind::JmesPath), &doc, "1"),
            "members[1]"
        );
        assert_eq!(accepting(".matrix[1][", None, &doc, "0"), ".matrix[1][0]");
    }

    #[test]
    fn open_bracket_digits_filter_the_indices_by_prefix() {
        let arr = Value::Array((0..12).map(Value::from).collect());
        let (_, items) = suggest_at(".[1", None, &arr);
        assert_eq!(labels(&items), ["1", "10", "11"]);
        assert_eq!(accepting(".[1", None, &arr, "10"), ".[10]");
    }

    #[test]
    fn bracket_completion_swallows_a_closing_bracket_already_there() {
        let doc = awkward_doc();
        assert_eq!(accepting(".members[‸]", None, &doc, "1"), ".members[1]");
        assert_eq!(
            accepting("[‸]", Some(Kind::Jq), &json!([7, 8]), "1"),
            ".[1]"
        );
        // ...and leaves whatever follows the `]` alone.
        assert_eq!(
            accepting(".members[‸].name", None, &doc, "1"),
            ".members[1].name"
        );
    }

    #[test]
    fn open_bracket_on_an_object_offers_quoted_keys_in_jq_and_jsonpath() {
        let doc = awkward_doc();
        let (_, items) = suggest_at(".x[", None, &doc);
        assert_eq!(labels(&items), ["first name", "plain"]);
        assert_eq!(accepting(".x[", None, &doc, "plain"), ".x[\"plain\"]");
        assert_eq!(
            accepting("$.x[", Some(Kind::JsonPath), &doc, "first name"),
            "$.x[\"first name\"]"
        );
        assert_eq!(accepting("[", Some(Kind::Jq), &doc, "name"), ".[\"name\"]");
    }

    #[test]
    fn open_bracket_on_an_object_offers_nothing_in_jmespath() {
        // `x["plain"]` is a syntax error in JMESPath.
        let doc = awkward_doc();
        let (_, items) = suggest_at("x[", Some(Kind::JmesPath), &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    #[test]
    fn typed_quote_after_a_bracket_filters_keys_and_swallows_the_closer() {
        let doc = awkward_doc();
        assert_eq!(
            accepting(".x[\"fi", None, &doc, "first name"),
            ".x[\"first name\"]"
        );
        assert_eq!(
            accepting(".x[\"fi‸\"]", None, &doc, "first name"),
            ".x[\"first name\"]"
        );
        // A single quote (JSONPath's other string spelling) is replaced too.
        assert_eq!(
            accepting("$.x['pl‸']", Some(Kind::JsonPath), &doc, "plain"),
            "$.x[\"plain\"]"
        );
    }

    #[test]
    fn open_bracket_never_dumps_keywords() {
        // The bug: a lone `[` listed `abs`, `add`, `all`, ... from the
        // keyword table, none of which belong inside an index.
        let doc = awkward_doc();
        for q in [".a[", ".nope[", ".members[0].name[", "$.matrix[0][0]["] {
            let (_, items) = suggest_at(q, None, &doc);
            assert!(items.is_empty(), "{q:?}: {:?}", labels(&items));
        }
        // A scalar root has nothing to index either.
        let (_, items) = suggest_at("[", Some(Kind::Jq), &json!(5));
        assert!(items.is_empty(), "{:?}", labels(&items));
        // Not even with no document to resolve against.
        assert!(suggest("[", 1, Some(Kind::Jq), None).is_empty());
        assert!(suggest(".a[", 3, None, None).is_empty());
    }

    #[test]
    fn brackets_holding_something_other_than_an_index_offer_no_document_candidates() {
        let doc = awkward_doc();
        for q in [
            ".members[?",
            ".members[*",
            ".members[1:",
            ".members[.",
            ".members[-",
            "members[?age > 1].",
            ".members[1:2:3].",
            "members[::2].",
        ] {
            let (text, cursor) = split_cursor(q);
            let scope = engines_in_scope(None, &text[..cursor]);
            assert!(
                doc_candidates(&scope, &text, cursor, Some(&doc)).is_empty(),
                "{q:?}"
            );
        }
    }

    // ---- separators ------------------------------------------------------

    #[test]
    fn a_key_after_a_closing_bracket_gets_its_dot() {
        // The bug: `.members[0]` + `name` inserted `.members[0]name`.
        let doc = awkward_doc();
        assert_eq!(
            accepting(".members[0]", Some(Kind::Jq), &doc, "name"),
            ".members[0].name"
        );
        assert_eq!(
            accepting(".members[0]", None, &doc, "name"),
            ".members[0].name"
        );
        assert_eq!(
            accepting("members[0]", Some(Kind::JmesPath), &doc, "name"),
            "members[0].name"
        );
        assert_eq!(
            accepting("$.members[0]", None, &doc, "name"),
            "$.members[0].name"
        );
    }

    #[test]
    fn a_key_after_the_jsonpath_root_gets_its_dot() {
        let doc = awkward_doc();
        assert_eq!(accepting("$", None, &doc, "name"), "$.name");
        assert_eq!(
            accepting("$", Some(Kind::JsonPath), &doc, "members"),
            "$.members"
        );
        // An index needs no separator at all.
        assert_eq!(accepting("$", None, &json!(["a", "b"]), "1"), "$[1]");
    }

    #[test]
    fn a_partly_typed_key_after_a_closing_bracket_or_root_gets_its_dot_too() {
        let doc = awkward_doc();
        assert_eq!(
            accepting(".members[0]na", None, &doc, "name"),
            ".members[0].name"
        );
        assert_eq!(accepting("$na", None, &doc, "name"), "$.name");
    }

    #[test]
    fn a_key_after_an_existing_dot_gets_no_second_one() {
        let doc = awkward_doc();
        assert_eq!(
            accepting(".members[0].", None, &doc, "name"),
            ".members[0].name"
        );
        assert_eq!(accepting("$.", None, &doc, "name"), "$.name");
        assert_eq!(accepting(".", None, &doc, "name"), ".name");
        assert_eq!(
            accepting("members[0].", Some(Kind::JmesPath), &doc, "name"),
            "members[0].name"
        );
    }

    #[test]
    fn an_index_after_a_closing_bracket_needs_no_separator() {
        let doc = awkward_doc();
        assert_eq!(accepting(".matrix[0]", None, &doc, "1"), ".matrix[0][1]");
        assert_eq!(accepting("$.matrix[0]", None, &doc, "1"), "$.matrix[0][1]");
    }

    #[test]
    fn json_pointer_needs_no_dot_handling() {
        let doc = awkward_doc();
        assert_eq!(
            accepting("/members/0/", None, &doc, "name"),
            "/members/0/name"
        );
        assert_eq!(accepting("/members/", None, &doc, "1"), "/members/1");
    }

    // ---- keys that aren't identifiers ------------------------------------

    #[test]
    fn awkward_keys_are_bracket_quoted_in_jq() {
        let doc = awkward_doc();
        for (label, want) in [
            ("first name", ".[\"first name\"]"),
            ("my-key", ".[\"my-key\"]"),
            ("1st", ".[\"1st\"]"),
            ("名前", ".[\"名前\"]"),
            ("a.b", ".[\"a.b\"]"),
            ("quo\"te", ".[\"quo\\\"te\"]"),
            // ...but a plain identifier stays plain.
            ("name", ".name"),
        ] {
            assert_eq!(accepting(".", Some(Kind::Jq), &doc, label), want);
        }
    }

    #[test]
    fn an_awkward_key_after_a_path_folds_the_separator_dot_away() {
        // `$.x.["k"]` is a JSONPath syntax error, and only some jq versions
        // accept `.x.["k"]` — `x["k"]` works everywhere.
        let doc = awkward_doc();
        assert_eq!(
            accepting(".x.", None, &doc, "first name"),
            ".x[\"first name\"]"
        );
        assert_eq!(
            accepting("$.x.", None, &doc, "first name"),
            "$.x[\"first name\"]"
        );
        assert_eq!(
            accepting(".list[0]", None, &doc, "first name"),
            ".list[0][\"first name\"]"
        );
        assert_eq!(
            accepting(".x.fi", None, &doc, "first name"),
            ".x[\"first name\"]"
        );
    }

    #[test]
    fn an_awkward_key_at_the_jsonpath_root_has_no_dot() {
        let doc = awkward_doc();
        assert_eq!(accepting("$.", None, &doc, "my-key"), "$[\"my-key\"]");
        assert_eq!(accepting("$", None, &doc, "my-key"), "$[\"my-key\"]");
    }

    #[test]
    fn awkward_keys_are_quoted_identifiers_in_jmespath() {
        let doc = awkward_doc();
        let jmes = Some(Kind::JmesPath);
        assert_eq!(
            accepting("x.", jmes, &doc, "first name"),
            "x.\"first name\""
        );
        assert_eq!(accepting("fir", jmes, &doc, "first name"), "\"first name\"");
        assert_eq!(
            accepting("list[0]", jmes, &doc, "first name"),
            "list[0].\"first name\""
        );
    }

    #[test]
    fn json_pointer_escapes_awkward_keys_instead_of_quoting() {
        let doc = json!({"a/b": 1, "c~d": 2, "first name": 3});
        let ptr = Some(Kind::JsonPointer);
        assert_eq!(accepting("/", ptr, &doc, "a/b"), "/a~1b");
        assert_eq!(accepting("/", ptr, &doc, "c~d"), "/c~0d");
        assert_eq!(accepting("/", ptr, &doc, "first name"), "/first name");
    }

    #[test]
    fn completion_continues_past_quoted_segments() {
        // Accepting `.["a b"]` must leave a query the popup can keep
        // completing, so quoted segments have to parse in every spelling
        // the popup itself produces (plus JSONPath's single quotes).
        let doc = json!({"a b": {"c": 1}, "q\"t": {"c": 2}});
        for (q, want) in [
            (".[\"a b\"].", ".[\"a b\"].c"),
            (".\"a b\".", ".\"a b\".c"),
            ("$[\"a b\"].", "$[\"a b\"].c"),
            ("$['a b'].", "$['a b'].c"),
            (".[\"q\\\"t\"].", ".[\"q\\\"t\"].c"),
        ] {
            assert_eq!(accepting(q, None, &doc, "c"), want);
        }
        assert_eq!(
            accepting("\"a b\".", Some(Kind::JmesPath), &doc, "c"),
            "\"a b\".c"
        );
        // A key typed right after a quoted JMESPath segment joins with a dot.
        assert_eq!(
            accepting("\"a b\"c", Some(Kind::JmesPath), &doc, "c"),
            "\"a b\".c"
        );
    }

    // ---- keywords only where a name can start -----------------------------

    #[test]
    fn no_keywords_right_after_a_dot() {
        // `.first` reads the field "first" — it doesn't call the builtin —
        // so offering `first`, `flatten`, ... after a `.` was wrong.
        let doc = awkward_doc();
        let (_, items) = suggest_at(".f", None, &doc);
        assert_eq!(labels(&items), ["first name"]);
        let (_, items) = suggest_at(".m", None, &doc);
        assert_eq!(labels(&items), ["members", "matrix", "my-key"]);
        // With no document there is simply nothing to offer after a dot.
        assert!(suggest(".f", 2, None, None).is_empty());
        assert!(suggest(".", 1, Some(Kind::Jq), None).is_empty());
        assert!(suggest("$.", 2, None, None).is_empty());
        assert!(suggest("..z", 3, None, None).is_empty());
        assert!(suggest(".a.b.na", 7, None, None).is_empty());
    }

    #[test]
    fn no_keywords_right_after_a_closing_bracket_paren_or_sigil() {
        assert!(suggest(".a[0]", 5, None, None).is_empty());
        assert!(suggest(".a[0]ab", 7, None, None).is_empty());
        assert!(suggest("select(.a)", 10, None, None).is_empty());
        assert!(suggest("select(.a)an", 12, None, None).is_empty());
        assert!(suggest(". as $so", 8, None, None).is_empty());
        assert!(suggest("@.so", 4, Some(Kind::JmesPath), None).is_empty());
    }

    #[test]
    fn no_keywords_inside_string_literals() {
        for (q, explicit) in [
            (".a == \"so", Some(Kind::Jq)),
            (".a == \"x\" and .b == \"so", Some(Kind::Jq)),
            ("a == 'so", Some(Kind::JmesPath)),
            ("a == `so", Some(Kind::JmesPath)),
        ] {
            assert!(
                suggest(q, q.len(), explicit, None).is_empty(),
                "{q:?}: {:?}",
                labels(&suggest(q, q.len(), explicit, None))
            );
        }
        // An escaped quote doesn't end the string...
        let q = ".a == \"x\\\"so";
        assert!(suggest(q, q.len(), Some(Kind::Jq), None).is_empty());
        // ...but a closed one does: keywords are back afterwards.
        let q = ".a == \"x\" | so";
        assert!(!suggest(q, q.len(), Some(Kind::Jq), None).is_empty());
    }

    #[test]
    fn keywords_are_still_offered_where_a_name_can_start() {
        for (q, explicit, want) in [
            (".a | ma", Some(Kind::Jq), "map"),
            (".a|ma", Some(Kind::Jq), "map"),
            ("map(sel", Some(Kind::Jq), "select"),
            ("[so", Some(Kind::Jq), "sort"),
            (".a, so", Some(Kind::Jq), "sort"),
            ("{a: so", Some(Kind::Jq), "sort"),
            ("people[?length", Some(Kind::JmesPath), "length"),
            ("$..book[?len", Some(Kind::JsonPath), "length"),
            ("so", None, "sort"),
        ] {
            let items = suggest(q, q.len(), explicit, None);
            assert!(labels(&items).iter().any(|l| l.starts_with(want)), "{q:?}");
        }
        // An empty word after a pipe still lists the keywords as a fallback.
        assert!(!suggest(".a | ", 5, Some(Kind::Jq), None).is_empty());
    }

    // ---- which dialect a path belongs to ----------------------------------

    #[test]
    fn explicit_jq_only_completes_dotted_paths() {
        // jq has no bare `foo` path — that's a function call.
        let doc = awkward_doc();
        let (_, items) = suggest_at("na", Some(Kind::Jq), &doc);
        assert!(!labels(&items).contains(&"name"), "{:?}", labels(&items));
    }

    #[test]
    fn explicit_jmespath_never_completes_a_leading_dot_path() {
        // JMESPath has no identity dot: `.name` is a syntax error there.
        let doc = awkward_doc();
        let (_, items) = suggest_at(".na", Some(Kind::JmesPath), &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    #[test]
    fn a_path_starting_with_a_closed_bracket_is_only_a_jmespath_path() {
        // `[0].name` is JMESPath; to jq it's a literal array, then a lookup.
        let doc = json!([{"name": "Ada"}]);
        let (_, items) = suggest_at("[0].", Some(Kind::Jq), &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
        assert_eq!(
            accepting("[0].", Some(Kind::JmesPath), &doc, "name"),
            "[0].name"
        );
    }

    #[test]
    fn leading_whitespace_is_ignored_and_a_blank_query_offers_nothing() {
        let doc = awkward_doc();
        assert_eq!(accepting("  .na", None, &doc, "name"), "  .name");
        assert_eq!(accepting("  [", Some(Kind::Jq), &json!([5]), "0"), "  .[0]");
        assert!(suggest("   ", 3, None, Some(&doc)).is_empty());
    }

    #[test]
    fn a_bare_number_at_query_start_is_not_an_index_completion() {
        // `1 + 2`: the `1` is a number being typed, not an index into the
        // document's root array.
        let arr = json!(["a", "b", "c"]);
        let (_, items) = suggest_at("1", None, &arr);
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    // ---- JSON Pointer -----------------------------------------------------

    #[test]
    fn pointer_escaped_token_is_replaced_whole() {
        // The bug: the replaced range was measured in *unescaped* bytes, so
        // `/a~1` + `a/b` produced `/aa~1b`.
        let doc = json!({"a/b": {"c": 1}, "a~b": 2});
        let ptr = Some(Kind::JsonPointer);
        assert_eq!(accepting("/a~1", ptr, &doc, "a/b"), "/a~1b");
        assert_eq!(accepting("/a~0", ptr, &doc, "a~b"), "/a~0b");
        assert_eq!(accepting("/a~1b/", ptr, &doc, "c"), "/a~1b/c");
    }

    #[test]
    fn pointer_index_partial_is_not_offered_the_tilde_escape() {
        // The bug: typing `/items/1` also listed a `~1` "hint" row, and
        // accepting it turned the `1` into `~1`.
        let doc = json!({"items": [0, 1, 2]});
        let (_, items) = suggest_at("/items/1", None, &doc);
        assert_eq!(labels(&items), ["1"]);
        assert!(suggest("/1", 2, None, None).is_empty());
    }

    #[test]
    fn pointer_tilde_offers_its_two_escapes() {
        let doc = json!({"a/b": 1});
        let items = suggest("/a~", 3, None, Some(&doc));
        assert_eq!(labels(&items), ["~0", "~1"]);
        assert_eq!(accept("/a~", &items[1]), "/a~1");
        assert!(suggest("/a", 2, None, Some(&doc))
            .iter()
            .all(|s| !s.label.starts_with('~')));
    }

    // ---- accepted text actually works --------------------------------------

    /// The strongest check on all of the above: accept *every* document
    /// candidate for a spread of queries, run the resulting text through the
    /// real engine for its dialect, and require a single result equal to the
    /// value the popup previewed for that row.
    #[test]
    fn every_accepted_candidate_runs_and_yields_the_value_it_previewed() {
        use jsonquery_core::engine::QueryEvent;
        use std::sync::atomic::AtomicBool;

        let doc = awkward_doc();
        let arr = json!(["a", "b", "c"]);
        let jq = Some(Kind::Jq);
        let jp = Some(Kind::JsonPath);
        let jm = Some(Kind::JmesPath);
        let ptr = Some(Kind::JsonPointer);
        let cases: Vec<(&Value, Option<Kind>, &str)> = vec![
            // jq, explicit and auto
            (&doc, jq, "."),
            (&doc, None, "."),
            (&doc, jq, ".m"),
            (&doc, jq, ".x."),
            (&doc, jq, ".x.fi"),
            (&doc, jq, ".x["),
            (&doc, jq, ".x[\""),
            (&doc, jq, ".x[\"pl‸\"]"),
            (&doc, jq, "["),
            (&doc, jq, ".members["),
            (&doc, jq, ".members[‸]"),
            (&doc, jq, ".members[1"),
            (&doc, jq, ".members[0]"),
            (&doc, jq, ".members[0]na"),
            (&doc, jq, ".members[0]."),
            (&doc, jq, ".members[0].skills"),
            (&doc, jq, ".members[0].skills["),
            (&doc, jq, ".matrix[1]"),
            (&doc, jq, ".matrix[1]["),
            (&doc, jq, ".list[0]"),
            (&doc, jq, ".list[0]."),
            (&doc, jq, ".[\"x\"]."),
            (&doc, jq, ".\"x\"."),
            (&arr, jq, "."),
            (&arr, jq, "["),
            (&arr, None, "["),
            (&arr, jq, ".["),
            // JSONPath
            (&doc, jp, "$"),
            (&doc, None, "$"),
            (&doc, jp, "$."),
            (&doc, jp, "$na"),
            (&doc, jp, "$.x."),
            (&doc, jp, "$.x["),
            (&doc, jp, "$["),
            (&doc, jp, "$['x']."),
            (&doc, jp, "$.members["),
            (&doc, jp, "$.members[0]"),
            (&doc, jp, "$.matrix[1]["),
            (&doc, jp, "$.list[0]"),
            (&arr, jp, "$"),
            (&arr, jp, "$["),
            // JMESPath
            (&doc, jm, "f"),
            (&doc, jm, "m"),
            (&doc, jm, "1"),
            (&doc, jm, "q"),
            (&doc, jm, "x."),
            (&doc, jm, "\"x\"."),
            (&doc, jm, "members["),
            (&doc, jm, "members[‸]"),
            (&doc, jm, "members[0]"),
            (&doc, jm, "members[0].skills["),
            (&doc, jm, "matrix[1]["),
            (&doc, jm, "list[0]"),
            (&arr, jm, "["),
            // JSON Pointer
            (&doc, ptr, "/"),
            (&doc, ptr, "/x/"),
            (&doc, ptr, "/members/"),
            (&doc, ptr, "/members/0/"),
            (&doc, ptr, "/members/0/skills/"),
            (&arr, ptr, "/"),
        ];

        for (d, explicit, marked) in cases {
            let (text, cursor) = split_cursor(marked);
            let scope = engines_in_scope(explicit, &text[..cursor]);
            let items = doc_candidates(&scope, &text, cursor, Some(d));
            assert!(!items.is_empty(), "{marked:?}: no candidates at all");

            for s in &items {
                let accepted = accept(&text, s);
                let kind = explicit.unwrap_or_else(|| Kind::detect(&accepted));
                let mut got = Vec::new();
                kind.engine()
                    .run(d, &accepted, &AtomicBool::new(false), &mut |ev| {
                        if let QueryEvent::Item(v) = ev {
                            got.push(v);
                        }
                    })
                    .unwrap_or_else(|e| {
                        panic!(
                            "{marked:?} + {:?} -> {accepted:?} ({}): {e}",
                            s.label,
                            kind.label()
                        )
                    });
                assert_eq!(
                    got.len(),
                    1,
                    "{marked:?} + {:?} -> {accepted:?} ({}) gave {got:?}",
                    s.label,
                    kind.label()
                );
                assert_eq!(
                    value_preview(&got[0]),
                    s.detail,
                    "{marked:?} + {:?} -> {accepted:?} ({})",
                    s.label,
                    kind.label()
                );
            }
        }
    }

    // ---- wildcards, negative indices and slices ---------------------------

    /// Elements that don't all have the same keys, an object of objects, and
    /// nested arrays of different lengths.
    fn projection_doc() -> Value {
        json!({
            "members": [
                {"name": "Ada", "age": 36, "skills": ["rust", "go"]},
                {"name": "Bo", "age": 20, "lang": "go"},
                {"name": "Cy", "extra": {"deep": 1}}
            ],
            "byname": {"ada": {"age": 1}, "bo": {"age": 2, "lang": "go"}},
            "matrix": [[1, 2], [3, 4, 5]]
        })
    }

    #[test]
    fn a_wildcard_completes_with_the_union_of_every_elements_keys() {
        // The gap this closes: `.members[].na` used to offer nothing.
        let doc = projection_doc();
        let (_, items) = suggest_at(".members[].", None, &doc);
        // First-seen order across the elements, each key once.
        assert_eq!(labels(&items), ["name", "age", "skills", "lang", "extra"]);
        // Previewed from the first element that has it.
        assert_eq!(items[3].detail, "\"go\"");
        assert_eq!(
            accepting(".members[]", None, &doc, "name"),
            ".members[].name"
        );
        assert_eq!(
            accepting(".members[].la", None, &doc, "lang"),
            ".members[].lang"
        );
    }

    #[test]
    fn each_wildcard_spelling_completes_in_the_dialects_that_have_it() {
        let doc = projection_doc();
        let jm = Some(Kind::JmesPath);
        let jp = Some(Kind::JsonPath);
        assert_eq!(accepting("members[]", jm, &doc, "name"), "members[].name");
        assert_eq!(accepting("members[*]", jm, &doc, "name"), "members[*].name");
        assert_eq!(accepting("byname.*.", jm, &doc, "age"), "byname.*.age");
        assert_eq!(
            accepting("$.members[*]", jp, &doc, "name"),
            "$.members[*].name"
        );
        // After `.*` the key still needs its own dot.
        assert_eq!(accepting("$.byname.*", jp, &doc, "age"), "$.byname.*.age");
        assert_eq!(accepting("byname.*", jm, &doc, "age"), "byname.*.age");
    }

    #[test]
    fn a_wildcard_only_completes_where_its_dialect_has_it() {
        // jq has no `[*]` / `.*` (`.*` is a multiplication); JSONPath no `[]`.
        let doc = projection_doc();
        for (q, explicit) in [
            (".members[*].", Some(Kind::Jq)),
            (".byname.*.", Some(Kind::Jq)),
            ("$.members[].", Some(Kind::JsonPath)),
            ("$.members[]", Some(Kind::JsonPath)),
        ] {
            let (_, items) = suggest_at(q, explicit, &doc);
            assert!(items.is_empty(), "{q:?}: {:?}", labels(&items));
        }
    }

    #[test]
    fn a_wildcard_reads_arrays_and_objects_as_each_dialect_does() {
        let doc = projection_doc();
        // jq's `[]` iterates an object's values too.
        let (_, items) = suggest_at(".byname[].", None, &doc);
        assert_eq!(labels(&items), ["age", "lang"]);
        // JMESPath is stricter: `[*]` is arrays only, `.*` objects only.
        let (_, items) = suggest_at("byname[*].", Some(Kind::JmesPath), &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
        let (_, items) = suggest_at("members.*.", Some(Kind::JmesPath), &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
        // JSONPath's `[*]` and `.*` take both.
        let (_, items) = suggest_at("$.byname[*].", Some(Kind::JsonPath), &doc);
        assert_eq!(labels(&items), ["age", "lang"]);
        let (_, items) = suggest_at("$.members.*.", Some(Kind::JsonPath), &doc);
        assert_eq!(labels(&items), ["name", "age", "skills", "lang", "extra"]);
    }

    #[test]
    fn a_bracket_after_a_wildcard_completes_indices_over_every_element() {
        let doc = projection_doc();
        // Two inner arrays, of length 2 and 3: indices up to the longest,
        // each previewed from the first array that has it.
        let (_, items) = suggest_at(".matrix[][", None, &doc);
        assert_eq!(labels(&items), ["0", "1", "2"]);
        assert_eq!(items[2].detail, "5");
        assert_eq!(accepting(".matrix[][", None, &doc, "2"), ".matrix[][2]");
        assert_eq!(
            accepting("matrix[*][", Some(Kind::JmesPath), &doc, "1"),
            "matrix[*][1]"
        );
        assert_eq!(accepting(".matrix[]", None, &doc, "1"), ".matrix[][1]");
    }

    #[test]
    fn jmespath_flatten_splices_nested_arrays_one_level() {
        // `matrix[]` is [1, 2, 3, 4, 5] in JMESPath — numbers, with nothing
        // to index — where jq's `.matrix[]` yields the inner arrays.
        let doc = projection_doc();
        let (_, items) = suggest_at("matrix[][", Some(Kind::JmesPath), &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
        // `[*]` doesn't flatten: the inner arrays are still there.
        let (_, items) = suggest_at("matrix[*][", Some(Kind::JmesPath), &doc);
        assert_eq!(labels(&items), ["0", "1", "2"]);
    }

    #[test]
    fn nothing_to_complete_past_a_wildcard_over_scalars() {
        let doc = projection_doc();
        let (_, items) = suggest_at(".members[].name.", None, &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
        let (_, items) = suggest_at(".members[].name[", None, &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    #[test]
    fn negative_indices_resolve_from_the_end() {
        let doc = projection_doc();
        let (_, items) = suggest_at(".members[-1].", None, &doc);
        assert_eq!(labels(&items), ["name", "extra"]);
        assert_eq!(
            accepting(".members[-1]", None, &doc, "extra"),
            ".members[-1].extra"
        );
        let (_, items) = suggest_at(".matrix[-1][", None, &doc);
        assert_eq!(labels(&items), ["0", "1", "2"]);
        let (_, items) = suggest_at(".matrix[-2][", None, &doc);
        assert_eq!(labels(&items), ["0", "1"]);
        // Further back than the array is long, and `[-0]`, reach nothing.
        for q in [".members[-4].", ".members[-0]."] {
            let (_, items) = suggest_at(q, None, &doc);
            assert!(items.is_empty(), "{q:?}: {:?}", labels(&items));
        }
        assert_eq!(
            accepting("members[-1].", Some(Kind::JmesPath), &doc, "name"),
            "members[-1].name"
        );
        assert_eq!(
            accepting("$.members[-1].", Some(Kind::JsonPath), &doc, "name"),
            "$.members[-1].name"
        );
    }

    #[test]
    fn slices_select_elements_in_jmespath_and_jsonpath() {
        let doc = projection_doc();
        let jm = Some(Kind::JmesPath);
        for (q, want) in [
            ("members[0:1].", vec!["name", "age", "skills"]),
            ("members[1:].", vec!["name", "age", "lang", "extra"]),
            ("members[:-1].", vec!["name", "age", "skills", "lang"]),
            ("members[-1:].", vec!["name", "extra"]),
            ("members[ 0 : 2 ].", vec!["name", "age", "skills", "lang"]),
        ] {
            let (_, items) = suggest_at(q, jm, &doc);
            assert_eq!(labels(&items), want, "{q:?}");
        }
        // Empty ranges select nothing.
        for q in ["members[5:].", "members[2:1]."] {
            let (_, items) = suggest_at(q, jm, &doc);
            assert!(items.is_empty(), "{q:?}: {:?}", labels(&items));
        }
        assert_eq!(
            accepting("$.members[0:1]", Some(Kind::JsonPath), &doc, "name"),
            "$.members[0:1].name"
        );
    }

    #[test]
    fn jq_slices_and_stepped_slices_end_completion() {
        // jq's `.[1:3]` is a fresh array, not a projection, and no dialect's
        // stepped slice (`[::2]`) is modelled.
        let doc = projection_doc();
        for (q, explicit) in [
            (".members[0:1].", Some(Kind::Jq)),
            (".members[0:1][", Some(Kind::Jq)),
            ("members[::2].", Some(Kind::JmesPath)),
            ("$.members[0:3:2].", Some(Kind::JsonPath)),
        ] {
            let (_, items) = suggest_at(q, explicit, &doc);
            assert!(items.is_empty(), "{q:?}: {:?}", labels(&items));
        }
    }

    #[test]
    fn slice_ranges_clamp_and_count_from_the_end() {
        assert_eq!(slice_range(5, Some(1), Some(3)), 1..3);
        assert_eq!(slice_range(5, None, None), 0..5);
        assert_eq!(slice_range(5, None, Some(-1)), 0..4);
        assert_eq!(slice_range(5, Some(-2), None), 3..5);
        assert_eq!(slice_range(5, Some(-9), Some(2)), 0..2);
        assert_eq!(slice_range(5, Some(9), None), 5..5);
        assert_eq!(slice_range(5, Some(3), Some(1)), 1..1);
        assert_eq!(slice_range(0, Some(1), Some(2)), 0..0);
        assert_eq!(slice_range(3, Some(i64::MIN), Some(i64::MAX)), 0..3);
    }

    #[test]
    fn a_wildcard_only_samples_the_first_elements_of_a_long_array() {
        let rows: Vec<Value> = (0..NODE_CAP + 100)
            .map(|i| json!({ format!("k{i}"): i }))
            .collect();
        let doc = json!({ "rows": rows });
        let steps = [Step::Key("rows".into()), Step::Each(Each::Brackets)];
        assert_eq!(resolve_steps(&doc, &steps, Syntax::Jq).len(), NODE_CAP);

        // The popup's own list stays capped no matter how many keys the
        // sampled elements contribute between them.
        let (_, items) = suggest_at(".rows[].", Some(Kind::Jq), &doc);
        assert_eq!(items.len(), DOC_CANDIDATE_CAP);
        assert_eq!(items[0].label, "k0");
        let (_, items) = suggest_at(".rows[].k199", Some(Kind::Jq), &doc);
        assert_eq!(labels(&items), ["k199"]);
        let (_, items) = suggest_at(".rows[].k200", Some(Kind::Jq), &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
    }

    #[test]
    fn typing_a_negative_index_does_not_dump_keywords() {
        // `[-` is the start of `[-1]`, not of an expression.
        let doc = projection_doc();
        let (_, items) = suggest_at(".members[-", None, &doc);
        assert!(items.is_empty(), "{:?}", labels(&items));
        assert!(suggest(".a[-", 4, Some(Kind::Jq), None).is_empty());
        assert!(suggest(".a -", 4, Some(Kind::Jq), None).is_empty());
        // A fresh expression, after whitespace or a separator, still may.
        assert!(!suggest(".a - ", 5, Some(Kind::Jq), None).is_empty());
        assert!(!suggest("map(", 4, Some(Kind::Jq), None).is_empty());
    }

    /// The counterpart of the every-candidate round trip for paths that fan
    /// out. A wildcard yields many results (or, in JMESPath, one array of
    /// them), and jq gives `null` for an element lacking the key — so the
    /// check is that the value the popup previewed is *among* the results.
    #[test]
    fn every_accepted_wildcard_candidate_runs_and_includes_the_previewed_value() {
        use jsonquery_core::engine::QueryEvent;
        use std::sync::atomic::AtomicBool;

        let doc = projection_doc();
        let jq = Some(Kind::Jq);
        let jm = Some(Kind::JmesPath);
        let jp = Some(Kind::JsonPath);
        let cases: Vec<(Option<Kind>, &str)> = vec![
            (jq, ".members[]"),
            (jq, ".members[]."),
            (jq, ".members[].sk"),
            (jq, ".members[-1]"),
            (jq, ".members[-1]."),
            (jq, ".byname[]."),
            (jq, ".matrix[]["),
            (jq, ".matrix[-1]["),
            (jq, ".matrix[][‸]"),
            (jm, "members[]"),
            (jm, "members[]."),
            (jm, "members[*]."),
            (jm, "members[*]"),
            (jm, "byname.*."),
            (jm, "members[0:2]."),
            (jm, "members[-1]."),
            (jm, "matrix[*]["),
            (jm, "matrix[-1]["),
            (jp, "$.members[*]"),
            (jp, "$.members[*]."),
            (jp, "$.byname.*"),
            (jp, "$.byname.*."),
            (jp, "$.members[0:2]."),
            (jp, "$.members[-1]."),
            (jp, "$.matrix[*]["),
            (jp, "$.matrix[-1]["),
        ];

        for (explicit, marked) in cases {
            let (text, cursor) = split_cursor(marked);
            let scope = engines_in_scope(explicit, &text[..cursor]);
            let items = doc_candidates(&scope, &text, cursor, Some(&doc));
            assert!(!items.is_empty(), "{marked:?}: no candidates at all");

            for s in &items {
                let accepted = accept(&text, s);
                let kind = explicit.expect("every case names its engine");
                let mut got = Vec::new();
                kind.engine()
                    .run(&doc, &accepted, &AtomicBool::new(false), &mut |ev| {
                        if let QueryEvent::Item(v) = ev {
                            got.push(v);
                        }
                    })
                    .unwrap_or_else(|e| {
                        panic!(
                            "{marked:?} + {:?} -> {accepted:?} ({}): {e}",
                            s.label,
                            kind.label()
                        )
                    });
                // JMESPath returns a projection as one array result.
                if kind == Kind::JmesPath && got.len() == 1 {
                    if let Value::Array(items) = got[0].clone() {
                        got = items;
                    }
                }
                assert!(
                    got.iter().any(|v| value_preview(v) == s.detail),
                    "{marked:?} + {:?} -> {accepted:?} ({}): previewed {} but got {got:?}",
                    s.label,
                    kind.label(),
                    s.detail
                );
            }
        }
    }
}
