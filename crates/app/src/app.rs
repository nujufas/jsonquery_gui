use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use jsonquery_core::{path_string, resolve, Document, DocumentSource, NodePath, PathSegment};
use serde_json::Value;

use crate::tree_view::{RowAction, TreeView};
use crate::worker::{self, Command, Event, SearchRoot};

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

    /// "Reveal in source" request/reply state (results panel row clicks).
    find_gen: u64,
    finding: bool,
    find_message: Option<String>,

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

    /// Which panel a click landed in most recently — Ctrl+F and Ctrl+S act
    /// on this one, the same way a desktop app's "Find"/"Save" menu commands
    /// act on whichever document window last had focus.
    focused_panel: PanelKind,

    /// State for the "Open URL…" popup.
    show_url_dialog: bool,
    url_input: String,

    /// State for the "Search…" popup and the results panel it feeds.
    show_search_dialog: bool,
    search_input: String,
    search_regex: bool,
    search_target: PanelKind,
    search_gen: u64,
    searching: bool,
    search_error: Option<String>,
    search_results: Vec<SearchMatch>,
    /// Whether the bottom search-results panel is shown; set when a search
    /// starts, cleared by its "Close" button.
    search_panel_open: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Tree,
    Text,
}

/// The Source or Results panel — which tree a search/save action targets,
/// and (via `App::focused_panel`) which one last had a click in it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelKind {
    Source,
    Results,
}

impl PanelKind {
    fn label(self) -> &'static str {
        match self {
            PanelKind::Source => "Source",
            PanelKind::Results => "Results",
        }
    }
}

/// One hit from a "Search…" run, ready for the results panel: enough to
/// display it and, on click, reveal it back in its owning tree.
struct SearchMatch {
    target: PanelKind,
    path: NodePath,
    preview: String,
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
            find_gen: 0,
            finding: false,
            find_message: None,
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
            focused_panel: PanelKind::Source,
            show_url_dialog: false,
            url_input: String::new(),
            show_search_dialog: false,
            search_input: String::new(),
            search_regex: false,
            search_target: PanelKind::Source,
            search_gen: 0,
            searching: false,
            search_error: None,
            search_results: Vec::new(),
            search_panel_open: false,
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

                    // Invalidates any in-flight "reveal in source" too — it
                    // would otherwise land on an unrelated new document.
                    self.find_gen += 1;
                    self.finding = false;
                    self.find_message = None;
                    self.invalidate_search();

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
                Event::Found { gen, path } => {
                    if gen != self.find_gen {
                        continue;
                    }
                    self.finding = false;
                    match path {
                        Some(p) => {
                            self.find_message = None;
                            self.source_tree.reveal(p);
                            self.source_view = ViewMode::Tree;
                        }
                        None => {
                            self.find_message = Some("Not found in source.".to_string());
                        }
                    }
                }
                Event::SearchDone { gen, matches } => {
                    if gen != self.search_gen {
                        continue;
                    }
                    self.searching = false;
                    self.search_error = None;
                    let root = match self.search_target {
                        PanelKind::Source => self.doc.as_ref().map(|d| &d.root),
                        PanelKind::Results => Some(&self.results),
                    };
                    self.search_results = build_search_matches(self.search_target, root, matches);
                }
                Event::SearchError { gen, error } => {
                    if gen != self.search_gen {
                        continue;
                    }
                    self.searching = false;
                    self.search_error = Some(error);
                    self.search_results.clear();
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
        let default_name = default_filename_for_source(&doc.source);
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("JSON", &["json"])
            .save_file()
        {
            let _ = self.cmd_tx.send(Command::SaveFile {
                doc,
                node_path: None,
                path,
            });
        }
    }

    /// Like `save_source`, but for one row of the source tree (a right-click
    /// "Save…") rather than the whole document.
    fn save_source_node(&mut self, node_path: NodePath) {
        let Some(doc) = self.doc.clone() else { return };
        let default_name = default_filename_for_node(&node_path, "data.json");
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("JSON", &["json"])
            .save_file()
        {
            let _ = self.cmd_tx.send(Command::SaveFile {
                doc,
                node_path: Some(node_path),
                path,
            });
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

    /// Like `save_results`, but for one row of the results tree.
    fn save_results_node(&mut self, node_path: NodePath) {
        let Some(value) = resolve(&self.results, &node_path) else {
            return;
        };
        let value = value.clone();
        let default_name = default_filename_for_node(&node_path, "results.json");
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("JSON", &["json"])
            .save_file()
        {
            let _ = self.cmd_tx.send(Command::SaveResults {
                results: value,
                path,
            });
        }
    }

    /// "Find in Source": look for the results row at `node_path`'s value
    /// somewhere in the loaded source document, and if found,
    /// expand/scroll/highlight it there (Architecture-style: the search runs
    /// on the worker thread since the source document can be large, and is
    /// discarded if it's stale by the time it comes back — same `gen`
    /// pattern as queries).
    fn navigate_to_source(&mut self, node_path: &NodePath) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(target) = resolve(&self.results, node_path) else {
            return;
        };
        self.find_gen += 1;
        self.finding = true;
        self.find_message = None;
        let _ = self.cmd_tx.send(Command::FindInSource {
            doc,
            target: target.clone(),
            gen: self.find_gen,
        });
    }

    /// Ctrl+F (search) and Ctrl+S (save) act on `self.focused_panel` — the
    /// Source or Results panel that last saw a click — the same way a
    /// desktop app's menu-bar Find/Save act on whichever document window is
    /// frontmost, rather than requiring a dedicated shortcut per panel.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let find = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
        if find {
            match self.focused_panel {
                PanelKind::Source if self.doc.is_some() => {
                    self.open_search_dialog(PanelKind::Source);
                }
                PanelKind::Results => self.open_search_dialog(PanelKind::Results),
                PanelKind::Source => {}
            }
        }

        let save = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
        if save {
            match self.focused_panel {
                PanelKind::Source if self.doc.is_some() => self.save_source(),
                PanelKind::Results if self.results.as_array().is_some_and(|a| !a.is_empty()) => {
                    self.save_results();
                }
                _ => {}
            }
        }
    }

    /// Update `focused_panel` from a click this frame, so the next Ctrl+F or
    /// Ctrl+S knows which panel to act on. `source_rect`/`results_rect` are
    /// each panel's full on-screen area for this frame.
    fn note_panel_click(
        &mut self,
        ctx: &egui::Context,
        source_rect: egui::Rect,
        results_rect: egui::Rect,
    ) {
        if !ctx.input(|i| i.pointer.any_pressed()) {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        if source_rect.contains(pos) {
            self.focused_panel = PanelKind::Source;
        } else if results_rect.contains(pos) {
            self.focused_panel = PanelKind::Results;
        }
    }

    /// Open the "Search…" dialog for `target`'s tree, discarding whatever
    /// text was left over from a previous search.
    fn open_search_dialog(&mut self, target: PanelKind) {
        self.show_search_dialog = true;
        self.search_target = target;
        self.search_input.clear();
    }

    /// Send the current search dialog's query to the worker thread, over
    /// whichever tree it was opened for.
    fn run_search(&mut self) {
        let root = match self.search_target {
            PanelKind::Source => match self.doc.clone() {
                Some(doc) => SearchRoot::Source(doc),
                None => return,
            },
            PanelKind::Results => SearchRoot::Results(Arc::new(self.results.clone())),
        };
        self.search_gen += 1;
        self.searching = true;
        self.search_error = None;
        self.search_results.clear();
        self.search_panel_open = true;
        let _ = self.cmd_tx.send(Command::Search {
            root,
            text: self.search_input.clone(),
            regex: self.search_regex,
            gen: self.search_gen,
        });
    }

    /// Discard any in-flight or displayed search — the tree it was searching
    /// just changed out from under it (a new document loaded, a new query
    /// run, or the source cleared).
    fn invalidate_search(&mut self) {
        self.search_gen += 1;
        self.searching = false;
        self.search_error = None;
        self.search_results.clear();
        self.search_panel_open = false;
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
        self.find_gen += 1;
        self.finding = false;
        self.find_message = None;
        self.invalidate_search();

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

        // A new query invalidates any search over the previous results —
        // the source tree isn't affected, but discarding both is simpler
        // and correct either way.
        self.invalidate_search();

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

    /// Popup prompting for a search query; shown when `show_search_dialog`
    /// is set by a row's "Search…" context-menu item. Runs over the whole
    /// tree it was opened for (`self.search_target`), not just the row that
    /// was right-clicked.
    fn search_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_search_dialog {
            return;
        }

        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new(format!("Search — {}", self.search_target.label()))
            .id(egui::Id::new("search_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Find:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search_input)
                        .desired_width(320.0)
                        .hint_text("text to find…"),
                );
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.checkbox(&mut self.search_regex, "Regex");
                ui.horizontal(|ui| {
                    let find_clicked = ui
                        .add_enabled(
                            !self.search_input.trim().is_empty(),
                            egui::Button::new("Find All"),
                        )
                        .clicked();
                    submit = find_clicked || enter;
                    cancel = ui.button("Cancel").clicked();
                });
            });

        if cancel || !open {
            self.show_search_dialog = false;
        } else if submit && !self.search_input.trim().is_empty() {
            self.show_search_dialog = false;
            self.run_search();
        }
    }

    /// The bottom "Search results" panel, populated by the last completed
    /// search — a Notepad++-style "Find All" list rather than jumping
    /// straight to one hit. Clicking a match reveals it in its owning tree.
    fn search_results_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut heading = format!("Search results — {}", self.search_target.label());
            if !self.search_input.is_empty() {
                heading.push_str(&format!(" \"{}\"", self.search_input));
            }
            if self.search_regex {
                heading.push_str(" (regex)");
            }
            ui.strong(heading);
            if self.searching {
                ui.spinner();
            } else if self.search_error.is_none() {
                ui.weak(format!("{} match(es)", self.search_results.len()));
            }

            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui.button("Close").clicked() {
                        self.search_panel_open = false;
                    }
                },
            );
        });
        ui.separator();

        if let Some(err) = &self.search_error {
            ui.colored_label(
                egui::Color32::from_rgb(220, 80, 80),
                format!("Search error: {err}"),
            );
            return;
        }
        if !self.searching && self.search_results.is_empty() {
            ui.weak("No matches found.");
            return;
        }

        let mut reveal: Option<(PanelKind, NodePath)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for m in &self.search_results {
                    let text = format!(
                        "[{}]  {}   {}",
                        m.target.label(),
                        path_string(&m.path),
                        m.preview
                    );
                    if ui
                        .add(
                            egui::Label::new(egui::RichText::new(text).monospace())
                                .sense(egui::Sense::click()),
                        )
                        .clicked()
                    {
                        reveal = Some((m.target, m.path.clone()));
                    }
                }
            });

        if let Some((target, path)) = reveal {
            match target {
                PanelKind::Source => {
                    self.source_tree.reveal(path);
                    self.source_view = ViewMode::Tree;
                }
                PanelKind::Results => {
                    self.results_tree.reveal(path);
                    self.results_view = ViewMode::Tree;
                }
            }
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

            if self.finding {
                ui.separator();
                ui.spinner();
                ui.label("Locating in source…");
            } else if let Some(msg) = &self.find_message {
                ui.separator();
                ui.weak(msg);
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
            ViewMode::Tree => {
                if let Some(action) = self
                    .results_tree
                    .ui(ui, "results_tree", &self.results, true)
                {
                    match action {
                        RowAction::Save(node_path) => self.save_results_node(node_path),
                        RowAction::FindInSource(node_path) => self.navigate_to_source(&node_path),
                        RowAction::OpenSearch => self.open_search_dialog(PanelKind::Results),
                    }
                }
            }
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
        self.search_dialog(ui.ctx());
        self.handle_shortcuts(ui.ctx());

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("query_bar").show(ui, |ui| self.query_bar(ui));
        egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));
        if self.search_panel_open {
            egui::Panel::bottom("search_results_panel")
                .resizable(true)
                .default_size(180.0)
                .show(ui, |ui| self.search_results_panel(ui));
        }

        // Matches `CentralPanel`'s inner margin (`Frame::central_panel` uses
        // `Margin::same(8)`) so the "Source" and "Results" headers — and
        // everything below them — line up; `Panel`'s own default
        // (`Margin::symmetric(8, 2)`) is 6px shorter on top/bottom.
        let source_frame =
            egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::same(8));

        let source_resp = egui::Panel::left("source_panel")
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
                        ViewMode::Tree => {
                            if let Some(action) =
                                self.source_tree.ui(ui, "source_tree", &doc.root, false)
                            {
                                match action {
                                    RowAction::Save(node_path) => self.save_source_node(node_path),
                                    RowAction::OpenSearch => {
                                        self.open_search_dialog(PanelKind::Source)
                                    }
                                    RowAction::FindInSource(_) => {}
                                }
                            }
                        }
                        ViewMode::Text => self.source_text_view(ui, &doc),
                    }
                }
                None => {
                    ui.heading("Source");
                    ui.separator();
                    self.paste_area(ui, hovering_drop);
                }
            });

        let results_resp = egui::CentralPanel::default().show(ui, |ui| self.results_panel(ui));

        self.note_panel_click(
            ui.ctx(),
            source_resp.response.rect,
            results_resp.response.rect,
        );
    }
}

fn default_filename_for_source(source: &DocumentSource) -> String {
    match source {
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
    }
}

/// Default save-dialog filename for a single tree row, derived from its own
/// key/index — e.g. row `.users[3]` suggests `item_3.json`, `.address`
/// suggests `address.json`. Just a suggestion the user can freely rename, so
/// this doesn't need to sanitize exotic key characters.
fn default_filename_for_node(node_path: &NodePath, fallback: &str) -> String {
    match node_path.last() {
        Some(PathSegment::Key(k)) => format!("{k}.json"),
        Some(PathSegment::Index(i)) => format!("item_{i}.json"),
        None => fallback.to_string(),
    }
}

/// Turn the worker's raw match paths into displayable `SearchMatch`es by
/// resolving each one against `root` for a preview snippet. `root` is `None`
/// only if the source document was cleared out from under an in-flight
/// source search — those paths just fall back to a placeholder rather than
/// being dropped, since `Event::SearchDone` is otherwise unconditionally
/// accepted once its `gen` matches.
fn build_search_matches(
    target: PanelKind,
    root: Option<&Value>,
    paths: Vec<NodePath>,
) -> Vec<SearchMatch> {
    paths
        .into_iter()
        .map(|path| {
            let preview = root
                .and_then(|r| resolve(r, &path))
                .map(preview_text)
                .unwrap_or_else(|| "<value>".to_string());
            SearchMatch {
                target,
                path,
                preview,
            }
        })
        .collect()
}

/// Short one-line rendering of a value for the search-results list — mirrors
/// the tree view's own row text (Architecture: `tree_view::draw_row_visual`)
/// without needing a `Ui` to draw it.
fn preview_text(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("{s:?}"),
        Value::Array(a) => format!("[…] ({} items)", a.len()),
        Value::Object(o) => format!("{{…}} ({} keys)", o.len()),
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
