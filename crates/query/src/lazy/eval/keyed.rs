//! What needs a key of every element of a list: `sort_by`, `group_by`,
//! `unique_by`, `min_by`, `max_by` and what jaq defines with them (`sort`,
//! `unique`, `min`, `max`), and `reverse`.
//!
//! A list that big cannot be handed to jaq, but what these make of it is made of
//! two things that can be held: the key of each element, which is what jaq makes
//! of the element with the function it is given, and where the element is in the
//! file. So the keys are made one element at a time (by threads, for a long list),
//! put in order or into groups as jaq would, and what comes out is the same list
//! in another order, or a list of lists: still in its file, to be read from where
//! it is as the stages after it ask for it.

use jsonquery_core::ValueKind;
use serde_json::Value;

use super::super::plan::{Keyed, KeyedOp, Pipeline};
use super::atom::Key;
use super::lazy::{Place, Span};
use super::{bytes_text, Emission, Evaluator, Failure, Flow, Item, Lazy, Sink};
use crate::to_val;

/// What a list was made into, or why it was not.
enum Made<'t> {
    Item(Item<'t>),
    Failed(Failure),
    Stopped,
}

impl Evaluator<'_> {
    pub(super) fn keyed<'t>(
        &self,
        p: &Pipeline,
        i: usize,
        keyed: &Keyed,
        lazy: Lazy<'t>,
        sink: Sink<'_, 't>,
    ) -> Flow {
        if lazy.kind() != ValueKind::Array {
            return sink(Emission::Error(format!(
                "cannot use {} as array",
                self.shown(&lazy)
            )));
        }
        let made = match keyed.op {
            KeyedOp::Reverse => {
                let mut places = lazy.places();
                places.reverse();
                Made::Item(Item::Lazy(lazy.reordered(places)))
            }
            KeyedOp::MinBy | KeyedOp::MaxBy => self.extreme(keyed, &lazy),
            KeyedOp::SortBy | KeyedOp::UniqueBy | KeyedOp::GroupBy => self.arranged(keyed, &lazy),
        };
        match made {
            Made::Item(item) => self.feed(p, i + 1, item, sink),
            Made::Failed(failure) => sink(failure.emission()),
            Made::Stopped => Flow::Stop,
        }
    }

    /// The element with the smallest (`min_by`) or the biggest key: of those
    /// whose key is the same, the first that is smallest and the last that is
    /// biggest, as in jaq. `null` for a list that has no elements. The keys are
    /// not kept: only the best so far.
    fn extreme<'t>(&self, keyed: &Keyed, lazy: &Lazy<'t>) -> Made<'t> {
        let Some(key) = &keyed.key else {
            return Made::Stopped;
        };
        let biggest = keyed.op == KeyedOp::MaxBy;
        let mut best: Option<(Key, Place)> = None;
        let mut failed = None;
        self.keys_of(key, lazy, &mut |place, result| match result {
            Err(failure) => {
                failed = Some(failure);
                Flow::Stop
            }
            Ok(key) => {
                let better = match &best {
                    None => true,
                    Some((best, _)) if biggest => key.as_slice() >= best.as_slice(),
                    Some((best, _)) => key.as_slice() < best.as_slice(),
                };
                if better {
                    best = Some((key, place));
                }
                Flow::Continue
            }
        });
        if let Some(failure) = failed {
            return Made::Failed(failure);
        }
        if self.stopped() {
            return Made::Stopped;
        }
        match best {
            None => Made::Item(Item::Value(Value::Null)),
            Some((_, place)) => match lazy.at_place(place) {
                Some(element) => Made::Item(Item::Lazy(element)),
                None => Made::Item(Item::Value(Value::Null)),
            },
        }
    }

    /// The list in the order of its keys (`sort_by`), or its elements that are
    /// the first of each key (`unique_by`), or its elements in groups of the
    /// same key (`group_by`): the order is the stable one, as in jaq.
    fn arranged<'t>(&self, keyed: &Keyed, lazy: &Lazy<'t>) -> Made<'t> {
        let Some(key) = &keyed.key else {
            return Made::Stopped;
        };
        if keyed.op == KeyedOp::GroupBy && lazy.is_groups() {
            return Made::Failed(Failure::Refusal(
                "a list of groups is not grouped again here: group its elements one at a \
                 time, with map(group_by(…))"
                    .to_owned(),
            ));
        }

        // Room for all of them at once, so that the list is never copied to grow; up to
        // what the limit would let be held.
        let room = self.limits.key_bytes / std::mem::size_of::<(Key, Place)>();
        let mut entries: Vec<(Key, Place)> = Vec::with_capacity(lazy.len().min(room));
        let mut bytes = 0usize;
        let mut failed = None;
        self.keys_of(key, lazy, &mut |place, result| match result {
            Err(failure) => {
                failed = Some(failure);
                Flow::Stop
            }
            Ok(key) => {
                bytes += key.bytes() + std::mem::size_of::<Place>();
                if bytes > self.limits.key_bytes {
                    failed = Some(Failure::Refusal(format!(
                        "{} is more than can be put in order here: the key of each of its \
                         elements takes more than {}; look at a part of it with a slice \
                         such as .[0:100000]",
                        self.describe(lazy),
                        bytes_text(self.limits.key_bytes)
                    )));
                    return Flow::Stop;
                }
                entries.push((key, place));
                Flow::Continue
            }
        });
        if let Some(failure) = failed {
            return Made::Failed(failure);
        }
        if self.stopped() {
            return Made::Stopped;
        }

        entries.sort_by(|a, b| a.0.as_slice().cmp(b.0.as_slice()));

        let places = |entries: Vec<(Key, Place)>| entries.into_iter().map(|(_, place)| place);
        match keyed.op {
            KeyedOp::SortBy => Made::Item(Item::Lazy(lazy.reordered(places(entries).collect()))),
            KeyedOp::UniqueBy => {
                let mut firsts = Vec::new();
                let mut last: Option<&Key> = None;
                for (key, place) in &entries {
                    if last.is_none_or(|last| last.as_slice() != key.as_slice()) {
                        firsts.push(*place);
                    }
                    last = Some(key);
                }
                Made::Item(Item::Lazy(lazy.reordered(firsts)))
            }
            _ => {
                let mut spans: Vec<Span> = Vec::with_capacity(entries.len());
                let mut ends = Vec::new();
                let mut last: Option<&Key> = None;
                for (key, place) in &entries {
                    if let Some(last) = last {
                        if last.as_slice() != key.as_slice() {
                            ends.push(spans.len());
                        }
                    }
                    last = Some(key);
                    if let Place::Node(span) = place {
                        spans.push(*span);
                    }
                }
                if !spans.is_empty() {
                    ends.push(spans.len());
                }
                Made::Item(Item::Lazy(Lazy::groups(lazy.node(), spans, ends)))
            }
        }
    }

    /// The key of each element of `lazy`, in the order of the list, handed to
    /// `each` with where the element is: the array of what `key` made of it, or
    /// the failure that was. The first few by this thread, the rest, for a long
    /// list, by threads.
    fn keys_of<'t>(
        &self,
        key: &Pipeline,
        lazy: &Lazy<'t>,
        each: &mut dyn FnMut(Place, Result<Key, Failure>) -> Flow,
    ) -> Flow {
        let mut at = 0usize;
        self.for_each_item(
            lazy,
            &|worker, item| worker.key_of(key, item),
            &mut |(span, result)| {
                let place = span.map_or(Place::Group(at), Place::Node);
                at += 1;
                each(place, result)
            },
        )
    }

    /// The key of one element, and where the element is.
    fn key_of<'t>(
        &self,
        key: &Pipeline,
        element: Lazy<'t>,
    ) -> (Option<Span>, Result<Key, Failure>) {
        let span = element.is_whole().then(|| Span::of(&element.node()));

        // An element that is small is given to jaq as it is, and what jaq makes of
        // it is taken as jaq makes it: not made a value, and read again.
        if let [stage] = key.stages.as_slice() {
            if self.is_small(&element) {
                let made = self
                    .materialize(&element, usize::MAX)
                    .map_err(Failure::Error)
                    .and_then(|value| {
                        let program = self.program(&stage.rest).map_err(Failure::Error)?;
                        let first = program.run_vals(to_val(&value), Vec::new()).next();
                        match first {
                            Some(Ok(made)) => Ok(Key::from_val(&made)),
                            Some(Err(e)) => Err(Failure::Error(e)),
                            // Stopped from outside, which is seen by whoever asked.
                            None => Err(Failure::Error(String::new())),
                        }
                    });
                return (span, made);
            }
        }

        let mut made = None;
        self.feed(key, 0, Item::Lazy(element), &mut |emission| {
            made = Some(match emission {
                Emission::Item(item) => self.part(item).map(|value| Key::from_value(&value)),
                Emission::Error(e) => Err(Failure::Error(e)),
                Emission::Refusal(e) => Err(Failure::Refusal(e)),
            });
            Flow::Stop
        });
        // Nothing made: it was stopped from outside, which is seen by whoever asked.
        (span, made.unwrap_or(Err(Failure::Error(String::new()))))
    }
}
