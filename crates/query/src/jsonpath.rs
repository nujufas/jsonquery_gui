//! [`JsonPathEngine`]: JSONPath (RFC 9535) support via the `jsonpath-rust`
//! crate's `Value::query` — its API (`&Value -> Vec<&Value>`) is close
//! enough to `QueryEngine::run`'s shape to be nearly a drop-in adapter (see
//! docs/query-engines.html's comparison matrix for why this crate was picked
//! over `serde_json_path`). Unlike jaq's generator-based evaluator, `query`
//! collects every match eagerly before returning, so `cancelled` can only
//! stop the `on_event` loop below, not the match itself.

use std::sync::atomic::{AtomicBool, Ordering};

use jsonpath_rust::JsonPath;
use jsonquery_core::engine::{QueryEngine, QueryError, QueryEvent};
use serde_json::Value;

pub struct JsonPathEngine;

impl QueryEngine for JsonPathEngine {
    fn id(&self) -> &'static str {
        "jsonpath"
    }

    fn run(
        &self,
        input: &Value,
        query_src: &str,
        cancelled: &AtomicBool,
        on_event: &mut dyn FnMut(QueryEvent),
    ) -> Result<usize, QueryError> {
        let matches = input
            .query(query_src)
            .map_err(|e| QueryError::Parse(e.to_string()))?;

        let mut count = 0usize;
        for value in matches {
            if cancelled.load(Ordering::Relaxed) {
                break;
            }
            on_event(QueryEvent::Item(value.clone()));
            count += 1;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collect(input: &Value, query: &str) -> Result<Vec<Value>, QueryError> {
        let cancelled = AtomicBool::new(false);
        let mut items = Vec::new();
        JsonPathEngine.run(input, query, &cancelled, &mut |ev| {
            if let QueryEvent::Item(v) = ev {
                items.push(v);
            }
        })?;
        Ok(items)
    }

    #[test]
    fn id_is_jsonpath() {
        assert_eq!(JsonPathEngine.id(), "jsonpath");
    }

    #[test]
    fn root_returns_whole_document() {
        let input = json!({"a": 1});
        assert_eq!(collect(&input, "$").unwrap(), vec![input]);
    }

    #[test]
    fn wildcard_over_an_array() {
        let input = json!({"shop": {"prices": [1, 2, 3]}});
        let got = collect(&input, "$.shop.prices[*]").unwrap();
        assert_eq!(got, vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn filter_expression() {
        let input = json!({"items": [{"n": 1}, {"n": 5}, {"n": 9}]});
        let got = collect(&input, "$.items[?(@.n > 3)].n").unwrap();
        assert_eq!(got, vec![json!(5), json!(9)]);
    }

    #[test]
    fn syntax_error_is_reported() {
        let input = json!(null);
        let err = collect(&input, "$[").unwrap_err();
        assert!(matches!(err, QueryError::Parse(_)));
    }
}
