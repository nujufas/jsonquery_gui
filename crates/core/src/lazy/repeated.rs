//! Objects that have a key twice.
//!
//! JSON that has one is read the way a parsed value would be: the key is there
//! once, in the place of its first, holding the value of its last. Finding the
//! objects that have one is a pass over the bytes of its own, made at the same
//! time as the scan that indexes them (by another thread, so that it takes no
//! longer), and what it finds is the places of those objects, of which there are
//! next to none: only they are read any other way than from the file as it is.

use std::collections::HashMap;
use std::ops::Range;

use super::{kind_of, Children, Layout, LazyTree};
use crate::tree::ValueKind;

/// The start of the objects of `b` (the place of their `{`) that have a key
/// twice, in order.
///
/// It does not check that `b` is JSON — the scan does that — so it has to get to
/// the end of whatever it is given: every step goes on at least one byte.
pub(super) fn find(b: &[u8]) -> Vec<usize> {
    struct Frame {
        object: bool,
        start: usize,
        /// Where the keys of the object begin in `keys`.
        key_base: usize,
        /// The next string is a key.
        want_key: bool,
    }

    let mut stack: Vec<Frame> = Vec::new();
    let mut keys: Vec<(u64, usize)> = Vec::new();
    let mut repeated = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' | b'[' => {
                stack.push(Frame {
                    object: b[i] == b'{',
                    start: i,
                    key_base: keys.len(),
                    want_key: true,
                });
                i += 1;
            }
            b'}' | b']' => {
                if let Some(frame) = stack.pop() {
                    if frame.object
                        && keys.len() - frame.key_base >= 2
                        && keys_repeat(b, &keys[frame.key_base..])
                    {
                        repeated.push(frame.start);
                    }
                    keys.truncate(frame.key_base);
                }
                i += 1;
            }
            b',' => {
                if let Some(frame) = stack.last_mut() {
                    frame.want_key = frame.object;
                }
                i += 1;
            }
            b'"' => {
                let start = i + 1;
                let (end, escaped) = string_end(b, start);
                if let Some(frame) = stack.last_mut() {
                    if frame.object && frame.want_key {
                        let signature = if escaped {
                            decoded_signature(&b[start - 1..(end + 1).min(b.len())])
                        } else {
                            signature(b, start, end)
                        };
                        keys.push((signature, start));
                        frame.want_key = false;
                    }
                }
                i = end + 1;
            }
            b' ' | b'\n' | b'\t' | b'\r' | b':' => i += 1,
            _ => {
                i += 1;
                while i < b.len()
                    && !matches!(
                        b[i],
                        b',' | b'}'
                            | b']'
                            | b' '
                            | b'\n'
                            | b'\t'
                            | b'\r'
                            | b'"'
                            | b'{'
                            | b'['
                            | b':'
                    )
                {
                    i += 1;
                }
            }
        }
    }
    repeated.sort_unstable();
    repeated
}

/// Where the string that begins at `start` (after its quote) ends, and whether
/// it has an escape in it.
fn string_end(b: &[u8], start: usize) -> (usize, bool) {
    let mut at = start;
    let mut escaped = false;
    while let Some(found) = memchr::memchr2(b'"', b'\\', &b[at.min(b.len())..]) {
        at += found;
        if b[at] == b'"' {
            return (at, escaped);
        }
        escaped = true;
        at += 2;
    }
    (b.len(), escaped)
}

/// A short number that is the same for the same key and very likely not for
/// another: its first and last eight bytes, and its length.
#[inline]
fn signature(b: &[u8], start: usize, end: usize) -> u64 {
    let len = end.saturating_sub(start);
    let load = |at: usize| -> u64 {
        let mut word = [0u8; 8];
        let rest = b.get(at..).unwrap_or_default();
        let n = rest.len().min(8);
        word[..n].copy_from_slice(&rest[..n]);
        u64::from_le_bytes(word)
    };
    let head = if len < 8 {
        load(start) & ((1u64 << (len * 8)) - 1)
    } else {
        load(start)
    };
    let tail = if len >= 8 { load(end - 8) } else { head };
    (head ^ tail.rotate_left(29) ^ ((len as u64) << 56)).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// The signature of a key that has an escape in it, from what it is: written two
/// ways — `a` and `a` — it is one key.
#[cold]
fn decoded_signature(quoted: &[u8]) -> u64 {
    let text: String = serde_json::from_slice(quoted).unwrap_or_default();
    signature(text.as_bytes(), 0, text.len())
}

/// Whether two of the keys of an object, given as (signature, where it begins),
/// are the same.
fn keys_repeat(b: &[u8], keys: &[(u64, usize)]) -> bool {
    if keys.len() <= 16 {
        return keys.iter().enumerate().any(|(at, key)| {
            keys[..at]
                .iter()
                .any(|before| before.0 == key.0 && same_key(b, before.1, key.1))
        });
    }
    let mut sorted = keys.to_vec();
    sorted.sort_unstable_by_key(|key| key.0);
    let mut from = 0;
    while from < sorted.len() {
        let run = sorted[from..]
            .iter()
            .take_while(|key| key.0 == sorted[from].0)
            .count();
        let same = &sorted[from..from + run];
        if run > 1
            && same
                .iter()
                .enumerate()
                .any(|(at, key)| same[..at].iter().any(|before| same_key(b, before.1, key.1)))
        {
            return true;
        }
        from += run;
    }
    false
}

/// Whether the keys that begin (after their quote) at `x` and at `y` are the
/// same key, as they are once their escapes are read.
fn same_key(b: &[u8], x: usize, y: usize) -> bool {
    let (kx, ky) = (
        &b[x.min(b.len())..string_end(b, x).0],
        &b[y.min(b.len())..string_end(b, y).0],
    );
    if kx == ky {
        return true;
    }
    if memchr::memchr(b'\\', kx).is_none() && memchr::memchr(b'\\', ky).is_none() {
        return false;
    }
    let decode = |key: &[u8]| -> Option<String> {
        let mut quoted = Vec::with_capacity(key.len() + 2);
        quoted.push(b'"');
        quoted.extend_from_slice(key);
        quoted.push(b'"');
        serde_json::from_slice(&quoted).ok()
    };
    decode(kx) == decode(ky)
}

/// One member of an object that has a key twice, as it is read: the key (as it
/// is written where it is first) and the value (where it is last).
#[derive(Clone, Copy, Debug)]
pub(super) struct Member {
    /// What is between the quotes of the key.
    pub key: (usize, usize),
    pub value: (usize, usize),
    pub kind: ValueKind,
}

/// The members of the object that begins at `start`, each key once.
pub(super) fn members(tree: &LazyTree, start: usize) -> Vec<Member> {
    let bytes = tree.bytes();
    let Some(kind) = bytes.get(start).copied().and_then(kind_of) else {
        return Vec::new();
    };
    if kind != ValueKind::Object {
        return Vec::new();
    }
    let raw = Children {
        tree,
        at: tree.skip_ws(start + 1),
        layout: Some(Layout::Object),
        index: 0,
        members: None,
    };
    let mut made: Vec<Member> = Vec::new();
    let mut places: HashMap<String, usize> = HashMap::new();
    for child in raw {
        let Some(key) = child.key else { continue };
        let range = child.node.byte_range();
        let key_at = key.offset_in(bytes);
        let member = Member {
            key: (key_at, key_at + key.len()),
            value: (range.start, range.end),
            kind: child.node.kind(),
        };
        match places.get(&key.to_string()) {
            // Its value is the last one's; its place is the first's.
            Some(&place) => made[place].value = member.value,
            None => {
                places.insert(key.to_string(), made.len());
                made.push(member);
            }
        }
        // The kind goes with the value.
        if let Some(&place) = places.get(&key.to_string()) {
            made[place].kind = member.kind;
        }
    }
    made
}

/// The range of the bytes of the key, for a member.
pub(super) fn key_range(member: &Member) -> Range<usize> {
    member.key.0..member.key.1
}
