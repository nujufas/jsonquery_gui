//! Reading a jq program for the parts that can be run against a file rather than
//! a value in memory.
//!
//! A program is a pipeline: stages joined by `|`, each of which is run on what
//! the one before it produced. A file that is too big to be a value is
//! walked through its first stages, the ones that only look for something in it
//! — `.items`, `.[]`, `.[0:100]`, `length`, `map(…)`, `[…]`, `first(…)` — and
//! what comes out of those, a node at a time, is small enough to be a value and
//! is handed to jaq, which runs the rest of the program on it.
//!
//! Nothing here runs anything: this finds the stages, tells the ones that can be
//! walked from the rest, and keeps the text of each stage's remainder so that
//! jaq can be given it.

use jaq_core::load::lex::{Lexer, StrPart, Tok, Token};
use jaq_core::load::span;
use jsonquery_core::ValueKind;

/// One step of a path: `.name`, `.[3]`, `.[2:5]` or `.[]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Key(String),
    Index(i64),
    Slice(Option<i64>, Option<i64>),
    Each,
}

/// A step, and whether it has a `?` after it: errors from it are then no output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PathStep {
    pub step: Step,
    pub optional: bool,
}

/// What a stage does, as far as a file is concerned.
#[derive(Debug)]
pub(super) enum Kind {
    /// `.`
    Identity,
    /// A path of constant steps, such as `.users[0].name`.
    Path(Vec<PathStep>),
    /// `length`
    Length,
    /// `keys` and `keys_unsorted`
    Keys { sorted: bool },
    /// `[E]` and `map(F)`: what the pipeline makes, as an array.
    Collect(Pipeline),
    /// `first(E)` and `limit(n; E)`: its first `count` outputs.
    Limit { count: usize, inner: Pipeline },
    /// `E1, E2` and `(E)`: what each of the pipelines makes, one after the other.
    Each(Vec<Pipeline>),
    /// `type`
    Type,
    /// `strings`, `numbers`, `values`, …: the node itself if it is one of these,
    /// and nothing if it is not.
    OfType(Types),
    /// `select(E)`: the node itself, once for each output of `E` that is not
    /// `null` or `false`.
    Select(Pipeline),
    /// `..` and `recurse`: the node, and then what is below each of its children.
    Recurse,
    /// An expression made of walked parts: `{count: length}`, `.[-1] - .[0]`.
    Compose(Compose),
    /// `E?`: what `E` makes up to its first error, which is no output.
    Try(Pipeline),
    /// What needs a key of every element of a list: `sort_by(f)`, `group_by(f)`,
    /// `min_by(f)`, `sort`, `reverse`, …
    Keyed(Keyed),
    /// Anything else: it needs the value it is run on, all of it.
    Opaque,
}

/// What a list is put through that needs a key of each of its elements.
#[derive(Debug)]
pub(super) struct Keyed {
    pub op: KeyedOp,
    /// What makes the key of an element: `[f]` for `sort_by(f)`.
    pub key: Option<Pipeline>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyedOp {
    SortBy,
    GroupBy,
    UniqueBy,
    MinBy,
    MaxBy,
    Reverse,
}

/// A set of the kinds of value there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Types(u8);

impl Types {
    const fn of(kinds: &[ValueKind]) -> Self {
        let mut bits = 0u8;
        let mut at = 0;
        while at < kinds.len() {
            bits |= 1 << kinds[at] as u8;
            at += 1;
        }
        Self(bits)
    }

    pub(super) fn has(self, kind: ValueKind) -> bool {
        self.0 & (1 << kind as u8) != 0
    }
}

/// The filters that let through the values of some kinds: `jaq-std` defines
/// them as `select(…)` of a test of the kind.
fn types_named(name: &str) -> Option<Types> {
    use ValueKind::*;
    Some(match name {
        "nulls" => Types::of(&[Null]),
        "booleans" => Types::of(&[Bool]),
        "numbers" => Types::of(&[Number]),
        "strings" => Types::of(&[String]),
        "arrays" => Types::of(&[Array]),
        "objects" => Types::of(&[Object]),
        "iterables" => Types::of(&[Array, Object]),
        "scalars" => Types::of(&[Null, Bool, Number, String]),
        "values" => Types::of(&[Bool, Number, String, Array, Object]),
        _ => return None,
    })
}

/// An expression whose parts are pipelines that are walked, put together by
/// jaq: each part is run on the node, what it makes stands in the text where it
/// was, and jaq runs the text. `{count: length, first: .[0]}` is the text
/// `{count: $__l0, first: $__l1}` and the pipelines `length` and `.[0]`; what jaq
/// does with a part that makes no output, or many, or an error, is what it
/// does with the part itself.
#[derive(Debug)]
pub(super) struct Compose {
    pub pieces: Vec<Piece>,
    pub leaves: Vec<Pipeline>,
}

#[derive(Debug)]
pub(super) enum Piece {
    Text(String),
    /// What the pipeline of this number makes.
    Leaf(usize),
}

#[derive(Debug)]
pub(super) struct Stage {
    pub kind: Kind,
    /// The program from this stage to the end of its pipeline: what jaq is given
    /// to run when it is given the value this stage would have been run on.
    pub rest: String,
}

#[derive(Debug)]
pub(super) struct Pipeline {
    pub stages: Vec<Stage>,
}

/// Read `src` as a pipeline. A program that cannot be taken apart (it defines a
/// function, or binds a variable) is one stage that is run as it is.
pub(super) fn plan(src: &str) -> Pipeline {
    let whole = || Pipeline {
        stages: vec![Stage {
            kind: Kind::Opaque,
            rest: src.to_owned(),
        }],
    };
    let Ok(tokens) = Lexer::new(src).lex() else {
        return whole();
    };

    // Split at the `|` that are not inside an `if … end`; where a variable or a
    // function is declared the program is not a list of stages (what is
    // declared reaches the stages after it), so there is no splitting it.
    let mut cuts = Vec::new();
    let mut start = 0;
    let mut open_ifs = 0usize;
    for Token(text, tok) in &tokens {
        let at = span(src, text);
        match tok {
            Tok::Word => match *text {
                "if" => open_ifs += 1,
                "end" => open_ifs = open_ifs.saturating_sub(1),
                "as" | "def" | "label" | "import" | "include" => return whole(),
                _ => {}
            },
            Tok::Sym if *text == "|" && open_ifs == 0 => {
                cuts.push((start, at.start));
                start = at.end;
            }
            _ => {}
        }
    }
    cuts.push((start, src.len()));

    Pipeline {
        stages: cuts
            .into_iter()
            .flat_map(|(from, to)| stages_of(src, from, to))
            .collect(),
    }
}

/// The stage that is `src[from..to]`; two if it is a call that is walked and a
/// path of what it makes, as in `first(.[])[0]`.
fn stages_of(src: &str, from: usize, to: usize) -> Vec<Stage> {
    let text = src[from..to].trim();
    let offset = from + (src[from..to].len() - src[from..to].trim_start().len());
    if let Some(tail) = call_and_path(text) {
        let call = classify(text[..tail].trim());
        if !matches!(call, Kind::Opaque) {
            let steps = text[tail..].trim();
            // The path starts with a dot, as a program that is run alone has to.
            let dotted = if steps.starts_with('.') {
                steps.to_owned()
            } else {
                format!(".{steps}")
            };
            let after = src[offset + tail..].trim_start();
            let after = after.strip_prefix('.').unwrap_or(after);
            if let Some(path) = parse_path(&dotted) {
                return vec![
                    Stage {
                        kind: call,
                        rest: src[offset..].to_owned(),
                    },
                    Stage {
                        kind: Kind::Path(path),
                        rest: format!(".{after}"),
                    },
                ];
            }
        }
    }
    vec![Stage {
        kind: classify(text),
        rest: src[from..].trim_start().to_owned(),
    }]
}

/// Where the path begins in `name(arguments)[0].a` or `name[0]`, a call that is
/// followed by steps.
fn call_and_path(text: &str) -> Option<usize> {
    let tokens = Lexer::new(text).lex().ok()?;
    let [Token(_, Tok::Word), rest @ ..] = tokens.as_slice() else {
        return None;
    };
    let rest = match rest {
        [Token(block, Tok::Block(_)), after @ ..] if block.starts_with('(') => after,
        _ => rest,
    };
    let first = rest.first()?;
    let mut previous_dot = false;
    for Token(piece, tok) in rest {
        let step = match tok {
            Tok::Block(_) => piece.starts_with('['),
            Tok::Sym => *piece == "?" || piece.starts_with('.') && !piece.starts_with(".."),
            Tok::Str(_) => previous_dot,
            _ => false,
        };
        if !step {
            return None;
        }
        previous_dot = matches!(tok, Tok::Sym) && *piece == ".";
    }
    Some(span(text, first.0).start)
}

fn classify(text: &str) -> Kind {
    if text == "." {
        return Kind::Identity;
    }
    if let Some(steps) = parse_path(text) {
        return Kind::Path(steps);
    }
    match text {
        "length" => return Kind::Length,
        "keys" => return Kind::Keys { sorted: true },
        "keys_unsorted" => return Kind::Keys { sorted: false },
        "first" => return Kind::Path(vec![at(Step::Index(0))]),
        "last" => return Kind::Path(vec![at(Step::Index(-1))]),
        "type" => return Kind::Type,
        ".." | "recurse" => return Kind::Recurse,
        // `sort` is `sort_by(.)`, and so on, as jaq defines them.
        "sort" => return keyed(KeyedOp::SortBy, "."),
        "unique" => return keyed(KeyedOp::UniqueBy, "."),
        "min" => return keyed(KeyedOp::MinBy, "."),
        "max" => return keyed(KeyedOp::MaxBy, "."),
        "reverse" => {
            return Kind::Keyed(Keyed {
                op: KeyedOp::Reverse,
                key: None,
            })
        }
        _ => {}
    }
    if let Some(types) = types_named(text) {
        return Kind::OfType(types);
    }

    let Ok(tokens) = Lexer::new(text).lex() else {
        return Kind::Opaque;
    };

    // `E1, E2`: split at the commas that are not inside anything.
    let mut parts = Vec::new();
    let mut start = 0;
    for Token(piece, tok) in &tokens {
        if matches!(tok, Tok::Sym) && *piece == "," {
            let at = span(text, piece);
            parts.push(&text[start..at.start]);
            start = at.end;
        }
    }
    if !parts.is_empty() {
        parts.push(&text[start..]);
        return Kind::Each(parts.into_iter().map(|part| plan(part.trim())).collect());
    }

    // `E?` is `try E` for a term `E`: one that has operators in it (`a + b?`) has
    // the `?` after its last term alone.
    if let [head @ .., Token("?", Tok::Sym)] = tokens.as_slice() {
        if !head.is_empty() && !head.iter().any(|Token(piece, tok)| is_operator(piece, tok)) {
            let inner = text[..span(text, tokens.last().map_or("", |t| t.0)).start].trim_end();
            let inner_kind = classify(inner);
            if !matches!(inner_kind, Kind::Opaque) {
                return Kind::Try(plan(inner));
            }
        }
    }

    if let Some(compose) = compose(text, &tokens) {
        return Kind::Compose(compose);
    }

    match tokens.as_slice() {
        [Token(name, Tok::Word), Token(block, Tok::Block(_))] if block.starts_with('(') => {
            let args = split_arguments(inside(block));
            match (*name, args.as_slice()) {
                ("map", [f]) => Kind::Collect(each_then(plan(f))),
                ("select", [condition]) => Kind::Select(plan(condition)),
                ("sort_by", [f]) => keyed(KeyedOp::SortBy, f),
                ("group_by", [f]) => keyed(KeyedOp::GroupBy, f),
                ("unique_by", [f]) => keyed(KeyedOp::UniqueBy, f),
                ("min_by", [f]) => keyed(KeyedOp::MinBy, f),
                ("max_by", [f]) => keyed(KeyedOp::MaxBy, f),
                ("first", [e]) => Kind::Limit {
                    count: 1,
                    inner: plan(e),
                },
                ("limit", [n, e]) => match n.trim().parse::<usize>() {
                    Ok(count) if count >= 1 => Kind::Limit {
                        count,
                        inner: plan(e),
                    },
                    _ => Kind::Opaque,
                },
                _ => Kind::Opaque,
            }
        }
        [Token(block, Tok::Block(_))]
            if block.starts_with('[') && !inside(block).trim().is_empty() =>
        {
            Kind::Collect(plan(inside(block)))
        }
        [Token(block, Tok::Block(_))]
            if block.starts_with('(') && !inside(block).trim().is_empty() =>
        {
            Kind::Each(vec![plan(inside(block))])
        }
        _ => Kind::Opaque,
    }
}

/// What a stage is if it is an object made of parts (`{a: length}`) or parts
/// joined by operators (`.[-1] - .[0]`, `length > 1`): the text jaq is to run,
/// with the parts left out of it, and the parts as pipelines. `None` for
/// anything else, which is left as it is.
fn compose(text: &str, tokens: &[Token<&str>]) -> Option<Compose> {
    // A number, a string, `null`: the same whatever it is run on.
    if is_constant(tokens) || matches!(text, "{}" | "[]") {
        return Some(Compose {
            pieces: vec![Piece::Text(text.to_owned())],
            leaves: Vec::new(),
        });
    }
    if let [Token(block, Tok::Block(inner))] = tokens {
        return block
            .starts_with('{')
            .then(|| compose_object(text, inner))?;
    }
    compose_operators(text, tokens)
}

/// The words that make a program something other than terms and operators.
const KEYWORDS: &[&str] = &[
    "if", "then", "elif", "else", "end", "try", "catch", "reduce", "foreach", "label", "def", "as",
    "import", "include",
];

/// The operators that join two terms, for which jaq does what it does with
/// whatever the terms make.
fn is_operator(piece: &str, tok: &Tok<&str>) -> bool {
    match tok {
        Tok::Sym => matches!(
            piece,
            "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | "<=" | ">" | ">=" | "//"
        ),
        Tok::Word => matches!(piece, "and" | "or"),
        _ => false,
    }
}

/// A term whose value is its text, which jaq can be given as it is: it needs
/// no input.
fn is_constant(run: &[Token<&str>]) -> bool {
    match run {
        [Token(_, Tok::Num)] => true,
        [Token(word, Tok::Word)] => matches!(*word, "null" | "true" | "false"),
        [Token(_, Tok::Str(parts))] => parts.iter().all(|part| !matches!(part, StrPart::Term(_))),
        _ => false,
    }
}

/// `a + b`, `-a`, `a == b and c`: the terms are the parts.
fn compose_operators(text: &str, tokens: &[Token<&str>]) -> Option<Compose> {
    let mut pieces = Vec::new();
    let mut leaves = Vec::new();
    let mut found = false;
    // The terms that are run on the node are the stretches between operators.
    let mut run_from = 0;
    let mut expecting_term = true;

    let mut close_run = |upto: usize, from: usize, pieces: &mut Vec<Piece>| -> Option<()> {
        let run = &tokens[from..upto];
        if run.is_empty() {
            return None;
        }
        if is_constant(run) {
            pieces.push(Piece::Text(run_text(text, run).to_owned()));
        } else {
            pieces.push(Piece::Leaf(leaves.len()));
            leaves.push(plan(run_text(text, run)));
        }
        Some(())
    };

    for (at, Token(piece, tok)) in tokens.iter().enumerate() {
        match tok {
            Tok::Word if KEYWORDS.contains(piece) => return None,
            Tok::Var | Tok::Fmt => return None,
            _ => {}
        }
        if is_operator(piece, tok) {
            if expecting_term {
                // Only a minus sign can come where a term is expected.
                if *piece != "-" {
                    return None;
                }
                pieces.push(Piece::Text("-".to_owned()));
                found = true;
                run_from = at + 1;
            } else {
                close_run(at, run_from, &mut pieces)?;
                pieces.push(Piece::Text(format!(" {piece} ")));
                found = true;
                expecting_term = true;
                run_from = at + 1;
            }
            continue;
        }
        if matches!(tok, Tok::Sym) && !piece.starts_with('.') && *piece != "?" {
            // `:`, `;`, `=`, `|=`, `?//`: not something to take apart.
            return None;
        }
        expecting_term = false;
    }
    if !found || expecting_term {
        return None;
    }
    close_run(tokens.len(), run_from, &mut pieces)?;
    Some(Compose { pieces, leaves })
}

/// The text from the first token of a run to the end of the last.
fn run_text<'a>(text: &'a str, run: &[Token<&str>]) -> &'a str {
    match (run.first(), run.last()) {
        (Some(first), Some(last)) => &text[span(text, first.0).start..span(text, last.0).end],
        _ => "",
    }
}

/// `{a: E, b}`: each value, and a key that is worked out, is a part.
fn compose_object(text: &str, inner: &[Token<&str>]) -> Option<Compose> {
    // The last token the lexer gives for a block is its closing bracket.
    let (close, body) = inner.split_last()?;
    if close.0 != "}" {
        return None;
    }
    let mut pieces = vec![Piece::Text("{".to_owned())];
    let mut leaves = Vec::new();
    let mut leaf = |source: &str, pieces: &mut Vec<Piece>| {
        // A number or a string needs nothing of the node.
        match Lexer::new(source).lex() {
            Ok(tokens) if is_constant(&tokens) => pieces.push(Piece::Text(source.to_owned())),
            _ => {
                pieces.push(Piece::Leaf(leaves.len()));
                leaves.push(plan(source));
            }
        }
    };

    let mut entries: Vec<&[Token<&str>]> = Vec::new();
    let mut from = 0;
    for (at, Token(piece, tok)) in body.iter().enumerate() {
        if matches!(tok, Tok::Sym) && *piece == "," {
            entries.push(&body[from..at]);
            from = at + 1;
        }
    }
    entries.push(&body[from..]);
    if entries.iter().any(|entry| entry.is_empty()) {
        return None;
    }

    for (n, entry) in entries.iter().enumerate() {
        if n > 0 {
            pieces.push(Piece::Text(", ".to_owned()));
        }
        match entry {
            // `{a: …}` and `{"a b": …}`
            [Token(key, key_tok @ (Tok::Word | Tok::Str(_))), Token(":", Tok::Sym), value @ ..]
                if !value.is_empty() && !has_interpolation(key_tok) =>
            {
                pieces.push(Piece::Text(format!("{key}: ")));
                leaf(run_text(text, value), &mut pieces);
            }
            // `{(…): …}`
            [Token(key, Tok::Block(_)), Token(":", Tok::Sym), value @ ..]
                if key.starts_with('(') && !value.is_empty() =>
            {
                pieces.push(Piece::Text("(".to_owned()));
                leaf(&key[1..key.len() - 1], &mut pieces);
                pieces.push(Piece::Text("): ".to_owned()));
                leaf(run_text(text, value), &mut pieces);
            }
            // `{a}` is `{a: .a}`
            [Token(key, Tok::Word)] => {
                pieces.push(Piece::Text(format!("{key}: ")));
                leaf(&format!(".{key}"), &mut pieces);
            }
            [Token(key, key_tok @ Tok::Str(_))] if !has_interpolation(key_tok) => {
                pieces.push(Piece::Text(format!("{key}: ")));
                leaf(&format!(".{key}"), &mut pieces);
            }
            _ => return None,
        }
    }
    pieces.push(Piece::Text("}".to_owned()));
    Some(Compose { pieces, leaves })
}

fn has_interpolation(tok: &Tok<&str>) -> bool {
    matches!(tok, Tok::Str(parts) if parts.iter().any(|part| matches!(part, StrPart::Term(_))))
}

/// `sort_by(f)` and the others: a pipeline that makes the key of an element, the
/// array of what `f` makes of it, as jaq makes it.
fn keyed(op: KeyedOp, f: &str) -> Kind {
    Kind::Keyed(Keyed {
        op,
        key: Some(plan(&format!("[{f}]"))),
    })
}

fn at(step: Step) -> PathStep {
    PathStep {
        step,
        optional: false,
    }
}

/// `.[] | pipeline`: what `map(pipeline)` is made of.
fn each_then(pipeline: Pipeline) -> Pipeline {
    let mut stages = vec![Stage {
        kind: Kind::Path(vec![at(Step::Each)]),
        rest: String::new(),
    }];
    stages.extend(pipeline.stages);
    Pipeline { stages }
}

/// What is between the brackets of a block, which is all of what the lexer
/// gave: `(…)`, `[…]` or `{…}`.
fn inside(block: &str) -> &str {
    block
        .get(1..block.len().saturating_sub(1))
        .unwrap_or_default()
}

/// The arguments of a call, split at the semicolons that are not inside
/// anything.
fn split_arguments(inner: &str) -> Vec<&str> {
    let Ok(tokens) = Lexer::new(inner).lex() else {
        return Vec::new();
    };
    let mut args = Vec::new();
    let mut start = 0;
    for Token(piece, tok) in &tokens {
        if matches!(tok, Tok::Sym) && *piece == ";" {
            let at = span(inner, piece);
            args.push(inner[start..at.start].trim());
            start = at.end;
        }
    }
    args.push(inner[start..].trim());
    args
}

/// A path made only of steps that are known when the program is read: `.a`,
/// `."a b"`, `.["a"]`, `.[3]`, `.[-1]`, `.[]`, `.[2:5]`, `.[:5]`, `.[2:]`, each
/// with an optional `?`, joined as in `.a.b[0]?.c` (or `.a.[0]`). Anything else
/// — a space, a function, a step that is worked out — is not one.
pub(super) fn parse_path(text: &str) -> Option<Vec<PathStep>> {
    let b = text.as_bytes();
    if b.first() != Some(&b'.') || b.get(1) == Some(&b'.') {
        return None;
    }
    let mut steps = Vec::new();
    let mut i = 1;
    loop {
        let step = match *b.get(i)? {
            b'[' => {
                let close = bracket_end(text, i)?;
                let step = parse_bracket(&text[i + 1..close])?;
                i = close + 1;
                step
            }
            b'"' => {
                let end = string_end(text, i)?;
                let key = serde_json::from_str::<String>(&text[i..end]).ok()?;
                i = end;
                Step::Key(key)
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while b
                    .get(i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    i += 1;
                }
                Step::Key(text[start..i].to_owned())
            }
            _ => return None,
        };
        let optional = b.get(i) == Some(&b'?');
        if optional {
            i += 1;
        }
        steps.push(PathStep { step, optional });
        match b.get(i) {
            None => return Some(steps),
            // `.a.b`, `.a.[0]`, `.a."b"`
            Some(b'.') => i += 1,
            // `.a[0]`
            Some(b'[') => {}
            Some(_) => return None,
        }
    }
}

/// Where the `]` that closes the `[` at `open` is, if what is between them has
/// no brackets of its own and no string with one in.
fn bracket_end(text: &str, open: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = open + 1;
    while let Some(&c) = b.get(i) {
        match c {
            b']' => return Some(i),
            b'"' => i = string_end(text, i)?,
            b'[' => return None,
            _ => i += 1,
        }
    }
    None
}

/// Just past the string literal that starts at `open`.
fn string_end(text: &str, open: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = open + 1;
    while let Some(&c) = b.get(i) {
        match c {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    None
}

fn parse_bracket(inner: &str) -> Option<Step> {
    let inner = inner.trim();
    if inner.is_empty() {
        return Some(Step::Each);
    }
    if inner.starts_with('"') {
        let end = string_end(inner, 0)?;
        return (end == inner.len())
            .then(|| serde_json::from_str::<String>(inner).ok())
            .flatten()
            .map(Step::Key);
    }
    if let Some((from, to)) = inner.split_once(':') {
        let bound = |s: &str| -> Option<Option<i64>> {
            let s = s.trim();
            if s.is_empty() {
                Some(None)
            } else {
                integer(s).map(Some)
            }
        };
        return Some(Step::Slice(bound(from)?, bound(to)?));
    }
    integer(inner).map(Step::Index)
}

/// An integer as jq writes one: no sign but `-`, no leading zeros.
fn integer(s: &str) -> Option<i64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    let canonical = digits == "0" && s == "0"
        || digits.starts_with(|c: char| ('1'..='9').contains(&c))
            && digits.bytes().all(|c| c.is_ascii_digit());
    if canonical {
        s.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> PathStep {
        at(Step::Key(k.to_owned()))
    }

    #[test]
    fn paths_of_constant_steps_are_read() {
        assert_eq!(parse_path(".a"), Some(vec![key("a")]));
        assert_eq!(parse_path(".a.b_2"), Some(vec![key("a"), key("b_2")]));
        assert_eq!(parse_path(".\"a b\""), Some(vec![key("a b")]));
        assert_eq!(parse_path(".[\"a b\"]"), Some(vec![key("a b")]));
        assert_eq!(parse_path(".[ \"a\\\"b\" ]"), Some(vec![key("a\"b")]));
        assert_eq!(parse_path(".[3]"), Some(vec![at(Step::Index(3))]));
        assert_eq!(parse_path(".[-1]"), Some(vec![at(Step::Index(-1))]));
        assert_eq!(parse_path(".[0]"), Some(vec![at(Step::Index(0))]));
        assert_eq!(parse_path(".[]"), Some(vec![at(Step::Each)]));
        assert_eq!(parse_path(".[ ]"), Some(vec![at(Step::Each)]));
        assert_eq!(
            parse_path(".[2:5]"),
            Some(vec![at(Step::Slice(Some(2), Some(5)))])
        );
        assert_eq!(
            parse_path(".[:5]"),
            Some(vec![at(Step::Slice(None, Some(5)))])
        );
        assert_eq!(
            parse_path(".[-2:]"),
            Some(vec![at(Step::Slice(Some(-2), None))])
        );
        assert_eq!(
            parse_path(".a[0].b[]"),
            Some(vec![key("a"), at(Step::Index(0)), key("b"), at(Step::Each)])
        );
        assert_eq!(
            parse_path(".a.[0].\"b\""),
            Some(vec![key("a"), at(Step::Index(0)), key("b")])
        );
        assert_eq!(
            parse_path(".a?.[]?"),
            Some(vec![
                PathStep {
                    step: Step::Key("a".into()),
                    optional: true
                },
                PathStep {
                    step: Step::Each,
                    optional: true
                }
            ])
        );
    }

    #[test]
    fn what_is_not_a_constant_path_is_not_read_as_one() {
        for text in [
            "",
            ".",
            "..",
            ".a b",
            ". a",
            ".a | .b",
            ".a-b",
            ".a+1",
            ".[.n]",
            ".[1.5]",
            ".[01]",
            ".[-0]",
            ".[+1]",
            ".[1,2]",
            ".[\"a\",\"b\"]",
            ".[1:2:3]",
            ".[a:b]",
            ".[",
            ".a[",
            ".a]",
            ".\"a",
            ".\"a\\(1)\"",
            ".a??",
            ".[]x",
            "a",
            "[.a]",
            ".[\"a\"]x",
            ".[[1]]",
            ".1",
            ".[9223372036854775808]",
        ] {
            assert_eq!(parse_path(text), None, "{text:?}");
        }
    }

    fn kinds(src: &str) -> Vec<String> {
        plan(src)
            .stages
            .iter()
            .map(|s| match &s.kind {
                Kind::Identity => "id".to_owned(),
                Kind::Path(p) => format!("path{}", p.len()),
                Kind::Length => "length".to_owned(),
                Kind::Keys { sorted } => format!("keys{}", if *sorted { "" } else { "_unsorted" }),
                Kind::Collect(p) => format!("collect[{}]", p.stages.len()),
                Kind::Limit { count, inner } => format!("limit{count}[{}]", inner.stages.len()),
                Kind::Each(ps) => format!("each{}", ps.len()),
                Kind::Type => "type".to_owned(),
                Kind::OfType(_) => "of_type".to_owned(),
                Kind::Select(p) => format!("select[{}]", p.stages.len()),
                Kind::Recurse => "recurse".to_owned(),
                Kind::Compose(c) => format!("compose{}", c.leaves.len()),
                Kind::Try(p) => format!("try[{}]", p.stages.len()),
                Kind::Keyed(k) => format!("{:?}", k.op).to_lowercase(),
                Kind::Opaque => "opaque".to_owned(),
            })
            .collect()
    }

    #[test]
    fn a_program_is_split_at_its_pipes() {
        assert_eq!(kinds(".users[] | .name"), ["path2", "path1"]);
        assert_eq!(
            kinds(".[] | select(.a > 1) | .b"),
            ["path1", "select[1]", "path1"]
        );
        assert_eq!(kinds("."), ["id"]);
        assert_eq!(kinds(".items | length"), ["path1", "length"]);
        assert_eq!(kinds(".a|.b"), ["path1", "path1"]);
        assert_eq!(kinds("keys"), ["keys"]);
        assert_eq!(kinds("keys_unsorted"), ["keys_unsorted"]);
        assert_eq!(kinds("first"), ["path1"]);
        assert_eq!(kinds("last"), ["path1"]);
    }

    #[test]
    fn the_calls_that_can_be_walked_are_read() {
        assert_eq!(kinds("map(.a)"), ["collect[2]"]);
        assert_eq!(kinds("map(.a | .b)"), ["collect[3]"]);
        assert_eq!(kinds("map(select(.a)) | length"), ["collect[2]", "length"]);
        assert_eq!(kinds("[.[] | .a]"), ["collect[2]"]);
        assert_eq!(kinds("[.[]]"), ["collect[1]"]);
        assert_eq!(kinds("first(.[] | select(.a))"), ["limit1[2]"]);
        assert_eq!(kinds("limit(10; .[])"), ["limit10[1]"]);
        assert_eq!(kinds(".[0], .[-1]"), ["each2"]);
        assert_eq!(kinds("(.a | .b) | .c"), ["each1", "path1"]);
        // A program that has a count that is not one, or no body, is not walked.
        assert_eq!(kinds("limit(0; .[])"), ["opaque"]);
        assert_eq!(kinds("limit(-1; .[])"), ["opaque"]);
        assert_eq!(kinds("limit(n; .[])"), ["opaque"]);
        assert_eq!(kinds("map(.a; .b)"), ["opaque"]);
        assert_eq!(kinds("add"), ["opaque"]);
        assert_eq!(kinds(".a += 1"), ["opaque"]);
        assert_eq!(kinds(".a |= 1"), ["opaque"]);
    }

    #[test]
    fn what_needs_a_key_of_every_element_is_read() {
        assert_eq!(kinds("sort_by(.a)"), ["sortby"]);
        assert_eq!(kinds("sort"), ["sortby"]);
        assert_eq!(kinds("group_by(.a)"), ["groupby"]);
        assert_eq!(kinds("unique_by(.a)"), ["uniqueby"]);
        assert_eq!(kinds("unique"), ["uniqueby"]);
        assert_eq!(kinds("min_by(.a)"), ["minby"]);
        assert_eq!(kinds("min"), ["minby"]);
        assert_eq!(kinds("max_by(.a)"), ["maxby"]);
        assert_eq!(kinds("max"), ["maxby"]);
        assert_eq!(kinds("reverse"), ["reverse"]);
        assert_eq!(kinds("sort_by(.a) | .[:3]"), ["sortby", "path1"]);
        assert_eq!(kinds("sort_by(.a)[0]"), ["sortby", "path1"]);
        assert_eq!(
            kinds("group_by(.a) | map(length)"),
            ["groupby", "collect[2]"]
        );
        // The key is the array of what the function makes, as jaq collects it.
        let Kind::Keyed(keyed) = plan("sort_by(.a, .b)").stages.remove(0).kind else {
            panic!("a keyed stage");
        };
        let key = keyed.key.expect("a key");
        assert_eq!(key.stages.len(), 1);
        assert_eq!(key.stages[0].rest, "[.a, .b]");
        // Not the ones with other arguments.
        assert_eq!(kinds("sort_by(.a; .b)"), ["opaque"]);
        assert_eq!(kinds("min(.a)"), ["opaque"]);
    }

    #[test]
    fn a_question_mark_after_a_term_is_a_try_and_after_an_expression_is_not() {
        assert_eq!(kinds("length?"), ["try[1]"]);
        assert_eq!(kinds("(.a | length)?"), ["try[1]"]);
        assert_eq!(kinds("first(.[])?"), ["try[1]"]);
        assert_eq!(kinds("keys?"), ["try[1]"]);
        // The question mark is the last term's, which an operator separates.
        assert_eq!(kinds(".[]? + .[]?"), ["compose2"]);
        assert_eq!(kinds("length + length?"), ["compose2"]);
        // What cannot be walked is not made to look as if it could.
        assert_eq!(kinds("add?"), ["opaque"]);
    }

    #[test]
    fn what_depends_on_the_kind_of_a_value_is_read() {
        assert_eq!(kinds("type"), ["type"]);
        assert_eq!(kinds(".. | numbers"), ["recurse", "of_type"]);
        assert_eq!(kinds("recurse | strings"), ["recurse", "of_type"]);
        for name in [
            "nulls",
            "booleans",
            "numbers",
            "strings",
            "arrays",
            "objects",
            "iterables",
            "scalars",
            "values",
        ] {
            assert_eq!(kinds(name), ["of_type"], "{name}");
        }
        assert_eq!(kinds("select(.a)"), ["select[1]"]);
        assert_eq!(kinds("select(.a | length)"), ["select[2]"]);
        assert_eq!(kinds("select(.a; .b)"), ["opaque"]);
        // The ones that do not depend on the kind alone are not these.
        assert_eq!(kinds("finites"), ["opaque"]);
        assert_eq!(kinds("recurse(.a)"), ["opaque"]);
    }

    #[test]
    fn an_expression_of_parts_that_are_walked_is_taken_apart() {
        // The parts are pipelines; what is between them is text for jaq.
        let composed = |src: &str| match plan(src).stages.remove(0).kind {
            Kind::Compose(c) => (
                c.pieces
                    .iter()
                    .map(|piece| match piece {
                        Piece::Text(text) => text.clone(),
                        Piece::Leaf(n) => format!("<{n}>"),
                    })
                    .collect::<String>(),
                c.leaves.len(),
            ),
            other => panic!("{src}: {other:?}"),
        };
        assert_eq!(composed("{a: length}"), ("{a: <0>}".to_owned(), 1));
        assert_eq!(
            composed("{n: length, first: .[0], \"k k\": .a.b}"),
            ("{n: <0>, first: <1>, \"k k\": <2>}".to_owned(), 3)
        );
        assert_eq!(
            composed("{a, \"b c\"}"),
            ("{a: <0>, \"b c\": <1>}".to_owned(), 2)
        );
        assert_eq!(composed("{(.k): length}"), ("{(<0>): <1>}".to_owned(), 2));
        assert_eq!(
            composed("{a: 1, b: \"x\"}"),
            ("{a: 1, b: \"x\"}".to_owned(), 0)
        );
        assert_eq!(composed("{}"), ("{}".to_owned(), 0));
        assert_eq!(composed("[]"), ("[]".to_owned(), 0));
        assert_eq!(composed(".[-1] - .[0]"), ("<0> - <1>".to_owned(), 2));
        assert_eq!(composed("length > 1"), ("<0> > 1".to_owned(), 1));
        assert_eq!(composed("-length"), ("-<0>".to_owned(), 1));
        assert_eq!(composed("1 - -1"), ("1 - -1".to_owned(), 0));
        assert_eq!(
            composed(".a // \"none\" and .b == null"),
            ("<0> // \"none\" and <1> == null".to_owned(), 2)
        );
        assert_eq!(
            composed("(.a | length) * 2 + length"),
            ("<0> * 2 + <1>".to_owned(), 2)
        );
        assert_eq!(composed("\"text\""), ("\"text\"".to_owned(), 0));
        assert_eq!(composed("null"), ("null".to_owned(), 0));
        // A pipe inside a value is a part of it.
        assert_eq!(composed("{a: .b | length}"), ("{a: <0>}".to_owned(), 1));
    }

    #[test]
    fn an_expression_that_is_more_than_terms_and_operators_is_left_alone() {
        for src in [
            "if .a then 1 else 2 end",
            "try .a",
            "{(.a): 1, $x}",
            "{$x}",
            "{@base64: 1}",
            "{\"a\\(.b)\": 1}",
            "{a: 1,}",
            "{a:}",
            ".a = 1",
            ".a |= 1",
            ".a += 1",
            "$x + 1",
            "@csv + 1",
            ". as $x | $x + 1",
            "reduce .[] as $x (0; . + $x)",
            "1 +",
            "+ 1",
            "* 2",
            ".a +* .b",
        ] {
            assert!(
                !kinds(src).iter().any(|k| k.starts_with("compose")),
                "{src}: {:?}",
                kinds(src)
            );
        }
    }

    #[test]
    fn a_call_that_is_walked_and_a_path_after_it_are_two_stages() {
        assert_eq!(kinds("first(.[])[0]"), ["limit1[1]", "path1"]);
        assert_eq!(kinds("map(.a)[0].b"), ["collect[2]", "path2"]);
        assert_eq!(kinds("map(.a).[1:3]"), ["collect[2]", "path1"]);
        assert_eq!(kinds("keys[0]"), ["keys", "path1"]);
        assert_eq!(
            kinds("map(.a)[] | length"),
            ["collect[2]", "path1", "length"]
        );
        // What the second stage is run with is the rest of the program from it.
        let p = plan("map(.a)[0].b | length");
        let rest: Vec<_> = p.stages.iter().map(|s| s.rest.as_str()).collect();
        assert_eq!(rest, ["map(.a)[0].b | length", ".[0].b | length", "length"]);
        // A call that is not walked is not split from its path, nor is a path that
        // is not made of constant steps.
        assert_eq!(kinds("add[0]"), ["opaque"]);
        assert_eq!(kinds("map(.a)[.n]"), ["opaque"]);
    }

    #[test]
    fn a_pipe_that_is_inside_something_does_not_split_it() {
        assert_eq!(
            kinds("if .a then .b | .c else .d end | .e"),
            ["opaque", "path1"]
        );
        assert_eq!(kinds("(.a | .b)"), ["each1"]);
        assert_eq!(kinds("[.a | .b]"), ["collect[2]"]);
        assert_eq!(kinds("map(.a | .b) | .c"), ["collect[3]", "path1"]);
        assert_eq!(kinds(".a | {b: (.c | .d)}"), ["path1", "compose1"]);
        assert_eq!(kinds(".a | \"x | y\""), ["path1", "compose0"]);
    }

    #[test]
    fn a_program_that_declares_something_is_not_taken_apart() {
        for src in [
            ".[] as $x | $x.a",
            "def f: .a; f | .b",
            "reduce .[] as $x (0; . + $x)",
            "label $out | .[] | ., break $out",
            ".a as [$x] | $x",
        ] {
            assert_eq!(kinds(src), ["opaque"], "{src}");
        }
    }

    #[test]
    fn each_stage_keeps_the_rest_of_the_program_from_where_it_starts() {
        let p = plan(".[] | select(.a > 1) |  .b");
        let rest: Vec<_> = p.stages.iter().map(|s| s.rest.as_str()).collect();
        assert_eq!(
            rest,
            [".[] | select(.a > 1) |  .b", "select(.a > 1) |  .b", ".b"]
        );
        // The first stage of a `map` is made up, and has none.
        let Kind::Collect(inner) = &plan("map(.a | .b)").stages[0].kind else {
            panic!("a collect");
        };
        let rest: Vec<_> = inner.stages.iter().map(|s| s.rest.as_str()).collect();
        assert_eq!(rest, ["", ".a | .b", ".b"]);
    }
}
