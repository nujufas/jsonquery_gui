//! [`JmesPathEngine`]: JMESPath support via the `jmespath` crate. Unlike jq
//! or JSONPath, JMESPath always evaluates to exactly one value (an array
//! result is still one JSON value, not a stream of items to iterate), so a
//! successful run always produces exactly one `QueryEvent::Item` — reaching
//! for `[*]`-style projections is how a JMESPath query itself spreads a
//! collection, the same way it works in `jmespath.org`'s own tester.
//!
//! Input is converted to a `jmespath::Variable` by hand (`value_to_variable`
//! below) rather than through `jmespath`'s own generic `ToJmespath`/`Serialize`
//! path: this workspace's `serde_json` runs with the `arbitrary_precision`
//! feature, under which `serde_json::Number`'s `Serialize` impl emits a
//! private sentinel map meant only for `serde_json`'s own `Deserializer` to
//! recognize. `jmespath`'s generic serializer doesn't know that sentinel, so
//! every number silently becomes an unrelated one-field object — breaking
//! any numeric comparison (verified against a throwaway probe: `age > \`30\``
//! filtered out everything). Building the `Variable` tree directly from
//! `Value`'s own variants (as `Variable::Number` also wraps a plain
//! `serde_json::Number`) sidesteps that Serialize round trip entirely.

use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use jmespath::{JmespathError, Rcvar, ToJmespath, Variable};
use jsonquery_core::engine::{QueryEngine, QueryError, QueryEvent};
use serde_json::Value;

/// Feeds an already-built `Rcvar` to `Expression::search` without going
/// through `ToJmespath`'s blanket `Serialize`-based impl (see module docs).
struct RawVariable(Rcvar);

impl ToJmespath for RawVariable {
    fn to_jmespath(self) -> Result<Rcvar, JmespathError> {
        Ok(self.0)
    }
}

fn value_to_variable(value: &Value) -> Variable {
    match value {
        Value::Null => Variable::Null,
        Value::Bool(b) => Variable::Bool(*b),
        Value::Number(n) => Variable::Number(n.clone()),
        Value::String(s) => Variable::String(s.clone()),
        Value::Array(arr) => {
            Variable::Array(arr.iter().map(|v| Rc::new(value_to_variable(v))).collect())
        }
        Value::Object(map) => Variable::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), Rc::new(value_to_variable(v))))
                .collect(),
        ),
    }
}

pub struct JmesPathEngine;

impl QueryEngine for JmesPathEngine {
    fn id(&self) -> &'static str {
        "jmespath"
    }

    fn run(
        &self,
        input: &Value,
        query_src: &str,
        _cancelled: &AtomicBool,
        on_event: &mut dyn FnMut(QueryEvent),
    ) -> Result<usize, QueryError> {
        let expr = jmespath::compile(query_src).map_err(|e| QueryError::Parse(e.to_string()))?;
        let data = RawVariable(Rc::new(value_to_variable(input)));
        let result = expr
            .search(data)
            .map_err(|e| QueryError::Engine(e.to_string()))?;
        let value =
            serde_json::to_value(&*result).map_err(|e| QueryError::Engine(e.to_string()))?;
        on_event(QueryEvent::Item(value));
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collect(input: &Value, query: &str) -> Result<Vec<Value>, QueryError> {
        let cancelled = AtomicBool::new(false);
        let mut items = Vec::new();
        JmesPathEngine.run(input, query, &cancelled, &mut |ev| {
            if let QueryEvent::Item(v) = ev {
                items.push(v);
            }
        })?;
        Ok(items)
    }

    #[test]
    fn id_is_jmespath() {
        assert_eq!(JmesPathEngine.id(), "jmespath");
    }

    #[test]
    fn field_access() {
        let input = json!({"foo": {"bar": {"baz": true}}});
        assert_eq!(collect(&input, "foo.bar | baz").unwrap(), vec![json!(true)]);
    }

    #[test]
    fn filter_projection() {
        let input = json!({"people": [{"age": 20}, {"age": 40}]});
        let got = collect(&input, "people[?age > `30`].age").unwrap();
        assert_eq!(got, vec![json!([40])]);
    }

    #[test]
    fn syntax_error_is_reported() {
        let input = json!(null);
        let err = collect(&input, "foo.").unwrap_err();
        assert!(matches!(err, QueryError::Parse(_)));
    }
}
