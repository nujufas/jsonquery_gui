//! A minimal seam for pluggable query languages.
//!
//! The original design sketch treated jaq's own `ValT` trait as this
//! project's query integration point — but jaq is meant to become one of
//! several supported dialects (JSONPath variants, JSONata, lodash-style,
//! jsonquerylang.org's own DSL, jspath, ...), not the architecture's
//! permanent center. So the query-facing contract lives here, in the
//! engine-agnostic core, rather than being owned by any one query crate:
//! `crates/query`'s `JaqEngine` is this trait's first implementor, not a
//! special case baked into `core` itself. A future query-language crate
//! implements the same trait against `jsonquery-core` alone, never against
//! `jsonquery-query`/jaq.

use std::sync::atomic::AtomicBool;

use serde_json::Value;

/// One item produced while a query runs: either a result value, or a
/// per-item evaluation error that doesn't abort the whole run (e.g. jq's
/// `.[] | .foo` over a mixed array, where one element lacks `.foo`).
pub enum QueryEvent {
    Item(Value),
    ItemError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("query syntax error: {0}")]
    Parse(String),
    #[error("query error: {0}")]
    Engine(String),
}

/// One pluggable query dialect: compiles `query_src` and runs it against
/// `input`, streaming results through `on_event` one at a time rather than
/// collecting them — so a slow or unbounded query can be cancelled mid-run
/// (checked via `cancelled`), and a short-circuiting query (e.g. jq's
/// `first(...)`) can genuinely stop pulling as soon as it has enough.
pub trait QueryEngine {
    /// Short identifier for this dialect (e.g. `"jq"`) — for status messages
    /// and, once more than one engine exists, an engine picker.
    fn id(&self) -> &'static str;

    /// Returns the number of items actually produced (fewer than the query
    /// would eventually produce, if cancelled).
    fn run(
        &self,
        input: &Value,
        query_src: &str,
        cancelled: &AtomicBool,
        on_event: &mut dyn FnMut(QueryEvent),
    ) -> Result<usize, QueryError>;
}
