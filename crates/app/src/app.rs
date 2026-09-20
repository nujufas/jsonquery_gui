use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use jsonquery_core::{
    path_string, resolve, Document, DocumentSource, NodePath, PathSegment, SourceMatches,
};
use serde_json::Value;

use crate::query_highlight;
use crate::query_suggest::{apply_suggestion, QuerySuggest};
use crate::tree_view::{RowAction, TreeView};
use crate::tutorial::{LoadRequest, Tutorial};
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

/// Width the toolbar keeps free to the right of its source field: the "…",
/// "Load" and "Clear" buttons, the byte size, and the icon buttons pinned to
/// the far right.
const TOOLBAR_TRAILING_RESERVE: f32 = 380.0;
/// Extra width reserved while the "(N NDJSON records)" note is showing.
const NDJSON_NOTE_WIDTH: f32 = 150.0;

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
    /// The results row the in-flight (or last) "Find in Source" was asked
    /// about, as a jq-style path — the heading of its candidate list.
    find_row: String,

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

    /// The tutorial window (a second native window) and its state.
    tutorial: Tutorial,
    /// The tutorial's "▶ Try it" replaced the source document and wants the
    /// query run as soon as that new document has loaded (loading is
    /// asynchronous, and finishing it cancels any query already running).
    run_after_load: bool,

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
    /// A "Find"/"Find All" over Results arrived while `results_truncated`
    /// was still true — deferred until the "Expand All" rerun it triggered
    /// finishes, so the search covers the complete results rather than the
    /// capped preview. Holds what the search is for.
    pending_results_search: Option<SearchPurpose>,
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

    /// The toolbar's source field: typed or pasted by the user (a URL or a
    /// local path), and set to name the source whenever one is opened or
    /// loads, so it always says what is showing.
    source_input: String,

    /// State for the "Search…" popup, which stays open while "Find" steps
    /// through the matches one at a time.
    show_search_dialog: bool,
    search_input: String,
    search_regex: bool,
    search_target: PanelKind,
    /// Ctrl+F / "Search…" asked for the Find field to take keyboard focus
    /// (with its text selected) the next time the popup is drawn.
    focus_search_field: bool,
    search_gen: u64,
    searching: bool,
    search_error: Option<String>,
    /// What the in-flight search (`searching`) was asked for.
    search_request: Option<SearchRequest>,
    /// The last completed search, and where "Find" has got to in it.
    find_cursor: Option<FindCursor>,

    /// The bottom panel — `Some` while it is shown: every match of a "Find
    /// All", or the candidates of a "Find in Source".
    hit_list: Option<HitList>,
}

/// What a "Search…" run looked for — "Find" and "Find All" re-search whenever
/// the popup no longer matches the query the hits were found for.
#[derive(Clone, PartialEq, Eq)]
struct SearchQuery {
    target: PanelKind,
    text: String,
    regex: bool,
}

/// The hits of the last completed search, in document order, and the one
/// "Find" revealed last — Notepad++'s "Find Next" position.
struct FindCursor {
    query: SearchQuery,
    hits: Vec<NodePath>,
    /// Index into `hits`; `None` until the first hit has been revealed.
    pos: Option<usize>,
    /// The last step ran past the final hit and came back to the first.
    wrapped: bool,
}

/// What a search from the dialog is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchPurpose {
    /// "Find": reveal the first match, then step on through the rest.
    Step,
    /// "Find All": list every match in the bottom panel.
    List,
}

/// A search that has been sent to the worker thread.
struct SearchRequest {
    query: SearchQuery,
    purpose: SearchPurpose,
}

/// The bottom panel: tree nodes to pick from, and what they are a list of.
struct HitList {
    kind: HitListKind,
    matches: Vec<SearchMatch>,
    /// Which entry is the one currently revealed in its tree — drawn
    /// highlighted in the panel: the best guess of a "Find in Source"
    /// (revealed as soon as it's found), then whatever was clicked, or
    /// stepped to with "Find".
    selected: Option<usize>,
}

enum HitListKind {
    /// Every match of a "Find All" for this query, in document order.
    Search(SearchQuery),
    /// The candidates of a "Find in Source" for the results row `row` (a
    /// jq-style path), best first.
    FindInSource {
        row: String,
        /// The text searched for when no node held an equal value (the row's
        /// value was computed), so the list is only an approximation.
        searched_for: Option<String>,
    },
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
            find_row: String::new(),
            query_text: String::new(),
            query_gen: 0,
            active_cancel: None,
            query_running: false,
            query_error: None,
            query_engine: None,
            last_resolved_engine: None,
            query_suggest: QuerySuggest::default(),
            tutorial: Tutorial::default(),
            run_after_load: false,
            results: Value::Array(Vec::new()),
            results_item_errors: 0,
            last_item_error: None,
            results_count_so_far: 0,
            results_truncated: false,
            results_cap: LIVE_PREVIEW_CAP,
            last_query_elapsed: None,
            last_query_cancelled: false,
            pending_results_search: None,
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
            source_input: String::new(),
            show_search_dialog: false,
            search_input: String::new(),
            search_regex: false,
            search_target: PanelKind::Source,
            focus_search_field: false,
            search_gen: 0,
            searching: false,
            search_error: None,
            search_request: None,
            find_cursor: None,
            hit_list: None,
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
                    self.pending_results_search = None;
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

                    // Pasted JSON has no address to name, so the field goes
                    // back to empty, ready for the next source.
                    self.source_input = match &doc.source {
                        DocumentSource::Pasted => String::new(),
                        source => source.label(),
                    };
                    self.doc = Some(doc);
                    self.source_tree.reset();
                    self.invalidate_source_text();

                    if std::mem::take(&mut self.run_after_load) {
                        self.run_query();
                    }
                }
                Event::LoadError(e) => {
                    self.loading = false;
                    self.load_error = Some(e);
                    self.run_after_load = false;
                }
                Event::Saved(path) => {
                    self.save_error = None;
                    self.last_saved = Some(path);
                }
                Event::SaveError(e) => {
                    self.last_saved = None;
                    self.save_error = Some(e);
                }
                Event::Found { gen, matches } => {
                    if gen != self.find_gen {
                        continue;
                    }
                    self.finding = false;
                    self.find_message = None;
                    let SourceMatches {
                        paths,
                        searched_for,
                    } = matches;
                    let Some(best) = paths.first().cloned() else {
                        self.find_message = Some("Not found in source.".to_string());
                        continue;
                    };
                    // An exact match is revealed straight away: the only one,
                    // or the best guess of several. A text-search fallback is
                    // approximate, so it's only listed — it shouldn't yank
                    // the Source tree somewhere on a guess.
                    if searched_for.is_none() {
                        self.source_tree.reveal(best);
                        self.source_view = ViewMode::Tree;
                    }
                    if searched_for.is_some() || paths.len() > 1 {
                        self.show_find_list(paths, searched_for);
                    }
                }
                Event::SearchDone { gen, matches } => {
                    if gen != self.search_gen {
                        continue;
                    }
                    self.searching = false;
                    self.search_error = None;
                    let Some(SearchRequest { query, purpose }) = self.search_request.take() else {
                        continue;
                    };
                    self.find_cursor = Some(FindCursor {
                        query,
                        hits: matches,
                        pos: None,
                        wrapped: false,
                    });
                    match purpose {
                        SearchPurpose::Step => self.reveal_first_hit_if_current(),
                        SearchPurpose::List => self.show_search_list(),
                    }
                }
                Event::SearchError { gen, error } => {
                    if gen != self.search_gen {
                        continue;
                    }
                    self.searching = false;
                    self.search_error = Some(error);
                    // Remembered like any other outcome, so "Find" doesn't
                    // re-run a query that is known not to compile.
                    if let Some(SearchRequest { query, .. }) = self.search_request.take() {
                        self.find_cursor = Some(FindCursor {
                            query,
                            hits: Vec::new(),
                            pos: None,
                            wrapped: false,
                        });
                    }
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
                        if self.pending_results_search.take().is_some() {
                            self.searching = false;
                            self.search_error =
                                Some("Cancelled while fetching full results.".to_string());
                        }
                        self.pending_save_results = None;
                    } else {
                        if let Some(purpose) = self.pending_results_search.take() {
                            self.run_search(purpose);
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
                    if self.pending_results_search.take().is_some() {
                        self.searching = false;
                        self.search_error = Some(error);
                    }
                    self.pending_save_results = None;
                }
            }
        }
    }

    // `open_file` and `open_url` put what they were asked to open in the
    // source field, so a load that fails still shows what was attempted
    // (and a dropped file shows up there too).
    fn open_file(&mut self, path: PathBuf) {
        self.source_input = path.display().to_string();
        let _ = self.cmd_tx.send(Command::OpenFile(path));
    }

    fn open_text(&mut self, text: String) {
        let _ = self.cmd_tx.send(Command::OpenText(text));
    }

    fn open_url(&mut self, url: String) {
        self.source_input.clone_from(&url);
        let _ = self.cmd_tx.send(Command::OpenUrl(url));
    }

    /// Load whatever the toolbar's source field holds — a URL or a local
    /// path (see [`parse_source_input`]).
    fn load_source_input(&mut self) {
        match parse_source_input(&self.source_input) {
            Some(SourceInput::Url(url)) => self.open_url(url),
            Some(SourceInput::Path(path)) => self.open_file(path),
            None => {}
        }
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

    /// "Find in Source": work out where the results row at `node_path` came
    /// from in the loaded source document (`jsonquery_core::locate`). One
    /// exact hit is expanded/scrolled/highlighted there; several — or a
    /// text-search fallback for a computed value — are listed in the bottom
    /// panel (`Event::Found`). The lookup runs on the worker thread since
    /// the source document can be large, and is discarded if it's stale by
    /// the time it comes back — same `gen` pattern as queries.
    fn navigate_to_source(&mut self, node_path: &NodePath) {
        let Some(doc) = self.doc.clone() else { return };
        // A results row's first segment is which query output it belongs to;
        // the results root, the whole result set, isn't anything the source
        // could hold — and isn't worth cloning to find that out.
        let Some((&PathSegment::Index(nth), rel)) = node_path.split_first() else {
            self.find_message = Some("Not found in source.".to_string());
            return;
        };
        let Some(target) = resolve(&self.results, node_path) else {
            return;
        };
        self.find_gen += 1;
        self.finding = true;
        self.find_message = None;
        self.find_row = path_string(node_path);
        let _ = self.cmd_tx.send(Command::FindInSource {
            doc,
            target: target.clone(),
            nth,
            rel: rel.to_vec(),
            gen: self.find_gen,
        });
    }

    /// List `paths` — Source nodes, best guess first — in the bottom panel as
    /// the candidates for the results row `self.find_row`, in place of any
    /// list it showed before.
    fn show_find_list(&mut self, paths: Vec<NodePath>, searched_for: Option<String>) {
        // Supersede any "Find" still in flight, so its late first hit can't
        // pull the Source tree away from the candidate revealed here.
        self.search_gen += 1;
        self.searching = false;
        self.search_error = None;
        let root = self.doc.as_ref().map(|d| &d.root);
        let matches = build_search_matches(PanelKind::Source, root, paths);
        // Only an exact match was revealed (see `Event::Found`), and it was
        // the first entry.
        let selected = searched_for.is_none().then_some(0);
        self.hit_list = Some(HitList {
            kind: HitListKind::FindInSource {
                row: self.find_row.clone(),
                searched_for,
            },
            matches,
            selected,
        });
    }

    /// "Find All": list every hit of `self.find_cursor` in the bottom panel,
    /// in place of any list it showed before — with the hit "Find" is at, if
    /// it has been stepping, selected.
    fn show_search_list(&mut self) {
        let Some(cursor) = &self.find_cursor else {
            return;
        };
        let root = match cursor.query.target {
            PanelKind::Source => self.doc.as_ref().map(|d| &d.root),
            PanelKind::Results => Some(&self.results),
        };
        let matches = build_search_matches(cursor.query.target, root, cursor.hits.clone());
        self.hit_list = Some(HitList {
            kind: HitListKind::Search(cursor.query.clone()),
            matches,
            selected: cursor.pos,
        });
    }

    /// Ctrl+F (search) and Ctrl+S (save) act on `self.focused_panel` — the
    /// Source or Results panel that last saw a click — the same way a
    /// desktop app's menu-bar Find/Save act on whichever document window is
    /// frontmost, rather than requiring a dedicated shortcut per panel.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            self.tutorial.open_or_focus(ctx);
        }

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

    /// Open the "Search…" dialog for `target`'s tree with the Find field
    /// ready to type into, discarding whatever text was left over from a
    /// previous search. Asked again for the tree it is already searching
    /// (Ctrl+F while it's open), it keeps the text — selected, so typing
    /// replaces it — as Notepad++'s Find dialog does.
    fn open_search_dialog(&mut self, target: PanelKind) {
        if !self.show_search_dialog || self.search_target != target {
            self.search_target = target;
            self.search_input.clear();
        }
        self.show_search_dialog = true;
        self.focus_search_field = true;
    }

    /// What the dialog is asking for right now.
    fn dialog_query(&self) -> SearchQuery {
        SearchQuery {
            target: self.search_target,
            text: self.search_input.clone(),
            regex: self.search_regex,
        }
    }

    /// Whether `self.find_cursor` holds the outcome of the query the dialog
    /// is asking for now, so "Find"/"Find All" can use it instead of searching
    /// again.
    fn find_cursor_is_current(&self) -> bool {
        self.find_cursor
            .as_ref()
            .is_some_and(|c| c.query == self.dialog_query())
    }

    /// The dialog's "Find" (button or Enter) — Notepad++'s "Find Next": reveal
    /// the match after the one revealed last, wrapping from the final match
    /// back to the first. The first Find for a query searches the whole tree
    /// on the worker thread and reveals the first match once that finishes
    /// (`Event::SearchDone`); a changed query starts over from there.
    fn find_next(&mut self) {
        if self.search_input.trim().is_empty() || self.searching {
            return;
        }
        if self.find_cursor_is_current() {
            self.step_find();
        } else {
            self.run_search(SearchPurpose::Step);
        }
    }

    /// The dialog's "Find All": list every match in the bottom panel, where
    /// clicking one reveals it. Searches the tree first, unless "Find" has
    /// already done so for this query.
    fn find_all(&mut self) {
        if self.search_input.trim().is_empty() || self.searching {
            return;
        }
        if !self.find_cursor_is_current() {
            self.run_search(SearchPurpose::List);
        } else if self.search_error.is_none() {
            self.show_search_list();
        }
    }

    /// Reveal the next hit of `self.find_cursor` in its tree, moving the
    /// cursor on to it.
    fn step_find(&mut self) {
        let Some(cursor) = &mut self.find_cursor else {
            return;
        };
        let count = cursor.hits.len();
        let (next, wrapped) = match cursor.pos {
            None => (0, false),
            Some(i) if i + 1 < count => (i + 1, false),
            Some(_) => (0, true),
        };
        let Some(path) = cursor.hits.get(next).cloned() else {
            return;
        };
        cursor.pos = Some(next);
        cursor.wrapped = wrapped;
        let target = cursor.query.target;
        // Keep the highlight of a "Find All" list on the hit just revealed.
        if let Some(HitList {
            kind: HitListKind::Search(query),
            selected,
            ..
        }) = &mut self.hit_list
        {
            if *query == cursor.query {
                *selected = Some(next);
            }
        }
        self.reveal_in_tree(target, path);
    }

    /// A search just finished: reveal its first hit, provided the dialog is
    /// still open and still asking for that query — the user may have closed
    /// it, or retyped, while the worker was searching.
    fn reveal_first_hit_if_current(&mut self) {
        let current = self.show_search_dialog
            && self
                .find_cursor
                .as_ref()
                .is_some_and(|c| c.query == self.dialog_query());
        if current {
            self.step_find();
        }
    }

    /// Expand, scroll to and highlight `path` in `target`'s tree, switching
    /// that panel to its Tree view if it was showing Text.
    fn reveal_in_tree(&mut self, target: PanelKind, path: NodePath) {
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

    /// Send the current search dialog's query to the worker thread, over
    /// whichever tree it was opened for, for `purpose`. Searching Results
    /// while the live preview is still capped would silently miss matches
    /// beyond the cap, so that case first re-runs the query unbounded
    /// (`expand_results`) and retries the search once it completes
    /// (`pending_results_search`, handled in `Event::QueryDone`).
    fn run_search(&mut self, purpose: SearchPurpose) {
        if self.search_target == PanelKind::Results && self.results_truncated {
            self.expand_results();
            self.pending_results_search = Some(purpose);
            self.searching = true;
            self.search_error = None;
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
        self.search_request = Some(SearchRequest {
            query: self.dialog_query(),
            purpose,
        });
        let _ = self.cmd_tx.send(Command::Search {
            root,
            text: self.search_input.clone(),
            regex: self.search_regex,
            gen: self.search_gen,
        });
    }

    /// Discard any in-flight or completed search, and the bottom panel's
    /// candidates — the tree they were found in just changed out from under
    /// them (a new document loaded, a new query run, or the source cleared).
    fn invalidate_search(&mut self) {
        self.search_gen += 1;
        self.searching = false;
        self.search_error = None;
        self.search_request = None;
        self.find_cursor = None;
        self.clear_hits();
    }

    /// Hide the bottom panel, dropping its list.
    fn clear_hits(&mut self) {
        self.hit_list = None;
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
        self.pending_results_search = None;
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
        self.pending_results_search = None;
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

    /// Apply what the tutorial window asked for: pin the engine to the
    /// lesson's dialect (a bare `office.city` would otherwise be
    /// auto-detected as jq), put the query in the box, and/or replace the
    /// source document — running the query once that document has loaded.
    fn apply_tutorial_request(&mut self, request: LoadRequest) {
        if let Some(query) = request.query {
            self.query_engine = Some(request.kind);
            self.query_text = query.to_owned();
            self.query_suggest.close();
        }
        match request.data {
            Some(data) => {
                self.run_after_load = request.run;
                self.open_text(data.to_owned());
            }
            None if request.run => self.run_query(),
            None => {}
        }
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

    /// One row: a source field that takes a typed or pasted URL or local
    /// path, then "…" (browse for a file), "Load" and "Clear", then what is
    /// loaded (size, NDJSON note) and the icon buttons pinned to the right.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Source:");

            // The field gets the row minus a reserve for everything after it
            // (the three buttons, the byte size and an occasional NDJSON
            // note, and the icon buttons pinned to the far right) rather
            // than a fixed width — a long URL should get to use the room a
            // short one leaves empty.
            let ndjson_note = self.doc.as_ref().is_some_and(|d| d.top_level_values > 1);
            let reserve =
                TOOLBAR_TRAILING_RESERVE + if ndjson_note { NDJSON_NOTE_WIDTH } else { 0.0 };
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.source_input)
                    .desired_width((ui.available_width() - reserve).max(120.0))
                    .font(egui::TextStyle::Monospace)
                    .hint_text("URL or local path…"),
            );
            // Enter takes the focus out of a single-line field, which is how
            // it's told apart from other keys.
            let mut load = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            if ui
                .button("…")
                .on_hover_text("Browse for a local JSON file")
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("JSON", &["json", "ndjson", "jsonl", "log", "txt"])
                    .pick_file()
                {
                    self.open_file(path);
                }
            }
            load |= ui
                .add_enabled(
                    !self.source_input.trim().is_empty(),
                    egui::Button::new("Load"),
                )
                .on_hover_text("Load the URL or file path in the field (Enter)")
                .clicked();
            if load {
                self.load_source_input();
            }

            let has_source = self.doc.is_some() || self.loading || self.load_error.is_some();
            if ui
                .add_enabled(
                    has_source || !self.source_input.is_empty(),
                    egui::Button::new("Clear"),
                )
                .on_hover_text("Empty the field and unload the current document")
                .clicked()
            {
                if has_source {
                    self.clear_source();
                }
                self.source_input.clear();
            }
            ui.separator();

            if let Some(doc) = &self.doc {
                if matches!(doc.source, DocumentSource::Pasted) {
                    ui.label(doc.source.label());
                }
                ui.weak(human_bytes(doc.byte_len));
                if doc.top_level_values > 1 {
                    ui.weak(format!("({} NDJSON records)", doc.top_level_values));
                }
            } else if self.loading {
                ui.spinner();
                ui.label("Loading…");
            }

            // Claims whatever width is left after everything above, so the
            // theme toggle (and, just to its left, the autocomplete toggle
            // and the tutorial button) sit pinned at the top-right corner
            // regardless of how long the status text is.
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    theme_toggle_button(ui);
                    autocomplete_toggle_button(ui, &mut self.query_suggest);
                    tutorial_button(ui, &mut self.tutorial);
                },
            );
        });
    }

    /// Popup prompting for a search query; shown when `show_search_dialog`
    /// is set by Ctrl+F or a row's "Search…" context-menu item. Searches the
    /// whole tree it was opened for (`self.search_target`), not just the row
    /// that was right-clicked. Like Notepad++'s Find dialog it stays open:
    /// "Find" steps through the matches one at a time, "Find All" lists them
    /// all in the bottom panel; Cancel or Escape closes it.
    fn search_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_search_dialog {
            return;
        }

        let field_id = egui::Id::new("search_find_field");
        let mut open = true;
        let mut cancel = false;
        egui::Window::new(format!("Search — {}", self.search_target.label()))
            .id(egui::Id::new("search_dialog"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_max_width(340.0);
                ui.label("Find:");
                // Focus (and select) the field before it is added, so it is
                // typable from the frame the dialog first shows. Not during
                // the window's invisible first "sizing pass": egui drops the
                // focus of every widget it lays out there.
                if !ui.is_sizing_pass() && std::mem::take(&mut self.focus_search_field) {
                    let len = self.search_input.chars().count();
                    select_text_edit_range(ui.ctx(), field_id, 0, len);
                }
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.search_input)
                        .id(field_id)
                        .desired_width(320.0)
                        .hint_text("text to find…"),
                );
                // Enter and Escape each take the focus out of a single-line
                // field, which is how they're told apart from other keys.
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let escape = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
                let regex = ui.checkbox(&mut self.search_regex, "Regex");
                if field.changed() || regex.changed() {
                    self.search_error = None;
                }
                if regex.changed() {
                    // So Enter still means "Find" after ticking the box.
                    field.request_focus();
                }
                let mut find = enter;
                let mut find_all = false;
                ui.horizontal(|ui| {
                    let has_text = !self.search_input.trim().is_empty();
                    find |= ui
                        .add_enabled(has_text, egui::Button::new("Find"))
                        .clicked();
                    find_all = ui
                        .add_enabled(has_text, egui::Button::new("Find All"))
                        .clicked();
                    cancel = ui.button("Cancel").clicked() || escape;
                });
                if find {
                    self.find_next();
                } else if find_all {
                    self.find_all();
                }
                if find || find_all {
                    // Keep typing, or pressing Enter for the next match,
                    // possible right after either.
                    field.request_focus();
                }
                self.find_status(ui);
            });

        if cancel || !open {
            self.show_search_dialog = false;
        }
    }

    /// The dialog's bottom line: where "Find" stands for the query in the
    /// field. Always takes a line's height, so the dialog doesn't jump as the
    /// message comes and goes.
    fn find_status(&self, ui: &mut egui::Ui) {
        let red = egui::Color32::from_rgb(220, 80, 80);
        let cursor = self
            .find_cursor
            .as_ref()
            .filter(|c| c.query == self.dialog_query());
        if self.searching {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Searching…");
            });
        } else if let Some(err) = &self.search_error {
            ui.colored_label(red, format!("Search error: {err}"));
        } else if cursor.is_some_and(|c| c.hits.is_empty()) {
            ui.colored_label(red, "No matches found.");
        } else if let Some((cursor, pos)) = cursor.and_then(|c| Some((c, c.pos?))) {
            let mut text = format!("{} of {}", pos + 1, cursor.hits.len());
            if cursor.wrapped {
                text.push_str(" — wrapped around to the top");
            }
            ui.label(text);
        } else if let Some(cursor) = cursor {
            // Searched, but "Find" hasn't stepped to any of them: a "Find All".
            let n = cursor.hits.len();
            ui.label(format!("{n} match{}", if n == 1 { "" } else { "es" }));
        } else {
            ui.label(" ");
        }
    }

    /// The bottom panel (`self.hit_list`): every match of a "Find All", or the
    /// candidates of a "Find in Source" that had more than one answer. Clicking
    /// an entry reveals it in its tree.
    fn hit_list_panel(&mut self, ui: &mut egui::Ui) {
        let Some(list) = &self.hit_list else {
            return;
        };
        let heading = match &list.kind {
            HitListKind::FindInSource { row, .. } => format!("Find in Source — {row}"),
            HitListKind::Search(query) => {
                let mut heading = format!("Search results — {}", query.target.label());
                if !query.text.is_empty() {
                    heading.push_str(&format!(" \"{}\"", query.text));
                }
                if query.regex {
                    heading.push_str(" (regex)");
                }
                heading
            }
        };
        let mut count = format!("{} match(es)", list.matches.len());
        match &list.kind {
            HitListKind::FindInSource {
                searched_for: Some(text),
                ..
            } => count.push_str(&format!(" — no exact match, text search for \"{text}\"")),
            HitListKind::FindInSource { .. } => count.push_str(" — best match first"),
            HitListKind::Search(_) => {}
        }

        let mut close = false;
        ui.horizontal(|ui| {
            ui.strong(heading);
            ui.weak(count);

            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| close = ui.button("Close").clicked(),
            );
        });
        if close {
            self.clear_hits();
            return;
        }
        ui.separator();

        if list.matches.is_empty() {
            ui.weak("No matches found.");
            return;
        }

        let mut reveal: Option<(usize, PanelKind, NodePath)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, m) in list.matches.iter().enumerate() {
                    let text = format!(
                        "[{}]  {}   {}",
                        m.target.label(),
                        path_string(&m.path),
                        m.preview
                    );
                    // Reserve the highlight's slot before the label so it's
                    // painted behind the text; same tint the trees use for a
                    // revealed row.
                    let full_width = ui.available_width();
                    let highlight = ui.painter().add(egui::Shape::Noop);
                    let resp = ui.add(
                        egui::Label::new(egui::RichText::new(text).monospace())
                            .sense(egui::Sense::click()),
                    );
                    if list.selected == Some(i) {
                        let row = egui::Rect::from_min_size(
                            resp.rect.min,
                            egui::vec2(full_width.max(resp.rect.width()), resp.rect.height()),
                        );
                        ui.painter().set(
                            highlight,
                            egui::Shape::rect_filled(
                                row,
                                2.0,
                                ui.visuals().selection.bg_fill.linear_multiply(0.5),
                            ),
                        );
                    }
                    if resp.clicked() {
                        reveal = Some((i, m.target, m.path.clone()));
                    }
                }
            });

        if let Some((i, target, path)) = reveal {
            if let Some(list) = &mut self.hit_list {
                list.selected = Some(i);
                // "Find" carries on from the entry picked, as Notepad++'s
                // Find Next carries on from where the caret was left.
                if let (HitListKind::Search(query), Some(cursor)) =
                    (&list.kind, &mut self.find_cursor)
                {
                    if cursor.query == *query {
                        cursor.pos = Some(i);
                        cursor.wrapped = false;
                    }
                }
            }
            self.reveal_in_tree(target, path);
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

        ui.weak("Drag & drop a file anywhere, or enter a URL or path above.");
        ui.add_space(4.0);

        // Fill whatever space is left in the panel rather than a fixed row
        // count, so the box grows/shrinks with the window instead of leaving
        // dead space below it. Captured before opening the ScrollArea below
        // (rather than calling `ui.available_size()` from inside it) so a
        // short/empty paste still fills the visible panel instead of
        // shrinking to a single line -- a vertical ScrollArea reports an
        // effectively unbounded height for its content.
        let avail = ui.available_size();
        // A bare `TextEdit` has no scrolling of its own -- confirmed during
        // implementation that pasting a long single-line document (no
        // scrollbar, mouse wheel did nothing, even Ctrl+End didn't scroll
        // the cursor into view) needs this explicit ScrollArea to be
        // navigable at all.
        let resp = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_sized(
                    avail,
                    egui::TextEdit::multiline(&mut self.paste_text)
                        .code_editor()
                        .hint_text("Paste JSON here…"),
                )
            })
            .inner;

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

        // Tints each step of the query, like the tutorial's examples. Reads
        // the text from the widget (`self.query_text` is borrowed by it) and
        // copies the engine choice out for the same reason.
        let engine = self.query_engine;
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            query_highlight::galley(ui, engine, text.as_str(), wrap_width)
        };

        let output = egui::TextEdit::multiline(&mut self.query_text)
            .id(id)
            .desired_rows(desired_rows)
            .desired_width(f32::INFINITY)
            .code_editor()
            .layouter(&mut layouter)
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
            // See the same-shaped fix in `paste_area` -- a bare `TextEdit`
            // doesn't scroll on its own, so a long pasted/edited document had
            // no way to reveal anything past the first screenful.
            let avail = ui.available_size();
            let resp = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_sized(
                        avail,
                        egui::TextEdit::multiline(&mut self.source_text_cache).code_editor(),
                    )
                })
                .inner;
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
    select_text_edit_range(ctx, id, char_idx, char_idx);
}

/// Like [`set_text_edit_cursor`], selecting the chars from `start` to `end`
/// (both char offsets) — an empty range is just a cursor.
fn select_text_edit_range(ctx: &egui::Context, id: egui::Id, start: usize, end: usize) {
    use egui::text::{CCursor, CCursorRange};
    use egui::widgets::text_edit::TextEditState;

    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(start),
        CCursor::new(end),
    )));
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
         On: click to turn off. Esc closes the list that's showing; the next \
         keystroke brings suggestions back."
    } else {
        "Autocomplete suggestions — experimental\n\
         Off: click to turn on."
    };
    if ui.button(icon).on_hover_text(tooltip).clicked() {
        suggest.set_enabled(!enabled);
    }
}

/// Small 📖 button that opens the tutorial window (or brings it to the front
/// if it's already open) — same single-icon-plus-hover-text shape as the
/// autocomplete and theme buttons beside it. F1 does the same.
fn tutorial_button(ui: &mut egui::Ui, tutorial: &mut Tutorial) {
    if ui
        .button("📖")
        .on_hover_text(
            "Tutorial — learn jq, JSON Pointer, JSONPath and JMESPath, with examples you can \
             load and run here.\nF1 also opens it.",
        )
        .clicked()
    {
        tutorial.open_or_focus(ui.ctx());
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();
        let hovering_drop = self.handle_drag_and_drop(ui);
        // Shortcuts first, so Ctrl+F's dialog is drawn (and focused) in the
        // frame it was pressed rather than the one after.
        self.handle_shortcuts(ui.ctx());
        self.search_dialog(ui.ctx());

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("query_bar")
            .resizable(true)
            .default_size(100.0)
            .min_size(60.0)
            .show(ui, |ui| self.query_bar(ui));
        egui::Panel::bottom("status_bar").show(ui, |ui| self.status_bar(ui));
        if self.hit_list.is_some() {
            egui::Panel::bottom("search_results_panel")
                .resizable(true)
                .default_size(180.0)
                .show(ui, |ui| self.hit_list_panel(ui));
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

        if let Some(request) = self.tutorial.show(ui.ctx()) {
            self.apply_tutorial_request(request);
        }
    }
}

/// What the toolbar's source field asks to load.
#[derive(Debug, PartialEq)]
enum SourceInput {
    Url(String),
    Path(PathBuf),
}

/// Read the toolbar's source field: an `http://` or `https://` address is a
/// download, anything else a local path. Surrounding whitespace and one pair
/// of quotes (which a file manager's "Copy as path" adds) are dropped, and a
/// leading `~` means the home directory. `None` for a blank field.
fn parse_source_input(text: &str) -> Option<SourceInput> {
    let text = text.trim();
    let text = ['"', '\'']
        .into_iter()
        .find_map(|quote| text.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(text)
        .trim();
    if text.is_empty() {
        return None;
    }
    let is_url = ["http://", "https://"].iter().any(|scheme| {
        text.get(..scheme.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
    });
    if is_url {
        return Some(SourceInput::Url(text.to_owned()));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    Some(SourceInput::Path(expand_home(
        text,
        home.map(PathBuf::from),
    )))
}

/// `~` or `~/rest` → `home` or `home/rest`; anything else (including `~user`,
/// or `~` when there's no home directory to expand it to) is left as typed.
fn expand_home(path: &str, home: Option<PathBuf>) -> PathBuf {
    let rest = path
        .strip_prefix('~')
        .filter(|rest| rest.is_empty() || rest.starts_with(['/', '\\']));
    match (rest, home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(path),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Option<SourceInput> {
        Some(SourceInput::Url(s.to_owned()))
    }

    #[test]
    fn source_input_http_and_https_are_downloads() {
        assert_eq!(
            parse_source_input("https://example.com/a.json?x=1"),
            url("https://example.com/a.json?x=1")
        );
        assert_eq!(
            parse_source_input("  http://localhost:8000/a.json \n"),
            url("http://localhost:8000/a.json")
        );
        // The scheme is case-insensitive; the rest is kept exactly as typed.
        assert_eq!(
            parse_source_input("HTTPS://Example.com/A.json"),
            url("HTTPS://Example.com/A.json")
        );
    }

    #[test]
    fn source_input_anything_else_is_a_local_path() {
        assert_eq!(
            parse_source_input("/tmp/data.json"),
            Some(SourceInput::Path(PathBuf::from("/tmp/data.json")))
        );
        assert_eq!(
            parse_source_input("data/records.ndjson"),
            Some(SourceInput::Path(PathBuf::from("data/records.ndjson")))
        );
        // Not a scheme this app downloads, so it isn't treated as a URL.
        assert_eq!(
            parse_source_input("ftp://host/a.json"),
            Some(SourceInput::Path(PathBuf::from("ftp://host/a.json")))
        );
    }

    #[test]
    fn source_input_strips_one_pair_of_quotes() {
        assert_eq!(
            parse_source_input("\"/tmp/my data.json\""),
            Some(SourceInput::Path(PathBuf::from("/tmp/my data.json")))
        );
        assert_eq!(
            parse_source_input("'https://example.com/a.json'"),
            url("https://example.com/a.json")
        );
        // A lone quote is part of the name, not a pair.
        assert_eq!(
            parse_source_input("/tmp/it's.json"),
            Some(SourceInput::Path(PathBuf::from("/tmp/it's.json")))
        );
    }

    #[test]
    fn source_input_blank_is_nothing_to_load() {
        assert_eq!(parse_source_input(""), None);
        assert_eq!(parse_source_input("  \t "), None);
        assert_eq!(parse_source_input("\"\""), None);
    }

    #[test]
    fn expand_home_only_touches_a_leading_tilde_directory() {
        let home = || Some(PathBuf::from("/home/me"));
        assert_eq!(
            expand_home("~/data/a.json", home()),
            PathBuf::from("/home/me/data/a.json")
        );
        assert_eq!(expand_home("~", home()), PathBuf::from("/home/me/"));
        assert_eq!(
            expand_home("~other/a.json", home()),
            PathBuf::from("~other/a.json")
        );
        assert_eq!(
            expand_home("/x/~/a.json", home()),
            PathBuf::from("/x/~/a.json")
        );
        assert_eq!(expand_home("~/a.json", None), PathBuf::from("~/a.json"));
    }
}
