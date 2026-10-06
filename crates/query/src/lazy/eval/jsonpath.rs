//! JSONPath over a document that is still in its file.
//!
//! The query is read by `jsonpath-rust`, as it is for any document, and what it
//! reads — a list of segments, each of selectors — is walked here for as long as
//! the node it is run on is too big to be a value: a name, an index, a wildcard or
//! a slice picks children of it, a filter tests each, a descendant segment goes
//! down through all of them. What is small enough is handed to `jsonpath-rust`
//! with the rest of the query, which is run on it as on a value of its own. So
//! what each selector does is what `jsonpath-rust` does with it; only the
//! walking, which is the file's size, is done here, in the order in which it
//! does it.
//!
//! A query is run on a part of the document as if it were the root, so one that
//! reads the root from inside a filter (`$.a[?(@.n == $.m)]`) cannot be.

use std::collections::HashMap;

use jsonpath_rust::parser::model::{JpQuery, Segment, Selector};
use jsonpath_rust::query::js_path_process;
use jsonquery_core::engine::{QueryError, QueryEvent};
use jsonquery_core::lazy::LazyTree;
use jsonquery_core::ValueKind;
use serde_json::{Map, Value};

use super::{Evaluator, Failure, Flow, Item, Lazy, Limits};
use std::sync::atomic::AtomicBool;

/// A JSONPath query, read, with the queries that its segments are run as on what
/// is small enough to be a value.
struct Plan {
    segments: Vec<Segment>,
    /// The segments from the `i`th on, as a query of their own.
    from: Vec<JpQuery>,
    /// The `k`th selector of the `i`th segment, as a child selector followed by
    /// the segments after it: what a filter is run as on the child of a node that
    /// is too big, on a list that has only that child.
    selected: HashMap<(usize, usize), JpQuery>,
    /// The same, for a descendant segment: what the `k`th selector of the `i`th is
    /// run as on a descendant that is small enough.
    below: HashMap<(usize, usize), JpQuery>,
}

/// The selectors of a segment, and whether it is a descendant one.
fn selectors_of(segment: &Segment) -> (Vec<&Selector>, bool) {
    match segment {
        Segment::Selector(selector) => (vec![selector], false),
        Segment::Selectors(selectors) => (selectors.iter().collect(), false),
        Segment::Descendant(inner) => (selectors_of(inner).0, true),
    }
}

impl Plan {
    fn new(query: JpQuery) -> Self {
        let segments = query.segments;
        let from = (0..=segments.len())
            .map(|i| JpQuery::new(segments[i..].to_vec()))
            .collect();
        let (mut selected, mut below) = (HashMap::new(), HashMap::new());
        for (i, segment) in segments.iter().enumerate() {
            let (selectors, descendant) = selectors_of(segment);
            for (k, selector) in selectors.into_iter().enumerate() {
                let after = segments[i + 1..].iter().cloned();
                let single = |first: Segment| {
                    JpQuery::new(std::iter::once(first).chain(after.clone()).collect())
                };
                selected.insert((i, k), single(Segment::Selector(selector.clone())));
                if descendant {
                    below.insert(
                        (i, k),
                        single(Segment::Descendant(Box::new(Segment::Selector(
                            selector.clone(),
                        )))),
                    );
                }
            }
        }
        Self {
            segments,
            from,
            selected,
            below,
        }
    }

    fn has_filter(&self) -> bool {
        self.segments.iter().any(|s| {
            selectors_of(s)
                .0
                .iter()
                .any(|s| matches!(s, Selector::Filter(_)))
        })
    }
}

/// What a walk of a node hands on: a value that matched, or why it could not
/// be told.
type Found = Result<Value, Failure>;

impl Evaluator<'_> {
    /// The query `query` on the document, a match at a time.
    fn jsonpath<'t>(
        &self,
        plan: &Plan,
        i: usize,
        lazy: Lazy<'t>,
        emit: &mut dyn FnMut(Found) -> Flow,
    ) -> Flow {
        if self.stopped() {
            return Flow::Stop;
        }
        if i == plan.segments.len() {
            return emit(self.part(Item::Lazy(lazy)));
        }
        if self.is_small(&lazy) {
            return match self.materialize(&lazy, usize::MAX) {
                Ok(value) => self.jsonpath_on(&plan.from[i], &value, emit),
                Err(e) => emit(Err(Failure::Error(e))),
            };
        }
        let (selectors, descendant) = selectors_of(&plan.segments[i]);
        for (k, selector) in selectors.into_iter().enumerate() {
            let flow = if descendant {
                self.jsonpath_below(plan, i, k, selector, &lazy, emit)
            } else {
                self.jsonpath_select(plan, i, k, selector, &lazy, emit)
            };
            if flow == Flow::Stop {
                return Flow::Stop;
            }
        }
        Flow::Continue
    }

    /// `query` run on `value` as on a document of its own.
    fn jsonpath_on(
        &self,
        query: &JpQuery,
        value: &Value,
        emit: &mut dyn FnMut(Found) -> Flow,
    ) -> Flow {
        match js_path_process(query, value) {
            Ok(found) => {
                for one in found {
                    if self.stopped() || emit(Ok(one.val.clone())) == Flow::Stop {
                        return Flow::Stop;
                    }
                }
                Flow::Continue
            }
            Err(e) => emit(Err(Failure::Error(e.to_string()))),
        }
    }

    /// The selector `selector` (the `k`th of segment `i`) on a node that is too big
    /// to be a value, and the rest of the query on each child it picks.
    fn jsonpath_select<'t>(
        &self,
        plan: &Plan,
        i: usize,
        k: usize,
        selector: &Selector,
        lazy: &Lazy<'t>,
        emit: &mut dyn FnMut(Found) -> Flow,
    ) -> Flow {
        let kind = lazy.kind();
        if !kind.is_container() {
            return Flow::Continue;
        }
        match selector {
            Selector::Name(key) => match lazy.node().child_by_key(key) {
                Some(child) if kind == ValueKind::Object => {
                    self.jsonpath(plan, i + 1, Lazy::of(child.node), emit)
                }
                _ => Flow::Continue,
            },
            Selector::Index(index) => {
                if kind != ValueKind::Array {
                    return Flow::Continue;
                }
                let len = lazy.len() as i64;
                let at = if *index >= 0 { *index } else { len + *index };
                match (0..len)
                    .contains(&at)
                    .then(|| lazy.item(at as usize))
                    .flatten()
                {
                    Some(child) => self.jsonpath(plan, i + 1, child, emit),
                    None => Flow::Continue,
                }
            }
            Selector::Wildcard => self.jsonpath_each(plan, i, lazy, emit),
            Selector::Slice(start, end, step) => {
                if kind != ValueKind::Array {
                    return Flow::Continue;
                }
                let len = lazy.len() as i64;
                let norm = |at: i64| if at >= 0 { at } else { len + at };
                let step = step.unwrap_or(1);
                if step > 0 {
                    let lower = norm(start.unwrap_or(0)).clamp(0, len);
                    let upper = norm(end.unwrap_or(len)).clamp(0, len);
                    if lower >= upper {
                        return Flow::Continue;
                    }
                    if step == 1 {
                        let window = lazy.slice(lower as usize, upper as usize);
                        return self.jsonpath_each(plan, i, &window, emit);
                    }
                    let mut at = lower;
                    while at < upper {
                        if let Some(child) = lazy.item(at as usize) {
                            if self.jsonpath(plan, i + 1, child, emit) == Flow::Stop {
                                return Flow::Stop;
                            }
                        }
                        at += step;
                    }
                } else if step < 0 {
                    let lower = norm(end.unwrap_or(-len - 1)).clamp(-1, len - 1);
                    let mut at = norm(start.unwrap_or(len - 1)).clamp(-1, len - 1);
                    while lower < at {
                        if let Some(child) = lazy.item(at as usize) {
                            if self.jsonpath(plan, i + 1, child, emit) == Flow::Stop {
                                return Flow::Stop;
                            }
                        }
                        at += step;
                    }
                }
                Flow::Continue
            }
            Selector::Filter(_) => {
                let query = &plan.selected[&(i, k)];
                let is_object = kind == ValueKind::Object;
                self.for_each_item_flat(
                    lazy,
                    &|worker, child| worker.jsonpath_filtered(query, is_object, child),
                    emit,
                )
            }
        }
    }

    /// Each child of `lazy`, and the rest of the query on it, in order.
    fn jsonpath_each<'t>(
        &self,
        plan: &Plan,
        i: usize,
        lazy: &Lazy<'t>,
        emit: &mut dyn FnMut(Found) -> Flow,
    ) -> Flow {
        self.for_each_item_flat(
            lazy,
            &|worker, child| {
                let mut found = Vec::new();
                worker.jsonpath(plan, i + 1, child, &mut |one| {
                    found.push(one);
                    Flow::Continue
                });
                found
            },
            emit,
        )
    }

    /// A filter on one child of a node: the child is put in a list (or an
    /// object, for the child of one) of its own, which the filter and the rest of
    /// the query are run on.
    fn jsonpath_filtered(&self, query: &JpQuery, in_object: bool, child: Lazy<'_>) -> Vec<Found> {
        if !self.is_small(&child) {
            return vec![Err(Failure::Refusal(format!(
                "a filter is not run on {}: it is too big to be tested",
                self.describe(&child)
            )))];
        }
        let value = match self.materialize(&child, usize::MAX) {
            Ok(value) => value,
            Err(e) => return vec![Err(Failure::Error(e))],
        };
        let alone = if in_object {
            let mut map = Map::new();
            map.insert("k".to_owned(), value);
            Value::Object(map)
        } else {
            Value::Array(vec![value])
        };
        let mut found = Vec::new();
        self.jsonpath_on(query, &alone, &mut |one| {
            found.push(one);
            Flow::Continue
        });
        found
    }

    /// The selector `selector` of a descendant segment: on the node, and then on
    /// each node below it, in the order in which they are met.
    fn jsonpath_below<'t>(
        &self,
        plan: &Plan,
        i: usize,
        k: usize,
        selector: &Selector,
        lazy: &Lazy<'t>,
        emit: &mut dyn FnMut(Found) -> Flow,
    ) -> Flow {
        if self.jsonpath_select(plan, i, k, selector, lazy, emit) == Flow::Stop {
            return Flow::Stop;
        }
        if !lazy.kind().is_container() {
            return Flow::Continue;
        }
        let query = &plan.below[&(i, k)];
        self.for_each_item_flat(
            lazy,
            &|worker, child| {
                let mut found = Vec::new();
                let mut keep = |one| {
                    found.push(one);
                    Flow::Continue
                };
                if worker.is_small(&child) {
                    match worker.materialize(&child, usize::MAX) {
                        Ok(value) => {
                            worker.jsonpath_on(query, &value, &mut keep);
                        }
                        Err(e) => {
                            keep(Err(Failure::Error(e)));
                        }
                    }
                } else {
                    worker.jsonpath_below(plan, i, k, selector, &child, &mut keep);
                }
                found
            },
            emit,
        )
    }

    /// `work` on each element of `lazy`, and what it found, in the order of the
    /// list, handed to `emit`.
    fn for_each_item_flat<'t>(
        &self,
        lazy: &Lazy<'t>,
        work: &(dyn Fn(&Evaluator<'_>, Lazy<'t>) -> Vec<Found> + Sync),
        emit: &mut dyn FnMut(Found) -> Flow,
    ) -> Flow {
        self.for_each_item(lazy, work, &mut |found: Vec<Found>| {
            for one in found {
                if emit(one) == Flow::Stop {
                    return Flow::Stop;
                }
            }
            Flow::Continue
        })
    }
}

/// Run the JSONPath query `src` over the document in `tree`, handing the matches
/// to `on_event` as they are found.
pub(in crate::lazy) fn run_jsonpath(
    tree: &LazyTree,
    src: &str,
    limits: Limits,
    cancelled: &AtomicBool,
    on_event: &mut dyn FnMut(QueryEvent),
) -> Result<usize, QueryError> {
    let query = jsonpath_rust::parser::parse_json_path(src)
        .map_err(|e| QueryError::Parse(e.to_string()))?;
    let plan = Plan::new(query);
    // A filter that reads the document from its root is run on a part of it.
    if plan.has_filter()
        && src
            .trim_start()
            .get(1..)
            .is_some_and(|rest| rest.contains('$'))
    {
        return Err(QueryError::Engine(
            "a JSONPath filter that reads the document from its root ($) is not run on a \
             document this big: use jq"
                .to_owned(),
        ));
    }
    let evaluator = Evaluator::new(cancelled, limits);
    let mut count = 0usize;
    evaluator.jsonpath(&plan, 0, Lazy::of(tree.root()), &mut |found| {
        count += 1;
        on_event(match found {
            Ok(value) => QueryEvent::Item(value),
            Err(Failure::Error(e) | Failure::Refusal(e)) => QueryEvent::ItemError(e),
        });
        Flow::Continue
    });
    Ok(count)
}
