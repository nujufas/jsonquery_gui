//! What a query's mistakes are called.
//!
//! The parsers report a mistake as data: jaq as what it expected and the text left at that point,
//! the JSONPath crate as a pest "failed to parse rule" report, the JMESPath crate as a message with
//! a caret excerpt and a position counted from zero. Shown as they came, the query box said
//! `[(File { code: ".a |", path: () }, Parse([(Term, "")]))]`. Here each is a sentence with a
//! place: what was expected, what was found, and where.
//!
//! Every function returns the text for the inside of `QueryError::Parse` or `QueryError::Engine`,
//! whose own prefix ("query syntax error: ") stays as it was.

use jaq_core::compile::Undefined;
use jaq_core::load::lex::Expect as LexExpect;
use jaq_core::load::parse::Expect as ParseExpect;
use jaq_core::load::{Error as LoadError, File};

use crate::highlight::{skip_comment, skip_string};
use crate::Kind;

/// What the jq loader says of a program it cannot read: the program and what is wrong with it.
type Unreadable<'a> = (File<&'a str, ()>, LoadError<&'a str>);
/// What the jq compiler says of a program that names what is not there.
type Missing<'a> = (File<&'a str, ()>, Vec<(&'a str, Undefined)>);

// ----- places --------------------------------------------------------------------------------

/// "column 9", or "line 2, column 3" when the query has more than one line.
fn place(code: &str, offset: usize) -> String {
    let before = code.get(..offset.min(code.len())).unwrap_or(code);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    position(code, line, column)
}

fn position(code: &str, line: usize, column: usize) -> String {
    if code.contains('\n') {
        format!("line {line}, column {column}")
    } else {
        format!("column {column}")
    }
}

/// Where `part` sits in `code`, if it is a piece of it (jaq hands back slices of the text).
fn offset_of(code: &str, part: &str) -> Option<usize> {
    let start = code.as_ptr() as usize;
    let at = part.as_ptr() as usize;
    (at >= start && at + part.len() <= start + code.len()).then(|| at - start)
}

/// A piece of the query as it is shown in a message: in backticks, cut if it is long.
fn quoted(text: &str) -> String {
    let mut shown: String = text.chars().take(24).collect();
    if shown.chars().count() < text.chars().count() {
        shown.push('…');
    }
    format!("`{shown}`")
}

// ----- jq: syntax -----------------------------------------------------------------------------

/// A jq program that could not be read, as one sentence. Only the first mistake is told in full:
/// the ones after it are mostly its echoes.
pub(crate) fn syntax(code: &str, errors: &[Unreadable<'_>]) -> String {
    let mut messages = errors.iter().flat_map(|(_, error)| match error {
        LoadError::Io(list) => list
            .iter()
            .map(|(_, message)| message.clone())
            .collect::<Vec<_>>(),
        LoadError::Lex(list) => list
            .iter()
            .map(|(expect, rest)| lexing(code, expect, rest))
            .collect(),
        LoadError::Parse(list) => list
            .iter()
            .map(|(expect, found)| parsing(code, expect, found))
            .collect(),
    });
    let first = messages
        .next()
        .unwrap_or_else(|| "the query could not be read".to_owned());
    match messages.count() {
        0 => first,
        1 => format!("{first} (and one more problem after it)"),
        more => format!("{first} (and {more} more problems after it)"),
    }
}

fn lexing(code: &str, expect: &LexExpect<&str>, rest: &str) -> String {
    let at = offset_of(code, rest).unwrap_or(code.len());
    let here = place(code, at);
    match expect {
        LexExpect::Delim(open) => unclosed(code, open),
        LexExpect::Token => {
            let found = rest
                .chars()
                .next()
                .map_or_else(|| "text".to_owned(), |c| quoted(&c.to_string()));
            format!("unexpected {found} at {here}")
        }
        LexExpect::Digit => format!("expected a digit in the number at {here}"),
        LexExpect::Ident => format!("expected a name after `$` or `@` at {here}"),
        LexExpect::Escape => format!(
            "this is not an escape a string can have, at {here}: \
             use `\\n`, `\\t`, `\\\"`, `\\\\`, `\\uXXXX` or `\\(…)`"
        ),
        LexExpect::Unicode => format!("expected four hexadecimal digits after `\\u` at {here}"),
        _ => format!("unexpected text at {here}"),
    }
}

/// "the `[` at column 7 is never closed: add a `]`".
fn unclosed(code: &str, open: &str) -> String {
    let close = match open {
        "(" => ")",
        "[" => "]",
        "{" => "}",
        other => other,
    };
    let opener = open.chars().next().unwrap_or('(');
    let at = last_unclosed(code, opener).map(|offset| place(code, offset));
    match (open, at) {
        ("\"", Some(at)) => {
            format!("the string that starts at {at} is never closed: add a closing `\"`")
        }
        ("\"", None) => "a string is never closed: add a closing `\"`".to_owned(),
        (_, Some(at)) => format!("the `{open}` at {at} is never closed: add a `{close}`"),
        (_, None) => format!("a `{open}` is never closed: add a `{close}`"),
    }
}

/// The last opener of kind `wanted` that nothing closes, as a byte offset. Strings and comments
/// are skipped, so a bracket in them does not count.
fn last_unclosed(code: &str, wanted: char) -> Option<usize> {
    let b = code.as_bytes();
    let mut open: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => open.push(i),
            b')' | b']' | b'}' => {
                open.pop();
            }
            b'"' => {
                let end = skip_string(Kind::Jq, b, i);
                // a string that runs to the end without its closing quote is the unclosed one
                let closed = end > i + 1 && b[end - 1] == b'"' && b.get(end - 2) != Some(&b'\\');
                if !closed {
                    open.push(i);
                }
                i = end;
                continue;
            }
            b'#' => {
                i = skip_comment(b, i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    open.into_iter().rev().find(|&at| b[at] as char == wanted)
}

fn parsing(code: &str, expect: &ParseExpect<&str>, found: &str) -> String {
    let wanted = match expect {
        ParseExpect::Just(token) => format!("`{token}`"),
        ParseExpect::Var => "a variable such as `$x`".to_owned(),
        ParseExpect::Pattern => "a pattern such as `$x`".to_owned(),
        ParseExpect::ElseOrEnd => "`else`, `elif` or `end`".to_owned(),
        ParseExpect::CommaOrRBrack => "`,` or `]`".to_owned(),
        ParseExpect::CommaOrRBrace => "`,` or `}`".to_owned(),
        ParseExpect::SemicolonOrRParen => "`;` or `)`".to_owned(),
        ParseExpect::Term => "an expression".to_owned(),
        ParseExpect::Key => "a key".to_owned(),
        ParseExpect::Ident => "a name".to_owned(),
        ParseExpect::Arg => "an argument".to_owned(),
        ParseExpect::Str => "a string".to_owned(),
        ParseExpect::Nothing => "nothing more".to_owned(),
        _ => "something else".to_owned(),
    };
    if found.is_empty() {
        format!(
            "expected {wanted}, found the end of the query at {}",
            place(code, code.len())
        )
    } else {
        let at = offset_of(code, found).unwrap_or(code.len());
        format!(
            "expected {wanted}, found {} at {}",
            quoted(found),
            place(code, at)
        )
    }
}

// ----- jq: what is not defined --------------------------------------------------------------

/// A jq program that names something that is not there.
pub(crate) fn undefined(errors: &[Missing<'_>]) -> String {
    let mut told = errors
        .iter()
        .flat_map(|(_, list)| list.iter())
        .map(|(name, kind)| match kind {
            Undefined::Filter(arity) => {
                if let Some(why) = unavailable(name) {
                    return format!("`{name}` is not available: {why}");
                }
                let shown = if *arity == 0 {
                    (*name).to_owned()
                } else {
                    format!("{name}/{arity}")
                };
                match arity {
                    0 => format!("`{shown}` is not defined"),
                    1 => format!(
                        "`{shown}` is not defined (no function of that name takes 1 argument)"
                    ),
                    n => format!(
                        "`{shown}` is not defined (no function of that name takes {n} arguments)"
                    ),
                }
            }
            Undefined::Var => match unavailable(name) {
                Some(why) => format!("`{name}` is not available: {why}"),
                None => format!("the variable `{name}` is not defined"),
            },
            Undefined::Label => format!("the label `{name}` is not defined"),
            Undefined::Mod => format!("the module `{name}` is not defined"),
            _ => format!("`{name}` is not defined"),
        });
    let first = told
        .next()
        .unwrap_or_else(|| "the query uses something that is not defined".to_owned());
    match told.count() {
        0 => first,
        1 => format!("{first} (and one more thing that is not defined)"),
        more => format!("{first} (and {more} more things that are not defined)"),
    }
}

/// jq has these; a query over one open document cannot.
fn unavailable(name: &str) -> Option<&'static str> {
    match name {
        "input" | "inputs" | "input_filename" | "input_line_number" => Some(
            "a query runs over the one document that is open, and there is no other input to read",
        ),
        "$ENV" | "$__prog_args" | "$__loc__" => Some("it has nothing to read here"),
        _ => None,
    }
}

// ----- JSONPath and JMESPath ---------------------------------------------------------------

/// The JSONPath crate's report ("Failed to parse rule:  --> 1:3 ... = expected selector") as a
/// sentence with the place. A message in another shape is left as it is.
pub(crate) fn jsonpath(query: &str, message: &str) -> String {
    let Some(report) = message.strip_prefix("Failed to parse rule:") else {
        return message.to_owned();
    };
    let at = report
        .split("-->")
        .nth(1)
        .and_then(|after| after.split_whitespace().next())
        .and_then(|line_column| line_column.split_once(':'))
        .and_then(|(line, column)| {
            Some((line.parse::<usize>().ok()?, column.parse::<usize>().ok()?))
        });
    let Some((line, column)) = at else {
        return message.to_owned();
    };
    let expected = report
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("= "))
        .map_or("something else", str::trim);
    let what = match expected {
        "expected jp_query" => "expected `$`: a JSONPath starts at the root".to_owned(),
        "expected selector" => {
            "expected a selector: a name, an index, `*`, a slice or a filter".to_owned()
        }
        "expected comparable" => {
            "expected something to compare with: a number, a string, a path or a function"
                .to_owned()
        }
        "expected atom_expr" => "expected a condition".to_owned(),
        other => other.replace('_', " "),
    };
    format!("{what}, at {}", position(query, line, column))
}

/// The JMESPath crate's message ("Parse error: Unexpected nud token -- found Eof (line 0, column
/// 2)" and a caret excerpt) as a sentence, with the position counted from one. A message in
/// another shape is left as it is.
pub(crate) fn jmespath(query: &str, message: &str) -> String {
    let first = message.lines().next().unwrap_or(message);
    let first = first.strip_prefix("Parse error: ").unwrap_or(first);
    let Some((text, tail)) = first.rsplit_once(" (line ") else {
        return first.to_owned();
    };
    let at = tail
        .strip_suffix(')')
        .and_then(|t| t.split_once(", column "))
        .and_then(|(line, column)| {
            Some((line.parse::<usize>().ok()?, column.parse::<usize>().ok()?))
        });
    let Some((line, column)) = at else {
        return first.to_owned();
    };
    let mut text = text
        .replace(
            "Unexpected nud token",
            "Unexpected token (an expression was expected)",
        )
        .replace(
            "Unexpected led token",
            "Unexpected token after an expression",
        );
    for (name, shown) in [
        ("found Eof", "found the end of the query"),
        ("found Dot", "found `.`"),
        ("found Comma", "found `,`"),
        ("found Colon", "found `:`"),
        ("found Pipe", "found `|`"),
        ("found Lbracket", "found `[`"),
        ("found Rbracket", "found `]`"),
        ("found Lbrace", "found `{`"),
        ("found Rbrace", "found `}`"),
        ("found Lparen", "found `(`"),
        ("found Rparen", "found `)`"),
    ] {
        text = text.replace(name, shown);
    }
    format!("{text}, at {}", position(query, line + 1, column + 1))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use serde_json::json;

    use crate::Kind;

    /// What the query box says for `query` in dialect `kind`: the error, whole, as the app shows it.
    fn said(kind: Kind, query: &str) -> String {
        let input = json!({"users": [{"name": "Ann"}]});
        let cancelled = AtomicBool::new(false);
        kind.engine()
            .run(&input, query, &cancelled, &mut |_| {})
            .expect_err("it is a mistake")
            .to_string()
    }

    #[test]
    fn a_bracket_that_is_never_closed_is_found_and_named() {
        assert_eq!(
            said(Kind::Jq, ".users[ | .name"),
            "query syntax error: the `[` at column 7 is never closed: add a `]`"
        );
        assert_eq!(
            said(Kind::Jq, ".users | map(.name"),
            "query syntax error: the `(` at column 13 is never closed: add a `)`"
        );
        assert_eq!(
            said(Kind::Jq, "{a: 1"),
            "query syntax error: the `{` at column 1 is never closed: add a `}`"
        );
        // a bracket inside a string is not one; the innermost unclosed is told first, and the
        // other is said to be there
        assert_eq!(
            said(Kind::Jq, ".a | map(.b[\"]\"] | (.c"),
            "query syntax error: the `(` at column 20 is never closed: add a `)` (and one more problem after it)"
        );
    }

    #[test]
    fn a_string_that_is_never_closed_says_where_it_starts() {
        assert_eq!(
            said(Kind::Jq, ".users[] | \"abc"),
            "query syntax error: the string that starts at column 12 is never closed: add a closing `\"`"
        );
    }

    #[test]
    fn what_cannot_be_read_says_what_was_there() {
        assert_eq!(
            said(Kind::Jq, ".a & .b"),
            "query syntax error: unexpected `&` at column 4"
        );
        assert_eq!(
            said(Kind::Jq, "{a: 1,, b}"),
            "query syntax error: expected a key, found `,` at column 7"
        );
    }

    #[test]
    fn a_number_a_name_or_an_escape_that_is_wrong_says_what_was_expected() {
        assert_eq!(
            said(Kind::Jq, "1e"),
            "query syntax error: expected a digit in the number at column 3"
        );
        assert_eq!(
            said(Kind::Jq, ". as $"),
            "query syntax error: expected a name after `$` or `@` at column 7"
        );
        assert_eq!(
            said(Kind::Jq, "\"\\u12\""),
            "query syntax error: expected four hexadecimal digits after `\\u` at column 6"
        );
        assert_eq!(
            said(Kind::Jq, "\"a\\qb\""),
            "query syntax error: this is not an escape a string can have, at column 4: \
             use `\\n`, `\\t`, `\\\"`, `\\\\`, `\\uXXXX` or `\\(…)`"
        );
    }

    #[test]
    fn what_is_missing_at_the_end_says_so() {
        assert_eq!(
            said(Kind::Jq, "if . then 1"),
            "query syntax error: expected `else`, `elif` or `end`, found the end of the query at column 12"
        );
        assert_eq!(
            said(Kind::Jq, ".users[0] as"),
            "query syntax error: expected a pattern such as `$x`, found the end of the query at column 13"
        );
    }

    #[test]
    fn a_place_in_a_longer_query_names_the_line() {
        assert_eq!(
            said(Kind::Jq, ".users\n| map(.name\n| x"),
            "query syntax error: the `(` at line 2, column 6 is never closed: add a `)`"
        );
    }

    #[test]
    fn what_is_not_defined_is_named_without_the_debug_dump() {
        assert_eq!(
            said(Kind::Jq, "foo(1)"),
            "query error: `foo/1` is not defined (no function of that name takes 1 argument)"
        );
        assert_eq!(said(Kind::Jq, "bar"), "query error: `bar` is not defined");
        assert_eq!(
            said(Kind::Jq, "$nope"),
            "query error: the variable `$nope` is not defined"
        );
        assert!(said(Kind::Jq, "input").contains("no other input to read"));
        assert!(said(Kind::Jq, "$ENV").contains("nothing to read here"));
    }

    #[test]
    fn jsonpath_says_what_it_expected_and_where() {
        assert_eq!(
            said(Kind::JsonPath, "$["),
            "query syntax error: expected a selector: a name, an index, `*`, a slice or a filter, at column 3"
        );
        assert_eq!(
            said(Kind::JsonPath, "users"),
            "query syntax error: expected `$`: a JSONPath starts at the root, at column 1"
        );
        assert!(said(Kind::JsonPath, "$.users[?(@.age >)]")
            .contains("expected something to compare with"));
        // no stack, no excerpt, no rule names
        for query in ["$[", "users", "$..[?", "$.a[?(@.b >)]"] {
            let message = said(Kind::JsonPath, query);
            assert!(
                !message.contains('\n') && !message.contains("-->") && !message.contains("---"),
                "{message}"
            );
        }
    }

    #[test]
    fn jmespath_says_what_it_found_where_it_did_not_belong() {
        assert_eq!(
            said(Kind::JmesPath, "[?"),
            "query syntax error: Unexpected token (an expression was expected) -- found the end of the query, at column 3"
        );
        assert_eq!(
            said(Kind::JmesPath, "users[*."),
            "query syntax error: Expected ']' for wildcard index -- found `.`, at column 8"
        );
        for query in ["[?", "users[?age >", "foo(bar"] {
            let message = said(Kind::JmesPath, query);
            assert!(
                !message.contains('\n') && !message.contains('^') && !message.contains("nud"),
                "{message}"
            );
        }
    }

    #[test]
    fn a_message_in_a_shape_not_expected_is_left_alone() {
        assert_eq!(
            super::jsonpath("$", "something else entirely"),
            "something else entirely"
        );
        assert_eq!(
            super::jmespath("x", "a different message"),
            "a different message"
        );
        assert_eq!(
            super::jmespath("x", "Parse error: no position here"),
            "no position here"
        );
    }
}
