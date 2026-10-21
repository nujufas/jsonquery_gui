//! The workspace pane in the whole app: the window as it always was while the pane
//! is not asked for, the pane and the strip that folds it, the list of folders and
//! items and what a click on one does, saving what is in a box with a sample, the
//! ids and SHA-256s that are kept of it all, and the menus, the windows and the
//! settings.

use super::*;
use crate::settings::WorkspaceView;
use crate::workspace::{sha256_hex, Group};
use crate::workspace_pane::{button_id, Action};
use std::path::{Path, PathBuf};

pub(super) const OPEN: &str = r#"{"workspace": {"pane": "open"}}"#;

/// What the workspace holds in most of these tests: two folders, a folder in one, a
/// query with a sample, a schema with a sample, a patch, and a query by itself.
pub(super) const FILES: &[(&str, &str)] = &[
    (
        "Logs/Errors/slow requests.jq",
        "map(select(.duration > 500))",
    ),
    (
        "Logs/Errors/slow requests.sample.json",
        r#"[{"duration": 900}, {"duration": 10}]"#,
    ),
    ("Logs/logins.jmespath", "[?ok]"),
    (
        "Services/orders.schema.json",
        r#"{"type": "object", "required": ["id"]}"#,
    ),
    ("Services/orders.sample.json", "{}"),
    (
        "Services/bump.patch.json",
        r#"[{"op": "add", "path": "/x", "value": 1}]"#,
    ),
    ("scratch.jq", "."),
];

pub(super) fn put(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The app with its settings (which say `settings`) and its workspace in a folder of
/// their own, the workspace holding [`FILES`].
pub(super) fn start(name: &str, settings: &str) -> (Harness, PathBuf) {
    start_with_more(name, settings, &[])
}

/// The same, with these files in the workspace too.
pub(super) fn start_with_more(
    name: &str,
    settings: &str,
    more: &[(&str, &str)],
) -> (Harness, PathBuf) {
    let file = settings_file(name);
    let dir = file.parent().unwrap().join("workspace");
    for (rel, text) in FILES.iter().chain(more) {
        put(&dir, rel, text);
    }
    std::fs::write(&file, settings).unwrap();
    (Harness::with_store(Store::at(Some(file.clone()))), file)
}

/// The workspace's folder, when the settings are in `file`.
pub(super) fn folder_of(file: &Path) -> PathBuf {
    file.parent().unwrap().join("workspace")
}

pub(super) fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Where the pane's button called `name` is.
pub(super) fn button_at(h: &Harness, name: &str) -> egui::Pos2 {
    h.ctx
        .read_response(button_id(name))
        .unwrap_or_else(|| panic!("the {name} button is drawn"))
        .rect
        .center()
}

pub(super) fn click_button(h: &mut Harness, name: &str) {
    let at = button_at(h, name);
    h.click(at.x, at.y);
}

/// Where the row called `name` is: the top-most text of that name (what is picked is
/// also named under the list).
pub(super) fn row_at(h: &Harness, name: &str) -> egui::Pos2 {
    text_rects(&h.shapes)
        .into_iter()
        .filter(|(t, _)| t == name)
        .map(|(_, rect)| rect.center())
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap_or_else(|| panic!("the row {name:?} is drawn: {:?}", h.texts()))
}

/// Click the row called `name`.
pub(super) fn pick(h: &mut Harness, name: &str) {
    let at = row_at(h, name);
    h.click(at.x, at.y);
}

/// Click the lowest of the texts drawn as `text`: a window's button is under its title.
pub(super) fn press_last(h: &mut Harness, text: &str) {
    let rect = text_rects(&h.shapes)
        .into_iter()
        .filter(|(t, _)| t == text)
        .map(|(_, rect)| rect)
        .max_by(|a, b| a.center().y.total_cmp(&b.center().y))
        .unwrap_or_else(|| panic!("{text:?} is drawn: {:?}", h.texts()));
    h.click(rect.center().x, rect.center().y);
}

/// Click at the beginning of the text drawn as `text`, not in its middle: a path in a box is as
/// long as the folder the test is in, and where that is long (the temp folder of macOS) the
/// middle of it is under the button beside the box.
pub(super) fn press_start(h: &mut Harness, text: &str) {
    let rect = text_rects(&h.shapes)
        .into_iter()
        .find(|(t, _)| t == text)
        .unwrap_or_else(|| panic!("{text:?} is drawn: {:?}", h.texts()))
        .1;
    h.click(rect.min.x + 4.0, rect.center().y);
}

/// Click the text that begins with `part`.
pub(super) fn press_starting(h: &mut Harness, part: &str) {
    let rect = text_rects(&h.shapes)
        .into_iter()
        .find(|(t, _)| t.starts_with(part))
        .unwrap_or_else(|| panic!("a text starting {part:?} is drawn: {:?}", h.texts()))
        .1;
    h.click(rect.center().x, rect.center().y);
}

/// Click the first text that is one of these (or contains one of these, for the ones
/// that end in `*`).
pub(super) fn press_one_of(h: &mut Harness, texts: &[&str]) {
    let rect = text_rects(&h.shapes)
        .into_iter()
        .find(|(t, _)| {
            texts.iter().any(|wanted| match wanted.strip_suffix('*') {
                Some(part) => t.contains(part),
                None => t == wanted,
            })
        })
        .unwrap_or_else(|| panic!("one of {texts:?} is drawn: {:?}", h.texts()))
        .1;
    h.click(rect.center().x, rect.center().y);
}

fn ctrl(key: egui::Key) -> (egui::Key, egui::Modifiers) {
    (key, egui::Modifiers::COMMAND)
}

/// Select what is in the box that has the focus, and type `text` over it.
pub(super) fn type_over(h: &mut Harness, text: &str) {
    let (key, modifiers) = ctrl(egui::Key::A);
    h.key(key, modifiers);
    h.type_text(text);
}

pub(super) fn index_of(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&read(dir, ".jsonquery/index.json")).expect("the index is JSON")
}

// ---- the window as it was --------------------------------------------------------------

#[test]
fn the_pane_is_not_there_until_it_is_asked_for_and_nothing_of_the_workspace_is_read() {
    let (h, file) = start("ws-hidden", "{}");
    assert!(!is_drawn(&h, "Workspace"));
    assert!(h.panel("workspace_pane").is_none() && h.panel("workspace_rail").is_none());
    assert!(h.app.workspace_pane.read_workspace().is_none(), "not read");
    assert!(
        !folder_of(&file).join(".jsonquery").exists(),
        "and nothing was made in it"
    );
    // The window is as it has always been.
    assert!((h.query_panel().bottom() - LEGACY_QUERY_PANEL_BOTTOM).abs() < 0.2);
    // The Query row has no button to save in the workspace: as many "Save…" as there
    // are in an app that has no workspace.
    let saves = |h: &Harness| h.texts().iter().filter(|(t, _)| t == "Save…").count();
    assert_eq!(saves(&h), saves(&Harness::new()));
    forget_settings(&file);
}

#[test]
fn a_button_in_the_status_bar_shows_the_pane_and_hides_it_again_and_moves_nothing() {
    let (mut h, file) = start("ws-toggle", "{}");
    let gear = rect_of(&h, "⚙");
    let bar = h.panel("status_bar").expect("the status bar");
    let at = button_at(&h, "status");
    assert!(at.x < gear.left(), "left of the ⚙: {at:?} {gear:?}");
    assert!((at.y - bar.center().y).abs() < 6.0, "in the status bar");

    h.click(at.x, at.y);
    assert_eq!(h.app.settings.interface.workspace, WorkspaceView::Open);
    assert!(is_drawn(&h, "Workspace"));
    assert_eq!(rect_of(&h, "⚙"), gear, "the ⚙ did not move");
    assert!(
        h.app.workspace_pane.read_workspace().is_some(),
        "now it is read"
    );

    h.click(at.x, at.y);
    assert_eq!(h.app.settings.interface.workspace, WorkspaceView::Hidden);
    assert!(!is_drawn(&h, "Workspace"));
    assert!(h.panel("workspace_rail").is_none());
    forget_settings(&file);
}

#[test]
fn the_button_is_at_the_left_end_of_the_status_bar_with_the_status_text_after_it() {
    let (mut h, file) = start("ws-left", "{}");
    load(&mut h, "[1, 2]");
    let bar = h.panel("status_bar").expect("the status bar");
    let button = h
        .ctx
        .read_response(button_id("status"))
        .expect("the button is drawn")
        .rect;
    let said = |h: &Harness, start: &str| {
        h.texts()
            .into_iter()
            .find(|(t, _)| t.starts_with(start))
            .unwrap_or_else(|| panic!("{start:?} is drawn: {:?}", h.texts()))
            .1
    };
    // At the left end of the bar, level with what the bar says, which begins after it.
    assert!(button.left() - bar.left() < 12.0, "{button:?} {bar:?}");
    let text = said(&h, "Parsed in ");
    assert!(text.x > button.right(), "{text:?} after {button:?}");
    // (`text` is where the text starts: its top is within the button's height.)
    assert!(
        text.y >= button.top() - 2.0 && text.y <= button.bottom(),
        "{button:?} {text:?}"
    );
    // The buttons at the right are the info, the website and the settings, as they were,
    // with nothing of the workspace left among them.
    let (info, globe, gear) = (said(&h, "ℹ"), said(&h, "🌐"), said(&h, "⚙"));
    assert!(info.x > SCREEN.x - 40.0, "the ℹ is in the corner: {info:?}");
    assert!(info.x - globe.x < 30.0 && globe.x - gear.x < 30.0);
    assert!(gear.x > text.x + 100.0, "far from the left end: {gear:?}");

    // With a tool's page in front, it is the page's text, after the same button.
    dock_tools(&mut h, Tool::Format);
    let docked = h
        .ctx
        .read_response(button_id("status"))
        .expect("the button is drawn")
        .rect;
    assert_eq!(docked.left_top(), button.left_top(), "it did not move");
    forget_settings(&file);
}

#[test]
fn a_long_status_text_wraps_beside_the_button_and_leaves_it_where_it_is() {
    let (mut h, file) = start("ws-left-long", "{}");
    let button = |h: &Harness| {
        h.ctx
            .read_response(button_id("status"))
            .expect("the button is drawn")
            .rect
    };
    let before = button(&h);
    let bar_before = h.panel("status_bar").expect("the status bar");
    h.app.load_error = Some("a very long reason that goes on and on ".repeat(30));
    h.settle();
    let bar = h.panel("status_bar").expect("the status bar");
    assert!(
        bar.height() > bar_before.height() + 15.0,
        "it wrapped: {bar:?}"
    );
    // The first line is beside the button, and the lines under it keep clear of it.
    assert_eq!(
        button(&h).left(),
        before.left(),
        "the button stays at the left"
    );
    let first = h
        .texts()
        .into_iter()
        .filter(|(t, at)| t.starts_with("Load error: a very long") && at.y > bar.top() - 1.0)
        .map(|(_, at)| at)
        .next()
        .expect("the text is drawn");
    assert!(first.x > button(&h).right(), "{first:?} {:?}", button(&h));
    forget_settings(&file);
}

#[test]
fn how_much_of_the_pane_is_shown_is_kept_and_the_pane_opens_as_it_was_left() {
    let (mut h, file) = start("ws-keep-view", "{}");
    let at = button_at(&h, "status");
    h.click(at.x, at.y);
    h.pause(1.0);
    assert_eq!(
        kept(&file).unwrap()["workspace"],
        serde_json::json!({"pane": "open"})
    );
    click_button(&mut h, "fold");
    h.pause(1.0);
    assert_eq!(
        kept(&file).unwrap()["workspace"],
        serde_json::json!({"pane": "collapsed"})
    );
    let rail = h.panel("workspace_rail").expect("the strip");
    assert!(rail.width() < 40.0, "{rail:?}");
    assert!(!is_drawn(&h, "Workspace"));
    assert!(h.ctx.read_response(button_id("fold")).is_none() || !is_drawn(&h, "Filter by name…"));
    drop(h);

    // The next run starts with the strip, and opening it puts the pane back.
    let mut h = Harness::with_store(Store::at(Some(file.clone())));
    assert!(h.panel("workspace_rail").is_some());
    click_button(&mut h, "unfold");
    assert!(is_drawn(&h, "Workspace"));
    assert_eq!(h.app.settings.interface.workspace, WorkspaceView::Open);
    forget_settings(&file);
}

#[test]
fn the_pane_stands_at_the_left_from_the_top_of_the_window_to_the_status_bar() {
    let (h, file) = start("ws-place", OPEN);
    let pane = h.panel("workspace_pane").expect("the pane");
    let bar = h.panel("status_bar").expect("the status bar");
    assert!(pane.left() < 1.0 && pane.top() < 1.0, "{pane:?}");
    assert!((pane.bottom() - bar.top()).abs() < 1.0, "{pane:?} {bar:?}");
    assert!(
        (pane.width() - workspace_pane::DEFAULT_WIDTH).abs() < 20.0,
        "the usual width: {pane:?}"
    );
    // What the window holds is beside it.
    let query = h.query_panel();
    assert!(
        (query.left() - pane.right()).abs() < 1.5,
        "{query:?} {pane:?}"
    );
    assert!(query.right() > SCREEN.x - 2.0);
    forget_settings(&file);
}

#[test]
fn with_the_tools_docked_the_pane_is_under_the_row_of_tabs() {
    let (mut h, file) = start("ws-docked", OPEN);
    dock_tools(&mut h, Tool::Validate);
    let tabs = h.panel("toolbar").expect("the row of tabs");
    let pane = h.panel("workspace_pane").expect("the pane");
    assert!(
        (pane.top() - tabs.bottom()).abs() < 1.5,
        "the pane starts under the tabs: {pane:?} {tabs:?}"
    );
    assert!(
        tabs.width() > SCREEN.x - 2.0,
        "and the tabs go across: {tabs:?}"
    );
    forget_settings(&file);
}

#[test]
fn the_pane_is_as_wide_as_it_was_dragged_to_and_that_is_kept() {
    let (mut h, file) = start("ws-width", OPEN);
    let pane = h.panel("workspace_pane").unwrap();
    h.drag_x(pane.right(), 300.0, pane.right() + 80.0);
    h.pause(1.0);
    let now = h.panel("workspace_pane").unwrap().width();
    assert!(
        now > pane.width() + 60.0,
        "wider: {now} against {}",
        pane.width()
    );
    let width = kept(&file).unwrap()["workspace"]["width"]
        .as_f64()
        .expect("a width");
    assert!(
        (width - f64::from(now)).abs() < 1.5,
        "kept {width}, is {now}"
    );
    drop(h);
    let h = Harness::with_store(Store::at(Some(file.clone())));
    assert!((h.panel("workspace_pane").unwrap().width() - now).abs() < 2.0);
    forget_settings(&file);
}

#[test]
fn the_header_has_room_for_its_title_and_its_buttons_at_the_least_width_and_at_any_width_beyond() {
    // At 180 points the 💾 was drawn over the end of "Workspace".
    for width in [
        10.0_f32,
        130.0,
        180.0,
        crate::workspace_pane::MIN_WIDTH,
        256.0,
        400.0,
    ] {
        let settings = format!(r#"{{"workspace": {{"pane": "open", "width": {width}}}}}"#);
        let (mut h, file) = start("ws-header-room", &settings);
        h.settle();
        let pane = h.panel("workspace_pane").expect("the pane");
        assert!(
            pane.width() >= crate::workspace_pane::MIN_WIDTH - 0.5,
            "a pane of {width} is {} wide, less than the least",
            pane.width()
        );
        let title = text_rects(&h.shapes)
            .into_iter()
            .find(|(t, _)| t == "Workspace")
            .expect("the title is drawn")
            .1;
        // (the 💾 is a button with a letter in it, the others are drawn pictures)
        let floppy = text_rects(&h.shapes)
            .into_iter()
            .find(|(t, _)| t == "\u{1f4be}")
            .expect("the 💾 is drawn")
            .1;
        let leftmost_button = ["new folder", "menu", "fold"]
            .iter()
            .map(|name| button_at(&h, name).x - 12.0)
            .fold(floppy.left() - 4.0, f32::min);
        assert!(
            title.right() + 3.0 <= leftmost_button,
            "at {width}: the title ends at {} and the buttons begin at {leftmost_button}",
            title.right()
        );
        forget_settings(&file);
    }
}

#[test]
fn in_a_window_too_narrow_for_it_only_the_strip_is_shown() {
    let file = settings_file("ws-narrow");
    let dir = folder_of(&file);
    put(&dir, "a.jq", ".");
    std::fs::write(&file, OPEN).unwrap();
    let mut h = Harness::with_store_in(
        Store::at(Some(file.clone())),
        egui::vec2(800.0, 700.0),
        egui::ViewportInfo::default(),
    );
    assert!(h.panel("workspace_pane").is_none());
    assert!(h.panel("workspace_rail").is_some(), "the strip is there");
    assert!(!is_drawn(&h, "Workspace"));
    // Its button does nothing while there is no room.
    click_button(&mut h, "unfold");
    assert!(h.panel("workspace_pane").is_none());
    forget_settings(&file);
}

// ---- the list -----------------------------------------------------------------------------

#[test]
fn the_list_has_the_top_folders_open_and_the_rest_shut_and_a_click_opens_or_shuts_one() {
    let (mut h, file) = start("ws-list", OPEN);
    for name in [
        "Logs", "Services", "Errors", "logins", "bump", "orders", "scratch",
    ] {
        assert!(is_drawn(&h, name), "{name}: {:?}", h.texts());
    }
    assert!(!is_drawn(&h, "slow requests"), "Errors is shut");
    // The folders come first, then the items, and every one is under the one it is in.
    assert!(rect_of(&h, "Logs").top() < rect_of(&h, "Errors").top());
    assert!(rect_of(&h, "Errors").top() < rect_of(&h, "logins").top());
    assert!(
        rect_of(&h, "Errors").left() > rect_of(&h, "Logs").left(),
        "in it"
    );
    assert!(rect_of(&h, "scratch").left() <= rect_of(&h, "Logs").left() + 1.0);

    pick(&mut h, "Errors");
    assert!(is_drawn(&h, "slow requests"));
    assert!(rect_of(&h, "slow requests").left() > rect_of(&h, "Errors").left());
    h.pause(1.0);
    pick(&mut h, "Errors");
    assert!(!is_drawn(&h, "slow requests"));
    // What kind each is, and that it has a sample, is in a tag.
    assert!(is_drawn(&h, "JM") && is_drawn(&h, "schema") && is_drawn(&h, "patch"));
    forget_settings(&file);
}

#[test]
fn what_is_typed_in_the_filter_narrows_the_list_and_the_chips_narrow_it_to_a_sort() {
    let (mut h, file) = start("ws-filter", OPEN);
    press(&mut h, "Filter by name…");
    h.type_text("slow");
    h.settle();
    assert!(
        is_drawn(&h, "slow requests"),
        "the folders it is in are open"
    );
    assert!(is_drawn(&h, "Errors") && is_drawn(&h, "Logs"));
    assert!(!is_drawn(&h, "scratch") && !is_drawn(&h, "orders"));
    let (key, modifiers) = ctrl(egui::Key::A);
    h.key(key, modifiers);
    h.key(egui::Key::Backspace, egui::Modifiers::NONE);
    h.settle();
    assert!(is_drawn(&h, "scratch"));

    press(&mut h, "Schemas");
    assert!(is_drawn(&h, "orders"));
    assert!(!is_drawn(&h, "scratch") && !is_drawn(&h, "bump") && !is_drawn(&h, "logins"));
    press(&mut h, "Patches");
    assert!(is_drawn(&h, "bump") && !is_drawn(&h, "orders"));
    press(&mut h, "Docs");
    assert!(
        is_drawn_containing(&h, "Nothing here matches"),
        "{:?}",
        h.texts()
    );
    press(&mut h, "All");
    assert!(is_drawn(&h, "scratch") && is_drawn(&h, "orders"));
    forget_settings(&file);
}

#[test]
fn a_page_of_the_tools_narrows_the_list_to_what_it_uses_and_the_first_tab_to_everything() {
    let (mut h, file) = start("ws-chips", OPEN);
    dock_tools(&mut h, Tool::Validate);
    assert_eq!(h.app.workspace_pane.group(), Group::Schemas);
    assert!(is_drawn(&h, "orders") && !is_drawn(&h, "scratch"));
    open_tool(&mut h, Tool::Patch);
    let ctx = h.ctx.clone();
    h.app.tools.open_on(&ctx, Tool::Patch);
    h.settle();
    assert_eq!(h.app.workspace_pane.group(), Group::Patches);
    assert!(is_drawn(&h, "bump") && !is_drawn(&h, "orders"));
    press(&mut h, "Json Query");
    assert_eq!(h.app.workspace_pane.group(), Group::All);
    assert!(is_drawn(&h, "scratch"));
    forget_settings(&file);
}

#[test]
fn a_click_picks_an_item_and_says_what_it_holds_and_a_double_click_opens_it() {
    let (mut h, file) = start("ws-pick", OPEN);
    pick(&mut h, "scratch");
    assert_eq!(h.app.workspace_pane.selected_path(), Some("scratch.jq"));
    assert!(is_drawn(&h, "scratch.jq"), "its file: {:?}", h.texts());
    assert!(is_drawn(&h, "No sample") && is_drawn(&h, "Open"));
    assert!(h.app.query_text.is_empty(), "a click does not open it");

    pick(&mut h, "orders");
    assert!(is_drawn(&h, "Use as schema") && is_drawn(&h, "With its sample"));
    assert!(is_drawn_containing(&h, "Sample: orders.sample.json"));

    let at = row_at(&h, "scratch");
    h.double_click(at.x, at.y);
    assert_eq!(h.app.query_text, ".");
    assert_eq!(h.app.query_engine, Some(jsonquery_query::Kind::Jq));
    forget_settings(&file);
}

#[test]
fn an_item_is_opened_in_the_language_its_end_says_and_the_query_row_says_where_it_came_from() {
    let (mut h, file) = start("ws-open-engine", OPEN);
    // The query row has the button to save, and nothing to say yet.
    assert!(is_drawn(&h, "Save…"));
    let at = row_at(&h, "logins");
    h.double_click(at.x, at.y);
    assert_eq!(h.app.query_text, "[?ok]");
    assert_eq!(h.app.query_engine, Some(jsonquery_query::Kind::JmesPath));
    assert!(
        is_drawn_containing(&h, "Logs / logins.jmespath"),
        "{:?}",
        h.texts()
    );
    assert!(!is_drawn_containing(&h, "\u{25cf}"), "not changed yet");
    // Edited, it says so.
    h.app.query_text.push_str(" | x");
    h.settle();
    assert!(
        is_drawn_containing(&h, "Logs / logins.jmespath \u{25cf}"),
        "{:?}",
        h.texts()
    );
    forget_settings(&file);
}

#[test]
fn open_with_sample_opens_the_sample_as_the_source_and_runs_the_query() {
    let (mut h, file) = start("ws-open-sample", OPEN);
    pick(&mut h, "Errors");
    pick(&mut h, "slow requests");
    press(&mut h, "Open with sample");
    assert_eq!(h.app.query_text, "map(select(.duration > 500))");
    wait_for(&mut h, "the query to have run on the sample", |h| {
        h.app.doc.is_some() && !h.app.query_running && h.app.results_count_so_far == 1
    });
    assert!(
        h.app.source_input.ends_with("slow requests.sample.json"),
        "{}",
        h.app.source_input
    );
    assert_eq!(
        h.app.results,
        serde_json::json!([[{"duration": 900}]]),
        "the sample was queried"
    );
    forget_settings(&file);
}

#[test]
fn a_schema_goes_in_the_schema_box_of_validate_with_its_sample_in_the_document_box_and_is_checked()
{
    let (mut h, file) = start("ws-schema", OPEN);
    dock_tools(&mut h, Tool::Validate);
    pick(&mut h, "orders");
    press(&mut h, "Use as schema");
    assert_eq!(
        h.app.tools.box_text(Tool::Validate, "Schema"),
        r#"{"type": "object", "required": ["id"]}"#
    );
    assert_eq!(
        h.app.tools.box_text(Tool::Validate, "Document"),
        "",
        "not the sample"
    );
    press(&mut h, "With its sample");
    assert_eq!(h.app.tools.box_text(Tool::Validate, "Document"), "{}");
    // `{}` has no `id`: one problem.
    wait_for(&mut h, "the check", |h| is_drawn_containing(h, "1 problem"));
    forget_settings(&file);
}

#[test]
fn a_document_is_opened_as_the_source_and_a_patch_goes_in_the_patch_box() {
    let (mut h, file) = start_with_more("ws-patch", OPEN, &[("Data/rows.json", r#"[{"a": 1}]"#)]);
    // A document has no sample to open, and no box: it is the Source.
    pick(&mut h, "rows");
    assert!(is_drawn(&h, "Open as Source"), "{:?}", h.texts());
    assert!(is_drawn_containing(&h, "Document ·"), "{:?}", h.texts());
    let at = row_at(&h, "rows");
    h.double_click(at.x, at.y);
    wait_for(&mut h, "the document", |h| h.app.doc.is_some());
    assert!(
        h.app.source_input.ends_with("rows.json"),
        "{}",
        h.app.source_input
    );

    pick(&mut h, "bump");
    assert!(is_drawn(&h, "Use as patch"));
    press(&mut h, "Use as patch");
    assert_eq!(
        h.app.tools.box_text(Tool::Patch, "Patch"),
        r#"[{"op": "add", "path": "/x", "value": 1}]"#
    );
    forget_settings(&file);
}

// ---- saving ---------------------------------------------------------------------------------

#[test]
fn a_query_is_saved_with_a_name_in_a_folder_and_what_is_kept_of_it_is_an_id_and_a_sha() {
    let (mut h, file) = start("ws-save", OPEN);
    let dir = folder_of(&file);
    h.app.query_text = ".name | ascii_downcase".to_owned();
    h.settle();
    press(&mut h, "💾");
    assert!(is_drawn(&h, "Save in the workspace"));
    h.type_text("lower names");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert!(!is_drawn(&h, "Save in the workspace"), "the window closed");
    assert_eq!(read(&dir, "lower names.jq"), ".name | ascii_downcase");
    assert!(is_drawn(&h, "lower names"), "it is in the list");
    assert_eq!(h.app.workspace_pane.selected_path(), Some("lower names.jq"));
    assert!(
        is_drawn_containing(&h, "lower names.jq"),
        "the query row says where it is"
    );
    assert!(h
        .app
        .workspace_pane
        .notice_text()
        .is_some_and(|t| t.contains("Saved")));

    // The id and the sha of it are kept, where the user does not see them.
    let index = index_of(&dir);
    let entry = &index["items"]["lower names.jq"];
    let id = entry["id"].as_str().expect("an id");
    assert_eq!(id.len(), 36, "{id}");
    assert_eq!(id.as_bytes()[14], b'4', "a version 4 UUID: {id}");
    assert_eq!(entry["sha256"], sha256_hex(b".name | ascii_downcase"));
    assert_eq!(entry["bytes"], ".name | ascii_downcase".len());
    // Every item of the workspace has its own.
    let items = index["items"].as_object().unwrap();
    assert_eq!(
        items.len(),
        FILES
            .iter()
            .filter(|(rel, _)| !rel.ends_with(".sample.json"))
            .count()
            + 1
    );
    let ids: std::collections::HashSet<&str> =
        items.values().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), items.len(), "no two alike");
    forget_settings(&file);
}

#[test]
fn a_query_that_came_from_the_workspace_is_saved_back_over_itself_and_keeps_its_id() {
    let (mut h, file) = start("ws-save-back", OPEN);
    let dir = folder_of(&file);
    let at = row_at(&h, "scratch");
    h.double_click(at.x, at.y);
    let before = index_of(&dir)["items"]["scratch.jq"].clone();
    h.app.query_text = ". | length".to_owned();
    h.settle();
    press(&mut h, "💾");
    // Its name and its folder are in the window, and it says that it replaces.
    assert!(is_drawn(&h, "Replace"), "{:?}", h.texts());
    assert!(is_drawn_containing(&h, "saving replaces it"));
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert_eq!(read(&dir, "scratch.jq"), ". | length");
    let after = index_of(&dir)["items"]["scratch.jq"].clone();
    assert_eq!(after["id"], before["id"], "the same item");
    assert_ne!(after["sha256"], before["sha256"], "what it holds changed");
    assert_eq!(after["sha256"], sha256_hex(b". | length"));
    assert!(
        !is_drawn_containing(&h, "\u{25cf}"),
        "and it is no longer changed"
    );
    forget_settings(&file);
}

#[test]
fn a_query_is_saved_with_the_source_as_its_sample() {
    let (mut h, file) = start("ws-save-sample", OPEN);
    let dir = folder_of(&file);
    load(&mut h, r#"{"a": [1, 2, 3]}"#);
    h.app.query_text = ".a".to_owned();
    h.settle();
    press(&mut h, "💾");
    h.type_text("the as");
    press_starting(&mut h, "Keep the Source as its sample");
    press_last(&mut h, "Save");
    wait_for(&mut h, "the sample", |_| {
        dir.join("the as.sample.json").exists()
    });
    h.pause(0.5);
    assert_eq!(read(&dir, "the as.jq"), ".a");
    let sample: serde_json::Value =
        serde_json::from_str(&read(&dir, "the as.sample.json")).unwrap();
    assert_eq!(sample, serde_json::json!({"a": [1, 2, 3]}));
    let ws = h.app.workspace_pane.read_workspace().unwrap();
    let item = ws.item("the as.jq").expect("it is in the workspace");
    let kept = item.sample.as_ref().expect("with its sample");
    assert_eq!(
        kept.sha256,
        sha256_hex(read(&dir, "the as.sample.json").as_bytes())
    );
    assert_eq!(
        index_of(&dir)["items"]["the as.jq"]["sample"]["sha256"],
        kept.sha256
    );
    // The staged file is gone, and the status bar did not say that a file was saved.
    let staged: Vec<_> = std::fs::read_dir(dir.join(".jsonquery"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("staged-"))
        .collect();
    assert!(staged.is_empty(), "{staged:?}");
    assert!(h.app.last_saved.is_none(), "{:?}", h.app.last_saved);
    forget_settings(&file);
}

#[test]
fn the_source_is_saved_as_a_document_of_the_workspace_with_an_id_and_a_sha() {
    let (mut h, file) = start("ws-save-document", OPEN);
    let dir = folder_of(&file);
    load(&mut h, r#"{"a": [1, 2, 3]}"#);
    h.app.query_text.clear();
    h.settle();
    press(&mut h, "💾");
    // With nothing in the query box, what is offered is the document that is open.
    assert!(is_drawn(&h, "Save in the workspace"));
    h.type_text("the doc");
    press_last(&mut h, "Save");
    wait_for(&mut h, "the document", |_| {
        dir.join("the doc.json").exists()
    });
    h.pause(0.5);
    let saved: serde_json::Value = serde_json::from_str(&read(&dir, "the doc.json")).unwrap();
    assert_eq!(saved, serde_json::json!({"a": [1, 2, 3]}));
    let ws = h.app.workspace_pane.read_workspace().unwrap();
    let item = ws.item("the doc.json").expect("it is in the workspace");
    assert!(item.sample.is_none(), "a document has no sample");
    let entry = &index_of(&dir)["items"]["the doc.json"];
    assert_eq!(entry["id"], item.id);
    assert_eq!(
        entry["sha256"],
        sha256_hex(read(&dir, "the doc.json").as_bytes())
    );
    let staged: Vec<_> = std::fs::read_dir(dir.join(".jsonquery"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("staged-"))
        .collect();
    assert!(staged.is_empty(), "{staged:?}");
    forget_settings(&file);
}

#[test]
fn a_name_that_cannot_be_a_file_name_is_said_and_nothing_is_saved() {
    let (mut h, file) = start("ws-save-bad", OPEN);
    let dir = folder_of(&file);
    h.app.query_text = ".x".to_owned();
    h.settle();
    press(&mut h, "💾");
    h.type_text("a/b");
    h.settle();
    assert!(
        is_drawn_containing(&h, "A name can't have any of"),
        "{:?}",
        h.texts()
    );
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert!(is_drawn(&h, "Save in the workspace"), "the window stays");
    assert!(!dir.join("a").exists() && !dir.join("a/b.jq").exists());
    // And an empty box has nothing to save.
    press_last(&mut h, "Cancel");
    h.app.query_text.clear();
    h.settle();
    press(&mut h, "💾");
    assert!(is_drawn(&h, "Save in the workspace"));
    assert!(!is_drawn(&h, "Replace"));
    forget_settings(&file);
}

#[test]
fn a_schema_is_saved_from_the_validate_box_with_the_document_box_as_its_sample() {
    let (mut h, file) = start("ws-save-schema", OPEN);
    let dir = folder_of(&file);
    dock_tools(&mut h, Tool::Validate);
    h.app
        .tools
        .fill(Tool::Validate, "Schema", r#"{"type": "string"}"#);
    h.app.tools.fill(Tool::Validate, "Document", r#""text""#);
    h.settle();
    press(&mut h, "💾");
    // The page in front says what is saved.
    assert!(
        is_drawn_containing(&h, "Keep the Document box of Validate"),
        "{:?}",
        h.texts()
    );
    h.type_text("strings");
    press_starting(&mut h, "Keep the Document box of Validate");
    press_last(&mut h, "Save");
    assert_eq!(read(&dir, "strings.schema.json"), r#"{"type": "string"}"#);
    assert_eq!(read(&dir, "strings.sample.json"), r#""text""#);
    let entry = &index_of(&dir)["items"]["strings.schema.json"];
    assert_eq!(entry["sha256"], sha256_hex(br#"{"type": "string"}"#));
    assert!(entry["id"].as_str().is_some_and(|id| id.len() == 36));
    forget_settings(&file);
}

#[test]
fn a_folder_is_made_from_the_pane_inside_the_one_picked() {
    let (mut h, file) = start("ws-new-folder", OPEN);
    let dir = folder_of(&file);
    pick(&mut h, "Services");
    click_button(&mut h, "new folder");
    assert!(is_drawn(&h, "New folder"));
    h.type_text("Billing");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert!(dir.join("Services/Billing").is_dir());
    assert!(is_drawn(&h, "Billing"), "it is in the list");
    assert_eq!(
        h.app.workspace_pane.selected_path(),
        Some("Services/Billing")
    );
    forget_settings(&file);
}

// ---- what is done with an item ---------------------------------------------------------------

#[test]
fn an_item_is_renamed_in_the_list_and_keeps_its_id() {
    let (mut h, file) = start("ws-rename", OPEN);
    let dir = folder_of(&file);
    let id = index_of(&dir)["items"]["scratch.jq"]["id"].clone();
    let at = row_at(&h, "scratch");
    h.right_click(at.x, at.y);
    press(&mut h, "Rename");
    type_over(&mut h, "fresh");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert!(dir.join("fresh.jq").is_file() && !dir.join("scratch.jq").exists());
    assert!(is_drawn(&h, "fresh") && !is_drawn(&h, "scratch"));
    assert_eq!(index_of(&dir)["items"]["fresh.jq"]["id"], id);
    forget_settings(&file);
}

#[test]
fn a_folder_that_is_renamed_stays_open_and_so_do_the_folders_in_it_that_were() {
    let (mut h, file) = start("ws-rename-open", OPEN);
    let dir = folder_of(&file);
    pick(&mut h, "Errors");
    assert!(is_drawn(&h, "slow requests"), "Errors is open");
    let at = row_at(&h, "Logs");
    h.right_click(at.x, at.y);
    press(&mut h, "Rename");
    type_over(&mut h, "Journal");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert!(dir.join("Journal/Errors/slow requests.jq").is_file());
    for name in ["Journal", "Errors", "logins", "slow requests"] {
        assert!(is_drawn(&h, name), "{name}: {:?}", h.texts());
    }
    assert!(!is_drawn(&h, "Logs"));
    forget_settings(&file);
}

#[test]
fn a_folder_that_was_shut_is_shut_when_it_is_renamed() {
    let (mut h, file) = start("ws-rename-shut", OPEN);
    assert!(!is_drawn(&h, "slow requests"), "Errors is shut");
    let at = row_at(&h, "Errors");
    h.right_click(at.x, at.y);
    press(&mut h, "Rename");
    type_over(&mut h, "Failed");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert!(folder_of(&file)
        .join("Logs/Failed/slow requests.jq")
        .is_file());
    assert!(
        is_drawn(&h, "Failed") && !is_drawn(&h, "slow requests"),
        "{:?}",
        h.texts()
    );
    forget_settings(&file);
}

#[test]
fn a_name_that_is_taken_is_said_and_the_item_is_not_renamed() {
    let (mut h, file) = start_with_more("ws-rename-taken", OPEN, &[("other.jq", ".other")]);
    let dir = folder_of(&file);
    let at = row_at(&h, "scratch");
    h.right_click(at.x, at.y);
    press(&mut h, "Rename");
    type_over(&mut h, "other");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert!(h
        .app
        .workspace_pane
        .notice_text()
        .is_some_and(|t| t.contains("already")));
    assert_eq!(read(&dir, "scratch.jq"), ".");
    assert_eq!(read(&dir, "other.jq"), ".other");
    forget_settings(&file);
}

#[test]
fn an_item_is_copied_with_its_sample_and_an_id_of_its_own() {
    let (mut h, file) = start("ws-duplicate", OPEN);
    let dir = folder_of(&file);
    pick(&mut h, "Errors");
    let at = row_at(&h, "slow requests");
    h.right_click(at.x, at.y);
    press(&mut h, "Duplicate");
    assert_eq!(
        read(&dir, "Logs/Errors/slow requests (2).jq"),
        "map(select(.duration > 500))"
    );
    assert!(dir
        .join("Logs/Errors/slow requests (2).sample.json")
        .is_file());
    assert!(is_drawn(&h, "slow requests (2)"));
    let index = index_of(&dir);
    assert_ne!(
        index["items"]["Logs/Errors/slow requests (2).jq"]["id"],
        index["items"]["Logs/Errors/slow requests.jq"]["id"]
    );
    forget_settings(&file);
}

#[test]
fn an_item_is_deleted_after_asking_and_not_before() {
    let (mut h, file) = start("ws-delete", OPEN);
    let dir = folder_of(&file);
    let at = row_at(&h, "scratch");
    h.right_click(at.x, at.y);
    press(&mut h, "Delete…");
    assert!(
        is_drawn_containing(&h, "Delete “scratch”?"),
        "{:?}",
        h.texts()
    );
    assert!(dir.join("scratch.jq").exists(), "not yet");
    press_last(&mut h, "Cancel");
    assert!(dir.join("scratch.jq").exists(), "not when it is not wanted");
    h.right_click(at.x, at.y);
    press(&mut h, "Delete…");
    press_last(&mut h, "Delete");
    assert!(!dir.join("scratch.jq").exists());
    assert!(!is_drawn(&h, "scratch"));
    assert!(!index_of(&dir)["items"]
        .as_object()
        .unwrap()
        .contains_key("scratch.jq"));
    assert_eq!(
        h.app.workspace_pane.selected_path(),
        None,
        "what was deleted is not picked any more"
    );
    assert_eq!(h.app.workspace_pane.notice_text(), Some("Deleted 1 item"));
    forget_settings(&file);
}

#[test]
fn deleting_a_folder_with_a_file_of_another_program_in_it_says_that_file_and_the_folder_are_left() {
    let (mut h, file) = start_with_more(
        "ws-delete-folder-left",
        OPEN,
        &[("Services/notes.txt", "mine")],
    );
    let dir = folder_of(&file);
    let at = row_at(&h, "Services");
    h.right_click(at.x, at.y);
    press(&mut h, "Delete…");
    press_last(&mut h, "Delete");
    assert!(
        dir.join("Services/notes.txt").is_file(),
        "the other file is left"
    );
    assert!(!dir.join("Services/orders.schema.json").exists());
    assert_eq!(
        h.app.workspace_pane.notice_text(),
        Some("Deleted 2 items; 1 other file in the folder was left, and so is the folder")
    );
    // The folder is still there (it holds the other file) but it is not the one that is picked
    // any more: it was deleted, as far as the workspace goes.
    assert_eq!(h.app.workspace_pane.selected_path(), None);
    forget_settings(&file);
}

#[test]
fn a_message_under_the_list_asks_for_the_window_to_be_drawn_again_when_it_is_due_to_go() {
    // The window is not drawn again unless something asks for it: a message that stays
    // until the next touch of the mouse is a message that never goes. What is asked for
    // in one frame is forgotten by the next, so the message asks in every frame it is shown.
    let (mut h, file) = start("ws-notice-repaint", OPEN);
    let at = row_at(&h, "scratch");
    h.right_click(at.x, at.y);
    press(&mut h, "Duplicate");
    assert!(is_drawn_containing(&h, "Copied as"), "{:?}", h.texts());
    let asked = |h: &Harness| h.repaint_delay.as_secs_f64();
    let first = asked(&h);
    assert!(
        (6.0..=8.2).contains(&first),
        "the first frames ask to be drawn when the message is due to go: {first}"
    );
    // A frame later, and three seconds later, it still asks, for what is left of the time.
    h.frame();
    assert!(asked(&h) < first + 0.1 && asked(&h) > 5.0, "{}", asked(&h));
    h.pause(3.0);
    let later = asked(&h);
    assert!(
        later < first - 2.0 && later > 2.0,
        "three seconds on it asks for the rest: {later} after {first}"
    );
    // Drawn when asked, the message is gone, and nothing is asked for any more.
    h.pause(6.0);
    assert!(!is_drawn_containing(&h, "Copied as"), "{:?}", h.texts());
    assert_eq!(
        h.repaint_delay,
        std::time::Duration::MAX,
        "nothing is left to wait for"
    );
    forget_settings(&file);
}

#[test]
fn the_text_of_an_item_is_copied_to_the_clipboard() {
    let (mut h, file) = start("ws-copy", OPEN);
    let at = row_at(&h, "scratch");
    h.right_click(at.x, at.y);
    press(&mut h, "Copy text");
    assert_eq!(h.copied.last().map(String::as_str), Some("."));
    forget_settings(&file);
}

// ---- export and import ----------------------------------------------------------------------

#[test]
fn the_workspace_is_exported_to_a_folder_through_the_menu() {
    let (mut h, file) = start("ws-export", OPEN);
    let to = file.parent().unwrap().parent().unwrap().join("exported");
    click_button(&mut h, "menu");
    press(&mut h, "Export the workspace…");
    assert!(is_drawn(&h, "Export the workspace"));
    assert!(
        is_drawn_containing(&h, "Everything: 5 items in 3 folders"),
        "{:?}",
        h.texts()
    );
    assert!(is_drawn_containing(&h, "include the samples (2 files"));
    // The place to go to is typed over what is there.
    press_one_of(
        &mut h,
        &["a new folder, or an empty one", "jsonquery-workspace-*"],
    );
    type_over(&mut h, &to.display().to_string());
    press_last(&mut h, "Export");
    assert_eq!(read(&to, "scratch.jq"), ".");
    assert_eq!(
        read(&to, "Logs/Errors/slow requests.sample.json"),
        FILES[1].1
    );
    assert!(to.join(".jsonquery/index.json").is_file());
    assert!(h
        .app
        .workspace_pane
        .notice_text()
        .is_some_and(|t| t.starts_with("Exported 5 items")));
    let _ = std::fs::remove_dir_all(&to);
    forget_settings(&file);
}

#[test]
fn a_folder_is_imported_into_a_folder_of_its_own_through_the_menu() {
    let (mut h, file) = start("ws-import", OPEN);
    let dir = folder_of(&file);
    let from = file
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("team queries");
    put(&from, "Reports/weekly.jq", ".weekly");
    put(&from, "Reports/weekly.sample.json", "[1]");
    put(&from, "check.schema.json", "{}");
    click_button(&mut h, "menu");
    press(&mut h, "Import a workspace…");
    assert!(is_drawn(&h, "Import a workspace"));
    press(&mut h, "a folder of queries, schemas…");
    h.type_text(&from.display().to_string());
    h.key(egui::Key::Tab, egui::Modifiers::NONE);
    h.settle();
    assert!(
        is_drawn_containing(&h, "2 items in 1 folder: 1 query, 1 schema"),
        "{:?}",
        h.texts()
    );
    press_last(&mut h, "Import");
    assert_eq!(read(&dir, "team queries/Reports/weekly.jq"), ".weekly");
    assert_eq!(read(&dir, "team queries/Reports/weekly.sample.json"), "[1]");
    assert!(is_drawn(&h, "team queries"));
    assert!(h
        .app
        .workspace_pane
        .notice_text()
        .is_some_and(|t| t.starts_with("Imported 2 items")));
    let _ = std::fs::remove_dir_all(&from);
    forget_settings(&file);
}

#[test]
fn a_folder_that_was_only_typed_is_looked_at_when_import_is_pressed_and_imported_at_once() {
    let (mut h, file) = start("ws-import-typed", OPEN);
    let dir = folder_of(&file);
    let from = file
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("typed queries");
    put(&from, "weekly.jq", ".weekly");
    click_button(&mut h, "menu");
    press(&mut h, "Import a workspace…");
    press(&mut h, "a folder of queries, schemas…");
    h.type_text(&from.display().to_string());
    h.settle();
    press_last(&mut h, "Import");
    assert_eq!(read(&dir, "typed queries/weekly.jq"), ".weekly");
    let said = h.app.workspace_pane.notice_text().unwrap_or_default();
    assert!(said.starts_with("Imported 1 item"), "{said:?}");
    assert!(!said.contains("1 items"), "{said:?}");
    let _ = std::fs::remove_dir_all(&from);
    forget_settings(&file);
}

#[test]
fn a_folder_that_cannot_be_imported_is_said_and_typing_another_lets_one_press_import_it() {
    let (mut h, file) = start("ws-import-bad", OPEN);
    let dir = folder_of(&file);
    let root = file.parent().unwrap().parent().unwrap();
    let missing = root.join("not there at all");
    let from = root.join("good queries");
    put(&from, "weekly.jq", ".weekly");
    click_button(&mut h, "menu");
    press(&mut h, "Import a workspace…");
    press(&mut h, "a folder of queries, schemas…");
    h.type_text(&missing.display().to_string());
    h.key(egui::Key::Tab, egui::Modifiers::NONE);
    h.settle();
    assert!(
        is_drawn_containing(&h, "not there at all is not a folder"),
        "{:?}",
        h.texts()
    );
    // The button is not for a folder that cannot be used: pressing it does nothing.
    press_last(&mut h, "Import");
    assert!(!dir.join("not there at all").exists());
    assert!(is_drawn(&h, "Import a workspace"));
    // Typing another folder takes the old words away, and one press does the rest.
    press_start(&mut h, &missing.display().to_string());
    type_over(&mut h, &from.display().to_string());
    h.settle();
    assert!(
        !is_drawn_containing(&h, "is not a folder"),
        "{:?}",
        h.texts()
    );
    press_last(&mut h, "Import");
    assert_eq!(read(&dir, "good queries/weekly.jq"), ".weekly");
    let _ = std::fs::remove_dir_all(&from);
    forget_settings(&file);
}

#[test]
fn the_new_folder_of_an_import_is_named_for_the_folder_that_comes_in_unless_it_was_named() {
    let (mut h, file) = start("ws-import-named", OPEN);
    let dir = folder_of(&file);
    let root = file.parent().unwrap().parent().unwrap();
    let first = root.join("first queries");
    let second = root.join("second queries");
    put(&first, "a.jq", ".a");
    put(&second, "b.jq", ".b");
    click_button(&mut h, "menu");
    press(&mut h, "Import a workspace…");
    press(&mut h, "a folder of queries, schemas…");
    h.type_text(&first.display().to_string());
    h.key(egui::Key::Tab, egui::Modifiers::NONE);
    h.settle();
    assert!(is_drawn(&h, "first queries"), "{:?}", h.texts());
    // A name that is typed stays when another folder comes in.
    press(&mut h, "first queries");
    type_over(&mut h, "mine");
    press_start(&mut h, &first.display().to_string());
    type_over(&mut h, &second.display().to_string());
    h.settle();
    press_last(&mut h, "Import");
    assert_eq!(read(&dir, "mine/b.jq"), ".b");
    assert!(!dir.join("second queries").exists() && !dir.join("first queries").exists());
    let _ = std::fs::remove_dir_all(&first);
    let _ = std::fs::remove_dir_all(&second);
    forget_settings(&file);
}

#[test]
fn what_is_said_of_a_count_is_in_the_singular_for_one() {
    use crate::workspace_pane::counted;
    assert_eq!(counted(0, "item", "items"), "0 items");
    assert_eq!(counted(1, "item", "items"), "1 item");
    assert_eq!(counted(2, "folder", "folders"), "2 folders");
    assert_eq!(counted(1, "other file", "other files"), "1 other file");
}

// ---- where the workspace is -------------------------------------------------------------------

#[test]
fn another_folder_chosen_for_the_workspace_is_kept_in_the_settings_and_the_usual_one_can_come_back()
{
    let (mut h, file) = start("ws-choose", OPEN);
    let other = file
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("chosen workspace");
    put(&other, "mine.jq", ".mine");
    assert!(is_drawn(&h, "scratch"));
    h.app
        .workspace_pane
        .push_action(Action::SetFolder(Some(other.clone())));
    h.settle();
    assert_eq!(
        h.app.settings.workspace_folder.as_deref(),
        Some(other.as_path())
    );
    assert!(is_drawn(&h, "mine") && !is_drawn(&h, "scratch"));
    assert_eq!(
        kept(&file).unwrap()["workspace"]["folder"],
        other.display().to_string()
    );
    assert!(
        is_drawn_containing(&h, "chosen workspace"),
        "the pane says where it is"
    );

    h.app.workspace_pane.push_action(Action::SetFolder(None));
    h.settle();
    assert!(is_drawn(&h, "scratch") && !is_drawn(&h, "mine"));
    assert!(kept(&file).unwrap()["workspace"].get("folder").is_none());
    let _ = std::fs::remove_dir_all(&other);
    forget_settings(&file);
}

#[test]
fn a_folder_that_was_chosen_and_is_gone_is_said_and_is_not_made_again() {
    let missing = std::env::temp_dir().join(format!("jsonquery-missing-{}", std::process::id()));
    let settings = format!(
        r#"{{"workspace": {{"pane": "open", "folder": {:?}}}}}"#,
        missing.display().to_string()
    );
    let (h, file) = start("ws-missing", &settings);
    assert!(
        is_drawn_containing(&h, "folder is not there"),
        "{:?}",
        h.texts()
    );
    assert!(is_drawn(&h, "Choose another…") && is_drawn(&h, "Use the usual one"));
    assert!(!missing.exists(), "it was not made");
    forget_settings(&file);
}

#[test]
fn when_the_window_gets_the_focus_back_the_folder_is_read_again() {
    let (mut h, file) = start("ws-focus", OPEN);
    let dir = folder_of(&file);
    assert!(!is_drawn(&h, "added elsewhere"));
    h.window.focused = Some(false);
    h.frame();
    put(&dir, "added elsewhere.jq", ".");
    h.window.focused = Some(true);
    h.settle();
    assert!(is_drawn(&h, "added elsewhere"), "{:?}", h.texts());
    forget_settings(&file);
}

#[test]
fn what_another_program_does_to_the_files_leaves_the_ids_of_the_items_as_they_were() {
    let (mut h, file) = start("ws-outside", OPEN);
    let dir = folder_of(&file);
    let index = index_of(&dir);
    let id = index["items"]["scratch.jq"]["id"].clone();
    h.window.focused = Some(false);
    h.frame();
    // Renamed, and one edited, and one taken away, in a file manager.
    std::fs::rename(dir.join("scratch.jq"), dir.join("renamed outside.jq")).unwrap();
    put(&dir, "Logs/logins.jmespath", "[?ok] | length(@)");
    std::fs::remove_file(dir.join("Services/bump.patch.json")).unwrap();
    h.window.focused = Some(true);
    h.settle();
    assert!(is_drawn(&h, "renamed outside") && !is_drawn(&h, "scratch") && !is_drawn(&h, "bump"));
    let now = index_of(&dir);
    assert_eq!(
        now["items"]["renamed outside.jq"]["id"], id,
        "found again by what it holds"
    );
    assert_eq!(
        now["items"]["Logs/logins.jmespath"]["id"], index["items"]["Logs/logins.jmespath"]["id"],
        "edited: the same item"
    );
    assert_eq!(
        now["items"]["Logs/logins.jmespath"]["sha256"],
        sha256_hex(b"[?ok] | length(@)")
    );
    assert!(now["items"].get("Services/bump.patch.json").is_none());
    forget_settings(&file);
}

// ---- from the pages of the Tools ---------------------------------------------------------------

#[test]
fn save_schema_on_the_validate_page_shows_the_pane_and_opens_the_window_that_saves_a_schema() {
    let (mut h, file) = start("ws-wish-save", "{}");
    let dir = folder_of(&file);
    open_tool(&mut h, Tool::Validate);
    // Nothing to keep in an empty box: the button does nothing.
    press(&mut h, "Save schema…");
    assert!(!is_drawn(&h, "Save in the workspace"));
    assert_eq!(h.app.settings.interface.workspace, WorkspaceView::Hidden);

    h.app
        .tools
        .fill(Tool::Validate, "Schema", r#"{"type": "number"}"#);
    // (A query in the box of the main window does not change what this button keeps.)
    h.app.query_text = ".a".to_owned();
    h.settle();
    press(&mut h, "Save schema…");
    assert_eq!(
        h.app.settings.interface.workspace,
        WorkspaceView::Open,
        "the pane is shown"
    );
    assert!(is_drawn(&h, "Save in the workspace"), "{:?}", h.texts());
    assert!(
        h.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Focus)),
        "the main window is brought forward"
    );
    h.type_text("numbers");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert_eq!(read(&dir, "numbers.schema.json"), r#"{"type": "number"}"#);
    assert!(is_drawn(&h, "numbers"), "it is in the list");
    forget_settings(&file);
}

#[test]
fn saved_schemas_shows_the_pane_with_the_schemas_of_the_workspace_and_nothing_else() {
    let (mut h, file) = start("ws-wish-pick", "{}");
    // (Docked: a window of the Tools drawn in the frame would lie over the pane.)
    dock_tools(&mut h, Tool::Validate);
    assert!(!is_drawn(&h, "Workspace"));
    press(&mut h, "Saved schemas…");
    assert_eq!(h.app.settings.interface.workspace, WorkspaceView::Open);
    assert_eq!(h.app.workspace_pane.group(), Group::Schemas);
    assert!(is_drawn(&h, "orders") && !is_drawn(&h, "scratch") && !is_drawn(&h, "bump"));
    // Picked there, it goes in the box of the page.
    pick(&mut h, "orders");
    press(&mut h, "Use as schema");
    assert_eq!(
        h.app.tools.box_text(Tool::Validate, "Schema"),
        r#"{"type": "object", "required": ["id"]}"#
    );
    forget_settings(&file);
}

#[test]
fn the_patch_page_has_the_same_two_buttons_for_patches() {
    let (mut h, file) = start("ws-wish-patch", "{}");
    let dir = folder_of(&file);
    open_tool(&mut h, Tool::Patch);
    h.app
        .tools
        .fill(Tool::Patch, "Patch", r#"[{"op": "remove", "path": "/a"}]"#);
    h.app.query_text = ".a".to_owned();
    h.settle();
    press(&mut h, "Save patch…");
    assert!(is_drawn(&h, "Save in the workspace"));
    h.type_text("drop a");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert_eq!(
        read(&dir, "drop a.patch.json"),
        r#"[{"op": "remove", "path": "/a"}]"#
    );
    press(&mut h, "Saved patches…");
    assert_eq!(h.app.workspace_pane.group(), Group::Patches);
    assert!(is_drawn(&h, "bump") && is_drawn(&h, "drop a") && !is_drawn(&h, "orders"));
    forget_settings(&file);
}

#[test]
fn the_filter_looks_at_the_name_of_the_folder_as_well_as_the_name_of_the_item() {
    let (mut h, file) = start("ws-filter-folder", OPEN);
    press(&mut h, "Filter by name…");
    h.type_text("errors");
    h.settle();
    assert!(
        is_drawn(&h, "slow requests"),
        "in a folder called Errors: {:?}",
        h.texts()
    );
    assert!(!is_drawn(&h, "logins") && !is_drawn(&h, "scratch") && !is_drawn(&h, "orders"));
    forget_settings(&file);
}

#[test]
fn save_in_the_query_pane_that_is_in_a_window_of_its_own_brings_the_main_window_forward() {
    // The window that saves is the main window's: the button in the other window has to bring
    // the main window in front of it, or nothing seems to happen (the window that saves is
    // behind the one that was clicked).
    let file = settings_file("ws-save-from-window");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, OPEN).unwrap();
    let mut w = Windows::with_store(Store::at(Some(file.clone())));
    w.pop_out(Pane::Query);
    let save = w.window_text(Pane::Query, "Save…");
    w.commands.clear();
    w.window_click(Pane::Query, save);
    for _ in 0..3 {
        w.main_frame(Vec::new());
    }
    assert!(
        w.commands
            .contains(&(egui::ViewportId::ROOT, egui::ViewportCommand::Focus)),
        "the main window is asked to come forward: {:?}",
        w.commands
    );
    assert!(
        w.text_in(egui::ViewportId::ROOT, "Save in the workspace")
            .is_some(),
        "and the window that saves is drawn in it"
    );
    forget_settings(&file);
}

#[test]
fn ctrl_z_in_the_query_box_brings_back_the_text_that_an_open_replaced() {
    let (mut h, file) = start("ws-undo-open", OPEN);
    // A query typed in the box and left for more than a second: the text box keeps it as a
    // step to go back to (a text that is changed again within the second is not one).
    let q = h.query_box().center();
    h.click(q.x, q.y);
    h.type_text(".keep");
    h.pause(1.5);
    assert_eq!(h.app.query_text, ".keep");
    let at = row_at(&h, "scratch");
    h.double_click(at.x, at.y);
    assert_eq!(h.app.query_text, ".", "the item replaced it");
    // The box clicked, and Ctrl+Z.
    let q = h.query_box().center();
    h.click(q.x, q.y);
    h.key(egui::Key::Z, egui::Modifiers::COMMAND);
    h.settle();
    assert_eq!(h.app.query_text, ".keep", "the text that was there is back");
    forget_settings(&file);
}
