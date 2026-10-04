//! The look the pages of the Tools window share: the boxes, the buttons, and
//! the rows a page is built from. A page is a heading over two halves; each
//! half has a title row on top, a box that takes all the height there is, and
//! its buttons pinned at the bottom, so it looks the same at any window size.

use std::path::Path;

use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, RichText, TextWrapMode};

use super::shared::{Notice, Run};

/// Width of the list of tools, on the left.
pub(super) const SIDEBAR_WIDTH: f32 = 168.0;
/// Space between the window's edge and the tool's page.
pub(super) const PAGE_MARGIN: i8 = 18;
/// Space on either side of the line between the two halves of a page.
pub(super) const GUTTER: i8 = 16;
/// Height of an entry in the list of tools.
pub(super) const TOOL_ROW_HEIGHT: f32 = 30.0;
/// Height of the title row over each half of a page, the same on both so the
/// boxes under them line up.
pub(super) const HEADER_HEIGHT: f32 = 26.0;
/// Height of the row of buttons under each half.
pub(super) const ACTION_HEIGHT: f32 = 28.0;
/// Space between a box and what is pinned under it.
pub(super) const BOX_GAP: i8 = 14;
pub(super) const BOX_RADIUS: u8 = 6;

/// A page's heading.
pub(super) fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).heading().strong());
    ui.add_space(14.0);
}

/// The two halves of a page, with a line between them: the panels give the
/// line, and the margins on the sides facing each other the space around it.
/// Show the left one first, then the right one.
pub(super) fn halves(ui: &egui::Ui, id: &'static str) -> (egui::Panel, egui::CentralPanel) {
    let half = (ui.available_width() / 2.0).floor();
    let left = egui::Panel::left(id)
        .resizable(false)
        .exact_size(half)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            right: GUTTER,
            ..egui::Margin::ZERO
        }));
    let right = egui::CentralPanel::default().frame(egui::Frame::NONE.inner_margin(egui::Margin {
        left: GUTTER,
        ..egui::Margin::ZERO
    }));
    (left, right)
}

/// A strip pinned to the bottom of its parent, as tall as what it holds. egui
/// measures that from the content, so the first frame uses `guess` (margin
/// included). Show it before the [`fill`] that takes the rest.
pub(super) fn pinned(id: &'static str, guess: f32) -> egui::Panel {
    egui::Panel::bottom(id)
        .resizable(false)
        .default_size(guess)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            top: BOX_GAP,
            ..egui::Margin::ZERO
        }))
}

/// What is left of the parent once the pinned strip has taken its share.
pub(super) fn fill() -> egui::CentralPanel {
    egui::CentralPanel::default().frame(egui::Frame::NONE)
}

/// The title row over a box, laid out from the left.
pub(super) fn title_row<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), HEADER_HEIGHT),
        Layout::left_to_right(Align::Center),
        add_contents,
    )
    .inner
}

/// A row of buttons, laid out from the right: the first one added is the
/// right-most.
pub(super) fn action_row<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ACTION_HEIGHT),
        Layout::right_to_left(Align::Center),
        add_contents,
    )
    .inner
}

/// The button that starts a job, at the right of its row; while one runs, a
/// Cancel button and a spinner instead. True when the job should start now:
/// the button was pressed, or Ctrl+Enter.
pub(super) fn run_row<T>(
    ui: &mut egui::Ui,
    run: &Run<T>,
    label: &str,
    ready: bool,
    tip: &str,
) -> bool {
    run_row_with(ui, run, label, ready, tip, |_| {})
}

/// [`run_row`] with something of the page's own at the left of the row.
pub(super) fn run_row_with<T>(
    ui: &mut egui::Ui,
    run: &Run<T>,
    label: &str,
    ready: bool,
    tip: &str,
    left: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut start = false;
    action_row(ui, |ui| {
        if run.running() {
            if ui.add(secondary_button("Cancel")).clicked() {
                run.cancel();
            }
            ui.label("Working…");
            ui.spinner();
        } else {
            let clicked = ui
                .add_enabled(ready, primary_button(ui, label))
                .on_hover_text(tip)
                .clicked();
            let shortcut = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
            start = ready && (clicked || shortcut);
        }
        ui.with_layout(Layout::left_to_right(Align::Center), left);
    });
    start
}

/// The line beside a result's buttons ("Saved to …"), in what is left of the
/// row after the buttons. Call it last, inside an [`action_row`].
pub(super) fn notice_line(ui: &mut egui::Ui, notice: Option<&Notice>) {
    let Some(notice) = notice else { return };
    let color = if notice.error {
        ui.visuals().error_fg_color
    } else {
        ui.visuals().hyperlink_color
    };
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add(egui::Label::new(RichText::new(&notice.text).small().color(color)).truncate())
            .on_hover_text(&notice.text);
    });
}

/// How the edge of a box is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Border {
    Solid,
    /// A place to drop things on.
    Dashed,
    /// Something is being dragged over it.
    Accent,
}

/// A rounded box that fills all the room there is, with `margin` between its
/// edge and what it holds — the look of the lists, the text boxes and the
/// results.
pub(super) fn inset_box(
    ui: &mut egui::Ui,
    border: Border,
    margin: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let visuals = ui.visuals().clone();
    let radius = egui::CornerRadius::same(BOX_RADIUS);

    let painter = ui.painter();
    match border {
        Border::Solid => {
            painter.rect(
                rect,
                radius,
                visuals.extreme_bg_color,
                visuals.widgets.noninteractive.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
        Border::Dashed => {
            painter.rect_filled(rect, radius, visuals.extreme_bg_color);
            let stroke = egui::Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.7));
            let corners = [
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
                rect.left_top(),
            ];
            painter.extend(egui::Shape::dashed_line(&corners, stroke, 6.0, 4.0));
        }
        Border::Accent => {
            painter.rect(
                rect,
                radius,
                visuals.hyperlink_color.gamma_multiply(0.12),
                egui::Stroke::new(1.5, visuals.hyperlink_color),
                egui::StrokeKind::Inside,
            );
        }
    }

    let inner_rect = rect.shrink(margin);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    inner.set_clip_rect(inner_rect.intersect(ui.clip_rect()));
    add_contents(&mut inner);
}

/// A quiet line in the middle of an empty box.
pub(super) fn centered_hint(ui: &mut egui::Ui, text: &str) {
    let rect = ui.max_rect();
    let color = ui.visuals().weak_text_color();
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(14.0),
        color,
    );
}

/// Two lines in the middle of an empty box: a plain one over a quieter one.
pub(super) fn centered_pair(ui: &mut egui::Ui, first: &str, second: &str) {
    let rect = ui.max_rect();
    let visuals = ui.visuals();
    let (strong, weak) = (visuals.text_color(), visuals.weak_text_color());
    let painter = ui.painter();
    painter.text(
        rect.center() - egui::vec2(0.0, 11.0),
        Align2::CENTER_CENTER,
        first,
        FontId::proportional(16.0),
        strong,
    );
    painter.text(
        rect.center() + egui::vec2(0.0, 13.0),
        Align2::CENTER_CENTER,
        second,
        FontId::proportional(13.0),
        weak,
    );
}

/// The box a finished job's result goes in. `show` draws an answer; the box
/// itself says what an error is, that a job was cancelled, that one is
/// running, and (`hint`) what to expect before there is one.
pub(super) fn result_box<T>(
    ui: &mut egui::Ui,
    id: &str,
    run: &Run<T>,
    hint: &str,
    show: impl FnOnce(&mut egui::Ui, &T),
) {
    let error_color = ui.visuals().error_fg_color;
    inset_box(ui, Border::Solid, 10.0, |ui| match &run.result {
        Some(Ok(outcome)) => show(ui, outcome),
        Some(Err(error)) if error == "cancelled" => centered_hint(ui, "Cancelled."),
        Some(Err(error)) => {
            egui::ScrollArea::vertical()
                .id_salt(format!("{id}_error"))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(error).color(error_color))
                            .wrap()
                            .selectable(true),
                    );
                });
        }
        None if run.running() => centered_hint(ui, "Working…"),
        None => centered_hint(ui, hint),
    });
}

/// Monospace text in a scrolling area, for a result that is a document. Long
/// lines wrap — a minified document is one — so there is only ever up and down
/// to scroll. `truncated` says only the start of it is there.
pub(super) fn preview_box(ui: &mut egui::Ui, id: &str, text: &str, truncated: bool) {
    egui::ScrollArea::vertical()
        .id_salt(format!("{id}_preview"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(text).monospace())
                    .wrap_mode(TextWrapMode::Wrap)
                    .selectable(true),
            );
            if truncated {
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Only the start is shown; Copy and Save take all of it.")
                        .small()
                        .weak(),
                );
            }
        });
}

/// One entry of the list of tools: a full-width row, filled when it is the
/// selected one.
pub(super) fn tool_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TOOL_ROW_HEIGHT),
        egui::Sense::click(),
    );

    let visuals = ui.visuals();
    let (fill, text) = if selected {
        (visuals.selection.bg_fill, visuals.selection.stroke.color)
    } else if response.hovered() {
        (visuals.widgets.hovered.weak_bg_fill, visuals.text_color())
    } else {
        (Color32::TRANSPARENT, visuals.text_color())
    };
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(BOX_RADIUS), fill);
    painter.text(
        rect.left_center() + egui::vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        text,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// How wide the column of paths in a list of rows is: as wide as the longest
/// path needs (in the monospace font the paths are set in), within limits, so
/// that what sits beside it — a message, a value — gets the rest.
pub(super) struct PathColumn {
    wanted: f32,
}

impl PathColumn {
    pub fn of<'a>(ui: &egui::Ui, paths: impl Iterator<Item = &'a str>) -> Self {
        let longest = paths.map(|p| p.chars().count()).max().unwrap_or(0);
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let glyph = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        Self {
            wanted: longest as f32 * glyph + 8.0,
        }
    }

    /// The width of the column in a row that is `total` wide: what the paths
    /// want, but no less than a fifth of the row and no more than half of it.
    pub fn width(&self, total: f32) -> f32 {
        let total = total.max(0.0);
        self.wanted.clamp(total * 0.2, total * 0.5).floor()
    }
}

/// The colours that mean good, bad and so-so.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tint {
    Good,
    Bad,
    Warn,
}

/// A green, red or amber that reads on the light theme and on the dark one.
pub(super) fn tint(visuals: &egui::Visuals, tint: Tint) -> Color32 {
    match (visuals.dark_mode, tint) {
        (true, Tint::Good) => Color32::from_rgb(87, 196, 115),
        (true, Tint::Bad) => Color32::from_rgb(244, 112, 103),
        (true, Tint::Warn) => Color32::from_rgb(229, 192, 123),
        (false, Tint::Good) => Color32::from_rgb(22, 128, 58),
        (false, Tint::Bad) => Color32::from_rgb(193, 40, 40),
        (false, Tint::Warn) => Color32::from_rgb(160, 100, 0),
    }
}

/// A button for something that is rarely needed: quiet text, no frame.
pub(super) fn flat_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let color = ui.visuals().weak_text_color();
    ui.add(egui::Button::new(RichText::new(text).color(color)).frame_when_inactive(false))
}

/// The button for what a half of the page is for, in the selection colour.
pub(super) fn primary_button(ui: &egui::Ui, text: &str) -> egui::Button<'static> {
    let visuals = ui.visuals();
    egui::Button::new(
        RichText::new(text)
            .strong()
            .color(visuals.selection.stroke.color),
    )
    .fill(visuals.selection.bg_fill)
    .min_size(egui::vec2(88.0, ACTION_HEIGHT))
}

pub(super) fn secondary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text)).min_size(egui::vec2(72.0, ACTION_HEIGHT))
}

/// The last part of a path, for a list.
pub(super) fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_column_is_as_wide_as_its_paths_within_limits() {
        let column = |wanted| PathColumn { wanted };
        // At least a fifth of the row, so a short path doesn't squeeze its
        // neighbour's label against the edge...
        assert_eq!(column(10.0).width(300.0), 60.0);
        // ...at most half of it, so a deep path doesn't take the row over...
        assert_eq!(column(500.0).width(300.0), 150.0);
        // ...and what the paths want when that is in between.
        assert_eq!(column(100.0).width(300.0), 100.0);
        // A row with no room has none to give (and does not panic).
        assert_eq!(column(100.0).width(-5.0), 0.0);
    }
}
