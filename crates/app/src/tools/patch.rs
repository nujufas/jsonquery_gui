//! Patch JSON: apply a patch to a document — a list of operations (JSON Patch,
//! RFC 6902) or a document to lay over it (JSON Merge Patch, RFC 7386; see
//! `jsonquery_query::patch`). Both boxes take typed or pasted JSON, a file, or
//! the document open in the main window. A patch that fails says which
//! operation, and changes nothing.
//!
//! The left half is the document and the patch with the kind of patch and the
//! Apply button pinned under them; the right half is the result, which can be
//! opened in the main window, saved or copied.

use std::path::Path;

use eframe::egui::{self, Align, Layout, RichText};

use super::format::COPY_LIMIT;
use super::jobs::{Job, Patched};
use super::operand::{deliver, dropped_files, stacked, Operand};
use super::shared::{Env, Run};
use super::widgets::{
    action_row, fill, halves, heading, notice_line, pinned, preview_box, primary_button,
    result_box, run_row, secondary_button, title_row,
};
use super::{Request, Tool};
use crate::worker::describe;

const OPTIONS_HEIGHT_GUESS: f32 = 100.0;
const RESULT_ACTIONS_HEIGHT_GUESS: f32 = 44.0;

pub(super) struct Patch {
    document: Operand,
    patch: Operand,
    /// A JSON Merge Patch (RFC 7386) rather than a list of operations (RFC 6902).
    merge_patch: bool,
    run: Run<Patched>,
}

impl Default for Patch {
    fn default() -> Self {
        Self {
            document: Operand::new(
                "Document",
                "patch_document",
                "Paste the JSON to patch, or drop a file",
            ),
            patch: Operand::new("Patch", "patch_patch", "Paste the patch, or drop a file"),
            merge_patch: false,
            run: Run::default(),
        }
    }
}

impl Patch {
    #[cfg(test)]
    pub(super) fn operand(&mut self, title: &str) -> Option<&mut Operand> {
        [&mut self.document, &mut self.patch]
            .into_iter()
            .find(|o| o.title() == title)
    }

    /// The answer to a [`Request::Job`] for this page.
    pub(super) fn done(&mut self, gen: u64, result: Result<Patched, String>) {
        self.run.finish(gen, result);
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        if env.own_input
            && deliver(
                &mut [&mut self.document, &mut self.patch],
                dropped_files(ui.ctx()),
            )
        {
            self.run.clear();
        }

        heading(ui, "Patch JSON");
        let mut request = None;
        let (left, right) = halves(ui, "patch_halves");
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

        pinned("patch_options", OPTIONS_HEIGHT_GUESS).show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| self.kind_ui(ui));
            ui.add_space(12.0);
            let ready = !self.document.is_empty() && !self.patch.is_empty();
            if run_row(
                ui,
                &self.run,
                "Apply",
                ready,
                "Apply the patch (Ctrl+Enter)",
            ) {
                request = self.start();
            }
        });

        fill().show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if stacked(ui, env.open_doc, &mut self.document, &mut self.patch) {
                    self.run.clear();
                }
            });
        });
        request
    }

    /// Which kind of patch the second box holds.
    fn kind_ui(&mut self, ui: &mut egui::Ui) {
        let before = self.merge_patch;
        ui.horizontal(|ui| {
            ui.label("Kind");
            let shown = if self.merge_patch {
                "Merge patch (RFC 7386)"
            } else {
                "Operations (RFC 6902)"
            };
            egui::ComboBox::from_id_salt("patch_kind")
                .selected_text(shown)
                .width(190.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.merge_patch, false, "Operations (RFC 6902)")
                        .on_hover_text(
                            "A list of operations, like [{\"op\": \"replace\", \"path\": \"/a\", \"value\": 1}]",
                        );
                    ui.selectable_value(&mut self.merge_patch, true, "Merge patch (RFC 7386)")
                        .on_hover_text(
                            "A document laid over the other: its members replace or add, a null deletes",
                        );
                });
        });
        if self.merge_patch != before {
            self.run.clear();
        }
    }

    fn start(&mut self) -> Option<Request> {
        let (document, patch) = (self.document.input()?, self.patch.input()?);
        let (gen, cancel) = self.run.start();
        Some(Request::Job {
            tool: Tool::Patch,
            job: Job::Patch {
                document,
                patch,
                merge_patch: self.merge_patch,
            },
            gen,
            cancel,
        })
    }

    fn result_half(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        env.shared.expire(ui.ctx());
        let mut request = None;

        title_row(ui, |ui| {
            ui.label(RichText::new("Result").strong());
            if let Some(patched) = self.run.outcome() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(summary(patched)).small().weak())
                        .on_hover_text(format!("Patched in {:.1?}.", patched.elapsed));
                });
            }
        });
        ui.add_space(6.0);

        pinned("patch_result_actions", RESULT_ACTIONS_HEIGHT_GUESS)
            .show(ui, |ui| request = self.result_actions(ui, env));

        fill().show(ui, |ui| {
            result_box(
                ui,
                "patch",
                &self.run,
                "The patched document appears here",
                |ui, patched| {
                    preview_box(
                        ui,
                        "patch",
                        &patched.preview.text,
                        patched.preview.truncated,
                    );
                },
            );
        });
        request
    }

    /// Open in main window, Save… and Copy, at the right — there from the
    /// start, usable once there is a result — and the line about what was last
    /// done.
    fn result_actions(&self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        enum Act {
            Open,
            Save,
            Copy,
        }

        let patched = self.run.outcome();
        let mut act = None;
        action_row(ui, |ui| {
            ui.add_enabled_ui(patched.is_some(), |ui| {
                if ui
                    .add(primary_button(ui, "Open in main window"))
                    .on_hover_text(
                        "Show the patched document in the main window, to explore and query it",
                    )
                    .clicked()
                {
                    act = Some(Act::Open);
                }
                if ui
                    .add(secondary_button("Save…"))
                    .on_hover_text("Write the patched document to a file")
                    .clicked()
                {
                    act = Some(Act::Save);
                }
                let too_big = patched.is_some_and(|p| p.text.len() > COPY_LIMIT);
                if ui
                    .add_enabled(!too_big, secondary_button("Copy"))
                    .on_hover_text("Copy the patched document")
                    .on_disabled_hover_text("Too big to copy; save it instead")
                    .clicked()
                {
                    act = Some(Act::Copy);
                }
            });
            notice_line(ui, env.shared.notice_for(Tool::Patch));
        });

        let patched = patched?;
        match act? {
            Act::Open => Some(Request::Open(patched.doc.clone())),
            Act::Copy => {
                ui.ctx().copy_text(patched.text.to_string());
                env.shared
                    .say(Tool::Patch, "Copied to the clipboard", false);
                None
            }
            Act::Save => {
                let path = rfd::FileDialog::new()
                    .set_file_name(save_name(self.document.source_file()))
                    .add_filter("JSON", &["json"])
                    .save_file()?;
                Some(Request::SaveText {
                    text: patched.text.clone(),
                    path,
                })
            }
        }
    }
}

/// What the result is, in a few words: "object · 3 keys · 2 operations".
fn summary(patched: &Patched) -> String {
    let mut summary = describe(&patched.doc.root);
    match patched.operations {
        Some(1) => summary.push_str(" · 1 operation"),
        Some(n) => summary.push_str(&format!(" · {n} operations")),
        None => summary.push_str(" · merge patch"),
    }
    summary
}

/// The file name a save of the result suggests: next to the file it was made
/// from, but not over it.
fn save_name(from: Option<&str>) -> String {
    match from.and_then(|name| Path::new(name).file_stem()) {
        Some(stem) => format!("{}.patched.json", stem.to_string_lossy()),
        None => "patched.json".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use jsonquery_core::{Document, DocumentSource};

    use super::*;
    use crate::tools::jobs::Preview;

    fn patched(root: serde_json::Value, operations: Option<usize>) -> Patched {
        Patched {
            doc: Arc::new(Document::from_value(
                root,
                DocumentSource::Derived {
                    label: "(patched)",
                    file_name: "patched.json",
                },
                0,
                Duration::ZERO,
            )),
            text: Arc::from(""),
            preview: Preview {
                text: String::new(),
                truncated: false,
            },
            operations,
            elapsed: Duration::ZERO,
        }
    }

    #[test]
    fn the_summary_says_what_came_out_and_how() {
        assert_eq!(
            summary(&patched(serde_json::json!({"a": 1}), Some(2))),
            "object · 1 key · 2 operations"
        );
        assert_eq!(
            summary(&patched(serde_json::json!([1, 2]), Some(1))),
            "array · 2 items · 1 operation"
        );
        assert_eq!(
            summary(&patched(serde_json::json!({}), None)),
            "object · 0 keys · merge patch"
        );
    }

    #[test]
    fn a_save_is_named_after_the_document() {
        assert_eq!(save_name(Some("orders.json")), "orders.patched.json");
        assert_eq!(save_name(None), "patched.json");
    }

    #[test]
    fn both_boxes_are_needed() {
        let mut patch = Patch::default();
        patch.document.set_text("{}");
        assert!(patch.start().is_none());
        patch.patch.set_text("[]");
        assert!(matches!(
            patch.start(),
            Some(Request::Job {
                tool: Tool::Patch,
                gen: 1,
                ..
            })
        ));
    }
}
