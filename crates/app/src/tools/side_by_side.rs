//! Diff's side-by-side view: the two documents next to each other, line by line,
//! as a file-comparison tool shows two files. What both have is on one line on
//! both sides; what only one has is red (removed) or green (added) with a blank
//! on the other side; what changed is amber on both, with the part of the line
//! that differs picked out. The rows come from `jsonquery_query::diff::compare`
//! already lined up and marked (see `diff/view.rs` there), so this only draws
//! them.
//!
//! Both sides are in one scroll area, a row of the view being a row of both
//! documents, so they can't drift apart; the text scrolls sideways together, with
//! a bar of its own under the rows (or Shift+wheel). Beside the scroll bar is an
//! overview of the whole document with a tick for each difference, that can be
//! clicked or dragged. Previous and Next step from one difference to the next,
//! the menu of a difference's lines moves it to the left or the right, and
//! "Differences only" folds what is the same into a line that says how many
//! lines it hides.

use std::ops::Range;

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, TextStyle,
};
use jsonquery_query::diff::{Block, Mark, Row, Side, SideBySide};

use super::widgets::{tint, Tint};
use crate::pane_header;

/// Height of a row: a monospace line and a hair.
const ROW: f32 = 17.0;
/// Space between the two documents.
const GUTTER: f32 = 14.0;
/// Width of the overview, and the room between it and the rows.
const OVERVIEW: f32 = 10.0;
const OVERVIEW_GAP: f32 = 4.0;
/// Space between the edge of a side and its text.
const TEXT_PAD: f32 = 6.0;
/// Rows of what is the same kept around a difference by "Differences only".
const CONTEXT: usize = 3;
/// Rows kept above a difference that is scrolled to.
const LEAD: f32 = 2.0;

/// What a line of the list on screen is: a row of the view, or a fold that
/// stands for rows that are the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Row(u32),
    Fold { first: u32, count: u32 },
}

impl Item {
    /// The first row of the view this is.
    fn first(self) -> u32 {
        match self {
            Item::Row(row) | Item::Fold { first: row, .. } => row,
        }
    }
}

/// What is on screen, in order: every row, or with "Differences only" the rows
/// near a difference, the rest folded. A view with no difference is shown whole.
fn items(view: &SideBySide, differences_only: bool) -> Vec<Item> {
    let len = view.rows.len();
    if !differences_only || view.blocks.is_empty() {
        return (0..len as u32).map(Item::Row).collect();
    }
    let mut near = vec![false; len];
    for block in &view.blocks {
        let from = block.start.saturating_sub(CONTEXT);
        let to = (block.end + CONTEXT).min(len);
        near[from..to].fill(true);
    }
    let mut items = Vec::new();
    let mut row = 0;
    while row < len {
        if near[row] {
            items.push(Item::Row(row as u32));
            row += 1;
        } else {
            let first = row;
            while row < len && !near[row] {
                row += 1;
            }
            items.push(Item::Fold {
                first: first as u32,
                count: (row - first) as u32,
            });
        }
    }
    items
}

/// Where in the list on screen `row` of the view is (a folded row is at its
/// fold).
fn position_of(items: &[Item], row: usize) -> usize {
    items
        .partition_point(|item| item.first() as usize <= row)
        .saturating_sub(1)
}

/// The block that has `row` in it.
fn block_at(blocks: &[Block], row: usize) -> Option<usize> {
    let at = blocks.partition_point(|b| b.end <= row);
    blocks.get(at).filter(|b| b.start <= row).map(|_| at)
}

/// Which characters of two lines differ: what is left when the start and the end
/// they have in common are taken off each.
fn differing(a: &str, b: &str) -> (Range<usize>, Range<usize>) {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    (prefix..a.len() - suffix, prefix..b.len() - suffix)
}

/// The two halves of a row `width` wide, with the gutter between them.
fn halves(rect: Rect) -> (Rect, Rect) {
    let half = ((rect.width() - GUTTER) / 2.0).floor().max(0.0);
    (
        Rect::from_min_size(rect.min, egui::vec2(half, rect.height())),
        Rect::from_min_size(
            Pos2::new(rect.max.x - half, rect.min.y),
            egui::vec2(half, rect.height()),
        ),
    )
}

/// What the view remembers between frames. It is made again (with
/// [`Viewer::reset`]) for each new comparison.
#[derive(Default)]
pub(super) struct Viewer {
    differences_only: bool,
    /// How far the text is scrolled to the left, in points, on both sides.
    h_off: f32,
    /// The difference Previous and Next are on.
    current: Option<usize>,
    /// The first item on screen, as of the last frame.
    top: usize,
    /// An item to scroll to at the next frame.
    jump: Option<usize>,
    /// What is on screen, and whether it was made for "Differences only".
    items: Vec<Item>,
    items_for: Option<bool>,
}

/// What [`Viewer::show`] has to tell the page.
pub(super) struct Shown {
    /// The path of the difference that was clicked, to copy.
    pub clicked: Option<String>,
    /// A difference (its place in the list) that was asked, from the menu of
    /// its lines, to be moved to the left or to the right.
    pub moved: Option<(usize, Side)>,
}

impl Viewer {
    /// A new comparison: forget where the old one was scrolled to.
    pub fn reset(&mut self) {
        let differences_only = self.differences_only;
        *self = Self {
            differences_only,
            ..Self::default()
        };
    }

    fn ensure_items(&mut self, view: &SideBySide) {
        if self.items_for == Some(self.differences_only) && !self.items.is_empty() {
            return;
        }
        // Switching "Differences only" on or off: stay at what is at the top.
        let top = self
            .items
            .get(self.top)
            .filter(|_| self.items_for.is_some())
            .map(|item| item.first() as usize);
        self.items = items(view, self.differences_only);
        self.items_for = Some(self.differences_only);
        if let Some(row) = top {
            self.jump = Some(position_of(&self.items, row) + LEAD as usize);
        }
    }

    /// There is a difference after (or before) the one the view is on.
    pub fn can_step(&self, view: &SideBySide, forward: bool) -> bool {
        self.target(view, forward).is_some()
    }

    /// The difference Previous or Next goes to: the one beside the current one,
    /// or, when none is, the first below (or above) the top of the screen.
    fn target(&self, view: &SideBySide, forward: bool) -> Option<usize> {
        let top = self
            .items
            .get(self.top)
            .map_or(0, |item| item.first() as usize);
        match (self.current, forward) {
            (Some(at), true) => (at + 1 < view.blocks.len()).then_some(at + 1),
            (Some(at), false) => at.checked_sub(1),
            (None, true) => view.blocks.iter().position(|b| b.start >= top),
            (None, false) => view.blocks.iter().rposition(|b| b.start < top),
        }
    }

    /// Go to the next (or the previous) difference.
    pub fn step(&mut self, view: &SideBySide, forward: bool) {
        self.ensure_items(view);
        if let Some(at) = self.target(view, forward) {
            self.go_to(view, at);
        }
    }

    fn go_to(&mut self, view: &SideBySide, block: usize) {
        self.current = Some(block);
        self.jump = Some(position_of(&self.items, view.blocks[block].start));
    }

    /// The difference Previous and Next are on, which a move acts on.
    pub fn current(&self) -> Option<usize> {
        self.current
    }

    /// Go to difference `block`, as Next does.
    pub fn pick(&mut self, view: &SideBySide, block: usize) {
        self.ensure_items(view);
        if block < view.blocks.len() {
            self.go_to(view, block);
        }
    }

    /// "Difference 2 of 5", when one is picked.
    pub fn position(&self, view: &SideBySide) -> Option<String> {
        self.current
            .map(|at| format!("Difference {} of {}", at + 1, view.blocks.len()))
    }

    /// Draw the view in the room that is left. `names` are what the two
    /// documents are called (a file, the open document), if they are.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        view: &SideBySide,
        names: [Option<&str>; 2],
    ) -> Shown {
        self.ensure_items(view);
        let items = std::mem::take(&mut self.items);

        let full = ui.available_rect_before_wrap();
        let header_h = ui.spacing().interact_size.y;
        let solid = egui::style::ScrollStyle::solid();
        let bar = solid.allocated_width();
        let strip_h = bar + 1.0;

        let top = full.min.y + header_h + 2.0;
        let rows_right = full.max.x - OVERVIEW - OVERVIEW_GAP;
        let rows_rect = Rect::from_min_max(
            Pos2::new(full.min.x, top),
            Pos2::new(rows_right, (full.max.y - strip_h).max(top)),
        );
        let strip_rect = Rect::from_min_max(
            Pos2::new(full.min.x, rows_rect.max.y),
            Pos2::new(rows_right, rows_rect.max.y + strip_h),
        );
        let overview_rect = Rect::from_min_max(
            Pos2::new(full.max.x - OVERVIEW, top),
            Pos2::new(full.max.x, rows_rect.max.y),
        );
        // What the rows are laid out in: the scroll area keeps a bar's width at
        // the right (always, so that the header above lines up).
        let inner = Rect::from_min_max(
            rows_rect.min,
            Pos2::new(
                (rows_rect.max.x - bar).max(rows_rect.min.x),
                rows_rect.max.y,
            ),
        );
        let (left_half, right_half) = halves(inner);

        let font = TextStyle::Monospace.resolve(ui.style());
        let glyph = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        let text_width = view.width as f32 * glyph + 2.0 * TEXT_PAD;
        let max_off = (text_width - left_half.width()).max(0.0);

        // Sideways: the wheel over the rows, or the bar under them.
        if ui.rect_contains_pointer(rows_rect) {
            self.h_off -= ui.input(|i| i.smooth_scroll_delta.x);
        }
        self.h_off = self.h_off.clamp(0.0, max_off);
        let mut strip = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("diff_sbs_strip")
                .max_rect(strip_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        strip.spacing_mut().scroll = solid;
        let sideways = egui::ScrollArea::horizontal()
            .id_salt("diff_sbs_h")
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .horizontal_scroll_offset(self.h_off)
            .show(&mut strip, |ui| {
                ui.allocate_space(egui::vec2(strip_rect.width() + max_off, 1.0));
            });
        self.h_off = sideways.state.offset.x.clamp(0.0, max_off);

        let paint = Paint {
            font,
            h_off: self.h_off,
            text: ui.visuals().text_color(),
            good: tint(ui.visuals(), Tint::Good),
            bad: tint(ui.visuals(), Tint::Bad),
            warn: tint(ui.visuals(), Tint::Warn),
            filler: ui.visuals().text_color().gamma_multiply(0.06),
            select: ui.visuals().selection.bg_fill,
            weak: ui.visuals().weak_text_color(),
        };

        // The rows.
        let mut rows_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("diff_sbs_rows")
                .max_rect(rows_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        rows_ui.set_clip_rect(rows_rect.intersect(ui.clip_rect()));
        rows_ui.spacing_mut().scroll = solid;
        // Rows touch.
        rows_ui.spacing_mut().item_spacing.y = 0.0;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("diff_sbs")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
        if let Some(item) = self.jump.take() {
            area = area.vertical_scroll_offset((item as f32 - LEAD).max(0.0) * ROW);
        }
        let current = self.current.and_then(|at| view.blocks.get(at)).copied();
        let mut clicked_row = None;
        let mut moved = None;
        let scrolled = area.show_rows(&mut rows_ui, ROW, items.len(), |ui, range| {
            for item in &items[range] {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(inner.width(), ROW), Sense::click());
                match *item {
                    Item::Fold { count, .. } => paint_fold(ui, rect, count, &paint),
                    Item::Row(index) => {
                        let row = &view.rows[index as usize];
                        let in_current =
                            current.is_some_and(|b| (b.start..b.end).contains(&(index as usize)));
                        paint_row(ui, rect, row, in_current, &paint);
                        if row.mark != Mark::Same {
                            let response = response
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_ui(|ui| {
                                    let path = row.path.as_deref().unwrap_or_default();
                                    ui.label(if path.is_empty() {
                                        "(whole document)"
                                    } else {
                                        path
                                    });
                                    ui.weak("Click to copy the path");
                                    ui.weak("Right-click to move it to the other side");
                                });
                            let block = block_at(&view.blocks, index as usize);
                            response.context_menu(|ui| {
                                for (label, tip, into) in [
                                    (
                                        "Move to the left",
                                        "Left takes what Right has here",
                                        Side::Left,
                                    ),
                                    (
                                        "Move to the right",
                                        "Right takes what Left has here",
                                        Side::Right,
                                    ),
                                ] {
                                    if ui.button(label).on_hover_text(tip).clicked() {
                                        moved = block.map(|block| (block, into));
                                        ui.close();
                                    }
                                }
                            });
                            if response.clicked() {
                                clicked_row = Some(index as usize);
                            }
                        }
                    }
                }
            }
        });
        let first_shown = (scrolled.state.offset.y / ROW).floor().max(0.0) as usize;
        self.top = first_shown.min(items.len().saturating_sub(1));
        let visible = rows_rect.height() / ROW;

        // The overview: a tick for each difference, and what is on screen.
        let overview = ui.interact(
            overview_rect,
            ui.id().with("diff_overview"),
            Sense::click_and_drag(),
        );
        paint_overview(
            ui,
            overview_rect,
            view,
            &items,
            (first_shown as f32, visible),
            current,
            &paint,
        );
        if let Some(pos) = overview.interact_pointer_pos() {
            if overview.is_pointer_button_down_on() || overview.clicked() {
                let at = (pos.y - overview_rect.min.y) / overview_rect.height().max(1.0);
                let item = at * items.len() as f32 - visible / 2.0 + LEAD;
                self.jump = Some(item.max(0.0) as usize);
                ui.ctx().request_repaint();
            }
        }

        self.header(
            ui,
            (left_half, right_half),
            view,
            names,
            header_h,
            full.min.y,
        );

        let mut clicked = None;
        if let Some(index) = clicked_row {
            clicked = view.rows[index].path.as_deref().map(str::to_owned);
            self.current = block_at(&view.blocks, index);
        }
        if let Some((block, _)) = moved {
            self.current = Some(block);
        }
        self.items = items;
        ui.allocate_rect(full, Sense::hover());
        Shown { clicked, moved }
    }

    /// What is over each side: which document it is, what it is called, and, at
    /// the right, "Differences only".
    fn header(
        &mut self,
        ui: &mut egui::Ui,
        (left, right): (Rect, Rect),
        view: &SideBySide,
        names: [Option<&str>; 2],
        height: f32,
        y: f32,
    ) {
        let side = |ui: &mut egui::Ui, id: &str, rect: Rect| {
            let rect =
                Rect::from_min_size(Pos2::new(rect.min.x, y), egui::vec2(rect.width(), height));
            ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(id)
                    .max_rect(rect)
                    .layout(Layout::left_to_right(Align::Center)),
            )
        };
        let describe = |name: Option<&str>, lines: usize| match name {
            Some(name) => format!("{name} · {lines} lines"),
            None => format!("{lines} lines"),
        };

        let mut first = side(ui, "diff_sbs_left", left);
        pane_header::title(&mut first, "Left");
        first.add(
            egui::Label::new(RichText::new(describe(names[0], view.left_lines)).weak()).truncate(),
        );

        let mut second = side(ui, "diff_sbs_right", right);
        pane_header::title(&mut second, "Right");
        let text = describe(names[1], view.right_lines);
        pane_header::pinned_right(&mut second, |ui| {
            ui.checkbox(&mut self.differences_only, "Differences only")
                .on_hover_text("Hide what is the same, but a few lines around each difference");
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(text).weak()).truncate());
            });
        });
    }
}

/// The colours a row is drawn in.
struct Paint {
    font: FontId,
    h_off: f32,
    text: Color32,
    good: Color32,
    bad: Color32,
    warn: Color32,
    /// What the blank side of a line that only the other side has is filled
    /// with.
    filler: Color32,
    /// The gutter beside the difference Previous and Next are on.
    select: Color32,
    weak: Color32,
}

fn paint_row(ui: &egui::Ui, rect: Rect, row: &Row, in_current: bool, p: &Paint) {
    let (left, right) = halves(rect);
    let painter = ui.painter();

    let colour = match row.mark {
        Mark::Same => None,
        Mark::Changed => Some(p.warn),
        Mark::Removed => Some(p.bad),
        Mark::Added => Some(p.good),
    };
    let tinted = |side: &Option<String>| {
        side.as_ref()
            .map(|_| colour.map(|c| c.gamma_multiply(0.16)))
            .unwrap_or(Some(p.filler))
    };
    for (half, fill) in [(left, tinted(&row.left)), (right, tinted(&row.right))] {
        if let Some(fill) = fill {
            painter.rect_filled(half, 0.0, fill);
        }
    }

    // Between the sides: a bar in the colour of what differs, and where the
    // difference that is picked is, all of it.
    let gutter = Rect::from_min_max(
        Pos2::new(left.max.x, rect.min.y),
        Pos2::new(right.min.x, rect.max.y),
    );
    if in_current {
        painter.rect_filled(gutter, 0.0, p.select);
    } else if let Some(colour) = colour {
        let inset = ((gutter.width() - 4.0) / 2.0).max(0.0);
        painter.rect_filled(gutter.shrink2(egui::vec2(inset, 0.0)), 0.0, colour);
    }

    // What differs inside a line that changed.
    if let (Mark::Changed, Some(a), Some(b)) = (row.mark, &row.left, &row.right) {
        let (range_a, range_b) = differing(a, b);
        for (half, text, range) in [(left, a, range_a), (right, b, range_b)] {
            let (from, to) = ui.fonts_mut(|fonts| {
                let mut x = 0.0;
                let (mut from, mut to) = (None, None);
                for (at, c) in text.chars().enumerate() {
                    if at == range.start {
                        from = Some(x);
                    }
                    if at == range.end {
                        to = Some(x);
                    }
                    x += fonts.glyph_width(&p.font, c);
                }
                (from.unwrap_or(x), to.unwrap_or(x))
            });
            let x = half.min.x + TEXT_PAD - p.h_off;
            let shaded = Rect::from_min_max(
                Pos2::new(x + from, rect.min.y + 1.0),
                Pos2::new(x + to.max(from + 2.0), rect.max.y - 1.0),
            )
            .intersect(half);
            if shaded.is_positive() {
                painter.rect_filled(shaded, 2.0, p.warn.gamma_multiply(0.45));
            }
        }
    }

    for (half, text) in [(left, &row.left), (right, &row.right)] {
        if let Some(text) = text {
            painter
                .with_clip_rect(half.intersect(painter.clip_rect()))
                .text(
                    Pos2::new(half.min.x + TEXT_PAD - p.h_off, half.center().y),
                    Align2::LEFT_CENTER,
                    text,
                    p.font.clone(),
                    p.text,
                );
        }
    }
}

/// A line that stands for rows that are the same on both sides.
fn paint_fold(ui: &egui::Ui, rect: Rect, count: u32, p: &Paint) {
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, p.filler);
    let text = if count == 1 {
        "… 1 line is the same".to_owned()
    } else {
        format!("… {count} lines are the same")
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        p.font.clone(),
        p.weak,
    );
}

/// The strip beside the rows: the whole view in a few points of height, a tick
/// where each difference is, the part on screen outlined, and the difference
/// that is picked in the selection colour.
fn paint_overview(
    ui: &egui::Ui,
    rect: Rect,
    view: &SideBySide,
    items: &[Item],
    (first, visible): (f32, f32),
    current: Option<Block>,
    p: &Paint,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, p.filler);
    let total = items.len().max(1) as f32;
    let y_of = |item: f32| rect.min.y + item / total * rect.height();
    for block in &view.blocks {
        let from = position_of(items, block.start) as f32;
        let to = position_of(items, block.end.saturating_sub(1)) as f32 + 1.0;
        let colour = match block.mark {
            Mark::Added => p.good,
            Mark::Removed => p.bad,
            _ => p.warn,
        };
        let tick = Rect::from_min_max(
            Pos2::new(rect.min.x + 1.0, y_of(from)),
            Pos2::new(rect.max.x - 1.0, y_of(to).max(y_of(from) + 2.0)),
        );
        let colour = if current == Some(*block) {
            p.select
        } else {
            colour
        };
        painter.rect_filled(tick, 0.0, colour);
    }
    let window = Rect::from_min_max(
        Pos2::new(rect.min.x, y_of(first)),
        Pos2::new(rect.max.x, y_of(first + visible).min(rect.max.y)),
    );
    painter.rect_stroke(
        window,
        2.0,
        egui::Stroke::new(1.0, p.weak),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonquery_query::diff::compare;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    fn view_of(before: serde_json::Value, after: serde_json::Value) -> SideBySide {
        compare(&before, &after, &AtomicBool::new(false))
            .unwrap()
            .view
            .unwrap()
    }

    /// Twenty numbers with the third and the eighteenth changed.
    fn spread() -> SideBySide {
        let before: Vec<u32> = (0..20).collect();
        let mut after = before.clone();
        after[2] = 100;
        after[17] = 200;
        view_of(json!(before), json!(after))
    }

    #[test]
    fn everything_is_shown_unless_asked_to_fold() {
        let view = spread();
        let all = items(&view, false);
        assert_eq!(all.len(), view.rows.len());
        assert!(all.iter().all(|i| matches!(i, Item::Row(_))));
    }

    #[test]
    fn folding_keeps_a_few_rows_around_each_difference() {
        let view = spread();
        // Rows: [ at 0, number n at n + 1, ] at 21. Changed rows are 3 and 18.
        assert_eq!(
            view.blocks.iter().map(|b| b.start).collect::<Vec<_>>(),
            [3, 18]
        );
        let folded = items(&view, true);
        // Rows 0 to 6 (the first difference and three after it), then the
        // eight rows that are the same, then rows 15 to 21 (three before the
        // second).
        let mut expected: Vec<Item> = (0..7).map(Item::Row).collect();
        expected.push(Item::Fold { first: 7, count: 8 });
        expected.extend((15..22).map(Item::Row));
        assert_eq!(folded, expected);
        let hidden: u32 = folded
            .iter()
            .map(|i| {
                if let Item::Fold { count, .. } = i {
                    *count
                } else {
                    0
                }
            })
            .sum();
        let shown = folded.iter().filter(|i| matches!(i, Item::Row(_))).count();
        assert_eq!(shown + hidden as usize, view.rows.len());
    }

    #[test]
    fn a_view_with_no_difference_is_never_folded() {
        let view = view_of(json!([1, 2, 3]), json!([1, 2, 3]));
        assert_eq!(items(&view, true).len(), view.rows.len());
    }

    #[test]
    fn a_row_is_found_in_the_list_even_when_folded_away() {
        let view = spread();
        let folded = items(&view, true);
        assert_eq!(position_of(&folded, 3), 3);
        // Row 10 is inside the fold that begins at row 7.
        assert_eq!(
            folded[position_of(&folded, 10)],
            Item::Fold { first: 7, count: 8 }
        );
        assert_eq!(position_of(&items(&view, false), 10), 10);
    }

    #[test]
    fn a_row_belongs_to_the_difference_it_is_in() {
        let view = spread();
        assert_eq!(block_at(&view.blocks, 3), Some(0));
        assert_eq!(block_at(&view.blocks, 18), Some(1));
        assert_eq!(block_at(&view.blocks, 4), None);
        assert_eq!(block_at(&view.blocks, 100), None);
    }

    #[test]
    fn the_part_that_differs_is_what_is_left_after_the_shared_ends() {
        assert_eq!(differing("  \"n\": \"b\"", "  \"n\": \"B\""), (8..9, 8..9));
        assert_eq!(differing("1,", "20,"), (0..1, 0..2));
        assert_eq!(differing("same", "same"), (4..4, 4..4));
        // Only one side has more: the other has nothing in between.
        assert_eq!(differing("ab", "abc"), (2..2, 2..3));
        assert_eq!(differing("", "x"), (0..0, 0..1));
        // Characters, not bytes.
        assert_eq!(differing("é1", "é2"), (1..2, 1..2));
    }

    #[test]
    fn previous_and_next_walk_the_differences() {
        let view = spread();
        let mut viewer = Viewer::default();
        viewer.ensure_items(&view);

        // Nothing picked, at the top: Next is the first, and nothing is before.
        assert!(viewer.can_step(&view, true));
        assert!(!viewer.can_step(&view, false));
        viewer.step(&view, true);
        assert_eq!(viewer.current, Some(0));
        assert_eq!(viewer.jump, Some(3));
        assert_eq!(viewer.position(&view).as_deref(), Some("Difference 1 of 2"));

        viewer.step(&view, true);
        assert_eq!(viewer.current, Some(1));
        assert!(!viewer.can_step(&view, true));
        viewer.step(&view, true);
        assert_eq!(viewer.current, Some(1), "there is no third");

        viewer.step(&view, false);
        assert_eq!(viewer.current, Some(0));
        assert!(!viewer.can_step(&view, false));
    }

    #[test]
    fn with_none_picked_the_screen_decides_where_next_and_previous_go() {
        let view = spread();
        let mut viewer = Viewer::default();
        viewer.ensure_items(&view);
        // Scrolled to row 10, between the two.
        viewer.top = 10;
        assert_eq!(viewer.target(&view, true), Some(1));
        assert_eq!(viewer.target(&view, false), Some(0));
        // Scrolled past both.
        viewer.top = 21;
        assert_eq!(viewer.target(&view, true), None);
        assert_eq!(viewer.target(&view, false), Some(1));
    }

    #[test]
    fn folding_changes_where_a_difference_is_on_screen() {
        let view = spread();
        let mut viewer = Viewer {
            differences_only: true,
            ..Viewer::default()
        };
        viewer.step(&view, true);
        viewer.step(&view, true);
        // Row 18 is item 11 now: the fold above it took seven rows out.
        assert_eq!(viewer.jump, Some(position_of(&viewer.items, 18)));
        assert_eq!(viewer.jump, Some(11));
    }

    #[test]
    fn switching_the_fold_on_or_off_keeps_what_was_at_the_top() {
        let view = spread();
        let mut viewer = Viewer::default();
        viewer.ensure_items(&view);
        assert_eq!(viewer.jump, None, "the first time there is nothing to keep");

        // Row 16 is at the top of the full list...
        viewer.top = 16;
        viewer.differences_only = true;
        viewer.ensure_items(&view);
        // ...and is item 9 of the folded one (the fold stands for rows 7 to 14).
        // A jump goes LEAD items above where it is sent, so it is sent that far
        // below.
        assert_eq!(viewer.jump, Some(9 + LEAD as usize));
        assert_eq!(viewer.items[9], Item::Row(16));

        // Back to the full list, with the fold at the top: its first row.
        viewer.top = 7;
        viewer.jump = None;
        viewer.differences_only = false;
        viewer.ensure_items(&view);
        assert_eq!(viewer.jump, Some(7 + LEAD as usize));
    }

    #[test]
    fn a_new_comparison_forgets_the_position_but_keeps_the_option() {
        let view = spread();
        let mut viewer = Viewer {
            differences_only: true,
            h_off: 40.0,
            ..Viewer::default()
        };
        viewer.step(&view, true);
        viewer.reset();
        assert!(viewer.differences_only);
        assert_eq!(
            (viewer.current, viewer.jump, viewer.h_off),
            (None, None, 0.0)
        );
        assert!(viewer.items.is_empty());
    }
}
