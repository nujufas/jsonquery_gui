//! Merge JSON: combine several files into one document by running a jq filter
//! over them — `jq -s` does the same on a command line (see
//! `jsonquery_query::merge`). The page only collects the files and the filter
//! and shows the outcome; the reading and merging happen on the worker thread
//! (`Command::Merge`), and the app does whatever the page asks of it through a
//! [`Request`].
//!
//! The left half is the files, with the filter and the Merge button pinned
//! under them; the right half is the result, with what to do with it pinned
//! under it.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;

use eframe::egui::{self, Align, Align2, FontId, Layout, RichText, TextWrapMode};
use jsonquery_query::merge::{self, PRESETS};

use super::shared::Env;
use super::widgets::{
    centered_hint, file_name, fill, flat_button, halves, heading, inset_box, notice_line, pinned,
    primary_button, secondary_button, Border, ACTION_HEIGHT, HEADER_HEIGHT,
};
use super::{Request, Tool};
use crate::app::human_bytes;
use crate::worker::{describe, MergeOutcome, MAX_MERGE_BYTES};

/// Height of a row in the list of files.
const FILE_ROW_HEIGHT: f32 = 30.0;
/// Heights (margin included) the pinned parts are given on the first frame,
/// before egui has measured them: the filter and Merge button under the files,
/// and the buttons under the result.
const OPTIONS_HEIGHT_GUESS: f32 = 214.0;
const RESULT_ACTIONS_HEIGHT_GUESS: f32 = 44.0;

const MOVE_UP: &str = "⏶";
const MOVE_DOWN: &str = "⏷";
const REMOVE: &str = "×";

/// The Merge tool's state: the files, the filter and the last outcome.
pub(super) struct Merge {
    files: Vec<PathBuf>,
    filter: String,
    /// Which merge the window is waiting on (or last got an answer for).
    gen: u64,
    /// The cancel flag of the merge in flight.
    running: Option<Arc<AtomicBool>>,
    result: Option<Result<MergeOutcome, String>>,
}

impl Default for Merge {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            filter: PRESETS[0].filter.to_owned(),
            gen: 0,
            running: None,
            result: None,
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
        self.result = None;
    }

    fn start(&mut self) -> Request {
        self.gen += 1;
        self.result = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.running = Some(cancel.clone());
        Request::Merge {
            paths: self.files.clone(),
            filter: self.filter.clone(),
            gen: self.gen,
            cancel,
        }
    }

    /// The answer to a [`Request::Merge`]. One from a merge that has since
    /// been cancelled or replaced is dropped.
    pub(super) fn done(&mut self, gen: u64, result: Result<MergeOutcome, String>) {
        if gen == self.gen {
            self.running = None;
            self.result = Some(result);
        }
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

        heading(ui, "Merge JSON files");

        let mut request = None;
        let (left, right) = halves(ui, "merge_inputs");
        left.show(ui, |ui| request = self.inputs_ui(ui));
        right.show(ui, |ui| {
            if let Some(r) = self.result_ui(ui, env) {
                request = Some(r);
            }
        });
        request
    }

    /// The left half: the files take the height there is, with the filter and
    /// the Merge button pinned under them.
    fn inputs_ui(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let busy = self.running.is_some();
        let mut request = None;

        pinned("merge_options", OPTIONS_HEIGHT_GUESS).show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| self.filter_ui(ui));
            ui.add_space(12.0);
            request = self.merge_row(ui);
        });

        fill().show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                self.files_header(ui);
                ui.add_space(6.0);
                self.files_box(ui);
            });
        });
        request
    }

    /// The Merge button, at the right; while a merge runs, its progress and Cancel.
    fn merge_row(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let mut request = None;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ACTION_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| match &self.running {
                Some(cancel) => {
                    if ui.add(secondary_button("Cancel")).clicked() {
                        cancel.store(true, AtomicOrdering::Relaxed);
                    }
                    ui.label("Merging…");
                    ui.spinner();
                }
                None => {
                    let ready = !self.files.is_empty() && !self.filter.trim().is_empty();
                    let clicked = ui
                        .add_enabled(ready, primary_button(ui, "Merge"))
                        .on_hover_text("Merge the files with the filter (Ctrl+Enter)")
                        .clicked();
                    let shortcut =
                        ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
                    if ready && (clicked || shortcut) {
                        request = Some(self.start());
                    }
                }
            },
        );
        request
    }

    /// "Files (3)" with the buttons that change the list, at the right.
    fn files_header(&mut self, ui: &mut egui::Ui) {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), HEADER_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| {
                let title = match self.files.len() {
                    0 => "Files".to_owned(),
                    n => format!("Files ({n})"),
                };
                ui.label(RichText::new(title).strong())
                    .on_hover_text(format!(
                        "Merged in the order listed. Merging happens in memory, so the files \
                     can add up to {}.",
                        human_bytes(MAX_MERGE_BYTES)
                    ));

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Right to left: Clear is the right-most.
                    if !self.files.is_empty() {
                        if flat_button(ui, "Clear")
                            .on_hover_text("Remove every file from the list")
                            .clicked()
                        {
                            self.files.clear();
                            self.inputs_changed();
                        }
                        if flat_button(ui, "Sort A–Z")
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
                });
            },
        );
    }

    /// The list of files — or, while there are none, a place to drop them.
    fn files_box(&mut self, ui: &mut egui::Ui) {
        let dragging = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let border = if dragging {
            Border::Accent
        } else if self.files.is_empty() {
            Border::Dashed
        } else {
            Border::Solid
        };

        // What each file turned out to be, once they have been merged.
        let details: Option<Vec<String>> = match &self.result {
            Some(Ok(o)) if o.files.len() == self.files.len() => Some(
                o.files
                    .iter()
                    .map(|f| format!("{} · {}", f.shape, human_bytes(f.bytes)))
                    .collect(),
            ),
            _ => None,
        };

        let mut action = None;
        inset_box(ui, border, 1.0, |ui| {
            if self.files.is_empty() {
                drop_hint(ui, dragging);
                return;
            }
            // Rows touch, so their stripes do.
            ui.spacing_mut().item_spacing.y = 0.0;
            let count = self.files.len();
            egui::ScrollArea::vertical()
                .id_salt("merge_files")
                .auto_shrink([false, false])
                .show_rows(ui, FILE_ROW_HEIGHT, count, |ui, rows| {
                    for index in rows {
                        let detail = details.as_ref().map(|d| d[index].as_str());
                        if let Some(a) = file_row(ui, index, count, &self.files[index], detail) {
                            action = Some((index, a));
                        }
                    }
                });
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

    /// "Merge as": the ready-made filters, and the filter itself to edit.
    fn filter_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Merge as").strong());
        ui.add_space(4.0);

        let current = merge::preset_for(&self.filter);
        let mut changed = false;
        let combo = egui::ComboBox::from_id_salt("merge_preset")
            .selected_text(current.map_or("Custom jq filter", |p| p.label))
            .width(ui.available_width())
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

        ui.add_space(6.0);
        changed |= ui
            .add(
                egui::TextEdit::multiline(&mut self.filter)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("a jq filter, e.g. add"),
            )
            .changed();
        ui.add_space(4.0);
        ui.label(
            RichText::new(". is the files' contents, $files their names")
                .small()
                .weak(),
        );

        if changed {
            self.inputs_changed();
        }
    }

    /// The right half: the result in a box that takes the height there is,
    /// with what to do with it pinned under it.
    fn result_ui(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        env.shared.expire(ui.ctx());
        let mut request = None;

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), HEADER_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(RichText::new("Result").strong());
                if let Some(Ok(outcome)) = &self.result {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(result_summary(outcome)).small().weak())
                            .on_hover_text(result_details(outcome));
                    });
                }
            },
        );
        ui.add_space(6.0);

        pinned("merge_result_actions", RESULT_ACTIONS_HEIGHT_GUESS)
            .show(ui, |ui| request = self.result_actions(ui, env));

        fill().show(ui, |ui| self.result_box(ui));
        request
    }

    fn result_box(&self, ui: &mut egui::Ui) {
        let error_color = ui.visuals().error_fg_color;
        inset_box(ui, Border::Solid, 10.0, |ui| match &self.result {
            Some(Ok(outcome)) => {
                egui::ScrollArea::both()
                    .id_salt("merge_preview")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&outcome.preview).monospace())
                                .wrap_mode(TextWrapMode::Extend)
                                .selectable(true),
                        );
                        if outcome.preview_truncated {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("Only the start of the document is shown.")
                                    .small()
                                    .weak(),
                            );
                        }
                    });
            }
            Some(Err(error)) if error == "cancelled" => centered_hint(ui, "Cancelled."),
            Some(Err(error)) => {
                egui::ScrollArea::vertical()
                    .id_salt("merge_error")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(error).color(error_color))
                                .wrap()
                                .selectable(true),
                        );
                    });
            }
            None if self.running.is_some() => centered_hint(ui, "Merging…"),
            None => centered_hint(ui, "The merged result appears here"),
        });
    }

    /// Open in main window and Save…, at the right — there from the start, but
    /// only usable once there is a result — and the line about the last save.
    fn result_actions(&self, ui: &mut egui::Ui, env: &Env) -> Option<Request> {
        enum Act {
            Open,
            Save,
        }

        let doc = match &self.result {
            Some(Ok(outcome)) => Some(outcome.doc.clone()),
            _ => None,
        };
        let mut act = None;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ACTION_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.add_enabled_ui(doc.is_some(), |ui| {
                    if ui
                        .add(primary_button(ui, "Open in main window"))
                        .on_hover_text(
                            "Show the merged document in the main window, to explore and query it",
                        )
                        .clicked()
                    {
                        act = Some(Act::Open);
                    }
                    if ui
                        .add(secondary_button("Save…"))
                        .on_hover_text("Write the merged document to a file")
                        .clicked()
                    {
                        act = Some(Act::Save);
                    }
                });
                notice_line(ui, env.shared.notice_for(Tool::Merge));
            },
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
}

/// What the result is, in a few words: "array · 6 items".
fn result_summary(outcome: &MergeOutcome) -> String {
    let mut summary = describe(&outcome.doc.root);
    if outcome.outputs > 1 {
        summary.push_str(&format!(" · {} outputs", outcome.outputs));
    }
    summary
}

/// The rest, for the tooltip over the summary.
fn result_details(outcome: &MergeOutcome) -> String {
    let files = outcome.files.len();
    let mut details = format!(
        "Merged {files} file{} ({}) in {:.1?}.",
        if files == 1 { "" } else { "s" },
        human_bytes(outcome.doc.byte_len),
        outcome.elapsed
    );
    if outcome.outputs > 1 {
        details.push_str(&format!(
            "\nThe filter gave {} outputs; they are collected in an array.",
            outcome.outputs
        ));
    }
    details
}

/// What the empty list of files says, in the middle of its box.
fn drop_hint(ui: &mut egui::Ui, dragging: bool) {
    let rect = ui.max_rect();
    let visuals = ui.visuals();
    let (strong, weak) = (visuals.text_color(), visuals.weak_text_color());
    let painter = ui.painter();
    if dragging {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Drop to add",
            FontId::proportional(16.0),
            strong,
        );
    } else {
        painter.text(
            rect.center() - egui::vec2(0.0, 11.0),
            Align2::CENTER_CENTER,
            "Drop JSON files here",
            FontId::proportional(16.0),
            strong,
        );
        painter.text(
            rect.center() + egui::vec2(0.0, 13.0),
            Align2::CENTER_CENTER,
            "or use Add files…",
            FontId::proportional(13.0),
            weak,
        );
    }
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
        egui::vec2(ui.available_width(), FILE_ROW_HEIGHT),
        egui::Sense::hover(),
    );
    let visuals = ui.visuals();
    let fill = if ui.rect_contains_pointer(rect) {
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

    let mut action = None;
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(10.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    row.add_sized(
        [22.0, FILE_ROW_HEIGHT],
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
            ui.label(RichText::new(detail).small().weak());
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
        egui::Button::new(RichText::new(glyph).size(12.0))
            .frame_when_inactive(false)
            .min_size(egui::vec2(22.0, 22.0)),
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
        let mut merge = Merge {
            result: Some(Err("old".to_owned())),
            ..Merge::default()
        };
        merge.add([PathBuf::from("/no/such/a.json")]);
        assert!(merge.result.is_none());
    }
}
