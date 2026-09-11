use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use jsonquery_core::{path_string, resolve, Document, DocumentSource, NodePath, PathSegment};
use serde_json::Value;

use crate::query_suggest::{apply_suggestion, QuerySuggest};
use crate::tree_view::{RowAction, TreeView};
use crate::worker::{self, Command, Event, SearchRoot};

/// Bounded live-preview cap (Architecture §7): the results tree never holds
/// more than this many items in memory at once, no matter how large the
/// query's actual output is. "Save results to file" (Phase 3) is what
/// bypasses this for the full output.
const LIVE_PREVIEW_CAP: usize = 50_000;

/// Cap on how many nodes the "Text" view's pretty-printer walks (see
/// `jsonquery_core::pretty_print_bounded`) — protects the render from a huge
/// source document, or a huge single query result (e.g. `.` over a
/// multi-GB doc, which `LIVE_PREVIEW_CAP` alone wouldn't bound: one item can
/// still be arbitrarily large).
const TEXT_VIEW_NODE_BUDGET: usize = 20_000;

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

    /// Explicit engine-picker selection (`None` = no button selected, so
    /// [`jsonquery_query::Kind::detect`] chooses one from the query text at
    /// run time).
    query_engine: Option<jsonquery_query::Kind>,
    /// Which engine the most recently run/running query actually used —
    /// `query_engine` if set, else whatever `detect` picked — kept around
    /// purely for the status bar's "ran with X" line.
    last_resolved_engine: Option<jsonquery_query::Kind>,
    /// The query box's autocomplete popup: current candidates, keyboard
    /// selection, and dismiss/re-arm state (see `query_suggest.rs`).
    query_suggest: QuerySuggest,

    /// Always a `Value::Array` — the accumulated (possibly capped) results
    /// of the current query, in the shape the results tree renders directly.
    results: Value,
    results_item_errors: usize,
    last_item_error: Option<String>,
    results_count_so_far: usize,
    results_truncated: bool,
    /// The live-preview item cap in effect for the *current* results —
    /// normally `LIVE_PREVIEW_CAP`, or `usize::MAX` once the user has hit
    /// "Expand All" to fetch the complete (unbounded) result set. Also
    /// doubles as the node budget for the results "Text" view, so expanding
    /// clears both caps together.
    results_cap: usize,
    last_query_elapsed: Option<Duration>,
    last_query_cancelled: bool,
    /// A "Search…" over Results arrived while `results_truncated` was still
    /// true — deferred until the "Expand All" rerun it triggered finishes,
    /// so the search covers the complete results rather than the capped
    /// preview.
    pending_results_search: bool,
    /// Same idea as `pending_results_search`, for "Save…" over Results: the
    /// destination path chosen while results were still truncated, applied
    /// once the "Expand All" rerun it triggered completes.
    pending_save_results: Option<PathBuf>,

    source_tree: TreeView,
    results_tree: TreeView,
    source_view: ViewMode,
    results_view: ViewMode,
    source_text_cache: String,
    source_text_dirty: bool,
    /// A `Command::RenderText` for the source is in flight; used to avoid
    /// spamming the worker with a fresh request every frame while dirty and
    /// to show a "Rendering…" placeholder instead of stale text.
    source_text_pending: bool,
    source_text_gen: u64,
    source_text_truncated: bool,
    results_text_cache: String,
    results_text_dirty: bool,
    results_text_pending: bool,
    results_text_gen: u64,
    results_text_truncated: bool,

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
            query_engine: None,
            last_resolved_engine: None,
            query_suggest: QuerySuggest::default(),
            results: Value::Array(Vec::new()),
            results_item_errors: 0,
            last_item_error: None,
            results_count_so_far: 0,
            results_truncated: false,
            results_cap: LIVE_PREVIEW_CAP,
            last_query_elapsed: None,
            last_query_cancelled: false,
            pending_results_search: false,
            pending_save_results: None,
            source_tree: TreeView::default(),
            results_tree: TreeView::default(),
            source_view: ViewMode::Tree,
            results_view: ViewMode::Tree,
            source_text_cache: String::new(),
            source_text_dirty: true,
            source_text_pending: false,
            source_text_gen: 0,
            source_text_truncated: false,
            results_text_cache: String::new(),
            results_text_dirty: true,
            results_text_pending: false,
            results_text_gen: 0,
            results_text_truncated: false,
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
                    self.results_cap = LIVE_PREVIEW_CAP;
                    self.pending_results_search = false;
                    self.pending_save_results = None;
                    self.last_query_elapsed = None;
                    self.last_resolved_engine = None;
                    self.results_tree.reset();
                    self.invalidate_results_text();

                    // Invalidates any in-flight "reveal in source" too — it
                    // would otherwise land on an unrelated new document.
                    self.find_gen += 1;
                    self.finding = false;
                    self.find_message = None;
                    self.invalidate_search();

                    self.doc = Some(doc);
                    self.source_tree.reset();
                    self.invalidate_source_text();
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
                Event::TextRendered {
                    target,
                    gen,
                    text,
                    truncated,
                } => match target {
                    worker::TextTargetKind::Source => {
                        if gen == self.source_text_gen {
                            self.source_text_cache = text;
                            self.source_text_truncated = truncated;
                            self.source_text_pending = false;
                        }
                    }
                    worker::TextTargetKind::Results => {
                        if gen == self.results_text_gen {
                            self.results_text_cache = text;
                            self.results_text_truncated = truncated;
                            self.results_text_pending = false;
                        }
                    }
                },
                Event::QueryItem { gen, value } => {
                    if gen != self.query_gen {
                        continue;
                    }
                    self.results_count_so_far += 1;
                    if let Value::Array(arr) = &mut self.results {
                        if arr.len() < self.results_cap {
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

                    // If the "Expand All" re-run a pending search/save was
                    // waiting on got cancelled instead of completing, its
                    // results are only a partial, arbitrarily-cut-off
                    // snapshot — not the complete set either was promised —
                    // so drop them rather than search/save that silently.
                    if cancelled {
                        if self.pending_results_search {
                            self.pending_results_search = false;
                            self.searching = false;
                            self.search_error =
                                Some("Cancelled while fetching full results.".to_string());
                        }
                        self.pending_save_results = None;
                    } else {
                        if self.pending_results_search {
                            self.pending_results_search = false;
                            self.run_search();
                        }
                        if let Some(path) = self.pending_save_results.take() {
                            let _ = self.cmd_tx.send(Command::SaveResults {
                                results: self.results.clone(),
                                path,
                            });
                        }
                    }
                }
                Event::QueryError { gen, error } => {
                    if gen != self.query_gen {
                        continue;
                    }
                    self.query_running = false;
                    self.query_error = Some(error.clone());
                    // The "Expand All" re-run that a pending search/save was
                    // waiting on failed outright — surface that instead of
                    // leaving the search panel spinning or the save silently
                    // dropped.
                    if self.pending_results_search {
                        self.pending_results_search = false;
                        self.searching = false;
                        self.search_error = Some(error);
                    }
                    self.pending_save_results = None;
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

    /// Prompt for a destination and write the current results to it as
    /// pretty-printed JSON. If the live preview is still capped, this first
    /// re-runs the query unbounded (`expand_results`) and defers the actual
    /// save until that completes, so the file gets the complete results, not
    /// just the up-to-`LIVE_PREVIEW_CAP` preview.
    fn save_results(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("results.json")
            .add_filter("JSON", &["json"])
            .save_file()
        {
            if self.results_truncated {
                self.expand_results();
                self.pending_save_results = Some(path);
            } else {
                let _ = self.cmd_tx.send(Command::SaveResults {
                    results: self.results.clone(),
                    path,
                });
            }
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
    /// whichever tree it was opened for. Searching Results while the live
    /// preview is still capped would silently miss matches beyond the cap,
    /// so that case first re-runs the query unbounded (`expand_results`) and
    /// retries the search once it completes (`pending_results_search`,
    /// handled in `Event::QueryDone`).
    fn run_search(&mut self) {
        if self.search_target == PanelKind::Results && self.results_truncated {
            self.expand_results();
            self.pending_results_search = true;
            self.searching = true;
            self.search_error = None;
            self.search_results.clear();
            self.search_panel_open = true;
            return;
        }

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

    /// Discard any in-flight or displayed source "Text" render — the
    /// document it was rendering just changed out from under it.
    fn invalidate_source_text(&mut self) {
        self.source_text_gen += 1;
        self.source_text_pending = false;
        self.source_text_dirty = true;
        self.source_text_truncated = false;
    }

    /// Same as `invalidate_source_text`, for the results panel.
    fn invalidate_results_text(&mut self) {
        self.results_text_gen += 1;
        self.results_text_pending = false;
        self.results_text_dirty = true;
        self.results_text_truncated = false;
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
        self.results_cap = LIVE_PREVIEW_CAP;
        self.pending_results_search = false;
        self.pending_save_results = None;
        self.last_query_elapsed = None;
        self.last_query_cancelled = false;
        self.last_resolved_engine = None;
        self.results_tree.reset();
        self.invalidate_results_text();

        self.source_tree.reset();
        self.source_text_cache.clear();
        self.invalidate_source_text();
        self.paste_text.clear();
    }

    /// Start a new query run, cancelling whatever query was previously in
    /// flight (Architecture §5's generation-counter pattern). Always resets
    /// the live-preview cap back to the bounded default — a fresh "Run"
    /// means a new query intent, not a continuation of a previous "Expand
    /// All".
    fn run_query(&mut self) {
        self.run_query_capped(LIVE_PREVIEW_CAP);
    }

    /// Re-run the current query with the live-preview cap lifted, so the
    /// results tree, its "Text" view, and (via `pending_results_search` /
    /// `pending_save_results`) any search or save waiting on it all end up
    /// working from the complete, unbounded result set instead of the
    /// capped live preview.
    fn expand_results(&mut self) {
        self.run_query_capped(usize::MAX);
    }

    fn run_query_capped(&mut self, cap: usize) {
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
        self.invalidate_results_text();
        self.results_item_errors = 0;
        self.last_item_error = None;
        self.results_count_so_far = 0;
        self.results_truncated = false;
        self.results_cap = cap;
        self.pending_results_search = false;
        self.pending_save_results = None;
        self.query_error = None;
        self.last_query_elapsed = None;
        self.query_running = true;

        let engine = self
            .query_engine
            .unwrap_or_else(|| jsonquery_query::Kind::detect(&self.query_text));
        self.last_resolved_engine = Some(engine);

        let _ = self.cmd_tx.send(Command::Query {
            doc,
            text: self.query_text.clone(),
            engine,
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
                // Give the path the whole row minus a modest reserve for
                // what follows it (the byte size, an occasional NDJSON
                // note, and the two icon buttons pinned to the far right)
                // rather than a fixed width — a long path should get to use
                // the room a short one leaves empty, not sit truncated next
                // to a mostly-blank toolbar.
                let label_width = (ui.available_width() - 200.0).max(120.0);
                ui.add(
                    egui::TextEdit::singleline(&mut label_ref)
                        .desired_width(label_width)
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
            // theme toggle (and, just to its left, the autocomplete toggle)
            // sit pinned at the top-right corner regardless of how long the
            // path/status text is.
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    theme_toggle_button(ui);
                    autocomplete_toggle_button(ui, &mut self.query_suggest);
                },
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

            // Engine picker, pinned to the query box's top right. None
            // selected (the default) means "auto" — `Kind::detect` picks a
            // dialect from the query text itself when the query runs.
            // `SelectableLabel` gives selected buttons a subtle tinted
            // background rather than a full button border, matching the
            // "small, subtly-highlighted" look the other toggle buttons
            // (theme, view mode) already use elsewhere in this bar.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for kind in jsonquery_query::Kind::ALL.into_iter().rev() {
                    let selected = self.query_engine == Some(kind);
                    let resp =
                        ui.selectable_label(selected, egui::RichText::new(kind.label()).small());
                    if resp.clicked() {
                        self.query_engine = if selected { None } else { Some(kind) };
                    }
                    resp.on_hover_text(kind.example());
                }
                ui.weak(egui::RichText::new("Engine:").small());
            });
        });
        // Ties the box's minimum height to whatever room the (now resizable)
        // "query_bar" panel above has for it this frame, so dragging the
        // panel's bottom edge visibly grows/shrinks the box even when the
        // query itself is short. `desired_rows` is still just a *minimum* —
        // a query with more lines than fit still grows the box further, same
        // as before. Wrapping this in a `ScrollArea` instead (so oversized
        // queries would scroll rather than grow) was tried and reverted: the
        // cursor's scroll-into-view request on every keystroke fights the
        // panel's own content-based auto-sizing and the two feed back into
        // each other, ballooning the panel to the full window height after
        // typing as little as a second line.
        let line_height = ui.text_style_height(&egui::TextStyle::Monospace)
            + ui.spacing().extra_text_line_spacing;
        let desired_rows = ((ui.available_height() / line_height).floor() as usize).max(1);
        self.query_text_edit(ui, desired_rows);
        ui.add_space(4.0);
    }

    /// The query box itself, plus its autocomplete popup
    /// (`jsonquery_query::suggest`, via `self.query_suggest`).
    ///
    /// Up/Down/Enter/Tab must be intercepted from the input queue *before*
    /// `TextEdit::show` runs, or the widget itself will consume them first
    /// (moving the cursor a line, inserting a newline/tab) — so a
    /// keyboard-driven accept splices `self.query_text` and repositions the
    /// widget's persisted cursor state a frame "early", ahead of `show`,
    /// rather than after it the way the popup's mouse-click handling does.
    fn query_text_edit(&mut self, ui: &mut egui::Ui, desired_rows: usize) {
        let id = egui::Id::new("query_text_edit_box");

        if let Some(idx) = self.query_suggest.intercept_keys(ui.ctx(), id) {
            if let Some(item) = self.query_suggest.items.get(idx).cloned() {
                let new_cursor = apply_suggestion(&mut self.query_text, &item);
                set_text_edit_cursor(ui.ctx(), id, new_cursor);
                self.query_suggest.accepted(new_cursor);
            }
        }

        let output = egui::TextEdit::multiline(&mut self.query_text)
            .id(id)
            .desired_rows(desired_rows)
            .desired_width(f32::INFINITY)
            .code_editor()
            .hint_text(
                "e.g. .[] | select(.age > 21) | .name\n\
                 (auto-detects jq / JSON Pointer / JSONPath / JMESPath — \
                 or pick one at top right)",
            )
            .show(ui);

        // Captured *before* the focus check below: clicking anywhere on the
        // popup itself (a separate `egui::Area`, outside the `TextEdit`'s
        // own widget rect) makes `output.response.has_focus()` false this
        // same frame — egui clears a text edit's focus as soon as a click
        // lands outside it, before this method even learns *what* was
        // clicked. Gating the popup's own rendering/click-handling below on
        // `self.query_suggest.open` directly would mean the `else` branch's
        // `close()` (which clears `items`) always runs first and hides the
        // popup this exact frame — so a click on a suggestion row would
        // never reach that row's own `clicked()` check at all, silently
        // swallowing every mouse-driven accept. Using `was_open` instead
        // keeps rendering the popup (and checking for a click) this frame
        // regardless of the focus change the click itself just caused; if
        // nothing in it was clicked, next frame's `open` is already false
        // and it simply stays gone, same as today.
        let was_open = self.query_suggest.open;
        // Whether to actually close things out below: deferred rather than
        // done right here, since doing it here would run *before* the
        // popup gets a chance to check whether the very click that took
        // focus away landed on one of its own rows (see `was_open`'s own
        // comment) — clearing `items` this early would blank the rows out
        // from under that check.
        let mut lost_focus_this_frame = false;

        if output.response.has_focus() {
            let cursor_char = output.cursor_range.map(|r| r.primary.index.0);
            self.query_suggest.recompute(
                &self.query_text,
                cursor_char,
                output.response.changed(),
                self.query_engine,
                self.doc.as_ref().map(|d| &d.root),
            );
        } else {
            lost_focus_this_frame = true;
        }

        if was_open {
            // A long candidate list (e.g. every index of a large array)
            // starts collapsed to a short preview rather than dumping
            // hundreds of rows straight into the window; keyboard
            // navigation past the fold, or clicking the trailing "N more"
            // row, reveals the rest inside a height-capped scroll area.
            const COLLAPSED_ROWS: usize = 10;
            const POPUP_MAX_HEIGHT: f32 = 320.0;

            let total = self.query_suggest.items.len();
            let show_all = self.query_suggest.expanded
                || total <= COLLAPSED_ROWS
                || self.query_suggest.selected >= COLLAPSED_ROWS;
            let visible_count = if show_all { total } else { COLLAPSED_ROWS };
            let row_count = visible_count + usize::from(!show_all);

            // Force a one-frame invisible "sizing pass" whenever the target
            // row count changes (see `QuerySuggest::popup_sized_for_rows`)
            // so the Area actually re-measures instead of reusing a stale
            // remembered rect from a previously-shown, differently-sized
            // popup.
            let force_resize = self.query_suggest.popup_sized_for_rows != Some(row_count);
            self.query_suggest.popup_sized_for_rows = Some(row_count);

            let anchor = output.response.rect.left_bottom();
            let mut clicked = None;
            let mut expand_clicked = false;
            egui::Area::new(id.with("suggest_popup"))
                .fixed_pos(anchor)
                .order(egui::Order::Foreground)
                // `egui::Area` remembers its rect by `Id` across frames —
                // including across a full close/reopen cycle, since
                // nothing clears that memory just because the popup wasn't
                // drawn for a few frames. So a popup last shown small (e.g.
                // 3 root-array items) that reopens later needing to be much
                // bigger (e.g. 50 items, collapsed to 10 + a "more" row)
                // would otherwise stay clamped at the old, wrong size
                // forever: the `ScrollArea` below only gets however much
                // room the Area's *stale* remembered rect hands it, no
                // matter how tall its own content wants to be. Forcing a
                // one-frame invisible "sizing pass" exactly when the
                // target row count changes (tracked in
                // `popup_sized_for_rows`) makes egui re-measure from a
                // generous default instead of reusing that stale rect.
                .sizing_pass(force_resize)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(440.0);
                        let row_height = ui.text_style_height(&egui::TextStyle::Body)
                            + ui.spacing().item_spacing.y;
                        // Still worth reserving as a floor even with the
                        // sizing-pass fix above: it's what lets the list
                        // grow smoothly while a *single* popup stays
                        // continuously open (e.g. more candidates appearing
                        // as the user keeps typing), without waiting for a
                        // row-count-changed sizing pass on every keystroke.
                        ui.set_min_height((row_count as f32 * row_height).min(POPUP_MAX_HEIGHT));
                        egui::ScrollArea::vertical()
                            .max_height(POPUP_MAX_HEIGHT)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (i, item) in self
                                    .query_suggest
                                    .items
                                    .iter()
                                    .take(visible_count)
                                    .enumerate()
                                {
                                    let selected = i == self.query_suggest.selected;
                                    let row = format!("{}  —  {}", item.label, item.detail);
                                    let resp = ui.selectable_label(selected, row);
                                    if selected {
                                        resp.scroll_to_me(Some(egui::Align::Center));
                                    }
                                    if resp.clicked() {
                                        clicked = Some(i);
                                    }
                                }
                                if !show_all {
                                    let remaining = total - visible_count;
                                    let more = ui.selectable_label(
                                        false,
                                        egui::RichText::new(format!("…  {remaining} more"))
                                            .weak()
                                            .italics(),
                                    );
                                    if more.clicked() {
                                        expand_clicked = true;
                                    }
                                }
                            });
                    });
                });
            if expand_clicked {
                self.query_suggest.expanded = true;
            }
            if let Some(idx) = clicked {
                if let Some(item) = self.query_suggest.items.get(idx).cloned() {
                    let new_cursor = apply_suggestion(&mut self.query_text, &item);
                    set_text_edit_cursor(ui.ctx(), id, new_cursor);
                    self.query_suggest.accepted(new_cursor);
                }
            } else if expand_clicked {
                // Also took focus away from the `TextEdit` this same frame
                // (it's a click on the popup, same as a row click above),
                // so it needs the same explicit reclaim a row-click accept
                // gets from `set_text_edit_cursor` — otherwise the box
                // stays unfocused and the next keystroke goes nowhere.
                ui.ctx().memory_mut(|m| m.request_focus(id));
            } else if lost_focus_this_frame {
                // The click (or whatever else took focus away) wasn't on
                // one of the popup's own rows after all — a genuine
                // "clicked elsewhere" — so close it out now.
                self.query_suggest.close();
            }
        } else if lost_focus_this_frame {
            self.query_suggest.close();
        }
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

            // Which engine actually ran — shown wherever the query's outcome
            // is, so a result or error is never ambiguous about which
            // dialect produced it, especially in "auto" mode.
            let engine_suffix = self.last_resolved_engine.map(|k| {
                if self.query_engine.is_none() {
                    format!(" [{} · auto]", k.label())
                } else {
                    format!(" [{}]", k.label())
                }
            });

            if self.query_running {
                ui.label(format!(
                    "Query running…{}",
                    engine_suffix.as_deref().unwrap_or("")
                ));
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
                s.push_str(engine_suffix.as_deref().unwrap_or(""));
                ui.label(s);
                if self.results_truncated
                    && ui
                        .add_enabled(!self.query_running, egui::Button::new("Expand All"))
                        .on_hover_text(
                            "Re-run the query without the live-preview cap, fetching all \
                             results into memory.",
                        )
                        .clicked()
                {
                    self.expand_results();
                }
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
                    format!(
                        "Query error: {err}{}",
                        engine_suffix.as_deref().unwrap_or("")
                    ),
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
    /// the results actually change (`results_text_dirty`), not every frame,
    /// and rendered on the worker thread, bounded to `TEXT_VIEW_NODE_BUDGET`
    /// nodes — a single query result can itself be arbitrarily large (e.g.
    /// `.` over a multi-GB document), so `LIVE_PREVIEW_CAP`'s item-count cap
    /// alone doesn't bound this. That node budget lifts too, to `usize::MAX`,
    /// once `results_cap` is unbounded (i.e. "Expand All" has run).
    fn results_text_view(&mut self, ui: &mut egui::Ui) {
        let node_budget = if self.results_cap == usize::MAX {
            usize::MAX
        } else {
            TEXT_VIEW_NODE_BUDGET
        };
        if self.results_text_dirty && !self.results_text_pending {
            self.results_text_pending = true;
            self.results_text_dirty = false;
            let _ = self.cmd_tx.send(Command::RenderText {
                target: worker::TextTarget::Results(self.results.clone()),
                node_budget,
                gen: self.results_text_gen,
            });
        }

        if self.results_text_pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Rendering…");
            });
            return;
        }

        if self.results_text_truncated {
            ui.horizontal(|ui| {
                ui.weak(format!(
                    "Showing the first {TEXT_VIEW_NODE_BUDGET} nodes — use Tree view, or Save… for the full results."
                ));
                if ui
                    .add_enabled(!self.query_running, egui::Button::new("Expand All"))
                    .on_hover_text(
                        "Re-run the query without the node-count cap, rendering the complete \
                         text.",
                    )
                    .clicked()
                {
                    self.expand_results();
                }
            });
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
    /// toggle as the results panel. For a *pasted* document the text is
    /// directly editable ("Apply", or Ctrl+Enter, re-parses the edited
    /// buffer in place), rendered synchronously since typed/pasted input is
    /// inherently human-sized — an editable buffer must hold the full,
    /// exact text anyway, so there's nothing to bound. Opened files and URL
    /// downloads are read-only and potentially huge, so those render on the
    /// worker thread, bounded to `TEXT_VIEW_NODE_BUDGET` nodes, instead of
    /// blocking the UI thread on a full-document stringify.
    fn source_text_view(&mut self, ui: &mut egui::Ui, doc: &Arc<Document>) {
        if matches!(&doc.source, DocumentSource::Pasted) {
            if self.source_text_dirty {
                self.source_text_cache = serde_json::to_string_pretty(&doc.root)
                    .unwrap_or_else(|e| format!("<failed to render source as text: {e}>"));
                self.source_text_dirty = false;
            }

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
            if self.source_text_dirty && !self.source_text_pending {
                self.source_text_pending = true;
                self.source_text_dirty = false;
                let _ = self.cmd_tx.send(Command::RenderText {
                    target: worker::TextTarget::Source(doc.clone()),
                    node_budget: TEXT_VIEW_NODE_BUDGET,
                    gen: self.source_text_gen,
                });
            }

            if self.source_text_pending {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Rendering…");
                });
                return;
            }

            if self.source_text_truncated {
                ui.weak(format!(
                    "Showing the first {TEXT_VIEW_NODE_BUDGET} nodes — use Tree view, or Save… for the full document."
                ));
            }
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

/// Reposition a `TextEdit`'s cursor from outside the widget itself (used
/// after `apply_suggestion` splices its text), and give it focus. Egui
/// keys a `TextEdit`'s cursor/selection state by widget `Id` in its own
/// persisted memory, separate from the string it's editing, so moving the
/// cursor after a programmatic edit means writing that state back directly
/// rather than through anything `TextEdit`'s own builder exposes.
fn set_text_edit_cursor(ctx: &egui::Context, id: egui::Id, char_idx: usize) {
    use egui::text::{CCursor, CCursorRange};
    use egui::widgets::text_edit::TextEditState;

    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(char_idx))));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
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

/// Small 💡 button for the (experimental) query-box autocomplete feature —
/// same single-icon-plus-hover-text shape as `theme_toggle_button`, but
/// dimmed rather than swapped for a different glyph when off: there's no
/// obvious "unlit bulb" icon to pair it with the way sun/moon pairs for
/// light/dark.
fn autocomplete_toggle_button(ui: &mut egui::Ui, suggest: &mut QuerySuggest) {
    let enabled = suggest.enabled;
    let icon = egui::RichText::new("💡");
    let icon = if enabled {
        icon
    } else {
        icon.color(ui.visuals().weak_text_color())
    };
    let tooltip = if enabled {
        "Autocomplete suggestions — experimental\n\
         On: click to turn off, or press Esc while a suggestion is showing."
    } else {
        "Autocomplete suggestions — experimental\n\
         Off: click to turn on."
    };
    if ui.button(icon).on_hover_text(tooltip).clicked() {
        suggest.set_enabled(!enabled);
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
        egui::Panel::top("query_bar")
            .resizable(true)
            .default_size(100.0)
            .min_size(60.0)
            .show(ui, |ui| self.query_bar(ui));
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
