use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use jsonquery_core::{Document, DocumentSource};
use serde_json::Value;

use crate::tree_view::TreeView;
use crate::worker::{self, Command, Event};

/// Bounded live-preview cap (Architecture §7): the results tree never holds
/// more than this many items in memory at once, no matter how large the
/// query's actual output is. "Save results to file" (Phase 3) is what
/// bypasses this for the full output.
const LIVE_PREVIEW_CAP: usize = 50_000;

pub struct App {
    cmd_tx: Sender<Command>,
    evt_rx: Receiver<Event>,

    doc: Option<Arc<Document>>,
    loading: bool,
    load_error: Option<String>,
    save_error: Option<String>,
    last_saved: Option<PathBuf>,

    query_text: String,
    query_gen: u64,
    active_cancel: Option<Arc<AtomicBool>>,
    query_running: bool,
    query_error: Option<String>,

    /// Always a `Value::Array` — the accumulated (possibly capped) results
    /// of the current query, in the shape the results tree renders directly.
    results: Value,
    results_item_errors: usize,
    last_item_error: Option<String>,
    results_count_so_far: usize,
    results_truncated: bool,
    last_query_elapsed: Option<Duration>,
    last_query_cancelled: bool,

    source_tree: TreeView,
    results_tree: TreeView,
    source_view: ViewMode,
    results_view: ViewMode,
    source_text_cache: String,
    source_text_dirty: bool,
    results_text_cache: String,
    results_text_dirty: bool,

    /// Buffer for the inline "paste JSON" text area shown while no document
    /// is loaded.
    paste_text: String,

    /// State for the "Open URL…" popup.
    show_url_dialog: bool,
    url_input: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Tree,
    Text,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (evt_tx, evt_rx) = crossbeam_channel::unbounded();

        // Deterministic starting theme (rather than following the system,
        // which would make the light/dark toggle's initial state a surprise).
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);

        let ctx = cc.egui_ctx.clone();
        worker::spawn(cmd_rx, evt_tx, move || ctx.request_repaint());

        Self {
            cmd_tx,
            evt_rx,
            doc: None,
            loading: false,
            load_error: None,
            save_error: None,
            last_saved: None,
            query_text: String::new(),
            query_gen: 0,
            active_cancel: None,
            query_running: false,
            query_error: None,
            results: Value::Array(Vec::new()),
            results_item_errors: 0,
            last_item_error: None,
            results_count_so_far: 0,
            results_truncated: false,
            last_query_elapsed: None,
            last_query_cancelled: false,
            source_tree: TreeView::default(),
            results_tree: TreeView::default(),
            source_view: ViewMode::Tree,
            results_view: ViewMode::Tree,
            source_text_cache: String::new(),
            source_text_dirty: true,
            results_text_cache: String::new(),
            results_text_dirty: true,
            paste_text: String::new(),
            show_url_dialog: false,
            url_input: String::new(),
        }
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.evt_rx.try_recv() {
            match event {
                Event::Loading => {
                    self.loading = true;
                    self.load_error = None;
                }
                Event::Loaded(doc) => {
                    self.loading = false;
                    self.load_error = None;
                    self.save_error = None;
                    self.last_saved = None;

                    // A newly loaded document invalidates any query that was
                    // running against the previous one.
                    if let Some(prev) = self.active_cancel.take() {
                        prev.store(true, Ordering::Relaxed);
                    }
                    self.query_running = false;
                    self.query_error = None;
                    self.results = Value::Array(Vec::new());
                    self.results_item_errors = 0;
                    self.last_item_error = None;
                    self.results_count_so_far = 0;
                    self.results_truncated = false;
                    self.last_query_elapsed = None;
                    self.results_tree.reset();
                    self.results_text_dirty = true;

                    self.doc = Some(doc);
                    self.source_tree.reset();
                    self.source_text_dirty = true;
                }
                Event::LoadError(e) => {
                    self.loading = false;
                    self.load_error = Some(e);
                }
                Event::Saved(path) => {
                    self.save_error = None;
                    self.last_saved = Some(path);
                }
                Event::SaveError(e) => {
                    self.last_saved = None;
                    self.save_error = Some(e);
                }
                Event::QueryItem { gen, value } => {
                    if gen != self.query_gen {
                        continue;
                    }
                    self.results_count_so_far += 1;
                    if let Value::Array(arr) = &mut self.results {
                        if arr.len() < LIVE_PREVIEW_CAP {
                            arr.push(value);
                        } else {
                            self.results_truncated = true;
                        }
                    }
                    self.results_tree.mark_dirty();
                    self.results_text_dirty = true;
                }
                Event::QueryItemError { gen, error } => {
                    if gen != self.query_gen {
                        continue;
                    }
                    self.results_item_errors += 1;
                    self.last_item_error = Some(error);
                }
                Event::QueryDone {
                    gen,
                    cancelled,
                    elapsed,
                } => {
                    if gen != self.query_gen {
                        continue;
                    }
                    self.query_running = false;
                    self.last_query_elapsed = Some(elapsed);
                    self.last_query_cancelled = cancelled;
                }
                Event::QueryError { gen, error } => {
                    if gen != self.query_gen {
                        continue;
                    }
                    self.query_running = false;
                    self.query_error = Some(error);
                }
            }
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        let _ = self.cmd_tx.send(Command::OpenFile(path));
    }

    fn open_text(&mut self, text: String) {
        let _ = self.cmd_tx.send(Command::OpenText(text));
    }

    fn open_url(&mut self, url: String) {
        let _ = self.cmd_tx.send(Command::OpenUrl(url));
    }

    /// Prompt for a destination and write the currently loaded source data
    /// to it as pretty-printed JSON — how pasted, edited, or
    /// URL-downloaded data (which otherwise only lives in memory or a temp
    /// file) gets made permanent.
    fn save_source(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let default_name = match &doc.source {
            DocumentSource::File(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "data.json".to_string()),
            DocumentSource::Pasted => "data.json".to_string(),
            DocumentSource::Url(url) => url
                .split(['?', '#'])
                .next()
                .unwrap_or(url)
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("data.json")
                .to_string(),
        };
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("JSON", &["json"])
            .save_file()
        {
            let _ = self.cmd_tx.send(Command::SaveFile { doc, path });
        }
    }

    /// Prompt for a destination and write the current results (as currently
    /// materialized — up to `LIVE_PREVIEW_CAP` items, same as what's shown)
    /// to it as pretty-printed JSON.
    fn save_results(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("results.json")
            .add_filter("JSON", &["json"])
            .save_file()
        {
            let _ = self.cmd_tx.send(Command::SaveResults {
                results: self.results.clone(),
                path,
            });
        }
    }

    /// Reset all document-derived state back to "nothing loaded", so the app
    /// can be pointed at a new source without restarting it.
    fn clear_source(&mut self) {
        if let Some(prev) = self.active_cancel.take() {
            prev.store(true, Ordering::Relaxed);
        }
        self.query_gen += 1;
        self.query_running = false;
        self.query_error = None;

        self.doc = None;
        self.loading = false;
        self.load_error = None;
        self.save_error = None;
        self.last_saved = None;

        self.results = Value::Array(Vec::new());
        self.results_item_errors = 0;
        self.last_item_error = None;
        self.results_count_so_far = 0;
        self.results_truncated = false;
        self.last_query_elapsed = None;
        self.last_query_cancelled = false;
        self.results_tree.reset();
        self.results_text_dirty = true;

        self.source_tree.reset();
        self.source_text_cache.clear();
        self.source_text_dirty = true;
        self.paste_text.clear();
    }

    /// Start a new query run, cancelling whatever query was previously in
    /// flight (Architecture §5's generation-counter pattern).
    fn run_query(&mut self) {
        let Some(doc) = self.doc.clone() else { return };

        if let Some(prev) = self.active_cancel.take() {
            prev.store(true, Ordering::Relaxed);
        }
        self.query_gen += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_cancel = Some(cancel.clone());

        self.results = Value::Array(Vec::new());
        self.results_tree.reset();
        self.results_text_dirty = true;
        self.results_item_errors = 0;
        self.last_item_error = None;
        self.results_count_so_far = 0;
        self.results_truncated = false;
        self.query_error = None;
        self.last_query_elapsed = None;
        self.query_running = true;

        let _ = self.cmd_tx.send(Command::Query {
            doc,
            text: self.query_text.clone(),
            gen: self.query_gen,
            cancel,
        });
    }

    fn cancel_query(&mut self) {
        if let Some(c) = self.active_cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.query_running = false;
        self.last_query_cancelled = true;
    }

    fn handle_drag_and_drop(&mut self, ui: &egui::Ui) -> bool {
        let (hovering, dropped_path) = ui.ctx().input(|i| {
            let hovering = !i.raw.hovered_files.is_empty();
            let dropped = i.raw.dropped_files.first().map(|f| f.path().to_path_buf());
            (hovering, dropped)
        });
        if let Some(path) = dropped_path {
            self.open_file(path);
        }
        hovering
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Open File…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("JSON", &["json", "ndjson", "jsonl", "log", "txt"])
                    .pick_file()
                {
                    self.open_file(path);
                }
            }
            if ui.button("Open URL…").clicked() {
                self.show_url_dialog = true;
            }
            if ui
                .add_enabled(
                    self.doc.is_some() || self.loading || self.load_error.is_some(),
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
                self.clear_source();
            }
            ui.separator();

            if let Some(doc) = &self.doc {
                // `&str` is an immutable `TextBuffer` impl, so this behaves as
                // a read-only field: selectable and copyable with the mouse,
                // but typing into it has no effect and nothing is written
                // back to `doc`.
                let label = doc.source.label();
                let mut label_ref = label.as_str();
                ui.add(
                    egui::TextEdit::singleline(&mut label_ref)
                        .desired_width(320.0)
                        .font(egui::TextStyle::Monospace),
                );
                ui.weak(human_bytes(doc.byte_len));
                if doc.top_level_values > 1 {
                    ui.weak(format!("({} NDJSON records)", doc.top_level_values));
                }
            } else if self.loading {
                ui.spinner();
                ui.label("Loading…");
            } else {
                ui.weak(
                    "No document loaded — drag & drop a JSON file anywhere, use Open File, or paste JSON on the left.",
                );
            }

            // Claims whatever width is left after everything above, so the
            // theme toggle sits pinned at the top-right corner regardless of
            // how long the path/status text is.
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::right_to_left(egui::Align::Center),
                theme_toggle_button,
            );
        });
    }

    /// Popup prompting for a URL to download; shown when `show_url_dialog`
    /// is set by the "Open URL…" toolbar button.
    fn url_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_url_dialog {
            return;
        }

        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new("Open URL")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("URL:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.url_input)
                        .desired_width(360.0)
                        .hint_text("https://example.com/data.json"),
                );
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    let load_clicked = ui
                        .add_enabled(!self.url_input.trim().is_empty(), egui::Button::new("Load"))
                        .clicked();
                    submit = load_clicked || enter;
                    cancel = ui.button("Cancel").clicked();
                });
            });

        if cancel || !open {
            self.show_url_dialog = false;
            self.url_input.clear();
        } else if submit && !self.url_input.trim().is_empty() {
            let url = std::mem::take(&mut self.url_input);
            self.show_url_dialog = false;
            self.open_url(url);
        }
    }

    /// The inline "paste JSON" panel shown in place of the source tree while
    /// no document is loaded: paste, and it loads immediately — no button.
    fn paste_area(&mut self, ui: &mut egui::Ui, hovering_drop: bool) {
        if hovering_drop {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("Drop to open");
            });
            return;
        }

        ui.weak("Drag & drop a file anywhere, or use Open File.");
        ui.add_space(4.0);

        // Fill whatever space is left in the panel rather than a fixed row
        // count, so the box grows/shrinks with the window instead of leaving
        // dead space below it.
        let resp = ui.add_sized(
            ui.available_size(),
            egui::TextEdit::multiline(&mut self.paste_text)
                .code_editor()
                .hint_text("Paste JSON here…"),
        );

        let pasted = resp.has_focus()
            && ui
                .ctx()
                .input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
        let submit_shortcut = resp.has_focus()
            && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
        if (pasted || submit_shortcut) && !self.paste_text.trim().is_empty() {
            let text = std::mem::take(&mut self.paste_text);
            self.open_text(text);
        }
    }

    fn query_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Query:");
            if self.query_running {
                ui.spinner();
                if ui.button("Cancel").clicked() {
                    self.cancel_query();
                }
            } else {
                let clicked = ui
                    .add_enabled(self.doc.is_some(), egui::Button::new("Run  (Ctrl+Enter)"))
                    .clicked();
                let shortcut = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
                if (clicked || shortcut) && self.doc.is_some() {
                    self.run_query();
                }
            }
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.query_text)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .code_editor()
                .hint_text(".   (jq-compatible — e.g. .[] | select(.age > 21) | .name)"),
        );
        ui.add_space(4.0);
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if let Some(doc) = &self.doc {
                ui.label(format!("Parsed in {:.1?}", doc.parse_time));
                ui.separator();
            }
            if let Some(err) = &self.load_error {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 80, 80),
                    format!("Load error: {err}"),
                );
                ui.separator();
            }
            if let Some(err) = &self.save_error {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 80, 80),
                    format!("Save error: {err}"),
                );
                ui.separator();
            } else if let Some(path) = &self.last_saved {
                ui.weak(format!("Saved to {}", path.display()));
                ui.separator();
            }

            if self.query_running {
                ui.label("Query running…");
            } else if let Some(elapsed) = self.last_query_elapsed {
                let shown = self.results.as_array().map(|a| a.len()).unwrap_or(0);
                let mut s = format!(
                    "Query ran in {:.1?} — {} result(s)",
                    elapsed, self.results_count_so_far
                );
                if self.results_truncated {
                    s.push_str(&format!(
                        ", {shown} shown (live preview capped at {LIVE_PREVIEW_CAP})"
                    ));
                }
                if self.last_query_cancelled {
                    s.push_str(" — cancelled");
                }
                ui.label(s);
            }

            if self.results_item_errors > 0 {
                ui.separator();
                let mut msg = format!("{} item error(s)", self.results_item_errors);
                if let Some(last) = &self.last_item_error {
                    msg.push_str(&format!(" (last: {last})"));
                }
                ui.colored_label(egui::Color32::from_rgb(210, 150, 40), msg);
            }
            if let Some(err) = &self.query_error {
                ui.separator();
                ui.colored_label(
                    egui::Color32::from_rgb(220, 80, 80),
                    format!("Query error: {err}"),
                );
            }
        });
    }

    fn results_panel(&mut self, ui: &mut egui::Ui) {
        let has_results = self.results.as_array().is_some_and(|a| !a.is_empty());
        ui.horizontal(|ui| {
            ui.heading("Results");
            ui.add_space(12.0);
            ui.selectable_value(&mut self.results_view, ViewMode::Tree, "Tree");
            ui.selectable_value(&mut self.results_view, ViewMode::Text, "Text");

            // Pinned to the right edge of the header, mirroring the Source
            // panel's "Save…".
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .add_enabled(has_results, egui::Button::new("Save…"))
                        .clicked()
                    {
                        self.save_results();
                    }
                },
            );
        });
        ui.separator();
        match self.results_view {
            ViewMode::Tree => self.results_tree.ui(ui, "results_tree", &self.results),
            ViewMode::Text => self.results_text_view(ui),
        }
    }

    /// Plain, selectable/copyable pretty-printed JSON — an alternative to the
    /// tree view for grabbing raw text with the mouse. Regenerated only when
    /// the results actually change (`results_text_dirty`), not every frame.
    fn results_text_view(&mut self, ui: &mut egui::Ui) {
        if self.results_text_dirty {
            self.results_text_cache = serde_json::to_string_pretty(&self.results)
                .unwrap_or_else(|e| format!("<failed to render results as text: {e}>"));
            self.results_text_dirty = false;
        }
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(&self.results_text_cache).monospace())
                        .selectable(true)
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
            });
    }

    /// Pretty-printed JSON for the source document — the same Tree/Text
    /// toggle as the results panel. Regenerated only when the loaded
    /// document changes (`source_text_dirty`). For a *pasted* document the
    /// text is directly editable; "Apply" (or Ctrl+Enter) re-parses the
    /// edited buffer in place, so a typo doesn't mean re-pasting from
    /// scratch. Opened files and URL downloads stay read-only, since editing
    /// them wouldn't touch the file/URL they came from.
    fn source_text_view(&mut self, ui: &mut egui::Ui, doc: &Document) {
        if self.source_text_dirty {
            self.source_text_cache = serde_json::to_string_pretty(&doc.root)
                .unwrap_or_else(|e| format!("<failed to render source as text: {e}>"));
            self.source_text_dirty = false;
        }

        if matches!(&doc.source, DocumentSource::Pasted) {
            let mut apply = false;
            ui.horizontal(|ui| {
                ui.weak("Editable — change the JSON below, then apply.");
                ui.add_space(8.0);
                if ui
                    .add_enabled(
                        !self.source_text_cache.trim().is_empty(),
                        egui::Button::new("Apply  (Ctrl+Enter)"),
                    )
                    .clicked()
                {
                    apply = true;
                }
            });
            let resp = ui.add_sized(
                ui.available_size(),
                egui::TextEdit::multiline(&mut self.source_text_cache).code_editor(),
            );
            if resp.has_focus()
                && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))
            {
                apply = true;
            }
            if apply && !self.source_text_cache.trim().is_empty() {
                let text = self.source_text_cache.clone();
                self.open_text(text);
            }
        } else {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&self.source_text_cache).monospace())
                            .selectable(true)
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
        }
    }
}

/// Small ☀/🌙 button that flips between light and dark theme.
fn theme_toggle_button(ui: &mut egui::Ui) {
    let (icon, tooltip, next) = if ui.ctx().theme() == egui::Theme::Dark {
        ("☀", "Switch to light theme", egui::ThemePreference::Light)
    } else {
        ("🌙", "Switch to dark theme", egui::ThemePreference::Dark)
    };
    if ui.button(icon).on_hover_text(tooltip).clicked() {
        ui.ctx().set_theme(next);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();
        let hovering_drop = self.handle_drag_and_drop(ui);
        self.url_dialog(ui.ctx());

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("query_bar").show(ui, |ui| self.query_bar(ui));
        egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));

        // Matches `CentralPanel`'s inner margin (`Frame::central_panel` uses
        // `Margin::same(8)`) so the "Source" and "Results" headers — and
        // everything below them — line up; `Panel`'s own default
        // (`Margin::symmetric(8, 2)`) is 6px shorter on top/bottom.
        let source_frame =
            egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::same(8));

        egui::Panel::left("source_panel")
            .resizable(true)
            .default_size(ui.available_width() * 0.5)
            .frame(source_frame)
            .show(ui, |ui| match self.doc.clone() {
                Some(doc) => {
                    ui.horizontal(|ui| {
                        ui.heading("Source");
                        ui.add_space(12.0);
                        ui.selectable_value(&mut self.source_view, ViewMode::Tree, "Tree");
                        ui.selectable_value(&mut self.source_view, ViewMode::Text, "Text");

                        // Pinned to the right edge of the header, mirroring
                        // the toolbar's theme toggle.
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.button("Save…").clicked() {
                                    self.save_source();
                                }
                            },
                        );
                    });
                    ui.separator();
                    match self.source_view {
                        ViewMode::Tree => self.source_tree.ui(ui, "source_tree", &doc.root),
                        ViewMode::Text => self.source_text_view(ui, &doc),
                    }
                }
                None => {
                    ui.heading("Source");
                    ui.separator();
                    self.paste_area(ui, hovering_drop);
                }
            });

        egui::CentralPanel::default().show(ui, |ui| self.results_panel(ui));
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}
