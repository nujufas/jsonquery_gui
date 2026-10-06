//! The stages that are walked: what each does with a node that is too big to be
//! given to jaq.

use serde_json::Value;

use jsonquery_core::ValueKind;

use super::super::plan::{Kind, PathStep, Pipeline, Stage, Step};
use super::{
    approximate_size, bytes_text, shorten, type_name, Emission, Evaluator, Failure, Flow, Item,
    Lazy, Sink,
};

impl Evaluator<'_> {
    /// The stage `i` of `p`, on a node that is too big to be given to jaq.
    pub(super) fn walk_stage<'t>(
        &self,
        p: &Pipeline,
        i: usize,
        stage: &Stage,
        lazy: Lazy<'t>,
        sink: Sink<'_, 't>,
    ) -> Flow {
        match &stage.kind {
            Kind::Identity => self.feed(p, i + 1, Item::Lazy(lazy), sink),
            Kind::Path(steps) => {
                let then = |eval: &Evaluator<'_>, item: Item<'t>, out: Sink<'_, 't>| {
                    eval.feed(p, i + 1, item, out)
                };
                self.path(steps, lazy, &then, sink)
            }
            Kind::Length => match self.length(&lazy) {
                Ok(n) => self.feed(p, i + 1, Item::Value(n), sink),
                Err(e) => sink(Emission::Error(e)),
            },
            Kind::Keys { sorted } => match self.keys(&lazy, *sorted) {
                Ok(keys) => self.feed(p, i + 1, Item::Value(keys), sink),
                Err(failure) => sink(failure.emission()),
            },
            Kind::Collect(inner) => {
                let mut gathered = Vec::new();
                let mut bytes = 0usize;
                let mut failure = None;
                let flow = self.feed(inner, 0, Item::Lazy(lazy), &mut |emission| {
                    let value = match emission {
                        Emission::Item(item) => self.to_value(item).map_err(Failure::Refusal),
                        Emission::Error(e) => Err(Failure::Error(e)),
                        Emission::Refusal(e) => Err(Failure::Refusal(e)),
                    };
                    match value {
                        Ok(value) => {
                            bytes += approximate_size(&value);
                            if bytes > self.limits.collect_bytes {
                                failure = Some(Failure::Refusal(format!(
                                    "what this gathers is over {} — too much to hold in memory; \
                                     narrow it, or run it over one part of the file at a time",
                                    bytes_text(self.limits.collect_bytes)
                                )));
                                return Flow::Stop;
                            }
                            gathered.push(value);
                            Flow::Continue
                        }
                        Err(e) => {
                            failure = Some(e);
                            Flow::Stop
                        }
                    }
                });
                match failure {
                    Some(failure) => sink(failure.emission()),
                    // Stopped from outside.
                    None if flow == Flow::Stop => Flow::Stop,
                    None => self.feed(p, i + 1, Item::Value(Value::Array(gathered)), sink),
                }
            }
            Kind::Limit { count, inner } => {
                // What comes out counts, an error as much as a value, as in jaq.
                let mut taken = 0usize;
                let mut stopped = false;
                self.feed(inner, 0, Item::Lazy(lazy), &mut |emission| {
                    taken += 1;
                    let flow = match emission {
                        Emission::Item(item) => self.feed(p, i + 1, item, sink),
                        other => sink(other),
                    };
                    if flow == Flow::Stop {
                        stopped = true;
                        return Flow::Stop;
                    }
                    if taken >= *count {
                        Flow::Stop
                    } else {
                        Flow::Continue
                    }
                });
                // Having taken what it wanted is not being stopped.
                if stopped || self.stopped() {
                    Flow::Stop
                } else {
                    Flow::Continue
                }
            }
            Kind::Each(pipelines) => {
                for inner in pipelines {
                    let flow =
                        self.feed(
                            inner,
                            0,
                            Item::Lazy(lazy.clone()),
                            &mut |emission| match emission {
                                Emission::Item(item) => self.feed(p, i + 1, item, sink),
                                other => sink(other),
                            },
                        );
                    if flow == Flow::Stop {
                        return Flow::Stop;
                    }
                }
                Flow::Continue
            }
            Kind::Type => self.feed(
                p,
                i + 1,
                Item::Value(Value::String(type_name(lazy.kind()).to_owned())),
                sink,
            ),
            Kind::OfType(types) => {
                if types.has(lazy.kind()) {
                    self.feed(p, i + 1, Item::Lazy(lazy), sink)
                } else {
                    Flow::Continue
                }
            }
            Kind::Select(condition) => {
                let mut stopped = false;
                self.feed(condition, 0, Item::Lazy(lazy.clone()), &mut |emission| {
                    let flow = match emission {
                        Emission::Item(item) if truthy(&item) => {
                            self.feed(p, i + 1, Item::Lazy(lazy.clone()), sink)
                        }
                        Emission::Item(_) => Flow::Continue,
                        other => sink(other),
                    };
                    stopped |= flow == Flow::Stop;
                    flow
                });
                if stopped || self.stopped() {
                    Flow::Stop
                } else {
                    Flow::Continue
                }
            }
            Kind::Try(inner) => {
                let mut stopped = false;
                self.feed(inner, 0, Item::Lazy(lazy), &mut |emission| match emission {
                    Emission::Item(item) => {
                        let flow = self.feed(p, i + 1, item, sink);
                        stopped |= flow == Flow::Stop;
                        flow
                    }
                    // The first error ends it, and is no output.
                    Emission::Error(_) => Flow::Stop,
                    // A refusal is not an error of the program: it is told.
                    refusal @ Emission::Refusal(_) => {
                        stopped = true;
                        sink(refusal);
                        Flow::Stop
                    }
                });
                if stopped || self.stopped() {
                    Flow::Stop
                } else {
                    Flow::Continue
                }
            }
            Kind::Recurse => {
                // `..` is `., (.[]? | ..)`: the node itself, then, for each of
                // its children, the same.
                if self.feed(p, i + 1, Item::Lazy(lazy.clone()), sink) == Flow::Stop {
                    return Flow::Stop;
                }
                if !lazy.kind().is_container() {
                    return Flow::Continue;
                }
                let below = [PathStep {
                    step: Step::Each,
                    optional: true,
                }];
                let then = |eval: &Evaluator<'_>, item: Item<'t>, out: Sink<'_, 't>| {
                    eval.feed(p, i, item, out)
                };
                self.path(&below, lazy, &then, sink)
            }
            Kind::Compose(compose) => self.compose(p, i, compose, lazy, sink),
            Kind::Keyed(keyed) => self.keyed(p, i, keyed, lazy, sink),
            Kind::Opaque => sink(Emission::Refusal(format!(
                "`{}` needs all of {} in memory, which is too much for this: look at a part \
                 of it with a slice such as .[0:100], or work on one element at a time with \
                 .[] | …, map(…), […] or first(…)",
                shorten(&stage.rest, 60),
                self.describe(&lazy)
            ))),
        }
    }

    fn length(&self, lazy: &Lazy<'_>) -> Result<Value, String> {
        match lazy.kind() {
            ValueKind::Array | ValueKind::Object => Ok(Value::from(lazy.len())),
            ValueKind::String => Ok(Value::from(
                lazy.node().string().map_or(0, |s| s.chars().count()),
            )),
            ValueKind::Null => Ok(Value::from(0)),
            _ => Err(format!("{} has no length", self.shown(lazy))),
        }
    }

    fn keys(&self, lazy: &Lazy<'_>, sorted: bool) -> Result<Value, Failure> {
        match lazy.kind() {
            ValueKind::Object => {
                let mut keys: Vec<String> = Vec::new();
                let mut seen = std::collections::HashSet::new();
                for child in lazy.node().children() {
                    let key = child.key.map(|k| k.to_string()).unwrap_or_default();
                    // A key that is there twice is one key, as in a parsed object.
                    if seen.insert(key.clone()) {
                        keys.push(key);
                    }
                }
                if sorted {
                    keys.sort();
                }
                Ok(Value::Array(keys.into_iter().map(Value::String).collect()))
            }
            ValueKind::Array => {
                const MOST: usize = 1_000_000;
                if lazy.len() > MOST {
                    return Err(Failure::Refusal(format!(
                        "the keys of {} are more than this can gather ({MOST})",
                        self.describe(lazy)
                    )));
                }
                Ok(Value::Array((0..lazy.len()).map(Value::from).collect()))
            }
            _ => Err(Failure::Error(format!(
                "cannot use {} as iterable (array or object)",
                self.shown(lazy)
            ))),
        }
    }
}

/// Whether `select` lets an output through: any that is not `null` or `false`.
fn truthy(item: &Item<'_>) -> bool {
    match item {
        Item::Value(value) => !matches!(value, Value::Null | Value::Bool(false)),
        Item::Lazy(lazy) => match lazy.kind() {
            ValueKind::Null => false,
            ValueKind::Bool => lazy.node().raw() != b"false",
            _ => true,
        },
    }
}
