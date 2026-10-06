//! A key: the value that a function made of an element, put in a form that can be
//! moved between threads (jaq's own values cannot) and compared quickly, and
//! that is compared as jaq compares them — `null`, then booleans, numbers,
//! strings, lists and objects, each in its own order — because what is sorted
//! here has to be sorted as jaq would sort it.

use std::cmp::Ordering;

use jaq_json::Num;
use serde_json::Value;

use crate::{from_val, to_val, Val};

/// A value, for comparing.
#[derive(Clone, Debug)]
pub(super) enum Atom {
    Null,
    Bool(bool),
    Int(isize),
    Float(f64),
    /// An integer that a machine integer cannot hold: its digits.
    Big(Box<str>),
    /// A string: its bytes, which are what is compared.
    Str(Box<[u8]>),
    Arr(Vec<Atom>),
    /// An object, which is compared by what jaq makes of it: they are rare as keys.
    Obj(Box<Value>),
}

impl Atom {
    pub(super) fn from_val(value: &Val) -> Atom {
        match value {
            Val::Null => Atom::Null,
            Val::Bool(b) => Atom::Bool(*b),
            Val::Num(n) => Atom::of_num(n),
            Val::BStr(bytes) | Val::TStr(bytes) => Atom::Str(bytes.to_vec().into_boxed_slice()),
            Val::Arr(items) => Atom::Arr(items.iter().map(Atom::from_val).collect()),
            Val::Obj(_) => Atom::Obj(Box::new(from_val(value))),
        }
    }

    fn of_num(n: &Num) -> Atom {
        match n {
            Num::Int(i) => Atom::Int(*i),
            Num::Float(f) => Atom::Float(*f),
            Num::BigInt(i) => Atom::Big(i.to_string().into_boxed_str()),
            // A decimal is compared as the float it is.
            Num::Dec(text) => Atom::of_num(&Num::from_dec_str(text)),
        }
    }

    /// The number, as jaq's, for the comparisons that are not of two machine
    /// numbers.
    fn num(&self) -> Option<Num> {
        match self {
            Atom::Int(i) => Some(Num::Int(*i)),
            Atom::Float(f) => Some(Num::Float(*f)),
            Atom::Big(text) => Num::from_str_radix(text, 10),
            _ => None,
        }
    }

    /// How much of memory it takes, about.
    pub(super) fn bytes(&self) -> usize {
        std::mem::size_of::<Atom>()
            + match self {
                Atom::Big(text) => text.len(),
                Atom::Str(bytes) => bytes.len(),
                Atom::Arr(items) => items.iter().map(Atom::bytes).sum(),
                Atom::Obj(value) => super::approximate_size(value),
                _ => 0,
            }
    }

    /// Where its kind comes in the order of kinds.
    fn rank(&self) -> u8 {
        match self {
            Atom::Null => 0,
            Atom::Bool(_) => 1,
            Atom::Int(_) | Atom::Float(_) | Atom::Big(_) => 2,
            Atom::Str(_) => 3,
            Atom::Arr(_) => 4,
            Atom::Obj(_) => 5,
        }
    }
}

/// How jaq orders two floats: zeros are the same, a NaN is before anything.
fn float_cmp(left: f64, right: f64) -> Ordering {
    if left == 0. && right == 0. {
        Ordering::Equal
    } else if left.is_nan() {
        Ordering::Less
    } else if right.is_nan() {
        Ordering::Greater
    } else {
        f64::total_cmp(&left, &right)
    }
}

impl Ord for Atom {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Atom::Null, Atom::Null) => Ordering::Equal,
            (Atom::Bool(x), Atom::Bool(y)) => x.cmp(y),
            (Atom::Int(x), Atom::Int(y)) => x.cmp(y),
            (Atom::Float(x), Atom::Float(y)) => float_cmp(*x, *y),
            (Atom::Int(x), Atom::Float(y)) => float_cmp(*x as f64, *y),
            (Atom::Float(x), Atom::Int(y)) => float_cmp(*x, *y as f64),
            (Atom::Str(x), Atom::Str(y)) => x.cmp(y),
            (Atom::Arr(x), Atom::Arr(y)) => x.cmp(y),
            (Atom::Obj(x), Atom::Obj(y)) => to_val(x).cmp(&to_val(y)),
            (x, y) if x.rank() == 2 && y.rank() == 2 => match (x.num(), y.num()) {
                (Some(x), Some(y)) => x.cmp(&y),
                _ => Ordering::Equal,
            },
            (x, y) => x.rank().cmp(&y.rank()),
        }
    }
}

impl PartialOrd for Atom {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Atom {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Atom {}

/// The key of an element: what the function made of it, all of it, as the
/// array that jaq's `sort_by` and the others make of the function.
#[derive(Clone, Debug)]
pub(super) enum Key {
    /// What it made was one value, which is almost always so.
    One(Atom),
    Many(Vec<Atom>),
}

impl Key {
    /// From the array of what the function made.
    pub(super) fn from_val(made: &Val) -> Key {
        match made {
            Val::Arr(items) if items.len() == 1 => Key::One(Atom::from_val(&items[0])),
            Val::Arr(items) => Key::Many(items.iter().map(Atom::from_val).collect()),
            other => Key::One(Atom::from_val(other)),
        }
    }

    pub(super) fn from_value(made: &Value) -> Key {
        Key::from_val(&to_val(made))
    }

    pub(super) fn as_slice(&self) -> &[Atom] {
        match self {
            Key::One(atom) => std::slice::from_ref(atom),
            Key::Many(atoms) => atoms,
        }
    }

    pub(super) fn bytes(&self) -> usize {
        std::mem::size_of::<Key>() + self.as_slice().iter().map(Atom::bytes).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A random number of every shape a number has: machine integers, ones that
    /// are not, floats, with exponents, and zeros of both signs.
    fn number(rng: &mut u64) -> String {
        let mut next = || {
            *rng ^= *rng << 13;
            *rng ^= *rng >> 7;
            *rng ^= *rng << 17;
            *rng
        };
        match next() % 12 {
            0 => "0".to_owned(),
            1 => "-0".to_owned(),
            2 => "0.0".to_owned(),
            3 => (next() % 20).to_string(),
            4 => format!("-{}", next() % 20),
            5 => format!("{}.{}", next() % 20, next() % 10),
            6 => format!("{}e{}", next() % 9, next() % 4),
            7 => "9223372036854775807".to_owned(),
            8 => "9223372036854775808".to_owned(),
            9 => "-9223372036854775809".to_owned(),
            10 => format!("{}", 9_007_199_254_740_990u64 + next() % 6),
            _ => format!("12345678901234567890{}", next() % 3),
        }
    }

    fn value(rng: &mut u64, depth: usize) -> Value {
        let mut next = || {
            *rng ^= *rng << 13;
            *rng ^= *rng >> 7;
            *rng ^= *rng << 17;
            *rng
        };
        let kinds = if depth == 0 { 5 } else { 7 };
        match next() % kinds {
            0 => Value::Null,
            1 => Value::Bool(next() % 2 == 0),
            2 | 3 => serde_json::from_str(&number(rng)).unwrap(),
            4 => Value::String(
                ["", "a", "b", "ab", "é", "\u{1F600}", "B", "10", "9"][(next() % 9) as usize]
                    .to_owned(),
            ),
            5 => Value::Array((0..next() % 4).map(|_| value(rng, depth - 1)).collect()),
            _ => Value::Object(
                (0..next() % 3)
                    .map(|n| {
                        (
                            ["a", "b", "c"][n as usize].to_owned(),
                            value(rng, depth - 1),
                        )
                    })
                    .collect(),
            ),
        }
    }

    #[test]
    fn keys_are_ordered_as_jaq_orders_values() {
        let mut rng = 0x9E37_79B9_7F4A_7C15u64;
        let values: Vec<Value> = (0..400).map(|_| value(&mut rng, 3)).collect();
        let mut compared = 0;
        for a in &values {
            for b in &values {
                let (x, y) = (Atom::from_val(&to_val(a)), Atom::from_val(&to_val(b)));
                assert_eq!(x.cmp(&y), to_val(a).cmp(&to_val(b)), "{a}  against  {b}");
                assert_eq!(x == y, to_val(a) == to_val(b), "{a}  is  {b}");
                compared += 1;
            }
        }
        assert_eq!(compared, 160_000);
    }

    #[test]
    fn a_key_of_one_value_and_a_key_of_several_are_compared_as_arrays() {
        let one = Key::from_value(&serde_json::json!([1]));
        let two = Key::from_value(&serde_json::json!([1, 0]));
        let none = Key::from_value(&serde_json::json!([]));
        assert!(matches!(one, Key::One(_)) && matches!(two, Key::Many(_)));
        assert!(none.as_slice() < one.as_slice());
        assert!(one.as_slice() < two.as_slice());
        assert!(two.as_slice() > one.as_slice());
        assert_eq!(
            one.as_slice(),
            Key::from_value(&serde_json::json!([1.0])).as_slice()
        );
    }
}
