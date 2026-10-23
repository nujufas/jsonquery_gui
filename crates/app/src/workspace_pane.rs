//! The workspace pane: the strip at the left of the main window that holds the
//! user's saved queries, schemas, patches, documents and web addresses
//! (`workspace.rs`), with their samples, in the folders they were put in.
//!
//! * The pane has a header (save what is in the box here, make a folder, a menu with
//!   import and export, fold it away), a line that says where the workspace is, a
//!   filter and the chips that narrow the list to one sort of item, the tree of
//!   folders and items, and under it the item that is picked: what it holds, its
//!   sample, and the buttons that put it where it belongs.
//! * A click picks a folder or an item, a double click opens it, a right click has
//!   the menu: open, rename, copy, move, delete — and a row can be dragged onto a
//!   folder to move it there (`drag.rs`).
//! * An item is put where it belongs: a query in the query box (and its
//!   language chosen), a schema in the Schema box of Validate, a patch in the
//!   Patch box of Patch, a document as the Source, a web address loaded from the web
//!   as the Source. *Open with sample* also opens the sample as the Source — or, for a
//!   schema or a patch, in the Document box — and runs it.
//! * Folded, the pane is a thin strip with the one button that opens it again; and
//!   until it is asked for it is not there at all, so that the window is as it has
//!   always been (`settings::WorkspaceView`).
//!
//! The pane never touches the rest of the app itself: what it wants done (open a
//! query, run a sample, write a document to a file) it says as an [`Action`], which
//! the app does, as the tutorial and the Tools do with theirs.
//!
//! The parts: `tree.rs` is the list of folders and items, `drag.rs` what moving a row
//! by dragging it needs, `forms.rs` the windows that save, make a folder, import,
//! export and ask, `icons.rs` the pictures.

mod drag;
mod forms;
mod icons;
mod tree;

#[cfg(test)]
pub(crate) use icons::button_id;

/// A row that is being carried, as a drag that nobody holds any more leaves it (the test of
/// what the list does with that).
#[cfg(test)]
pub(crate) fn carry_for_test(ctx: &eframe::egui::Context, path: &str, name: &str, is_folder: bool) {
    eframe::egui::DragAndDrop::set_payload(
        ctx,
        drag::Carried {
            path: path.to_owned(),
            name: name.to_owned(),
            is_folder,
        },
    );
}

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use jsonquery_core::Document;
use jsonquery_query::Kind as Engine;

use crate::pane_header;
use crate::settings::WorkspaceView;
use crate::text_menu;
use crate::tools::{Tool, Tools};
use crate::workspace::{Group, Workspace};

/// How wide the pane is the first time, in points.
pub const DEFAULT_WIDTH: f32 = 256.0;

/// The least the pane can be dragged to: the room that its header needs for its title
/// and its four buttons (narrower, the buttons are drawn over the end of the title).
pub const MIN_WIDTH: f32 = 208.0;

/// How wide the strip is that a folded pane leaves.
pub const RAIL_WIDTH: f32 = 26.0;

/// The narrowest a window can be for the pane to have room beside what the window
/// holds; in a narrower one the strip is all that is shown.
pub const MIN_WINDOW_WIDTH: f32 = 900.0;

/// How long a message stays under the list, in seconds.
const NOTICE_SECONDS: f64 = 8.0;

/// What is said of a web address's file that holds no web address.
const NO_ADDRESS: &str = "There is no web address in it";

/// The most of a query that is put in the box, from an item or from a file that is dropped
/// on the window: it is laid out whole, as the query box's text always is.
pub const QUERY_LIMIT: u64 = 1024 * 1024;

/// What a page of the Tools asks of the workspace: it has no pane of its own, and
/// leaves the asking to the window that has (see [`wish`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wish {
    /// Save what is in the Schema box of Validate.
    SaveSchema,
    /// Save what is in the Patch box of Patch.
    SavePatch,
    /// Show the schemas of the workspace, to pick one.
    PickSchema,
    /// Show the patches of the workspace, to pick one.
    PickPatch,
}

/// Ask the workspace for something, from wherever a window is drawn: the pane (and its
/// windows) are the main window's, which is brought forward, and shown, if it is not.
pub fn wish(ctx: &egui::Context, wish: Wish) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("workspace_wish"), wish));
    ctx.request_repaint_of(egui::ViewportId::ROOT);
}

/// The wish that was made since the last time it was asked for, if one was.
fn take_wish(ctx: &egui::Context) -> Option<Wish> {
    let id = egui::Id::new("workspace_wish");
    ctx.data_mut(|data| {
        let wish = data.get_temp::<Wish>(id);
        if wish.is_some() {
            data.remove::<Wish>(id);
        }
        wish
    })
}

/// What the pane asks the app to do.
pub enum Action {
    /// Put this query in the query box, in this language; and, if there is a
    /// sample, open it as the Source and run the query.
    OpenQuery {
        id: String,
        engine: Engine,
        text: String,
        sample: Option<PathBuf>,
    },
    /// Put this file in the box of `tool` that takes it (the schema of Validate,
    /// the patch of Patch), show the page and, if there is a sample, run it.
    UseFiles {
        tool: Tool,
        path: PathBuf,
        sample: Option<PathBuf>,
    },
    /// Open this file as the Source.
    OpenDocument(PathBuf),
    /// Load this web address as the Source, as if it had been typed in the Source field
    /// and Load pressed.
    OpenUrl(String),
    /// Write the document that is open to this file (the sample of an item that is
    /// being saved), and say when it is written.
    WriteDocument { doc: Arc<Document>, to: PathBuf },
    /// Keep the workspace in this folder from now on (none: in the usual place).
    SetFolder(Option<PathBuf>),
    /// Show more or less of the pane.
    SetView(WorkspaceView),
    /// Show this folder or file in the file manager.
    Reveal(PathBuf),
}

/// Which page the main window shows, which says what the pane is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Front {
    Query,
    Tool(Tool),
}

/// What the app lends the pane for a frame.
pub struct Env<'a> {
    /// Where the query pane was drawn last: a query that is carried from the list and
    /// put down there is opened. None while it is not in the window (popped out, or
    /// behind the page of a tool).
    pub query_area: Option<egui::Rect>,
    pub query_text: &'a str,
    /// The language the query box runs its text as.
    pub engine: Engine,
    /// The document that is open as the Source.
    pub doc: Option<&'a Arc<Document>>,
    /// What is in the Source field, which may be an address that has not been loaded.
    pub source_text: &'a str,
    /// The most a document may be to be kept as a sample (what the Tools take).
    pub sample_limit: u64,
    pub front: Front,
    pub tools: &'a Tools,
}

/// The item the query box was filled from.
struct Opened {
    id: String,
    /// What the box held then, to tell whether it has been changed since.
    text: String,
}

/// A message under the list.
struct Notice {
    text: String,
    error: bool,
    until: f64,
}

/// A save that waits for a document to be written to a file by the worker.
struct Waiting {
    staged: PathBuf,
    save: forms::PendingSave,
}

pub struct WorkspacePane {
    /// The folder the user chose; none for the usual one.
    chosen: Option<PathBuf>,
    /// The usual one, beside the settings.
    usual: Option<PathBuf>,
    workspace: Option<Workspace>,

    filter: String,
    group: Group,
    /// The folders that are open, by relative path.
    open_folders: HashSet<String>,
    /// The folders that start open have been opened for this workspace.
    opened_first_level: bool,
    /// The folder or item that is picked, by relative path.
    selected: Option<String>,
    /// The item being renamed in the list, and what its new name is so far.
    renaming: Option<(String, String)>,
    /// What the query box was last filled from.
    opened: Option<Opened>,
    /// What the pane was told the main window shows, to see it change.
    front: Option<Front>,
    /// The picked item's first lines, and for which version of it.
    preview: Option<(String, String)>,
    /// The rows of the list as drawn last, as places to put a carried row down on.
    spots: Vec<drag::Spot>,
    /// The shut folder that a carried row rests on, and since when.
    dwell: Option<(String, f64)>,

    form: forms::Form,
    /// A save (from the query row's button) that wants the form opened when there
    /// is a frame with what it needs.
    wants_save: bool,
    /// What the form is to offer to save, if a page of the Tools asked for it.
    prefer: Option<forms::What>,
    waiting: Option<Waiting>,
    notice: Option<Notice>,
    actions: Vec<Action>,
    /// The window had the focus last frame: when it gets it back the folder is read
    /// again, as another program may have changed it.
    was_focused: bool,
}

impl WorkspacePane {
    /// A pane for a workspace in `chosen`, or, if the user has not chosen a folder,
    /// in `usual`.
    pub fn new(usual: Option<PathBuf>, chosen: Option<PathBuf>) -> Self {
        Self {
            chosen,
            usual,
            workspace: None,
            filter: String::new(),
            group: Group::All,
            open_folders: HashSet::new(),
            opened_first_level: false,
            selected: None,
            renaming: None,
            opened: None,
            front: None,
            preview: None,
            spots: Vec::new(),
            dwell: None,
            form: forms::Form::None,
            wants_save: false,
            prefer: None,
            waiting: None,
            notice: None,
            actions: Vec::new(),
            was_focused: true,
        }
    }

    /// Keep the workspace in this folder from now on (none: in the usual place).
    pub fn use_folder(&mut self, chosen: Option<PathBuf>) {
        self.chosen = chosen;
        self.workspace = None;
        self.open_folders.clear();
        self.opened_first_level = false;
        self.selected = None;
        self.renaming = None;
        self.opened = None;
        self.preview = None;
        self.spots.clear();
        self.dwell = None;
        self.form = forms::Form::None;
    }

    /// The folder the workspace is in.
    pub fn root(&self) -> Option<&Path> {
        self.chosen.as_deref().or(self.usual.as_deref())
    }

    /// The workspace, read if it was not yet.
    fn workspace(&mut self) -> Option<&mut Workspace> {
        if self.workspace.is_none() {
            self.workspace = self.root().map(|root| Workspace::open(root.to_path_buf()));
            self.opened_first_level = false;
        }
        self.workspace.as_mut()
    }

    /// What the pane has asked the app to do since it was last asked.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// The query box was filled from the item `id`, with `text`.
    pub fn note_opened(&mut self, id: String, text: String) {
        self.opened = Some(Opened { id, text });
    }

    /// The query box was filled with `text` from the file `file`, which was dropped on the
    /// window: it is an item's if it is in the workspace, and nobody's if it is not. The
    /// toolkit hands the path of a dropped file over with its links resolved, which the
    /// folder's own path may not have: the file is looked for under both.
    pub fn note_dropped(&mut self, file: &Path, text: String) {
        let id = self.workspace.as_ref().and_then(|ws| {
            let root = self.root()?;
            let relative = file.strip_prefix(root).ok().or_else(|| {
                let resolved = root.canonicalize().ok()?;
                file.strip_prefix(resolved).ok()
            })?;
            let path = relative.to_str()?.replace('\\', "/");
            ws.item(&path).map(|item| item.id.clone())
        });
        self.opened = id.map(|id| Opened { id, text });
    }

    /// The button of the query row was pressed: the form to save what is in the box
    /// opens as soon as the pane is drawn with what it needs.
    pub fn ask_to_save(&mut self) {
        self.wants_save = true;
    }

    /// A message under the list.
    fn say(&mut self, ctx: &egui::Context, text: impl Into<String>, error: bool) {
        let now = ctx.input(|i| i.time);
        self.notice = Some(Notice {
            text: text.into(),
            error,
            until: now + NOTICE_SECONDS,
        });
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(NOTICE_SECONDS + 0.1));
    }

    /// The workspace was written: the picked item, and what the query box came
    /// from, are looked for again by their ids, as a path may have changed.
    fn after_change(&mut self) {
        self.preview = None;
        let selected = self.selected.clone();
        if let (Some(ws), Some(path)) = (&self.workspace, selected) {
            if ws.item(&path).is_none() && ws.tree().find_folder(&path).is_none() {
                self.selected = None;
            }
        }
    }

    /// The path of the item the query box was filled from, if it still is in the
    /// workspace.
    fn opened_path(&self) -> Option<String> {
        let (ws, opened) = (self.workspace.as_ref()?, self.opened.as_ref()?);
        ws.item_by_id(&opened.id).map(|item| item.path.clone())
    }

    /// The query row's part: the button that saves what is in the box in the
    /// workspace, and where it came from, with a dot if it was changed since.
    /// `room` is how much of the row's width this may take.
    pub fn query_row(&mut self, ui: &mut egui::Ui, query_text: &str, room: f32) {
        if ui
            .button("Save…")
            .on_hover_text("Save this query in the workspace…")
            .clicked()
        {
            self.ask_to_save();
        }
        let Some(path) = self.opened_path() else {
            return;
        };
        let changed = self.opened.as_ref().is_some_and(|o| o.text != query_text);
        let label = if changed {
            format!("{} \u{25cf}", path.replace('/', " / "))
        } else {
            path.replace('/', " / ")
        };
        let hover = if changed {
            "From the workspace, and changed since: Save… puts it back"
        } else {
            "From the workspace"
        };
        ui.allocate_ui(
            egui::vec2(room.max(40.0), ui.spacing().interact_size.y),
            |ui| {
                ui.add(egui::Label::new(egui::RichText::new(label).weak()).truncate())
                    .on_hover_text(hover);
            },
        );
    }

    /// The pane, in the room that it has: the header, the filter, the list and the
    /// item that is picked.
    pub fn pane(&mut self, ui: &mut egui::Ui, env: &Env<'_>) {
        let ctx = ui.ctx().clone();
        self.follow_the_page(env.front);
        self.read_again_on_focus(&ctx);
        self.workspace();

        self.header(ui);
        self.where_it_is(ui);
        self.filter_and_chips(ui);
        let mut events = Vec::new();
        // The item that is picked sits at the bottom, as high as it needs to be, and
        // the list has the rest.
        egui::Panel::bottom("workspace_details")
            .resizable(false)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                self.details_ui(ui, env, &mut events);
                self.notice_ui(ui);
            });
        let list = ui.available_height().max(40.0);
        self.tree_ui(ui, list, env.query_area, &mut events);
        self.apply(&ctx, env, events);
    }

    /// The strip of a folded pane: the button that opens it.
    pub fn rail(&mut self, ui: &mut egui::Ui, room: bool) {
        ui.vertical_centered(|ui| {
            let button = ui.add_enabled_ui(room, |ui| {
                icons::button(ui, "unfold", |painter, c, color| {
                    icons::pointer(painter, c, true, color);
                })
            });
            let button = if room {
                button.inner.on_hover_text("Show the workspace")
            } else {
                button
                    .inner
                    .on_disabled_hover_text("Make the window wider to show the workspace")
            };
            if button.clicked() {
                self.actions.push(Action::SetView(WorkspaceView::Open));
            }
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(24.0, 18.0), egui::Sense::hover());
            icons::side_panel(ui.painter(), rect.center(), ui.visuals().weak_text_color());
            response.on_hover_text("Workspace — your saved queries, schemas and samples");
        });
    }

    /// A page of the Tools being in front, the chips that narrow the list are set to
    /// what the page uses; once, when the page changes.
    fn follow_the_page(&mut self, front: Front) {
        if self.front == Some(front) {
            return;
        }
        let first = self.front.is_none();
        self.front = Some(front);
        if first {
            return;
        }
        self.group = match front {
            Front::Tool(Tool::Validate) => Group::Schemas,
            Front::Tool(Tool::Patch) => Group::Patches,
            Front::Query | Front::Tool(_) => Group::All,
        };
    }

    /// The window got the focus back: read the folder again.
    fn read_again_on_focus(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if focused && !self.was_focused {
            if let Some(ws) = self.workspace.as_mut() {
                ws.rescan();
            }
            self.after_change();
        }
        self.was_focused = focused;
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            pane_header::title(ui, "Workspace");
            pane_header::pinned_right(ui, |ui| {
                if icons::button(ui, "fold", |painter, c, color| {
                    icons::pointer(painter, c, false, color);
                })
                .on_hover_text("Fold the workspace away")
                .clicked()
                {
                    self.actions.push(Action::SetView(WorkspaceView::Rail));
                }
                let more = icons::button(ui, "menu", icons::dots)
                    .on_hover_text("Import, export, and where the workspace is");
                egui::Popup::menu(&more).show(|ui| self.menu_items(ui));
                if icons::button(ui, "new folder", icons::new_folder)
                    .on_hover_text("New folder…")
                    .clicked()
                {
                    self.form = forms::Form::folder(self.selected_folder());
                }
                if ui
                    .button("\u{1f4be}")
                    .on_hover_text("Save the query in the box here…")
                    .clicked()
                {
                    self.ask_to_save();
                }
            });
        });
    }

    /// What the pane's menu has in it: import, export and where the workspace is.
    fn menu_items(&mut self, ui: &mut egui::Ui) {
        let folder = self.selected_folder();
        if ui.button("New folder…").clicked() {
            self.form = forms::Form::folder(folder.clone());
            ui.close();
        }
        ui.separator();
        if ui.button("Import a workspace…").clicked() {
            self.form = forms::Form::import(folder.clone());
            ui.close();
        }
        if ui.button("Export the workspace…").clicked() {
            let selected = self.selected_folder_if_any();
            self.form = forms::Form::export(selected, self.root().map(Path::to_path_buf));
            ui.close();
        }
        ui.separator();
        if ui.button("Open another folder as the workspace…").clicked() {
            let start = self.root().map(Path::to_path_buf);
            let mut dialog = rfd::FileDialog::new().set_title("Choose the workspace's folder");
            if let Some(start) = start.and_then(|p| p.parent().map(Path::to_path_buf)) {
                dialog = dialog.set_directory(start);
            }
            if let Some(path) = dialog.pick_folder() {
                self.actions.push(Action::SetFolder(
                    Some(path).filter(|p| Some(p.as_path()) != self.usual.as_deref()),
                ));
            }
            ui.close();
        }
        if self.chosen.is_some() && ui.button("Use the usual workspace").clicked() {
            self.actions.push(Action::SetFolder(None));
            ui.close();
        }
        if let Some(root) = self.root().map(Path::to_path_buf) {
            if ui.button("Show it in the file manager").clicked() {
                self.actions.push(Action::Reveal(root));
                ui.close();
            }
        }
        if ui.button("Read it again").clicked() {
            if let Some(ws) = self.workspace.as_mut() {
                ws.rescan();
            }
            self.after_change();
            ui.close();
        }
        ui.separator();
        if ui.button("Hide the workspace").clicked() {
            self.actions.push(Action::SetView(WorkspaceView::Hidden));
            ui.close();
        }
    }

    /// Where the workspace is, and how much is in it.
    fn where_it_is(&mut self, ui: &mut egui::Ui) {
        let root = self.root().map(Path::to_path_buf);
        let (items, notes) = match &self.workspace {
            Some(ws) => (ws.tree().item_count(), ws.notes().to_vec()),
            None => (0, Vec::new()),
        };
        ui.horizontal(|ui| {
            let line = match &root {
                Some(root) => format!(
                    "{}  ·  {}",
                    abbreviate(root),
                    counted(items, "item", "items")
                ),
                None => "There is nowhere to keep a workspace".to_owned(),
            };
            let response =
                ui.add(egui::Label::new(egui::RichText::new(line).small().weak()).truncate());
            if let Some(root) = &root {
                response.on_hover_text(root.display().to_string());
            }
            if !notes.is_empty() {
                ui.label(
                    egui::RichText::new("\u{26a0}")
                        .small()
                        .color(ui.visuals().warn_fg_color),
                )
                .on_hover_text(notes.join("\n"));
            }
        });
    }

    fn filter_and_chips(&mut self, ui: &mut egui::Ui) {
        let filter = ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter by name…")
                .desired_width(f32::INFINITY),
        );
        text_menu::attach(&filter, &self.filter, text_menu::Kind::Typed);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for group in Group::ALL {
                let label = egui::RichText::new(group.label()).small();
                if ui.selectable_label(self.group == group, label).clicked() {
                    self.group = group;
                }
            }
        });
    }

    /// The folder that new things go in: the one that is picked, or the one the item
    /// that is picked is in, or the one the query box's item is in.
    pub(super) fn selected_folder(&self) -> String {
        self.selected_folder_if_any()
            .or_else(|| {
                let ws = self.workspace.as_ref()?;
                let opened = ws.item_by_id(&self.opened.as_ref()?.id)?;
                Some(opened.folder().to_owned())
            })
            .unwrap_or_default()
    }

    fn selected_folder_if_any(&self) -> Option<String> {
        let (ws, selected) = (self.workspace.as_ref()?, self.selected.as_ref()?);
        if ws.tree().find_folder(selected).is_some() {
            Some(selected.clone())
        } else {
            ws.item(selected).map(|item| item.folder().to_owned())
        }
    }

    /// A message that has been there long enough is taken away; one that is there is
    /// shown under the list.
    fn notice_ui(&mut self, ui: &mut egui::Ui) {
        let now = ui.ctx().input(|i| i.time);
        if self.notice.as_ref().is_some_and(|n| n.until <= now) {
            self.notice = None;
        }
        if let Some(notice) = &self.notice {
            // The window is drawn again when the message is due to go, even if nobody
            // touches it: what was asked for in an earlier frame is forgotten by the next
            // one, so every frame that shows the message asks again.
            ui.ctx()
                .request_repaint_after_secs((notice.until - now).max(0.0) as f32 + 0.05);
            let color = if notice.error {
                crate::tools::ERROR_COLOR
            } else {
                ui.visuals().weak_text_color()
            };
            ui.add(egui::Label::new(egui::RichText::new(&notice.text).small().color(color)).wrap());
        }
    }

    /// The query that the user just saved, or opened, is the one the box came from.
    pub fn sample_written(&mut self, ctx: &egui::Context, path: &Path) -> bool {
        let Some(waiting) = self.waiting.take_if(|w| w.staged == path) else {
            return false;
        };
        self.finish_save(ctx, waiting);
        true
    }

    /// A save the worker was to write the sample for failed.
    pub fn sample_failed(&mut self, ctx: &egui::Context, error: &str) -> bool {
        let Some(waiting) = self.waiting.take() else {
            return false;
        };
        let _ = std::fs::remove_file(&waiting.staged);
        self.say(
            ctx,
            format!("“{}” was not saved: {error}", waiting.save.name),
            true,
        );
        true
    }
}

/// A count with its noun: `1 item`, `2 items`.
pub(super) fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `path` as it is shown: the user's home is a `~`.
pub(super) fn abbreviate(path: &Path) -> String {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if let Some(rest) = home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        let mut shown = PathBuf::from("~");
        shown.push(rest);
        return shown.display().to_string();
    }
    path.display().to_string()
}

/// The small button in the status bar that shows the workspace pane or hides it: the
/// picture of a window with its left column filled, dimmed while the pane is not shown.
/// Gives whether it was clicked.
pub fn status_button(ui: &mut egui::Ui, shown: bool, tip: &str) -> bool {
    let weak = ui.visuals().weak_text_color();
    let response = icons::button(ui, "status", |painter, c, color| {
        icons::side_panel(painter, c, if shown { color } else { weak });
    });
    response.on_hover_text(tip).clicked()
}

/// What the tests look at, and the one thing they make the pane do.
#[cfg(test)]
impl WorkspacePane {
    pub fn read_workspace(&self) -> Option<&Workspace> {
        self.workspace.as_ref()
    }

    pub fn selected_path(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn notice_text(&self) -> Option<&str> {
        self.notice.as_ref().map(|n| n.text.as_str())
    }

    pub fn group(&self) -> Group {
        self.group
    }

    pub fn push_action(&mut self, action: Action) {
        self.actions.push(action);
    }
}
