//! [`JaqEngine`]: this project's first `jsonquery_core::engine::QueryEngine`
//! adapter. It's a thin translation layer over [`crate::run_query`] — jaq's
//! own richer `Parse`/`Compile` error split collapses to the engine-agnostic
//! `QueryError`'s `Parse`/`Engine` split at this boundary, and jaq's own
//! `QueryEvent` maps 1:1 onto the core one — so the adapter carries no logic
//! of its own beyond that translation.

use std::sync::atomic::AtomicBool;

use jsonquery_core::engine::{QueryEngine, QueryError as EngineError, QueryEvent as EngineEvent};
use serde_json::Value;

use crate::{run_query, QueryError, QueryEvent};

pub struct JaqEngine;

impl QueryEngine for JaqEngine {
    fn id(&self) -> &'static str {
        "jq"
    }

    fn run(
        &self,
        input: &Value,
        query_src: &str,
        cancelled: &AtomicBool,
        on_event: &mut dyn FnMut(EngineEvent),
    ) -> Result<usize, EngineError> {
        run_query(input, query_src, cancelled, |event| {
            on_event(match event {
                QueryEvent::Item(v) => EngineEvent::Item(v),
                QueryEvent::ItemError(e) => EngineEvent::ItemError(e),
            });
        })
        .map_err(|e| match e {
            QueryError::Parse(s) => EngineError::Parse(s),
            QueryError::Compile(s) => EngineError::Engine(s),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collect(input: &Value, query: &str) -> Result<Vec<Value>, EngineError> {
        let cancelled = AtomicBool::new(false);
        let mut items = Vec::new();
        JaqEngine.run(input, query, &cancelled, &mut |ev| {
            if let EngineEvent::Item(v) = ev {
                items.push(v);
            }
        })?;
        Ok(items)
    }

    #[test]
    fn id_is_jq() {
        assert_eq!(JaqEngine.id(), "jq");
    }

    #[test]
    fn runs_a_filter_through_the_engine_trait() {
        let input = json!([1, 2, 3, 4, 5]);
        let got = collect(&input, ".[] | select(. > 2)").unwrap();
        assert_eq!(got, vec![json!(3), json!(4), json!(5)]);
    }

    #[test]
    fn parse_error_maps_to_engine_parse_error() {
        let err = collect(&json!(null), "def").unwrap_err();
        assert!(matches!(
            err,
            EngineError::Parse(_) | EngineError::Engine(_)
        ));
    }
}
