//! The look the pages of the Tools window share — which is the look of the
//! main window, made of the same parts and sized the same: a row of tabs on top
//! (the main window's toolbar), under it a command row with the page's main
//! button and its options (the Query row), then two panes, each with a title and
//! its buttons in a header (Source and Results; `crate::pane_header`), and a
//! status bar at the bottom. Buttons, spacing, text sizes and margins are
//! egui's own; nothing here is bigger than the main window's.

use std::path::Path;

use eframe::egui::{
    self, Align, Color32, Layout, Margin, RichText, Shape, StrokeKind, TextStyle, TextWrapMode,
};

use super::shared::Run;
use crate::pane_header;

/// What the main window paints an error in.
pub(super) const ERROR: Color32 = Color32::from_rgb(220, 80, 80);

/// Space between a pane's edge and what it holds: the main window's panes.
const PANE_MARGIN: i8 = 8;
/// The least the left pane may be dragged to: room for the header of a box (its
/// title and Open file…, Open document and Clear).
const LEFT_MIN: f32 = 340.0;
/// The least the right pane needs: room for the header of the widest of the
/// pages' results but Diff's (a title and three buttons).
pub(super) const RIGHT_MIN: f32 = 330.0;
/// Space between two boxes one above the other.
pub(super) const STACK_GAP: f32 = 6.0;
/// What a text box keeps between its edge and its text: the main window's query
/// box has the same.
const TEXT_MARGIN: Margin = Margin::symmetric(4, 2);
/// Height of a row of a list (the changes, the problems, the files).
pub(super) const ROW_HEIGHT: f32 = 20.0;

/// The row under the tabs that holds a page's main button and its options, the
/// way the main window's Query row holds Run: a panel of its own, with a line
/// under it. The page lays out a `ui.horizontal` row (or more) in it.
pub(super) fn command_bar(
    ui: &mut egui::Ui,
    id: &'static str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Panel::top(id).show(ui, add_contents);
}

/// The two panes of a page, as the main window's Source and Results are: the
/// left one can be dragged wider or narrower (within what their headers need),
/// the right one takes the rest. `right_min` is the least the right one needs.
/// Show the left one first, then the right one.
pub(super) fn halves(
    ui: &egui::Ui,
    id: &'static str,
    right_min: f32,
) -> (egui::Panel, egui::CentralPanel) {
    let width = ui.available_width();
    let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(Margin::same(PANE_MARGIN));
    let left = egui::Panel::left(id)
        .resizable(true)
        .default_size((width / 2.0).floor())
        .size_range(LEFT_MIN..=(width - right_min).max(LEFT_MIN))
        .frame(frame);
    (left, egui::CentralPanel::default())
}

/// The header of a pane, laid out as the headers of the main window's are
/// (`crate::pane_header`): the title, what goes after it, and the buttons
/// pinned at the right edge — added right to left, so the first is the
/// right-most. Nothing under it: the panel's edge above is the only line. Gives
/// the title's response, for a tooltip.
pub(super) fn header(
    ui: &mut egui::Ui,
    title: &str,
    after_title: impl FnOnce(&mut egui::Ui),
    pinned_right: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    ui.horizontal(|ui| {
        let title = pane_header::title(ui, title);
        after_title(ui);
        pane_header::pinned_right(ui, pinned_right);
        title
    })
    .inner
}

/// The button that starts a job; while one runs, a spinner and Cancel instead,
/// as in the main window's Query row. True when the job should start now: the
/// button was pressed, or Ctrl+Enter.
pub(super) fn run_button<T>(
    ui: &mut egui::Ui,
    run: &Run<T>,
    label: &str,
    ready: bool,
    tip: &str,
) -> bool {
    if run.running() {
        ui.spinner();
        if ui.button("Cancel").clicked() {
            run.cancel();
        }
        return false;
    }
    let clicked = ui
        .add_enabled(ready, egui::Button::new(label))
        .on_hover_text(tip)
        .clicked();
    let shortcut = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
    ready && (clicked || shortcut)
}

/// A multi-line monospace text box with the look of the main window's query box,
/// that fills the room it is given (or is `rows` lines tall) and scrolls when
/// its text is longer. Its frame is painted around the viewport, so it stays
/// put while the text scrolls. `accent` outlines it, to say a dragged file would
/// land here. True when the text changed.
pub(super) fn text_box(
    ui: &mut egui::Ui,
    id: &str,
    text: &mut String,
    hint: &str,
    rows: Option<usize>,
    accent: bool,
) -> bool {
    // The height of a line as the text box counts it.
    let line = ui.text_style_height(&TextStyle::Monospace) + ui.spacing().extra_text_line_spacing;
    let margin_height = TEXT_MARGIN.sum().y;
    let height = match rows {
        Some(rows) => rows as f32 * line + margin_height,
        None => ui.available_height().max(line + margin_height),
    };
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );

    // The frame is painted last, into this slot, because it depends on what the
    // editor reports (hover, focus), and it has to be behind the text.
    let frame_slot = ui.painter().add(Shape::Noop);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    inner.set_clip_rect(rect.intersect(ui.clip_rect()));

    // Whole rows, the remainder going into the bottom margin, so the editor —
    // and the area that takes a click — fills the viewport.
    let fitting = (((rect.height() - margin_height) / line + 1e-3).floor() as usize).max(1);
    let leftover = rect.height() - (fitting as f32 * line + margin_height);
    let margin = Margin {
        bottom: TEXT_MARGIN.bottom + leftover.clamp(0.0, 100.0).floor() as i8,
        ..TEXT_MARGIN
    };
    let output = egui::ScrollArea::vertical()
        .id_salt(format!("{id}_scroll"))
        .auto_shrink([false, false])
        .min_scrolled_height(line)
        .show(&mut inner, |ui| {
            egui::TextEdit::multiline(text)
                .id_salt(format!("{id}_edit"))
                .code_editor()
                .desired_rows(fitting)
                .desired_width(f32::INFINITY)
                .frame(egui::Frame::NONE.inner_margin(margin))
                .hint_text(hint)
                .show(ui)
        })
        .inner;

    let visuals = ui.style().interact(&output.response);
    let stroke = if accent {
        egui::Stroke::new(1.5, ui.visuals().hyperlink_color)
    } else if output.response.has_focus() {
        ui.visuals().selection.stroke
    } else {
        visuals.bg_stroke
    };
    ui.painter().set(
        frame_slot,
        egui::epaint::RectShape::new(
            rect.expand(visuals.expansion),
            visuals.corner_radius,
            ui.visuals().text_edit_bg_color(),
            stroke,
            StrokeKind::Inside,
        ),
    );
    output.response.changed()
}

/// What stands in a text box that holds something it can't show: a line of what
/// it is and a quieter one under it, in the same frame as a text box.
pub(super) fn note_box(ui: &mut egui::Ui, accent: bool, first: &str, second: &str) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let visuals = ui.visuals();
    let stroke = if accent {
        egui::Stroke::new(1.5, visuals.hyperlink_color)
    } else {
        visuals.widgets.inactive.bg_stroke
    };
    ui.painter().rect(
        rect,
        visuals.widgets.inactive.corner_radius,
        visuals.text_edit_bg_color(),
        stroke,
        StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(
                f32::from(TEXT_MARGIN.left),
                f32::from(TEXT_MARGIN.top),
            )))
            .layout(Layout::top_down(Align::Min)),
    );
    inner.set_clip_rect(rect.intersect(ui.clip_rect()));
    inner.label(first);
    inner.weak(second);
}

/// What a finished job's result area shows: `show` draws an answer; an error
/// is in red, that a job was cancelled and that one is running are said
/// plainly, and `hint` says what to expect before there is any of it — all as
/// the main window says such things, in plain text at the top of the pane.
pub(super) fn result_area<T>(
    ui: &mut egui::Ui,
    id: &str,
    run: &Run<T>,
    hint: &str,
    show: impl FnOnce(&mut egui::Ui, &T),
) {
    match &run.result {
        Some(Ok(outcome)) => show(ui, outcome),
        Some(Err(error)) if error == "cancelled" => {
            ui.weak("Cancelled.");
        }
        Some(Err(error)) => {
            egui::ScrollArea::vertical()
                .id_salt(format!("{id}_error"))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(error).color(ERROR))
                            .wrap()
                            .selectable(true),
                    );
                });
        }
        None if run.running() => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Working…");
            });
        }
        None => {
            ui.weak(hint);
        }
    }
}

/// Monospace text in a scrolling area, for a result that is a document, as the
/// main window's Text view shows one. Long lines wrap — a minified document is
/// one — so there is only ever up and down to scroll. `truncated` says only the
/// start of it is there.
pub(super) fn preview_box(ui: &mut egui::Ui, id: &str, text: &str, truncated: bool) {
    if truncated {
        ui.weak("Only the start is shown — Copy and Save take all of it.");
    }
    egui::ScrollArea::vertical()
        .id_salt(format!("{id}_preview"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(text).monospace())
                    .wrap_mode(TextWrapMode::Wrap)
                    .selectable(true),
            );
        });
}

/// The highlight behind a row of a list that is picked or under the pointer:
/// the tint the main window's lists use for a picked row.
pub(super) fn row_background(ui: &egui::Ui, rect: egui::Rect, picked: bool, hovered: bool) {
    let visuals = ui.visuals();
    let fill = if picked {
        visuals.selection.bg_fill.linear_multiply(0.5)
    } else if hovered {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        return;
    };
    ui.painter().rect_filled(rect, 2.0, fill);
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
        let font = TextStyle::Monospace.resolve(ui.style());
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

/// The colours the main window uses: the green of a string in the tree, and the
/// red and the amber of its status bar.
pub(super) fn tint(visuals: &egui::Visuals, tint: Tint) -> Color32 {
    match (visuals.dark_mode, tint) {
        (true, Tint::Good) => Color32::from_rgb(152, 195, 121),
        (false, Tint::Good) => Color32::from_rgb(80, 130, 60),
        (_, Tint::Bad) => ERROR,
        (_, Tint::Warn) => Color32::from_rgb(210, 150, 40),
    }
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
