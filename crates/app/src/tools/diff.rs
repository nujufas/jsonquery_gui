//! Diff JSON: compare two documents and show what was added, removed and
//! changed — side by side, line by line, as a file-comparison tool does, as a
//! list of changes with their paths, and as the RFC 6902 JSON Patch that turns
//! the first into the second (see `jsonquery_query::diff`). The documents can be
//! typed or pasted, files, or the document open in the main window.
//!
//! The command row has Compare and Swap, the four views as tabs, Previous and
//! Next difference for the side-by-side view, Move to the left and Move to the
//! right for the difference that is picked there (or one that is right-clicked),
//! and — pinned at the right, in every view — Copy patch and Save patch….
//! Documents is where the two documents
//! are put in, next to each other; Compare opens the side-by-side view, which is
//! two read-only columns (`side_by_side.rs`); Changes is the list; Patch the
//! patch text.

use eframe::egui::{self, Align, Color32, Key, Layout, Modifiers, Rect, RichText};
use jsonquery_query::diff::{Change, ChangeKind, Side, MAX_ROWS};

use super::jobs::{Compared, Job, Take};
use super::operand::{deliver, drop_target, dropped_files, Operand};
use super::shared::{Env, Run};
use super::side_by_side::Viewer;
use super::widgets::{
    command_bar, halves, preview_box, result_area, row_background, run_button, tint, PathColumn,
    Tint, RIGHT_MIN, ROW_HEIGHT,
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
    /// What the two documents were called when they were compared (a file's
    /// name, the open document), for the side-by-side view's headings.
    names: [Option<String>; 2],
    /// A move that is on the worker: which difference of the view it was (its
    /// place in the list, which the one that follows it takes) and which
    /// document it changes.
    moving: Option<(usize, Side)>,
    /// A move that has been made and not yet announced.
    moved: Option<Side>,
}

impl Default for Diff {
    fn default() -> Self {
        Self {
            left: Operand::new("Left", "diff_left", "Paste JSON here, or drop a file"),
            right: Operand::new("Right", "diff_right", "Paste JSON here, or drop a file"),
            page: Page::Documents,
            run: Run::default(),
            viewer: Viewer::default(),
            names: [None, None],
            moving: None,
            moved: None,
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
        // A move has changed one of the documents: its box has the new text.
        if let Some(moved) = self.run.outcome().and_then(|c| c.moved.as_ref()) {
            let text = moved.text.clone();
            let (operand, name) = match moved.into {
                Side::Left => (&mut self.left, &mut self.names[0]),
                Side::Right => (&mut self.right, &mut self.names[1]),
            };
            operand.set_moved(text);
            *name = None;
            self.moved = Some(moved.into);
        }
        self.viewer.reset();
        if self.page == Page::Documents && !cancelled {
            self.page = Page::SideBySide;
        }
        // On to the difference that has taken the place of the one that was
        // moved (the last one, if that was the last).
        let view = self.run.outcome().and_then(|c| c.view.as_ref().ok());
        if let (Some((block, _)), Some(view)) = (moving, view) {
            if !view.blocks.is_empty() {
                self.viewer.pick(view, block.min(view.blocks.len() - 1));
            }
        }
    }

    /// What was compared no longer matches what is on the page.
    fn invalidate(&mut self) {
        self.run.clear();
        self.viewer.reset();
        self.page = Page::Documents;
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
        // A difference of the view to move, and to which side.
        let mut take: Option<(usize, Side)> = None;
        let picked = self.viewer.current();
        let patch = self.run.outcome().map(|c| c.patch.clone());
        // Whether Previous and Next have anywhere to go: none when the view is
        // not the one on show.
        let steps = self
            .run
            .outcome()
            .and_then(|c| c.view.as_ref().ok())
            .filter(|_| self.page == Page::SideBySide)
            .map(|view| [false, true].map(|forward| self.viewer.can_step(view, forward)));
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
                    let free = picked.filter(|_| !self.run.running());
                    for (label, into, tip) in [
                        (
                            "⏴",
                            Side::Left,
                            "Move the difference to the left: Left takes what Right has there \
                             (Alt+Left)",
                        ),
                        (
                            "⏵",
                            Side::Right,
                            "Move the difference to the right: Right takes what Left has there \
                             (Alt+Right)",
                        ),
                    ] {
                        let button = ui
                            .add_enabled(free.is_some(), egui::Button::new(label))
                            .on_hover_text(tip)
                            .on_disabled_hover_text(
                                "Pick a difference to move first: click it, or use Previous and \
                                 Next",
                            );
                        if button.clicked() {
                            take = free.map(|block| (block, into));
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
        if steps.is_some() && !swap {
            if ui.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowDown)) {
                step = Some(true);
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowUp)) {
                step = Some(false);
            }
            if let Some(block) = picked.filter(|_| !self.run.running()) {
                for (key, into) in [(Key::ArrowLeft, Side::Left), (Key::ArrowRight, Side::Right)] {
                    if ui.input_mut(|i| i.consume_key(Modifiers::ALT, key)) {
                        take = Some((block, into));
                    }
                }
            }
            let view = self.run.outcome().and_then(|c| c.view.as_ref().ok());
            if let (Some(forward), Some(view)) = (step, view) {
                self.viewer.step(view, forward);
            }
        }

        let copied_path = egui::CentralPanel::default()
            .show(ui, |ui| match self.page {
                Page::Documents => {
                    self.documents(ui, env);
                    None
                }
                _ => self.result(ui, &mut take),
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

        if let Some((block, into)) = take {
            // (Not while Compare has just been pressed: that one is asked for.)
            if request.is_none() {
                request = self.start_move(block, into);
            }
        }
        if let Some(side) = self.moved.take() {
            let to = match side {
                Side::Left => "left",
                Side::Right => "right",
            };
            env.shared.say(
                Tool::Diff,
                format!("Moved the difference to the {to}"),
                false,
            );
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
        self.names = [
            self.left.label().map(str::to_owned),
            self.right.label().map(str::to_owned),
        ];
        let (gen, cancel) = self.run.start();
        Some(Request::Job {
            tool: Tool::Diff,
            job: Job::Diff {
                left,
                right,
                take: None,
            },
            gen,
            cancel,
        })
    }

    /// Ask the worker to move the difference `block` of the view (counting
    /// them from the top) to the left or the right: it makes the new document,
    /// and compares again, and the answer puts the new text in the box.
    fn start_move(&mut self, block: usize, into: Side) -> Option<Request> {
        let view = self.run.outcome()?.view.as_ref().ok()?;
        let changes = view.changes_of(view.blocks.get(block)?);
        let (left, right) = (self.left.input()?, self.right.input()?);
        let (gen, cancel) = self.run.start();
        self.moving = Some((block, into));
        Some(Request::Job {
            tool: Tool::Diff,
            job: Job::Diff {
                left,
                right,
                take: Some(Take { changes, into }),
            },
            gen,
            cancel,
        })
    }

    /// The answer, on the view the page is on. Gives the path of a difference
    /// that was clicked, which the caller copies; `take` is set to a difference
    /// that was asked to be moved, from its menu.
    fn result(&mut self, ui: &mut egui::Ui, take: &mut Option<(usize, Side)>) -> Option<String> {
        let page = self.page;
        let names = [self.names[0].as_deref(), self.names[1].as_deref()];
        let mut copied_path = None;
        result_area(
            ui,
            "diff",
            &self.run,
            "What differs appears here",
            |ui, compared| match (page, &compared.view) {
                (Page::SideBySide, Ok(view)) => {
                    let shown = self.viewer.show(ui, view, names);
                    copied_path = shown.clicked;
                    if shown.moved.is_some() {
                        *take = shown.moved;
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
    /// which difference the side-by-side view is on, and how long it took. False
    /// when there is nothing to say.
    pub(super) fn status_line(&self, ui: &mut egui::Ui) -> bool {
        let Some(compared) = self.run.outcome() else {
            return false;
        };
        ui.label(counts(compared));
        if self.page == Page::SideBySide {
            if let Some(position) = compared
                .view
                .as_ref()
                .ok()
                .and_then(|view| self.viewer.position(view))
            {
                ui.separator();
                ui.label(position);
            }
        }
        ui.separator();
        ui.weak(format!("Compared in {:.1?}", compared.elapsed));
        true
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
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::tools::jobs::Preview;
    use jsonquery_query::diff::SideBySide;

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
    fn the_names_of_the_documents_are_those_they_had_when_compared() {
        let mut diff = Diff::default();
        diff.left.set_text("[1]");
        diff.right.set_text("[2]");
        assert!(diff.start().is_some());
        assert_eq!(diff.names, [None, None], "typed text has no name");
    }
}
