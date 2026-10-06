//! Format JSON: print a document with two or four spaces or a tab to a level,
//! or minified, with its keys sorted and its non-ASCII characters escaped if
//! wanted (see `jsonquery_query::reformat`). The input can be typed or pasted,
//! a file, or the document open in the main window; the result is previewed and
//! can be copied or saved.
//!
//! The command row has the Format button and the options; the left pane is the
//! input, the right pane the result, with Copy and Save in its header.

use std::path::Path;

use eframe::egui;
use jsonquery_query::reformat::{Indent, Options};

use super::jobs::{Formatted, Job};
use super::operand::{deliver, dropped_files, Operand};
use super::shared::{Env, Run};
use super::widgets::{
    command_bar, halves, header, preview_box, result_area, run_button, RIGHT_MIN,
};
use super::{Request, Tool};
use crate::app::human_bytes;

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
            input: Operand::new("Input", "format_input", "Paste JSON here, or drop a file")
                .and_documents_on_disk(),
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

        let mut request = None;
        command_bar(ui, "format_command", |ui| request = self.command_ui(ui));
        let (left, right) = halves(ui, "format_panes", RIGHT_MIN);
        left.show(ui, |ui| self.input_pane(ui, env));
        right.show(ui, |ui| {
            if let Some(r) = self.result_pane(ui, env) {
                request = Some(r);
            }
        });
        request
    }

    /// The Format button and the options.
    fn command_ui(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let busy = self.run.running();
        let mut request = None;
        ui.horizontal(|ui| {
            let ready = !self.input.is_empty();
            if run_button(
                ui,
                &self.run,
                "Format",
                ready,
                "Format the document (Ctrl+Enter)",
            ) {
                request = self.start();
            }
            ui.add_enabled_ui(!busy, |ui| self.options_ui(ui));
        });
        request
    }

    /// Indent, and the two checkboxes. Changing one drops an old result.
    fn options_ui(&mut self, ui: &mut egui::Ui) {
        let before = self.options;
        ui.label("Indent");
        let shown = INDENTS
            .iter()
            .find(|(indent, _)| *indent == self.options.indent)
            .map_or("2 spaces", |(_, label)| *label);
        egui::ComboBox::from_id_salt("format_indent")
            .selected_text(shown)
            .width(90.0)
            .show_ui(ui, |ui| {
                for (indent, label) in INDENTS {
                    ui.selectable_value(&mut self.options.indent, indent, label);
                }
            });
        ui.checkbox(&mut self.options.sort_keys, "Sort keys")
            .on_hover_text("Put the keys of every object in order, at every depth");
        ui.checkbox(&mut self.options.ascii_only, "ASCII only")
            .on_hover_text("Write characters outside ASCII as \\uXXXX escapes");
        if self.options != before {
            self.run.clear();
        }
    }

    fn input_pane(&mut self, ui: &mut egui::Ui, env: &Env) {
        ui.add_enabled_ui(!self.run.running(), |ui| {
            if self.input.ui(ui, env, true) {
                self.run.clear();
            }
        });
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

    /// The result, with Copy and Save… in the header — there from the start,
    /// usable once there is a result.
    fn result_pane(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        enum Act {
            Save,
            Copy,
        }

        let text = self.run.outcome().map(|f| f.text.clone());
        // A document that is kept as its file is written from there as it is saved,
        // and is no text to copy.
        let streamed = self.run.outcome().and_then(|f| f.streamed.clone());
        let too_big = streamed.is_some() || text.as_ref().is_some_and(|t| t.len() > COPY_LIMIT);
        let mut act = None;
        header(
            ui,
            "Result",
            |_| {},
            |ui| {
                // Right to left: Save… is the right-most.
                if ui
                    .add_enabled(text.is_some(), egui::Button::new("Save…"))
                    .on_hover_text("Write the formatted text to a file")
                    .clicked()
                {
                    act = Some(Act::Save);
                }
                if ui
                    .add_enabled(text.is_some() && !too_big, egui::Button::new("Copy"))
                    .on_hover_text("Copy the formatted text")
                    .on_disabled_hover_text(if too_big {
                        "Too big to copy; save it instead"
                    } else {
                        "Nothing to copy yet"
                    })
                    .clicked()
                {
                    act = Some(Act::Copy);
                }
            },
        );

        result_area(
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
                Some(match streamed {
                    Some(streamed) => Request::SaveFormatted { streamed, path },
                    None => Request::SaveText { text, path },
                })
            }
        }
    }

    /// What the status bar says about the result: its size against the input's,
    /// and how long it took. False when there is nothing to say.
    pub(super) fn status_line(&self, ui: &mut egui::Ui) -> bool {
        let Some(formatted) = self.run.outcome() else {
            return false;
        };
        ui.label(summary(formatted));
        ui.separator();
        ui.weak(format!("Formatted in {:.1?}", formatted.elapsed));
        true
    }
}

/// "3.4 KB (was 1.2 KB)".
fn summary(formatted: &Formatted) -> String {
    if formatted.streamed.is_some() {
        return format!(
            "{} — written from its file when saved",
            human_bytes(formatted.bytes_in)
        );
    }
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
