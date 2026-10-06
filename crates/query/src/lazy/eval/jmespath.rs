//! JMESPath over a document that is still in its file.
//!
//! The expression is read by the `jmespath` crate, as it is for any document, and
//! its syntax tree is walked here for as long as what it is run on is too big to
//! be a value: a field or an index picks a child, a slice a part of a list, a
//! projection runs what is after it on each element (by threads, for a long list),
//! a function of a big list that has a native way (`length`) is run that way. What
//! is small enough is given to the crate's own interpreter, with the part of the
//! tree that is left, so what each node does is what the crate does with it; only
//! the walking, which is the file's size, is done here.
//!
//! What cannot be done is a function that needs a whole big list as its argument
//! (`sort_by`, `max_by`, `sum`, …): it is an error that says what was too big.

use std::collections::BTreeMap;

use jmespath::ast::Ast;
use jmespath::{interpret, Context, Rcvar, DEFAULT_RUNTIME};
use jsonquery_core::engine::{QueryError, QueryEvent};
use jsonquery_core::lazy::LazyTree;
use jsonquery_core::ValueKind;
use serde_json::{Map, Value};
use std::sync::atomic::AtomicBool;

use super::lazy::Place;
use super::{approximate_size, bytes_text, Evaluator, Failure, Flow, Item, Lazy, Limits};
use crate::jmespath_engine::value_to_variable;

/// An expression, read.
struct Plan {
    /// What was typed, which the interpreter quotes in its errors.
    text: String,
    ast: Ast,
}

/// What a node of the expression makes: a value that is still in the file, or one
/// in memory.
enum Got<'t> {
    Lazy(Lazy<'t>),
    Value(Value),
}

impl Evaluator<'_> {
    /// `ast` run on `got`.
    fn jmes<'t>(&self, plan: &Plan, ast: &Ast, got: Got<'t>) -> Result<Got<'t>, Failure> {
        if self.stopped() {
            return Err(Failure::Error(String::new()));
        }
        let lazy = match got {
            Got::Value(value) => return self.jmes_value(plan, ast, &value).map(Got::Value),
            Got::Lazy(lazy) => lazy,
        };
        if self.is_small(&lazy) {
            let value = self
                .materialize(&lazy, usize::MAX)
                .map_err(Failure::Error)?;
            return self.jmes_value(plan, ast, &value).map(Got::Value);
        }

        let kind = lazy.kind();
        match ast {
            Ast::Identity { .. } => Ok(Got::Lazy(lazy)),
            Ast::Field { name, .. } => Ok(match lazy.node().child_by_key(name) {
                Some(child) if kind == ValueKind::Object && lazy.is_whole() => {
                    Got::Lazy(Lazy::of(child.node))
                }
                _ => Got::Value(Value::Null),
            }),
            Ast::Index { idx, .. } => {
                if kind != ValueKind::Array {
                    return Ok(Got::Value(Value::Null));
                }
                let len = lazy.len() as i64;
                let at = if *idx >= 0 {
                    i64::from(*idx)
                } else {
                    len + i64::from(*idx)
                };
                Ok(
                    match (0..len)
                        .contains(&at)
                        .then(|| lazy.item(at as usize))
                        .flatten()
                    {
                        Some(item) => Got::Lazy(item),
                        None => Got::Value(Value::Null),
                    },
                )
            }
            Ast::Subexpr { lhs, rhs, .. } => {
                let left = self.jmes(plan, lhs, Got::Lazy(lazy))?;
                self.jmes(plan, rhs, left)
            }
            Ast::Slice {
                start, stop, step, ..
            } => {
                if *step == 0 {
                    // The interpreter's own error for it, whatever it is run on.
                    return self.jmes_value(plan, ast, &Value::Null).map(Got::Value);
                }
                if kind != ValueKind::Array {
                    return Ok(Got::Value(Value::Null));
                }
                self.jmes_slice(&lazy, *start, *stop, *step)
            }
            Ast::Projection { lhs, rhs, .. } => {
                let left = self.jmes(plan, lhs, Got::Lazy(lazy))?;
                self.jmes_project(plan, rhs, left)
            }
            Ast::Flatten { node, .. } => {
                let inner = self.jmes(plan, node, Got::Lazy(lazy))?;
                self.jmes_flatten(inner)
            }
            Ast::ObjectValues { node, .. } => {
                let inner = self.jmes(plan, node, Got::Lazy(lazy))?;
                match inner {
                    // The interpreter keeps the members of an object by their keys.
                    Got::Lazy(inner) if inner.kind() == ValueKind::Object => {
                        let mut made = BTreeMap::new();
                        for child in inner.node().children() {
                            let key = child.key.map(|k| k.to_string()).unwrap_or_default();
                            let value = self.got_value(Got::Lazy(Lazy::of(child.node)))?;
                            made.insert(key, value);
                        }
                        Ok(Got::Value(Value::Array(made.into_values().collect())))
                    }
                    Got::Value(Value::Object(map)) => {
                        let made: BTreeMap<String, Value> = map.into_iter().collect();
                        Ok(Got::Value(Value::Array(made.into_values().collect())))
                    }
                    _ => Ok(Got::Value(Value::Null)),
                }
            }
            Ast::MultiList { elements, .. } => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    let made = self.jmes(plan, element, Got::Lazy(lazy.clone()))?;
                    values.push(self.got_value(made)?);
                }
                Ok(Got::Value(Value::Array(values)))
            }
            Ast::MultiHash { elements, .. } => {
                // The interpreter keeps the keys in order, with the last of a key.
                let mut made = BTreeMap::new();
                for pair in elements {
                    let value = self.jmes(plan, &pair.value, Got::Lazy(lazy.clone()))?;
                    made.insert(pair.key.clone(), self.got_value(value)?);
                }
                Ok(Got::Value(Value::Object(Map::from_iter(made))))
            }
            Ast::Or { lhs, rhs, .. } => {
                let left = self.jmes(plan, lhs, Got::Lazy(lazy.clone()))?;
                if self.truthy(&left) {
                    Ok(left)
                } else {
                    self.jmes(plan, rhs, Got::Lazy(lazy))
                }
            }
            Ast::And { lhs, rhs, .. } => {
                let left = self.jmes(plan, lhs, Got::Lazy(lazy.clone()))?;
                if !self.truthy(&left) {
                    Ok(left)
                } else {
                    self.jmes(plan, rhs, Got::Lazy(lazy))
                }
            }
            Ast::Not { node, .. } => {
                let inner = self.jmes(plan, node, Got::Lazy(lazy))?;
                Ok(Got::Value(Value::Bool(!self.truthy(&inner))))
            }
            Ast::Condition {
                predicate, then, ..
            } => {
                let test = self.jmes(plan, predicate, Got::Lazy(lazy.clone()))?;
                if self.truthy(&test) {
                    self.jmes(plan, then, Got::Lazy(lazy))
                } else {
                    Ok(Got::Value(Value::Null))
                }
            }
            Ast::Comparison {
                offset,
                comparator,
                lhs,
                rhs,
            } => {
                let left = self.jmes(plan, lhs, Got::Lazy(lazy.clone()))?;
                let left = self.got_value(left)?;
                let right = self.jmes(plan, rhs, Got::Lazy(lazy))?;
                let right = self.got_value(right)?;
                let comparison = Ast::Comparison {
                    offset: *offset,
                    comparator: comparator.clone(),
                    lhs: Box::new(literal(&left)),
                    rhs: Box::new(literal(&right)),
                };
                self.jmes_value(plan, &comparison, &Value::Null)
                    .map(Got::Value)
            }
            Ast::Literal { value, .. } => Ok(Got::Value(
                serde_json::to_value(&**value).map_err(|e| Failure::Error(e.to_string()))?,
            )),
            Ast::Function {
                offset, name, args, ..
            } => {
                // What its arguments make, each in its place: an expression that
                // the function runs itself is left as it is.
                let mut made = Vec::with_capacity(args.len());
                for arg in args {
                    if matches!(arg, Ast::Expref { .. }) {
                        made.push(arg.clone());
                        continue;
                    }
                    let value = self.jmes(plan, arg, Got::Lazy(lazy.clone()))?;
                    // The length of a list is not made by making it a value.
                    if let (true, 1, Got::Lazy(list)) = (name == "length", args.len(), &value) {
                        match list.kind() {
                            ValueKind::Array | ValueKind::Object => {
                                return Ok(Got::Value(Value::from(list.len())));
                            }
                            ValueKind::String => {
                                return Ok(Got::Value(Value::from(
                                    list.node().string().map_or(0, |s| s.chars().count()),
                                )));
                            }
                            // Anything else is small, and the interpreter's to say.
                            _ => {}
                        }
                    }
                    made.push(literal(&self.got_value(value)?));
                }
                let call = Ast::Function {
                    offset: *offset,
                    name: name.clone(),
                    args: made,
                };
                self.jmes_value(plan, &call, &Value::Null).map(Got::Value)
            }
            Ast::Expref { .. } => Err(Failure::Refusal(
                "a function of an expression is not run on this document: it needs all of it"
                    .to_owned(),
            )),
        }
    }

    /// `ast` run by the interpreter on a value.
    fn jmes_value(&self, plan: &Plan, ast: &Ast, value: &Value) -> Result<Value, Failure> {
        let data = Rcvar::new(value_to_variable(value));
        let mut context = Context::new(&plan.text, &DEFAULT_RUNTIME);
        let made =
            interpret(&data, ast, &mut context).map_err(|e| Failure::Error(e.to_string()))?;
        serde_json::to_value(&*made).map_err(|e| Failure::Error(e.to_string()))
    }

    /// What a node of the expression made, as a value: if it is not too big.
    fn got_value(&self, got: Got<'_>) -> Result<Value, Failure> {
        match got {
            Got::Value(value) => Ok(value),
            Got::Lazy(lazy) => self.part(Item::Lazy(lazy)),
        }
    }

    /// Whether the interpreter would take it for true.
    fn truthy(&self, got: &Got<'_>) -> bool {
        match got {
            Got::Value(value) => match value {
                Value::Bool(b) => *b,
                Value::String(s) => !s.is_empty(),
                Value::Array(a) => !a.is_empty(),
                Value::Object(o) => !o.is_empty(),
                Value::Number(_) => true,
                Value::Null => false,
            },
            Got::Lazy(lazy) => match lazy.kind() {
                ValueKind::Array | ValueKind::Object => lazy.len() > 0,
                ValueKind::String => lazy.node().byte_len() > 2,
                ValueKind::Number => true,
                ValueKind::Bool => lazy.node().raw() == b"true",
                ValueKind::Null => false,
            },
        }
    }

    /// `array[start:stop:step]` of a list that is too big to be a value, as the
    /// interpreter makes it.
    fn jmes_slice<'t>(
        &self,
        lazy: &Lazy<'t>,
        start: Option<i32>,
        stop: Option<i32>,
        step: i32,
    ) -> Result<Got<'t>, Failure> {
        let len = i32::try_from(lazy.len()).map_err(|_| {
            Failure::Refusal(format!(
                "{} is too long to be sliced here",
                self.describe(lazy)
            ))
        })?;
        let endpoint = |at: i32| {
            if at < 0 {
                let at = at + len;
                if at >= 0 {
                    at
                } else if step < 0 {
                    -1
                } else {
                    0
                }
            } else if at < len {
                at
            } else if step < 0 {
                len - 1
            } else {
                len
            }
        };
        let a = start.map_or(if step < 0 { len - 1 } else { 0 }, endpoint);
        let b = stop.map_or(if step < 0 { -1 } else { len }, endpoint);
        if step == 1 {
            return Ok(Got::Lazy(if a < b {
                lazy.slice(a as usize, b as usize)
            } else {
                lazy.slice(0, 0)
            }));
        }
        let mut places = Vec::new();
        let mut at = a;
        while (step > 0 && at < b) || (step < 0 && at > b) {
            if let Some(item) = lazy.item(at as usize) {
                places.push(Place::Node(super::lazy::Span::of(&item.node())));
            }
            at += step;
        }
        Ok(Got::Lazy(lazy.reordered(places)))
    }

    /// `rhs` run on each element of the list `left` is, and what it makes, the
    /// nulls left out.
    fn jmes_project<'t>(&self, plan: &Plan, rhs: &Ast, left: Got<'t>) -> Result<Got<'t>, Failure> {
        match left {
            Got::Value(Value::Array(items)) => {
                let mut kept = Vec::new();
                for item in items {
                    let made = self.jmes(plan, rhs, Got::Value(item))?;
                    let value = self.got_value(made)?;
                    if !value.is_null() {
                        kept.push(value);
                    }
                }
                Ok(Got::Value(Value::Array(kept)))
            }
            Got::Lazy(list) if list.kind() == ValueKind::Array => {
                let mut kept = Vec::new();
                let mut bytes = 0usize;
                let mut failed = None;
                self.for_each_item(
                    &list,
                    &|worker, item| -> Result<Value, Failure> {
                        let made = worker.jmes(plan, rhs, Got::Lazy(item))?;
                        worker.got_value(made)
                    },
                    &mut |made| match made {
                        Ok(value) if value.is_null() => Flow::Continue,
                        Ok(value) => {
                            bytes += approximate_size(&value);
                            if bytes > self.limits.collect_bytes {
                                failed = Some(Failure::Refusal(format!(
                                    "what this gathers is over {} — too much to hold in memory; \
                                     narrow it",
                                    bytes_text(self.limits.collect_bytes)
                                )));
                                return Flow::Stop;
                            }
                            kept.push(value);
                            Flow::Continue
                        }
                        Err(failure) => {
                            failed = Some(failure);
                            Flow::Stop
                        }
                    },
                );
                match failed {
                    Some(failure) => Err(failure),
                    None if self.stopped() => Err(Failure::Error(String::new())),
                    None => Ok(Got::Value(Value::Array(kept))),
                }
            }
            _ => Ok(Got::Value(Value::Null)),
        }
    }

    /// A list flattened by one level: the elements that are lists have their
    /// elements in their place.
    fn jmes_flatten<'t>(&self, inner: Got<'t>) -> Result<Got<'t>, Failure> {
        let flat = |items: Vec<Value>| {
            let mut kept = Vec::new();
            for item in items {
                match item {
                    Value::Array(inside) => kept.extend(inside),
                    other => kept.push(other),
                }
            }
            Value::Array(kept)
        };
        match inner {
            Got::Value(Value::Array(items)) => Ok(Got::Value(flat(items))),
            Got::Lazy(list) if list.kind() == ValueKind::Array => {
                let all =
                    self.jmes_gather(&list, &|worker, child| worker.got_value(Got::Lazy(child)))?;
                match all {
                    Got::Value(Value::Array(items)) => Ok(Got::Value(flat(items))),
                    other => Ok(other),
                }
            }
            _ => Ok(Got::Value(Value::Null)),
        }
    }

    /// The elements of `list`, each made a value by `work`, in a list.
    fn jmes_gather<'t>(
        &self,
        list: &Lazy<'t>,
        work: &(dyn Fn(&Evaluator<'_>, Lazy<'t>) -> Result<Value, Failure> + Sync),
    ) -> Result<Got<'t>, Failure> {
        let mut kept = Vec::new();
        let mut bytes = 0usize;
        let mut failed = None;
        self.for_each_item(list, work, &mut |made| match made {
            Ok(value) => {
                bytes += approximate_size(&value);
                if bytes > self.limits.collect_bytes {
                    failed = Some(Failure::Refusal(format!(
                        "what this gathers is over {} — too much to hold in memory; narrow it",
                        bytes_text(self.limits.collect_bytes)
                    )));
                    return Flow::Stop;
                }
                kept.push(value);
                Flow::Continue
            }
            Err(failure) => {
                failed = Some(failure);
                Flow::Stop
            }
        });
        match failed {
            Some(failure) => Err(failure),
            None => Ok(Got::Value(Value::Array(kept))),
        }
    }
}

/// A value, as a literal of an expression.
fn literal(value: &Value) -> Ast {
    Ast::Literal {
        offset: 0,
        value: Rcvar::new(value_to_variable(value)),
    }
}

/// Run the JMESPath expression `src` over the document in `tree`, handing the one
/// value it makes to `on_event`.
pub(in crate::lazy) fn run_jmespath(
    tree: &LazyTree,
    src: &str,
    limits: Limits,
    cancelled: &AtomicBool,
    on_event: &mut dyn FnMut(QueryEvent),
) -> Result<usize, QueryError> {
    let expression = jmespath::compile(src).map_err(|e| QueryError::Parse(e.to_string()))?;
    let plan = Plan {
        text: src.to_owned(),
        ast: expression.as_ast().clone(),
    };
    let evaluator = Evaluator::new(cancelled, limits);
    let made = evaluator.jmes(&plan, &plan.ast, Got::Lazy(Lazy::of(tree.root())));
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(0);
    }
    let failure = |failure: Failure| match failure {
        Failure::Error(e) | Failure::Refusal(e) => QueryError::Engine(e),
    };
    let value = match made.map_err(failure)? {
        Got::Value(value) => value,
        Got::Lazy(lazy) => {
            if !evaluator.fits(&lazy, limits.result_bytes) {
                return Err(QueryError::Engine(format!(
                    "{} is too big to show (over {}): narrow it with a slice such as [0:100], \
                     or look at it in the Source tree",
                    evaluator.describe(&lazy),
                    bytes_text(limits.result_bytes)
                )));
            }
            evaluator
                .materialize(&lazy, usize::MAX)
                .map_err(QueryError::Engine)?
        }
    };
    // The members of an object come out by their keys, as the interpreter has them.
    let value = serde_json::to_value(value_to_variable(&value))
        .map_err(|e| QueryError::Engine(e.to_string()))?;
    on_event(QueryEvent::Item(value));
    Ok(1)
}
