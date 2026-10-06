//! The scan that builds a lazy document's index: one pass over the bytes that
//! checks them as strictly as `serde_json` would (so that a node of a document
//! that opened can always be parsed later), and that notes where the children
//! of the big containers are.
//!
//! The grammar, the order in which things are checked and the words of the
//! errors follow `serde_json`'s own parser, as `Deserializer::from_slice` runs
//! it over a stream of values: what it accepts is accepted here, and what it
//! refuses is refused with the same words and, as far as the tests could tell,
//! the same line and column. Only the work is different: nothing is built.

use std::fmt;

use super::{Checkpoint, Container, Index, IndexConfig, Top, STREAM_ROOT};
use crate::tree::ValueKind;

/// How deeply containers may nest: `serde_json` refuses the 128th level.
pub(super) const MAX_DEPTH: usize = 127;

/// Why a document is not JSON, in `serde_json`'s words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    EofWhileParsingList,
    EofWhileParsingObject,
    EofWhileParsingString,
    EofWhileParsingValue,
    ExpectedColon,
    ExpectedListCommaOrEnd,
    ExpectedObjectCommaOrEnd,
    ExpectedSomeIdent,
    ExpectedSomeValue,
    InvalidEscape,
    InvalidNumber,
    InvalidUnicodeCodePoint,
    ControlCharacterWhileParsingString,
    KeyMustBeAString,
    LoneLeadingSurrogateInHexEscape,
    TrailingComma,
    TrailingCharacters,
    UnexpectedEndOfHexEscape,
    RecursionLimitExceeded,
    /// Not a fault of the text: the file was cut short while it was being read.
    FileChanged,
}

impl ErrorKind {
    fn message(self) -> &'static str {
        match self {
            ErrorKind::EofWhileParsingList => "EOF while parsing a list",
            ErrorKind::EofWhileParsingObject => "EOF while parsing an object",
            ErrorKind::EofWhileParsingString => "EOF while parsing a string",
            ErrorKind::EofWhileParsingValue => "EOF while parsing a value",
            ErrorKind::ExpectedColon => "expected `:`",
            ErrorKind::ExpectedListCommaOrEnd => "expected `,` or `]`",
            ErrorKind::ExpectedObjectCommaOrEnd => "expected `,` or `}`",
            ErrorKind::ExpectedSomeIdent => "expected ident",
            ErrorKind::ExpectedSomeValue => "expected value",
            ErrorKind::InvalidEscape => "invalid escape",
            ErrorKind::InvalidNumber => "invalid number",
            ErrorKind::InvalidUnicodeCodePoint => "invalid unicode code point",
            ErrorKind::ControlCharacterWhileParsingString => {
                "control character (\\u0000-\\u001F) found while parsing a string"
            }
            ErrorKind::KeyMustBeAString => "key must be a string",
            ErrorKind::LoneLeadingSurrogateInHexEscape => "lone leading surrogate in hex escape",
            ErrorKind::TrailingComma => "trailing comma",
            ErrorKind::TrailingCharacters => "trailing characters",
            ErrorKind::UnexpectedEndOfHexEscape => "unexpected end of hex escape",
            ErrorKind::RecursionLimitExceeded => "recursion limit exceeded",
            ErrorKind::FileChanged => "the file was changed while it was being read",
        }
    }
}

/// A document that is not JSON: what is wrong, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanError {
    pub kind: ErrorKind,
    pub line: usize,
    pub column: usize,
}

impl ScanError {
    /// The file was cut short while it was being read: no line or column of it
    /// is to blame.
    pub(super) fn file_changed() -> Self {
        Self {
            kind: ErrorKind::FileChanged,
            line: 0,
            column: 0,
        }
    }
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == ErrorKind::FileChanged {
            return f.write_str(self.kind.message());
        }
        write!(
            f,
            "{} at line {} column {}",
            self.kind.message(),
            self.line,
            self.column
        )
    }
}

impl std::error::Error for ScanError {}

/// Check `bytes` as a stream of JSON values and index them, and find the objects
/// that have a key twice. The two are passes of their own; for bytes of any size
/// they are made at the same time, by two threads, so that the second costs the
/// time of neither.
pub(super) fn scan(bytes: &[u8], cfg: IndexConfig) -> Result<Index, ScanError> {
    /// Below this the thread costs more than it saves.
    const TWO_THREADS_FROM: usize = 1 << 20;
    if bytes.len() < TWO_THREADS_FROM {
        let mut index = scan_index(bytes, cfg)?;
        index.repeated = super::repeated::find(bytes);
        return Ok(index);
    }
    std::thread::scope(|scope| {
        let finder = scope.spawn(|| super::repeated::find(bytes));
        let index = scan_index(bytes, cfg);
        let repeated = finder.join().unwrap_or_default();
        index.map(|mut index| {
            index.repeated = repeated;
            index
        })
    })
}

fn scan_index(bytes: &[u8], cfg: IndexConfig) -> Result<Index, ScanError> {
    let mut scanner = Scanner {
        b: bytes,
        i: 0,
        cfg,
        stack: Vec::with_capacity(32),
        cp_stack: Vec::new(),
        pool: Vec::new(),
        containers: Vec::new(),
    };

    // The top level is a container of its own that has no brackets: the values
    // of the stream are its children.
    let mut top = Frame::new(false, STREAM_ROOT, 0);
    let mut top_cps: Vec<Checkpoint> = Vec::new();
    let mut first = None;
    loop {
        scanner.skip_ws();
        if scanner.i >= bytes.len() {
            break;
        }
        let start = scanner.i;
        top.begin_child(start, &cfg, &mut top_cps);
        let kind = scanner.value()?;
        if !matches!(
            kind,
            ValueKind::Array | ValueKind::Object | ValueKind::String
        ) {
            scanner.end_of_value()?;
        }
        if first.is_none() {
            first = Some((start, scanner.i, kind));
        }
    }

    let top = match first {
        Some((start, end, kind)) if top.count == 1 => Top::Single { start, end, kind },
        _ => {
            let cp_start = scanner.pool.len();
            scanner.pool.extend(top_cps.iter().copied());
            scanner.containers.push(Container {
                start: STREAM_ROOT,
                end: bytes.len(),
                count: top.count,
                cp_start,
                cp_len: top_cps.len(),
            });
            Top::Stream
        }
    };

    let mut containers = scanner.containers;
    containers.sort_unstable_by_key(|c| c.start);
    containers.shrink_to_fit();
    let mut pool = scanner.pool;
    pool.shrink_to_fit();
    Ok(Index {
        containers,
        pool,
        repeated: Vec::new(),
        top,
    })
}

/// A container that is being scanned.
struct Frame {
    object: bool,
    /// Where its bracket is.
    start: usize,
    /// How many children have begun.
    count: usize,
    /// The last child that has a checkpoint, and where it is.
    last_child: usize,
    last_offset: usize,
    /// Where its checkpoints begin in `Scanner::cp_stack`.
    cp_base: usize,
}

impl Frame {
    fn new(object: bool, start: usize, cp_base: usize) -> Self {
        Self {
            object,
            start,
            count: 0,
            last_child: 0,
            last_offset: 0,
            cp_base,
        }
    }

    /// A child begins at `offset` (the value of an array's, the key of an
    /// object's): count it, and note a checkpoint if enough children, or enough
    /// bytes, have gone by since the last.
    #[inline]
    fn begin_child(&mut self, offset: usize, cfg: &IndexConfig, cps: &mut Vec<Checkpoint>) {
        let child = self.count;
        self.count += 1;
        if child == 0 {
            self.last_child = 0;
            self.last_offset = offset;
        } else if child - self.last_child >= cfg.cp_children
            || offset - self.last_offset >= cfg.cp_bytes
        {
            cps.push(Checkpoint { child, offset });
            self.last_child = child;
            self.last_offset = offset;
        }
    }
}

struct Scanner<'a> {
    b: &'a [u8],
    i: usize,
    cfg: IndexConfig,
    stack: Vec<Frame>,
    /// The checkpoints of the containers being scanned, the innermost last.
    cp_stack: Vec<Checkpoint>,
    /// The checkpoints of the containers that were kept.
    pool: Vec<Checkpoint>,
    containers: Vec<Container>,
}

impl Scanner<'_> {
    /// An error caused by what is at `pos - 1`, or by the end of the input when
    /// `pos` is its length: `serde_json` counts the column up to and including
    /// the byte it reports.
    #[cold]
    fn error_at(&self, kind: ErrorKind, pos: usize) -> ScanError {
        let pos = pos.min(self.b.len());
        let before = &self.b[..pos];
        let line_start = before
            .iter()
            .rposition(|&c| c == b'\n')
            .map_or(0, |at| at + 1);
        let line = 1 + before[..line_start].iter().filter(|&&c| c == b'\n').count();
        ScanError {
            kind,
            line,
            column: pos - line_start,
        }
    }

    /// An error caused by the byte at `self.i`, which was looked at and not yet
    /// taken (or by the end of the input).
    #[cold]
    fn peek_error(&self, kind: ErrorKind) -> ScanError {
        self.error_at(kind, self.i + 1)
    }

    /// An error caused by the byte just taken, at `self.i - 1`.
    #[cold]
    fn taken_error(&self, kind: ErrorKind) -> ScanError {
        self.error_at(kind, self.i)
    }

    #[inline]
    fn skip_ws(&mut self) {
        while let Some(&c) = self.b.get(self.i) {
            if matches!(c, b' ' | b'\n' | b'\t' | b'\r') {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    /// Scan one value that starts at `self.i`, a byte that is not whitespace,
    /// and leave `self.i` just past it. Containers are not recursed into: a
    /// stack of frames follows where in them the scan is.
    fn value(&mut self) -> Result<ValueKind, ScanError> {
        let mut last;
        'value: loop {
            let Some(&c) = self.b.get(self.i) else {
                return Err(self.peek_error(ErrorKind::EofWhileParsingValue));
            };
            match c {
                b'"' => {
                    self.i += 1;
                    self.string()?;
                    last = ValueKind::String;
                }
                b'[' => {
                    self.push_frame(false)?;
                    self.skip_ws();
                    match self.b.get(self.i) {
                        None => return Err(self.peek_error(ErrorKind::EofWhileParsingList)),
                        Some(b']') => {
                            self.i += 1;
                            self.pop_frame();
                            last = ValueKind::Array;
                        }
                        Some(_) => {
                            self.begin_child();
                            continue 'value;
                        }
                    }
                }
                b'{' => {
                    self.push_frame(true)?;
                    self.skip_ws();
                    match self.b.get(self.i) {
                        None => return Err(self.peek_error(ErrorKind::EofWhileParsingObject)),
                        Some(b'}') => {
                            self.i += 1;
                            self.pop_frame();
                            last = ValueKind::Object;
                        }
                        Some(b'"') => {
                            self.begin_child();
                            self.key()?;
                            continue 'value;
                        }
                        Some(_) => return Err(self.peek_error(ErrorKind::KeyMustBeAString)),
                    }
                }
                b'n' => {
                    self.i += 1;
                    self.ident(b"ull")?;
                    last = ValueKind::Null;
                }
                b't' => {
                    self.i += 1;
                    self.ident(b"rue")?;
                    last = ValueKind::Bool;
                }
                b'f' => {
                    self.i += 1;
                    self.ident(b"alse")?;
                    last = ValueKind::Bool;
                }
                b'-' | b'0'..=b'9' => {
                    self.number()?;
                    last = ValueKind::Number;
                }
                _ => return Err(self.peek_error(ErrorKind::ExpectedSomeValue)),
            }

            // A value is complete. Whatever contains it says what comes next,
            // and a container that ends is a value that is complete in turn.
            loop {
                let Some(frame) = self.stack.last() else {
                    return Ok(last);
                };
                let object = frame.object;
                self.skip_ws();
                let Some(&c) = self.b.get(self.i) else {
                    return Err(self.peek_error(if object {
                        ErrorKind::EofWhileParsingObject
                    } else {
                        ErrorKind::EofWhileParsingList
                    }));
                };
                if c == if object { b'}' } else { b']' } {
                    self.i += 1;
                    self.pop_frame();
                    last = if object {
                        ValueKind::Object
                    } else {
                        ValueKind::Array
                    };
                    continue;
                }
                if c != b',' {
                    return Err(self.peek_error(if object {
                        ErrorKind::ExpectedObjectCommaOrEnd
                    } else {
                        ErrorKind::ExpectedListCommaOrEnd
                    }));
                }
                self.i += 1;
                self.skip_ws();
                let Some(&next) = self.b.get(self.i) else {
                    return Err(self.peek_error(ErrorKind::EofWhileParsingValue));
                };
                if object {
                    match next {
                        b'"' => {
                            self.begin_child();
                            self.key()?;
                        }
                        b'}' => return Err(self.peek_error(ErrorKind::TrailingComma)),
                        _ => return Err(self.peek_error(ErrorKind::KeyMustBeAString)),
                    }
                } else if next == b']' {
                    return Err(self.peek_error(ErrorKind::TrailingComma));
                } else {
                    self.begin_child();
                }
                continue 'value;
            }
        }
    }

    /// The key of an object member, from its opening quote, and the colon after
    /// it; `self.i` is left at the start of the value.
    fn key(&mut self) -> Result<(), ScanError> {
        self.i += 1;
        self.string()?;
        self.skip_ws();
        match self.b.get(self.i) {
            Some(b':') => self.i += 1,
            Some(_) => return Err(self.peek_error(ErrorKind::ExpectedColon)),
            None => return Err(self.peek_error(ErrorKind::EofWhileParsingObject)),
        }
        self.skip_ws();
        if self.i >= self.b.len() {
            return Err(self.peek_error(ErrorKind::EofWhileParsingValue));
        }
        Ok(())
    }

    #[inline]
    fn begin_child(&mut self) {
        let offset = self.i;
        if let Some(frame) = self.stack.last_mut() {
            frame.begin_child(offset, &self.cfg, &mut self.cp_stack);
        }
    }

    fn push_frame(&mut self, object: bool) -> Result<(), ScanError> {
        if self.stack.len() >= MAX_DEPTH {
            return Err(self.peek_error(ErrorKind::RecursionLimitExceeded));
        }
        self.stack
            .push(Frame::new(object, self.i, self.cp_stack.len()));
        self.i += 1;
        Ok(())
    }

    /// The closing bracket was taken: the container is complete. It is kept in
    /// the index if it has many children or many bytes.
    fn pop_frame(&mut self) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        let end = self.i;
        if frame.count >= self.cfg.min_children || end - frame.start >= self.cfg.min_bytes {
            let cp_start = self.pool.len();
            self.pool.extend(self.cp_stack.drain(frame.cp_base..));
            self.containers.push(Container {
                start: frame.start,
                end,
                count: frame.count,
                cp_start,
                cp_len: self.pool.len() - cp_start,
            });
        } else {
            self.cp_stack.truncate(frame.cp_base);
        }
    }

    /// What `serde_json` asks of the end of a top-level number or literal: it
    /// is followed by white space, by a byte that starts or ends something, or
    /// by nothing, so that `truefalse` and `1a` are refused.
    fn end_of_value(&mut self) -> Result<(), ScanError> {
        match self.b.get(self.i) {
            None
            | Some(b' ' | b'\n' | b'\t' | b'\r' | b'"' | b'[' | b']' | b'{' | b'}' | b',' | b':') => {
                Ok(())
            }
            Some(_) => Err(self.peek_error(ErrorKind::TrailingCharacters)),
        }
    }

    /// The rest of `null`, `true` or `false`, the first letter being taken.
    fn ident(&mut self, rest: &[u8]) -> Result<(), ScanError> {
        for &expected in rest {
            match self.b.get(self.i) {
                None => return Err(self.error_at(ErrorKind::EofWhileParsingValue, self.i)),
                Some(&c) => {
                    self.i += 1;
                    if c != expected {
                        return Err(self.taken_error(ErrorKind::ExpectedSomeIdent));
                    }
                }
            }
        }
        Ok(())
    }

    /// A number, as `serde_json` reads one with `arbitrary_precision`: `-`
    /// when negative, no leading zeros, a digit on each side of the point and
    /// after the exponent.
    fn number(&mut self) -> Result<(), ScanError> {
        if self.b[self.i] == b'-' {
            self.i += 1;
        }
        // The first digit is taken, not looked at: what is wrong with it is
        // reported at the byte itself, and the end of the input is "EOF".
        match self.b.get(self.i) {
            None => return Err(self.error_at(ErrorKind::EofWhileParsingValue, self.i)),
            Some(b'0') => {
                self.i += 1;
                if matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
                    return Err(self.peek_error(ErrorKind::InvalidNumber));
                }
            }
            Some(b'1'..=b'9') => {
                self.i += 1;
                self.digits();
            }
            Some(_) => {
                self.i += 1;
                return Err(self.taken_error(ErrorKind::InvalidNumber));
            }
        }

        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            let before = self.i;
            self.digits();
            if self.i == before {
                return Err(self.peek_error(if self.i < self.b.len() {
                    ErrorKind::InvalidNumber
                } else {
                    ErrorKind::EofWhileParsingValue
                }));
            }
        }

        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            match self.b.get(self.i) {
                None => return Err(self.error_at(ErrorKind::EofWhileParsingValue, self.i)),
                Some(b'0'..=b'9') => self.i += 1,
                Some(_) => {
                    self.i += 1;
                    return Err(self.taken_error(ErrorKind::InvalidNumber));
                }
            }
            self.digits();
        }
        Ok(())
    }

    #[inline]
    fn digits(&mut self) {
        while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
            self.i += 1;
        }
    }

    /// The rest of a string, its opening quote being taken: up to and past the
    /// closing one. Control characters, bad escapes and bytes that are not
    /// UTF-8 are errors, in the order in which `serde_json` meets them.
    fn string(&mut self) -> Result<(), ScanError> {
        let mut segment = self.i;
        let mut bad_utf8 = false;
        loop {
            let (at, high) = find_special(self.b, self.i);
            self.i = at;
            // Bytes past 0x7F are checked as UTF-8 where they are, once
            // the end of the segment they are in is known.
            let check = |segment: &[u8]| high && std::str::from_utf8(segment).is_err();
            match self.b.get(at) {
                None => return Err(self.error_at(ErrorKind::EofWhileParsingString, at)),
                Some(b'"') => {
                    bad_utf8 |= check(&self.b[segment..at]);
                    self.i = at + 1;
                    if bad_utf8 {
                        return Err(self.taken_error(ErrorKind::InvalidUnicodeCodePoint));
                    }
                    return Ok(());
                }
                Some(b'\\') => {
                    bad_utf8 |= check(&self.b[segment..at]);
                    self.i = at + 1;
                    self.escape()?;
                    segment = self.i;
                }
                Some(_) => {
                    self.i = at + 1;
                    return Err(self.taken_error(ErrorKind::ControlCharacterWhileParsingString));
                }
            }
        }
    }

    /// An escape sequence, its backslash being taken.
    fn escape(&mut self) -> Result<(), ScanError> {
        let Some(&c) = self.b.get(self.i) else {
            return Err(self.error_at(ErrorKind::EofWhileParsingString, self.i));
        };
        self.i += 1;
        match c {
            b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => Ok(()),
            b'u' => self.unicode_escape(),
            _ => Err(self.taken_error(ErrorKind::InvalidEscape)),
        }
    }

    /// The four hex digits of a `\u` escape.
    fn hex4(&mut self) -> Result<u16, ScanError> {
        let Some(digits) = self.b.get(self.i..self.i + 4) else {
            self.i = self.b.len();
            return Err(self.error_at(ErrorKind::EofWhileParsingString, self.i));
        };
        self.i += 4;
        let mut n = 0u16;
        for &d in digits {
            let v = match d {
                b'0'..=b'9' => d - b'0',
                b'a'..=b'f' => d - b'a' + 10,
                b'A'..=b'F' => d - b'A' + 10,
                _ => return Err(self.taken_error(ErrorKind::InvalidEscape)),
            };
            n = n << 4 | u16::from(v);
        }
        Ok(n)
    }

    /// A `\u` escape, `\u` being taken. A leading surrogate has to be followed
    /// by a trailing one in a second escape; a trailing one alone is wrong.
    fn unicode_escape(&mut self) -> Result<(), ScanError> {
        let n = self.hex4()?;
        if (0xDC00..=0xDFFF).contains(&n) {
            return Err(self.taken_error(ErrorKind::LoneLeadingSurrogateInHexEscape));
        }
        if !(0xD800..=0xDBFF).contains(&n) {
            return Ok(());
        }
        // `\` and `u` have to follow, each looked at and then taken.
        for expected in *b"\\u" {
            match self.b.get(self.i) {
                None => return Err(self.error_at(ErrorKind::EofWhileParsingString, self.i)),
                Some(&c) => {
                    self.i += 1;
                    if c != expected {
                        return Err(self.taken_error(ErrorKind::UnexpectedEndOfHexEscape));
                    }
                }
            }
        }
        let n2 = self.hex4()?;
        if !(0xDC00..=0xDFFF).contains(&n2) {
            return Err(self.taken_error(ErrorKind::LoneLeadingSurrogateInHexEscape));
        }
        Ok(())
    }
}

const ONES: u64 = 0x0101_0101_0101_0101;
const HIGHS: u64 = 0x8080_8080_8080_8080;

/// The first byte of `b[from..]` that ends a run of plain string content — a
/// quote, a backslash or a control character — as an offset into `b` (its
/// length if there is none), and whether any byte before it is past 0x7F.
///
/// Eight bytes at a time, with the "has a byte that is ..." arithmetic of
/// Mycroft's: it may flag bytes above the first true one, never the first.
#[inline]
fn find_special(b: &[u8], from: usize) -> (usize, bool) {
    let mut at = from;
    let mut high = 0u64;
    while let Some(chunk) = b.get(at..at + 8) {
        let w = u64::from_le_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]);
        let control = w.wrapping_sub(ONES * 0x20) & !w;
        let quote = {
            let x = w ^ (ONES * u64::from(b'"'));
            x.wrapping_sub(ONES) & !x
        };
        let backslash = {
            let x = w ^ (ONES * u64::from(b'\\'));
            x.wrapping_sub(ONES) & !x
        };
        let hit = (control | quote | backslash) & HIGHS;
        if hit != 0 {
            let lane = (hit.trailing_zeros() / 8) as usize;
            // The high bits of the lanes before the one that was hit.
            let before = if lane == 0 {
                0
            } else {
                w & HIGHS & (u64::MAX >> (64 - 8 * lane))
            };
            return (at + lane, (high | before) != 0);
        }
        high |= w & HIGHS;
        at += 8;
    }
    while let Some(&c) = b.get(at) {
        if c == b'"' || c == b'\\' || c < 0x20 {
            break;
        }
        high |= u64::from(c & 0x80);
        at += 1;
    }
    (at, high != 0)
}
