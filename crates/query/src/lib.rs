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
pub mod diff;
mod errors;
pub mod highlight;
pub mod jmespath_engine;
pub mod jq;
mod jq_ext;
pub mod json_pointer;
pub mod jsonpath;
pub mod lazy;
pub mod merge;
pub mod output;
pub mod patch;
pub mod reformat;
pub mod schema;
pub mod suggest;
pub mod tutorial;

pub use convert::{from_val, to_val};
pub use jaq_json::Val;
pub use jmespath_engine::JmesPathEngine;
pub use jq::JaqEngine;
pub use json_pointer::JsonPointerEngine;
pub use jsonpath::JsonPathEngine;
pub use output::OutputFormat;
pub use suggest::{engines_in_scope, suggest, Suggestion};

use std::sync::atomic::{AtomicBool, Ordering};

use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, Compiler, Ctx, Error, ValR, ValX, Vars};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    on_event: impl FnMut(QueryEvent),
) -> Result<usize, QueryError> {
    run_query_with_vars(input, query_src, &[], cancelled, on_event)
}

/// Like [`run_query`], with named global variables the query can read —
/// `("$files", value)` makes `$files` mean `value`. Every name must start with
/// `$`. (The merge tool uses this to hand its filter the list of file names.)
pub fn run_query_with_vars(
    input: &Value,
    query_src: &str,
    vars: &[(&str, Value)],
    cancelled: &AtomicBool,
    mut on_event: impl FnMut(QueryEvent),
) -> Result<usize, QueryError> {
    let names: Vec<&str> = vars.iter().map(|(name, _)| *name).collect();
    let program = Program::compile(query_src, &names)?;
    let values = vars.iter().map(|(_, value)| to_val(value)).collect();

    let mut count = 0usize;
    for item in program.run(to_val(input), values) {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        count += 1;
        match item {
            Ok(v) => on_event(QueryEvent::Item(v)),
            Err(e) => on_event(QueryEvent::ItemError(e)),
        }
    }
    Ok(count)
}

/// A jq program, compiled, which can be run on any number of inputs: what the
/// query box makes of its text once, and what a query over a very large file
/// runs for each of the values it reads from it.
pub(crate) struct Program {
    filter: jaq_core::Filter<data::JustLut<Val>>,
}

impl Program {
    /// Compile `query_src`, which may read the global variables `globals` (each
    /// a name that starts with `$`).
    pub(crate) fn compile(query_src: &str, globals: &[&str]) -> Result<Self, QueryError> {
        let program = File {
            code: query_src,
            path: (),
        };

        let defs = jaq_core::defs()
            .chain(jaq_std::defs())
            .chain(jaq_json::defs())
            .chain(jq_ext::defs());
        let funs = jaq_core::funs()
            .chain(jaq_std::funs())
            .chain(jaq_json::funs())
            .chain(jq_ext::funs());

        let loader = Loader::new(defs);
        let arena = Arena::default();

        let modules = loader
            .load(&arena, program)
            .map_err(|e| QueryError::Parse(errors::syntax(query_src, &e)))?;

        let filter = Compiler::default()
            .with_funs(funs)
            .with_global_vars(globals.iter().copied())
            .compile(modules)
            .map_err(|e| QueryError::Compile(errors::undefined(&e)))?;
        Ok(Self { filter })
    }

    /// What it makes of `input`, one output (or the error that ends it) at a time,
    /// as they are pulled — so a query that takes the first few stops there.
    /// `globals` are the values of the variables it was compiled with.
    pub(crate) fn run<'a>(
        &'a self,
        input: Val,
        globals: Vec<Val>,
    ) -> impl Iterator<Item = Result<Value, String>> + 'a {
        self.run_vals(input, globals).map(|item| match item {
            Ok(v) => Ok(from_val(&v)),
            Err(e) => Err(e),
        })
    }

    /// [`run`](Self::run), with the outputs as jaq makes them: what is to be
    /// compared with another is not made a text and read again first.
    pub(crate) fn run_vals<'a>(
        &'a self,
        input: Val,
        globals: Vec<Val>,
    ) -> impl Iterator<Item = Result<Val, String>> + 'a {
        let ctx = Ctx::<data::JustLut<Val>>::new(&self.filter.lut, Vars::new(globals));
        let mut halted = false;
        self.filter
            .id
            .run((ctx, input))
            .map_while(move |item| {
                if halted {
                    return None;
                }
                let (item, halt) = unwrap_exception(item);
                halted = halt;
                Some(item)
            })
            .map(|item| item.map_err(error_text))
    }
}

/// What a main filter's outputs are made of, without `jaq_core::unwrap_valr`: that one ends the
/// *process* when the program says `halt` (or `halt_error`), which is what `jq` does and
/// nothing a query box should: typing `halt` would close the whole app. Here it is an error,
/// the last output of the stream (`true` in the second place says it was a halt).
fn unwrap_exception(item: ValX<'_, Val>) -> (ValR<Val>, bool) {
    match item {
        Ok(value) => (Ok(value), false),
        Err(exception) => match exception.get_err() {
            Ok(error) => (Err(error), false),
            Err(other) => match other.get_halt() {
                Ok(code) => (
                    Err(Error::str(format!(
                        "halt ({code}): jq's halt ends the program, which a query cannot do here"
                    ))),
                    true,
                ),
                // Neither of these leaves a main filter in jaq; say so rather than panic.
                Err(_) => (
                    Err(Error::str("the query ended in a way a query cannot")),
                    true,
                ),
            },
        },
    }
}

/// An error as the text a reader should see. jaq prints an error that carries
/// a string — `error("boom")`, or the ones [`jq_ext`] raises — as a JSON
/// string, quotes and escapes included; the text itself is what is wanted.
fn error_text(error: jaq_json::Error) -> String {
    match error.into_val() {
        Val::TStr(text) => String::from_utf8_lossy(&text).into_owned(),
        other => other.to_string(),
    }
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

    /// `halt` ends a jq program, and the process with it, in jq and in jaq's own
    /// `unwrap_valr`. Here it is an error, so a query box cannot close the app.
    #[test]
    fn halt_is_an_error_not_the_end_of_the_process() {
        let input = json!({"a": 1});
        for query in ["halt", "halt_error", "halt_error(3)", "\"x\" | halt_error"] {
            let cancelled = AtomicBool::new(false);
            let mut errors = Vec::new();
            run_query(&input, query, &cancelled, |event| {
                if let QueryEvent::ItemError(message) = event {
                    errors.push(message);
                }
            })
            .unwrap();
            assert_eq!(errors.len(), 1, "{query}");
            assert!(errors[0].starts_with("halt ("), "{query}: {}", errors[0]);
        }
        // what came before it is kept, and the stream ends at the halt
        let mut items = Vec::new();
        let mut errors = 0;
        run_query(
            &input,
            "1, halt, 2",
            &AtomicBool::new(false),
            |event| match event {
                QueryEvent::Item(v) => items.push(v),
                QueryEvent::ItemError(_) => errors += 1,
            },
        )
        .unwrap();
        assert_eq!((items, errors), (vec![json!(1)], 1));
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
