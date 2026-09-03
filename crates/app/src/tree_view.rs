//! The virtualized tree widget (Architecture §6). One `TreeView` instance
//! backs the source panel, another backs the results panel.
//!
//! Expand/collapse state is kept out of the `Value` itself, in a side map
//! keyed by node path. The flattened visible-row list is cached and only
//! recomputed when the backing value or the expand state actually changes —
//! not every frame — and `egui::ScrollArea::show_rows` then draws only the
//! rows inside the current viewport, so per-frame cost is bounded by
//! viewport height, not by document size.

use eframe::egui;
use jsonquery_core::{
    flatten_visible, new_expanded_at_root, ExpandState, NodePath, PathSegment, RowInfo, ValueKind,
};
use serde_json::Value;

pub struct TreeView {
    expand: ExpandState,
    rows: Vec<RowInfo>,
    dirty: bool,
}

impl Default for TreeView {
    fn default() -> Self {
        Self {
            expand: new_expanded_at_root(),
            rows: Vec::new(),
            dirty: true,
        }
    }
}

impl TreeView {
    /// Call whenever the value backing this tree has been replaced wholesale
    /// (a new document loaded, or a fresh query run started) — clears expand
    /// state back to "just the root open".
    pub fn reset(&mut self) {
        self.expand = new_expanded_at_root();
        self.dirty = true;
    }

    /// Call when the backing value changed in place (more result items
    /// streamed in) without wanting to disturb existing expand/collapse
    /// state.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn refresh(&mut self, root: &Value) {
        if self.dirty {
            self.rows = flatten_visible(root, &self.expand);
            self.dirty = false;
        }
    }

    pub fn row_count(&mut self, root: &Value) -> usize {
        self.refresh(root);
        self.rows.len()
    }

    /// Draw the tree. `salt` must be unique per instance on screen (egui
    /// needs it to keep this scroll area's state distinct from the other
    /// tree).
    pub fn ui(&mut self, ui: &mut egui::Ui, salt: &str, root: &Value) {
        self.refresh(root);

        let row_height = ui.text_style_height(&egui::TextStyle::Monospace).max(18.0);
        let total_rows = self.rows.len();

        if total_rows == 0 {
            ui.weak("(empty)");
            return;
        }

        let mut toggled: Option<NodePath> = None;

        egui::ScrollArea::both()
            .id_salt(salt)
            .auto_shrink([false, false])
            .show_rows(ui, row_height, total_rows, |ui, range| {
                for i in range {
                    let Some(row) = self.rows.get(i) else {
                        continue;
                    };
                    if draw_row(ui, row) {
                        toggled = Some(row.path.clone());
                    }
                }
            });

        if let Some(path) = toggled {
            if !self.expand.remove(&path) {
                self.expand.insert(path);
            }
            self.dirty = true;
        }
    }
}

/// Draw one row; returns `true` if its expand/collapse arrow was clicked.
fn draw_row(ui: &mut egui::Ui, row: &RowInfo) -> bool {
    let mut toggle_clicked = false;
    ui.horizontal(|ui| {
        ui.add_space(row.depth as f32 * 16.0);

        if row.kind.is_container() {
            let arrow = if row.expanded { "\u{25be}" } else { "\u{25b8}" };
            let resp = ui.add(
                egui::Label::new(egui::RichText::new(arrow).monospace())
                    .sense(egui::Sense::click()),
            );
            if resp.clicked() {
                toggle_clicked = true;
            }
        } else {
            ui.add_space(14.0);
        }

        if let Some(key) = &row.key {
            let key_text = match key {
                PathSegment::Key(k) => format!("{k:?}: "),
                PathSegment::Index(i) => format!("{i}: "),
            };
            ui.label(egui::RichText::new(key_text).monospace().weak());
        }

        let (value_text, color) = match row.kind {
            ValueKind::Object => (
                format!("{{\u{2026}}} ({} keys)", row.child_count),
                ui.visuals().weak_text_color(),
            ),
            ValueKind::Array => (
                format!("[\u{2026}] ({} items)", row.child_count),
                ui.visuals().weak_text_color(),
            ),
            ValueKind::String => (
                row.scalar_preview.clone().unwrap_or_default(),
                string_color(ui),
            ),
            ValueKind::Number => (
                row.scalar_preview.clone().unwrap_or_default(),
                number_color(ui),
            ),
            ValueKind::Bool => (
                row.scalar_preview.clone().unwrap_or_default(),
                bool_color(ui),
            ),
            ValueKind::Null => (
                row.scalar_preview.clone().unwrap_or_default(),
                ui.visuals().weak_text_color(),
            ),
        };
        let label = ui.label(egui::RichText::new(value_text).monospace().color(color));
        if row.kind.is_container() && label.double_clicked() {
            toggle_clicked = true;
        }
    });
    toggle_clicked
}

fn string_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(152, 195, 121)
    } else {
        egui::Color32::from_rgb(80, 130, 60)
    }
}

fn number_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(209, 154, 102)
    } else {
        egui::Color32::from_rgb(170, 90, 20)
    }
}

fn bool_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(97, 175, 239)
    } else {
        egui::Color32::from_rgb(30, 90, 180)
    }
}
