//! `tostream`: a value as the stream of events `jq --stream` would read.
//!
//! A scalar, or an empty array or object, is one event `[path, value]`. A
//! non-empty array or object is the events of its children, in order, followed
//! by a closing event `[path to its last child]`. So `{"a":[1,2]}` streams as
//!
//! ```text
//! [["a",0],1]  [["a",1],2]  [["a",1]]  [["a"]]
//! ```
//!
//! jq defines it in jq (`path(def r: (.[]?|r), .; r) as $p | getpath($p) | …`),
//! which jaq runs about 17 times slower than the walk here over a 25 MB
//! document. This walk is also lazy, so `limit(5; tostream)` stops after five
//! events, and it keeps its own stack instead of recursing.

use std::rc::Rc;

use jaq_core::native::{run, Filter, Fun};
use jaq_core::{DataT, RunPtr};
use jaq_json::{Map, Val};

pub(super) fn fun<D: for<'a> DataT<V<'a> = Val>>() -> Fun<D> {
    let tostream: Filter<RunPtr<D>> = ("tostream", jaq_core::native::v(0), |cv| {
        Box::new(Events::new(cv.1).map(Ok))
    });
    run::<D>(tostream)
}

/// The children of a non-empty array or object, kept alive by their `Rc`.
enum Children {
    Array(Rc<Vec<Val>>),
    Object(Rc<Map>),
}

impl Children {
    /// `None` for anything that is one event by itself: a scalar, `[]`, `{}`.
    fn of(val: &Val) -> Option<Self> {
        match val {
            Val::Arr(items) if !items.is_empty() => Some(Self::Array(items.clone())),
            Val::Obj(entries) if !entries.is_empty() => Some(Self::Object(entries.clone())),
            _ => None,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Array(items) => items.len(),
            Self::Object(entries) => entries.len(),
        }
    }

    /// The `i`-th child as `(key or index, value)`.
    fn get(&self, i: usize) -> (Val, Val) {
        match self {
            Self::Array(items) => (Val::from(i), items[i].clone()),
            Self::Object(entries) => {
                let (key, value) = entries.get_index(i).expect("index below len");
                (key.clone(), value.clone())
            }
        }
    }
}

/// A container whose children are being walked.
struct Frame {
    children: Children,
    /// How many children have been taken.
    next: usize,
    /// The path to this container.
    path: Vec<Val>,
    /// The key or index of the child taken last: the closing event names it.
    last: Option<Val>,
}

struct Events {
    /// The value, until the first event has been made from it.
    root: Option<Val>,
    stack: Vec<Frame>,
}

impl Events {
    fn new(root: Val) -> Self {
        Self {
            root: Some(root),
            stack: Vec::new(),
        }
    }
}

fn array(items: Vec<Val>) -> Val {
    items.into_iter().collect()
}

impl Iterator for Events {
    type Item = Val;

    fn next(&mut self) -> Option<Val> {
        if let Some(root) = self.root.take() {
            match Children::of(&root) {
                Some(children) => self.stack.push(Frame {
                    children,
                    next: 0,
                    path: Vec::new(),
                    last: None,
                }),
                None => return Some(array(vec![array(Vec::new()), root])),
            }
        }
        loop {
            let frame = self.stack.last_mut()?;
            if frame.next < frame.children.len() {
                let (key, value) = frame.children.get(frame.next);
                frame.next += 1;
                frame.last = Some(key.clone());
                let mut path = frame.path.clone();
                path.push(key);
                match Children::of(&value) {
                    Some(children) => self.stack.push(Frame {
                        children,
                        next: 0,
                        path,
                        last: None,
                    }),
                    None => return Some(array(vec![array(path), value])),
                }
            } else {
                // Every child is done: close the container.
                let done = self.stack.pop()?;
                let mut path = done.path;
                path.extend(done.last);
                return Some(array(vec![array(path)]));
            }
        }
    }
}
