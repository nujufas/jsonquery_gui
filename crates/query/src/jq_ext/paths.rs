//! `delpaths`, and with it `del`: every path is deleted in one go, as in jq.
//!
//! jaq deletes the paths one after the other (`delpaths` is a `reduce` of `|= empty`),
//! so the second path of `del(.a[0], .a[1])` meets an array that is one shorter and
//! takes the wrong item (`[1,2,3]` gives `[2]` where jq gives `[3]`); `del(.[1,2])` fails
//! on the item that is no longer there; `del(.)` gives no output at all, where jq
//! gives `null`; and deleting a key of an object moved the last key into its place.
//!
//! jq sorts the paths, takes those that begin with the same key together, and goes down
//! into that key for the rest of them; the keys that are to go at one level go in
//! one pass over it. This is that, with jq's errors (`Cannot delete fields from number`)
//! and its rules for an index: a negative one counts from the end, a fraction is cut
//! to a whole number, one that is not there deletes nothing, and a slice is a range
//! of items.

use std::cell::Cell;
use std::collections::HashSet;
use std::iter::once;
use std::rc::Rc;

use jaq_core::native::{bome, v, Filter, Fun};
use jaq_core::path::Opt;
use jaq_core::{DataT, RunPtr, ValR, ValT as _};
use jaq_json::{Error, Map, Val};
use jaq_std::ValT as _;

use super::tabular::kind;

/// The native filter `_delpaths($paths)`, which `delpaths` and `del` are written with in
/// `prelude.jq`: a definition of the same name would be found before it.
pub(super) fn funs<D: for<'a> DataT<V<'a> = Val>>() -> [Fun<D>; 1] {
    let delpaths: Filter<RunPtr<D>> = ("_delpaths", v(1), |mut cv| {
        let paths = cv.0.pop_var();
        bome(delpaths(&cv.1, &paths))
    });
    [delpaths].map(jaq_core::native::run::<D>)
}

fn delpaths(root: &Val, paths: &Val) -> ValR<Val> {
    let Val::Arr(paths) = paths else {
        return Err(Error::str("Paths must be specified as an array"));
    };
    let mut sorted: Vec<&[Val]> = Vec::with_capacity(paths.len());
    for path in paths.iter() {
        let Val::Arr(path) = path else {
            return Err(Error::str(format!(
                "Path must be specified as array, not {}",
                kind(path)
            )));
        };
        // NaN is not equal to itself, and sorts before itself: paths with one in them
        // could not be put in order, or in groups (jq stops with a failed assertion).
        if path.iter().any(has_nan) {
            return Err(Error::str("Cannot delete array element at NaN index"));
        }
        sorted.push(path);
    }
    // Arrays sort item by item, and a path before the longer ones it begins: so the
    // paths that begin with one key are next to each other, and one that ends at that
    // key is the first of them.
    sorted.sort();
    match sorted.first() {
        None => Ok(root.clone()),
        // The path to the whole value: nothing is left of it.
        Some([]) => Ok(Val::Null),
        Some(_) => delete(root.clone(), &sorted, 0),
    }
}

/// `object` without what `paths` name, from their item `start` on. The paths are sorted,
/// all longer than `start` and equal before it.
fn delete(mut object: Val, paths: &[&[Val]], start: usize) -> ValR<Val> {
    let mut keys = Vec::new();
    let mut from = 0;
    while from < paths.len() {
        let key = &paths[from][start];
        // (the path itself is in its group, whatever its key is equal to)
        let upto = from
            + 1
            + paths[from + 1..]
                .iter()
                .take_while(|p| p[start] == *key)
                .count();
        if paths[from].len() == start + 1 {
            // All of this key goes; whatever else is named inside it does not matter.
            keys.push(key.clone());
        } else {
            let inside = get(&object, key)?;
            // A path into something that is not there deletes nothing.
            if !matches!(inside, Val::Null) {
                let inside = delete(inside, &paths[from..upto], start + 1)?;
                object = put(object, key, inside)?;
            }
        }
        from = upto;
    }
    delete_keys(object, &keys)
}

/// Whether `key` is a NaN, or an object or array with one in it (a slice `{start, end}`).
fn has_nan(key: &Val) -> bool {
    match key {
        Val::Num(_) => key.as_f64().is_some_and(f64::is_nan),
        Val::Arr(items) => items.iter().any(has_nan),
        Val::Obj(fields) => fields.values().any(has_nan),
        _ => false,
    }
}

/// What is at `key` in `object`, as jq's reading of a path takes it: `null` for a key that
/// is not there, and an error for a kind of key that the kind of value has none of (jaq
/// reads a number out of an object as `null`).
fn get(object: &Val, key: &Val) -> ValR<Val> {
    match (object, key) {
        (Val::Obj(_), Val::TStr(_) | Val::BStr(_))
        | (Val::Arr(_), Val::Num(_) | Val::Obj(_) | Val::Arr(_))
        | (Val::Null, Val::TStr(_) | Val::BStr(_) | Val::Num(_) | Val::Obj(_)) => {
            object.clone().index(key)
        }
        (_, Val::TStr(name)) => Err(Error::str(format!(
            "Cannot index {} with string \"{}\"",
            kind(object),
            String::from_utf8_lossy(name)
        ))),
        _ => Err(Error::str(format!(
            "Cannot index {} with {}",
            kind(object),
            kind(key)
        ))),
    }
}

/// `object` with `value` at `key`, where a slice is a range.
fn put(object: Val, key: &Val, value: Val) -> ValR<Val> {
    let value = Cell::new(Some(value));
    let put = |_| once(Ok(value.take().unwrap_or(Val::Null)));
    object.map_index(key, Opt::Essential, put).map_err(|exn| {
        exn.get_err()
            .unwrap_or_else(|_| Error::str("unexpected exit"))
    })
}

/// `value` without the items or fields `keys` name, all at once.
fn delete_keys(value: Val, keys: &[Val]) -> ValR<Val> {
    if keys.is_empty() {
        return Ok(value);
    }
    match value {
        Val::Null => Ok(Val::Null),
        Val::Arr(items) => {
            let mut gone = vec![false; items.len()];
            for key in keys {
                match key {
                    Val::Num(_) => {
                        if let Some(at) = position(key, items.len()) {
                            gone[at] = true;
                        }
                    }
                    Val::Obj(slice) => {
                        let (from, upto) = bounds(slice, items.len())?;
                        gone[from..upto].fill(true);
                    }
                    other => {
                        return Err(Error::str(format!(
                            "Cannot delete {} element of array",
                            kind(other)
                        )))
                    }
                }
            }
            Ok(items
                .iter()
                .zip(gone)
                .filter(|(_, gone)| !gone)
                .map(|(item, _)| item.clone())
                .collect())
        }
        Val::Obj(mut fields) => {
            let mut doomed = HashSet::with_capacity(keys.len());
            for key in keys {
                if !matches!(key, Val::TStr(_) | Val::BStr(_)) {
                    return Err(Error::str(format!(
                        "Cannot delete field at object index of {}",
                        kind(key)
                    )));
                }
                doomed.insert(key);
            }
            Rc::make_mut(&mut fields).retain(|key, _| !doomed.contains(key));
            Ok(Val::Obj(fields))
        }
        other => Err(Error::str(format!(
            "Cannot delete fields from {}",
            kind(&other)
        ))),
    }
}

/// The item of an array of `len` items that the number `index` is, if there is one: a
/// fraction is cut to a whole number, and a negative number counts from the end.
fn position(index: &Val, len: usize) -> Option<usize> {
    let number = index.as_f64()?;
    // (`as` cuts towards zero and saturates; a fraction between -1 and 0 is a negative
    // number that is 0 once cut, and so the item after the last, which is none.)
    let whole = number as i64;
    let at = if number < 0.0 {
        len as i64 + whole
    } else {
        whole
    };
    usize::try_from(at).ok().filter(|at| *at < len)
}

/// The items `from..upto` of an array of `len` items that the slice `{start, end}` is,
/// bounds as jq clips them: a negative one counts from the end, a fraction cuts the start
/// down and the end up.
fn bounds(slice: &Map, len: usize) -> Result<(usize, usize), Error> {
    let bound = |name: &'static str, default: f64| match slice.get(&Val::utf8_str(name)) {
        None | Some(Val::Null) => Ok(default),
        Some(number @ Val::Num(_)) => Ok(number.as_f64().unwrap_or(default)),
        Some(_) => Err(Error::str(
            "Start and end indices of an array slice must be numbers",
        )),
    };
    let len_f = len as f64;
    let mut start = bound("start", 0.0)?;
    let mut end = bound("end", len_f)?;
    if start < 0.0 {
        start += len_f;
    }
    if end < 0.0 {
        end += len_f;
    }
    start = start.clamp(0.0, len_f);
    end = end.min(len_f).max(start);
    let from = start as usize;
    let mut upto = end as usize;
    if (upto as f64) < end {
        upto += 1;
    }
    Ok((from, upto.min(len)))
}
