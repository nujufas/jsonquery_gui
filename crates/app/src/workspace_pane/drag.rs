//! Moving what is in the list by dragging it. A row is picked up and carried over the
//! list: the folder it would go into is lit, and is opened if the pointer rests on it
//! for a moment; let go over a folder, the row goes into it; over an item, into the
//! folder that item is in; over the empty part of the list, to the top. What cannot be
//! done (into the folder it is in already, a folder into itself) is not lit, and
//! letting go there does nothing — nor does letting go outside the list, or Escape.
//! The moving itself is `Workspace::move_to`, the same as the menu's *Move to*.
//!
//! A query can also be carried out of the list and put down on the query pane, which is
//! lit, and it is opened there, as a double click on it opens it.
//!
//! This file has what can be said without a window — what a drop would do, what the
//! carried row says, when a folder opens, how fast the list scrolls — and the few
//! pictures; `tree.rs` has the list and puts it together.

use std::sync::Arc;

use eframe::egui::{self, vec2, Pos2, Rect, Stroke, StrokeKind};

use crate::workspace::{last_part, parent_of};

/// How long the pointer rests on a shut folder, carrying a row, before it opens.
pub(super) const OPEN_AFTER: f64 = 0.7;

/// How near the top or the bottom of the list the pointer has to be, carrying a row,
/// for the list to scroll.
const EDGE: f32 = 24.0;

/// How fast the list scrolls, in points a second, with the pointer at the very edge.
const SCROLL_SPEED: f32 = 480.0;

/// What is carried: the row that was picked up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Carried {
    /// Where it is in the workspace.
    pub path: String,
    /// What its row says.
    pub name: String,
    pub is_folder: bool,
}

/// What would happen if what is carried were put down in a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// It would go there.
    Moves,
    /// It is in that folder already.
    AlreadyThere,
    /// A folder cannot go into itself, nor into a folder that is in it.
    IntoItself,
}

/// A row of the list as a place to put something down: where its rectangle is, and
/// the folder that what is put down on it goes into.
#[derive(Clone, Debug)]
pub(super) struct Spot {
    pub rect: Rect,
    /// A folder row's own path; for an item, the folder it is in.
    pub folder: String,
    pub is_folder: bool,
    /// A folder that is shut.
    pub shut: bool,
}

/// Whether `carried` may be put down in the folder `folder` (the empty path is the
/// top of the workspace).
pub(super) fn verdict(carried: &Carried, folder: &str) -> Verdict {
    if carried.is_folder
        && (folder == carried.path || folder.starts_with(&format!("{}/", carried.path)))
    {
        return Verdict::IntoItself;
    }
    if parent_of(&carried.path) == folder {
        return Verdict::AlreadyThere;
    }
    Verdict::Moves
}

/// What the row that is carried says, over the query pane, where it would be opened:
/// `Open “slow requests” as the query`.
pub(super) fn open_text(carried: &Carried) -> String {
    format!("Open “{}” as the query", carried.name)
}

/// Where `folder` is said to be: its path with spaces round the slashes, as the
/// query row says it, or the top.
pub(super) fn place(folder: &str) -> String {
    if folder.is_empty() {
        "the top".to_owned()
    } else {
        format!("“{}”", folder.replace('/', " / "))
    }
}

/// What the row that is carried says beside the pointer: its name, and, over a place
/// where it can be put down, where it would go.
pub(super) fn ghost_text(carried: &Carried, over: Option<(&str, Verdict)>) -> String {
    match over {
        Some(("", Verdict::Moves)) => format!("Move “{}” to the top", carried.name),
        Some((folder, Verdict::Moves)) => {
            format!("Move “{}” into {}", carried.name, place(folder))
        }
        _ => carried.name.clone(),
    }
}

/// What is said under the list when a row was moved: `Moved “x” into “Logs”`.
pub(super) fn moved_text(name: &str, folder: &str) -> String {
    if folder.is_empty() {
        format!("Moved “{name}” to the top")
    } else {
        format!("Moved “{name}” into {}", place(folder))
    }
}

/// Resting on a shut folder for long enough opens it. `held` is the folder the
/// pointer was resting on and since when (the clock of the window), `over` the shut
/// folder it is on now (none if it is not on one). Gives the folder to open, once
/// it is time.
pub(super) fn dwell(
    held: &mut Option<(String, f64)>,
    over: Option<&str>,
    now: f64,
) -> Option<String> {
    match (over, held.as_ref()) {
        (Some(path), Some((there, _))) if there == path => {}
        (Some(path), _) => *held = Some((path.to_owned(), now)),
        (None, _) => *held = None,
    }
    let (path, since) = held.as_ref()?;
    if now - since < OPEN_AFTER {
        return None;
    }
    let path = path.clone();
    *held = None;
    Some(path)
}

/// How many points the list is to be scrolled in a frame of `dt` seconds with the
/// pointer at height `y` and the list between `top` and `bottom`: down (the content
/// moves up, a negative number) near the bottom, up (a positive number) near the top,
/// faster the nearer the edge, and not at all in the middle or outside the list.
pub(super) fn edge_scroll(y: f32, top: f32, bottom: f32, dt: f32) -> f32 {
    if y < top || y > bottom {
        return 0.0;
    }
    let band = EDGE.min((bottom - top) / 2.0);
    let from_top = y - top;
    let from_bottom = bottom - y;
    if from_top < band {
        SCROLL_SPEED * dt * (band - from_top) / band
    } else if from_bottom < band {
        -SCROLL_SPEED * dt * (band - from_bottom) / band
    } else {
        0.0
    }
}

/// The row that is carried, while one is.
pub(super) fn carried(ctx: &egui::Context) -> Option<Arc<Carried>> {
    egui::DragAndDrop::payload::<Carried>(ctx)
}

/// The row is put down, or the button was let go where nobody saw: nothing is carried.
pub(super) fn put_down(ctx: &egui::Context) {
    let _ = egui::DragAndDrop::take_payload::<Carried>(ctx);
}

/// A folder's row, lit as the place that what is carried would go into.
pub(super) fn light_row(ui: &egui::Ui, clip: Rect, row: Rect) {
    let visuals = ui.visuals();
    let painter = ui.painter_at(clip);
    painter.rect(
        row.shrink2(vec2(1.0, 0.0)),
        2.0,
        visuals.selection.bg_fill.linear_multiply(0.4),
        Stroke::new(1.5, visuals.selection.stroke.color),
        StrokeKind::Inside,
    );
}

/// The whole list, outlined as the place that what is carried would go into (the top).
pub(super) fn light_list(ui: &egui::Ui, list: Rect) {
    let visuals = ui.visuals();
    ui.painter_at(list).rect_stroke(
        list.shrink(0.75),
        2.0,
        Stroke::new(1.5, visuals.selection.stroke.color),
        StrokeKind::Inside,
    );
}

/// The query pane, outlined as the place where a query that is carried would be opened. It
/// is drawn in front of everything, the pane being one the list is not in.
pub(super) fn light_query(ui: &egui::Ui, area: Rect) {
    let visuals = ui.visuals();
    ui.ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("workspace_query_target"),
        ))
        .rect_stroke(
            area.shrink(0.75),
            2.0,
            Stroke::new(1.5, visuals.selection.stroke.color),
            StrokeKind::Inside,
        );
}

/// What is carried, beside the pointer.
pub(super) fn show_ghost(ctx: &egui::Context, pointer: Pos2, text: &str) {
    egui::Area::new(egui::Id::new("workspace_ghost"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .fixed_pos(pointer + vec2(14.0, 16.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.add(egui::Label::new(text).extend().selectable(false));
            });
        });
}

/// The last part of a path, for what is said of a row that is not an item.
pub(super) fn name_of_folder(path: &str) -> String {
    last_part(path).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str) -> Carried {
        Carried {
            path: path.to_owned(),
            name: last_part(path).to_owned(),
            is_folder: false,
        }
    }

    fn folder(path: &str) -> Carried {
        Carried {
            is_folder: true,
            ..item(path)
        }
    }

    #[test]
    fn an_item_goes_into_any_folder_but_the_one_it_is_in() {
        let a = item("Logs/Errors/slow.jq");
        assert_eq!(verdict(&a, ""), Verdict::Moves);
        assert_eq!(verdict(&a, "Logs"), Verdict::Moves);
        assert_eq!(verdict(&a, "Services"), Verdict::Moves);
        assert_eq!(verdict(&a, "Logs/Errors"), Verdict::AlreadyThere);
        // An item at the top is in the top.
        let top = item("scratch.jq");
        assert_eq!(verdict(&top, ""), Verdict::AlreadyThere);
        assert_eq!(verdict(&top, "Logs"), Verdict::Moves);
    }

    #[test]
    fn an_item_may_go_into_a_folder_that_is_called_like_the_one_it_is_in_somewhere_else() {
        let a = item("Logs/a.jq");
        assert_eq!(verdict(&a, "Old/Logs"), Verdict::Moves);
        assert_eq!(verdict(&a, "LogsOld"), Verdict::Moves);
    }

    #[test]
    fn a_folder_goes_into_another_but_not_into_itself_nor_into_one_in_it_nor_where_it_is() {
        let errors = folder("Logs/Errors");
        assert_eq!(verdict(&errors, ""), Verdict::Moves);
        assert_eq!(verdict(&errors, "Services"), Verdict::Moves);
        assert_eq!(verdict(&errors, "Logs"), Verdict::AlreadyThere);
        assert_eq!(verdict(&errors, "Logs/Errors"), Verdict::IntoItself);
        assert_eq!(verdict(&errors, "Logs/Errors/Old"), Verdict::IntoItself);
        assert_eq!(
            verdict(&errors, "Logs/Errors/Old/Deeper"),
            Verdict::IntoItself
        );
        // A folder whose name only starts like this one's is another folder.
        assert_eq!(verdict(&errors, "Logs/Errors2"), Verdict::Moves);
        let logs = folder("Logs");
        assert_eq!(verdict(&logs, "Logs/Errors"), Verdict::IntoItself);
        assert_eq!(verdict(&logs, ""), Verdict::AlreadyThere);
        assert_eq!(verdict(&logs, "LogsOld"), Verdict::Moves);
    }

    #[test]
    fn the_ghost_says_the_name_and_where_it_would_go_when_it_can() {
        let a = item("Logs/slow requests.jq");
        let named = Carried {
            name: "slow requests".to_owned(),
            ..a
        };
        assert_eq!(ghost_text(&named, None), "slow requests");
        assert_eq!(
            ghost_text(&named, Some(("Services", Verdict::Moves))),
            "Move “slow requests” into “Services”"
        );
        assert_eq!(
            ghost_text(&named, Some(("Logs/Errors", Verdict::Moves))),
            "Move “slow requests” into “Logs / Errors”"
        );
        assert_eq!(
            ghost_text(&named, Some(("", Verdict::Moves))),
            "Move “slow requests” to the top"
        );
        assert_eq!(
            ghost_text(&named, Some(("Logs", Verdict::AlreadyThere))),
            "slow requests"
        );
        assert_eq!(
            ghost_text(&named, Some(("Logs", Verdict::IntoItself))),
            "slow requests"
        );
    }

    #[test]
    fn over_the_query_pane_the_ghost_says_that_the_query_would_be_opened() {
        let slow = Carried {
            name: "slow requests".to_owned(),
            ..item("Logs/slow requests.jq")
        };
        assert_eq!(open_text(&slow), "Open “slow requests” as the query");
    }

    #[test]
    fn what_is_said_after_a_move_names_the_row_and_the_place() {
        assert_eq!(
            moved_text("orders", "Services/Old"),
            "Moved “orders” into “Services / Old”"
        );
        assert_eq!(moved_text("orders", ""), "Moved “orders” to the top");
        assert_eq!(place(""), "the top");
        assert_eq!(place("A"), "“A”");
    }

    #[test]
    fn a_shut_folder_opens_after_the_pointer_has_rested_on_it_and_not_before() {
        let mut held = None;
        assert_eq!(dwell(&mut held, Some("Logs"), 10.0), None);
        assert_eq!(
            dwell(&mut held, Some("Logs"), 10.0 + OPEN_AFTER - 0.01),
            None
        );
        assert_eq!(
            dwell(&mut held, Some("Logs"), 10.0 + OPEN_AFTER + 0.001),
            Some("Logs".to_owned())
        );
        // Once it has opened, the wait begins again: it is not asked again at once.
        assert_eq!(held, None);
        assert_eq!(dwell(&mut held, Some("Logs"), 11.0), None);
    }

    #[test]
    fn leaving_a_folder_for_another_starts_the_wait_again() {
        let mut held = None;
        assert_eq!(dwell(&mut held, Some("A"), 1.0), None);
        assert_eq!(dwell(&mut held, Some("B"), 1.5), None);
        // A had been there since 1.0, but B only since 1.5.
        assert_eq!(dwell(&mut held, Some("B"), 1.0 + OPEN_AFTER + 0.1), None);
        assert_eq!(
            dwell(&mut held, Some("B"), 1.5 + OPEN_AFTER + 0.001),
            Some("B".to_owned())
        );
    }

    #[test]
    fn leaving_every_folder_forgets_the_wait() {
        let mut held = None;
        assert_eq!(dwell(&mut held, Some("A"), 1.0), None);
        assert_eq!(dwell(&mut held, None, 1.2), None);
        assert_eq!(held, None);
        assert_eq!(dwell(&mut held, Some("A"), 1.2 + OPEN_AFTER), None);
        assert_eq!(
            dwell(&mut held, Some("A"), 1.2 + 2.0 * OPEN_AFTER + 0.001),
            Some("A".to_owned())
        );
    }

    #[test]
    fn the_list_scrolls_near_its_edges_and_faster_the_nearer_the_edge() {
        let (top, bottom, dt) = (100.0, 300.0, 1.0 / 60.0);
        // In the middle, and outside the list: not at all.
        assert_eq!(edge_scroll(200.0, top, bottom, dt), 0.0);
        assert_eq!(edge_scroll(top + EDGE, top, bottom, dt), 0.0);
        assert_eq!(edge_scroll(bottom - EDGE, top, bottom, dt), 0.0);
        assert_eq!(edge_scroll(50.0, top, bottom, dt), 0.0);
        assert_eq!(edge_scroll(350.0, top, bottom, dt), 0.0);
        // Near the top, the content moves down; near the bottom, up.
        let up = edge_scroll(top + 6.0, top, bottom, dt);
        let up_more = edge_scroll(top + 1.0, top, bottom, dt);
        assert!(up > 0.0 && up_more > up, "{up} {up_more}");
        let down = edge_scroll(bottom - 6.0, top, bottom, dt);
        let down_more = edge_scroll(bottom - 1.0, top, bottom, dt);
        assert!(down < 0.0 && down_more < down, "{down} {down_more}");
        // At the very edge: the whole speed.
        let at_edge = edge_scroll(top, top, bottom, 1.0);
        assert!((at_edge - SCROLL_SPEED).abs() < 0.01, "{at_edge}");
        let at_foot = edge_scroll(bottom, top, bottom, 1.0);
        assert!((at_foot + SCROLL_SPEED).abs() < 0.01, "{at_foot}");
        // A longer frame goes further.
        assert!(edge_scroll(top + 1.0, top, bottom, 0.1) > up_more);
    }

    #[test]
    fn a_list_shorter_than_two_edges_still_scrolls_towards_the_nearer_one() {
        // 30 points high: the bands are 15 each, and meet in the middle.
        assert!(edge_scroll(105.0, 100.0, 130.0, 0.1) > 0.0);
        assert!(edge_scroll(125.0, 100.0, 130.0, 0.1) < 0.0);
        assert_eq!(edge_scroll(115.0, 100.0, 130.0, 0.1), 0.0);
    }

    #[test]
    fn a_folders_name_is_the_last_part_of_its_path() {
        assert_eq!(name_of_folder("Logs/Errors"), "Errors");
        assert_eq!(name_of_folder("Logs"), "Logs");
    }
}
