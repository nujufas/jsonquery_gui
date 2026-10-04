//! The Tools window: a second native window (an egui *viewport*, like the
//! tutorial) that holds the small utilities that go with the main viewer:
//!
//! * **Merge JSON** — several files into one, with a jq filter (`merge.rs`);
//! * **Format JSON** — pretty-print, minify, sort the keys (`format.rs`);
//! * **Diff JSON** — what differs between two documents, as a list and as a
//!   JSON Patch (`diff.rs`);
//! * **Patch JSON** — apply a JSON Patch or a JSON Merge Patch (`patch.rs`);
//! * **Validate schema** — check a document against a JSON Schema
//!   (`validate.rs`).
//!
//! The logic of each is in `crates/query` and the work runs on the worker
//! thread (`jobs.rs`); a page here only collects what the tool needs, shows what
//! came out, and asks the app — through a [`Request`], the way the tutorial
//! does — to do whatever has to be done outside the window: start a job, open
//! a result in the main window, save a file.
//!
//! The window looks like the main window, and is made of the same parts
//! (`widgets.rs`): a row of tabs on top, one for each tool; a command row with
//! the page's main button and options, as the Query row has Run; two panes, each
//! with a heading, its buttons and a line in a header, as Source and Results
//! have; and a status bar at the bottom. The JSON a tool works on goes in the
//! same kind of box everywhere (`operand.rs`). Explanations are tooltips rather
//! than paragraphs.
//!
//! On a desktop it is a *deferred* viewport — eframe redraws the window by
//! itself and calls back into the app through its lock (`App::satellite_frame`)
//! — so it keeps working when the main window is not being redrawn, which is
//! the case once something covers the main window completely (GNOME sends no
//! redraw callbacks then). Where there are no real windows (an embedded
//! viewport; the headless tests) it is an immediate viewport, drawn inside the
//! main window's frame, and egui shows it as a floating window.

mod diff;
mod format;
pub(crate) mod jobs;
mod merge;
mod operand;
mod patch;
mod shared;
mod side_by_side;
mod validate;
mod widgets;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use eframe::egui::{self, RichText};
use jsonquery_core::Document;

use crate::worker::MergeOutcome;
use diff::Diff;
use format::Format;
use jobs::{Job, Outcome};
use merge::Merge;
use patch::Patch;
use shared::{Env, Shared};
use validate::Validate;
use widgets::ERROR;

/// What the window asks the app to do.
pub enum Request {
    /// Start a merge on the worker; its answer comes back to
    /// [`Tools::merge_done`] under the same `gen`.
    Merge {
        paths: Vec<PathBuf>,
        filter: String,
        gen: u64,
        cancel: Arc<AtomicBool>,
    },
    /// Start a job on the worker; its answer comes back to [`Tools::job_done`]
    /// under the same `tool` and `gen`.
    Job {
        tool: Tool,
        job: Job,
        gen: u64,
        cancel: Arc<AtomicBool>,
    },
    /// Show the document in the main window.
    Open(Arc<Document>),
    /// Write the document to `path`.
    Save { doc: Arc<Document>, path: PathBuf },
    /// Write the text to `path`.
    SaveText { text: Arc<str>, path: PathBuf },
    /// Show the value at this JSON Pointer in the document open in the main
    /// window.
    Show(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Merge,
    Format,
    Diff,
    Patch,
    Validate,
}

impl Tool {
    const ALL: [Tool; 5] = [
        Tool::Merge,
        Tool::Format,
        Tool::Diff,
        Tool::Patch,
        Tool::Validate,
    ];

    fn label(self) -> &'static str {
        match self {
            Tool::Merge => "Merge JSON",
            Tool::Format => "Format JSON",
            Tool::Diff => "Diff JSON",
            Tool::Patch => "Patch JSON",
            Tool::Validate => "Validate schema",
        }
    }
}

pub struct Tools {
    open: bool,
    tool: Tool,
    merge: Merge,
    format: Format,
    diff: Diff,
    patch: Patch,
    validate: Validate,
    shared: Shared,
    icon: Option<Arc<egui::IconData>>,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            open: false,
            tool: Tool::Merge,
            merge: Merge::default(),
            format: Format::default(),
            diff: Diff::default(),
            patch: Patch::default(),
            validate: Validate::default(),
            shared: Shared::default(),
            icon: None,
        }
    }
}

pub(crate) fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("jsonquery_tools_window")
}

impl Tools {
    /// Open the window, or bring it to the front if it's already open.
    pub fn open_or_focus(&mut self, ctx: &egui::Context) {
        if self.open {
            ctx.send_viewport_cmd_to(viewport_id(), egui::ViewportCommand::Focus);
        } else {
            self.open = true;
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The window was closed (by the user): stop showing it.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Open the window on `tool`'s page.
    #[cfg(test)]
    pub fn open_on(&mut self, ctx: &egui::Context, tool: Tool) {
        self.tool = tool;
        self.open_or_focus(ctx);
    }

    /// Put `text` in the box called `title` on `tool`'s page.
    #[cfg(test)]
    pub fn fill(&mut self, tool: Tool, title: &str, text: &str) {
        let operand = match tool {
            Tool::Merge => None,
            Tool::Format => self.format.operand(title),
            Tool::Diff => self.diff.operand(title),
            Tool::Patch => self.patch.operand(title),
            Tool::Validate => self.validate.operand(title),
        };
        operand
            .unwrap_or_else(|| panic!("there is no box called {title:?} on {tool:?}"))
            .set_text(text);
    }

    /// The text in the box called `title` on `tool`'s page.
    #[cfg(test)]
    pub fn box_text(&mut self, tool: Tool, title: &str) -> String {
        let operand = match tool {
            Tool::Merge => None,
            Tool::Format => self.format.operand(title),
            Tool::Diff => self.diff.operand(title),
            Tool::Patch => self.patch.operand(title),
            Tool::Validate => self.validate.operand(title),
        };
        operand
            .unwrap_or_else(|| panic!("there is no box called {title:?} on {tool:?}"))
            .text()
            .to_owned()
    }

    /// Read the file at `path` into the box called `title` on `tool`'s page, as
    /// "Open file…" does.
    #[cfg(test)]
    pub fn fill_file(&mut self, tool: Tool, title: &str, path: &Path) {
        let operand = match tool {
            Tool::Merge => None,
            Tool::Format => self.format.operand(title),
            Tool::Diff => self.diff.operand(title),
            Tool::Patch => self.patch.operand(title),
            Tool::Validate => self.validate.operand(title),
        };
        operand
            .unwrap_or_else(|| panic!("there is no box called {title:?} on {tool:?}"))
            .load_file(path);
    }

    /// Put `paths` in the Merge tool's list (after the files already there)
    /// and show it — what dropping several files on the main window does.
    pub fn add_files(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        self.tool = Tool::Merge;
        self.merge.add(paths);
        self.open_or_focus(ctx);
    }

    /// The answer to a [`Request::Merge`]. One from a merge that has since
    /// been cancelled or replaced is dropped.
    pub fn merge_done(&mut self, gen: u64, result: Result<MergeOutcome, String>) {
        self.merge.done(gen, result);
    }

    /// The answer to a [`Request::Job`]. One to a job that has since been
    /// replaced is dropped.
    pub fn job_done(&mut self, tool: Tool, gen: u64, result: Result<Outcome, String>) {
        fn pick<T>(
            result: Result<Outcome, String>,
            from: impl FnOnce(Outcome) -> Option<T>,
        ) -> Result<T, String> {
            result.and_then(|outcome| {
                from(outcome).ok_or_else(|| "the answer was for another tool".to_owned())
            })
        }
        match tool {
            Tool::Merge => {}
            Tool::Format => self.format.done(
                gen,
                pick(result, |o| match o {
                    Outcome::Format(f) => Some(f),
                    _ => None,
                }),
            ),
            Tool::Diff => self.diff.done(
                gen,
                pick(result, |o| match o {
                    Outcome::Diff(d) => Some(d),
                    _ => None,
                }),
            ),
            Tool::Patch => self.patch.done(
                gen,
                pick(result, |o| match o {
                    Outcome::Patch(p) => Some(p),
                    _ => None,
                }),
            ),
            Tool::Validate => self.validate.done(
                gen,
                pick(result, |o| match o {
                    Outcome::Validate(v) => Some(v),
                    _ => None,
                }),
            ),
        }
    }

    /// A save finished; says so if it was one this window asked for.
    pub fn saved(&mut self, path: &Path) {
        if let Some((tool, _)) = self
            .shared
            .saving
            .take_if(|(_, saving)| saving.as_path() == path)
        {
            self.shared
                .say(tool, format!("Saved to {}", path.display()), false);
        }
    }

    /// A save failed; says so if it was one this window asked for.
    pub fn save_failed(&mut self, error: &str) {
        if let Some((tool, _)) = self.shared.saving.take() {
            self.shared
                .say(tool, format!("Could not save: {error}"), true);
        }
    }

    /// The window the page is shown in. The same every frame it is open: egui
    /// patches the real window to match a builder that changed.
    pub fn builder(&mut self) -> egui::ViewportBuilder {
        let icon = self
            .icon
            .get_or_insert_with(|| Arc::new(crate::app_icon()))
            .clone();
        egui::ViewportBuilder::default()
            .with_title("jsonquery — Tools")
            .with_inner_size([900.0, 580.0])
            .with_min_inner_size([760.0, 440.0])
            .with_app_id(crate::APP_ID)
            .with_icon(icon)
    }

    /// One frame of the window when it is a window of its own, redrawn by
    /// eframe by itself. `open_doc` is the document open in the main window.
    /// Returns what the window asks the app to do, if anything.
    pub fn window_frame(
        &mut self,
        ui: &mut egui::Ui,
        open_doc: Option<&Arc<Document>>,
    ) -> Option<Request> {
        self.contents(ui, true, open_doc)
    }

    /// Draw the window (if open) for this frame as an immediate viewport, inside
    /// the main window's frame — where there are no real windows to redraw by
    /// themselves. `open_doc` is the document open in the main window, which the
    /// tools can work on. Returns what the window asks the app to do, if
    /// anything.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open_doc: Option<&Arc<Document>>,
    ) -> Option<Request> {
        if !self.open {
            return None;
        }

        let builder = self.builder();
        let mut request = None;
        let mut close = false;
        ctx.show_viewport_immediate(viewport_id(), builder, |ui, class| {
            close = ui.ctx().input(|i| i.viewport().close_requested());
            // An embedded window shares the main window's input, and so its
            // dropped files — which the main window has already taken.
            let own_input = class != egui::ViewportClass::EmbeddedWindow;
            request = self.contents(ui, own_input, open_doc);
        });
        if close {
            self.open = false;
        }
        request
    }

    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        own_input: bool,
        open_doc: Option<&Arc<Document>>,
    ) -> Option<Request> {
        self.shared.expire(ui.ctx());
        egui::Panel::top("tools_tabs").show(ui, |ui| self.tabs(ui));
        egui::Panel::bottom("tools_status").show(ui, |ui| self.status_bar(ui));

        // The page takes what is left: its command row, then its two panes.
        let mut env = Env {
            open_doc,
            own_input,
            shared: &mut self.shared,
        };
        let request = match self.tool {
            Tool::Merge => self.merge.ui(ui, &mut env),
            Tool::Format => self.format.ui(ui, &mut env),
            Tool::Diff => self.diff.ui(ui, &mut env),
            Tool::Patch => self.patch.ui(ui, &mut env),
            Tool::Validate => self.validate.ui(ui, &mut env),
        };

        // A save the window asked for: remember which, to say how it went.
        if let Some(Request::Save { path, .. } | Request::SaveText { path, .. }) = &request {
            self.shared.saving = Some((self.tool, path.clone()));
            self.shared.say(self.tool, "Saving…", false);
        }
        request
    }

    /// The tools, as tabs: what the Tree and Text tabs of the main window's
    /// panes are.
    fn tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for tool in Tool::ALL {
                ui.selectable_value(&mut self.tool, tool, tool.label());
            }
        });
    }

    /// What the page says about its result, then the line about what was last
    /// done (a save, a copy), as the main window's status bar says such things.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        // The bar keeps its height when there is nothing to say.
        let row = ui.spacing().interact_size.y;
        ui.set_min_height(row);
        ui.horizontal_wrapped(|ui| {
            let said = match self.tool {
                Tool::Merge => self.merge.status_line(ui),
                Tool::Format => self.format.status_line(ui),
                Tool::Diff => self.diff.status_line(ui),
                Tool::Patch => self.patch.status_line(ui),
                Tool::Validate => self.validate.status_line(ui),
            };
            if let Some(notice) = self.shared.notice_for(self.tool) {
                if said {
                    ui.separator();
                }
                if notice.error {
                    ui.colored_label(ERROR, notice.text.as_str());
                } else {
                    ui.weak(notice.text.as_str());
                }
            }
        });
    }
}

/// Small, dimmed 🛠 button that opens the Tools window (or brings it to the
/// front if it's already open) — the same single-icon-plus-hover-text shape as
/// the tutorial and autocomplete buttons beside it.
pub fn button(ui: &mut egui::Ui, tools: &mut Tools) {
    let icon = RichText::new("🛠").color(ui.visuals().weak_text_color());
    if ui
        .button(icon)
        .on_hover_text(
            "Tools — merge, format, diff, patch and validate JSON.\n\
             Drop several files on the window to merge them.",
        )
        .clicked()
    {
        tools.open_or_focus(ui.ctx());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_the_window_asked_for_is_reported_on_the_page_that_asked() {
        let mut tools = Tools::default();
        tools.shared.saving = Some((Tool::Patch, PathBuf::from("/x/a.json")));

        // Someone else's save (the main window's) is none of its business.
        tools.saved(Path::new("/x/other.json"));
        assert!(tools.shared.saving.is_some());
        assert!(tools.shared.notice_for(Tool::Patch).is_none());

        tools.saved(Path::new("/x/a.json"));
        assert!(tools.shared.saving.is_none());
        let notice = tools.shared.notice_for(Tool::Patch).expect("it says so");
        assert_eq!(notice.text, "Saved to /x/a.json");
        assert!(!notice.error);
        assert!(tools.shared.notice_for(Tool::Format).is_none());
    }

    #[test]
    fn a_failed_save_is_an_error_line_but_only_if_the_window_asked() {
        let mut tools = Tools::default();
        tools.save_failed("disk full");
        assert!(tools.shared.notice_for(Tool::Format).is_none());

        tools.shared.saving = Some((Tool::Format, PathBuf::from("/x/a.json")));
        tools.save_failed("disk full");
        let notice = tools.shared.notice_for(Tool::Format).expect("it says so");
        assert_eq!(notice.text, "Could not save: disk full");
        assert!(notice.error);
        assert!(tools.shared.saving.is_none());
    }

    #[test]
    fn every_tool_has_a_page_in_the_list() {
        let labels: Vec<_> = Tool::ALL.iter().map(|t| t.label()).collect();
        assert_eq!(
            labels,
            [
                "Merge JSON",
                "Format JSON",
                "Diff JSON",
                "Patch JSON",
                "Validate schema"
            ]
        );
    }
}
