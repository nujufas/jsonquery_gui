//! Moving what is in the workspace's list by dragging it: a row is picked up, carried
//! over the list and put down on a folder, on an item, or on the empty part of the
//! list; what is lit and said while it is carried; what is not done (the folder it is
//! in already, a folder into itself, outside the list, Escape); and that what is moved
//! is the same item afterwards (its id, its sample, its place in the query box).

use super::workspace_tests::{
    folder_of, index_of, pick, read, row_at, start, start_with_more, FILES, OPEN,
};
use super::*;

/// Press at `from`, move to `to` in small steps, and keep the button down.
pub(super) fn pick_up(h: &mut Harness, from: egui::Pos2, to: egui::Pos2) {
    h.pointer(from.x, from.y);
    h.button(from.x, from.y, true);
    move_to(h, from, to);
}

/// Move the pointer, button down, from `from` to `to` in small steps.
pub(super) fn move_to(h: &mut Harness, from: egui::Pos2, to: egui::Pos2) {
    let steps = 12;
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        h.pointer(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
    }
}

/// Let go at `at`.
pub(super) fn let_go(h: &mut Harness, at: egui::Pos2) {
    h.button(at.x, at.y, false);
    h.settle();
}

/// Carry the row at `from` to `to` and put it down there.
pub(super) fn drag(h: &mut Harness, from: egui::Pos2, to: egui::Pos2) {
    pick_up(h, from, to);
    let_go(h, to);
}

/// The row called `name` that is lowest in the list (there may be two of a name).
fn lowest(h: &Harness, name: &str) -> egui::Pos2 {
    text_rects(&h.shapes)
        .into_iter()
        .filter(|(t, _)| t == name)
        .map(|(_, rect)| rect.center())
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap_or_else(|| panic!("the row {name:?} is drawn: {:?}", h.texts()))
}

/// Carry the row called `name` along its own row, and let go on it: far enough to be a
/// drag and not a click.
fn drag_along(h: &mut Harness, name: &str) {
    let at = row_at(h, name);
    drag(h, at, at + egui::vec2(80.0, 0.0));
}

/// Carry the row called `from` onto the row called `onto`.
fn drag_row(h: &mut Harness, from: &str, onto: &str) {
    let (a, b) = (row_at(h, from), row_at(h, onto));
    drag(h, a, b);
}

/// A place in the list below its last row, where there is nothing.
fn empty_part(h: &Harness) -> egui::Pos2 {
    let last = row_at(h, "scratch");
    let at = egui::pos2(last.x, last.y + 120.0);
    let pane = h.panel("workspace_pane").expect("the pane is drawn");
    let details = h.panel("workspace_details").expect("the details are drawn");
    assert!(
        pane.contains(at) && at.y < details.top(),
        "an empty place in the list: {at:?} {pane:?} {details:?}"
    );
    at
}

pub(super) fn notice(h: &Harness) -> Option<String> {
    h.app.workspace_pane.notice_text().map(str::to_owned)
}

/// The rectangles of the shapes that are rectangles with an outline of `width` in the
/// colour of a selection, and whether they are filled.
pub(super) fn outlines(h: &Harness, width: f32) -> Vec<(egui::Rect, bool)> {
    fn walk(
        shape: &egui::Shape,
        color: egui::Color32,
        width: f32,
        out: &mut Vec<(egui::Rect, bool)>,
    ) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, color, width, out)),
            egui::Shape::Rect(r)
                if r.stroke.color == color && (r.stroke.width - width).abs() < 0.01 =>
            {
                out.push((r.rect, r.fill.a() > 0));
            }
            _ => {}
        }
    }
    let color = h.ctx.global_style().visuals.selection.stroke.color;
    let mut out = Vec::new();
    for clipped in &h.shapes {
        walk(&clipped.shape, color, width, &mut out);
    }
    out
}

/// What is lit as the place that what is carried would go into: rows, and whether the
/// whole list is.
pub(super) fn lit(h: &Harness) -> Vec<egui::Rect> {
    outlines(h, 1.5).into_iter().map(|(rect, _)| rect).collect()
}

/// The colour a text near `at` is drawn in.
fn color_near(h: &Harness, text: &str, at: egui::Pos2) -> egui::Color32 {
    fn walk(shape: &egui::Shape, text: &str, out: &mut Vec<(egui::Pos2, egui::Color32)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, out)),
            egui::Shape::Text(t) if t.galley.text() == text => {
                let color = t.galley.job.sections[0].format.color;
                out.push((t.galley.rect.translate(t.pos.to_vec2()).center(), color));
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &h.shapes {
        walk(&clipped.shape, text, &mut found);
    }
    found
        .into_iter()
        .min_by(|a, b| a.0.distance(at).total_cmp(&b.0.distance(at)))
        .unwrap_or_else(|| panic!("{text:?} is drawn: {:?}", h.texts()))
        .1
}

fn exists(dir: &std::path::Path, rel: &str) -> bool {
    dir.join(rel).exists()
}

// ---- put down on a folder -----------------------------------------------------------

#[test]
fn a_row_dragged_onto_a_folder_goes_into_it_and_is_the_same_item_afterwards() {
    let (mut h, file) = start("ws-drag-folder", OPEN);
    let dir = folder_of(&file);
    let before = index_of(&dir)["items"]["scratch.jq"].clone();
    drag_row(&mut h, "scratch", "Logs");
    assert_eq!(read(&dir, "Logs/scratch.jq"), ".");
    assert!(!exists(&dir, "scratch.jq"), "and it is not where it was");
    // Kept as the same item: its id, and what it holds.
    let index = index_of(&dir);
    assert!(index["items"].get("scratch.jq").is_none());
    let after = &index["items"]["Logs/scratch.jq"];
    assert_eq!(after["id"], before["id"]);
    assert_eq!(after["sha256"], before["sha256"]);
    // It is shown in its new place, picked, and the move is said.
    assert!(is_drawn(&h, "scratch"));
    assert!(row_at(&h, "scratch").x > row_at(&h, "Logs").x, "in Logs");
    assert_eq!(
        h.app.workspace_pane.selected_path(),
        Some("Logs/scratch.jq")
    );
    assert_eq!(notice(&h).as_deref(), Some("Moved “scratch” into “Logs”"));
    forget_settings(&file);
}

#[test]
fn an_item_takes_its_sample_along_and_the_sample_keeps_its_sha() {
    let (mut h, file) = start("ws-drag-sample", OPEN);
    let dir = folder_of(&file);
    pick(&mut h, "Errors");
    let sample = index_of(&dir)["items"]["Logs/Errors/slow requests.jq"]["sample"].clone();
    assert!(sample["sha256"].is_string(), "{sample}");
    drag_row(&mut h, "slow requests", "Services");
    assert_eq!(
        read(&dir, "Services/slow requests.jq"),
        "map(select(.duration > 500))"
    );
    assert_eq!(
        read(&dir, "Services/slow requests.sample.json"),
        r#"[{"duration": 900}, {"duration": 10}]"#
    );
    assert!(!exists(&dir, "Logs/Errors/slow requests.jq"));
    assert!(!exists(&dir, "Logs/Errors/slow requests.sample.json"));
    let moved = index_of(&dir)["items"]["Services/slow requests.jq"].clone();
    assert_eq!(moved["sample"]["sha256"], sample["sha256"]);
    assert!(is_drawn(&h, "📎"), "it still has its sample");
    forget_settings(&file);
}

#[test]
fn a_folder_dragged_onto_a_folder_takes_everything_in_it_with_the_ids_unchanged() {
    let (mut h, file) = start("ws-drag-folder-folder", OPEN);
    let dir = folder_of(&file);
    let before = index_of(&dir);
    drag_row(&mut h, "Errors", "Services");
    assert!(dir.join("Services/Errors/slow requests.jq").is_file());
    assert!(dir
        .join("Services/Errors/slow requests.sample.json")
        .is_file());
    assert!(!exists(&dir, "Logs/Errors"), "and it is not where it was");
    assert!(exists(&dir, "Logs/logins.jmespath"), "what stayed, stayed");
    let after = index_of(&dir);
    assert_eq!(
        after["items"]["Services/Errors/slow requests.jq"]["id"],
        before["items"]["Logs/Errors/slow requests.jq"]["id"]
    );
    assert_eq!(
        h.app.workspace_pane.selected_path(),
        Some("Services/Errors")
    );
    assert_eq!(
        notice(&h).as_deref(),
        Some("Moved “Errors” into “Services”")
    );
    forget_settings(&file);
}

#[test]
fn a_row_put_down_on_a_shut_folder_goes_in_and_the_folder_opens_to_show_it() {
    let (mut h, file) = start("ws-drag-shut", OPEN);
    let dir = folder_of(&file);
    assert!(!is_drawn(&h, "slow requests"), "Errors is shut");
    drag_row(&mut h, "scratch", "Errors");
    assert!(dir.join("Logs/Errors/scratch.jq").is_file());
    assert!(
        is_drawn(&h, "slow requests") && is_drawn(&h, "scratch"),
        "{:?}",
        h.texts()
    );
    forget_settings(&file);
}

// ---- put down on an item, or on the empty part of the list --------------------------------

#[test]
fn a_folder_that_is_open_is_open_where_it_is_put_down_and_so_are_the_folders_in_it_that_were() {
    let (mut h, file) = start("ws-drag-open", OPEN);
    pick(&mut h, "Errors");
    assert!(is_drawn(&h, "slow requests"), "Errors is open");
    drag_row(&mut h, "Logs", "Services");
    assert!(folder_of(&file)
        .join("Services/Logs/Errors/slow requests.jq")
        .is_file());
    for name in ["Services", "Logs", "Errors", "logins", "slow requests"] {
        assert!(is_drawn(&h, name), "{name}: {:?}", h.texts());
    }
    forget_settings(&file);
}

#[test]
fn a_folder_that_is_shut_is_shut_where_it_is_put_down() {
    let (mut h, file) = start("ws-drag-shut-folder", OPEN);
    drag_row(&mut h, "Errors", "Services");
    assert!(folder_of(&file)
        .join("Services/Errors/slow requests.jq")
        .is_file());
    assert!(
        is_drawn(&h, "Errors") && !is_drawn(&h, "slow requests"),
        "{:?}",
        h.texts()
    );
    forget_settings(&file);
}

#[test]
fn a_row_put_down_on_an_item_goes_into_the_folder_that_item_is_in() {
    let (mut h, file) = start("ws-drag-item", OPEN);
    let dir = folder_of(&file);
    drag_row(&mut h, "scratch", "bump");
    assert_eq!(read(&dir, "Services/scratch.jq"), ".");
    assert!(!exists(&dir, "scratch.jq"));
    assert_eq!(
        notice(&h).as_deref(),
        Some("Moved “scratch” into “Services”")
    );
    forget_settings(&file);
}

#[test]
fn a_row_put_down_on_an_item_at_the_top_goes_to_the_top() {
    let (mut h, file) = start_with_more("ws-drag-item-top", OPEN, &[("other.jq", ".other")]);
    let dir = folder_of(&file);
    drag_row(&mut h, "logins", "scratch");
    assert!(dir.join("logins.jmespath").is_file());
    assert!(!exists(&dir, "Logs/logins.jmespath"));
    assert_eq!(notice(&h).as_deref(), Some("Moved “logins” to the top"));
    forget_settings(&file);
}

#[test]
fn a_row_put_down_on_the_empty_part_of_the_list_goes_to_the_top() {
    let (mut h, file) = start("ws-drag-empty", OPEN);
    let dir = folder_of(&file);
    let to = empty_part(&h);
    let from = row_at(&h, "bump");
    drag(&mut h, from, to);
    assert!(dir.join("bump.patch.json").is_file());
    assert!(!exists(&dir, "Services/bump.patch.json"));
    assert_eq!(notice(&h).as_deref(), Some("Moved “bump” to the top"));
    // A folder too.
    let to = empty_part(&h);
    let from = row_at(&h, "Errors");
    drag(&mut h, from, to);
    assert!(dir.join("Errors/slow requests.jq").is_file());
    assert!(!exists(&dir, "Logs/Errors"));
    forget_settings(&file);
}

// ---- what is not done ------------------------------------------------------------------

#[test]
fn a_row_put_down_on_the_folder_it_is_in_stays_and_nothing_is_said() {
    let (mut h, file) = start("ws-drag-same", OPEN);
    let dir = folder_of(&file);
    let index = index_of(&dir);
    drag_row(&mut h, "logins", "Logs");
    assert!(exists(&dir, "Logs/logins.jmespath"));
    drag_row(&mut h, "bump", "orders");
    assert!(
        exists(&dir, "Services/bump.patch.json"),
        "an item beside it is the same folder"
    );
    drag_along(&mut h, "scratch");
    assert!(exists(&dir, "scratch.jq"));
    let at = empty_part(&h);
    let from = row_at(&h, "scratch");
    drag(&mut h, from, at);
    assert!(
        exists(&dir, "scratch.jq"),
        "the empty part is the top, where it is"
    );
    assert_eq!(notice(&h), None, "nothing was done, so nothing is said");
    assert_eq!(
        index_of(&dir),
        index,
        "not even the index was written again"
    );
    forget_settings(&file);
}

#[test]
fn a_folder_is_not_put_into_itself_nor_into_a_folder_in_it() {
    let (mut h, file) = start("ws-drag-itself", OPEN);
    let dir = folder_of(&file);
    drag_row(&mut h, "Logs", "Errors");
    assert!(
        exists(&dir, "Logs/Errors/slow requests.jq"),
        "Logs is where it was"
    );
    assert!(!exists(&dir, "Logs/Errors/Logs"));
    drag_along(&mut h, "Logs");
    assert!(exists(&dir, "Logs/logins.jmespath"));
    assert!(
        is_drawn(&h, "logins"),
        "and it did not shut: it was dragged, not clicked"
    );
    assert_eq!(notice(&h), None);
    // Nor is it lit while carried over them.
    let (from, over) = (row_at(&h, "Logs"), row_at(&h, "Errors"));
    pick_up(&mut h, from, over);
    assert!(lit(&h).is_empty(), "{:?}", lit(&h));
    let_go(&mut h, over);
    forget_settings(&file);
}

#[test]
fn a_name_that_is_taken_in_the_folder_is_said_and_nothing_moves() {
    let (mut h, file) = start_with_more("ws-drag-taken", OPEN, &[("Logs/scratch.jq", ".other")]);
    let dir = folder_of(&file);
    let (from, to) = (lowest(&h, "scratch"), row_at(&h, "Logs"));
    drag(&mut h, from, to);
    assert_eq!(read(&dir, "scratch.jq"), ".");
    assert_eq!(read(&dir, "Logs/scratch.jq"), ".other");
    let said = notice(&h).expect("it is said");
    assert!(
        said.contains("There is already a query called “scratch”"),
        "{said}"
    );
    // A folder of the same name is refused too.
    let (mut h, file) = start_with_more(
        "ws-drag-taken-folder",
        OPEN,
        &[("Services/Errors/x.jq", ".")],
    );
    let dir = folder_of(&file);
    pick(&mut h, "Errors");
    drag_row(&mut h, "Errors", "Services");
    assert!(
        exists(&dir, "Logs/Errors/slow requests.jq"),
        "{:?}",
        h.texts()
    );
    assert!(notice(&h).is_some_and(|t| t.contains("There is already a folder called “Errors”")));
    forget_settings(&file);
}

#[test]
fn escape_puts_the_row_back_and_letting_go_afterwards_does_nothing() {
    let (mut h, file) = start("ws-drag-escape", OPEN);
    let dir = folder_of(&file);
    let (from, to) = (row_at(&h, "scratch"), row_at(&h, "Services"));
    pick_up(&mut h, from, to);
    assert!(
        is_drawn(&h, "Move “scratch” into “Services”"),
        "it was being carried"
    );
    h.key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(
        !is_drawn_containing(&h, "Move “scratch”"),
        "and now it is not"
    );
    let_go(&mut h, to);
    assert!(exists(&dir, "scratch.jq") && !exists(&dir, "Services/scratch.jq"));
    assert_eq!(notice(&h), None);
    forget_settings(&file);
}

#[test]
fn a_row_let_go_outside_the_list_stays_where_it_is() {
    let (mut h, file) = start("ws-drag-outside", OPEN);
    let dir = folder_of(&file);
    // An item that is in a folder (let go outside the list it is not taken out to the top, as
    // it would be on the empty part of the list), and one that is at the top.
    let inside = row_at(&h, "logins");
    let top = row_at(&h, "scratch");
    // On the header of the pane, and on the filter, and in the details.
    let filter = {
        let pane = h.panel("workspace_pane").expect("pane");
        egui::pos2(pane.center().x, pane.top() + 8.0)
    };
    let details = h.panel("workspace_details").expect("details").center();
    for to in [filter, details] {
        drag(&mut h, inside, to);
        assert!(
            exists(&dir, "Logs/logins.jmespath") && !exists(&dir, "logins.jmespath"),
            "let go at {to:?}"
        );
        drag(&mut h, top, to);
        assert!(exists(&dir, "scratch.jq"), "let go at {to:?}");
        assert_eq!(notice(&h), None);
        assert_eq!(h.app.query_text, "", "and the query box was not touched");
    }
    // What is not a query is not opened by being put down on the query box either: a
    // patch, a schema, a folder.
    let query = h.query_box().center();
    for name in ["bump", "orders", "Services"] {
        let from = row_at(&h, name);
        drag(&mut h, from, query);
        assert_eq!(notice(&h), None, "{name}");
        assert_eq!(
            h.app.query_text, "",
            "{name}: the query box was not touched"
        );
        assert!(exists(&dir, "Services/bump.patch.json"), "{name}");
        assert!(exists(&dir, "Services/orders.schema.json"), "{name}");
    }
    assert!(h.app.workspace_pane.take_actions().is_empty());
    forget_settings(&file);
}

#[test]
fn a_row_that_nobody_holds_any_more_is_not_carried_on() {
    let (mut h, file) = start("ws-drag-nobody", OPEN);
    let dir = folder_of(&file);
    let services = row_at(&h, "Services");
    // What is left of a drag when the button went up where the window was not told.
    crate::workspace_pane::carry_for_test(&h.ctx, "scratch.jq", "scratch", false);
    h.pointer(services.x, services.y);
    h.frame();
    assert!(
        !is_drawn_containing(&h, "Move “scratch”"),
        "nothing is carried: {:?}",
        h.texts()
    );
    assert!(lit(&h).is_empty(), "and nothing is lit");
    assert!(
        !egui::DragAndDrop::has_any_payload(&h.ctx),
        "what was left is thrown away"
    );
    assert!(exists(&dir, "scratch.jq") && !exists(&dir, "Services/scratch.jq"));
    forget_settings(&file);
}

#[test]
fn a_press_that_moves_a_little_is_a_click_and_carries_nothing() {
    let (mut h, file) = start("ws-drag-click", OPEN);
    let dir = folder_of(&file);
    let at = row_at(&h, "Logs");
    pick_up(&mut h, at, egui::pos2(at.x + 3.0, at.y + 2.0));
    assert!(!is_drawn_containing(&h, "Move “Logs”"), "not carried");
    let_go(&mut h, egui::pos2(at.x + 3.0, at.y + 2.0));
    assert_eq!(
        h.app.workspace_pane.selected_path(),
        Some("Logs"),
        "it was a click"
    );
    assert!(!is_drawn(&h, "logins"), "which shut the folder");
    assert!(exists(&dir, "Logs/logins.jmespath"));
    forget_settings(&file);
}

#[test]
fn a_double_click_still_opens_an_item_with_rows_that_can_be_dragged() {
    let (mut h, file) = start("ws-drag-double", OPEN);
    let at = row_at(&h, "scratch");
    h.double_click(at.x, at.y);
    assert_eq!(h.app.query_text, ".", "it was opened");
    assert_eq!(h.app.workspace_pane.selected_path(), Some("scratch.jq"));
    forget_settings(&file);
}

#[test]
fn only_the_main_button_carries_a_row() {
    let (mut h, file) = start("ws-drag-secondary", OPEN);
    let dir = folder_of(&file);
    let (from, to) = (row_at(&h, "scratch"), row_at(&h, "Services"));
    h.pointer(from.x, from.y);
    let press = |pressed: bool, at: egui::Pos2| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.frame_with(vec![press(true, from)]);
    move_to(&mut h, from, to);
    assert!(!is_drawn_containing(&h, "Move “scratch”"));
    h.frame_with(vec![press(false, to)]);
    h.settle();
    assert!(exists(&dir, "scratch.jq") && !exists(&dir, "Services/scratch.jq"));
    forget_settings(&file);
}

#[test]
fn the_row_that_is_being_renamed_is_not_carried() {
    let (mut h, file) = start("ws-drag-renaming", OPEN);
    let dir = folder_of(&file);
    let at = row_at(&h, "scratch");
    h.right_click(at.x, at.y);
    press(&mut h, "Rename");
    // Its right end (the tag that says what it is), and then to a folder.
    let pane = h.panel("workspace_pane").expect("pane");
    let end = egui::pos2(pane.right() - 12.0, at.y);
    let to = row_at(&h, "Logs");
    pick_up(&mut h, end, to);
    assert!(!is_drawn_containing(&h, "Move “scratch”"), "not carried");
    let_go(&mut h, to);
    assert!(exists(&dir, "scratch.jq"));
    forget_settings(&file);
}

// ---- what is shown while a row is carried -----------------------------------------------

#[test]
fn nothing_is_moved_until_the_row_is_put_down_and_nothing_is_said() {
    let (mut h, file) = start("ws-drag-not-yet", OPEN);
    let dir = folder_of(&file);
    let (from, to) = (row_at(&h, "scratch"), row_at(&h, "Services"));
    pick_up(&mut h, from, to);
    assert!(exists(&dir, "scratch.jq") && !exists(&dir, "Services/scratch.jq"));
    assert_eq!(notice(&h), None);
    let_go(&mut h, to);
    assert!(exists(&dir, "Services/scratch.jq"));
    forget_settings(&file);
}

#[test]
fn what_is_carried_says_where_it_would_go_beside_the_pointer() {
    let (mut h, file) = start("ws-drag-ghost", OPEN);
    let from = row_at(&h, "scratch");
    pick_up(&mut h, from, from + egui::vec2(0.0, -60.0));
    // Picked up, over a folder.
    let services = row_at(&h, "Services");
    move_to(&mut h, from + egui::vec2(0.0, -60.0), services);
    assert!(
        is_drawn(&h, "Move “scratch” into “Services”"),
        "{:?}",
        h.texts()
    );
    // Beside the pointer: a little to the right of it and under it, not far away.
    let (_, label) = h
        .texts()
        .into_iter()
        .find(|(text, _)| text == "Move “scratch” into “Services”")
        .expect("the label is drawn");
    let beside = label - services;
    assert!(
        (0.0..60.0).contains(&beside.x) && (0.0..60.0).contains(&beside.y),
        "the label is at {beside:?} from the pointer"
    );
    // Over a folder in a folder: the way to it is said.
    let errors = row_at(&h, "Errors");
    move_to(&mut h, services, errors);
    assert!(
        is_drawn(&h, "Move “scratch” into “Logs / Errors”"),
        "{:?}",
        h.texts()
    );
    // Over where it is, it only says what it is.
    move_to(&mut h, errors, from);
    assert!(!is_drawn_containing(&h, "Move “"), "{:?}", h.texts());
    // Over the filter, which is not a place for it: only the name.
    let pane = h.panel("workspace_pane").expect("pane");
    let filter = egui::pos2(pane.center().x, pane.top() + 8.0);
    move_to(&mut h, from, filter);
    assert!(!is_drawn_containing(&h, "Move “"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Open “"), "{:?}", h.texts());
    let names = h.texts().iter().filter(|(t, _)| t == "scratch").count();
    assert_eq!(names, 2, "the row, and what is carried");
    let_go(&mut h, filter);
    let names = h.texts().iter().filter(|(t, _)| t == "scratch").count();
    assert_eq!(names, 1, "it is not carried any more");
    forget_settings(&file);
}

#[test]
fn a_row_carried_to_the_top_says_so() {
    let (mut h, file) = start("ws-drag-ghost-top", OPEN);
    let (from, to) = (row_at(&h, "logins"), empty_part(&h));
    pick_up(&mut h, from, to);
    assert!(is_drawn(&h, "Move “logins” to the top"), "{:?}", h.texts());
    let_go(&mut h, to);
    forget_settings(&file);
}

#[test]
fn the_folder_that_would_take_the_row_is_lit_and_one_that_would_not_is_not() {
    let (mut h, file) = start("ws-drag-lit", OPEN);
    let from = row_at(&h, "scratch");
    assert!(lit(&h).is_empty(), "nothing is lit at rest");
    let services = row_at(&h, "Services");
    pick_up(&mut h, from, services);
    let rows = lit(&h);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].contains(services),
        "it is the row of Services: {:?} {services:?}",
        rows[0]
    );
    assert!(
        rows[0].height() > 15.0 && rows[0].height() < 25.0,
        "one row high: {:?}",
        rows[0]
    );
    // Over an item of another folder, the folder it is in is lit (not the item).
    let bump = row_at(&h, "bump");
    move_to(&mut h, services, bump);
    let rows = lit(&h);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].contains(services) && !rows[0].contains(bump),
        "{rows:?}"
    );
    // Over where it is already: nothing is lit.
    move_to(&mut h, bump, from);
    assert!(lit(&h).is_empty(), "{:?}", lit(&h));
    // Over the empty part of the list, the whole list is outlined — scratch is at the
    // top already, so carry something that is not.
    let_go(&mut h, from);
    let (logins, empty) = (row_at(&h, "logins"), empty_part(&h));
    pick_up(&mut h, logins, empty);
    let rows = lit(&h);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].height() > 200.0 && rows[0].contains(empty),
        "the whole list: {rows:?}"
    );
    let_go(&mut h, empty);
    assert!(lit(&h).is_empty(), "and nothing is lit afterwards");
    forget_settings(&file);
}

#[test]
fn the_row_that_is_carried_is_dimmed_where_it_was_and_only_while_it_is() {
    let (mut h, file) = start("ws-drag-dim", OPEN);
    let weak = h.ctx.global_style().visuals.weak_text_color();
    let from = row_at(&h, "scratch");
    assert_ne!(
        color_near(&h, "scratch", from),
        weak,
        "at rest it is not dim"
    );
    let (services, bump) = (row_at(&h, "Services"), row_at(&h, "bump"));
    pick_up(&mut h, from, services);
    assert_eq!(
        color_near(&h, "scratch", from),
        weak,
        "carried, its place is dim"
    );
    assert_ne!(color_near(&h, "bump", bump), weak, "and the others are not");
    let_go(&mut h, services);
    forget_settings(&file);
}

// ---- holding a row over a shut folder ------------------------------------------------------

#[test]
fn a_shut_folder_that_a_row_rests_on_opens_after_a_moment_and_not_before() {
    let (mut h, file) = start("ws-drag-dwell", OPEN);
    let (from, errors) = (row_at(&h, "scratch"), row_at(&h, "Errors"));
    pick_up(&mut h, from, errors);
    h.pause(0.4);
    assert!(!is_drawn(&h, "slow requests"), "not yet");
    h.pause(0.5);
    assert!(
        is_drawn(&h, "slow requests"),
        "after a moment: {:?}",
        h.texts()
    );
    // The row can be put down inside it, where the way there is open now.
    let inside = row_at(&h, "slow requests");
    move_to(&mut h, errors, inside);
    let_go(&mut h, inside);
    assert!(folder_of(&file).join("Logs/Errors/scratch.jq").is_file());
    forget_settings(&file);
}

#[test]
fn moving_on_to_another_folder_starts_the_wait_again() {
    let (mut h, file) = start("ws-drag-dwell-again", OPEN);
    let (from, errors, services) = (
        row_at(&h, "scratch"),
        row_at(&h, "Errors"),
        row_at(&h, "Services"),
    );
    pick_up(&mut h, from, errors);
    h.pause(0.5);
    move_to(&mut h, errors, services);
    h.pause(0.5);
    move_to(&mut h, services, errors);
    h.pause(0.5);
    assert!(
        !is_drawn(&h, "slow requests"),
        "0.5 s there, 1.5 s all told: still shut"
    );
    h.pause(0.4);
    assert!(is_drawn(&h, "slow requests"), "0.9 s there: open");
    let_go(&mut h, errors);
    forget_settings(&file);
}

#[test]
fn a_folder_that_is_open_stays_open_while_a_row_rests_on_it() {
    let (mut h, file) = start("ws-drag-dwell-open", OPEN);
    let (from, logs) = (row_at(&h, "scratch"), row_at(&h, "Logs"));
    pick_up(&mut h, from, logs);
    h.pause(1.5);
    assert!(is_drawn(&h, "logins"), "it was not shut by waiting on it");
    let_go(&mut h, logs);
    forget_settings(&file);
}

// ---- a long list ---------------------------------------------------------------------------

#[test]
fn a_row_carried_near_the_foot_of_a_long_list_brings_the_rest_of_it_into_view() {
    let more: Vec<(String, String)> = (0..60)
        .map(|n| (format!("zz{n:02}.jq"), ".".to_owned()))
        .collect();
    let more: Vec<(&str, &str)> = more.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (mut h, file) = start_with_more("ws-drag-scroll", OPEN, &more);
    assert!(!is_drawn(&h, "zz59"), "it is out of view");
    let foot = h.panel("workspace_details").expect("details").top();
    let from = row_at(&h, "scratch");
    let near_the_foot = egui::pos2(from.x, foot - 5.0);
    pick_up(&mut h, from, near_the_foot);
    h.pause(3.0);
    assert!(is_drawn(&h, "zz59"), "the end of the list came into view");
    assert!(!is_drawn(&h, "Logs"), "and the head went out of it");
    let_go(&mut h, near_the_foot);
    forget_settings(&file);
}

#[test]
fn a_row_carried_in_the_middle_of_a_long_list_does_not_scroll_it() {
    let more: Vec<(String, String)> = (0..60)
        .map(|n| (format!("zz{n:02}.jq"), ".".to_owned()))
        .collect();
    let more: Vec<(&str, &str)> = more.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (mut h, file) = start_with_more("ws-drag-noscroll", OPEN, &more);
    let from = row_at(&h, "scratch");
    let middle = egui::pos2(from.x, from.y + 150.0);
    pick_up(&mut h, from, middle);
    h.pause(2.0);
    assert!(
        is_drawn(&h, "Logs") && !is_drawn(&h, "zz59"),
        "{:?}",
        h.texts()
    );
    let_go(&mut h, middle);
    forget_settings(&file);
}

#[test]
fn over_an_item_whose_folder_is_out_of_view_that_item_is_lit_and_not_the_whole_list() {
    // A folder with a long list in it, and the folder after it with one item.
    let mut more: Vec<(String, String)> = (0..60)
        .map(|n| (format!("Yyy/a{n:02}.jq"), ".".to_owned()))
        .collect();
    more.push(("Zzz/zee.jq".to_owned(), ".".to_owned()));
    let more: Vec<(&str, &str)> = more.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (mut h, file) = start_with_more("ws-drag-folder-out-of-view", OPEN, &more);
    // The list is scrolled so that the long folder's own row has gone and its items have not.
    let pane = h.panel("workspace_pane").expect("pane");
    h.pointer(pane.center().x, pane.center().y);
    h.frame_with(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -800.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }]);
    h.pause(1.0);
    assert!(!is_drawn(&h, "Yyy"), "the folder is out of view");
    assert!(
        is_drawn(&h, "a50") && is_drawn(&h, "zee"),
        "{:?}",
        h.texts()
    );
    let (from, over) = (row_at(&h, "zee"), row_at(&h, "a50"));
    pick_up(&mut h, from, over);
    let lit = lit(&h);
    assert_eq!(lit.len(), 1, "one place is lit: {lit:?}");
    assert!(
        lit[0].height() < 30.0 && lit[0].contains(over),
        "the item that is under the pointer is lit, and not the whole list: {:?}",
        lit[0]
    );
    assert!(
        is_drawn(&h, "Move “zee” into “Yyy”"),
        "and it says where: {:?}",
        h.texts()
    );
    let_go(&mut h, over);
    assert!(folder_of(&file).join("Yyy/zee.jq").is_file());
    forget_settings(&file);
}

// ---- what a move does to the rest ---------------------------------------------------------

#[test]
fn an_item_that_is_in_the_query_box_is_still_said_to_be_where_it_came_from_after_it_moves() {
    let (mut h, file) = start("ws-drag-opened", OPEN);
    let at = row_at(&h, "scratch");
    h.double_click(at.x, at.y);
    assert!(is_drawn_containing(&h, "scratch.jq"), "{:?}", h.texts());
    drag_row(&mut h, "scratch", "Logs");
    assert!(
        is_drawn_containing(&h, "Logs / scratch.jq"),
        "{:?}",
        h.texts()
    );
    forget_settings(&file);
}

#[test]
fn a_move_by_dragging_leaves_the_rest_of_the_window_alone() {
    let (mut h, file) = start("ws-drag-alone", OPEN);
    h.app.query_text = ".keep".to_owned();
    h.settle();
    drag_row(&mut h, "scratch", "Services");
    assert_eq!(h.app.query_text, ".keep");
    assert!(
        h.app.workspace_pane.take_actions().is_empty(),
        "nothing was opened"
    );
    assert_eq!(
        h.app.settings.interface.workspace,
        crate::settings::WorkspaceView::Open
    );
    forget_settings(&file);
}

#[test]
fn the_same_rows_can_be_moved_again_and_back() {
    let (mut h, file) = start("ws-drag-back", OPEN);
    let dir = folder_of(&file);
    let id = index_of(&dir)["items"]["scratch.jq"]["id"].clone();
    drag_row(&mut h, "scratch", "Services");
    let to = empty_part(&h);
    let from = row_at(&h, "scratch");
    drag(&mut h, from, to);
    assert_eq!(read(&dir, "scratch.jq"), ".");
    assert!(!exists(&dir, "Services/scratch.jq"));
    assert_eq!(
        index_of(&dir)["items"]["scratch.jq"]["id"],
        id,
        "the same item all the way"
    );
    forget_settings(&file);
}

#[test]
fn a_list_that_is_narrowed_has_rows_that_can_be_carried_too() {
    let (mut h, file) = start("ws-drag-narrowed", OPEN);
    let dir = folder_of(&file);
    // Narrowed to what has "o" in it: the folders that have such items, open.
    pick(&mut h, "Logs");
    let filter = h
        .panel("workspace_pane")
        .map(|p| egui::pos2(p.center().x, p.top() + 55.0))
        .unwrap();
    h.click(filter.x, filter.y);
    h.type_text("orders");
    assert!(
        is_drawn(&h, "orders") && !is_drawn(&h, "scratch"),
        "{:?}",
        h.texts()
    );
    let (from, to) = (row_at(&h, "orders"), row_at(&h, "Services"));
    drag(&mut h, from, to);
    assert!(
        exists(&dir, "Services/orders.schema.json"),
        "it is in Services already: not moved"
    );
    assert_eq!(notice(&h), None);
    forget_settings(&file);
}

#[test]
fn what_the_fixture_holds_is_all_still_there_after_every_refusal() {
    let (mut h, file) = start("ws-drag-refusals", OPEN);
    let dir = folder_of(&file);
    drag_row(&mut h, "Logs", "Errors");
    drag_along(&mut h, "scratch");
    drag_row(&mut h, "bump", "Services");
    for (rel, text) in FILES {
        assert_eq!(&read(&dir, rel), text, "{rel}");
    }
    forget_settings(&file);
}
