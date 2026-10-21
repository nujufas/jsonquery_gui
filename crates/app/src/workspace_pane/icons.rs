//! The small pictures of the workspace pane — the arrows of a folder, the folder,
//! the three dots of a menu — drawn with shapes and not
//! as letters, because the fonts that the window has do not all have a glyph for
//! them (an arrow or a folder that is a letter shows as an empty box in some), and
//! so that they look the same on every system.

use eframe::egui::{
    self, pos2, Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui,
};

/// How big a button of the pane's header is.
const BUTTON: egui::Vec2 = egui::vec2(24.0, 18.0);

/// A little triangle that points right (a folder that is shut) or down (one that is open).
pub(super) fn arrow(painter: &Painter, c: Pos2, open: bool, color: Color32) {
    let points = if open {
        vec![
            pos2(c.x - 3.5, c.y - 2.0),
            pos2(c.x + 3.5, c.y - 2.0),
            pos2(c.x, c.y + 2.5),
        ]
    } else {
        vec![
            pos2(c.x - 2.0, c.y - 3.5),
            pos2(c.x - 2.0, c.y + 3.5),
            pos2(c.x + 2.5, c.y),
        ]
    };
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A triangle that points left (`right` false) or right, for the button that folds the
/// pane away and the one that opens it again.
pub(super) fn pointer(painter: &Painter, c: Pos2, right: bool, color: Color32) {
    let sign = if right { 1.0 } else { -1.0 };
    let points = vec![
        pos2(c.x - 2.5 * sign, c.y - 4.0),
        pos2(c.x - 2.5 * sign, c.y + 4.0),
        pos2(c.x + 3.0 * sign, c.y),
    ];
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A folder: the plain picture of a directory that a file manager draws — a body with
/// a tab at its top left, in one colour — so that it needs no telling. It is the same
/// whether the folder is open or shut (the little arrow beside it says that) and
/// wherever it is: in the list, on the strip, on the buttons.
pub(super) fn folder(painter: &Painter, c: Pos2, color: Color32) {
    // 14 by 11 points, on whole pixels so that the edges are sharp.
    let ppp = painter.pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let (left, right) = (snap(c.x - 7.0), snap(c.x + 7.0));
    let (top, shoulder, bottom) = (snap(c.y - 5.5), snap(c.y - 3.0), snap(c.y + 5.5));
    let tab = Rect::from_min_max(pos2(left, top), pos2(snap(left + 6.0), shoulder + 1.0));
    let body = Rect::from_min_max(pos2(left, shoulder), pos2(right, bottom));
    painter.rect_filled(
        tab,
        CornerRadius {
            nw: 1,
            ne: 1,
            sw: 0,
            se: 0,
        },
        color,
    );
    painter.rect_filled(body, 1.5, color);
}

/// A picture of whole points on the pixel grid: the points of it are given from its top
/// left, which is put where a whole pixel is, so that every edge is sharp at any scale.
struct Sprite<'a> {
    painter: &'a Painter,
    origin: Pos2,
    ppp: f32,
}

impl<'a> Sprite<'a> {
    /// A picture of `width` by `height` points, in the middle of `c`.
    fn new(painter: &'a Painter, c: Pos2, width: f32, height: f32) -> Self {
        let ppp = painter.pixels_per_point();
        let snap = |v: f32| (v * ppp).round() / ppp;
        Self {
            painter,
            origin: pos2(snap(c.x - width / 2.0), snap(c.y - height / 2.0)),
            ppp,
        }
    }

    fn snap(&self, v: f32) -> f32 {
        (v * self.ppp).round() / self.ppp
    }

    /// The rectangle from (`x0`, `y0`) to (`x1`, `y1`), on whole pixels.
    fn rect(&self, [x0, y0, x1, y1]: [f32; 4]) -> Rect {
        Rect::from_min_max(
            pos2(self.snap(self.origin.x + x0), self.snap(self.origin.y + y0)),
            pos2(self.snap(self.origin.x + x1), self.snap(self.origin.y + y1)),
        )
    }

    fn fill(&self, bounds: [f32; 4], radius: CornerRadius, color: Color32) {
        self.painter.rect_filled(self.rect(bounds), radius, color);
    }

    /// A line of a whole pixel (or two, on a screen that has more), which is as thin as one
    /// can be and still sharp.
    fn line(&self) -> f32 {
        self.ppp.round().max(1.0) / self.ppp
    }
}

/// A window with its left column filled: the pane that the button shows and hides, as the
/// window's own sidebar button draws it. The same wherever it is: on the button of the
/// status bar and on the strip a folded pane leaves.
pub(super) fn side_panel(painter: &Painter, c: Pos2, color: Color32) {
    // 14 by 12 points.
    let sprite = Sprite::new(painter, c, 14.0, 12.0);
    painter.rect_stroke(
        sprite.rect([0.0, 0.0, 14.0, 12.0]),
        2.0,
        Stroke::new(sprite.line(), color),
        StrokeKind::Inside,
    );
    let round_left = CornerRadius {
        nw: 2,
        ne: 0,
        sw: 2,
        se: 0,
    };
    sprite.fill([0.0, 0.0, 5.0, 12.0], round_left, color);
}

/// A folder with a plus cut out of its body: what makes a new one. The plus is not painted
/// over the folder but left out of it, so that it is what is behind the button that shows.
pub(super) fn new_folder(painter: &Painter, c: Pos2, color: Color32) {
    // 15 by 11 points: the folder is as wide as the one of the list is, and a point more.
    let sprite = Sprite::new(painter, c, 15.0, 11.0);
    let round_top = CornerRadius {
        nw: 1,
        ne: 1,
        sw: 0,
        se: 0,
    };
    let round_foot = CornerRadius {
        nw: 0,
        ne: 0,
        sw: 1,
        se: 1,
    };
    sprite.fill([0.0, 0.0, 6.0, 3.0], round_top, color);
    // The body: a band at the top and one at the foot, which have its round corners, and,
    // between them, what is left of five rows when the plus is not there: its upright, a
    // point wide and five high, and its bar, five wide and a point high.
    sprite.fill([0.0, 2.0, 15.0, 4.0], round_top, color);
    sprite.fill([0.0, 9.0, 15.0, 11.0], round_foot, color);
    let square = CornerRadius::ZERO;
    for bounds in [
        [0.0, 4.0, 7.0, 6.0],
        [8.0, 4.0, 15.0, 6.0],
        [0.0, 6.0, 5.0, 7.0],
        [10.0, 6.0, 15.0, 7.0],
        [0.0, 7.0, 7.0, 9.0],
        [8.0, 7.0, 15.0, 9.0],
    ] {
        sprite.fill(bounds, square, color);
    }
}

/// Three dots one over the other: a menu.
pub(super) fn dots(painter: &Painter, c: Pos2, color: Color32) {
    for dy in [-4.0, 0.0, 4.0] {
        painter.circle_filled(pos2(c.x, c.y + dy), 1.4, color);
    }
}

/// The id of the button called `name`, which is the same in every frame (and which a
/// test finds a button by).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn button_id(name: &str) -> egui::Id {
    egui::Id::new(("workspace_button", name))
}

/// A button that is a picture, in the look of the buttons beside it. `draw` paints the
/// picture, centred on the button, in the colour the button's text has. `name` tells
/// it from the others.
pub(super) fn button(
    ui: &mut Ui,
    name: &str,
    draw: impl FnOnce(&Painter, Pos2, Color32),
) -> Response {
    let (rect, _) = ui.allocate_exact_size(BUTTON, Sense::hover());
    let response = ui.interact(rect, button_id(name), Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(
            rect.expand(visuals.expansion),
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );
        draw(ui.painter(), rect.center(), visuals.fg_stroke.color);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `draw` paints in a frame of its own, the lists of shapes taken apart.
    fn painted(draw: impl FnOnce(&Painter)) -> Vec<Shape> {
        painted_at(1.0, draw)
    }

    /// The same on a screen of `scale` pixels to the point.
    fn painted_at(scale: f32, draw: impl FnOnce(&Painter)) -> Vec<Shape> {
        fn flat(shape: Shape, out: &mut Vec<Shape>) {
            match shape {
                Shape::Vec(shapes) => shapes.into_iter().for_each(|s| flat(s, out)),
                other => out.push(other),
            }
        }
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(scale);
        let mut draw = Some(draw);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(draw) = draw.take() {
                draw(ui.painter());
            }
        });
        output.textures_delta.clear();
        let mut shapes = Vec::new();
        for clipped in output.shapes {
            flat(clipped.shape, &mut shapes);
        }
        shapes
    }

    /// The three corners of the one triangle that was painted.
    fn triangle(shapes: &[Shape]) -> Vec<Pos2> {
        let paths: Vec<&egui::epaint::PathShape> = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Path(path) => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(paths.len(), 1, "one triangle in {shapes:?}");
        assert_eq!(
            paths[0].points.len(),
            3,
            "a triangle: {:?}",
            paths[0].points
        );
        paths[0].points.clone()
    }

    /// The corner that no other corner shares a `key` with — the tip of a triangle that has
    /// two corners level — and the two others.
    fn tip_and_base(corners: &[Pos2], key: impl Fn(&Pos2) -> f32) -> (Pos2, [Pos2; 2]) {
        let level = |a: &Pos2, b: &Pos2| (key(a) - key(b)).abs() < 1e-3;
        let tip = corners
            .iter()
            .copied()
            .find(|c| corners.iter().filter(|o| level(c, o)).count() == 1)
            .unwrap_or_else(|| panic!("two corners are level in {corners:?}"));
        let base: Vec<Pos2> = corners.iter().copied().filter(|c| *c != tip).collect();
        assert!(
            level(&base[0], &base[1]),
            "the other two are level: {corners:?}"
        );
        (tip, [base[0], base[1]])
    }

    #[test]
    fn the_arrow_of_an_open_folder_points_down_with_its_tip_under_the_middle() {
        let c = pos2(40.0, 60.0);
        let corners = triangle(&painted(|p| arrow(p, c, true, Color32::WHITE)));
        let (tip, base) = tip_and_base(&corners, |p| p.y);
        assert!(tip.y > base[0].y, "the tip is under the base: {corners:?}");
        assert!(
            (tip.x - c.x).abs() < 1e-3,
            "the tip is in the middle: {corners:?}"
        );
        assert!(base[0].x.min(base[1].x) < c.x && base[0].x.max(base[1].x) > c.x);
    }

    #[test]
    fn the_arrow_of_a_shut_folder_points_right_with_its_tip_beside_the_middle() {
        let c = pos2(40.0, 60.0);
        let corners = triangle(&painted(|p| arrow(p, c, false, Color32::WHITE)));
        let (tip, base) = tip_and_base(&corners, |p| p.x);
        assert!(
            tip.x > base[0].x,
            "the tip is right of the base: {corners:?}"
        );
        assert!(
            (tip.y - c.y).abs() < 1e-3,
            "the tip is in the middle: {corners:?}"
        );
        assert!(base[0].y.min(base[1].y) < c.y && base[0].y.max(base[1].y) > c.y);
    }

    #[test]
    fn the_two_arrows_are_the_same_size() {
        let c = pos2(40.0, 60.0);
        let size = |open| {
            let corners = triangle(&painted(|p| arrow(p, c, open, Color32::WHITE)));
            let bounds = Rect::from_points(&corners);
            (bounds.width() - bounds.height()).abs()
        };
        // The one that points right is a little wider than high, the one that points down the
        // other way round, by the same: they are one picture turned.
        assert!((size(true) - size(false)).abs() < 1e-3);
    }

    #[test]
    fn the_button_that_folds_the_pane_points_left_and_the_one_that_opens_it_points_right() {
        let c = pos2(40.0, 60.0);
        for right in [false, true] {
            let corners = triangle(&painted(|p| pointer(p, c, right, Color32::WHITE)));
            let (tip, base) = tip_and_base(&corners, |p| p.x);
            assert!(
                (tip.y - c.y).abs() < 1e-3,
                "the tip is in the middle: {corners:?}"
            );
            if right {
                assert!(tip.x > base[0].x, "points right: {corners:?}");
            } else {
                assert!(tip.x < base[0].x, "points left: {corners:?}");
            }
        }
    }

    #[test]
    fn the_menu_is_three_dots_one_over_the_other_four_points_apart() {
        let c = pos2(40.0, 60.0);
        let shapes = painted(|p| dots(p, c, Color32::WHITE));
        let mut circles: Vec<(Pos2, f32)> = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Circle(circle) => Some((circle.center, circle.radius)),
                _ => None,
            })
            .collect();
        assert_eq!(circles.len(), 3, "three dots: {shapes:?}");
        assert_eq!(shapes.len(), 3, "and nothing else: {shapes:?}");
        circles.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
        for (centre, radius) in &circles {
            assert!(
                (centre.x - c.x).abs() < 1e-3,
                "over one another: {circles:?}"
            );
            assert!((radius - 1.4).abs() < 1e-3, "a dot is small: {circles:?}");
        }
        assert!(
            (circles[1].0.y - c.y).abs() < 1e-3,
            "the middle one is in the middle"
        );
        assert!((circles[1].0.y - circles[0].0.y - 4.0).abs() < 1e-3);
        assert!((circles[2].0.y - circles[1].0.y - 4.0).abs() < 1e-3);
    }

    #[test]
    fn a_button_is_24_wide_and_18_high_and_has_its_picture_in_the_middle() {
        let ctx = egui::Context::default();
        let mut seen = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut centre = None;
            let response = button(ui, "probe", |_, c, _| centre = Some(c));
            seen = Some((response.rect, centre));
        });
        output.textures_delta.clear();
        let (rect, centre) = seen.expect("the button was drawn");
        assert_eq!(rect.size(), egui::vec2(24.0, 18.0));
        assert_eq!(centre, Some(rect.center()));
    }

    /// The rectangles that were painted: where, with what fill, and whether with a line
    /// round them.
    fn rects(shapes: &[Shape]) -> Vec<(Rect, Color32, Option<Stroke>)> {
        shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Rect(r) => Some((
                    r.rect,
                    r.fill,
                    (r.stroke.width > 0.0 && r.stroke.color.a() > 0).then_some(r.stroke),
                )),
                _ => None,
            })
            .collect()
    }

    /// The pixels of a picture of `width` by `height` points that the filled rectangles
    /// cover, a string for a row: `#` for covered, `.` for not.
    fn art(shapes: &[Shape], origin: Pos2, width: usize, height: usize) -> Vec<String> {
        let filled: Vec<Rect> = rects(shapes)
            .into_iter()
            .filter(|r| r.1.a() > 0)
            .map(|r| r.0)
            .collect();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        let at = pos2(origin.x + x as f32 + 0.5, origin.y + y as f32 + 0.5);
                        if filled.iter().any(|r| r.contains(at)) {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_side_panel_is_a_window_with_its_left_column_filled() {
        let shapes = painted(|p| side_panel(p, pos2(47.0, 66.0), Color32::WHITE));
        let found = rects(&shapes);
        assert_eq!(shapes.len(), 2, "a line round it and a column: {shapes:?}");
        let window = found
            .iter()
            .find(|r| r.2.is_some())
            .expect("the line round the window");
        let column = found
            .iter()
            .find(|r| r.2.is_none())
            .expect("the left column");
        assert_eq!(
            window.0,
            Rect::from_min_max(pos2(40.0, 60.0), pos2(54.0, 72.0)),
            "14 by 12 points"
        );
        assert_eq!(
            column.0,
            Rect::from_min_max(pos2(40.0, 60.0), pos2(45.0, 72.0)),
            "five wide, as high as the window, at its left"
        );
        assert_eq!(
            column.1,
            Color32::WHITE,
            "the column is in the colour the button's text has"
        );
        assert_eq!(window.1.a(), 0, "the window is empty");
        // The line is inside the 14 by 12 points, not round them.
        let kinds: Vec<StrokeKind> = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Rect(r) if r.stroke.width > 0.0 => Some(r.stroke_kind),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, [StrokeKind::Inside]);
        assert_eq!(
            window.2.map(|s| (s.width, s.color)),
            Some((1.0, Color32::WHITE))
        );
    }

    #[test]
    fn the_new_folder_is_a_folder_with_a_plus_left_out_of_its_body() {
        let shapes = painted(|p| new_folder(p, pos2(47.5, 65.5), Color32::WHITE));
        assert_eq!(
            art(&shapes, pos2(40.0, 60.0), 15, 11),
            [
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
            ]
        );
        // Nothing but filled rectangles, all in the colour of the text, and none beyond
        // the picture: the plus is a gap and not another colour.
        let found = rects(&shapes);
        assert_eq!(found.len(), shapes.len(), "{shapes:?}");
        assert!(found.iter().all(|r| r.1 == Color32::WHITE && r.2.is_none()));
        let whole = found.iter().fold(Rect::NOTHING, |all, r| all.union(r.0));
        assert_eq!(
            whole,
            Rect::from_min_max(pos2(40.0, 60.0), pos2(55.0, 71.0)),
            "15 by 11 points"
        );
    }

    #[test]
    fn the_plus_of_the_new_folder_is_in_the_middle_of_its_body() {
        let shapes = painted(|p| new_folder(p, pos2(47.5, 65.5), Color32::WHITE));
        let rows = art(&shapes, pos2(40.0, 60.0), 15, 11);
        // The body is the rows from the third; its middle is the seventh, and the bar of the
        // plus is there, five wide, round the middle column.
        let bar = &rows[6];
        assert_eq!(bar.find('.'), Some(5), "{bar}");
        assert_eq!(bar.rfind('.'), Some(9), "{bar}");
        // (Above the body, the middle column is empty beside the tab too: the body's rows only.)
        let upright: Vec<usize> = (2..11).filter(|&y| rows[y].as_bytes()[7] == b'.').collect();
        assert_eq!(upright, [4, 5, 6, 7, 8], "five high, round the middle row");
        // As far from the top of the body (the third row) as from its foot (the eleventh).
        assert_eq!(upright[0] - 2, 10 - upright[4], "{rows:?}");
    }

    #[test]
    fn the_two_pictures_lie_on_whole_pixels_at_any_scale_so_that_their_edges_are_sharp() {
        type Picture = fn(&Painter, Pos2, Color32);
        let pictures: [(&str, Picture); 2] =
            [("side panel", side_panel), ("new folder", new_folder)];
        for scale in [1.0_f32, 1.25, 1.5, 2.0, 3.0] {
            for (name, draw) in pictures {
                let shapes = painted_at(scale, |p| draw(p, pos2(47.3, 65.7), Color32::WHITE));
                for (rect, _, _) in rects(&shapes) {
                    for edge in [rect.left(), rect.top(), rect.right(), rect.bottom()] {
                        let pixels = edge * scale;
                        assert!(
                            (pixels - pixels.round()).abs() < 0.01,
                            "{name}, a scale of {scale}: {rect:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_line_round_the_window_is_a_whole_number_of_pixels_wide_and_at_least_one() {
        for (scale, pixels) in [
            // (A window zoomed far out: a line that rounds to no pixel is one pixel.)
            (0.4_f32, 1.0),
            (1.0, 1.0),
            (1.25, 1.0),
            (1.5, 2.0),
            (2.0, 2.0),
            (3.0, 3.0),
        ] {
            let shapes = painted_at(scale, |p| side_panel(p, pos2(47.0, 66.0), Color32::WHITE));
            let line = rects(&shapes)
                .into_iter()
                .find_map(|r| r.2)
                .expect("a line round the window");
            assert!(
                (line.width * scale - pixels).abs() < 0.01,
                "a scale of {scale}: {} points",
                line.width
            );
        }
    }
}
