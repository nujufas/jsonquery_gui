//! Format JSON: print a document with two or four spaces or a tab to a level,
//! or minified, with its keys sorted and its non-ASCII characters escaped if
//! wanted (see `jsonquery_query::reformat`). The input can be typed or pasted,
//! a file, or the document open in the main window; the result is previewed and
//! can be copied or saved.
//!
//! The left half is the input with the options and the Format button pinned
//! under it, the right half the result with Save and Copy under it.

use std::path::Path;

use eframe::egui::{self, Align, Layout, RichText};
use jsonquery_query::reformat::{Indent, Options};

use super::jobs::{Formatted, Job};
use super::operand::{deliver, dropped_files, Operand};
use super::shared::{Env, Run};
use super::widgets::{
    action_row, fill, halves, heading, notice_line, pinned, preview_box, primary_button,
    result_box, run_row, secondary_button, title_row,
};
use super::{Request, Tool};
use crate::app::human_bytes;

/// Heights (margin included) the pinned parts are given on the first frame,
/// before egui has measured them.
const OPTIONS_HEIGHT_GUESS: f32 = 100.0;
const RESULT_ACTIONS_HEIGHT_GUESS: f32 = 44.0;

/// The most text Copy puts on the clipboard; Save takes any amount.
pub(super) const COPY_LIMIT: usize = 16 * 1024 * 1024;

const INDENTS: [(Indent, &str); 4] = [
    (Indent::Spaces(2), "2 spaces"),
    (Indent::Spaces(4), "4 spaces"),
    (Indent::Tab, "Tab"),
    (Indent::Minified, "Minified"),
];

pub(super) struct Format {
    input: Operand,
    options: Options,
    run: Run<Formatted>,
}

impl Default for Format {
    fn default() -> Self {
        Self {
            input: Operand::new("Input", "format_input", "Paste JSON here, or drop a file"),
            options: Options::default(),
            run: Run::default(),
        }
    }
}

impl Format {
    #[cfg(test)]
    pub(super) fn operand(&mut self, title: &str) -> Option<&mut Operand> {
        [&mut self.input].into_iter().find(|o| o.title() == title)
    }

    /// The answer to a [`Request::Job`] for this page.
    pub(super) fn done(&mut self, gen: u64, result: Result<Formatted, String>) {
        self.run.finish(gen, result);
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        if env.own_input && deliver(&mut [&mut self.input], dropped_files(ui.ctx())) {
            self.run.clear();
        }

        heading(ui, "Format JSON");
        let mut request = None;
        let (left, right) = halves(ui, "format_halves");
        left.show(ui, |ui| request = self.input_half(ui, env));
        right.show(ui, |ui| {
            if let Some(r) = self.result_half(ui, env) {
                request = Some(r);
            }
        });
        request
    }

    fn input_half(&mut self, ui: &mut egui::Ui, env: &Env) -> Option<Request> {
        let busy = self.run.running();
        let mut request = None;

        pinned("format_options", OPTIONS_HEIGHT_GUESS).show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| self.options_ui(ui));
            ui.add_space(12.0);
            let ready = !self.input.is_empty();
            if run_row(
                ui,
                &self.run,
                "Format",
                ready,
                "Format the document (Ctrl+Enter)",
            ) {
                request = self.start();
            }
        });

        fill().show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if self.input.ui(ui, env.open_doc, true) {
                    self.run.clear();
                }
            });
        });
        request
    }

    /// Indent, and the two checkboxes. Changing one drops an old result.
    fn options_ui(&mut self, ui: &mut egui::Ui) {
        let before = self.options;
        ui.horizontal_wrapped(|ui| {
            ui.label("Indent");
            let shown = INDENTS
                .iter()
                .find(|(indent, _)| *indent == self.options.indent)
                .map_or("2 spaces", |(_, label)| *label);
            egui::ComboBox::from_id_salt("format_indent")
                .selected_text(shown)
                .width(100.0)
                .show_ui(ui, |ui| {
                    for (indent, label) in INDENTS {
                        ui.selectable_value(&mut self.options.indent, indent, label);
                    }
                });
            ui.add_space(8.0);
            ui.checkbox(&mut self.options.sort_keys, "Sort keys")
                .on_hover_text("Put the keys of every object in order, at every depth");
            ui.checkbox(&mut self.options.ascii_only, "ASCII only")
                .on_hover_text("Write characters outside ASCII as \\uXXXX escapes");
        });
        if self.options != before {
            self.run.clear();
        }
    }

    fn start(&mut self) -> Option<Request> {
        let input = self.input.input()?;
        let (gen, cancel) = self.run.start();
        Some(Request::Job {
            tool: Tool::Format,
            job: Job::Format {
                input,
                options: self.options,
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
            if let Some(formatted) = self.run.outcome() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(summary(formatted)).small().weak())
                        .on_hover_text(format!("Formatted in {:.1?}.", formatted.elapsed));
                });
            }
        });
        ui.add_space(6.0);

        pinned("format_result_actions", RESULT_ACTIONS_HEIGHT_GUESS)
            .show(ui, |ui| request = self.result_actions(ui, env));

        fill().show(ui, |ui| {
            result_box(
                ui,
                "format",
                &self.run,
                "The formatted document appears here",
                |ui, formatted| {
                    preview_box(
                        ui,
                        "format",
                        &formatted.preview.text,
                        formatted.preview.truncated,
                    );
                },
            );
        });
        request
    }

    /// Save… and Copy, at the right — there from the start, usable once there
    /// is a result — and the line about what was last done.
    fn result_actions(&self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        enum Act {
            Save,
            Copy,
        }

        let text = self.run.outcome().map(|f| f.text.clone());
        let mut act = None;
        action_row(ui, |ui| {
            ui.add_enabled_ui(text.is_some(), |ui| {
                if ui
                    .add(primary_button(ui, "Save…"))
                    .on_hover_text("Write the formatted text to a file")
                    .clicked()
                {
                    act = Some(Act::Save);
                }
                let too_big = text.as_ref().is_some_and(|t| t.len() > COPY_LIMIT);
                if ui
                    .add_enabled(!too_big, secondary_button("Copy"))
                    .on_hover_text("Copy the formatted text")
                    .on_disabled_hover_text("Too big to copy; save it instead")
                    .clicked()
                {
                    act = Some(Act::Copy);
                }
            });
            notice_line(ui, env.shared.notice_for(Tool::Format));
        });

        let text = text?;
        match act? {
            Act::Copy => {
                ui.ctx().copy_text(text.to_string());
                env.shared
                    .say(Tool::Format, "Copied to the clipboard", false);
                None
            }
            Act::Save => {
                let name = save_name(
                    self.input.source_file(),
                    self.options.indent == Indent::Minified,
                );
                let path = rfd::FileDialog::new()
                    .set_file_name(name)
                    .add_filter("JSON", &["json"])
                    .save_file()?;
                Some(Request::SaveText { text, path })
            }
        }
    }
}

/// "3.4 KB (was 1.2 KB)".
fn summary(formatted: &Formatted) -> String {
    format!(
        "{} (was {})",
        human_bytes(formatted.text.len() as u64),
        human_bytes(formatted.bytes_in)
    )
}

/// The file name a save of the result suggests: next to the file it was made
/// from, but not over it.
fn save_name(from: Option<&str>, minified: bool) -> String {
    let kind = if minified { "min" } else { "formatted" };
    match from.and_then(|name| Path::new(name).file_stem()) {
        Some(stem) => format!("{}.{kind}.json", stem.to_string_lossy()),
        None => format!("{kind}.json"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_is_named_after_the_file_it_came_from() {
        assert_eq!(
            save_name(Some("orders.json"), false),
            "orders.formatted.json"
        );
        assert_eq!(save_name(Some("orders.json"), true), "orders.min.json");
        assert_eq!(save_name(Some("data"), false), "data.formatted.json");
        assert_eq!(save_name(None, false), "formatted.json");
        assert_eq!(save_name(None, true), "min.json");
    }

    #[test]
    fn nothing_to_format_nothing_to_start() {
        let mut format = Format::default();
        assert!(format.start().is_none());
        assert!(!format.run.running());
        format.input.set_text("[1]");
        let Some(Request::Job {
            tool: Tool::Format,
            gen,
            ..
        }) = format.start()
        else {
            panic!("a job was expected")
        };
        assert_eq!(gen, 1);
        assert!(format.run.running());
    }
}
