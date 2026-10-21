//! A query that is put in the query box by a drop: a file that is dropped on the window and
//! is one (what its name ends in says so: `.jq`, `.jsonpath`, `.jmespath`, `.jsonptr`), and a
//! query that is carried out of the workspace's list and put down on the query pane.
//! Anything else that is dropped is opened as the Source, as it always was.

use super::workspace_drag_tests::{let_go, lit, pick_up};
use super::workspace_tests::{folder_of, row_at, start, OPEN};
use super::*;
use jsonquery_query::Kind;

/// A file of a query in each of the four languages: its name, what it holds, and what it is.
const QUERIES: [(&str, &str, Kind); 4] = [
    ("q.jq", ".[] | .name", Kind::Jq),
    ("q.jsonpath", "$.store.book[*].title", Kind::JsonPath),
    ("q.jmespath", "people[?age > `21`].name", Kind::JmesPath),
    ("q.jsonptr", "/a/0/b", Kind::JsonPointer),
];

/// One frame of the main window with the files at `paths` dragged over it.
fn hover(h: &mut Harness, paths: &[&str]) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
        time: Some(h.time),
        hovered_files: paths
            .iter()
            .map(|path| egui::HoveredFile {
                path: Some((*path).into()),
                mime: String::new(),
            })
            .collect(),
        ..Default::default()
    };
    h.time += 1.0 / 60.0;
    let app = &mut h.app;
    let mut out = h.ctx.run_ui(input, |ui| app.show(ui));
    out.textures_delta.clear();
    h.shapes = out.shapes;
}

/// A file called `name` with `text` in it, in a folder of its own that goes with it.
fn file_with(name: &str, text: &str) -> crate::scratch_dir::ScratchFile {
    let file = crate::scratch_dir::ScratchDir::new("query-drop", "file").into_file(name);
    std::fs::write(&file, text).unwrap();
    file
}

// ---- what putting a query in the box does ----------------------------------------------------------

#[test]
fn a_query_that_is_put_in_the_box_shows_the_box_follows_the_caret_and_shuts_the_suggestions() {
    let mut h = Harness::new();
    dock_tools(&mut h, Tool::Format);
    assert!(
        h.app.tools.page_in_front(),
        "the page of a tool is in front"
    );
    h.app.query_suggest.open = true;
    h.app.put_query(Kind::JsonPath, "$.a".to_owned());
    assert_eq!(h.app.query_text, "$.a");
    assert_eq!(h.app.query_engine, Some(Kind::JsonPath));
    assert!(h.app.query_follow_cursor, "the view goes to the caret");
    assert!(!h.app.query_suggest.open, "and the suggestions are shut");
    assert!(!h.app.tools.page_in_front(), "the query tab is in front");
}

#[test]
fn a_query_opened_from_the_pane_brings_the_query_tab_in_front_of_the_page_of_a_tool() {
    let (mut h, file) = start("qd-open-tab", OPEN);
    dock_tools(&mut h, Tool::Format);
    assert!(h.app.tools.page_in_front());
    let at = row_at(&h, "scratch");
    h.double_click(at.x, at.y);
    assert_eq!(h.app.query_text, ".");
    assert!(!h.app.tools.page_in_front(), "the query is in front");
    forget_settings(&file);
}

// ---- a file dropped on the window --------------------------------------------------------------

#[test]
fn a_query_file_dropped_on_the_window_goes_in_the_query_box_in_its_language() {
    for (name, text, kind) in QUERIES {
        let mut h = Harness::new();
        let file = file_with(name, text);
        drop_files(&mut h, &[&*file]);
        assert_eq!(h.app.query_text, text, "{name}");
        assert_eq!(h.app.query_engine, Some(kind), "{name}");
        assert_eq!(
            h.app.source_input, "",
            "{name}: it is not opened as the Source"
        );
        assert!(h.app.doc.is_none(), "{name}");
        assert_eq!(h.app.load_error, None, "{name}");
    }
}

#[test]
fn the_end_of_the_name_is_all_that_says_it_is_a_query_whatever_its_case_and_where_it_is_hidden() {
    for name in ["Q.JQ", "Query.JsonPath", ".hidden.jq", ".jq"] {
        let mut h = Harness::new();
        let file = file_with(name, ".a");
        drop_files(&mut h, &[&*file]);
        assert_eq!(h.app.query_text, ".a", "{name}");
        assert_eq!(h.app.source_input, "", "{name}");
    }
}

#[test]
fn a_file_that_does_not_end_like_a_query_is_opened_as_the_source_as_before() {
    for name in [
        "data.json",
        "data.ndjson",
        "orders.schema.json",
        "bump.patch.json",
        "queries.txt",
        "jq",
        "a.jq.bak",
        "a.jqx",
    ] {
        let mut h = Harness::new();
        let file = file_with(name, "[1]");
        drop_files(&mut h, &[&*file]);
        assert_eq!(
            h.app.source_input,
            file.display().to_string(),
            "{name}: opened"
        );
        assert_eq!(h.app.query_text, "", "{name}: the query box is as it was");
        assert_eq!(h.app.query_engine, None, "{name}");
    }
}

#[test]
fn a_query_file_takes_the_place_of_what_was_in_the_box_and_the_undo_brings_that_back() {
    let mut h = Harness::new();
    let q = h.query_box().center();
    h.click(q.x, q.y);
    h.type_text(".keep");
    // Left for more than a second, the text box keeps it as a step to go back to.
    h.pause(1.5);
    let file = file_with("q.jq", ".new");
    drop_files(&mut h, &[&*file]);
    assert_eq!(h.app.query_text, ".new", "the file replaced it");
    let q = h.query_box().center();
    h.click(q.x, q.y);
    h.key(egui::Key::Z, egui::Modifiers::COMMAND);
    h.settle();
    assert_eq!(h.app.query_text, ".keep", "the text that was there is back");
}

#[test]
fn the_file_may_start_with_a_byte_order_mark_and_the_box_does_not_have_it() {
    let mut h = Harness::new();
    let file = file_with("q.jq", "\u{feff}.name");
    drop_files(&mut h, &[&*file]);
    assert_eq!(h.app.query_text, ".name");
}

#[test]
fn a_query_file_that_cannot_be_used_is_said_and_the_box_is_left_alone() {
    let mut h = Harness::new();
    let q = h.query_box().center();
    h.click(q.x, q.y);
    h.type_text(".keep");

    // One that is not there.
    drop_files(&mut h, &[std::path::Path::new("/nonexistent/gone.jq")]);
    let said = h.app.load_error.clone().expect("it is said");
    assert!(said.starts_with("gone.jq could not be read"), "{said}");
    assert!(
        is_drawn_containing(&h, "Load error: gone.jq"),
        "{:?}",
        h.texts()
    );
    assert_eq!(h.app.query_text, ".keep");

    // One that is not text.
    let binary = crate::scratch_dir::ScratchDir::new("query-drop", "binary").into_file("b.jq");
    std::fs::write(&binary, [0xff_u8, 0xfe, 0x00, 0x80]).unwrap();
    h.app.load_error = None;
    drop_files(&mut h, &[&*binary]);
    let said = h.app.load_error.clone().expect("it is said");
    assert!(said.starts_with("b.jq could not be read"), "{said}");
    assert_eq!(h.app.query_text, ".keep");

    // One that is too big to be a query.
    let big = crate::scratch_dir::ScratchDir::new("query-drop", "big").into_file("big.jq");
    std::fs::write(
        &big,
        vec![b'.'; crate::workspace_pane::QUERY_LIMIT as usize + 1],
    )
    .unwrap();
    h.app.load_error = None;
    drop_files(&mut h, &[&*big]);
    assert_eq!(
        h.app.load_error.as_deref(),
        Some("big.jq is too big to put in a box")
    );
    assert_eq!(h.app.query_text, ".keep");

    // A good one afterwards is used, and what was said of the one before is not said any more.
    let good = file_with("good.jq", ".good");
    assert!(h.app.load_error.is_some(), "the last one is still said");
    drop_files(&mut h, &[&*good]);
    assert_eq!(h.app.query_text, ".good");
    assert_eq!(h.app.load_error, None);
    assert!(
        !is_drawn_containing(&h, "Load error"),
        "and it is not drawn: {:?}",
        h.texts()
    );
}

#[test]
fn a_query_file_that_is_used_leaves_the_document_as_it_was_and_takes_away_what_was_said() {
    let mut h = Harness::new();
    load(&mut h, "[1, 2]");
    let gone = std::path::Path::new("/nonexistent/gone.jq");
    drop_files(&mut h, &[gone]);
    assert!(h.app.load_error.is_some(), "it is said");
    assert!(h.app.doc.is_some(), "the document is there");

    let good = file_with("good.jq", ".good");
    drop_files(&mut h, &[&*good]);
    assert_eq!(h.app.load_error, None, "it is not said any more");
    assert!(h.app.doc.is_some(), "the document is still there");
    assert_eq!(h.app.query_text, ".good");
}

#[test]
fn several_files_are_for_merge_whatever_they_are_and_the_query_box_is_left_alone() {
    let mut h = Harness::new();
    dock_tools_on_jsonquery(&mut h);
    let a = file_with("a.jq", ".a");
    let b = file_with("b.jq", ".b");
    drop_files(&mut h, &[&*a, &*b]);
    assert_eq!(h.app.query_text, "");
    assert!(h.app.tools.page_in_front(), "Merge is in front");
}

#[test]
fn with_the_page_of_a_tool_in_front_a_dropped_file_is_the_pages_and_not_the_query_boxs() {
    let mut h = Harness::new();
    dock_tools(&mut h, Tool::Format);
    let file = file_with("q.jq", ".a");
    drop_files(&mut h, &[&*file]);
    assert_eq!(h.app.query_text, "", "the query box is not in front");
    assert_eq!(h.app.query_engine, None);
}

#[test]
fn a_query_file_dropped_on_the_query_window_that_is_popped_out_goes_in_the_box() {
    let mut h = Harness::new();
    h.app.dock.pop_out(Pane::Query, None, None);
    let file = file_with("q.jmespath", "a.b");
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
        time: Some(h.time),
        dropped_files: vec![std::sync::Arc::new(FakeDroppedFile(file.to_path_buf()))],
        ..Default::default()
    };
    let app = &mut h.app;
    let mut out = h
        .ctx
        .run_ui(input, |ui| app.popped_window_frame(ui, Pane::Query));
    out.textures_delta.clear();
    assert_eq!(h.app.query_text, "a.b");
    assert_eq!(h.app.query_engine, Some(Kind::JmesPath));
    assert_eq!(h.app.source_input, "", "and it is not the Source");
}

// ---- what is said while a file is over the window ----------------------------------------------------

#[test]
fn a_query_file_over_the_window_outlines_the_query_box_and_says_what_it_would_be() {
    let mut h = Harness::new();
    assert!(!has_drop_outline(&h), "nothing at rest");
    hover(&mut h, &["/x/q.jq"]);
    assert!(has_drop_outline(&h), "the box is outlined");
    assert!(is_drawn(&h, "Drop to use as the query"), "{:?}", h.texts());
    assert!(!is_drawn(&h, "Drop to open"));
    // The file is gone: so is the outline.
    h.frame();
    assert!(!has_drop_outline(&h));
}

#[test]
fn a_document_over_the_window_is_not_a_query_and_the_box_is_not_outlined() {
    for paths in [vec!["/x/data.json"], vec!["/x/a.jq", "/x/b.jq"]] {
        let mut h = Harness::new();
        hover(&mut h, &paths);
        assert!(!has_drop_outline(&h), "{paths:?}");
        assert!(!is_drawn(&h, "Drop to use as the query"), "{paths:?}");
    }
    let mut h = Harness::new();
    hover(&mut h, &["/x/data.json"]);
    assert!(is_drawn(&h, "Drop to open"), "{:?}", h.texts());
}

// ---- the workspace's origin of what is in the box ---------------------------------------------------

#[test]
fn a_file_dropped_from_the_workspace_folder_is_the_item_the_box_came_from() {
    let (mut h, file) = start("qd-origin", OPEN);
    let item = folder_of(&file).join("Logs/logins.jmespath");
    drop_files(&mut h, &[&item]);
    assert_eq!(h.app.query_text, "[?ok]");
    assert_eq!(h.app.query_engine, Some(Kind::JmesPath));
    assert!(
        is_drawn(&h, "Logs / logins.jmespath"),
        "the query row says where it came from: {:?}",
        h.texts()
    );
    // Changed since, it says so.
    let q = h.query_box().center();
    h.click(q.x, q.y);
    h.type_text("x");
    h.settle();
    assert!(
        is_drawn_containing(&h, "Logs / logins.jmespath \u{25cf}"),
        "{:?}",
        h.texts()
    );
    forget_settings(&file);
}

/// The toolkit hands over the path of a dropped file with its links resolved, so a workspace
/// that is reached through a link has the files that are let go from it under another name.
#[cfg(unix)]
#[test]
fn a_file_from_a_workspace_that_is_reached_through_a_link_is_the_item_all_the_same() {
    use super::workspace_tests::{put, FILES};

    let file = settings_file("qd-link");
    let real = file
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .to_path_buf();
    let workspace = real.join("jsonquery_gui/workspace");
    for (rel, text) in FILES {
        put(&workspace, rel, text);
    }
    std::fs::write(&file, OPEN).unwrap();
    let link = real.with_file_name(format!(
        "{}-link",
        real.file_name().unwrap().to_string_lossy()
    ));
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut h = Harness::with_store(Store::at(Some(link.join("jsonquery_gui/settings.json"))));
    // What the toolkit gives: the path with the link taken out, which is not under the folder
    // that the pane was told to use.
    let item = workspace
        .join("Logs/logins.jmespath")
        .canonicalize()
        .unwrap();
    assert!(!item.starts_with(link.join("jsonquery_gui/workspace")));
    drop_files(&mut h, &[&item]);
    assert_eq!(h.app.query_text, "[?ok]");
    assert!(
        is_drawn(&h, "Logs / logins.jmespath"),
        "the query row says where it came from: {:?}",
        h.texts()
    );
    let _ = std::fs::remove_file(&link);
    forget_settings(&file);
}

#[test]
fn a_file_from_anywhere_else_is_nobodys_and_the_row_forgets_the_item_it_had() {
    let (mut h, file) = start("qd-nobody", OPEN);
    // What the query row says is in the row of the query box (the details of the picked
    // item, lower down, say the same of it).
    let said_in_the_row = |h: &Harness| {
        let row = h.query_box().top();
        h.texts()
            .iter()
            .any(|(text, at)| text.contains("scratch.jq") && at.y < row)
    };
    let at = row_at(&h, "scratch");
    h.double_click(at.x, at.y);
    assert!(said_in_the_row(&h), "from the workspace: {:?}", h.texts());
    let elsewhere = file_with("other.jq", ".other");
    drop_files(&mut h, &[&*elsewhere]);
    assert_eq!(h.app.query_text, ".other");
    assert!(
        !said_in_the_row(&h),
        "it is not the item's any more: {:?}",
        h.texts()
    );
    forget_settings(&file);
}

// ---- a query carried out of the list ------------------------------------------------------------------

#[test]
fn a_query_carried_onto_the_query_pane_lights_it_and_is_opened_when_it_is_put_down() {
    let (mut h, file) = start("qd-carry", OPEN);
    let from = row_at(&h, "scratch");
    let query = h.query_box().center();
    assert!(lit(&h).is_empty(), "nothing is lit at rest");
    pick_up(&mut h, from, query);
    // The pane is outlined, and the label beside the pointer says what would be done.
    let panel = h.query_panel();
    let outlined = lit(&h);
    assert_eq!(outlined.len(), 1, "{outlined:?}");
    assert!(
        (outlined[0].min - panel.min).length() < 3.0
            && (outlined[0].max - panel.max).length() < 3.0,
        "the pane: {:?} {panel:?}",
        outlined[0]
    );
    assert!(
        is_drawn(&h, "Open “scratch” as the query"),
        "{:?}",
        h.texts()
    );
    assert_eq!(h.app.query_text, "", "not yet");
    let_go(&mut h, query);
    assert_eq!(h.app.query_text, ".");
    assert_eq!(h.app.query_engine, Some(Kind::Jq));
    assert!(lit(&h).is_empty(), "and nothing is lit any more");
    assert!(!is_drawn_containing(&h, "Open “"), "{:?}", h.texts());
    assert_eq!(
        h.app.workspace_pane.selected_path(),
        Some("scratch.jq"),
        "the item that was opened is the one that is picked"
    );
    assert!(
        is_drawn(&h, "scratch.jq"),
        "the query row says where it came from"
    );
    forget_settings(&file);
}

#[test]
fn what_is_opened_by_being_put_down_is_what_double_clicking_it_opens() {
    let (mut h, file) = start("qd-carry-same", OPEN);
    let query = h.query_box().center();
    for (name, text, kind) in [
        ("logins", "[?ok]", Kind::JmesPath),
        ("scratch", ".", Kind::Jq),
    ] {
        let from = row_at(&h, name);
        pick_up(&mut h, from, query);
        let_go(&mut h, query);
        assert_eq!(h.app.query_text, text, "{name}");
        assert_eq!(h.app.query_engine, Some(kind), "{name}");
    }
    // Nothing was moved or changed in the workspace by it.
    let dir = folder_of(&file);
    assert!(dir.join("Logs/logins.jmespath").is_file() && dir.join("scratch.jq").is_file());
    assert_eq!(h.app.workspace_pane.take_actions().len(), 0);
    forget_settings(&file);
}

#[test]
fn what_is_not_a_query_is_not_lit_on_the_query_pane_and_not_opened_there() {
    let (mut h, file) = start("qd-carry-not", OPEN);
    let query = h.query_box().center();
    for name in ["bump", "orders", "Services", "Logs"] {
        let from = row_at(&h, name);
        pick_up(&mut h, from, query);
        assert!(lit(&h).is_empty(), "{name}: {:?}", lit(&h));
        assert!(!is_drawn_containing(&h, "Open “"), "{name}");
        let_go(&mut h, query);
        assert_eq!(h.app.query_text, "", "{name}");
    }
    forget_settings(&file);
}

#[test]
fn a_query_carried_over_the_query_pane_and_taken_back_over_the_list_is_moved_as_before() {
    let (mut h, file) = start("qd-carry-back", OPEN);
    let from = row_at(&h, "scratch");
    let query = h.query_box().center();
    pick_up(&mut h, from, query);
    assert!(is_drawn(&h, "Open “scratch” as the query"));
    // Back over a folder: it is a move that is offered again, and the pane is not lit.
    let services = row_at(&h, "Services");
    super::workspace_drag_tests::move_to(&mut h, query, services);
    assert!(!is_drawn_containing(&h, "Open “"), "{:?}", h.texts());
    assert!(
        is_drawn(&h, "Move “scratch” into “Services”"),
        "{:?}",
        h.texts()
    );
    let_go(&mut h, services);
    assert!(folder_of(&file).join("Services/scratch.jq").is_file());
    assert_eq!(h.app.query_text, "", "and nothing was opened");
    forget_settings(&file);
}

#[test]
fn escape_while_a_query_is_over_the_query_pane_opens_nothing() {
    let (mut h, file) = start("qd-carry-escape", OPEN);
    let from = row_at(&h, "scratch");
    let query = h.query_box().center();
    pick_up(&mut h, from, query);
    h.key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!is_drawn_containing(&h, "Open “"), "{:?}", h.texts());
    assert!(lit(&h).is_empty());
    let_go(&mut h, query);
    assert_eq!(h.app.query_text, "");
    forget_settings(&file);
}

#[test]
fn where_there_is_no_query_pane_in_the_window_a_query_is_not_opened_by_being_put_down() {
    // Popped out into a window of its own.
    let (mut h, file) = start("qd-carry-popped", OPEN);
    let from = row_at(&h, "scratch");
    let query = h.query_box().center();
    h.app.dock.pop_out(Pane::Query, None, None);
    h.settle();
    pick_up(&mut h, from, query);
    assert!(!is_drawn_containing(&h, "Open “"), "{:?}", h.texts());
    let_go(&mut h, query);
    assert_eq!(h.app.query_text, "");
    forget_settings(&file);

    // Behind the page of a tool.
    let (mut h, file) = start("qd-carry-page", OPEN);
    dock_tools(&mut h, Tool::Format);
    let from = row_at(&h, "scratch");
    let middle = h
        .panel("source_panel")
        .map_or(egui::pos2(600.0, 300.0), |r| r.center());
    pick_up(&mut h, from, middle);
    assert!(!is_drawn_containing(&h, "Open “"), "{:?}", h.texts());
    let_go(&mut h, middle);
    assert_eq!(h.app.query_text, "");
    forget_settings(&file);
}
