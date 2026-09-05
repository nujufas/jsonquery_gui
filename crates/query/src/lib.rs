//! This project's query dialects, each plugged into
//! `jsonquery_core::engine::QueryEngine`. jaq (in [`jq`]) was the first;
//! [`json_pointer`], [`jsonpath`], and [`jmespath_engine`] add JSON Pointer
//! (RFC 6901), JSONPath (RFC 9535, via `jsonpath-rust`), and JMESPath (via
//! the `jmespath` crate) as independently pluggable dialects picked from the
//! survey in docs/query-engines.html. [`Kind`] is the UI-facing enum tying a
//! dialect's id to its `QueryEngine` impl and to [`Kind::detect`]'s
//! best-effort auto-selection from the query text alone.
//!
//! [`run_query`] below is jaq's own lower-level entry point, kept for
//! [`JaqEngine`] and its tests to wrap; other code should reach for a
//! `QueryEngine` (via [`Kind::engine`]) rather than this function directly.
//! Its results are streamed out through a callback rather than collected
//! into a `Vec` first: jaq's evaluator is generator-based, so pulling one
//! item at a time is what lets a slow or unbounded query be cancelled
//! mid-run and lets `first`/`limit` genuinely short-circuit (Architecture
//! §4, §7). The other three dialects don't have this concern — JSONPath and
//! JMESPath both evaluate to a bounded result eagerly — so their
//! `QueryEngine` impls live directly in their own modules with no separate
//! lower-level function.

mod convert;
pub mod jmespath_engine;
pub mod jq;
pub mod json_pointer;
pub mod jsonpath;

pub use convert::{from_val, to_val};
pub use jaq_json::Val;
pub use jmespath_engine::JmesPathEngine;
pub use jq::JaqEngine;
pub use json_pointer::JsonPointerEngine;
pub use jsonpath::JsonPathEngine;

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

/// Which `QueryEngine` a query should run against — set explicitly via the
/// UI's engine picker, or left to [`Kind::detect`] when none of its buttons
/// is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jq,
    JsonPointer,
    JsonPath,
    JmesPath,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Jq, Kind::JsonPointer, Kind::JsonPath, Kind::JmesPath];

    /// Short label for the engine-picker buttons and status-bar text.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Jq => "jq",
            Kind::JsonPointer => "Pointer",
            Kind::JsonPath => "JSONPath",
            Kind::JmesPath => "JMESPath",
        }
    }

    /// One-line syntax example, for the engine-picker buttons' hover text.
    pub fn example(self) -> &'static str {
        match self {
            Kind::Jq => "jq — e.g. .[] | select(.age > 21) | .name",
            Kind::JsonPointer => "JSON Pointer (RFC 6901) — e.g. /store/book/0",
            Kind::JsonPath => "JSONPath (RFC 9535) — e.g. $.store.book[*].author",
            Kind::JmesPath => "JMESPath — e.g. people[?age > `30`].age",
        }
    }

    /// This dialect's `QueryEngine` implementor. Every engine here is a
    /// zero-sized, stateless struct, so a `&'static dyn` reference is enough
    /// — no allocation needed to hand the caller something to `.run()`.
    pub fn engine(self) -> &'static dyn jsonquery_core::engine::QueryEngine {
        match self {
            Kind::Jq => &JaqEngine,
            Kind::JsonPointer => &JsonPointerEngine,
            Kind::JsonPath => &JsonPathEngine,
            Kind::JmesPath => &JmesPathEngine,
        }
    }

    /// Best-effort dialect detection from `query`'s own syntax, used when no
    /// engine button is explicitly selected. JSON Pointer and JSONPath have
    /// unambiguous leading markers (`/` and `$`), and jq filters
    /// overwhelmingly start with `.`; JMESPath has no such marker and
    /// overlaps a lot syntactically with jq (both use bare `foo.bar`-style
    /// paths), so a handful of JMESPath-only substrings are checked before
    /// falling back to jq, the richer and originally-default dialect.
    pub fn detect(query: &str) -> Kind {
        let trimmed = query.trim();
        if trimmed.is_empty() || trimmed.starts_with('.') {
            return Kind::Jq;
        }
        if trimmed.starts_with('/') {
            return Kind::JsonPointer;
        }
        if trimmed.starts_with('$') {
            return Kind::JsonPath;
        }
        const JMESPATH_MARKERS: [&str; 4] = ["[?", "&&", "||", "`"];
        if JMESPATH_MARKERS.iter().any(|m| trimmed.contains(m)) {
            return Kind::JmesPath;
        }
        Kind::Jq
    }
}

#[cfg(test)]
mod kind_tests {
    use super::Kind;

    #[test]
    fn detects_json_pointer() {
        assert_eq!(Kind::detect("/a/b/0"), Kind::JsonPointer);
        assert_eq!(Kind::detect(""), Kind::Jq); // ambiguous; jq's "." also means "whole doc"
    }

    #[test]
    fn detects_jsonpath() {
        assert_eq!(Kind::detect("$.store.book[*].author"), Kind::JsonPath);
    }

    #[test]
    fn detects_jq() {
        assert_eq!(Kind::detect(".[] | select(.age > 21)"), Kind::Jq);
    }

    #[test]
    fn detects_jmespath_via_filter_bracket() {
        assert_eq!(Kind::detect("people[?age > `30`].age"), Kind::JmesPath);
    }

    #[test]
    fn ambiguous_bare_path_defaults_to_jq() {
        assert_eq!(Kind::detect("foo.bar"), Kind::Jq);
    }
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
