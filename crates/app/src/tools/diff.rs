//! Diff JSON: compare two documents and say what was added, removed and
//! changed, with the paths, and as the RFC 6902 JSON Patch that turns the first
//! into the second (see `jsonquery_query::diff`). The documents can be typed or
//! pasted, files, or the document open in the main window.
//!
//! The left half is the two documents with Compare pinned under them; the right
//! half is the answer, as a list of changes or as the patch text, with Copy and
//! Save for the patch under it.

use eframe::egui::{self, Align, Color32, Layout, Rect, RichText};
use jsonquery_query::diff::{Change, ChangeKind};

use super::jobs::{Compared, Job};
use super::operand::{deliver, dropped_files, stacked, Operand};
use super::shared::{Env, Run};
use super::widgets::{
    action_row, centered_hint, fill, flat_button, halves, heading, notice_line, pinned,
    preview_box, primary_button, result_box, run_row_with, secondary_button, tint, title_row,
    PathColumn, Tint,
};
use super::{Request, Tool};

const ACTIONS_HEIGHT_GUESS: f32 = 44.0;
/// What the path of a change that is the whole document is called.
const WHOLE: &str = "(whole document)";
const RESULT_ACTIONS_HEIGHT_GUESS: f32 = 44.0;
const ROW_HEIGHT: f32 = 26.0;
/// Width of the column with "Added", "Removed" or "Changed" in it.
const TAG_WIDTH: f32 = 64.0;
/// Width of the arrow between the two values of a change.
const ARROW_WIDTH: f32 = 24.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Changes,
    Patch,
}

pub(super) struct Diff {
    before: Operand,
    after: Operand,
    view: View,
    run: Run<Compared>,
}

impl Default for Diff {
    fn default() -> Self {
        Self {
            before: Operand::new(
                "Before",
                "diff_before",
                "Paste the original JSON, or drop a file",
            ),
            after: Operand::new(
                "After",
                "diff_after",
                "Paste the changed JSON, or drop a file",
            ),
            view: View::Changes,
            run: Run::default(),
        }
    }
}

impl Diff {
    #[cfg(test)]
    pub(super) fn operand(&mut self, title: &str) -> Option<&mut Operand> {
        [&mut self.before, &mut self.after]
            .into_iter()
            .find(|o| o.title() == title)
    }

    /// The answer to a [`Request::Job`] for this page.
    pub(super) fn done(&mut self, gen: u64, result: Result<Compared, String>) {
        self.run.finish(gen, result);
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        if env.own_input
            && deliver(
                &mut [&mut self.before, &mut self.after],
                dropped_files(ui.ctx()),
            )
        {
            self.run.clear();
        }

        heading(ui, "Diff JSON");
        let mut request = None;
        let (left, right) = halves(ui, "diff_halves");
        left.show(ui, |ui| request = self.inputs_half(ui, env));
        right.show(ui, |ui| {
            if let Some(r) = self.result_half(ui, env) {
                request = Some(r);
            }
        });
        request
    }

    fn inputs_half(&mut self, ui: &mut egui::Ui, env: &Env) -> Option<Request> {
        let busy = self.run.running();
        let mut request = None;
        let mut swap = false;

        pinned("diff_actions", ACTIONS_HEIGHT_GUESS).show(ui, |ui| {
            let ready = !self.before.is_empty() && !self.after.is_empty();
            let start = run_row_with(
                ui,
                &self.run,
                "Compare",
                ready,
                "Compare the two documents (Ctrl+Enter)",
                |ui| {
                    if flat_button(ui, "Swap")
                        .on_hover_text("Exchange Before and After")
                        .clicked()
                    {
                        swap = true;
                    }
                },
            );
            if start {
                request = self.start();
            }
        });
        if swap && !busy {
            self.before.swap_with(&mut self.after);
            self.run.clear();
        }

        fill().show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if stacked(ui, env.open_doc, &mut self.before, &mut self.after) {
                    self.run.clear();
                }
            });
        });
        request
    }

    fn start(&mut self) -> Option<Request> {
        let (before, after) = (self.before.input()?, self.after.input()?);
        let (gen, cancel) = self.run.start();
        Some(Request::Job {
            tool: Tool::Diff,
            job: Job::Diff { before, after },
            gen,
            cancel,
        })
    }

    fn result_half(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        env.shared.expire(ui.ctx());
        let mut request = None;
        let mut copied_path = None;

        title_row(ui, |ui| {
            let count = self
                .run
                .outcome()
                .map_or(String::new(), |c| format!(" ({})", c.total()));
            ui.selectable_value(&mut self.view, View::Changes, format!("Changes{count}"));
            ui.selectable_value(&mut self.view, View::Patch, "Patch");
            if let Some(compared) = self.run.outcome() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(counts(compared)).small().weak())
                        .on_hover_text(format!("Compared in {:.1?}.", compared.elapsed));
                });
            }
        });
        ui.add_space(6.0);

        pinned("diff_result_actions", RESULT_ACTIONS_HEIGHT_GUESS)
            .show(ui, |ui| request = self.result_actions(ui, env));

        let view = self.view;
        fill().show(ui, |ui| {
            result_box(
                ui,
                "diff",
                &self.run,
                "What differs appears here",
                |ui, compared| match view {
                    View::Changes if compared.equal => {
                        centered_hint(ui, "The documents are the same");
                    }
                    View::Changes => copied_path = changes_list(ui, compared),
                    View::Patch => preview_box(
                        ui,
                        "diff_patch",
                        &compared.patch_preview.text,
                        compared.patch_preview.truncated,
                    ),
                },
            );
        });
        if let Some(path) = copied_path {
            let shown = if path.is_empty() {
                "the whole document".to_owned()
            } else {
                path.clone()
            };
            ui.ctx().copy_text(path);
            env.shared
                .say(Tool::Diff, format!("Copied the path to {shown}"), false);
        }
        request
    }

    /// Save patch… and Copy patch, at the right — there from the start, usable
    /// once there is a result — and the line about what was last done.
    fn result_actions(&self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        enum Act {
            Save,
            Copy,
        }

        let patch = self.run.outcome().map(|c| c.patch.clone());
        let mut act = None;
        action_row(ui, |ui| {
            ui.add_enabled_ui(patch.is_some(), |ui| {
                if ui
                    .add(primary_button(ui, "Save patch…"))
                    .on_hover_text("Write the JSON Patch to a file")
                    .clicked()
                {
                    act = Some(Act::Save);
                }
                if ui
                    .add(secondary_button("Copy patch"))
                    .on_hover_text("Copy the JSON Patch")
                    .clicked()
                {
                    act = Some(Act::Copy);
                }
            });
            notice_line(ui, env.shared.notice_for(Tool::Diff));
        });

        let text = patch?;
        match act? {
            Act::Copy => {
                ui.ctx().copy_text(text.to_string());
                env.shared
                    .say(Tool::Diff, "Copied the patch to the clipboard", false);
                None
            }
            Act::Save => {
                let path = rfd::FileDialog::new()
                    .set_file_name("patch.json")
                    .add_filter("JSON", &["json"])
                    .save_file()?;
                Some(Request::SaveText { text, path })
            }
        }
    }
}

/// "2 added · 1 removed · 2 changed", leaving out what there is none of.
fn counts(compared: &Compared) -> String {
    if compared.equal {
        return "same".to_owned();
    }
    let mut parts = Vec::new();
    for (n, what) in [
        (compared.added, "added"),
        (compared.removed, "removed"),
        (compared.changed, "changed"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    parts.join(" · ")
}

fn tag(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "Added",
        ChangeKind::Removed => "Removed",
        ChangeKind::Changed => "Changed",
    }
}

fn tag_color(visuals: &egui::Visuals, kind: ChangeKind) -> Color32 {
    tint(
        visuals,
        match kind {
            ChangeKind::Added => Tint::Good,
            ChangeKind::Removed => Tint::Bad,
            ChangeKind::Changed => Tint::Warn,
        },
    )
}

/// The list of changes. Returns the path of the one that was clicked, which
/// the caller copies.
fn changes_list(ui: &mut egui::Ui, compared: &Compared) -> Option<String> {
    let shown = compared.changes.len();
    // A last row says how many more there are, when the list was cut short.
    let hidden = compared.total().saturating_sub(shown);
    let rows = shown + usize::from(hidden > 0);
    let mut clicked = None;
    let paths = PathColumn::of(
        ui,
        compared.changes.iter().map(|c| {
            if c.path.is_empty() {
                WHOLE
            } else {
                c.path.as_str()
            }
        }),
    );

    // Rows touch, so their stripes do. (Set before the scroll area, which takes
    // its idea of how tall a row is from this.)
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("diff_changes")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, rows, |ui, range| {
            for index in range {
                match compared.changes.get(index) {
                    Some(change) => {
                        if change_row(ui, index, change, &paths) {
                            clicked = Some(change.path.clone());
                        }
                    }
                    None => {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!(
                                "…and {hidden} more; the patch has all of them."
                            ))
                            .small()
                            .weak(),
                        );
                    }
                }
            }
        });
    clicked
}

/// One change: what kind, where, and the value (or the two values). True when
/// it was clicked.
fn change_row(ui: &mut egui::Ui, index: usize, change: &Change, paths: &PathColumn) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );
    let visuals = ui.visuals().clone();
    let fill = if response.hovered() {
        Some(visuals.widgets.hovered.weak_bg_fill)
    } else if index % 2 == 1 {
        Some(visuals.faint_bg_color)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::ZERO, fill);
    }

    // The row is cut into columns: the kind, the path and the values.
    let inner = rect.shrink2(egui::vec2(10.0, 0.0));
    let tag_rect = Rect::from_min_size(inner.min, egui::vec2(TAG_WIDTH, ROW_HEIGHT));
    let rest = inner.width() - TAG_WIDTH;
    let path_width = paths.width(rest);
    let path_rect = Rect::from_min_size(
        egui::pos2(tag_rect.max.x + 4.0, inner.min.y),
        egui::vec2(path_width, ROW_HEIGHT),
    );
    let values_rect =
        Rect::from_min_max(egui::pos2(path_rect.max.x + 12.0, inner.min.y), inner.max);

    let cell = |ui: &mut egui::Ui, rect: Rect| {
        ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        )
    };
    cell(ui, tag_rect).label(
        RichText::new(tag(change.kind))
            .small()
            .strong()
            .color(tag_color(&visuals, change.kind)),
    );
    let path = if change.path.is_empty() {
        WHOLE
    } else {
        change.path.as_str()
    };
    // The labels don't take clicks (selecting their text would), so that a click
    // anywhere on the row is a click on the row.
    cell(ui, path_rect).add(
        egui::Label::new(RichText::new(path).monospace())
            .truncate()
            .selectable(false),
    );

    let weak = visuals.weak_text_color();
    let value = |ui: &mut egui::Ui, rect: Rect, text: &str| {
        cell(ui, rect).add(
            egui::Label::new(RichText::new(text).monospace().weak())
                .truncate()
                .selectable(false),
        );
    };
    match (&change.before, &change.after) {
        (Some(before), Some(after)) => {
            let side = ((values_rect.width() - ARROW_WIDTH) / 2.0).floor().max(0.0);
            let left = Rect::from_min_size(values_rect.min, egui::vec2(side, ROW_HEIGHT));
            let right = Rect::from_min_size(
                egui::pos2(values_rect.max.x - side, values_rect.min.y),
                egui::vec2(side, ROW_HEIGHT),
            );
            value(ui, left, before);
            value(ui, right, after);
            arrow(
                ui,
                egui::pos2(
                    values_rect.min.x + side + ARROW_WIDTH / 2.0,
                    rect.center().y,
                ),
                weak,
            );
        }
        (Some(only), None) | (None, Some(only)) => value(ui, values_rect, only),
        (None, None) => {}
    }

    let hover = match (&change.before, &change.after) {
        (Some(before), Some(after)) => format!("{path}\nwas {before}\nnow {after}"),
        (Some(before), None) => format!("{path}\nwas {before}"),
        (None, Some(after)) => format!("{path}\nnow {after}"),
        (None, None) => path.to_owned(),
    };
    response
        .on_hover_text(format!("{hover}\n\nClick to copy the path"))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// An arrow pointing right, drawn rather than typed: the font has no arrows.
fn arrow(ui: &egui::Ui, center: egui::Pos2, color: Color32) {
    let stroke = egui::Stroke::new(1.3, color);
    let painter = ui.painter();
    let tip = center + egui::vec2(6.0, 0.0);
    painter.line_segment([center - egui::vec2(6.0, 0.0), tip], stroke);
    painter.line_segment([tip, tip + egui::vec2(-3.5, -3.5)], stroke);
    painter.line_segment([tip, tip + egui::vec2(-3.5, 3.5)], stroke);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::tools::jobs::Preview;

    fn compared(added: usize, removed: usize, changed: usize) -> Compared {
        Compared {
            changes: Vec::new(),
            added,
            removed,
            changed,
            equal: added + removed + changed == 0,
            patch: Arc::from("[]"),
            patch_preview: Preview {
                text: "[]".to_owned(),
                truncated: false,
            },
            elapsed: Duration::ZERO,
        }
    }

    #[test]
    fn the_counts_leave_out_what_there_is_none_of() {
        assert_eq!(
            counts(&compared(2, 1, 3)),
            "2 added · 1 removed · 3 changed"
        );
        assert_eq!(counts(&compared(0, 0, 1)), "1 changed");
        assert_eq!(counts(&compared(0, 0, 0)), "same");
    }

    #[test]
    fn nothing_to_compare_nothing_to_start() {
        let mut diff = Diff::default();
        diff.before.set_text("[1]");
        assert!(diff.start().is_none(), "the second document is missing");
        diff.after.set_text("[2]");
        assert!(matches!(
            diff.start(),
            Some(Request::Job {
                tool: Tool::Diff,
                gen: 1,
                ..
            })
        ));
    }
}
