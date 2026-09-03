use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use jsonquery_core::Document;
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
    results_view: ResultsView,
    results_text_cache: String,
    results_text_dirty: bool,

    /// Buffer for the inline "paste JSON" text area shown while no document
    /// is loaded.
    paste_text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResultsView {
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
            results_view: ResultsView::Tree,
            results_text_cache: String::new(),
            results_text_dirty: true,
            paste_text: String::new(),
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
                }
                Event::LoadError(e) => {
                    self.loading = false;
                    self.load_error = Some(e);
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
            ui.separator();
            theme_toggle_button(ui);
            ui.separator();
            if let Some(doc) = &self.doc {
                ui.label(format!(
                    "{}  ·  {}",
                    doc.source.label(),
                    human_bytes(doc.byte_len)
                ));
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
        });
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

        ui.weak("Paste JSON below, drag & drop a file anywhere, or use Open File.");
        ui.add_space(4.0);

        let resp = ui.add(
            egui::TextEdit::multiline(&mut self.paste_text)
                .desired_rows(24)
                .desired_width(f32::INFINITY)
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
        let rows = self.results_tree.row_count(&self.results);
        ui.horizontal(|ui| {
            ui.heading(format!("Results ({rows} row{})", plural(rows)));
            ui.add_space(12.0);
            ui.selectable_value(&mut self.results_view, ResultsView::Tree, "Tree");
            ui.selectable_value(&mut self.results_view, ResultsView::Text, "Text");
        });
        ui.separator();
        match self.results_view {
            ResultsView::Tree => self.results_tree.ui(ui, "results_tree", &self.results),
            ResultsView::Text => self.results_text_view(ui),
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

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("query_bar").show(ui, |ui| self.query_bar(ui));
        egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));

        egui::Panel::left("source_panel")
            .resizable(true)
            .default_size(ui.available_width() * 0.5)
            .show(ui, |ui| match &self.doc {
                Some(doc) => {
                    let rows = self.source_tree.row_count(&doc.root);
                    ui.heading(format!("Source ({rows} row{})", plural(rows)));
                    ui.separator();
                    self.source_tree.ui(ui, "source_tree", &doc.root);
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

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
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
