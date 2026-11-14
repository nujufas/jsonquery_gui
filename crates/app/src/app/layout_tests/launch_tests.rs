//! The app started with a command line (`cli.rs`): the document it opens and the
//! query it runs on it, documents for Merge's list, and a tool's page filled with
//! files and run.

use super::*;
use crate::cli::{Launch, Query, Source};
use crate::scratch_dir::ScratchDir;
use jsonquery_query::Kind;
use serde_json::json;

const PEOPLE: &str = r#"{"items": [{"name": "a"}, {"name": "b"}]}"#;

/// A folder with these files in it, named and holding what they hold.
fn folder(name: &str, files: &[(&str, &str)]) -> ScratchDir {
    let dir = ScratchDir::new("launch", name);
    for (file, text) in files {
        std::fs::write(dir.join(file), text).unwrap();
    }
    dir
}

/// The app as it starts with `launch`: before its first frame, as `Shared::new` does.
fn started(launch: Launch) -> Harness {
    started_with(launch, Place::Window)
}

/// The same, with the Tools in `place`.
fn started_with(launch: Launch, place: Place) -> Harness {
    Harness::with_app(
        |ctx| {
            let mut app = App::with_context(ctx);
            app.tools.use_place(place);
            app.launch(ctx, launch);
            app
        },
        SCREEN,
        egui::ViewportInfo::default(),
    )
}

fn query(text: &str, language: Option<Kind>) -> Option<Query> {
    Some(Query {
        text: text.to_owned(),
        language,
        file: None,
    })
}

/// The results, once the document has loaded and the query has run on it.
fn results(h: &mut Harness) -> serde_json::Value {
    wait_for(h, "the results", |h| {
        h.app.doc.is_some()
            && !h.app.query_running
            && h.app.results.as_array().is_some_and(|r| !r.is_empty())
    });
    h.app.results.clone()
}

#[test]
fn the_document_is_opened_and_the_query_run_on_it_once_it_has_loaded() {
    let dir = folder("query", &[("data.json", PEOPLE)]);
    let path = dir.join("data.json");
    let mut h = started(Launch {
        open: Some(Source::File(path.clone())),
        query: query(".items[].name", None),
        ..Launch::default()
    });
    assert_eq!(results(&mut h), json!(["a", "b"]));
    assert_eq!(h.app.source_input, path.display().to_string());
    assert_eq!(h.app.query_text, ".items[].name");
    assert_eq!(
        h.app.query_engine, None,
        "the language is told from the query"
    );
    assert_eq!(h.app.last_resolved_engine, Some(Kind::Jq));
}

#[test]
fn the_query_is_run_in_its_language() {
    let dir = folder("language", &[("data.json", PEOPLE)]);
    let mut h = started(Launch {
        open: Some(Source::File(dir.join("data.json"))),
        query: query("items[].name", Some(Kind::JmesPath)),
        ..Launch::default()
    });
    // One answer, a list (where jq gives each name as an answer of its own).
    assert_eq!(results(&mut h), json!([["a", "b"]]));
    assert_eq!(h.app.query_engine, Some(Kind::JmesPath));
}

#[test]
fn a_pointer_shows_the_value_it_points_at() {
    let dir = folder("pointer", &[("data.json", PEOPLE)]);
    let mut h = started(Launch {
        open: Some(Source::File(dir.join("data.json"))),
        query: query("/items/1", Some(Kind::JsonPointer)),
        ..Launch::default()
    });
    assert_eq!(results(&mut h), json!([{"name": "b"}]));
    assert_eq!(h.app.query_engine, Some(Kind::JsonPointer));
}

#[test]
fn a_document_alone_is_opened_with_nothing_run() {
    let dir = folder("alone", &[("data.json", PEOPLE)]);
    let mut h = started(Launch {
        open: Some(Source::File(dir.join("data.json"))),
        ..Launch::default()
    });
    wait_for(&mut h, "the document", |h| h.app.doc.is_some());
    assert_eq!(h.app.query_text, "");
    assert!(!h.app.query_running && h.app.last_resolved_engine.is_none());
}

#[test]
fn a_query_without_a_document_waits_in_the_box() {
    let mut h = started(Launch {
        query: query(".a", None),
        ..Launch::default()
    });
    h.settle();
    assert_eq!(h.app.query_text, ".a");
    assert!(h.app.doc.is_none() && !h.app.query_running && !h.app.run_after_load);
}

#[test]
fn a_document_that_cannot_be_opened_says_why_and_its_query_waits() {
    let dir = folder("missing", &[]);
    let path = dir.join("gone.json");
    let mut h = started(Launch {
        open: Some(Source::File(path.clone())),
        query: query(".a", None),
        ..Launch::default()
    });
    wait_for(&mut h, "the error", |h| h.app.load_error.is_some());
    assert_eq!(
        h.app.source_input,
        path.display().to_string(),
        "what was tried"
    );
    assert_eq!(h.app.query_text, ".a");
    assert!(
        !h.app.run_after_load,
        "a document opened later does not run it"
    );
}

#[test]
fn several_documents_go_to_merges_list_as_when_they_are_dropped() {
    let dir = folder("several", &[("a.json", "[1, 2]"), ("b.json", "[3]")]);
    let h = started(Launch {
        merge: vec![dir.join("a.json"), dir.join("b.json")],
        ..Launch::default()
    });
    assert!(h.app.tools.is_open() && h.app.tools.current_tool() == Tool::Merge);
    assert!(is_drawn(&h, "Files (2)"), "{:?}", h.texts());
    assert!(is_drawn(&h, "a.json") && is_drawn(&h, "b.json"));
    assert!(
        !is_drawn(&h, "[\n  1,\n  2,\n  3\n]"),
        "nothing is merged until asked, as when they are dropped"
    );
}

#[test]
fn merge_merges_the_files() {
    let dir = folder("merge", &[("a.json", "[1, 2]"), ("b.json", "[3]")]);
    let mut h = started(Launch {
        tool: Some((Tool::Merge, vec![dir.join("a.json"), dir.join("b.json")])),
        ..Launch::default()
    });
    wait_for(&mut h, "the merged preview", |h| {
        is_drawn(h, "[\n  1,\n  2,\n  3\n]")
    });
    assert!(h.app.doc.is_none(), "nothing is opened in the main window");
}

#[test]
fn diff_compares_the_left_file_with_the_right() {
    let dir = folder(
        "diff",
        &[
            ("old.json", r#"{"a":1,"b":2}"#),
            ("new.json", r#"{"a":1,"b":3}"#),
        ],
    );
    let mut h = started(Launch {
        tool: Some((Tool::Diff, vec![dir.join("old.json"), dir.join("new.json")])),
        ..Launch::default()
    });
    wait_for(&mut h, "the changes", |h| is_drawn(h, "Changes (1)"));
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Right"),
        r#"{"a":1,"b":3}"#
    );
}

#[test]
fn a_tool_with_a_box_empty_is_filled_and_waits() {
    let dir = folder("one", &[("old.json", r#"{"a":1}"#)]);
    let mut h = started(Launch {
        tool: Some((Tool::Diff, vec![dir.join("old.json")])),
        ..Launch::default()
    });
    h.settle();
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1}"#);
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Right"), "");
    assert!(!is_drawn_containing(&h, "Changes ("), "{:?}", h.texts());
}

#[test]
fn a_tool_with_a_file_that_cannot_be_read_says_so_and_does_not_run() {
    let dir = folder("unread", &[("new.json", r#"{"a":1}"#)]);
    let mut h = started(Launch {
        tool: Some((
            Tool::Diff,
            vec![dir.join("gone.json"), dir.join("new.json")],
        )),
        ..Launch::default()
    });
    h.settle();
    assert!(is_drawn_containing(&h, "gone.json"), "{:?}", h.texts());
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Left"),
        "",
        "not the right's file"
    );
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Right"), r#"{"a":1}"#);
    assert!(!is_drawn_containing(&h, "Changes ("));
}

#[test]
fn validate_checks_the_document_against_the_schema() {
    let dir = folder(
        "validate",
        &[
            ("doc.json", r#"{"age": -1}"#),
            (
                "schema.json",
                r#"{"properties":{"age":{"minimum":0}},"required":["name"]}"#,
            ),
        ],
    );
    let mut h = started(Launch {
        tool: Some((
            Tool::Validate,
            vec![dir.join("doc.json"), dir.join("schema.json")],
        )),
        ..Launch::default()
    });
    wait_for(&mut h, "the problems", |h| is_drawn(h, "2 problems"));
    assert_eq!(
        h.app.tools.box_text(Tool::Validate, "Document"),
        r#"{"age": -1}"#
    );
}

#[test]
fn patch_applies_the_patch_to_the_document() {
    let dir = folder(
        "patch",
        &[
            ("doc.json", r#"{"a":1}"#),
            ("fix.json", r#"[{"op":"add","path":"/b","value":2}]"#),
        ],
    );
    let mut h = started(Launch {
        tool: Some((
            Tool::Patch,
            vec![dir.join("doc.json"), dir.join("fix.json")],
        )),
        ..Launch::default()
    });
    wait_for(&mut h, "the patched document", |h| {
        is_drawn(h, "{\n  \"a\": 1,\n  \"b\": 2\n}")
    });
}

#[test]
fn format_formats_the_file() {
    let dir = folder("format", &[("a.json", r#"{"b":1,"a":[1,2]}"#)]);
    let mut h = started(Launch {
        tool: Some((Tool::Format, vec![dir.join("a.json")])),
        ..Launch::default()
    });
    wait_for(&mut h, "the formatted text", |h| {
        is_drawn(h, "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2\n  ]\n}")
    });
}

#[test]
fn create_schema_makes_the_schema_of_the_file() {
    let dir = folder("create", &[("a.json", r#"{"id": 1, "tags": ["x"]}"#)]);
    let mut h = started(Launch {
        tool: Some((Tool::Create, vec![dir.join("a.json")])),
        ..Launch::default()
    });
    wait_for(&mut h, "the schema", |h| {
        h.app.tools.created_schema().is_some()
    });
    let schema = h.app.tools.created_schema().unwrap();
    assert!(schema.contains("\"tags\""), "{schema}");
}

#[test]
fn with_the_tools_docked_the_tools_page_is_in_front_and_a_query_is_on_its_own_tab() {
    let dir = folder("docked", &[("old.json", "[1]"), ("new.json", "[2]")]);
    let h = started_with(
        Launch {
            tool: Some((Tool::Diff, vec![dir.join("old.json"), dir.join("new.json")])),
            ..Launch::default()
        },
        Place::Docked,
    );
    assert!(h.app.tools.page_in_front() && h.app.tools.current_tool() == Tool::Diff);

    let mut h = started_with(
        Launch {
            open: Some(Source::File(dir.join("old.json"))),
            query: query(".[0]", None),
            ..Launch::default()
        },
        Place::Docked,
    );
    assert!(!h.app.tools.page_in_front(), "the query's tab is in front");
    assert_eq!(results(&mut h), json!([1]));
}
