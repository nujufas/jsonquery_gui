//! Searching a document that is still in its file for a key or a value that
//! contains a text, or matches an expression: what [`crate::tree::search`] does
//! on any [`ValueView`](crate::view::ValueView), but without making a string of
//! every key and every value on the way, which is most of what it costs: a key
//! or a string that has no escapes in it is looked at where it is in the file.
//! Which nodes match, and the order they are found in, are the same.

use memchr::memchr2;
use regex::Regex;

use super::{Key, Node};
use crate::tree::{NodePath, PathSegment, ValueKind, MAX_SEARCH_MATCHES};

/// What is looked for.
enum Matcher {
    /// A text, in lower case, which a key or a value contains whatever the case it is
    /// written in.
    Plain {
        needle: String,
    },
    Regex(Regex),
}

impl Matcher {
    fn new(query: &str, use_regex: bool) -> anyhow::Result<Self> {
        Ok(if use_regex {
            Matcher::Regex(Regex::new(query)?)
        } else {
            Matcher::Plain {
                needle: query.to_lowercase(),
            }
        })
    }

    fn text(&self, text: &str) -> bool {
        match self {
            Matcher::Plain { needle } => text.to_lowercase().contains(needle),
            Matcher::Regex(re) => re.is_match(text),
        }
    }

    /// The bytes of a key or a string as they are in the file, between its quotes,
    /// when they have no escapes in them (and are UTF-8, as the file was checked
    /// to be).
    fn raw(&self, raw: &[u8]) -> bool {
        match self {
            Matcher::Plain { needle } => {
                // For the ASCII there is, what lower-casing does is what it does
                // to a byte; anything else takes the long way, as the case of a
                // letter that is not ASCII is not a thing of bytes.
                if raw.is_ascii() && needle.is_ascii() {
                    contains_ignoring_case(raw, needle.as_bytes())
                } else {
                    std::str::from_utf8(raw).is_ok_and(|text| self.text(text))
                }
            }
            Matcher::Regex(re) => std::str::from_utf8(raw).is_ok_and(|text| re.is_match(text)),
        }
    }

    fn key(&self, key: Key<'_>) -> bool {
        if memchr::memchr(b'\\', key.raw).is_none() {
            self.raw(key.raw)
        } else {
            key.decoded().is_some_and(|text| self.text(&text))
        }
    }

    /// Whether a scalar, as the generic search would render it, matches.
    fn scalar(&self, node: &Node<'_>) -> bool {
        let raw = node.raw();
        match node.kind() {
            ValueKind::String => {
                let inner = raw.get(1..raw.len().saturating_sub(1)).unwrap_or_default();
                if memchr::memchr(b'\\', inner).is_none() {
                    self.raw(inner)
                } else {
                    node.string().is_some_and(|text| self.text(&text))
                }
            }
            // Null and booleans are written as they are shown.
            ValueKind::Null | ValueKind::Bool => self.raw(raw),
            ValueKind::Number => {
                if super::write::is_plain_integer(raw) {
                    self.raw(raw)
                } else {
                    self.text(&super::write::number_text(raw))
                }
            }
            ValueKind::Array | ValueKind::Object => false,
        }
    }
}

/// Whether `haystack` has `needle` (lower case ASCII) in it, in any case.
fn contains_ignoring_case(haystack: &[u8], needle: &[u8]) -> bool {
    let Some(&first) = needle.first() else {
        return true;
    };
    let upper = first.to_ascii_uppercase();
    let mut from = 0;
    while let Some(found) = memchr2(first, upper, &haystack[from..]) {
        let at = from + found;
        match haystack.get(at..at + needle.len()) {
            Some(candidate) if candidate.eq_ignore_ascii_case(needle) => return true,
            Some(_) => from = at + 1,
            None => return false,
        }
    }
    false
}

/// One step on the way down, which is only made into a path when something is
/// found there.
#[derive(Clone, Copy)]
enum Step<'a> {
    Index(usize),
    Key(Key<'a>),
}

/// Search `root` for every node whose key or scalar value contains `query`
/// (whatever the case), or — if `use_regex` — matches it as a regular
/// expression, depth-first pre-order, capped at the same number of hits as
/// [`crate::tree::search`]. The paths are the same ones it finds.
pub fn search(root: Node<'_>, query: &str, use_regex: bool) -> anyhow::Result<Vec<NodePath>> {
    let matcher = Matcher::new(query, use_regex)?;
    let mut found = Vec::new();
    let mut path = Vec::new();
    walk(root, None, &matcher, &mut path, &mut found);
    Ok(found)
}

fn walk<'a>(
    node: Node<'a>,
    key: Option<Key<'a>>,
    matcher: &Matcher,
    path: &mut Vec<Step<'a>>,
    found: &mut Vec<NodePath>,
) {
    if found.len() >= MAX_SEARCH_MATCHES {
        return;
    }
    if key.is_some_and(|k| matcher.key(k)) || matcher.scalar(&node) {
        found.push(path.iter().map(segment).collect());
    }
    for child in node.children() {
        if found.len() >= MAX_SEARCH_MATCHES {
            break;
        }
        path.push(match child.key {
            Some(key) => Step::Key(key),
            None => Step::Index(child.index),
        });
        walk(child.node, child.key, matcher, path, found);
        path.pop();
    }
}

fn segment(step: &Step<'_>) -> PathSegment {
    match step {
        Step::Index(i) => PathSegment::Index(*i),
        Step::Key(key) => PathSegment::Key(key.to_string()),
    }
}
