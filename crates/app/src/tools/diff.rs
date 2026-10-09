//! Diff JSON: compare two documents and show what was added, removed and
//! changed — side by side, line by line, as a file-comparison tool does, as a
//! list of changes with their paths, and as the RFC 6902 JSON Patch that turns
//! the first into the second (see `jsonquery_query::diff`). The documents can be
//! typed or pasted, files, or the document open in the main window.
//!
//! The command row has Compare and Swap, the four views as tabs, Previous and
//! Next difference for the side-by-side view, Move to the left and Move to the
//! right for what is picked there (the lines that are, or else the difference
//! that is), and — pinned at the right, in every view — Copy patch and Save patch….
//! Documents is where the two documents are put in, next to each other. The
//! three views of a comparison — Side by side, Changes and Patch — compare the two
//! when they are asked for and nothing has been compared yet, so Compare need not be
//! pressed; it does the same, and opens the side-by-side view, which is two
//! columns (`side_by_side.rs`) with two arrows in the gutter for each difference to
//! move it, a line of either column to double-click and type over (`line_edit.rs`), and a
//! Save… over each column for a document that a move or an edit changed; Changes is
//! the list; Patch the patch text.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Color32, Key, Layout, Modifiers, Rect, RichText};
use jsonquery_query::diff::{Change, ChangeKind, Side, MAX_ROWS};

use super::jobs::{Compared, Job, Take};
use super::operand::{deliver, drop_target, dropped_files, Operand};
use super::shared::{Env, Run};
use super::side_by_side::{Doc, Move, Typed, Viewer};
use super::widgets::{
    command_bar, halves, preview_box, result_area, row_background, run_button, tint, PathColumn,
    Tint, ERROR, RIGHT_MIN, ROW_HEIGHT,
};
use super::{Request, Tool};
use crate::pane_header;

/// What the path of a change that is the whole document is called.
const WHOLE: &str = "(whole document)";
/// Width of the column with "Added", "Removed" or "Changed" in it.
const TAG_WIDTH: f32 = 60.0;
/// Width of the arrow between the two values of a change.
const ARROW_WIDTH: f32 = 20.0;

/// What the page shows under the command row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    /// The two documents, to put in or change.
    Documents,
    SideBySide,
    Changes,
    Patch,
}

pub(super) struct Diff {
    /// The document the patch starts from, and the one it ends at. Called
    /// Left and Right, not Before and After: two files that are compared need
    /// not be two versions of one.
    left: Operand,
    right: Operand,
    page: Page,
    run: Run<Compared>,
    viewer: Viewer,
    /// A move that is on the worker: where in the view it was (the place in the
    /// list that the difference that follows it takes), and into which document.
    moving: Option<Move>,
    /// A move that has been made and not yet announced: into which document, and
    /// whether it was lines that were picked.
    moved: Option<(Side, bool)>,
    /// A line that was typed over, which is on the worker.
    editing: Option<Editing>,
    /// One that has been made and not yet announced.
    edited: Option<Editing>,
    /// A document that was asked to be saved, and where to: until the worker says it
    /// is written.
    saving: Option<(Side, PathBuf)>,
}

/// A line typed over: in which document, which line of it, and whether that took it out.
#[derive(Clone, Copy)]
struct Editing {
    side: Side,
    line: usize,
    deleted: bool,
}

impl Default for Diff {
    fn default() -> Self {
        Self {
            left: Operand::new("Left", "diff_left", "Paste JSON here, or drop a file"),
            right: Operand::new("Right", "diff_right", "Paste JSON here, or drop a file"),
            page: Page::Documents,
            run: Run::default(),
            viewer: Viewer::default(),
            moving: None,
            moved: None,
            editing: None,
            edited: None,
            saving: None,
        }
    }
}

impl Diff {
    #[cfg(test)]
    pub(super) fn operand(&mut self, title: &str) -> Option<&mut Operand> {
        [&mut self.left, &mut self.right]
            .into_iter()
            .find(|o| o.title() == title)
    }

    fn operand_of(&mut self, side: Side) -> &mut Operand {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    /// The answer to a [`Request::Job`] for this page. A comparison that is
    /// answered (or fails) is shown at once, on the side-by-side view if the
    /// page was still on the documents — except one that was cancelled, which
    /// leaves the documents to be worked on.
    pub(super) fn done(&mut self, gen: u64, result: Result<Compared, String>) {
        let cancelled = matches!(&result, Err(e) if e == "cancelled");
        if !self.run.finish(gen, result) {
            return;
        }
        let moving = self.moving.take();
        let editing = self.editing.take();
        // A move, or a line that was typed over, has changed one of the documents: its box
        // has the new text.
        if let Some(moved) = self.run.outcome().and_then(|c| c.moved.as_ref()) {
            let (text, into) = (moved.text.clone(), moved.into);
            self.operand_of(into).set_moved(text);
            match editing {
                Some(editing) => self.edited = Some(editing),
                None => self.moved = Some((into, moving.as_ref().is_some_and(|m| m.picked))),
            }
        }
        // What was typed over, or moved, is looked at where it was: the view does not go on
        // to the next difference by itself, nor pick it; the lines under it are where they
        // were (higher by the lines the move took out).
        if editing.is_some() || moving.is_some() {
            self.viewer.reset_keeping_place();
        } else {
            self.viewer.reset();
        }
        if self.page == Page::Documents && !cancelled {
            self.page = Page::SideBySide;
        }
    }

    /// What was compared no longer matches what is on the page.
    fn invalidate(&mut self) {
        self.run.clear();
        self.viewer.reset();
        self.page = Page::Documents;
    }

    /// The comparison is wanted, as the view being looked at needs it, and has not been
    /// made (or was cancelled): both documents are there and nothing is working on it.
    fn wants_comparing(&self) -> bool {
        let made = match &self.run.result {
            None => false,
            Some(Err(error)) => error != "cancelled",
            Some(Ok(_)) => true,
        };
        !made && !self.run.running() && !self.left.is_empty() && !self.right.is_empty()
    }

    /// What the buttons of the command row and the keys move into `into`: the lines
    /// that are picked in the side-by-side view, or else the difference that is.
    fn to_move(&self, into: Side) -> Option<Move> {
        let view = self.run.outcome()?.view.as_ref().ok()?;
        self.viewer.to_move(view, into)
    }

    /// A save of a document was written to `path`: it is that file now.
    pub(super) fn saved(&mut self, path: &Path) {
        if let Some((side, _)) = self.saving.take_if(|(_, saving)| saving == path) {
            self.operand_of(side).saved_as(path);
        }
    }

    /// A save failed: the document is as it was.
    pub(super) fn save_failed(&mut self) {
        self.saving = None;
    }

    /// The request to write a document to `path`, which is waited for.
    fn save_to(&mut self, side: Side, path: PathBuf) -> Option<Request> {
        let text = self.operand_of(side).text_to_save()?;
        self.saving = Some((side, path.clone()));
        Some(Request::SaveText { text, path })
    }

    /// Ask where a document should be written, and write it there: named as the file it
    /// came from was, and in its folder.
    fn save_document(&mut self, side: Side) -> Option<Request> {
        let operand = self.operand_of(side);
        let fallback = match side {
            Side::Left => "left.json",
            Side::Right => "right.json",
        };
        let path = ask_where(operand.save_folder(), &operand.save_name(fallback))?;
        self.save_to(side, path)
    }

    /// What the view says over a document.
    fn doc(&self, side: Side) -> Doc {
        let operand = match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        };
        Doc {
            name: operand.heading(),
            changed: operand.changed(),
            can_save: operand.can_save(),
        }
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        if env.own_input
            && deliver(
                &mut [&mut self.left, &mut self.right],
                dropped_files(ui.ctx()),
            )
        {
            self.invalidate();
        }

        enum Act {
            Save,
            Copy,
        }

        let mut request = None;
        let mut swap = false;
        let mut act = None;
        let mut step = None;
        // What to move, and into which document: from the buttons, the keys, the
        // arrows and the menu of a line.
        let mut take: Option<Move> = None;
        let page_was = self.page;
        let patch = self.run.outcome().map(|c| c.patch.clone());
        // Whether Previous and Next have anywhere to go: none when the view is
        // not the one on show.
        let steps = self
            .run
            .outcome()
            .and_then(|c| c.view.as_ref().ok())
            .filter(|_| self.page == Page::SideBySide)
            .map(|view| [false, true].map(|forward| self.viewer.can_step(view, forward)));
        // Whether there is anything to move: lines are picked or a difference is.
        let can_move = steps.is_some() && self.viewer.can_move() && !self.run.running();
        let count = self
            .run
            .outcome()
            .map_or(String::new(), |c| format!(" ({})", c.total()));

        command_bar(ui, "diff_command", |ui| {
            ui.horizontal(|ui| {
                let ready = !self.left.is_empty() && !self.right.is_empty();
                if run_button(
                    ui,
                    &self.run,
                    "Compare",
                    ready,
                    "Compare the two documents (Ctrl+Enter)",
                ) {
                    request = self.start();
                }
                if ui
                    .add_enabled(!self.run.running(), egui::Button::new("Swap"))
                    .on_hover_text("Exchange Left and Right")
                    .clicked()
                {
                    swap = true;
                }
                ui.separator();

                ui.selectable_value(&mut self.page, Page::Documents, "Documents");
                ui.selectable_value(&mut self.page, Page::SideBySide, "Side by side");
                ui.selectable_value(&mut self.page, Page::Changes, format!("Changes{count}"));
                ui.selectable_value(&mut self.page, Page::Patch, "Patch");

                if let Some(can) = steps {
                    ui.separator();
                    for (label, tip, forward) in [
                        ("⏶", "Previous difference (Alt+Up)", false),
                        ("⏷", "Next difference (Alt+Down)", true),
                    ] {
                        let button = ui
                            .add_enabled(can[usize::from(forward)], egui::Button::new(label))
                            .on_hover_text(tip);
                        if button.clicked() {
                            step = Some(forward);
                        }
                    }
                    ui.separator();
                    for (label, into, tip) in [
                        (
                            "⏴",
                            Side::Left,
                            "Move to the left: Left takes what Right has there. The lines that \
                             are picked, or else the difference that is (Alt+Left)",
                        ),
                        (
                            "⏵",
                            Side::Right,
                            "Move to the right: Right takes what Left has there. The lines that \
                             are picked, or else the difference that is (Alt+Right)",
                        ),
                    ] {
                        let button = ui
                            .add_enabled(can_move, egui::Button::new(label))
                            .on_hover_text(tip)
                            .on_disabled_hover_text(
                                "Pick lines or a difference to move first: click them, use \
                                 Previous and Next, or press an arrow between the two documents",
                            );
                        if button.clicked() {
                            take = self.to_move(into);
                        }
                    }
                }

                pane_header::pinned_right(ui, |ui| {
                    // Right to left: Save patch… is the right-most.
                    if ui
                        .add_enabled(patch.is_some(), egui::Button::new("Save patch…"))
                        .on_hover_text("Write the JSON Patch that turns Left into Right to a file")
                        .clicked()
                    {
                        act = Some(Act::Save);
                    }
                    if ui
                        .add_enabled(patch.is_some(), egui::Button::new("Copy patch"))
                        .on_hover_text("Copy the JSON Patch that turns Left into Right")
                        .clicked()
                    {
                        act = Some(Act::Copy);
                    }
                });
            });
        });

        if swap {
            self.left.swap_with(&mut self.right);
            self.invalidate();
        }
        // A view of the comparison that was asked for, and the documents have not been
        // compared: it is made now, as the Compare button would. (Not while Compare has
        // just been pressed: that one is asked for.)
        if self.page != page_was
            && self.page != Page::Documents
            && request.is_none()
            && !swap
            && self.wants_comparing()
        {
            request = self.start();
        }
        if steps.is_some() && !swap {
            if ui.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowDown)) {
                step = Some(true);
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowUp)) {
                step = Some(false);
            }
            // (not with something typed in a line and not yet taken: that is put in first)
            if can_move && !self.viewer.has_typing() {
                for (key, into) in [(Key::ArrowLeft, Side::Left), (Key::ArrowRight, Side::Right)] {
                    if ui.input_mut(|i| i.consume_key(Modifiers::ALT, key)) {
                        take = self.to_move(into);
                    }
                }
            }
            // Escape lets go of the lines that are picked.
            if self.viewer.has_selection() && ui.input(|i| i.key_pressed(Key::Escape)) {
                self.viewer.clear_selection();
            }
            let view = self.run.outcome().and_then(|c| c.view.as_ref().ok());
            if let (Some(forward), Some(view)) = (step, view) {
                self.viewer.step(view, forward);
            }
        }

        let mut save = None;
        let mut typed = None;
        let copied_path = egui::CentralPanel::default()
            .show(ui, |ui| match self.page {
                Page::Documents => {
                    self.documents(ui, env);
                    None
                }
                _ => self.result(ui, &mut take, &mut save, &mut typed),
            })
            .inner;
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

        if let Some(typed) = typed {
            // A line that was typed over is made before anything else: what else was pressed
            // in the same frame is what made the caret leave it, and is pressed again.
            take = None;
            save = None;
            if request.is_none() {
                request = self.start_edit(typed);
            }
        }
        if let Some(mv) = take {
            // (Not while Compare has just been pressed: that one is asked for.)
            if request.is_none() {
                request = self.start_move(mv);
            }
        }
        if let Some(side) = save {
            if request.is_none() {
                request = self.save_document(side);
            }
        }
        if let Some(editing) = self.edited.take() {
            let to = match editing.side {
                Side::Left => "left",
                Side::Right => "right",
            };
            let said = if editing.deleted {
                format!("Took line {} out of the {to} document", editing.line)
            } else {
                format!("Changed line {} of the {to} document", editing.line)
            };
            env.shared.say(Tool::Diff, said, false);
        }
        if let Some((side, picked)) = self.moved.take() {
            let to = match side {
                Side::Left => "left",
                Side::Right => "right",
            };
            let what = if picked {
                "the picked lines"
            } else {
                "the difference"
            };
            env.shared
                .say(Tool::Diff, format!("Moved {what} to the {to}"), false);
        }

        // Not `?`: with no patch yet, or nothing pressed, `request` (Compare,
        // pressed above) still has to go to the worker.
        if let (Some(text), Some(act)) = (patch, act) {
            match act {
                Act::Copy => {
                    ui.ctx().copy_text(text.to_string());
                    env.shared
                        .say(Tool::Diff, "Copied the patch to the clipboard", false);
                }
                Act::Save => {
                    let path = rfd::FileDialog::new()
                        .set_file_name("patch.json")
                        .add_filter("JSON", &["json"])
                        .save_file();
                    if let Some(path) = path {
                        request = Some(Request::SaveText { text, path });
                    }
                }
            }
        }
        request
    }

    /// The two documents, next to each other.
    fn documents(&mut self, ui: &mut egui::Ui, env: &Env) {
        let (left, right) = halves(ui, "diff_documents", RIGHT_MIN);
        let target = drop_target(&[&self.left, &self.right]);
        let running = self.run.running();
        let mut changed = false;
        left.show(ui, |ui| {
            ui.add_enabled_ui(!running, |ui| {
                changed |= self.left.ui(ui, env, target == 0);
            });
        });
        right.show(ui, |ui| {
            ui.add_enabled_ui(!running, |ui| {
                changed |= self.right.ui(ui, env, target == 1);
            });
        });
        if changed {
            self.invalidate();
        }
    }

    fn start(&mut self) -> Option<Request> {
        let (left, right) = (self.left.input()?, self.right.input()?);
        let (gen, cancel) = self.run.start();
        self.moving = None;
        self.editing = None;
        Some(Request::Job {
            tool: Tool::Diff,
            job: Job::Diff {
                left,
                right,
                take: None,
                edit: None,
            },
            gen,
            cancel,
        })
    }

    /// Ask the worker to type a line over: it changes the document, and compares again, and
    /// the answer puts the new text in the box.
    fn start_edit(&mut self, typed: Typed) -> Option<Request> {
        let (left, right) = (self.left.input()?, self.right.input()?);
        let (gen, cancel) = self.run.start();
        self.moving = None;
        self.editing = Some(Editing {
            side: typed.edit.into,
            line: typed.line,
            deleted: typed.edit.deletes(),
        });
        Some(Request::Job {
            tool: Tool::Diff,
            job: Job::Diff {
                left,
                right,
                take: None,
                edit: Some(typed.edit),
            },
            gen,
            cancel,
        })
    }

    /// Ask the worker to move what `mv` says — differences of the view, or lines
    /// picked out of them — into the left or the right document: it makes the new
    /// document, and compares again, and the answer puts the new text in the box.
    fn start_move(&mut self, mv: Move) -> Option<Request> {
        let (left, right) = (self.left.input()?, self.right.input()?);
        let (gen, cancel) = self.run.start();
        let take = Take {
            changes: mv.changes.clone(),
            into: mv.into,
        };
        self.moving = Some(mv);
        Some(Request::Job {
            tool: Tool::Diff,
            job: Job::Diff {
                left,
                right,
                take: Some(take),
                edit: None,
            },
            gen,
            cancel,
        })
    }

    /// The answer, on the view the page is on. Gives the path of a line whose menu
    /// asked for it to be copied, which the caller copies; `take` is set to what was
    /// asked to be moved, from an arrow or a menu, and `save` to a document whose
    /// Save… was pressed.
    fn result(
        &mut self,
        ui: &mut egui::Ui,
        take: &mut Option<Move>,
        save: &mut Option<Side>,
        typed: &mut Option<Typed>,
    ) -> Option<String> {
        let page = self.page;
        let docs = [self.doc(Side::Left), self.doc(Side::Right)];
        let mut copied_path = None;
        result_area(
            ui,
            "diff",
            &self.run,
            if self.left.is_empty() || self.right.is_empty() {
                "Put a document in both boxes to see what differs"
            } else {
                "What differs appears here"
            },
            |ui, compared| match (page, &compared.view) {
                (Page::SideBySide, Ok(view)) => {
                    let shown = self.viewer.show(ui, view, docs);
                    copied_path = shown.copy_path;
                    if shown.moved.is_some() {
                        *take = shown.moved;
                    }
                    if shown.save.is_some() {
                        *save = shown.save;
                    }
                    if shown.typed.is_some() {
                        *typed = shown.typed;
                    }
                }
                (Page::SideBySide, Err(_)) => {
                    ui.weak(format!(
                        "Too long to show side by side — more than {MAX_ROWS} lines. \
                         Changes and Patch have every difference."
                    ));
                }
                (Page::Changes, _) if compared.equal => {
                    ui.label("The documents are the same");
                }
                (Page::Changes, _) => copied_path = changes_list(ui, compared),
                (Page::Patch, _) => preview_box(
                    ui,
                    "diff_patch",
                    &compared.patch_preview.text,
                    compared.patch_preview.truncated,
                ),
                (Page::Documents, _) => {}
            },
        );
        copied_path
    }

    /// What the status bar says about the result: how many changes of each kind,
    /// which difference the side-by-side view is on and how many lines are picked,
    /// and how long it took. False when there is nothing to say.
    pub(super) fn status_line(&self, ui: &mut egui::Ui) -> bool {
        let Some(compared) = self.run.outcome() else {
            return false;
        };
        ui.label(counts(compared));
        if self.page == Page::SideBySide {
            let view = compared.view.as_ref().ok();
            if let Some(position) = view.and_then(|view| self.viewer.position(view)) {
                ui.separator();
                ui.label(position);
            }
            if let Some(picked) = self.viewer.selection_text() {
                ui.separator();
                ui.label(picked);
            }
            if let Some((text, error)) = self.viewer.editing_text() {
                ui.separator();
                if error {
                    ui.colored_label(ERROR, text);
                } else {
                    ui.label(text);
                }
            }
        }
        ui.separator();
        ui.weak(format!("Compared in {:.1?}", compared.elapsed));
        true
    }

    /// Where the arrow of difference `block` pointing to `into` is, for the tests.
    #[cfg(test)]
    pub(super) fn arrow(&self, block: usize, into: Side) -> Option<egui::Pos2> {
        self.viewer
            .arrows
            .iter()
            .find(|(b, side, _)| *b == block && *side == into)
            .map(|(_, _, rect)| rect.center())
    }
}

/// Ask where to write a document: in `folder` and under `name` to begin with.
fn ask_where(folder: Option<&Path>, name: &str) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(answer) = tests::ANSWER.with(|answer| answer.borrow().clone()) {
        return answer;
    }
    let dialog = rfd::FileDialog::new()
        .set_file_name(name)
        .add_filter("JSON", &["json"]);
    match folder {
        Some(folder) => dialog.set_directory(folder),
        None => dialog,
    }
    .save_file()
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

    // Rows touch, so their highlights do. (Set before the scroll area, which
    // takes its idea of how tall a row is from this.)
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("diff_changes")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, rows, |ui, range| {
            for index in range {
                match compared.changes.get(index) {
                    Some(change) => {
                        if change_row(ui, change, &paths) {
                            clicked = Some(change.path.clone());
                        }
                    }
                    None => {
                        ui.weak(format!("…and {hidden} more; the patch has all of them."));
                    }
                }
            }
        });
    clicked
}

/// One change: what kind, where, and the value (or the two values). True when
/// it was clicked.
fn change_row(ui: &mut egui::Ui, change: &Change, paths: &PathColumn) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );
    row_background(ui, rect, false, response.hovered());
    let visuals = ui.visuals().clone();

    // The row is cut into columns: the kind, the path and the values.
    let inner = rect.shrink2(egui::vec2(4.0, 0.0));
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
    cell(ui, tag_rect)
        .label(RichText::new(tag(change.kind)).color(tag_color(&visuals, change.kind)));
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
        (Some(before), Some(after)) => format!("{path}\nLeft: {before}\nRight: {after}"),
        (Some(before), None) => format!("{path}\nLeft: {before}"),
        (None, Some(after)) => format!("{path}\nRight: {after}"),
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
pub(super) mod tests {
    use std::cell::RefCell;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::tools::jobs::{Moved, Preview};
    use jsonquery_query::diff::SideBySide;

    thread_local! {
        /// What a test answers when a document's Save… asks where to write it: `None`
        /// leaves the dialog to be asked (it must not be), `Some(None)` is a dialog that
        /// was cancelled.
        pub(in crate::tools) static ANSWER: RefCell<Option<Option<PathBuf>>> =
            const { RefCell::new(None) };
    }

    /// Make the next dialogs of this thread answer with `path` (or be cancelled).
    pub(in crate::tools) fn answer_save_dialogs_with(path: Option<PathBuf>) {
        ANSWER.with(|answer| *answer.borrow_mut() = Some(path));
    }

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
            view: Ok(SideBySide::default()),
            moved: None,
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
        diff.left.set_text("[1]");
        assert!(diff.start().is_none(), "the second document is missing");
        diff.right.set_text("[2]");
        assert!(matches!(
            diff.start(),
            Some(Request::Job {
                tool: Tool::Diff,
                gen: 1,
                ..
            })
        ));
    }

    fn started(diff: &mut Diff) -> u64 {
        diff.left.set_text("[1]");
        diff.right.set_text("[2]");
        match diff.start() {
            Some(Request::Job { gen, .. }) => gen,
            _ => panic!("should start"),
        }
    }

    #[test]
    fn a_finished_comparison_opens_the_side_by_side_view() {
        let mut diff = Diff::default();
        assert_eq!(diff.page, Page::Documents);
        let gen = started(&mut diff);
        diff.done(gen, Ok(compared(0, 0, 1)));
        assert_eq!(diff.page, Page::SideBySide);
    }

    #[test]
    fn a_comparison_that_failed_is_shown_where_its_error_is() {
        let mut diff = Diff::default();
        let gen = started(&mut diff);
        diff.done(gen, Err("Right: not JSON".to_owned()));
        assert_eq!(diff.page, Page::SideBySide);
    }

    #[test]
    fn a_comparison_that_was_cancelled_leaves_the_documents_up() {
        let mut diff = Diff::default();
        let gen = started(&mut diff);
        diff.done(gen, Err("cancelled".to_owned()));
        assert_eq!(diff.page, Page::Documents);
    }

    #[test]
    fn an_answer_to_an_old_comparison_changes_nothing() {
        let mut diff = Diff::default();
        let first = started(&mut diff);
        let second = match diff.start() {
            Some(Request::Job { gen, .. }) => gen,
            _ => panic!("should start"),
        };
        diff.done(first, Ok(compared(0, 0, 1)));
        assert_eq!(diff.page, Page::Documents);
        assert!(diff.run.running());
        diff.done(second, Ok(compared(1, 0, 0)));
        assert_eq!(diff.page, Page::SideBySide);
    }

    #[test]
    fn a_change_to_the_documents_takes_the_page_back_to_them() {
        let mut diff = Diff::default();
        let gen = started(&mut diff);
        diff.done(gen, Ok(compared(0, 0, 1)));
        diff.page = Page::Patch;
        diff.invalidate();
        assert_eq!(diff.page, Page::Documents);
        assert!(diff.run.outcome().is_none());
    }

    #[test]
    fn a_view_that_needs_the_comparison_wants_it_made_only_when_it_is_not() {
        let mut diff = Diff::default();
        assert!(!diff.wants_comparing(), "nothing in the boxes");
        diff.left.set_text("[1]");
        assert!(!diff.wants_comparing(), "one box is empty");
        diff.right.set_text("[2]");
        assert!(diff.wants_comparing(), "both documents are there");

        let gen = match diff.start() {
            Some(Request::Job { gen, .. }) => gen,
            _ => panic!("should start"),
        };
        assert!(!diff.wants_comparing(), "it is being made");
        diff.done(gen, Ok(compared(0, 0, 1)));
        assert!(!diff.wants_comparing(), "it has been made");

        // One that failed is not made again until a document is changed...
        let mut failed = Diff::default();
        let gen = started(&mut failed);
        failed.done(gen, Err("Right: not JSON".to_owned()));
        assert!(!failed.wants_comparing());
        // ...but one that was cancelled is asked for again.
        let mut cancelled = Diff::default();
        let gen = started(&mut cancelled);
        cancelled.done(gen, Err("cancelled".to_owned()));
        assert!(cancelled.wants_comparing());
        // And a change to a document drops the comparison, so that it is wanted.
        diff.invalidate();
        assert!(diff.wants_comparing());
    }

    /// A comparison of `[1]` and `[2]`, the left one read from /data/orders.json, whose
    /// answer says a move changed the Left document.
    fn moved_left(diff: &mut Diff) {
        diff.left.set_text("[1]");
        diff.right.set_text("[2]");
        diff.left.pretend_read_from(Path::new("/data/orders.json"));
        let gen = match diff.start() {
            Some(Request::Job { gen, .. }) => gen,
            _ => panic!("should start"),
        };
        let mut answer = compared(0, 0, 0);
        answer.moved = Some(Moved {
            into: Side::Left,
            text: Arc::from("[\n  2\n]"),
        });
        diff.done(gen, Ok(answer));
    }

    #[test]
    fn a_move_leaves_the_document_called_by_its_file_and_changed() {
        let mut diff = Diff::default();
        moved_left(&mut diff);
        assert_eq!(diff.left.text(), "[\n  2\n]");
        assert!(diff.left.changed());
        assert_eq!(
            diff.left.heading().as_deref(),
            Some("orders.json (changed)")
        );
        assert!(!diff.right.changed(), "the other one is as it was");
        assert_eq!(diff.right.heading(), None);
        assert_eq!(
            diff.doc(Side::Left).name.as_deref(),
            Some("orders.json (changed)")
        );
        assert!(diff.doc(Side::Left).can_save && diff.doc(Side::Left).changed);
        assert_eq!(diff.moved, Some((Side::Left, false)));
    }

    #[test]
    fn a_document_is_saved_as_a_file_and_then_is_that_file_and_not_changed() {
        let mut diff = Diff::default();
        moved_left(&mut diff);
        let Some(Request::SaveText { text, path }) =
            diff.save_to(Side::Left, PathBuf::from("/data/orders.json"))
        else {
            panic!("a save was expected");
        };
        assert_eq!(&*text, "[\n  2\n]");
        assert_eq!(path, PathBuf::from("/data/orders.json"));
        // Until the worker says it is written, it is changed.
        assert!(diff.left.changed());

        // Someone else's save is none of its business.
        diff.saved(Path::new("/data/other.json"));
        assert!(diff.left.changed());
        diff.saved(Path::new("/data/orders.json"));
        assert!(!diff.left.changed());
        assert_eq!(diff.left.heading().as_deref(), Some("orders.json"));
        assert_eq!(diff.left.save_folder(), Some(Path::new("/data")));
        assert!(diff.saving.is_none());

        // A save that fails leaves it as it was.
        moved_left(&mut diff);
        diff.save_to(Side::Left, PathBuf::from("/data/orders.json"));
        diff.save_failed();
        assert!(diff.saving.is_none());
        assert!(diff.left.changed());
    }

    #[test]
    fn what_there_is_no_text_of_is_not_saved() {
        let mut diff = Diff::default();
        assert!(diff.save_to(Side::Left, PathBuf::from("/x.json")).is_none());
        assert!(diff.saving.is_none());
        diff.left.set_text("   ");
        assert!(!diff.left.can_save(), "blanks are no document");
    }

    #[test]
    fn the_save_dialog_is_asked_with_the_name_of_the_file_and_can_be_cancelled() {
        let mut diff = Diff::default();
        moved_left(&mut diff);
        answer_save_dialogs_with(None);
        assert!(diff.save_document(Side::Left).is_none(), "cancelled");
        assert!(diff.saving.is_none());
        answer_save_dialogs_with(Some(PathBuf::from("/tmp/chosen.json")));
        let Some(Request::SaveText { path, .. }) = diff.save_document(Side::Left) else {
            panic!("a save was expected");
        };
        assert_eq!(path, PathBuf::from("/tmp/chosen.json"));
        assert_eq!(
            diff.saving,
            Some((Side::Left, PathBuf::from("/tmp/chosen.json")))
        );
    }
}
