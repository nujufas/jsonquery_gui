//! The header of a pane, the same in every window: the main window's Source and
//! Results panes and the boxes of the Tools window each begin with a title, what
//! goes after it (tabs, a count) and, pinned at the right edge, their buttons.
//!
//! The title is a small, strong line of text — no bigger than the buttons beside
//! it, so a header is no taller than its buttons — and nothing is drawn under the
//! header: the line above the panes (the panel's own edge) is the only divider.

use eframe::egui::{self, Align, Layout, RichText};

/// Size of a pane's title. (A heading is 18; at this size the title is no
/// taller than the buttons in its row.)
pub(crate) const TITLE_SIZE: f32 = 14.0;
/// Room between a title and what follows it.
const TITLE_GAP: f32 = 12.0;

/// A pane's title, with the gap after it. Gives the title's response, for a
/// tooltip.
pub(crate) fn title(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let title = ui.label(RichText::new(text).size(TITLE_SIZE).strong());
    ui.add_space(TITLE_GAP);
    title
}

/// The rest of a header row, laid out right to left from its right edge, so the
/// first thing added is the right-most. Takes whatever width the row has left.
pub(crate) fn pinned_right(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        Layout::right_to_left(Align::Center),
        add_contents,
    );
}
