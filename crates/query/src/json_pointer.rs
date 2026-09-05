//! [`JsonPointerEngine`]: RFC 6901 JSON Pointer support, built directly on
//! `serde_json::Value::pointer` — no extra dependency, the cheapest possible
//! second `QueryEngine` (see docs/query-engines.html's comparison matrix). A
//! pointer names at most one value, so unlike jq's streaming filters this
//! always produces zero or one `QueryEvent::Item`, never partial per-item
//! errors.

use std::sync::atomic::AtomicBool;

use jsonquery_core::engine::{QueryEngine, QueryError, QueryEvent};
use serde_json::Value;

pub struct JsonPointerEngine;

impl QueryEngine for JsonPointerEngine {
    fn id(&self) -> &'static str {
        "pointer"
    }

    fn run(
        &self,
        input: &Value,
        query_src: &str,
        _cancelled: &AtomicBool,
        on_event: &mut dyn FnMut(QueryEvent),
    ) -> Result<usize, QueryError> {
        let pointer = query_src.trim();
        if !pointer.is_empty() && !pointer.starts_with('/') {
            return Err(QueryError::Parse(
                "a JSON pointer must be empty (whole document) or start with '/'".into(),
            ));
        }
        match input.pointer(pointer) {
            Some(value) => {
                on_event(QueryEvent::Item(value.clone()));
                Ok(1)
            }
            None => Err(QueryError::Engine(format!(
                "no value at pointer '{pointer}'"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collect(input: &Value, query: &str) -> Result<Vec<Value>, QueryError> {
        let cancelled = AtomicBool::new(false);
        let mut items = Vec::new();
        JsonPointerEngine.run(input, query, &cancelled, &mut |ev| {
            if let QueryEvent::Item(v) = ev {
                items.push(v);
            }
        })?;
        Ok(items)
    }

    #[test]
    fn id_is_pointer() {
        assert_eq!(JsonPointerEngine.id(), "pointer");
    }

    #[test]
    fn empty_pointer_returns_whole_document() {
        let input = json!({"a": 1});
        assert_eq!(collect(&input, "").unwrap(), vec![input]);
    }

    #[test]
    fn resolves_a_nested_path() {
        let input = json!({"a": {"b": [1, 2, 3]}});
        assert_eq!(collect(&input, "/a/b/1").unwrap(), vec![json!(2)]);
    }

    #[test]
    fn missing_path_is_an_error() {
        let input = json!({"a": 1});
        assert!(collect(&input, "/nope").is_err());
    }

    #[test]
    fn malformed_pointer_is_a_parse_error() {
        let input = json!({"a": 1});
        let err = collect(&input, "a").unwrap_err();
        assert!(matches!(err, QueryError::Parse(_)));
    }
}
