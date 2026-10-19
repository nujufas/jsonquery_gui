//! `@base64d`, as jq decodes it, in place of jaq's `decode_base64`.
//!
//! jaq's needs the text to be padded with `=` to a multiple of four, and to be exactly
//! what an encoder writes. The base64 people meet in JSON is often not: a JWT segment
//! has no padding at all (`"eyJhIjoxfQ"`), and jq decodes it. So does this, by jq's rules:
//! it reads up to the first `=` (what follows is not looked at), a letter, a digit,
//! `+` or `/` makes six bits, and a byte or two at the end is what the last two or three
//! characters make. Only a single character left over (`"a"`) is an error, as is any
//! other character, a space or a line break included.
//!
//! The bytes decoded are text the way jq makes text of bytes, which is not quite the
//! way Rust does: every sequence of bytes that is not UTF-8 is one U+FFFD ([`text`]).
//! jaq kept the bytes as they were, and `explode` of such a string gave negative numbers.

use jaq_core::native::{bome, v, Filter, Fun};
use jaq_core::{DataT, RunPtr, ValR};
use jaq_json::{Error, Val};

use super::tabular::describe;

/// Whether jaq-std's native filter `name` taking `arity` arguments is the one written
/// again here, and so left out.
pub(super) fn replaces_fun(name: &str, arity: usize) -> bool {
    name == "decode_base64" && arity == 0
}

pub(super) fn funs<D: for<'a> DataT<V<'a> = Val>>() -> [Fun<D>; 1] {
    let decode: Filter<RunPtr<D>> = ("decode_base64", v(0), |cv| bome(decode(&cv.1)));
    [decode].map(jaq_core::native::run::<D>)
}

fn decode(val: &Val) -> ValR<Val> {
    let (Val::TStr(text) | Val::BStr(text)) = val else {
        return Err(Error::str(format!(
            "{} cannot be base64-decoded",
            describe(val)
        )));
    };
    match bytes(text) {
        Ok(bytes) => Ok(Val::utf8_str(text_of(&bytes))),
        Err(reason) => Err(Error::str(format!("{} {reason}", describe(val)))),
    }
}

/// What the base64 `text` says, up to its first `=`.
fn bytes(text: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3 + 2);
    // The bits of the characters of the group so far, and how many characters that is.
    let (mut bits, mut chars) = (0u32, 0u8);
    for &byte in text.iter().take_while(|byte| **byte != b'=') {
        let six = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err("is not valid base64 data"),
        };
        bits = (bits << 6) | u32::from(six);
        chars += 1;
        if chars == 4 {
            out.extend([(bits >> 16) as u8, (bits >> 8) as u8, bits as u8]);
            (bits, chars) = (0, 0);
        }
    }
    match chars {
        0 => {}
        1 => return Err("trailing base64 byte found"),
        2 => out.push((bits >> 4) as u8),
        _ => out.extend([(bits >> 10) as u8, (bits >> 2) as u8]),
    }
    Ok(out)
}

/// The text jq makes of `bytes`: what is UTF-8 is read as it is, and every sequence that
/// is not becomes one U+FFFD, the way jq's own reader of UTF-8 takes it. (Rust's
/// `from_utf8_lossy` differs: it counts a surrogate, or a character written in more bytes
/// than it needs, as one replacement for every byte, and not as one for the sequence.)
fn text_of(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    let mut rest = bytes;
    while !rest.is_empty() {
        let (char, len) = next_char(rest);
        text.push(char.unwrap_or('\u{fffd}'));
        rest = &rest[len..];
    }
    text
}

/// The character `rest` begins with, if it is one, and how many bytes it takes up: the
/// bytes of a broken sequence are taken up with it.
fn next_char(rest: &[u8]) -> (Option<char>, usize) {
    let first = rest[0];
    if first < 0x80 {
        return (Some(char::from(first)), 1);
    }
    // How many bytes a character that begins with `first` takes, and the bits of
    // `first` that are part of it. A byte that cannot begin one (a continuation byte, the
    // old 0xC0 and 0xC1, and 0xF5 and up) is a single byte of nothing.
    let (len, lead_bits) = match first {
        0xC2..=0xDF => (2, first & 0x1F),
        0xE0..=0xEF => (3, first & 0x0F),
        0xF0..=0xF4 => (4, first & 0x07),
        _ => return (None, 1),
    };
    if rest.len() < len {
        // The text ends before the sequence does: all that is left of it is one.
        return (None, rest.len());
    }
    let mut code = u32::from(lead_bits);
    for (at, &byte) in rest[1..len].iter().enumerate() {
        if byte & 0xC0 != 0x80 {
            // The lead and the continuation bytes before this one, without it.
            return (None, at + 1);
        }
        code = (code << 6) | u32::from(byte & 0x3F);
    }
    // Not the shortest way to write it, or a surrogate, or past U+10FFFF: not a character.
    let shortest = [0, 0, 0x80, 0x800, 0x1_0000][len];
    (char::from_u32(code).filter(|_| code >= shortest), len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What jq 1.8.1 makes of each sequence of bytes: the code points of `@base64d | explode`
    /// over their base64. (U+FFFD is 0xFFFD, one for each sequence that is not a character.)
    #[test]
    #[rustfmt::skip]
    fn bytes_that_are_not_utf8_become_replacement_characters_as_jq_takes_them() {
        let cases: &[(&[u8], &[u32])] = &[
                (&[0xC0], &[0xFFFD]),
                (&[0xC1], &[0xFFFD]),
                (&[0xC2], &[0xFFFD]),
                (&[0xC3, 0xA9], &[233]),
                (&[0xC3], &[0xFFFD]),
                (&[0xC3, 0x41], &[0xFFFD, 65]),
                (&[0xE0, 0x80], &[0xFFFD]),
                (&[0xE0, 0x80, 0x80], &[0xFFFD]),
                (&[0xE0, 0xA0, 0x80], &[0x800]),
                (&[0xE0, 0x9F, 0xBF], &[0xFFFD]),
                (&[0xE0, 0x82, 0x80], &[0xFFFD]),
                (&[0xE0, 0xA0, 0x7F], &[0xFFFD, 127]),
                (&[0xE1, 0x80, 0x80], &[0x1000]),
                (&[0xEF, 0xBF, 0xBF], &[0xFFFF]),
                (&[0xC0, 0x80], &[0xFFFD, 0xFFFD]),
                (&[0xC1, 0xBF], &[0xFFFD, 0xFFFD]),
                (&[0xC2, 0x80], &[128]),
                (&[0xF0, 0x8F, 0xBF, 0xBF], &[0xFFFD]),
                (&[0xF0, 0x80, 0xBF, 0xBF], &[0xFFFD]),
                (&[0xE2, 0x82], &[0xFFFD]),
                (&[0xE2, 0x82, 0xAC], &[0x20AC]),
                (&[0xED, 0xA0, 0x80], &[0xFFFD]),
                (&[0xED, 0xA0], &[0xFFFD]),
                (&[0xED, 0x9F, 0xBF], &[0xD7FF]),
                (&[0xEE, 0x80, 0x80], &[0xE000]),
                (&[0xF0, 0x80, 0x80, 0x80], &[0xFFFD]),
                (&[0xF0, 0x90, 0x80, 0x80], &[0x10000]),
                (&[0xF0, 0x9F, 0x98], &[0xFFFD]),
                (&[0xF0, 0x9F, 0x98, 0x80], &[0x1F600]),
                (&[0xF4, 0x8F, 0xBF, 0xBF], &[0x10FFFF]),
                (&[0xF4, 0x90, 0x80, 0x80], &[0xFFFD]),
                (&[0xF5], &[0xFFFD]),
                (&[0xFF], &[0xFFFD]),
                (&[0x80], &[0xFFFD]),
                (&[0xBF], &[0xFFFD]),
                (&[0x41, 0xE2, 0x82], &[65, 0xFFFD]),
                (&[0x41, 0xE2, 0x82, 0x41], &[65, 0xFFFD, 65]),
                (&[0x9E, 0xE9, 0x65], &[0xFFFD, 0xFFFD]),
                (&[0xE9, 0x65], &[0xFFFD]),
                (&[0xC3, 0xC3, 0xA9], &[0xFFFD, 233]),
                (&[0xE2, 0x82, 0xE2, 0x82, 0xAC], &[0xFFFD, 0x20AC]),
                (&[0xF0, 0x9F, 0x41, 0x41], &[0xFFFD, 65, 65]),
        ];
        for (bytes, expected) in cases {
            let got: Vec<u32> = text_of(bytes).chars().map(u32::from).collect();
            assert_eq!(&got, expected, "{bytes:02x?}");
        }
    }

    #[test]
    fn every_character_is_read_as_the_character_it_is() {
        for char in (0..=0x10FFFF).filter_map(char::from_u32) {
            let mut buf = [0; 4];
            let bytes = char.encode_utf8(&mut buf).as_bytes();
            assert_eq!(text_of(bytes), char.to_string(), "{char:?}");
        }
    }

    #[test]
    fn the_end_of_the_text_is_one_or_two_bytes_or_a_lone_character_that_is_an_error() {
        assert_eq!(bytes(b""), Ok(vec![]));
        assert_eq!(bytes(b"YQ"), Ok(b"a".to_vec()));
        assert_eq!(bytes(b"YWI"), Ok(b"ab".to_vec()));
        assert_eq!(bytes(b"YWJj"), Ok(b"abc".to_vec()));
        assert_eq!(bytes(b"YWJjZA"), Ok(b"abcd".to_vec()));
        // padding, however much, ends the text; what follows it is not looked at
        assert_eq!(bytes(b"YQ=="), Ok(b"a".to_vec()));
        assert_eq!(bytes(b"YQ====="), Ok(b"a".to_vec()));
        assert_eq!(bytes(b"YQ=$$$"), Ok(b"a".to_vec()));
        assert_eq!(bytes(b"="), Ok(vec![]));
        assert_eq!(bytes(b"Y"), Err("trailing base64 byte found"));
        assert_eq!(bytes(b"YWJjZ"), Err("trailing base64 byte found"));
        assert_eq!(bytes(b"Y==="), Err("trailing base64 byte found"));
        for text in [&b"YQ$"[..], b"Y Q", b"YQ\n", b"-_", b"\xc3\xa9"] {
            assert_eq!(bytes(text), Err("is not valid base64 data"), "{text:?}");
        }
    }

    #[test]
    fn every_byte_value_survives_a_round_trip_through_the_alphabet() {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        // three bytes from each group of four characters of the alphabet
        for (i, group) in ALPHABET.chunks(4).enumerate() {
            let got = bytes(group).unwrap();
            let expected: Vec<u8> = {
                let six = |c: u8| ALPHABET.iter().position(|a| *a == c).unwrap() as u32;
                let bits = group.iter().fold(0, |bits, c| (bits << 6) | six(*c));
                vec![(bits >> 16) as u8, (bits >> 8) as u8, bits as u8]
            };
            assert_eq!(got, expected, "group {i}");
        }
    }
}
