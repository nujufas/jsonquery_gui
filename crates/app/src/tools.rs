//! The Tools window: a second native window (an egui *viewport*, like the
//! tutorial) that holds the small utilities that go with the main viewer. Today
//! that is Merge JSON; formatting and diff/merge have their places in the list
//! already.
//!
//! Merge JSON combines several files into one document by running a jq filter
//! over them — `jq -s` does the same on a command line (see
//! `jsonquery_query::merge`). The window only collects the files and the filter
//! and shows the outcome; the reading and merging happen on the worker thread
//! (`Command::Merge`), and the app does whatever the window asks of it through
//! a [`Request`], the way it does for the tutorial.
//!
//! The page is two halves, the files and the filter on the left and the result
//! on the right. Each has a title row on top, a box that takes all the height
//! there is, and its buttons pinned at the bottom, so it looks the same at any
//! window size (the pinned parts are bottom panels, whose height egui takes from
//! their content). Explanations are tooltips rather than paragraphs.
//!
//! It is an *immediate* viewport, so it runs inside the main window's frame with
//! plain `&mut self` access, and egui falls back to an embedded floating window
//! when the backend can't open native ones.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, RichText, TextWrapMode};
use jsonquery_core::Document;
use jsonquery_query::merge::{self, PRESETS};

use crate::app::human_bytes;
use crate::worker::{describe, MergeOutcome, MAX_MERGE_BYTES};

/// How long a line like "Saved to …" stays beside the result's buttons.
const NOTICE_FOR: Duration = Duration::from_secs(5);

/// Width of the list of tools, on the left.
const SIDEBAR_WIDTH: f32 = 168.0;
/// Space between the window's edge and the tool's page.
const PAGE_MARGIN: i8 = 18;
/// Space on either side of the line between the two halves of the Merge page.
const GUTTER: i8 = 16;
/// Height of an entry in the list of tools, and of a row in the list of files.
const TOOL_ROW_HEIGHT: f32 = 30.0;
const FILE_ROW_HEIGHT: f32 = 30.0;
/// Height of the title row over each half of the Merge page, the same on both
/// so the boxes under them line up.
const HEADER_HEIGHT: f32 = 26.0;
/// Height of the row of buttons under each half.
const ACTION_HEIGHT: f32 = 28.0;
/// Space between a box and what is pinned under it.
const BOX_GAP: i8 = 14;
/// Heights (margin included) the pinned parts are given on the first frame,
/// before egui has measured them: the filter and Merge button under the files,
/// and the buttons under the result.
const OPTIONS_HEIGHT_GUESS: f32 = 214.0;
const RESULT_ACTIONS_HEIGHT_GUESS: f32 = 44.0;
const BOX_RADIUS: u8 = 6;

const MOVE_UP: &str = "⏶";
const MOVE_DOWN: &str = "⏷";
const REMOVE: &str = "×";

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
    /// Show the merged document in the main window.
    Open(Arc<Document>),
    /// Write the merged document to `path`.
    Save { doc: Arc<Document>, path: PathBuf },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Merge,
    Format,
    Diff,
}

impl Tool {
    const ALL: [Tool; 3] = [Tool::Merge, Tool::Format, Tool::Diff];

    fn label(self) -> &'static str {
        match self {
            Tool::Merge => "Merge JSON",
            Tool::Format => "Format JSON",
            Tool::Diff => "Diff & merge",
        }
    }

    /// Whether the tool exists yet; the others are listed so it is clear
    /// where they will be.
    fn ready(self) -> bool {
        self == Tool::Merge
    }

    fn coming(self) -> &'static str {
        match self {
            Tool::Merge => "",
            Tool::Format => "Coming soon: pretty-print, minify and sort the keys of a document.",
            Tool::Diff => "Coming soon: compare two documents and merge their differences.",
        }
    }
}

pub struct Tools {
    open: bool,
    tool: Tool,
    merge: Merge,
    icon: Option<Arc<egui::IconData>>,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            open: false,
            tool: Tool::Merge,
            merge: Merge::default(),
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
        if gen == self.merge.gen {
            self.merge.running = None;
            self.merge.result = Some(result);
        }
    }

    /// A save finished; says so if it was one this window asked for.
    pub fn saved(&mut self, path: &Path) {
        if self.merge.saving.as_deref() == Some(path) {
            self.merge.saving = None;
            self.merge
                .say(format!("Saved to {}", path.display()), false);
        }
    }

    /// A save failed; says so if it was one this window asked for.
    pub fn save_failed(&mut self, error: &str) {
        if self.merge.saving.take().is_some() {
            self.merge.say(format!("Could not save: {error}"), true);
        }
    }

    /// Draw the window (if open) for this frame. Returns what it asks the app
    /// to do, if anything.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Request> {
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
            .with_min_inner_size([820.0, 520.0])
            .with_app_id(crate::APP_ID)
            .with_icon(icon);

        let mut request = None;
        let mut close = false;
        ctx.show_viewport_immediate(viewport_id(), builder, |ui, class| {
            close = ui.ctx().input(|i| i.viewport().close_requested());
            // An embedded window shares the main window's input, and so its
            // dropped files — which the main window has already taken.
            let own_input = class != egui::ViewportClass::EmbeddedWindow;
            request = self.contents(ui, own_input);
        });
        if close {
            self.open = false;
        }
        request
    }

    fn contents(&mut self, ui: &mut egui::Ui, own_input: bool) -> Option<Request> {
        let mut request = None;

        let sidebar = egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::same(10));
        egui::Panel::left("tools_list")
            .resizable(false)
            .exact_size(SIDEBAR_WIDTH)
            .frame(sidebar)
            .show(ui, |ui| self.tool_list(ui));

        let page =
            egui::Frame::central_panel(ui.style()).inner_margin(egui::Margin::same(PAGE_MARGIN));
        egui::CentralPanel::default()
            .frame(page)
            .show(ui, |ui| match self.tool {
                Tool::Merge => request = self.merge.ui(ui, own_input),
                other => {
                    ui.label(RichText::new(other.label()).heading().strong());
                    ui.add_space(6.0);
                    ui.weak(other.coming());
                }
            });

        if matches!(request, Some(Request::Save { .. })) {
            self.merge.say("Saving…", false);
        }
        request
    }

    fn tool_list(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 2.0;
        for tool in Tool::ALL {
            let ready = tool.ready();
            let row = tool_row(
                ui,
                tool.label(),
                self.tool == tool,
                (!ready).then_some("soon"),
            );
            if ready {
                if row.clicked() {
                    self.tool = tool;
                }
            } else {
                row.on_hover_text(tool.coming());
            }
        }
    }
}

/// A line beside the result's buttons ("Saved to …"), shown for a few seconds.
struct Notice {
    text: String,
    error: bool,
    at: Instant,
}

/// The Merge tool's state: the files, the filter and the last outcome.
struct Merge {
    files: Vec<PathBuf>,
    filter: String,
    /// Which merge the window is waiting on (or last got an answer for).
    gen: u64,
    /// The cancel flag of the merge in flight.
    running: Option<Arc<AtomicBool>>,
    result: Option<Result<MergeOutcome, String>>,
    /// Where a save this window asked for is going.
    saving: Option<PathBuf>,
    notice: Option<Notice>,
}

impl Default for Merge {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            filter: PRESETS[0].filter.to_owned(),
            gen: 0,
            running: None,
            result: None,
            saving: None,
            notice: None,
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
    fn add(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
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

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    fn expire_notice(&mut self, ctx: &egui::Context) {
        let expired = self
            .notice
            .as_ref()
            .is_some_and(|n| n.at.elapsed() >= NOTICE_FOR);
        if expired {
            self.notice = None;
        } else if self.notice.is_some() {
            ctx.request_repaint_after(NOTICE_FOR);
        }
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

    fn ui(&mut self, ui: &mut egui::Ui, own_input: bool) -> Option<Request> {
        if own_input {
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

        ui.label(RichText::new("Merge JSON files").heading().strong());
        ui.add_space(14.0);

        // Two halves with a line between them: the panels give the line, and
        // the margins on the sides facing each other the space around it.
        let half = (ui.available_width() / 2.0).floor();
        let mut request = None;
        egui::Panel::left("merge_inputs")
            .resizable(false)
            .exact_size(half)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                right: GUTTER,
                ..egui::Margin::ZERO
            }))
            .show(ui, |ui| request = self.inputs_ui(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                left: GUTTER,
                ..egui::Margin::ZERO
            }))
            .show(ui, |ui| {
                if let Some(r) = self.result_ui(ui) {
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

        egui::Panel::bottom("merge_options")
            .resizable(false)
            .default_size(OPTIONS_HEIGHT_GUESS)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                top: BOX_GAP,
                ..egui::Margin::ZERO
            }))
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| self.filter_ui(ui));
                ui.add_space(12.0);
                request = self.merge_row(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
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
    fn result_ui(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        self.expire_notice(ui.ctx());
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

        egui::Panel::bottom("merge_result_actions")
            .resizable(false)
            .default_size(RESULT_ACTIONS_HEIGHT_GUESS)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                top: BOX_GAP,
                ..egui::Margin::ZERO
            }))
            .show(ui, |ui| request = self.result_actions(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.result_box(ui));
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
    fn result_actions(&mut self, ui: &mut egui::Ui) -> Option<Request> {
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
                if let Some(notice) = &self.notice {
                    let color = if notice.error {
                        ui.visuals().error_fg_color
                    } else {
                        ui.visuals().hyperlink_color
                    };
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&notice.text).small().color(color))
                                .truncate(),
                        )
                        .on_hover_text(&notice.text);
                    });
                }
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
                self.saving = Some(path.clone());
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

/// One entry of the list of tools: a full-width row, filled when it is the
/// selected one. An entry that isn't there yet (it has a `tag`) is dimmed.
fn tool_row(ui: &mut egui::Ui, label: &str, selected: bool, tag: Option<&str>) -> egui::Response {
    let enabled = tag.is_none();
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), TOOL_ROW_HEIGHT), sense);

    let visuals = ui.visuals();
    let weak = visuals.weak_text_color();
    let (fill, text) = if selected {
        (visuals.selection.bg_fill, visuals.selection.stroke.color)
    } else if enabled && response.hovered() {
        (visuals.widgets.hovered.weak_bg_fill, visuals.text_color())
    } else if enabled {
        (Color32::TRANSPARENT, visuals.text_color())
    } else {
        (Color32::TRANSPARENT, weak)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(BOX_RADIUS), fill);
    painter.text(
        rect.left_center() + egui::vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        text,
    );
    if let Some(tag) = tag {
        painter.text(
            rect.right_center() - egui::vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            tag,
            egui::TextStyle::Small.resolve(ui.style()),
            weak,
        );
    }

    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// How the edge of a box is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Border {
    Solid,
    /// A place to drop things on.
    Dashed,
    /// Something is being dragged over it.
    Accent,
}

/// A rounded box that fills all the room there is, with `margin` between its
/// edge and what it holds — the look of the list of files and of the result.
fn inset_box(
    ui: &mut egui::Ui,
    border: Border,
    margin: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let visuals = ui.visuals().clone();
    let radius = egui::CornerRadius::same(BOX_RADIUS);

    let painter = ui.painter();
    match border {
        Border::Solid => {
            painter.rect(
                rect,
                radius,
                visuals.extreme_bg_color,
                visuals.widgets.noninteractive.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
        Border::Dashed => {
            painter.rect_filled(rect, radius, visuals.extreme_bg_color);
            let stroke = egui::Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.7));
            let corners = [
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
                rect.left_top(),
            ];
            painter.extend(egui::Shape::dashed_line(&corners, stroke, 6.0, 4.0));
        }
        Border::Accent => {
            painter.rect(
                rect,
                radius,
                visuals.hyperlink_color.gamma_multiply(0.12),
                egui::Stroke::new(1.5, visuals.hyperlink_color),
                egui::StrokeKind::Inside,
            );
        }
    }

    let inner_rect = rect.shrink(margin);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    inner.set_clip_rect(inner_rect.intersect(ui.clip_rect()));
    add_contents(&mut inner);
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

/// A quiet line in the middle of an empty box.
fn centered_hint(ui: &mut egui::Ui, text: &str) {
    let rect = ui.max_rect();
    let color = ui.visuals().weak_text_color();
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(14.0),
        color,
    );
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

/// A button for something the list can do but rarely needs to: quiet text.
fn flat_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let color = ui.visuals().weak_text_color();
    ui.add(egui::Button::new(RichText::new(text).color(color)).frame_when_inactive(false))
}

/// The button for what a half of the page is for, in the selection colour.
fn primary_button(ui: &egui::Ui, text: &str) -> egui::Button<'static> {
    let visuals = ui.visuals();
    egui::Button::new(
        RichText::new(text)
            .strong()
            .color(visuals.selection.stroke.color),
    )
    .fill(visuals.selection.bg_fill)
    .min_size(egui::vec2(88.0, ACTION_HEIGHT))
}

fn secondary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text)).min_size(egui::vec2(72.0, ACTION_HEIGHT))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
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

/// Small, dimmed 🛠 button that opens the Tools window (or brings it to the
/// front if it's already open) — the same single-icon-plus-hover-text shape as
/// the tutorial and autocomplete buttons beside it.
pub fn button(ui: &mut egui::Ui, tools: &mut Tools) {
    let icon = RichText::new("🛠").color(ui.visuals().weak_text_color());
    if ui
        .button(icon)
        .on_hover_text("Tools — merge JSON files with jq (more to come).\nDrop several files on the window to merge them.")
        .clicked()
    {
        tools.open_or_focus(ui.ctx());
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
