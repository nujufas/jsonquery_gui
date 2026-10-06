//! Nodes written out as text: a line for a tree row, indented JSON for the
//! text view, a file, or the clipboard.
//!
//! The JSON is what `serde_json::to_string_pretty` makes of the parsed value —
//! two spaces to a level, `"key": value`, `[]` and `{}` for what is empty,
//! numbers as `serde_json` keeps them — but made straight from the bytes, a
//! node at a time, so a document of any size can be written, and one of any
//! size can be written in part.

use std::io::{self, Write};

use super::Node;
use crate::tree::ValueKind;
use crate::view::PREVIEW_BYTES;

/// What a bounded rendering may take. The default is no bound at all.
#[derive(Clone, Copy, Debug)]
pub struct PrettyLimits {
    /// How many nodes it may show: scalars, arrays and objects count one each.
    pub nodes: usize,
    /// How many bytes of text it may make, give or take what the node that was
    /// being written when it ran out took.
    pub bytes: usize,
    /// How many bytes of a string it may show before cutting it short with `…`.
    pub string_bytes: usize,
}

impl Default for PrettyLimits {
    fn default() -> Self {
        Self {
            nodes: usize::MAX,
            bytes: usize::MAX,
            string_bytes: usize::MAX,
        }
    }
}

/// How a level of a document is indented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    /// This many spaces to a level.
    Spaces(u8),
    Tab,
    /// No white space at all: `{"a":[1,2]}`.
    Minified,
}

/// How a node is laid out as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub indent: Indent,
    /// Every character outside ASCII is written as `\uXXXX` (a surrogate pair
    /// above U+FFFF).
    pub ascii_only: bool,
}

impl Default for Style {
    /// Two spaces to a level, as `serde_json::to_string_pretty` makes it.
    fn default() -> Self {
        Self {
            indent: Indent::Spaces(2),
            ascii_only: false,
        }
    }
}

/// The text of `text` as a JSON string, with what is outside ASCII as escapes.
fn escape_ascii(text: &str) -> Vec<u8> {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c if !c.is_ascii() => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out.into_bytes()
}

/// What came of writing a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    /// Whether all of it was written, and none cut short.
    pub complete: bool,
    pub bytes: usize,
}

/// A scalar as a tree row shows it: what `serde_json` prints, a string in
/// quotes with its escapes as Rust writes them, and a string that is very long
/// cut short with `…`.
pub(super) fn scalar_preview(node: &Node<'_>) -> Option<String> {
    match node.kind() {
        ValueKind::Array | ValueKind::Object => None,
        ValueKind::Null => Some("null".to_string()),
        ValueKind::Bool => Some(String::from_utf8_lossy(node.raw()).into_owned()),
        ValueKind::Number => Some(number_text(node.raw())),
        ValueKind::String => {
            let (text, cut) = decode_prefix(node.raw(), PREVIEW_BYTES);
            Some(if cut {
                format!("{text:?}…")
            } else {
                format!("{text:?}")
            })
        }
    }
}

/// A number as `serde_json` (with `arbitrary_precision`) keeps it, which is as
/// it is written, except that an integer is its digits (`-0` is `0`) and an
/// exponent is `e`, a sign and digits.
pub(super) fn number_text(raw: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Number>(raw)
        .map(|n| n.to_string())
        .unwrap_or_else(|_| String::from_utf8_lossy(raw).into_owned())
}

/// Whether `raw` is an integer written the way `serde_json` prints it back.
pub(super) fn is_plain_integer(raw: &[u8]) -> bool {
    let digits = raw.strip_prefix(b"-").unwrap_or(raw);
    match digits {
        [b'0'] => raw.len() == 1,
        [b'1'..=b'9', rest @ ..] => rest.iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

/// The text of the string `quoted` (the bytes of it in the file, quotes and
/// all), up to about `max` bytes of the file's, and whether it was cut there.
pub(super) fn decode_prefix(quoted: &[u8], max: usize) -> (String, bool) {
    let inner = quoted
        .get(1..quoted.len().saturating_sub(1))
        .unwrap_or_default();
    if inner.len() <= max {
        let text = serde_json::from_slice(quoted)
            .unwrap_or_else(|_| String::from_utf8_lossy(inner).into_owned());
        return (text, false);
    }

    // Cut it where neither an escape nor a character is in two.
    let mut cut = max;
    let mut at = 0;
    while at < cut {
        if inner[at] == b'\\' {
            let len = match inner.get(at + 1) {
                Some(b'u') => {
                    // A leading surrogate takes its trailing one with it.
                    let high = inner
                        .get(at + 2..at + 6)
                        .and_then(|hex| std::str::from_utf8(hex).ok())
                        .and_then(|hex| u16::from_str_radix(hex, 16).ok())
                        .is_some_and(|n| (0xD800..=0xDBFF).contains(&n));
                    if high {
                        12
                    } else {
                        6
                    }
                }
                _ => 2,
            };
            if at + len > cut {
                cut = at;
                break;
            }
            at += len;
        } else {
            at += 1;
        }
    }
    while cut > 0 && inner[cut] & 0xC0 == 0x80 {
        cut -= 1;
    }
    let mut piece = Vec::with_capacity(cut + 2);
    piece.push(b'"');
    piece.extend_from_slice(&inner[..cut]);
    piece.push(b'"');
    let text = serde_json::from_slice(&piece)
        .unwrap_or_else(|_| String::from_utf8_lossy(&inner[..cut]).into_owned());
    (text, true)
}

impl<'a> Node<'a> {
    /// Write the node as indented JSON, up to `limits`.
    pub fn write_pretty<W: Write>(&self, out: &mut W, limits: PrettyLimits) -> io::Result<Written> {
        self.write_styled(out, Style::default(), limits)
    }

    /// Write the node as JSON laid out as `style` says, up to `limits`.
    pub fn write_styled<W: Write>(
        &self,
        out: &mut W,
        style: Style,
        limits: PrettyLimits,
    ) -> io::Result<Written> {
        let mut writer = Pretty {
            out,
            style,
            written: 0,
            limits,
            nodes_left: limits.nodes,
            complete: true,
        };
        let finished = writer.node(self, 0)?;
        Ok(Written {
            complete: finished && writer.complete,
            bytes: writer.written,
        })
    }

    /// The node as indented JSON text, up to `limits`, and whether it was all
    /// there.
    pub fn to_pretty_string(&self, limits: PrettyLimits) -> (String, bool) {
        let mut out = Vec::new();
        let written = self.write_pretty(&mut out, limits);
        let complete = written.map(|w| w.complete).unwrap_or(false);
        (String::from_utf8_lossy(&out).into_owned(), !complete)
    }
}

struct Pretty<'o, W: Write> {
    out: &'o mut W,
    style: Style,
    written: usize,
    limits: PrettyLimits,
    nodes_left: usize,
    complete: bool,
}

impl<W: Write> Pretty<'_, W> {
    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.written += bytes.len();
        self.out.write_all(bytes)
    }

    /// A line break and the indentation of `level` (nothing when minified).
    fn line(&mut self, level: usize) -> io::Result<()> {
        const SPACES: [u8; 64] = [b' '; 64];
        const TABS: [u8; 64] = [b'\t'; 64];
        let (fill, mut left) = match self.style.indent {
            Indent::Minified => return Ok(()),
            Indent::Spaces(n) => (&SPACES, level * usize::from(n)),
            Indent::Tab => (&TABS, level),
        };
        self.put(b"\n")?;
        while left > 0 {
            let n = left.min(fill.len());
            self.put(&fill[..n])?;
            left -= n;
        }
        Ok(())
    }

    /// A string as JSON text, as `style` has it: `serde_json`'s way unless only
    /// ASCII is wanted.
    fn quoted(&self, text: &str) -> Vec<u8> {
        if self.style.ascii_only {
            escape_ascii(text)
        } else {
            serde_json::to_vec(text).unwrap_or_default()
        }
    }

    /// Writes one node. Returns whether all of it was written; `false` means
    /// the limits ran out somewhere inside it and what was written is a prefix
    /// with the brackets closed (not necessarily JSON).
    fn node(&mut self, node: &Node<'_>, level: usize) -> io::Result<bool> {
        if self.nodes_left == 0 || self.written >= self.limits.bytes {
            self.put("…".as_bytes())?;
            return Ok(false);
        }
        self.nodes_left -= 1;
        match node.kind() {
            ValueKind::Null | ValueKind::Bool => {
                self.put(node.raw())?;
                Ok(true)
            }
            ValueKind::Number => {
                let raw = node.raw();
                if is_plain_integer(raw) {
                    self.put(raw)?;
                } else {
                    let text = number_text(raw);
                    self.put(text.as_bytes())?;
                }
                Ok(true)
            }
            ValueKind::String => {
                self.string(node)?;
                Ok(true)
            }
            ValueKind::Array | ValueKind::Object => self.container(node, level),
        }
    }

    fn string(&mut self, node: &Node<'_>) -> io::Result<()> {
        let raw = node.raw();
        let inner_len = raw.len().saturating_sub(2);
        if inner_len > self.limits.string_bytes {
            let (text, _) = decode_prefix(raw, self.limits.string_bytes);
            let mut quoted = self.quoted(&text);
            // `…` before the closing quote.
            quoted.pop();
            quoted.extend_from_slice("…\"".as_bytes());
            self.complete = false;
            return self.put(&quoted);
        }
        if memchr::memchr(b'\\', raw).is_none() && (!self.style.ascii_only || raw.is_ascii()) {
            // Nothing in it that would be written differently.
            return self.put(raw);
        }
        let text: String = serde_json::from_slice(raw).unwrap_or_default();
        let quoted = self.quoted(&text);
        self.put(&quoted)
    }

    fn container(&mut self, node: &Node<'_>, level: usize) -> io::Result<bool> {
        let object = node.kind() == ValueKind::Object;
        let (open, close) = if object { (b'{', b'}') } else { (b'[', b']') };
        let mut children = node.children().peekable();
        if children.peek().is_none() {
            self.put(if object { b"{}" } else { b"[]" })?;
            return Ok(true);
        }
        self.put(&[open])?;
        let mut complete = true;
        while let Some(child) = children.next() {
            self.line(level + 1)?;
            if let Some(key) = child.key {
                let key = key.to_string();
                let quoted = self.quoted(&key);
                self.put(&quoted)?;
                self.put(if self.style.indent == Indent::Minified {
                    b":"
                } else {
                    b": "
                })?;
            }
            if !self.node(&child.node, level + 1)? {
                complete = false;
                break;
            }
            if children.peek().is_some() {
                self.put(b",")?;
            }
        }
        self.line(level)?;
        self.put(&[close])?;
        Ok(complete)
    }
}
