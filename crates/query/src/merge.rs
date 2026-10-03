//! Merging several JSON documents with jq itself — what the Tools window's
//! "Merge JSON" runs.
//!
//! It works the way `jq -s` does: the documents are *slurped* into one array,
//! in the order they were given, and a jq filter turns that array into the
//! merged result. In the filter `.` is that array and `$files` is the array of
//! the documents' file names (same order), so a filter can tell them apart.
//! The presets below are ordinary filters, shown (and editable) in the UI —
//! "custom merge" is just a different filter.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::{run_query_with_vars, QueryError, QueryEvent};

/// A ready-made merge filter, offered in the Tools window's "Merge as" list.
pub struct Preset {
    pub label: &'static str,
    pub filter: &'static str,
    /// One or two sentences on what the filter does, shown under the list.
    pub about: &'static str,
}

/// The ready-made filters, the default (append) first.
pub const PRESETS: [Preset; 4] = [
    Preset {
        label: "Append arrays",
        filter: "add",
        about: "Joins the files' arrays end to end, in the order listed (jq's add). \
                Objects work too: their keys are combined and a later file wins a clash.",
    },
    Preset {
        label: "Append, sorted and de-duplicated",
        filter: "add | unique",
        about: "Like Append arrays, then drops repeated items. jq's unique also sorts.",
    },
    Preset {
        label: "Deep-merge objects",
        filter: "reduce .[] as $file ({}; . * $file)",
        about: "Merges objects key by key, all the way down; a later file wins a clash \
                between plain values.",
    },
    Preset {
        label: "Bundle by file name",
        filter: "[range(0; length) as $i | {file: $files[$i], data: .[$i]}]",
        about: "Keeps each file whole, labelled with its name — nothing is combined.",
    },
];

/// The preset whose filter is exactly `filter`, if any — a filter that matches
/// none is a custom one.
pub fn preset_for(filter: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.filter == filter.trim())
}

/// Most outputs a filter may produce before the merge gives up: they are all
/// collected, so a filter that never stops (`repeat(1)`) must not eat the
/// memory of the machine.
pub const MAX_OUTPUTS: usize = 1_000_000;

#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error(transparent)]
    Query(#[from] QueryError),
    /// The filter failed while running (jq's usual runtime error, e.g. adding
    /// an array to an object).
    #[error("the filter failed: {0}")]
    Eval(String),
    #[error("the filter produced no output")]
    NoOutput,
    #[error("the filter produced more than {MAX_OUTPUTS} outputs")]
    TooManyOutputs,
    #[error("cancelled")]
    Cancelled,
}

/// What a merge produced.
#[derive(Debug, PartialEq)]
pub struct Merged {
    /// The merged document. A filter that gives several outputs (jq's `.[]`)
    /// yields them as one array, the way the app treats a file of several
    /// top-level values.
    pub value: Value,
    /// How many outputs the filter produced.
    pub outputs: usize,
}

/// Slurp `inputs` into one array and run `filter` over it (see the module
/// docs). `names` are the documents' names, for `$files`.
///
/// `cancelled` stops the run from outside; it is also set by the merge itself
/// when it gives up, which is how it stops jq's stream early.
pub fn merge(
    inputs: Vec<Value>,
    names: &[String],
    filter: &str,
    cancelled: &AtomicBool,
) -> Result<Merged, MergeError> {
    let files = Value::Array(names.iter().cloned().map(Value::String).collect());
    let slurped = Value::Array(inputs);

    let mut outputs = Vec::new();
    let mut failure = None;
    run_query_with_vars(
        &slurped,
        filter,
        &[("$files", files)],
        cancelled,
        |event| match event {
            QueryEvent::Item(value) => {
                if outputs.len() >= MAX_OUTPUTS {
                    failure.get_or_insert(MergeError::TooManyOutputs);
                    cancelled.store(true, Ordering::Relaxed);
                } else {
                    outputs.push(value);
                }
            }
            QueryEvent::ItemError(error) => {
                failure.get_or_insert(MergeError::Eval(error));
                cancelled.store(true, Ordering::Relaxed);
            }
        },
    )?;

    if let Some(failure) = failure {
        return Err(failure);
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err(MergeError::Cancelled);
    }
    let count = outputs.len();
    let value = match count {
        0 => return Err(MergeError::NoOutput),
        1 => outputs.remove(0),
        _ => Value::Array(outputs),
    };
    Ok(Merged {
        value,
        outputs: count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(inputs: Vec<Value>, filter: &str) -> Result<Merged, MergeError> {
        let names: Vec<String> = (1..=inputs.len()).map(|i| format!("f{i}.json")).collect();
        merge(inputs, &names, filter, &AtomicBool::new(false))
    }

    fn preset(label: &str) -> &'static str {
        PRESETS.iter().find(|p| p.label == label).unwrap().filter
    }

    #[test]
    fn appending_arrays_keeps_the_order_of_the_files() {
        let merged = run(
            vec![json!([1, 2]), json!([3]), json!([]), json!([4, 5])],
            preset("Append arrays"),
        )
        .unwrap();
        assert_eq!(merged.value, json!([1, 2, 3, 4, 5]));
        assert_eq!(merged.outputs, 1);
    }

    #[test]
    fn appending_nothing_gives_null_like_jq_does() {
        assert_eq!(run(vec![], "add").unwrap().value, Value::Null);
    }

    #[test]
    fn appending_objects_combines_keys_and_a_later_file_wins() {
        let merged = run(vec![json!({"a": 1, "b": 1}), json!({"b": 2})], "add").unwrap();
        assert_eq!(merged.value, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn sorted_and_deduplicated_drops_repeats() {
        let merged = run(
            vec![json!([3, 1]), json!([1, 2])],
            preset("Append, sorted and de-duplicated"),
        )
        .unwrap();
        assert_eq!(merged.value, json!([1, 2, 3]));
    }

    #[test]
    fn deep_merge_goes_down_into_nested_objects() {
        let merged = run(
            vec![
                json!({"a": {"x": 1, "y": 2}, "b": 1}),
                json!({"a": {"y": 9, "z": 3}, "c": 2}),
            ],
            preset("Deep-merge objects"),
        )
        .unwrap();
        assert_eq!(
            merged.value,
            json!({"a": {"x": 1, "y": 9, "z": 3}, "b": 1, "c": 2})
        );
    }

    #[test]
    fn bundling_labels_each_file_with_its_name() {
        let merged = run(
            vec![json!([1]), json!({"a": 2})],
            preset("Bundle by file name"),
        )
        .unwrap();
        assert_eq!(
            merged.value,
            json!([
                {"file": "f1.json", "data": [1]},
                {"file": "f2.json", "data": {"a": 2}},
            ])
        );
    }

    #[test]
    fn a_custom_filter_can_use_files_and_any_jq() {
        let merged = run(
            vec![json!([{"id": 1}]), json!([{"id": 2}])],
            "[range(0; length) as $i | .[$i][] | . + {from: $files[$i]}]",
        )
        .unwrap();
        assert_eq!(
            merged.value,
            json!([{"id": 1, "from": "f1.json"}, {"id": 2, "from": "f2.json"}])
        );
    }

    #[test]
    fn a_custom_filter_can_merge_by_key() {
        let merged = run(
            vec![
                json!([{"id": 1, "v": "a"}, {"id": 2, "v": "b"}]),
                json!([{"id": 2, "v": "B"}, {"id": 3, "v": "c"}]),
            ],
            "add | group_by(.id) | map(add)",
        )
        .unwrap();
        assert_eq!(
            merged.value,
            json!([{"id": 1, "v": "a"}, {"id": 2, "v": "B"}, {"id": 3, "v": "c"}])
        );
    }

    #[test]
    fn the_other_filters_in_the_docs_work() {
        // docs/tools.md lists these as examples of a custom merge.
        let records = vec![
            json!([{"id": 1, "v": "a"}, {"id": 2, "v": "b"}]),
            json!([{"id": 2, "v": "B"}, {"id": 3, "v": "c"}]),
        ];
        let first_seen = run(records, "add | unique_by(.id)").unwrap();
        assert_eq!(
            first_seen.value,
            json!([{"id": 1, "v": "a"}, {"id": 2, "v": "b"}, {"id": 3, "v": "c"}])
        );

        let skipped = run(
            vec![json!([1]), json!({"not": "an array"}), json!([2])],
            "map(select(type == \"array\")) | add",
        )
        .unwrap();
        assert_eq!(skipped.value, json!([1, 2]));
    }

    #[test]
    fn several_outputs_become_one_array() {
        let merged = run(vec![json!([1, 2]), json!([3])], ".[]").unwrap();
        assert_eq!(merged.value, json!([[1, 2], [3]]));
        assert_eq!(merged.outputs, 2);
    }

    #[test]
    fn mixing_arrays_and_objects_is_the_usual_jq_error() {
        let err = run(vec![json!([1]), json!({"a": 1})], "add").unwrap_err();
        assert!(
            matches!(&err, MergeError::Eval(m) if m.contains("cannot calculate")),
            "{err}"
        );
    }

    #[test]
    fn a_filter_with_no_output_is_reported() {
        let err = run(vec![json!([1])], "empty").unwrap_err();
        assert!(matches!(err, MergeError::NoOutput), "{err}");
    }

    #[test]
    fn a_bad_filter_is_reported_as_a_query_error() {
        let err = run(vec![json!([1])], "def").unwrap_err();
        assert!(matches!(err, MergeError::Query(_)), "{err}");
    }

    #[test]
    fn a_filter_that_never_stops_is_cut_off() {
        let err = run(vec![json!([1])], "repeat(1)").unwrap_err();
        assert!(matches!(err, MergeError::TooManyOutputs), "{err}");
    }

    #[test]
    fn cancelling_stops_the_merge() {
        let cancelled = AtomicBool::new(true);
        let err = merge(vec![json!([1])], &["a.json".into()], "add", &cancelled).unwrap_err();
        assert!(matches!(err, MergeError::Cancelled), "{err}");
    }

    #[test]
    fn numbers_keep_their_exact_digits() {
        let big: Value = serde_json::from_str("[12345678901234567890.5]").unwrap();
        let merged = run(vec![big, json!([1])], "add").unwrap();
        assert_eq!(
            serde_json::to_string(&merged.value).unwrap(),
            "[12345678901234567890.5,1]"
        );
    }

    #[test]
    fn a_preset_is_recognised_by_its_filter_and_a_custom_one_is_not() {
        assert_eq!(preset_for(" add ").map(|p| p.label), Some("Append arrays"));
        assert!(preset_for("add | length").is_none());
    }
}
