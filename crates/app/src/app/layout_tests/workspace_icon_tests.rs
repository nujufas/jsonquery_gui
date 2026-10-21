//! The pictures of the workspace pane: the button in the status bar and the strip of a
//! folded pane have the window with its left column filled, the button that makes a
//! folder has a folder with a plus left out of it, and the folders of the list are the plain
//! picture of a directory — a body with a tab at its top left, in one colour.

use super::workspace_tests::{button_at, row_at, start, OPEN};
use super::*;
use crate::workspace_pane::button_id;

/// A rectangle that was painted filled, and how it was (its colour, and whether it has
/// an outline).
#[derive(Clone, Debug)]
struct Filled {
    rect: egui::Rect,
    color: egui::Color32,
    outlined: bool,
}

/// Every shape of the last frame that lies inside `area`, as what it is.
fn shapes_in(h: &Harness, area: egui::Rect) -> (Vec<Filled>, usize) {
    fn walk(shape: &egui::Shape, area: egui::Rect, rects: &mut Vec<Filled>, others: &mut usize) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, area, rects, others)),
            egui::Shape::Rect(r) => {
                // What is bigger than the area (the button's own ground, a panel's) is
                // not in it.
                if area.contains_rect(r.rect) {
                    rects.push(Filled {
                        rect: r.rect,
                        color: r.fill,
                        outlined: r.stroke.width > 0.0 && r.stroke.color.a() > 0,
                    });
                }
            }
            egui::Shape::Text(_) | egui::Shape::Noop => {}
            other => {
                let bounds = other.visual_bounding_rect();
                if bounds.is_finite() && area.contains_rect(bounds) {
                    *others += 1;
                }
            }
        }
    }
    let (mut rects, mut others) = (Vec::new(), 0);
    for clipped in &h.shapes {
        walk(&clipped.shape, area, &mut rects, &mut others);
    }
    (rects, others)
}

/// The pieces of a picture drawn in `area`, apart from the button they are on (which is
/// as big as the area), and that nothing else but rectangles is.
fn pieces_in(h: &Harness, area: egui::Rect) -> Vec<Filled> {
    let (rects, others) = shapes_in(h, area);
    assert_eq!(others, 0, "only rectangles: in {area:?}");
    rects
        .into_iter()
        .filter(|r| r.rect.width() < area.width() - 2.0)
        .collect()
}

/// The two rectangles that a folder is — its tab, which is the higher and the
/// narrower, and its body — drawn in `area`, apart from the button they are on.
fn folder_in(h: &Harness, area: egui::Rect) -> (Filled, Filled) {
    let (rects, others) = shapes_in(h, area);
    let mut pieces: Vec<Filled> = rects
        .into_iter()
        .filter(|r| r.rect.width() < area.width() - 2.0 && r.color.a() > 0)
        .collect();
    assert_eq!(pieces.len(), 2, "a tab and a body: {pieces:?} in {area:?}");
    assert_eq!(
        others, 0,
        "and nothing else (no lines, no plus): in {area:?}"
    );
    pieces.sort_by(|a, b| a.rect.top().total_cmp(&b.rect.top()));
    (pieces[0].clone(), pieces[1].clone())
}

/// The window and its left column, drawn in `area`: the line round the window (which has
/// no fill) and the column (which has no line).
fn pane_in(h: &Harness, area: egui::Rect) -> (Filled, Filled) {
    let pieces = pieces_in(h, area);
    assert_eq!(
        pieces.len(),
        2,
        "a window and a column: {pieces:?} in {area:?}"
    );
    let window = pieces.iter().find(|p| p.outlined).cloned();
    let column = pieces.iter().find(|p| !p.outlined).cloned();
    match (window, column) {
        (Some(window), Some(column)) => (window, column),
        _ => panic!("one with a line and one filled: {pieces:?}"),
    }
}

/// What `pieces` cover, a string for each row of pixels: `#` where something is, `.` where
/// not (at a scale of one pixel to the point).
fn art_of(pieces: &[Filled]) -> Vec<String> {
    let filled: Vec<&Filled> = pieces.iter().filter(|p| p.color.a() > 0).collect();
    let whole = filled
        .iter()
        .fold(egui::Rect::NOTHING, |all, p| all.union(p.rect));
    let (width, height) = (
        whole.width().round() as usize,
        whole.height().round() as usize,
    );
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| {
                    let at =
                        egui::pos2(whole.left() + x as f32 + 0.5, whole.top() + y as f32 + 0.5);
                    if filled.iter().any(|p| p.rect.contains(at)) {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect()
        })
        .collect()
}

/// The folder with a plus left out of it, as the pixels it covers.
const NEW_FOLDER: [&str; 11] = [
    "######.........",
    "######.........",
    "###############",
    "###############",
    "#######.#######",
    "#######.#######",
    "#####.....#####",
    "#######.#######",
    "#######.#######",
    "###############",
    "###############",
];

/// The rectangle the button called `name` fills.
fn button_rect(h: &Harness, name: &str) -> egui::Rect {
    h.ctx
        .read_response(button_id(name))
        .unwrap_or_else(|| panic!("the {name} button is drawn"))
        .rect
}

/// What the picture of the pane looks like, whatever the colour and wherever it is: a line
/// round a window 14 wide and 12 high, and its left column, five wide, filled, in the same
/// colour as the line.
fn assert_is_the_pane(window: &Filled, column: &Filled, what: &str) {
    assert_eq!(window.color.a(), 0, "{what}: the window is empty");
    assert!(
        (window.rect.width() - 14.0).abs() <= 1.0 && (window.rect.height() - 12.0).abs() <= 1.0,
        "{what}: 14 by 12: {window:?}"
    );
    assert!(
        (column.rect.width() - 5.0).abs() <= 1.0,
        "{what}: the column is five wide: {column:?}"
    );
    assert!(
        (column.rect.height() - window.rect.height()).abs() < 0.6
            && (column.rect.top() - window.rect.top()).abs() < 0.6,
        "{what}: as high as the window: {column:?} {window:?}"
    );
    assert!(
        (column.rect.left() - window.rect.left()).abs() < 0.6,
        "{what}: at its left"
    );
}

/// What a folder looks like, whatever the colour and wherever it is: the tab and the
/// body, from the left top, in one colour and without an outline.
fn assert_is_a_plain_folder(tab: &Filled, body: &Filled, what: &str) {
    assert_eq!(tab.color, body.color, "{what}: one colour");
    assert!(
        !tab.outlined && !body.outlined,
        "{what}: no outline, it is solid"
    );
    assert!(
        (body.rect.width() - 14.0).abs() <= 1.0,
        "{what}: 14 wide: {body:?}"
    );
    assert!(
        (9.0..=10.0).contains(&(body.rect.height() + 0.0)),
        "{what}: body {body:?}"
    );
    let whole = tab.rect.union(body.rect);
    assert!((whole.width() - 14.0).abs() <= 1.0, "{what}: {whole:?}");
    assert!(
        (whole.height() - 11.0).abs() <= 1.0,
        "{what}: 11 high: {whole:?}"
    );
    assert!(
        (tab.rect.left() - body.rect.left()).abs() < 0.6,
        "{what}: the tab is at the left"
    );
    assert!(tab.rect.top() < body.rect.top(), "{what}: and on top");
    assert!(
        tab.rect.width() < body.rect.width() / 2.0 + 0.6,
        "{what}: and narrower than half"
    );
    assert!(
        tab.rect.height() >= 2.0,
        "{what}: with a height to see: {tab:?}"
    );
}

#[test]
fn the_status_bars_button_is_a_window_with_its_left_column_filled() {
    let (h, file) = start("ws-icon-status", OPEN);
    let area = button_rect(&h, "status");
    let (window, column) = pane_in(&h, area);
    assert_is_the_pane(&window, &column, "the status bar");
    // In the middle of its button.
    let centre = button_at(&h, "status");
    let whole = window.rect.union(column.rect);
    assert!(
        (whole.center().x - centre.x).abs() <= 1.0,
        "{whole:?} {centre:?}"
    );
    assert!(
        (whole.center().y - centre.y).abs() <= 1.0,
        "{whole:?} {centre:?}"
    );
    forget_settings(&file);
}

#[test]
fn the_button_that_makes_a_folder_is_a_folder_with_a_plus_left_out_of_it() {
    let (h, file) = start("ws-icon-new", OPEN);
    let area = button_rect(&h, "new folder");
    let pieces = pieces_in(&h, area);
    assert_eq!(art_of(&pieces), NEW_FOLDER);
    // One colour and no outline: the plus is a gap, and what shows in it is the button.
    assert!(
        pieces
            .iter()
            .all(|p| p.color == pieces[0].color && !p.outlined),
        "{pieces:?}"
    );
    // In the middle of its button.
    let whole = pieces
        .iter()
        .fold(egui::Rect::NOTHING, |all, p| all.union(p.rect));
    let centre = button_at(&h, "new folder");
    assert!(
        (whole.center().x - centre.x).abs() <= 1.0,
        "{whole:?} {centre:?}"
    );
    assert!(
        (whole.center().y - centre.y).abs() <= 1.0,
        "{whole:?} {centre:?}"
    );
    forget_settings(&file);
}

#[test]
fn the_strip_of_a_folded_pane_has_the_same_picture_as_the_status_bar() {
    let (mut h, file) = start("ws-icon-rail", r#"{"workspace": {"pane": "collapsed"}}"#);
    h.settle();
    assert!(h.panel("workspace_rail").is_some(), "the strip is drawn");
    let (status_window, status_column) = pane_in(&h, button_rect(&h, "status"));
    // Under the button that unfolds the pane, in the width of the strip.
    let unfold = button_rect(&h, "unfold");
    let under = egui::Rect::from_min_max(
        egui::pos2(0.0, unfold.bottom() + 2.0),
        egui::pos2(crate::workspace_pane::RAIL_WIDTH, unfold.bottom() + 40.0),
    );
    let (window, column) = pane_in(&h, under);
    assert_is_the_pane(&window, &column, "the strip");
    assert_eq!(window.rect.size(), status_window.rect.size());
    assert_eq!(column.rect.size(), status_column.rect.size());
    forget_settings(&file);
}

#[test]
fn a_folder_in_the_list_is_the_same_picture_open_or_shut() {
    let (mut h, file) = start("ws-icon-rows", OPEN);
    // Logs is open, Errors is shut.
    assert!(is_drawn(&h, "logins") && !is_drawn(&h, "slow requests"));
    let row = |h: &Harness, name: &str| {
        let at = row_at(h, name);
        egui::Rect::from_min_max(
            egui::pos2(at.x - 40.0, at.y - 10.0),
            egui::pos2(at.x - 8.0 - 3.0, at.y + 10.0),
        )
    };
    // The picture is between the little arrow and the name.
    let (open_tab, open_body) = folder_in(&h, row(&h, "Logs"));
    let (shut_tab, shut_body) = folder_in(&h, row(&h, "Errors"));
    assert_is_a_plain_folder(&open_tab, &open_body, "an open folder");
    assert_is_a_plain_folder(&shut_tab, &shut_body, "a shut folder");
    assert_eq!(open_tab.rect.size(), shut_tab.rect.size());
    assert_eq!(open_body.rect.size(), shut_body.rect.size());
    assert_eq!(open_body.color, shut_body.color, "and the same colour");
    // Shutting the open one changes nothing about its picture.
    pick(&mut h, "Logs");
    let (tab, body) = folder_in(&h, row(&h, "Logs"));
    assert_eq!(tab.rect.size(), open_tab.rect.size());
    assert_eq!(body.rect.size(), open_body.rect.size());
    forget_settings(&file);
}

#[test]
fn an_item_has_no_folder_picture() {
    let (h, file) = start("ws-icon-item", OPEN);
    let at = row_at(&h, "scratch");
    let area = egui::Rect::from_min_max(
        egui::pos2(at.x - 40.0, at.y - 10.0),
        egui::pos2(at.x - 8.0, at.y + 10.0),
    );
    let (rects, _) = shapes_in(&h, area);
    let solid: Vec<_> = rects
        .iter()
        .filter(|r| r.color.a() > 0 && r.rect.width() < 30.0)
        .collect();
    assert!(
        solid
            .iter()
            .all(|r| r.rect.width() < 1.0 || r.rect.height() < 1.0),
        "{solid:?}"
    );
    forget_settings(&file);
}

#[test]
fn the_picture_of_the_status_bar_is_dim_while_the_pane_is_hidden_and_not_when_it_is_shown() {
    let (mut h, file) = start("ws-icon-dim", "{}");
    let weak = h.ctx.global_style().visuals.weak_text_color();
    let (hidden_window, hidden) = pane_in(&h, button_rect(&h, "status"));
    assert_eq!(hidden.color, weak, "hidden: dim");
    assert_eq!(hidden_window.color.a(), 0);
    let at = button_at(&h, "status");
    h.click(at.x, at.y);
    let (_, shown) = pane_in(&h, button_rect(&h, "status"));
    assert_ne!(
        shown.color, weak,
        "shown: as bright as the buttons beside it"
    );
    h.click(at.x, at.y);
    let (_, hidden_again) = pane_in(&h, button_rect(&h, "status"));
    assert_eq!(hidden_again.color, weak);
    forget_settings(&file);
}

#[test]
fn the_pictures_are_drawn_in_the_colour_of_the_theme() {
    for (theme, name) in [("light", "ws-icon-light"), ("dark", "ws-icon-dark")] {
        let (h, file) = start(
            name,
            &format!(r#"{{"theme": "{theme}", "workspace": {{"pane": "open"}}}}"#),
        );
        let buttons = h
            .ctx
            .global_style()
            .visuals
            .widgets
            .inactive
            .fg_stroke
            .color;
        let folder = pieces_in(&h, button_rect(&h, "new folder"));
        assert!(
            folder.iter().all(|p| p.color == buttons),
            "{theme}: as the text of the buttons: {folder:?}"
        );
        // The status bar's button is dim as long as the pane is hidden, and this one is not.
        let (_, column) = pane_in(&h, button_rect(&h, "status"));
        assert_eq!(column.color, buttons, "{theme}");
        forget_settings(&file);
    }
}

use super::workspace_tests::pick;

#[test]
fn the_strip_of_a_folded_pane_is_wide_enough_for_its_button_and_its_picture() {
    let (h, file) = start(
        "ws-icon-rail-room",
        r#"{"workspace": {"pane": "collapsed"}}"#,
    );
    let rail = h.panel("workspace_rail").expect("the strip is drawn");
    let visible = egui::Rect::from_min_max(
        egui::pos2(rail.left().max(0.0), rail.top()),
        rail.right_bottom(),
    );
    let unfold = button_rect(&h, "unfold");
    assert!(
        visible.contains_rect(unfold),
        "the button is cut off: {unfold:?} in {visible:?}"
    );
    let under = egui::Rect::from_min_max(
        egui::pos2(0.0, unfold.bottom() + 2.0),
        egui::pos2(crate::workspace_pane::RAIL_WIDTH, unfold.bottom() + 40.0),
    );
    let (window, column) = pane_in(&h, under);
    let picture = window.rect.union(column.rect);
    assert!(
        visible.contains_rect(picture),
        "the picture is cut off: {picture:?} in {visible:?}"
    );
    // And what the window holds starts where the strip ends.
    assert!(
        (rail.right() - crate::workspace_pane::RAIL_WIDTH + 0.5).abs() < 1.5,
        "{rail:?}"
    );
    forget_settings(&file);
}

#[test]
fn the_pictures_lie_on_whole_pixels_at_any_scale_so_that_their_edges_are_sharp() {
    for scale in [1.0_f32, 1.25, 1.5, 2.0, 3.0] {
        let (mut h, file) = start("ws-icon-scale", OPEN);
        h.ctx.set_pixels_per_point(scale);
        h.settle();
        let on_pixels = |what: &str, rect: egui::Rect| {
            for (edge, value) in [
                ("left", rect.left()),
                ("top", rect.top()),
                ("right", rect.right()),
                ("bottom", rect.bottom()),
            ] {
                let pixels = value * scale;
                assert!(
                    (pixels - pixels.round()).abs() < 0.01,
                    "{what}, {edge}: {value} points, {pixels} pixels, at a scale of {scale}"
                );
            }
        };
        let folder = pieces_in(&h, button_rect(&h, "new folder"));
        for piece in &folder {
            on_pixels("the new folder", piece.rect);
        }
        let (window, column) = pane_in(&h, button_rect(&h, "status"));
        on_pixels("the window", window.rect);
        on_pixels("the column", column.rect);
        // The same size in points at every scale (to a pixel).
        let whole = folder
            .iter()
            .fold(egui::Rect::NOTHING, |all, p| all.union(p.rect));
        assert!(
            (whole.width() - 15.0).abs() <= 1.0 / scale + 0.01,
            "{whole:?} at {scale}"
        );
        assert!(
            (whole.height() - 11.0).abs() <= 1.0 / scale + 0.01,
            "{whole:?} at {scale}"
        );
        assert!(
            (window.rect.width() - 14.0).abs() <= 1.0 / scale + 0.01,
            "{window:?} at {scale}"
        );
        forget_settings(&file);
    }
}
