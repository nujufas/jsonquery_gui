//! Embeds jaq to run jq-compatible queries against an in-memory
//! `serde_json::Value`. jaq is this project's first query dialect, not its
//! only one — [`JaqEngine`] (in [`jq`]) is the `jsonquery_core::engine`
//! adapter other code should reach for; [`run_query`] below is the
//! lower-level function it wraps.
//!
//! Results are streamed out through a callback rather than collected into a
//! `Vec` first: jaq's evaluator is generator-based, so pulling one item at a
//! time is what lets a slow or unbounded query be cancelled mid-run and lets
//! `first`/`limit` genuinely short-circuit (Architecture §4, §7).

mod convert;
pub mod jq;

pub use convert::{from_val, to_val};
pub use jaq_json::Val;
pub use jq::JaqEngine;

use std::sync::atomic::{AtomicBool, Ordering};

use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, unwrap_valr, Compiler, Ctx, Vars};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("query syntax error: {0}")]
    Parse(String),
    #[error("query compile error: {0}")]
    Compile(String),
}

/// One item produced while a query runs: either a result value, or a
/// per-item evaluation error (jq filters can fail partway through a stream
/// without aborting the whole query, e.g. `.[] | .foo` over a mixed array).
pub enum QueryEvent {
    Item(Value),
    ItemError(String),
}

/// Compile `query_src` and run it against `input`, calling `on_event` once
/// per item as jaq produces it. `cancelled` is checked between items so a
/// long-running or unbounded query can be stopped from another thread
/// without waiting for it to finish (Architecture §5's generation-counter
/// cancellation, applied here at the query layer).
///
/// Returns the number of items actually pulled (fewer than the query would
/// eventually produce, if cancelled).
pub fn run_query(
    input: &Value,
    query_src: &str,
    cancelled: &AtomicBool,
    mut on_event: impl FnMut(QueryEvent),
) -> Result<usize, QueryError> {
    let program = File {
        code: query_src,
        path: (),
    };

    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());

    let loader = Loader::new(defs);
    let arena = Arena::default();

    let modules = loader
        .load(&arena, program)
        .map_err(|e| QueryError::Parse(format!("{e:?}")))?;

    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|e| QueryError::Compile(format!("{e:?}")))?;

    let val_input = to_val(input);
    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    let out = filter.id.run((ctx, val_input)).map(unwrap_valr);

    let mut count = 0usize;
    for item in out {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        count += 1;
        match item {
            Ok(v) => on_event(QueryEvent::Item(from_val(&v))),
            Err(e) => on_event(QueryEvent::ItemError(e.to_string())),
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collect(input: &Value, query: &str) -> Result<Vec<Value>, QueryError> {
        let cancelled = AtomicBool::new(false);
        let mut items = Vec::new();
        run_query(input, query, &cancelled, |ev| {
            if let QueryEvent::Item(v) = ev {
                items.push(v);
            }
        })?;
        Ok(items)
    }

    #[test]
    fn identity() {
        let input = json!({"a": 1});
        assert_eq!(collect(&input, ".").unwrap(), vec![input]);
    }

    #[test]
    fn iterate_and_filter() {
        let input = json!([1, 2, 3, 4, 5]);
        let got = collect(&input, ".[] | select(. > 2)").unwrap();
        assert_eq!(got, vec![json!(3), json!(4), json!(5)]);
    }

    #[test]
    fn cancellation_stops_the_stream_early() {
        let input = json!((0..1_000_000).collect::<Vec<_>>());
        let cancelled = AtomicBool::new(false);
        let mut items = Vec::new();
        run_query(&input, ".[]", &cancelled, |ev| {
            if let QueryEvent::Item(v) = ev {
                items.push(v);
                if items.len() == 5 {
                    cancelled.store(true, Ordering::Relaxed);
                }
            }
        })
        .unwrap();
        assert_eq!(items.len(), 5);
    }

    #[test]
    fn syntax_error_is_reported() {
        let input = json!(null);
        let err = collect(&input, "def").unwrap_err();
        assert!(matches!(err, QueryError::Parse(_) | QueryError::Compile(_)));
    }

    #[test]
    fn short_circuits_with_first() {
        // Large enough that eagerly collecting the whole stream first would
        // be a very different runtime profile from actually short-circuiting.
        let input = json!((0..5_000_000).collect::<Vec<_>>());
        let got = collect(&input, "first(.[] | select(. > 10))").unwrap();
        assert_eq!(got, vec![json!(11)]);
    }
}
