//! Merge JSON: combine several files into one document by running a jq filter
//! over them — `jq -s` does the same on a command line (see
//! `jsonquery_query::merge`). The page only collects the files and the filter
//! and shows the outcome; the reading and merging happen on the worker thread
//! (`Command::Merge`), and the app does whatever the page asks of it through a
//! [`Request`].
//!
//! The command row has the Merge button, the ready-made filters and, under it,
//! the filter to edit, as the main window's Query row and box do; the left pane
//! is the files, the right pane the result, with what to do with it in its
//! header.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Layout, RichText};
use jsonquery_query::merge::{self, PRESETS};

use super::shared::{Env, Run};
use super::widgets::{
    command_bar, file_name, halves, header, preview_box, result_area, row_background, run_button,
    text_box, RIGHT_MIN, ROW_HEIGHT,
};
use super::Request;
use crate::app::human_bytes;
use crate::worker::{describe, MergeOutcome};

const MOVE_UP: &str = "⏶";
const MOVE_DOWN: &str = "⏷";
const REMOVE: &str = "×";

/// The Merge tool's state: the files, the filter and the last outcome.
pub(super) struct Merge {
    files: Vec<PathBuf>,
    filter: String,
    run: Run<MergeOutcome>,
}

impl Default for Merge {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            filter: PRESETS[0].filter.to_owned(),
            run: Run::default(),
        }
    }
}

enum RowAction {
    Up,
    Down,
    Remove,
}

impl Merge {
    /// Add `paths` to the end of the list, leaving out folders and files that
    /// are already in it (a file dropped twice is far likelier a slip than
    /// something that should be merged with itself).
    pub(super) fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let before = self.files.len();
        for path in paths {
            if !path.is_dir() && !self.files.contains(&path) {
                self.files.push(path);
            }
        }
        if self.files.len() != before {
            self.inputs_changed();
        }
    }

    /// What was merged no longer matches the files or the filter.
    fn inputs_changed(&mut self) {
        self.run.clear();
    }

    fn start(&mut self) -> Request {
        let (gen, cancel) = self.run.start();
        Request::Merge {
            paths: self.files.clone(),
            filter: self.filter.clone(),
            gen,
            cancel,
        }
    }

    /// The answer to a [`Request::Merge`]. One from a merge that has since
    /// been cancelled or replaced is dropped.
    pub(super) fn done(&mut self, gen: u64, result: Result<MergeOutcome, String>) {
        self.run.finish(gen, result);
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        if env.own_input {
            let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            });
            self.add(dropped);
        }

        let mut request = None;
        let tool_bytes = env.tool_bytes;
        command_bar(ui, "merge_command", |ui| request = self.command_ui(ui));
        let (left, right) = halves(ui, "merge_panes", RIGHT_MIN);
        left.show(ui, |ui| self.files_pane(ui, tool_bytes));
        right.show(ui, |ui| {
            if let Some(r) = self.result_pane(ui) {
                request = Some(r);
            }
        });
        request
    }

    /// The Merge button (while a merge runs, a spinner and Cancel), "Merge as"
    /// with the ready-made filters, and under them the filter itself.
    fn command_ui(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let busy = self.run.running();
        let mut request = None;
        let mut changed = false;
        ui.horizontal(|ui| {
            let ready = !self.files.is_empty() && !self.filter.trim().is_empty();
            if run_button(
                ui,
                &self.run,
                "Merge",
                ready,
                "Merge the files with the filter (Ctrl+Enter)",
            ) {
                request = Some(self.start());
            }
            ui.add_enabled_ui(!busy, |ui| changed |= self.preset_ui(ui));
            ui.add(
                egui::Label::new(
                    RichText::new(". is the files' contents, $files their names").weak(),
                )
                .truncate(),
            );
        });
        ui.add_enabled_ui(!busy, |ui| {
            changed |= text_box(
                ui,
                "merge_filter",
                &mut self.filter,
                "a jq filter, e.g. add",
                Some(3),
                false,
            );
        });
        ui.add_space(2.0);
        if changed {
            self.inputs_changed();
        }
        request
    }

    /// "Merge as": the ready-made filters. True when one was picked.
    fn preset_ui(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label("Merge as");
        let current = merge::preset_for(&self.filter);
        let mut changed = false;
        let combo = egui::ComboBox::from_id_salt("merge_preset")
            .selected_text(current.map_or("Custom jq filter", |p| p.label))
            .width(210.0)
            .show_ui(ui, |ui| {
                for preset in &PRESETS {
                    let selected = current.is_some_and(|c| c.filter == preset.filter);
                    if ui
                        .selectable_label(selected, preset.label)
                        .on_hover_text(preset.about)
                        .clicked()
                    {
                        self.filter = preset.filter.to_owned();
                        changed = true;
                    }
                }
            });
        combo
            .response
            .on_hover_text(current.map_or("Your own jq filter, written below.", |p| p.about));
        changed
    }

    /// The left pane: the files, in the order they will be merged.
    fn files_pane(&mut self, ui: &mut egui::Ui, tool_bytes: u64) {
        ui.add_enabled_ui(!self.run.running(), |ui| {
            self.files_header(ui, tool_bytes);
            self.files_list(ui);
        });
    }

    /// "Files (3)" with the buttons that change the list, at the right.
    fn files_header(&mut self, ui: &mut egui::Ui, tool_bytes: u64) {
        let title = match self.files.len() {
            0 => "Files".to_owned(),
            n => format!("Files ({n})"),
        };
        let has_files = !self.files.is_empty();
        header(
            ui,
            &title,
            |_| {},
            |ui| {
                // Right to left: Clear is the right-most.
                if has_files {
                    if ui
                        .button("Clear")
                        .on_hover_text("Remove every file from the list")
                        .clicked()
                    {
                        self.files.clear();
                        self.inputs_changed();
                    }
                    if ui
                        .button("Sort A–Z")
                        .on_hover_text(
                            "Order by file name, with numbers in order (part2 before part10)",
                        )
                        .clicked()
                    {
                        self.files.sort_by(|a, b| natural_path_cmp(a, b));
                        self.inputs_changed();
                    }
                }
                if ui.button("Add files…").clicked() {
                    if let Some(paths) = rfd::FileDialog::new()
                        .add_filter("JSON", &["json", "ndjson", "jsonl", "log", "txt"])
                        .pick_files()
                    {
                        self.add(paths);
                    }
                }
            },
        )
        .on_hover_text(format!(
            "Merged in the order listed. Merging happens in memory, so the files can add up to {}.",
            human_bytes(tool_bytes)
        ));
    }

    /// The list of files — or, while there are none, what to do about it.
    fn files_list(&mut self, ui: &mut egui::Ui) {
        let dragging = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        if self.files.is_empty() {
            if dragging {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.heading("Drop to add");
                });
            } else {
                ui.weak("Drop JSON files here, or use Add files…");
            }
            return;
        }

        // What each file turned out to be, once they have been merged.
        let details: Option<Vec<String>> = match self.run.outcome() {
            Some(o) if o.files.len() == self.files.len() => Some(
                o.files
                    .iter()
                    .map(|f| format!("{} · {}", f.shape, human_bytes(f.bytes)))
                    .collect(),
            ),
            _ => None,
        };

        let mut action = None;
        // Rows touch, so their highlights do. (Set before the scroll area, which
        // takes its idea of how tall a row is from this.)
        ui.spacing_mut().item_spacing.y = 0.0;
        let count = self.files.len();
        egui::ScrollArea::vertical()
            .id_salt("merge_files")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, count, |ui, rows| {
                for index in rows {
                    let detail = details.as_ref().map(|d| d[index].as_str());
                    if let Some(a) = file_row(ui, index, count, &self.files[index], detail) {
                        action = Some((index, a));
                    }
                }
            });

        if let Some((i, action)) = action {
            match action {
                RowAction::Up => self.files.swap(i, i - 1),
                RowAction::Down => self.files.swap(i, i + 1),
                RowAction::Remove => {
                    self.files.remove(i);
                }
            }
            self.inputs_changed();
        }
    }

    /// The right pane: the result, with Open in main window and Save… in the
    /// header — there from the start, usable once there is a result.
    fn result_pane(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        enum Act {
            Open,
            Save,
        }

        let doc = self.run.outcome().map(|outcome| outcome.doc.clone());
        let mut act = None;
        header(
            ui,
            "Result",
            |_| {},
            |ui| {
                // Right to left: Open in main window is the right-most.
                if ui
                    .add_enabled(doc.is_some(), egui::Button::new("Open in main window"))
                    .on_hover_text(
                        "Show the merged document in the main window, to explore and query it",
                    )
                    .clicked()
                {
                    act = Some(Act::Open);
                }
                if ui
                    .add_enabled(doc.is_some(), egui::Button::new("Save…"))
                    .on_hover_text("Write the merged document to a file")
                    .clicked()
                {
                    act = Some(Act::Save);
                }
            },
        );

        result_area(
            ui,
            "merge",
            &self.run,
            "The merged result appears here",
            |ui, outcome| preview_box(ui, "merge", &outcome.preview, outcome.preview_truncated),
        );

        let doc = doc?;
        match act? {
            Act::Open => Some(Request::Open(doc)),
            Act::Save => {
                let path = rfd::FileDialog::new()
                    .set_file_name("merged.json")
                    .add_filter("JSON", &["json"])
                    .save_file()?;
                Some(Request::Save { doc, path })
            }
        }
    }

    /// What the status bar says about the result: what it is and where it came
    /// from. False when there is nothing to say.
    pub(super) fn status_line(&self, ui: &mut egui::Ui) -> bool {
        let Some(outcome) = self.run.outcome() else {
            return false;
        };
        ui.label(result_summary(outcome));
        ui.separator();
        ui.weak(result_details(outcome));
        true
    }
}

/// What the result is, in a few words: "array · 6 items".
fn result_summary(outcome: &MergeOutcome) -> String {
    let mut summary = describe(outcome.doc.root());
    if outcome.outputs > 1 {
        summary.push_str(&format!(" · {} outputs", outcome.outputs));
    }
    summary
}

/// How it came about: "Merged 2 files (6 B) in 1.2ms".
fn result_details(outcome: &MergeOutcome) -> String {
    let files = outcome.files.len();
    format!(
        "Merged {files} file{} ({}) in {:.1?}",
        if files == 1 { "" } else { "s" },
        human_bytes(outcome.doc.byte_len),
        outcome.elapsed
    )
}

/// One row of the list of files: its position, its name (the whole path in the
/// tooltip), what it turned out to be once merged, and the buttons to move or
/// remove it — which only show their frame under the pointer.
fn file_row(
    ui: &mut egui::Ui,
    index: usize,
    count: usize,
    path: &Path,
    detail: Option<&str>,
) -> Option<RowAction> {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::hover(),
    );
    row_background(ui, rect, false, ui.rect_contains_pointer(rect));

    let mut action = None;
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(4.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.add_sized(
        [22.0, ROW_HEIGHT],
        egui::Label::new(RichText::new((index + 1).to_string()).weak()),
    );
    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if icon_button(ui, REMOVE, true, "Remove").clicked() {
            action = Some(RowAction::Remove);
        }
        if icon_button(ui, MOVE_DOWN, index + 1 < count, "Merge later").clicked() {
            action = Some(RowAction::Down);
        }
        if icon_button(ui, MOVE_UP, index > 0, "Merge earlier").clicked() {
            action = Some(RowAction::Up);
        }
        if let Some(detail) = detail {
            ui.weak(detail);
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(egui::Label::new(file_name(path)).truncate())
                .on_hover_text(path.display().to_string());
        });
    });
    action
}

/// A small button that is only a glyph until the pointer is over it.
fn icon_button(ui: &mut egui::Ui, glyph: &str, enabled: bool, tip: &str) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(glyph)
            .frame_when_inactive(false)
            .min_size(egui::vec2(18.0, 18.0)),
    );
    if enabled {
        response.on_hover_text(tip)
    } else {
        response
    }
}

/// File names in the order a person would list them: case-insensitive, and a
/// run of digits is one number (so `part2` comes before `part10`).
fn natural_path_cmp(a: &Path, b: &Path) -> Ordering {
    natural_cmp(&file_name(a), &file_name(b)).then_with(|| a.cmp(b))
}

fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
        let mut run = String::new();
        while let Some(&c) = chars.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            run.push(c);
            chars.next();
        }
        run
    }

    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (x, y) = (digits(&mut a), digits(&mut b));
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(order: &mut [&str]) -> Vec<String> {
        order.sort_by(|a, b| natural_cmp(a, b));
        order.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn numbers_in_names_sort_as_numbers() {
        assert_eq!(
            names(&mut ["part10.json", "part2.json", "part1.json"]),
            ["part1.json", "part2.json", "part10.json"]
        );
    }

    #[test]
    fn names_sort_without_regard_to_case() {
        assert_eq!(
            names(&mut ["b.json", "A.json", "c.json"]),
            ["A.json", "b.json", "c.json"]
        );
    }

    #[test]
    fn leading_zeros_and_huge_numbers_are_fine() {
        assert_eq!(
            names(&mut ["x9", "x007", "x99999999999999999999999", "x08"]),
            ["x007", "x08", "x9", "x99999999999999999999999"]
        );
    }

    #[test]
    fn a_file_added_twice_is_listed_once() {
        let mut merge = Merge::default();
        merge.add([
            PathBuf::from("/no/such/a.json"),
            PathBuf::from("/no/such/b.json"),
        ]);
        merge.add([PathBuf::from("/no/such/a.json")]);
        assert_eq!(merge.files.len(), 2);
    }

    #[test]
    fn a_folder_is_not_added() {
        let mut merge = Merge::default();
        merge.add([std::env::temp_dir()]);
        assert!(merge.files.is_empty());
    }

    #[test]
    fn changing_the_files_drops_the_old_result() {
        let mut merge = Merge::default();
        merge.run.result = Some(Err("old".to_owned()));
        merge.add([PathBuf::from("/no/such/a.json")]);
        assert!(merge.run.result.is_none());
    }

    #[test]
    fn an_answer_to_an_old_merge_is_dropped() {
        let mut merge = Merge::default();
        merge.add([PathBuf::from("/no/such/a.json")]);
        let Request::Merge { gen: first, .. } = merge.start() else {
            panic!("a merge was expected")
        };
        let Request::Merge { gen: second, .. } = merge.start() else {
            panic!("a merge was expected")
        };
        merge.done(first, Err("old".to_owned()));
        assert!(merge.run.running() && merge.run.result.is_none());
        merge.done(second, Err("new".to_owned()));
        assert!(!merge.run.running());
        assert!(matches!(&merge.run.result, Some(Err(e)) if e == "new"));
    }
}
