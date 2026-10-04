//! The box a tool's JSON goes in. Four ways in: type or paste it; drop a file on
//! the window; "Open file…"; or "Open document", the document open in the main
//! window. A file that fits is read into the box, where it can be edited; one too
//! big for a text box (see [`TEXT_LIMIT`]) is not shown but read when the tool
//! runs, and the open document is shared rather than copied.
//!
//! Format uses one box; Diff, Patch and Validate use two.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align, Layout, RichText};
use jsonquery_core::{Document, DocumentSource};

use super::jobs::{Input, MAX_TOOL_BYTES};
use super::widgets::{
    centered_pair, file_name, flat_button, inset_box, title_row, Border, BOX_GAP,
};
use crate::app::human_bytes;

/// The most text the box takes. egui lays out a whole text box at once, so a
/// megabyte is about as much as it can edit without stalling; a bigger file is
/// kept as a path instead (up to what the tools can handle — see
/// `jobs::MAX_TOOL_BYTES`).
pub(super) const TEXT_LIMIT: u64 = 1024 * 1024;

enum Content {
    /// Whatever is in the text box.
    Text,
    /// A file too big to show: read when the tool runs.
    Big { path: PathBuf, bytes: u64 },
    /// The document open in the main window, as it was when it was picked.
    Open(Arc<Document>),
}

pub(super) struct Operand {
    /// The title over the box: "Before", "Schema"…
    title: &'static str,
    /// Tells this box from the others on the page, for widget ids.
    id: &'static str,
    /// What the empty box says.
    hint: &'static str,
    text: String,
    content: Content,
    /// What the box holds, as the title row says it ("orders.json"): the file
    /// the text was read from, or the open document; until the text is edited.
    name: Option<String>,
    /// The file that was read, if one was (and not an open document that has no
    /// file): what a save of the result is named after.
    file: Option<String>,
    /// Why the last file could not be used.
    problem: Option<String>,
}

impl Operand {
    pub fn new(title: &'static str, id: &'static str, hint: &'static str) -> Self {
        Self {
            title,
            id,
            hint,
            text: String::new(),
            content: Content::Text,
            name: None,
            file: None,
            problem: None,
        }
    }

    /// Nothing to work on yet.
    pub fn is_empty(&self) -> bool {
        matches!(self.content, Content::Text) && self.text.trim().is_empty()
    }

    /// The document as a job takes it; none while the box is empty.
    pub fn input(&self) -> Option<Input> {
        match &self.content {
            Content::Text if self.text.trim().is_empty() => None,
            Content::Text => Some(Input::Text(self.text.clone())),
            Content::Big { path, .. } => Some(Input::File(path.clone())),
            Content::Open(doc) => Some(Input::Document(doc.clone())),
        }
    }

    /// The name of the file this was read from, if it was read from one: for
    /// a default file name.
    pub fn source_file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.content = Content::Text;
        self.name = None;
        self.file = None;
        self.problem = None;
    }

    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[cfg(test)]
    pub fn title(&self) -> &'static str {
        self.title
    }

    pub fn clear(&mut self) {
        self.set_text(String::new());
    }

    /// Put the file in the box — its text, if it is small enough to edit, else
    /// just where it is. If it can't be read the box is left as it was, and
    /// says why.
    pub fn load_file(&mut self, path: &Path) {
        let name = file_name(path);
        let bytes = match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() => meta.len(),
            Ok(_) => {
                self.problem = Some(format!("{name} is not a file"));
                return;
            }
            Err(e) => {
                self.problem = Some(format!("Could not read {name}: {e}"));
                return;
            }
        };
        if bytes > TEXT_LIMIT {
            self.text.clear();
            self.content = Content::Big {
                path: path.to_owned(),
                bytes,
            };
            self.name = Some(name.clone());
            self.file = Some(name);
            self.problem = None;
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                // A byte-order mark is not JSON.
                let text = match text.strip_prefix('\u{feff}') {
                    Some(rest) => rest.to_owned(),
                    None => text,
                };
                self.set_text(text);
                self.name = Some(name.clone());
                self.file = Some(name);
            }
            Err(e) => self.problem = Some(format!("Could not read {name}: {e}")),
        }
    }

    /// Use the document open in the main window. One that the tools would only
    /// refuse (see [`MAX_TOOL_BYTES`]) is not taken — the box says so and stays
    /// as it was — rather than keep a huge document alive for nothing.
    pub fn use_document(&mut self, doc: Arc<Document>) {
        if doc.byte_len > MAX_TOOL_BYTES {
            self.problem = Some(format!(
                "The open document is {}, more than the {} these tools take",
                human_bytes(doc.byte_len),
                human_bytes(MAX_TOOL_BYTES)
            ));
            return;
        }
        self.text.clear();
        self.name = Some(short_source(&doc));
        self.file = match &doc.source {
            DocumentSource::File(path) => Some(file_name(path)),
            _ => None,
        };
        self.content = Content::Open(doc);
        self.problem = None;
    }

    /// Exchange what two boxes hold (not their titles).
    pub fn swap_with(&mut self, other: &mut Operand) {
        std::mem::swap(&mut self.text, &mut other.text);
        std::mem::swap(&mut self.content, &mut other.content);
        std::mem::swap(&mut self.name, &mut other.name);
        std::mem::swap(&mut self.file, &mut other.file);
        std::mem::swap(&mut self.problem, &mut other.problem);
    }

    /// Draw the title row and the box, which fill the height there is. True
    /// when what the box holds changed. `drop_target` says a dragged file would
    /// land here, which the box shows.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        open_doc: Option<&Arc<Document>>,
        drop_target: bool,
    ) -> bool {
        let mut changed = self.header(ui, open_doc);
        ui.add_space(6.0);

        let dragging = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let border = if dragging && drop_target {
            Border::Accent
        } else {
            Border::Solid
        };
        let mut edited = false;
        inset_box(ui, border, 8.0, |ui| match &self.content {
            Content::Text => edited = text_edit(ui, &mut self.text, self.id, self.hint),
            Content::Big { path, bytes } => centered_pair(
                ui,
                &file_name(path),
                &format!("{} · read when you run it", human_bytes(*bytes)),
            ),
            Content::Open(doc) => centered_pair(
                ui,
                "The open document",
                &format!("{} · {}", short_source(doc), human_bytes(doc.byte_len)),
            ),
        });
        if edited {
            self.name = None;
            self.file = None;
            self.problem = None;
            changed = true;
        }
        changed
    }

    /// The title, what the box holds, and the buttons to fill or empty it.
    fn header(&mut self, ui: &mut egui::Ui, open_doc: Option<&Arc<Document>>) -> bool {
        let mut changed = false;
        title_row(ui, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Right to left: Clear is the right-most.
                if !self.is_empty() && flat_button(ui, "Clear").clicked() {
                    self.clear();
                    changed = true;
                }
                let open = ui
                    .add_enabled_ui(open_doc.is_some(), |ui| {
                        ui.button("Open document")
                            .on_hover_text("Use the document open in the main window")
                            .on_disabled_hover_text("No document is open in the main window")
                    })
                    .inner;
                if open.clicked() {
                    if let Some(doc) = open_doc {
                        self.use_document(doc.clone());
                        changed = true;
                    }
                }
                if ui
                    .button("Open file…")
                    .on_hover_text("Read a JSON file into this box")
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("JSON", &["json", "ndjson", "jsonl", "log", "txt"])
                        .pick_file()
                    {
                        self.load_file(&path);
                        changed = true;
                    }
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.label(RichText::new(self.title).strong());
                    if let Some(problem) = &self.problem {
                        let color = ui.visuals().error_fg_color;
                        ui.add(
                            egui::Label::new(RichText::new(problem).small().color(color))
                                .truncate(),
                        )
                        .on_hover_text(problem);
                    } else if let Some(name) = &self.name {
                        ui.add(egui::Label::new(RichText::new(name).small().weak()).truncate())
                            .on_hover_text(name);
                    }
                });
            });
        });
        changed
    }
}

/// A multi-line monospace text box that fills the room it is given and scrolls
/// when its text is longer. True when the text changed.
fn text_edit(ui: &mut egui::Ui, text: &mut String, id: &str, hint: &str) -> bool {
    // The height of a line as the text box counts it.
    let row =
        ui.text_style_height(&egui::TextStyle::Monospace) + ui.spacing().extra_text_line_spacing;
    let rows = (ui.available_height() / row).floor().max(1.0) as usize;
    egui::ScrollArea::vertical()
        .id_salt(format!("{id}_scroll"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(text)
                    .id_salt(format!("{id}_edit"))
                    .font(egui::TextStyle::Monospace)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .desired_rows(rows)
                    .hint_text(hint),
            )
            .changed()
        })
        .inner
}

/// A document's source in a few words: a file by its name, others as they
/// label themselves.
fn short_source(doc: &Document) -> String {
    match &doc.source {
        DocumentSource::File(path) => file_name(path),
        other => other.label(),
    }
}

/// Two boxes, one above the other, sharing the height there is. True when
/// either changed.
pub(super) fn stacked(
    ui: &mut egui::Ui,
    open_doc: Option<&Arc<Document>>,
    first: &mut Operand,
    second: &mut Operand,
) -> bool {
    let target = drop_target(&[&*first, &*second]);
    // Besides the gap, egui puts its item spacing after the first box.
    let spaces = f32::from(BOX_GAP) + ui.spacing().item_spacing.y;
    let height = ((ui.available_height() - spaces) / 2.0).floor().max(0.0);
    let size = egui::vec2(ui.available_width(), height);
    let mut changed = false;
    ui.allocate_ui(size, |ui| {
        changed |= first.ui(ui, open_doc, target == 0);
    });
    ui.add_space(f32::from(BOX_GAP));
    ui.allocate_ui(size, |ui| {
        changed |= second.ui(ui, open_doc, target == 1);
    });
    changed
}

/// The files dropped on the window this frame.
pub(super) fn dropped_files(ctx: &egui::Context) -> Vec<PathBuf> {
    ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .collect()
    })
}

/// Which of a page's boxes a dropped file goes to: the first one that is
/// empty, or the last when none is.
pub(super) fn drop_target(boxes: &[&Operand]) -> usize {
    target_of(boxes.iter().map(|b| b.is_empty()), boxes.len())
}

fn target_of(mut empty: impl Iterator<Item = bool>, count: usize) -> usize {
    empty
        .position(|is_empty| is_empty)
        .unwrap_or(count.saturating_sub(1))
}

/// Put dropped files in a page's boxes, one each as [`drop_target`] says. True
/// when there was anything to put.
pub(super) fn deliver(boxes: &mut [&mut Operand], paths: Vec<PathBuf>) -> bool {
    let mut any = false;
    for path in paths {
        let at = target_of(boxes.iter().map(|b| b.is_empty()), boxes.len());
        if let Some(target) = boxes.get_mut(at) {
            target.load_file(&path);
            any = true;
        }
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jsonquery-operand-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn operand() -> Operand {
        Operand::new("Document", "t", "hint")
    }

    #[test]
    fn a_box_with_only_white_space_is_empty() {
        let mut o = operand();
        assert!(o.is_empty() && o.input().is_none());
        o.set_text("  \n ");
        assert!(o.is_empty() && o.input().is_none());
        o.set_text("[1]");
        assert!(!o.is_empty());
        assert!(matches!(o.input(), Some(Input::Text(t)) if t == "[1]"));
    }

    #[test]
    fn a_small_file_is_read_into_the_box() {
        let dir = temp_dir("small");
        let path = dir.join("a.json");
        std::fs::write(&path, "\u{feff}[1, 2]").unwrap();
        let mut o = operand();
        o.load_file(&path);
        assert_eq!(o.text(), "[1, 2]", "the byte-order mark is dropped");
        assert_eq!(o.source_file(), Some("a.json"));
        assert!(o.problem.is_none());
    }

    #[test]
    fn a_big_file_is_kept_as_a_path() {
        let dir = temp_dir("big");
        let path = dir.join("big.json");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(TEXT_LIMIT + 1)
            .unwrap();
        let mut o = operand();
        o.set_text("old");
        o.load_file(&path);
        assert!(o.text().is_empty());
        assert!(matches!(o.input(), Some(Input::File(p)) if p == path));
        assert!(!o.is_empty());
    }

    #[test]
    fn a_file_that_cannot_be_read_leaves_the_box_as_it_was() {
        let mut o = operand();
        o.set_text("[1]");
        o.load_file(&temp_dir("none").join("nope.json"));
        assert_eq!(o.text(), "[1]");
        assert!(o
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("nope.json")));

        let dir = temp_dir("binary");
        let path = dir.join("b.bin");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        o.load_file(&path);
        assert_eq!(o.text(), "[1]");
        assert!(o.problem.is_some());

        o.load_file(&dir);
        assert!(o
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("not a file")));
    }

    #[test]
    fn the_open_document_is_shared_not_copied() {
        let doc = Arc::new(jsonquery_core::load_text("[1]").unwrap());
        let mut o = operand();
        o.use_document(doc.clone());
        assert!(!o.is_empty());
        assert!(matches!(o.input(), Some(Input::Document(d)) if Arc::ptr_eq(&d, &doc)));
        o.clear();
        assert!(o.is_empty() && o.input().is_none());
    }

    #[test]
    fn only_a_file_gives_a_name_for_a_save() {
        let mut o = operand();
        o.use_document(Arc::new(jsonquery_core::load_text("[1]").unwrap()));
        assert_eq!(
            o.name.as_deref(),
            Some("(pasted JSON)"),
            "shown in the title row"
        );
        assert_eq!(o.source_file(), None, "but it is not a file name");

        let dir = temp_dir("open-file");
        let path = dir.join("orders.json");
        std::fs::write(&path, "[1]").unwrap();
        let doc = jsonquery_core::load(&path).unwrap();
        o.use_document(Arc::new(doc));
        assert_eq!(o.source_file(), Some("orders.json"));

        // Typing over it makes it text again, which came from no file.
        o.set_text("[2]");
        assert_eq!(o.source_file(), None);
    }

    #[test]
    fn a_document_the_tools_would_refuse_is_not_held() {
        let big = Arc::new(Document::from_value(
            serde_json::json!([1]),
            DocumentSource::Pasted,
            MAX_TOOL_BYTES + 1,
            std::time::Duration::ZERO,
        ));
        let mut o = operand();
        o.set_text("[1]");
        o.use_document(big);
        assert_eq!(o.text(), "[1]", "the box is as it was");
        assert!(matches!(o.input(), Some(Input::Text(_))));
        assert!(
            o.problem
                .as_deref()
                .is_some_and(|p| p.contains("more than")),
            "{:?}",
            o.problem
        );

        let fits = Arc::new(Document::from_value(
            serde_json::json!([1]),
            DocumentSource::Pasted,
            MAX_TOOL_BYTES,
            std::time::Duration::ZERO,
        ));
        o.use_document(fits);
        assert!(matches!(o.input(), Some(Input::Document(_))));
        assert!(o.problem.is_none());
    }

    #[test]
    fn swapping_keeps_the_titles() {
        let mut a = Operand::new("Before", "a", "");
        let mut b = Operand::new("After", "b", "");
        a.set_text("1");
        b.set_text("2");
        a.swap_with(&mut b);
        assert_eq!((a.text(), b.text()), ("2", "1"));
        assert_eq!((a.title, b.title), ("Before", "After"));
    }

    #[test]
    fn dropped_files_fill_the_empty_boxes_first() {
        let dir = temp_dir("deliver");
        let one = dir.join("one.json");
        let two = dir.join("two.json");
        let three = dir.join("three.json");
        std::fs::write(&one, "1").unwrap();
        std::fs::write(&two, "2").unwrap();
        std::fs::write(&three, "3").unwrap();

        let mut a = Operand::new("A", "a", "");
        let mut b = Operand::new("B", "b", "");
        assert_eq!(drop_target(&[&a, &b]), 0);
        assert!(deliver(&mut [&mut a, &mut b], vec![one, two]));
        assert_eq!((a.text(), b.text()), ("1", "2"));

        // Both are full: the last one is replaced.
        assert_eq!(drop_target(&[&a, &b]), 1);
        deliver(&mut [&mut a, &mut b], vec![three]);
        assert_eq!((a.text(), b.text()), ("1", "3"));

        assert!(!deliver(&mut [&mut a, &mut b], Vec::new()));
    }
}
