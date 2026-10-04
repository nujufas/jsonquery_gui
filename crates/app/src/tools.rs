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
//! The pages share one look (`widgets.rs`): a heading over two halves, each with
//! a title row, a box that takes all the height there is, and its buttons pinned
//! at the bottom. The JSON a tool works on goes in the same kind of box
//! everywhere (`operand.rs`). Explanations are tooltips rather than paragraphs.
//!
//! It is an *immediate* viewport, so it runs inside the main window's frame with
//! plain `&mut self` access, and egui falls back to an embedded floating window
//! when the backend can't open native ones.

mod diff;
mod format;
pub(crate) mod jobs;
mod merge;
mod operand;
mod patch;
mod shared;
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
use widgets::{tool_row, PAGE_MARGIN, SIDEBAR_WIDTH};

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

fn viewport_id() -> egui::ViewportId {
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

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.open
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

    /// Draw the window (if open) for this frame. `open_doc` is the document
    /// open in the main window, which the tools can work on. Returns what the
    /// window asks the app to do, if anything.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open_doc: Option<&Arc<Document>>,
    ) -> Option<Request> {
        if !self.open {
            return None;
        }

        let icon = self
            .icon
            .get_or_insert_with(|| Arc::new(crate::app_icon()))
            .clone();
        let builder = egui::ViewportBuilder::default()
            .with_title("jsonquery — Tools")
            .with_inner_size([940.0, 620.0])
            .with_min_inner_size([900.0, 540.0])
            .with_app_id(crate::APP_ID)
            .with_icon(icon);

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
        let mut request = None;

        let sidebar = egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::same(10));
        egui::Panel::left("tools_list")
            .resizable(false)
            .exact_size(SIDEBAR_WIDTH)
            .frame(sidebar)
            .show(ui, |ui| self.tool_list(ui));

        let page =
            egui::Frame::central_panel(ui.style()).inner_margin(egui::Margin::same(PAGE_MARGIN));
        egui::CentralPanel::default().frame(page).show(ui, |ui| {
            let mut env = Env {
                open_doc,
                own_input,
                shared: &mut self.shared,
            };
            request = match self.tool {
                Tool::Merge => self.merge.ui(ui, &mut env),
                Tool::Format => self.format.ui(ui, &mut env),
                Tool::Diff => self.diff.ui(ui, &mut env),
                Tool::Patch => self.patch.ui(ui, &mut env),
                Tool::Validate => self.validate.ui(ui, &mut env),
            };
        });

        // A save the window asked for: remember which, to say how it went.
        if let Some(Request::Save { path, .. } | Request::SaveText { path, .. }) = &request {
            self.shared.saving = Some((self.tool, path.clone()));
            self.shared.say(self.tool, "Saving…", false);
        }
        request
    }

    fn tool_list(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 2.0;
        for tool in Tool::ALL {
            if tool_row(ui, tool.label(), self.tool == tool).clicked() {
                self.tool = tool;
            }
        }
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
