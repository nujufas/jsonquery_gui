//! Checking a JSON document against a JSON Schema — what the Tools window's
//! Validate schema does.
//!
//! The checking itself is the `jsonschema` crate's (drafts 4, 6, 7, 2019-09 and
//! 2020-12; the draft is taken from the schema's `$schema`, and is the newest
//! when there is none). This module asks it for the problems, a bounded number
//! of them, and turns each into something a window can list: where in the
//! document, what is wrong, and which keyword of the schema said so.
//!
//! The crate is built without its network and file features, on purpose: a
//! schema is something a person pastes in, and a `$ref` in it must not be able to
//! make the app fetch a URL or read a file. A reference has to point inside the
//! schema (`#/$defs/name`, an `$id` or an `$anchor` in it); the meta-schemas of
//! the drafts are built in.

use std::sync::atomic::{AtomicBool, Ordering};

use jsonschema::Draft;
use serde_json::Value;

/// Problems reported at most; there can be many more (a thousand elements each
/// of the wrong type), and a list that long is of no use to anyone.
pub const MAX_PROBLEMS: usize = 1_000;

/// Longest a problem's message is, in characters.
const MAX_MESSAGE_CHARS: usize = 300;

/// One way the document does not satisfy the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// A JSON Pointer to the value in the document that is wrong; empty for the
    /// document itself.
    pub path: String,
    /// What is wrong, in a sentence.
    pub message: String,
    /// The schema keyword that was not satisfied: `required`, `type`,
    /// `minimum`…
    pub keyword: String,
    /// A JSON Pointer to that keyword in the schema.
    pub schema_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The draft of the specification the schema was read as: "Draft 2020-12".
    pub draft: &'static str,
    /// The problems found, at most [`MAX_PROBLEMS`], in the order they were
    /// found. Empty when the document is valid.
    pub problems: Vec<Problem>,
    /// There were more problems than are listed.
    pub more: bool,
}

impl Report {
    pub fn is_valid(&self) -> bool {
        self.problems.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    /// Hold strings to their `format` (`email`, `date-time`, `uuid`…). The
    /// specification leaves that to the validator in recent drafts, and most
    /// people expect it.
    pub check_formats: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The schema can't be used: it is not a valid schema, or it refers to
    /// something that isn't in it. The text says what.
    Schema(String),
    Cancelled,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Schema(message) => f.write_str(message),
            Error::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for Error {}

/// Check `instance` against `schema`.
pub fn validate(
    schema: &Value,
    instance: &Value,
    options: Options,
    cancel: &AtomicBool,
) -> Result<Report, Error> {
    // The draft is worked out here, and handed over, because the validator
    // reports the draft it was *configured* with (the newest, unless told) and
    // not the one it found in `$schema`.
    let draft = Draft::default().detect(schema);
    let mut builder = jsonschema::options().should_validate_formats(options.check_formats);
    if is_known(draft) {
        builder = builder.with_draft(draft);
    }
    let validator = builder
        .build(schema)
        .map_err(|e| Error::Schema(e.to_string()))?;

    let mut problems = Vec::new();
    let mut more = false;
    for error in validator.iter_errors(instance) {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        if problems.len() == MAX_PROBLEMS {
            more = true;
            break;
        }
        problems.push(Problem {
            path: error.instance_path().as_str().to_owned(),
            message: message_of(&error),
            keyword: error.kind().keyword().to_owned(),
            schema_path: error.schema_path().as_str().to_owned(),
        });
    }
    Ok(Report {
        draft: draft_name(draft),
        problems,
        more,
    })
}

/// One of the drafts the library knows by name — not the placeholder it uses
/// for a `$schema` it doesn't recognize.
fn is_known(draft: Draft) -> bool {
    matches!(
        draft,
        Draft::Draft4 | Draft::Draft6 | Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012
    )
}

fn draft_name(draft: Draft) -> &'static str {
    match draft {
        Draft::Draft4 => "Draft 4",
        Draft::Draft6 => "Draft 6",
        Draft::Draft7 => "Draft 7",
        Draft::Draft201909 => "Draft 2019-09",
        Draft::Draft202012 => "Draft 2020-12",
        _ => "a custom meta-schema",
    }
}

/// The error as a sentence. The value that is wrong is part of the library's
/// wording (`-1 is less than the minimum of 0`), which is a help for a number
/// and a flood for an object, so those are left out — the path says where.
fn message_of(error: &jsonschema::ValidationError<'_>) -> String {
    let instance: &Value = error.instance();
    let bulky = match instance {
        Value::Array(_) | Value::Object(_) => true,
        Value::String(text) => text.len() > 200,
        _ => false,
    };
    let mut message = if bulky {
        error.masked().to_string()
    } else {
        error.to_string()
    };
    if let Some((cut, _)) = message.char_indices().nth(MAX_MESSAGE_CHARS) {
        message.truncate(cut);
        message.push('…');
    }
    message
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn check(schema: &Value, instance: &Value) -> Report {
        validate(
            schema,
            instance,
            Options {
                check_formats: true,
            },
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    fn found(report: &Report) -> Vec<(&str, &str)> {
        let mut found: Vec<_> = report
            .problems
            .iter()
            .map(|p| (p.path.as_str(), p.keyword.as_str()))
            .collect();
        found.sort_unstable();
        found
    }

    fn person_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0},
                "tags": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    #[test]
    fn a_document_that_fits_has_no_problems() {
        let report = check(
            &person_schema(),
            &json!({"name": "Ada", "age": 36, "tags": ["x"]}),
        );
        assert!(report.is_valid());
        assert!(!report.more);
        assert_eq!(report.draft, "Draft 2020-12");
    }

    #[test]
    fn each_problem_says_where_it_is_and_which_keyword_found_it() {
        let report = check(
            &person_schema(),
            &json!({"age": -1, "tags": ["ok", 5], "extra": true}),
        );
        assert_eq!(
            found(&report),
            [
                ("", "additionalProperties"),
                ("", "required"),
                ("/age", "minimum"),
                ("/tags/1", "type"),
            ]
        );
        let minimum = report
            .problems
            .iter()
            .find(|p| p.keyword == "minimum")
            .unwrap();
        assert!(minimum.message.contains("-1"), "{}", minimum.message);
        assert!(
            minimum.schema_path.ends_with("/minimum"),
            "{}",
            minimum.schema_path
        );
    }

    #[test]
    fn the_draft_comes_from_the_schema() {
        let schema =
            json!({"$schema": "http://json-schema.org/draft-07/schema#", "type": "string"});
        let report = check(&schema, &json!("x"));
        assert_eq!(report.draft, "Draft 7");

        let schema =
            json!({"$schema": "http://json-schema.org/draft-04/schema#", "type": "string"});
        assert_eq!(check(&schema, &json!("x")).draft, "Draft 4");

        let schema =
            json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "type": "string"});
        assert_eq!(check(&schema, &json!("x")).draft, "Draft 2019-09");
    }

    #[test]
    fn formats_are_checked_when_asked() {
        let schema = json!({"type": "string", "format": "email"});
        let bad = json!("not an email");
        let on = validate(
            &schema,
            &bad,
            Options {
                check_formats: true,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(found(&on), [("", "format")]);
        let off = validate(
            &schema,
            &bad,
            Options {
                check_formats: false,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(off.is_valid());
    }

    #[test]
    fn references_inside_the_schema_work() {
        let schema = json!({
            "$defs": {"id": {"type": "integer", "minimum": 1}},
            "type": "object",
            "properties": {"a": {"$ref": "#/$defs/id"}, "b": {"$ref": "#/$defs/id"}}
        });
        let report = check(&schema, &json!({"a": 1, "b": 0}));
        assert_eq!(found(&report), [("/b", "minimum")]);
    }

    #[test]
    fn a_reference_to_somewhere_else_is_an_error_and_fetches_nothing() {
        let schema = json!({"$ref": "https://example.invalid/schemas/person.json"});
        let error = validate(
            &schema,
            &json!({}),
            Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(matches!(error, Error::Schema(_)), "{error:?}");

        let schema = json!({"$ref": "file:///etc/hostname"});
        let error = validate(
            &schema,
            &json!({}),
            Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(matches!(error, Error::Schema(_)), "{error:?}");
    }

    #[test]
    fn a_schema_that_is_not_one_is_an_error() {
        let error = validate(
            &json!({"type": 5}),
            &json!(1),
            Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(matches!(error, Error::Schema(_)), "{error:?}");
    }

    #[test]
    fn a_meta_schema_it_does_not_know_is_an_error_not_a_guess() {
        let schema = json!({"$schema": "http://example.invalid/my-own-draft", "type": "string"});
        let error = validate(
            &schema,
            &json!("x"),
            Options::default(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(matches!(error, Error::Schema(_)), "{error:?}");
    }

    #[test]
    fn true_and_false_are_schemas() {
        assert!(check(&json!(true), &json!([1])).is_valid());
        assert_eq!(
            found(&check(&json!(false), &json!([1]))),
            [("", "falseSchema")]
        );
    }

    #[test]
    fn numbers_are_held_to_their_exact_value() {
        let schema = json!({"maximum": 100});
        let big: Value = serde_json::from_str("123456789012345678901234567890").unwrap();
        assert_eq!(found(&check(&schema, &big)), [("", "maximum")]);
        let fine: Value = serde_json::from_str("100.0").unwrap();
        assert!(check(&schema, &fine).is_valid());
    }

    #[test]
    fn only_so_many_problems_are_listed() {
        let items: Vec<u32> = (0..1_500).collect();
        let report = check(&json!({"items": {"type": "string"}}), &json!(items));
        assert_eq!(report.problems.len(), MAX_PROBLEMS);
        assert!(report.more);

        let items: Vec<u32> = (0..MAX_PROBLEMS as u32).collect();
        let report = check(&json!({"items": {"type": "string"}}), &json!(items));
        assert_eq!(report.problems.len(), MAX_PROBLEMS);
        assert!(!report.more);
    }

    #[test]
    fn a_problem_does_not_repeat_a_big_value() {
        let schema = json!({"anyOf": [{"type": "string"}, {"type": "number"}]});
        let big: serde_json::Map<String, Value> =
            (0..2_000).map(|i| (format!("key{i}"), json!(i))).collect();
        let report = check(&schema, &Value::Object(big));
        assert_eq!(report.problems.len(), 1);
        assert!(report.problems[0].message.chars().count() <= MAX_MESSAGE_CHARS + 1);
        assert!(!report.problems[0].message.contains("key1999"));

        let long = "x".repeat(10_000);
        let report = check(&json!({"maxLength": 5}), &json!(long));
        assert_eq!(report.problems.len(), 1);
        assert!(report.problems[0].message.chars().count() <= MAX_MESSAGE_CHARS + 1);
    }

    #[test]
    fn paths_are_json_pointers() {
        let schema = json!({"properties": {"a/b": {"properties": {"m~n": {"type": "string"}}}}});
        let report = check(&schema, &json!({"a/b": {"m~n": 1}}));
        assert_eq!(found(&report), [("/a~1b/m~0n", "type")]);
    }

    #[test]
    fn a_cancelled_check_stops() {
        let cancel = AtomicBool::new(true);
        let error = validate(
            &json!({"type": "string"}),
            &json!(1),
            Options::default(),
            &cancel,
        )
        .unwrap_err();
        assert_eq!(error, Error::Cancelled);
    }
}
