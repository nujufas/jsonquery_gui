//! Expressions made of parts that are walked: `{count: length, first: .[0]}`,
//! `.[-1] - .[0]`, `length > 1`.
//!
//! Each part is a pipeline run on the node, which is walked as any is; what it
//! makes stands in the text of the expression where it was, and the text, which
//! now needs no node, is run by jaq. So what jaq does with a part that makes no
//! output, or many, or an error, is what it does with the part itself: the
//! objects of `{a: .x, b: .y}` where each makes two are four, in jaq's order,
//! and an error is met where the part would have been.

use serde_json::Value;

use super::super::plan::{Compose, Piece, Pipeline};
use super::{approximate_size, bytes_text, Emission, Evaluator, Failure, Flow, Item, Lazy, Sink};
use crate::{to_val, Val};

/// What the parts of an expression made.
enum Gathered {
    /// What it made, each output a value or the text of the error that it was; and,
    /// if it was refused, why.
    Made(Vec<Result<Value, String>>, Option<String>),
    Stopped,
}

impl Evaluator<'_> {
    /// An expression of parts that are walked: each is run on `lazy`, and jaq
    /// is given the text of the expression with what they made in their places.
    pub(super) fn compose<'t>(
        &self,
        p: &Pipeline,
        i: usize,
        compose: &Compose,
        lazy: Lazy<'t>,
        sink: Sink<'_, 't>,
    ) -> Flow {
        let mut made = Vec::with_capacity(compose.leaves.len());
        // What was too big for a part: it is an error where jaq comes to it, and
        // only there, so that `a and b` is not refused for a `b` that is not run.
        let mut refusals: Vec<String> = Vec::new();
        for leaf in &compose.leaves {
            match self.gather(leaf, &lazy) {
                Gathered::Made(mut outputs, refusal) => {
                    if let Some(refusal) = refusal {
                        outputs.push(Err(refusal.clone()));
                        refusals.push(refusal);
                    }
                    made.push(Some(outputs));
                }
                Gathered::Stopped => return Flow::Stop,
            }
        }

        let mut text = String::new();
        let mut globals: Vec<Val> = Vec::with_capacity(made.len());
        for piece in &compose.pieces {
            match piece {
                Piece::Text(literal) => text.push_str(literal),
                Piece::Leaf(n) => {
                    let outputs = made[*n].take().unwrap_or_default();
                    // The variable of this part is the slot it is in.
                    debug_assert_eq!(globals.len(), *n);
                    let (stand_in, bound) = stand_in(*n, outputs);
                    text.push_str(&stand_in);
                    globals.push(bound);
                }
            }
        }

        let program = match self.program_of_parts(&text, made.len()) {
            Ok(program) => program,
            Err(e) => return sink(Emission::Error(e)),
        };
        for result in program.run(Val::Null, globals) {
            if self.stopped() {
                return Flow::Stop;
            }
            let flow = match result {
                Ok(v) => self.feed(p, i + 1, Item::Value(v), sink),
                Err(e) if refusals.contains(&e) => sink(Emission::Refusal(e)),
                Err(e) => sink(Emission::Error(e)),
            };
            if flow == Flow::Stop {
                return Flow::Stop;
            }
        }
        Flow::Continue
    }

    /// What `pipeline` makes of `lazy`, each output as a value: all of what it
    /// makes, in order, the errors among it as the text of the error. Or the
    /// refusal that ended it, or the end of it from outside.
    fn gather<'t>(&self, pipeline: &Pipeline, lazy: &Lazy<'t>) -> Gathered {
        let mut made = Vec::new();
        let mut bytes = 0usize;
        let mut refused = None;
        self.feed(pipeline, 0, Item::Lazy(lazy.clone()), &mut |emission| {
            let value = match emission {
                Emission::Item(item) => self.part(item),
                Emission::Error(e) => Err(Failure::Error(e)),
                Emission::Refusal(e) => Err(Failure::Refusal(e)),
            };
            match value {
                Ok(value) => {
                    bytes += approximate_size(&value);
                    made.push(Ok(value));
                }
                Err(Failure::Error(e)) => made.push(Err(e)),
                Err(Failure::Refusal(e)) => {
                    refused = Some(e);
                    return Flow::Stop;
                }
            }
            if bytes > self.limits.collect_bytes {
                refused = Some(format!(
                    "what this makes is over {}: too much to hold in memory; narrow it",
                    bytes_text(self.limits.collect_bytes)
                ));
                return Flow::Stop;
            }
            Flow::Continue
        });
        if self.stopped() {
            Gathered::Stopped
        } else {
            Gathered::Made(made, refused)
        }
    }
}

/// What stands in an expression for a part that made `outputs`, and the value of
/// the variable it is read from: the part's value if it made one, nothing if it
/// made none, its values one after the other if it made many, and, where there
/// are errors among them, each in its place, so that jaq meets them as it would
/// have met the part.
fn stand_in(n: usize, outputs: Vec<Result<Value, String>>) -> (String, Val) {
    if outputs.iter().all(Result::is_ok) {
        let mut values: Vec<Value> = outputs.into_iter().flatten().collect();
        return match values.len() {
            0 => ("empty".to_owned(), Val::Null),
            1 => (format!("$__l{n}"), to_val(&values.remove(0))),
            _ => (format!("$__l{n}[]"), to_val(&Value::Array(values))),
        };
    }
    let mut values = Vec::new();
    let mut parts = Vec::with_capacity(outputs.len());
    for output in outputs {
        match output {
            Ok(value) => {
                parts.push(format!("$__l{n}[{}]", values.len()));
                values.push(value);
            }
            Err(e) => parts.push(format!(
                "error({})",
                serde_json::to_string(&e).unwrap_or_default()
            )),
        }
    }
    (
        format!("({})", parts.join(", ")),
        to_val(&Value::Array(values)),
    )
}
