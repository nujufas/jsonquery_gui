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
    flatten_visible, new_expanded_at_root, path_string, ExpandState, NodePath, PathSegment,
    RowInfo, ValueKind,
};
use serde_json::Value;

/// What a row's right-click menu asked the owning `App` to do — resolving
/// the path against the right root, and actually performing the action, both
/// need context `TreeView` doesn't have.
pub enum RowAction {
    /// "Save…" was chosen from a row's context menu.
    Save(NodePath),
    /// "Find in Source" was chosen from a results row's context menu — look
    /// for this value somewhere in the loaded source document.
    FindInSource(NodePath),
    /// "Search…" was chosen from a row's context menu — open a find dialog
    /// that searches the whole tree, not just this row.
    OpenSearch,
}

pub struct TreeView {
    expand: ExpandState,
    rows: Vec<RowInfo>,
    dirty: bool,
    /// Set by `reveal()`; consumed the next time `ui()` runs, scrolling that
    /// row into view.
    pending_scroll: Option<NodePath>,
    /// The row `reveal()` last pointed at, drawn with a highlighted
    /// background so it's easy to spot after scrolling to it.
    highlight: Option<NodePath>,
}

impl Default for TreeView {
    fn default() -> Self {
        Self {
            expand: new_expanded_at_root(),
            rows: Vec::new(),
            dirty: true,
            pending_scroll: None,
            highlight: None,
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
        self.pending_scroll = None;
        self.highlight = None;
    }

    /// Call when the backing value changed in place (more result items
    /// streamed in) without wanting to disturb existing expand/collapse
    /// state.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Expand every ancestor of `path` (so it's visible even if currently
    /// collapsed), and arrange for the next `ui()` call to scroll to it and
    /// highlight it — used by the results panel's "reveal in source" click.
    pub fn reveal(&mut self, path: NodePath) {
        for i in 0..=path.len() {
            self.expand.insert(path[..i].to_vec());
        }
        self.dirty = true;
        self.pending_scroll = Some(path.clone());
        self.highlight = Some(path);
    }

    fn refresh(&mut self, root: &Value) {
        if self.dirty {
            self.rows = flatten_visible(root, &self.expand);
            self.dirty = false;
        }
    }

    /// Draw the tree. `salt` must be unique per instance on screen (egui
    /// needs it to keep this scroll area's state distinct from the other
    /// tree). `find_in_source` adds a "Find in Source" item to every row's
    /// context menu — pass `true` for the results tree, `false` for the
    /// source tree itself. Returns the row action (if any) chosen this frame.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        salt: &str,
        root: &Value,
        find_in_source: bool,
    ) -> Option<RowAction> {
        self.refresh(root);

        let row_height = ui.text_style_height(&egui::TextStyle::Monospace).max(18.0);
        let total_rows = self.rows.len();

        if total_rows == 0 {
            ui.weak("(empty)");
            return None;
        }

        let mut scroll_area = egui::ScrollArea::both()
            .id_salt(salt)
            .auto_shrink([false, false]);
        if let Some(target) = self.pending_scroll.take() {
            if let Some(idx) = self.rows.iter().position(|r| r.path == target) {
                let row_stride = row_height + ui.spacing().item_spacing.y;
                let target_y = idx as f32 * row_stride;
                let viewport_h = ui.available_height();
                let offset_y = (target_y - viewport_h / 2.0).max(0.0);
                scroll_area = scroll_area.vertical_scroll_offset(offset_y);
            }
        }

        // Pass 1 (inside the scroll area): draw each visible row and record
        // its on-screen rect. We *don't* sense clicks here — see pass 2.
        let mut visible: Vec<(usize, egui::Rect)> = Vec::new();
        scroll_area.show_rows(ui, row_height, total_rows, |ui, range| {
            for i in range {
                let Some(row) = self.rows.get(i) else {
                    continue;
                };
                let row_rect = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width(), row_height),
                );
                let highlighted = self.highlight.as_ref() == Some(&row.path);
                draw_row_visual(ui, row, row_rect, highlighted);
                visible.push((i, row_rect));
            }
        });

        // Pass 2 (after the scroll area returns): sense clicks on those same
        // rects. This has to happen *after*, not during, pass 1 — egui's
        // `ScrollArea` registers its own click-and-drag "pan" sense over the
        // whole content area once `show_rows` finishes, and in this egui
        // version, when that background is registered *later* than an
        // overlapping click-only widget, the hit-test silently drops the
        // click rather than awarding it to either one (see
        // `hit_test::hit_test_on_close`'s `(Some, Some)` arm — a drag-only
        // widget "on top" suppresses a click-only widget underneath it).
        // Registering our own click sense here, afterward, puts our rows on
        // top instead, so they win.
        let mut toggled: Option<NodePath> = None;
        let mut action: Option<RowAction> = None;
        for (i, row_rect) in visible {
            let Some(row) = self.rows.get(i) else {
                continue;
            };
            if let Some(row_action) = sense_row(ui, row, row_rect, find_in_source, &mut toggled) {
                action = Some(row_action);
            }
        }

        if let Some(path) = toggled {
            if !self.expand.remove(&path) {
                self.expand.insert(path);
            }
            self.dirty = true;
        }

        action
    }
}

/// Approximate width of the depth-indent + expand arrow column, used to tell
/// "clicked the arrow" (toggle) from "clicked the row" (context-dependent —
/// e.g. "reveal in source" for the results tree) apart. Matches the
/// `add_space(14.0)` reserved for non-container rows below.
const ARROW_COLUMN_WIDTH: f32 = 14.0;

/// Draw one row's visual content (arrow, key, value) at `row_rect`, plus its
/// highlight background if any. Purely visual — no interaction (see
/// `sense_row`, called separately after the scroll area).
fn draw_row_visual(ui: &mut egui::Ui, row: &RowInfo, row_rect: egui::Rect, highlighted: bool) {
    if highlighted {
        ui.painter().rect_filled(
            row_rect,
            2.0,
            ui.visuals().selection.bg_fill.linear_multiply(0.5),
        );
    }

    ui.horizontal(|ui| {
        ui.add_space(row.depth as f32 * 16.0);

        if row.kind.is_container() {
            let arrow = if row.expanded { "\u{25be}" } else { "\u{25b8}" };
            ui.label(egui::RichText::new(arrow).monospace());
        } else {
            ui.add_space(ARROW_COLUMN_WIDTH);
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
        ui.label(egui::RichText::new(value_text).monospace().color(color));
    });
}

/// Register one row's click/right-click sense at its already-drawn
/// `row_rect`, and interpret the result: sets `*toggled` if the row's
/// expand/collapse state should flip, and returns a `RowAction` chosen from
/// the row's context menu. A plain click elsewhere on the row (not the
/// expand arrow) does nothing — `find_in_source` is how a results row's
/// value gets located in the source tree.
fn sense_row(
    ui: &mut egui::Ui,
    row: &RowInfo,
    row_rect: egui::Rect,
    find_in_source: bool,
    toggled: &mut Option<NodePath>,
) -> Option<RowAction> {
    let arrow_left = row_rect.left() + row.depth as f32 * 16.0;
    let arrow_right = arrow_left + ARROW_COLUMN_WIDTH;

    let row_id = ui.id().with("row").with(&row.path);
    let row_resp = ui.interact(row_rect, row_id, egui::Sense::click());

    let mut action = None;

    if row.kind.is_container() && row_resp.double_clicked() {
        *toggled = Some(row.path.clone());
    } else if row_resp.clicked() {
        let on_arrow = row.kind.is_container()
            && row_resp
                .interact_pointer_pos()
                .is_some_and(|p| (arrow_left..arrow_right).contains(&p.x));
        if on_arrow {
            *toggled = Some(row.path.clone());
        }
    }

    row_resp.context_menu(|ui| {
        if ui.button("Save…").clicked() {
            action = Some(RowAction::Save(row.path.clone()));
            ui.close();
        }
        if ui.button("Copy JSON Path").clicked() {
            ui.ctx().copy_text(path_string(&row.path));
            ui.close();
        }
        if find_in_source && ui.button("Find in Source").clicked() {
            action = Some(RowAction::FindInSource(row.path.clone()));
            ui.close();
        }
        ui.separator();
        if ui.button("Search…").clicked() {
            action = Some(RowAction::OpenSearch);
            ui.close();
        }
    });

    action
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
