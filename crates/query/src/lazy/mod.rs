//! Queries over a document that is still in its file (`jsonquery_core::lazy`),
//! which is too big to be a value in memory.
//!
//! - jq: the program is read for the stages that can be walked (see [`plan`]),
//!   which are, and so the file's size is passed over without it ever being in
//!   memory, and what is left is run by jaq on one piece of the file at a time
//!   (see [`eval`]). What cannot be done that way — a program that needs the
//!   whole of a list at once, such as `sort_by` or `group_by` — is an error that
//!   says so and says what to do instead.
//! - JSON Pointer: a pointer names one place, which it is walked to.
//! - JSONPath and JMESPath work on a value in memory only, and are refused.

mod eval;
mod plan;

#[cfg(test)]
mod tests;

use std::sync::atomic::AtomicBool;

use jsonquery_core::engine::{QueryError, QueryEvent};
use jsonquery_core::lazy::{LazyTree, CHANGED_WHILE_OPEN};

pub use eval::{Limits, Parallel};

use crate::Kind;

/// Run `query_src`, a query of the dialect `kind`, over the document in `tree`.
/// Returns how many outputs (and errors) it made, as [`QueryEngine::run`] does.
///
/// [`QueryEngine::run`]: jsonquery_core::engine::QueryEngine::run
pub fn run(
    kind: Kind,
    tree: &LazyTree,
    query_src: &str,
    cancelled: &AtomicBool,
    on_event: &mut dyn FnMut(QueryEvent),
) -> Result<usize, QueryError> {
    run_with(
        kind,
        tree,
        query_src,
        Limits::default(),
        cancelled,
        on_event,
    )
}

/// [`run`] with the limits it holds itself to given, so that a small document
/// can be treated as a big one.
pub fn run_with(
    kind: Kind,
    tree: &LazyTree,
    query_src: &str,
    limits: Limits,
    cancelled: &AtomicBool,
    on_event: &mut dyn FnMut(QueryEvent),
) -> Result<usize, QueryError> {
    let changed = || QueryError::Engine(CHANGED_WHILE_OPEN.to_owned());
    // A file that was cut short has zeros where its end was: whatever is read
    // from it is no answer.
    if tree.damaged() {
        return Err(changed());
    }
    let result = match kind {
        Kind::Jq => eval::run_jq(tree, query_src, limits, cancelled, on_event),
        Kind::JsonPointer => pointer(tree, query_src, limits, on_event),
        Kind::JsonPath => eval::run_jsonpath(tree, query_src, limits, cancelled, on_event),
        Kind::JmesPath => eval::run_jmespath(tree, query_src, limits, cancelled, on_event),
    };
    if tree.damaged() {
        return Err(changed());
    }
    result
}

/// The value a JSON Pointer names, found the way `serde_json::Value::pointer`
/// finds it.
fn pointer(
    tree: &LazyTree,
    query_src: &str,
    limits: Limits,
    on_event: &mut dyn FnMut(QueryEvent),
) -> Result<usize, QueryError> {
    use jsonquery_core::ValueKind;

    let pointer = query_src.trim();
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return Err(QueryError::Parse(
            "a JSON pointer must be empty (whole document) or start with '/'".into(),
        ));
    }
    let not_found = || QueryError::Engine(format!("no value at pointer '{pointer}'"));

    // Index tokens are written as they are in an array: no `+`, no leading zero.
    let index = |token: &str| -> Option<usize> {
        if token.starts_with('+') || (token.starts_with('0') && token.len() != 1) {
            return None;
        }
        token.parse().ok()
    };

    let mut node = tree.root();
    for token in pointer.split('/').skip(1) {
        let token = token.replace("~1", "/").replace("~0", "~");
        let next = match node.kind() {
            ValueKind::Object => node.child_by_key(&token),
            ValueKind::Array => index(&token).and_then(|i| node.child(i)),
            _ => None,
        };
        node = next.ok_or_else(not_found)?.node;
    }

    let value = node.to_value(limits.result_bytes).map_err(|e| {
        QueryError::Engine(format!(
            "the value at '{pointer}' is too big to show ({e}): point at something inside it"
        ))
    })?;
    on_event(QueryEvent::Item(value));
    Ok(1)
}
