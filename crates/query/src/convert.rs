//! Conversion between the document's `serde_json::Value` and jaq's own `Val`.
//!
//! jaq-json's `Val` is jaq's native value representation (see
//! `jaq_json::Val`), distinct from `serde_json::Value`. Converting through it
//! rather than writing a custom `ValT` adapter is exactly what Phase 1 of the
//! roadmap calls for ("jaq embedded directly against owned Values — no
//! adapter needed yet"); the lazy `Arc<Mmap>`-backed adapter sketched in
//! Architecture §4 is Phase 2 work, once the working set stops fitting
//! comfortably in memory.
//!
//! Numbers go through [`jaq_core::ValT::from_num`], which stores the
//! original literal text for anything that isn't a plain machine integer —
//! this is what preserves e.g. a 64-bit snowflake ID through a query
//! untouched instead of rounding it through an f64.

use jaq_core::ValT;
use jaq_json::Val;
use serde_json::Value;

pub fn to_val(v: &Value) -> Val {
    match v {
        Value::Null => Val::Null,
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => Val::from_num(&n.to_string()).unwrap_or(Val::Null),
        Value::String(s) => Val::utf8_str(s.clone().into_bytes()),
        Value::Array(items) => items.iter().map(to_val).collect(),
        Value::Object(map) => Val::obj(
            map.iter()
                .map(|(k, v)| (Val::utf8_str(k.clone().into_bytes()), to_val(v)))
                .collect(),
        ),
    }
}

/// Convert a query-result `Val` back into a `serde_json::Value` for display,
/// by writing it through jaq's own JSON-compatible `Display` impl and
/// re-parsing it with `arbitrary_precision` so number literals still survive
/// exactly. Standard JSON-in/JSON-out queries always round-trip cleanly here;
/// the fallback only matters for jq extensions (e.g. binary strings) that
/// can't be represented as plain JSON, which don't arise from filters over
/// ordinary JSON input.
pub fn from_val(v: &Val) -> Value {
    let text = v.to_string();
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_common_shapes() {
        let v = json!({"a": [1, 2.5, "x", null, true], "b": {"c": -3}});
        assert_eq!(from_val(&to_val(&v)), v);
    }

    #[test]
    fn preserves_big_integer_literal() {
        let v = json!({"id": 9223372036854775807i64});
        assert_eq!(from_val(&to_val(&v)), v);

        // Bigger than any primitive integer type serde_json exposes directly;
        // arbitrary_precision keeps this as exact literal text end to end.
        let big = json!({"id": serde_json::Number::from_string_unchecked(
            "123456789012345678901234567890".to_string()
        )});
        assert_eq!(from_val(&to_val(&big)), big);
    }
}
