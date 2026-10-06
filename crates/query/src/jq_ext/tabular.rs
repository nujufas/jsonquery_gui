//! `@csv` and `@tsv`: an array as one row of text, the way jq writes it.
//!
//! The input has to be an array whose items are `null`, booleans, numbers or
//! strings; anything else is an error worded as jq words it. A `null` is an
//! empty field, booleans and numbers are written as they print everywhere else,
//! and a string is
//! - for CSV, put in quotes with each `"` doubled;
//! - for TSV, left bare, with the characters that would break the row written
//!   as escapes: tab `\t`, newline `\n`, carriage return `\r`, backslash `\\`
//!   (and NUL, `\0`).
//!
//! jaq has these two only in `jaq-fmts`, a crate that comes with YAML, TOML,
//! XML and CBOR parsers this app has no use for; its `@tsv` also builds a
//! string matcher for every field it writes, which made it five times slower
//! than `@csv` over a few hundred thousand rows.

use std::fmt::{self, Write as _};
use std::io::Write as _;

use jaq_core::native::{bome, v, Filter, Fun};
use jaq_core::{DataT, RunPtr, ValR};
use jaq_json::{Error, Val};

/// How much of a value an error message quotes, in characters (jq cuts it at
/// 11; this is a little more generous).
const QUOTE_CHARS: usize = 24;

#[derive(Clone, Copy)]
enum Table {
    Csv,
    Tsv,
}

impl Table {
    fn name(self) -> &'static str {
        match self {
            Table::Csv => "csv",
            Table::Tsv => "tsv",
        }
    }
}

pub(super) fn funs<D: for<'a> DataT<V<'a> = Val>>() -> [Fun<D>; 2] {
    let csv: Filter<RunPtr<D>> = ("@csv", v(0), |cv| bome(row(&cv.1, Table::Csv)));
    let tsv: Filter<RunPtr<D>> = ("@tsv", v(0), |cv| bome(row(&cv.1, Table::Tsv)));
    [csv, tsv].map(jaq_core::native::run::<D>)
}

fn row(val: &Val, table: Table) -> ValR<Val> {
    let name = table.name();
    let Val::Arr(fields) = val else {
        return Err(Error::str(format!(
            "{} cannot be {name}-formatted, only an array can be",
            describe(val)
        )));
    };
    let mut out: Vec<u8> = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push(match table {
                Table::Csv => b',',
                Table::Tsv => b'\t',
            });
        }
        match field {
            Val::Null => {}
            // Writing into a `Vec` cannot fail.
            Val::Bool(_) | Val::Num(_) => write!(out, "{field}").unwrap(),
            Val::TStr(text) | Val::BStr(text) => match table {
                Table::Csv => quote(&mut out, text),
                Table::Tsv => escape(&mut out, text),
            },
            other => {
                return Err(Error::str(format!(
                    "{} is not valid in a {name} row",
                    describe(other)
                )))
            }
        }
    }
    Ok(Val::utf8_str(out))
}

/// A CSV string field: in quotes, every quote inside doubled.
fn quote(out: &mut Vec<u8>, text: &[u8]) {
    out.push(b'"');
    for &byte in text {
        if byte == b'"' {
            out.push(b'"');
        }
        out.push(byte);
    }
    out.push(b'"');
}

/// A TSV string field: the bytes that would end the field or the row, escaped.
fn escape(out: &mut Vec<u8>, text: &[u8]) {
    for &byte in text {
        match byte {
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0 => out.extend_from_slice(b"\\0"),
            _ => out.push(byte),
        }
    }
}

/// A value as jq names it in an error: its type, then its JSON cut short —
/// `object ({"b":[1,2,3...)`.
fn describe(val: &Val) -> String {
    let kind = match val {
        Val::Null => "null",
        Val::Bool(_) => "boolean",
        Val::Num(_) => "number",
        Val::TStr(_) | Val::BStr(_) => "string",
        Val::Arr(_) => "array",
        Val::Obj(_) => "object",
    };
    let mut quoted = Quoted::default();
    // Cut short on purpose: the value may be huge, and there is no need to
    // print all of it only to throw most of it away.
    let _ = write!(quoted, "{val}");
    let cut = if quoted.cut { "..." } else { "" };
    format!("{kind} ({}{cut})", quoted.text)
}

/// A `fmt::Write` that keeps the first [`QUOTE_CHARS`] characters and then
/// stops the formatting that feeds it.
#[derive(Default)]
struct Quoted {
    text: String,
    chars: usize,
    cut: bool,
}

impl fmt::Write for Quoted {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            if self.chars == QUOTE_CHARS {
                self.cut = true;
                return Err(fmt::Error);
            }
            self.text.push(c);
            self.chars += 1;
        }
        Ok(())
    }
}
