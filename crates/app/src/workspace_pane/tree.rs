//! The list of the workspace's folders and items, the item that is picked under it,
//! and what is done when something in the pane is clicked.

use std::collections::HashSet;

use eframe::egui::{self, Color32, FontId, Rect, TextStyle};

use super::drag::{self, Carried, Spot, Verdict};
use super::{forms, icons, Action, Env, Front, WorkspacePane, NO_ADDRESS, QUERY_LIMIT};
use crate::tools::Tool;
use crate::workspace::{address_in, Folder, Group, Item, ItemKind};

const ROW_HEIGHT: f32 = 20.0;
const INDENT: f32 = 14.0;

/// What the list is asked for by a click, done after the list is drawn.
pub(super) enum Event {
    Toggle(String),
    /// Open a folder (it is held open, not turned over: a row carried over it asks).
    Expand(String),
    Select(String),
    Open {
        path: String,
        with_sample: bool,
    },
    StartRename(String),
    FinishRename {
        path: String,
        name: String,
    },
    CancelRename,
    Duplicate(String),
    MoveTo {
        path: String,
        folder: String,
    },
    Delete(String),
    CopyText(String),
    Reveal(String),
    SaveInto(String),
    SaveUrlInto(String),
    NewFolderIn(String),
    ExportFolder(String),
    ImportInto(String),
}

/// One row of the list.
struct Row {
    path: String,
    name: String,
    depth: usize,
    what: What,
}

enum What {
    Folder { open: bool, count: usize },
    Item { kind: ItemKind, has_sample: bool },
}

/// What a walk through the tree needs to know of how the list is narrowed.
struct Narrowing<'a> {
    group: Group,
    needle: String,
    open: &'a HashSet<String>,
}

impl Narrowing<'_> {
    fn is_on(&self) -> bool {
        self.group != Group::All || !self.needle.is_empty()
    }

    fn takes(&self, item: &Item) -> bool {
        self.group.takes(item.kind)
            && (self.needle.is_empty()
                || item.name.to_lowercase().contains(&self.needle)
                || item.folder().to_lowercase().contains(&self.needle))
    }
}

/// The rows of `folder`'s contents: its folders (each with what is in it, if it is
/// open) and then its items. Gives how many items there are that the list shows.
fn rows_of(folder: &Folder, depth: usize, narrowing: &Narrowing<'_>, out: &mut Vec<Row>) -> usize {
    let mut shown = 0;
    for child in &folder.folders {
        let mut inside = Vec::new();
        let count = rows_of(child, depth + 1, narrowing, &mut inside);
        if narrowing.is_on() && count == 0 {
            continue;
        }
        let open = narrowing.is_on() || narrowing.open.contains(&child.path);
        let count = if narrowing.is_on() {
            count
        } else {
            child.item_count()
        };
        out.push(Row {
            path: child.path.clone(),
            name: child.name.clone(),
            depth,
            what: What::Folder { open, count },
        });
        if open {
            out.extend(inside);
        }
        shown += count;
    }
    for item in &folder.items {
        if narrowing.takes(item) {
            out.push(Row {
                path: item.path.clone(),
                name: item.name.clone(),
                depth,
                what: What::Item {
                    kind: item.kind,
                    has_sample: item.sample.is_some(),
                },
            });
            shown += 1;
        }
    }
    shown
}

/// The colours of the little tag that says what an item is.
fn badge_colors(kind: ItemKind, dark: bool) -> (Color32, Color32) {
    use jsonquery_query::Kind::{JmesPath, Jq, JsonPath, JsonPointer};
    let (light, deep) = match kind {
        ItemKind::Query(Jq) => ((219, 232, 251), (31, 53, 85)),
        ItemKind::Query(JsonPath) => ((224, 241, 214), (37, 64, 28)),
        ItemKind::Query(JmesPath) => ((251, 231, 204), (74, 48, 18)),
        ItemKind::Query(JsonPointer) => ((234, 223, 247), (58, 39, 80)),
        ItemKind::Schema => ((211, 238, 238), (23, 72, 75)),
        ItemKind::Patch => ((250, 219, 228), (82, 32, 47)),
        ItemKind::Document => ((228, 228, 228), (56, 56, 56)),
        ItemKind::Url => ((246, 240, 160), (84, 76, 0)),
    };
    let text = match kind {
        ItemKind::Query(Jq) => ((26, 63, 115), (180, 207, 247)),
        ItemKind::Query(JsonPath) => ((36, 90, 20), (181, 221, 156)),
        ItemKind::Query(JmesPath) => ((122, 66, 0), (242, 196, 138)),
        ItemKind::Query(JsonPointer) => ((91, 47, 138), (211, 185, 240)),
        ItemKind::Schema => ((15, 79, 82), (166, 223, 226)),
        ItemKind::Patch => ((124, 30, 59), (243, 176, 196)),
        ItemKind::Document => ((68, 68, 68), (192, 192, 192)),
        ItemKind::Url => ((98, 76, 0), (244, 226, 134)),
    };
    let rgb = |(r, g, b): (u8, u8, u8)| Color32::from_rgb(r, g, b);
    if dark {
        (rgb(deep), rgb(text.1))
    } else {
        (rgb(light), rgb(text.0))
    }
}

impl WorkspacePane {
    fn rows(&self) -> Vec<Row> {
        let Some(workspace) = &self.workspace else {
            return Vec::new();
        };
        let narrowing = Narrowing {
            group: self.group,
            needle: self.filter.trim().to_lowercase(),
            open: &self.open_folders,
        };
        let mut rows = Vec::new();
        rows_of(workspace.tree(), 0, &narrowing, &mut rows);
        rows
    }

    /// What the row at `path` is called: an item's name, a folder's last part.
    fn name_of(&self, path: &str) -> String {
        self.workspace
            .as_ref()
            .and_then(|ws| ws.item(path).map(|item| item.name.clone()))
            .unwrap_or_else(|| drag::name_of_folder(path))
    }

    /// A folder was renamed or moved from `old` to `new`: the folders in it (and it) that
    /// were open are open at their new places.
    fn follow_open_folders(&mut self, old: &str, new: &str) {
        let prefix = format!("{old}/");
        let moved: Vec<(String, String)> = self
            .open_folders
            .iter()
            .filter_map(|path| {
                if path == old {
                    Some((path.clone(), new.to_owned()))
                } else {
                    path.strip_prefix(&prefix)
                        .map(|rest| (path.clone(), format!("{new}/{rest}")))
                }
            })
            .collect();
        for (from, to) in moved {
            self.open_folders.remove(&from);
            self.open_folders.insert(to);
        }
    }

    /// The folders of the workspace, for a menu that moves something.
    fn folder_paths(&self) -> Vec<String> {
        self.workspace
            .as_ref()
            .map(|ws| ws.tree().folder_paths())
            .unwrap_or_default()
    }

    /// The list, in `height`, scrolling. What is clicked is put in `events`.
    pub(super) fn tree_ui(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        query_area: Option<Rect>,
        events: &mut Vec<Event>,
    ) {
        // The folders at the top start open, once for each workspace that is read.
        if !self.opened_first_level {
            if let Some(ws) = &self.workspace {
                self.open_folders
                    .extend(ws.tree().folders.iter().map(|f| f.path.clone()));
                self.opened_first_level = true;
            }
        }
        let rows = self.rows();
        let missing = self.chosen.is_some() && self.workspace.as_ref().is_some_and(|w| !w.exists());
        if rows.is_empty() {
            ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
                self.empty_ui(ui, missing);
            });
            return;
        }
        let folders = self.folder_paths();
        let mut renaming = self.renaming.take();
        self.spots.clear();
        let carried = drag::carried(ui.ctx());
        let pointer = ui.ctx().pointer_interact_pos();
        let list = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .id_salt("workspace_tree")
                    .max_height(height)
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                        // A row carried to the foot of the list, or to its head, brings
                        // more of the list into view.
                        if let (Some(_), Some(at)) = (&carried, pointer) {
                            let clip = ui.clip_rect();
                            let dt = ui.ctx().input(|i| i.stable_dt);
                            let by = drag::edge_scroll(at.y, clip.top(), clip.bottom(), dt);
                            if by != 0.0 {
                                // At once: the speed is the pointer's to set, not the animation's.
                                ui.scroll_with_delta_animation(
                                    egui::vec2(0.0, by),
                                    egui::style::ScrollAnimation::none(),
                                );
                                ui.ctx().request_repaint();
                            }
                        }
                        for row in &rows[range] {
                            self.row_ui(ui, row, &folders, &mut renaming, events);
                        }
                    })
                    .inner_rect
            })
            .inner;
        self.renaming = renaming;
        self.carry(ui, list, query_area, carried, events);
    }

    /// Whether `carried` is a query, which may be put down on the query pane (a folder is
    /// not an item, so it is not).
    fn carries_a_query(&self, carried: &Carried) -> bool {
        self.workspace
            .as_ref()
            .and_then(|ws| ws.item(&carried.path))
            .is_some_and(|item| matches!(item.kind, ItemKind::Query(_)))
    }

    /// What is done with a row that is being carried: where it would go is lit and
    /// said beside the pointer, a shut folder it rests on is opened, and let go, it is
    /// put there. A query that is carried over the query pane (`query_area`) is opened
    /// there instead.
    fn carry(
        &mut self,
        ui: &egui::Ui,
        list: Rect,
        query_area: Option<Rect>,
        carried: Option<std::sync::Arc<Carried>>,
        events: &mut Vec<Event>,
    ) {
        let ctx = ui.ctx();
        let Some(carried) = carried else {
            self.dwell = None;
            return;
        };
        let (down, released, now) =
            ctx.input(|i| (i.pointer.any_down(), i.pointer.any_released(), i.time));
        if !down && !released {
            // The button went up where the window was not told: nothing is carried.
            drag::put_down(ctx);
            self.dwell = None;
            return;
        }
        let pointer = ctx.pointer_interact_pos();
        // Over the query pane, a query that is carried would be opened.
        let onto_query = pointer
            .zip(query_area)
            .filter(|(at, area)| area.contains(*at))
            .filter(|_| self.carries_a_query(&carried))
            .map(|(_, area)| area);
        // Where the pointer is: outside the list, or in it — on a row, or not.
        let over: Option<Option<Spot>> = pointer
            .filter(|at| list.contains(*at))
            .map(|at| self.spots.iter().find(|s| s.rect.contains(at)).cloned());
        let folder: Option<String> = over
            .as_ref()
            .map(|spot| spot.as_ref().map(|s| s.folder.clone()).unwrap_or_default());
        let verdict = folder.as_deref().map(|f| drag::verdict(&carried, f));
        if released {
            if onto_query.is_some() {
                events.push(Event::Select(carried.path.clone()));
                events.push(Event::Open {
                    path: carried.path.clone(),
                    with_sample: false,
                });
            } else if let (Some(folder), Some(Verdict::Moves)) = (&folder, verdict) {
                events.push(Event::MoveTo {
                    path: carried.path.clone(),
                    folder: folder.clone(),
                });
            }
            drag::put_down(ctx);
            self.dwell = None;
            return;
        }
        if let (Some(folder), Some(Verdict::Moves)) = (&folder, verdict) {
            let named = self
                .spots
                .iter()
                .find(|s| s.is_folder && s.folder == *folder)
                .map(|s| s.rect);
            match (named, over.as_ref().and_then(|s| s.as_ref())) {
                (Some(row), _) => drag::light_row(ui, list, row),
                (None, Some(hit)) if !folder.is_empty() => drag::light_row(ui, list, hit.rect),
                _ => drag::light_list(ui, list),
            }
        }
        let shut = over
            .as_ref()
            .and_then(|s| s.as_ref())
            .filter(|s| s.shut)
            .map(|s| s.folder.as_str());
        if let Some(path) = drag::dwell(&mut self.dwell, shut, now) {
            events.push(Event::Expand(path));
        } else if let Some((_, since)) = &self.dwell {
            ctx.request_repaint_after_secs(
                (drag::OPEN_AFTER - (now - since)).max(0.0) as f32 + 0.02,
            );
        }
        if let Some(area) = onto_query {
            drag::light_query(ui, area);
        }
        if let Some(at) = pointer {
            let text = if onto_query.is_some() {
                drag::open_text(&carried)
            } else {
                drag::ghost_text(&carried, folder.as_deref().zip(verdict))
            };
            drag::show_ghost(ctx, at, &text);
        }
    }

    /// What is shown in place of the list while there is nothing to list.
    fn empty_ui(&mut self, ui: &mut egui::Ui, missing: bool) {
        ui.add_space(8.0);
        if missing {
            let root = self
                .root()
                .map(|r| r.display().to_string())
                .unwrap_or_default();
            ui.label(
                egui::RichText::new(format!("The workspace's folder is not there:\n{root}"))
                    .small(),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("Choose another…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.actions.push(Action::SetFolder(Some(path)));
                    }
                }
                if ui.button("Use the usual one").clicked() {
                    self.actions.push(Action::SetFolder(None));
                }
            });
            return;
        }
        let narrowed = self.group != Group::All || !self.filter.trim().is_empty();
        let text = if self.root().is_none() {
            "There is nowhere to keep a workspace."
        } else if narrowed {
            "Nothing here matches."
        } else {
            "Nothing is saved yet.\nSave the query in the box with \u{1f4be} above, or import a workspace from the menu."
        };
        ui.label(egui::RichText::new(text).small().weak());
    }

    /// One row: what it is, and what a click on it does.
    fn row_ui(
        &mut self,
        ui: &mut egui::Ui,
        row: &Row,
        folders: &[String],
        renaming: &mut Option<(String, String)>,
        events: &mut Vec<Event>,
    ) {
        // A row is dragged to move it — not the one that is being renamed.
        let renamed = renaming.as_ref().is_some_and(|(path, _)| path == &row.path);
        let sense = if renamed {
            egui::Sense::click()
        } else {
            egui::Sense::click_and_drag()
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), sense);
        let selected = self.selected.as_deref() == Some(row.path.as_str());
        let opened = self.opened_path().as_deref() == Some(row.path.as_str());
        if ui.is_rect_visible(rect) {
            self.paint_row(ui, rect, &response, row, selected, opened, renaming, events);
            self.spots.push(Spot {
                rect,
                folder: match row.what {
                    What::Folder { .. } => row.path.clone(),
                    What::Item { .. } => crate::workspace::parent_of(&row.path).to_owned(),
                },
                is_folder: matches!(row.what, What::Folder { .. }),
                shut: matches!(row.what, What::Folder { open: false, .. }),
            });
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            response.dnd_set_drag_payload(Carried {
                path: row.path.clone(),
                name: row.name.clone(),
                is_folder: matches!(row.what, What::Folder { .. }),
            });
        }
        if response.clicked() {
            if matches!(row.what, What::Folder { .. }) {
                events.push(Event::Toggle(row.path.clone()));
            }
            events.push(Event::Select(row.path.clone()));
        }
        if response.double_clicked() && matches!(row.what, What::Item { .. }) {
            events.push(Event::Open {
                path: row.path.clone(),
                with_sample: false,
            });
        }
        response.context_menu(|ui| {
            events.push(Event::Select(row.path.clone()));
            row_menu(ui, row, folders, events);
        });
    }

    /// The look of a row, and its name's box while it is being renamed.
    #[allow(clippy::too_many_arguments)]
    fn paint_row(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        response: &egui::Response,
        row: &Row,
        selected: bool,
        opened: bool,
        renaming: &mut Option<(String, String)>,
        events: &mut Vec<Event>,
    ) {
        let visuals = ui.visuals().clone();
        let painter = ui.painter_at(rect);
        if selected {
            painter.rect_filled(rect, 2.0, visuals.selection.bg_fill.linear_multiply(0.5));
        } else if response.hovered() {
            painter.rect_filled(rect, 2.0, visuals.widgets.hovered.weak_bg_fill);
        }
        let body: FontId = TextStyle::Body.resolve(ui.style());
        let small: FontId = TextStyle::Small.resolve(ui.style());
        let centre = rect.center().y;
        let weak = visuals.weak_text_color();
        let lifted = drag::carried(ui.ctx()).is_some_and(|c| c.path == row.path);
        let text_color = if lifted {
            weak
        } else if opened {
            visuals.strong_text_color()
        } else {
            visuals.text_color()
        };
        let mut x = rect.left() + 4.0 + row.depth as f32 * INDENT;
        let mut right = rect.right() - 4.0;

        match &row.what {
            What::Folder { open, count } => {
                icons::arrow(&painter, egui::pos2(x + 4.0, centre), *open, weak);
                x += 14.0;
                icons::folder(&painter, egui::pos2(x + 7.0, centre), weak);
                x += 20.0;
                let galley = painter.layout_no_wrap(count.to_string(), small.clone(), weak);
                right -= galley.size().x;
                painter.galley(
                    egui::pos2(right, centre - galley.size().y / 2.0),
                    galley,
                    weak,
                );
            }
            What::Item { kind, has_sample } => {
                // The width of the arrow and the folder, so that names line up.
                x += 14.0 + 20.0;
                if *has_sample {
                    let galley =
                        painter.layout_no_wrap("\u{1f4ce}".to_owned(), small.clone(), weak);
                    right -= galley.size().x;
                    painter.galley(
                        egui::pos2(right, centre - galley.size().y / 2.0),
                        galley,
                        weak,
                    );
                    right -= 4.0;
                }
                let (background, foreground) = badge_colors(*kind, visuals.dark_mode);
                let tag =
                    painter.layout_no_wrap(kind.badge().to_owned(), small.clone(), foreground);
                let tag_rect = Rect::from_center_size(
                    egui::pos2(right - tag.size().x / 2.0 - 4.0, centre),
                    egui::vec2(tag.size().x + 8.0, tag.size().y + 2.0),
                );
                painter.rect_filled(tag_rect, tag_rect.height() / 2.0, background);
                painter.galley(tag_rect.center() - tag.size() / 2.0, tag, foreground);
                right = tag_rect.left() - 4.0;
            }
        }

        let room = (right - x - 4.0).max(10.0);
        if renaming.as_ref().is_some_and(|(path, _)| path == &row.path) {
            if let Some((path, text)) = renaming.as_mut() {
                let edit_rect = Rect::from_min_max(
                    egui::pos2(x, rect.top() + 1.0),
                    egui::pos2(x + room, rect.bottom() - 1.0),
                );
                let edit = ui.put(
                    edit_rect,
                    egui::TextEdit::singleline(text).desired_width(room),
                );
                if !edit.has_focus() && !edit.lost_focus() {
                    edit.request_focus();
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if enter {
                    events.push(Event::FinishRename {
                        path: path.clone(),
                        name: text.clone(),
                    });
                } else if escape || edit.lost_focus() {
                    events.push(Event::CancelRename);
                }
            }
            return;
        }
        let mut job = egui::text::LayoutJob::simple_singleline(row.name.clone(), body, text_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        painter.galley(
            egui::pos2(x, centre - galley.size().y / 2.0),
            galley,
            text_color,
        );
    }

    /// The item or folder that is picked, under the list: what it holds, its
    /// sample, and the buttons that put it where it belongs.
    pub(super) fn details_ui(&mut self, ui: &mut egui::Ui, env: &Env<'_>, events: &mut Vec<Event>) {
        let Some(path) = self.selected.clone() else {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Pick an item to see it here.")
                    .small()
                    .weak(),
            );
            return;
        };
        let Some(workspace) = &self.workspace else {
            return;
        };
        if let Some(item) = workspace.item(&path).cloned() {
            let preview = self.preview_of(&item);
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(&item.name).strong()).truncate());
            });
            ui.add(
                egui::Label::new(egui::RichText::new(&item.path).small().monospace().weak())
                    .truncate(),
            );
            ui.add(
                egui::Label::new(egui::RichText::new(preview).small().monospace())
                    .truncate()
                    .selectable(false),
            );
            let sample = match &item.sample {
                Some(sample) => format!(
                    "Sample: {} · {}",
                    crate::workspace::sample_name(&item),
                    crate::app::human_bytes(sample.bytes)
                ),
                None if item.kind == ItemKind::Document => {
                    format!("Document · {}", crate::app::human_bytes(item.bytes))
                }
                None if item.kind == ItemKind::Url => "Web address".to_owned(),
                None => "No sample".to_owned(),
            };
            ui.label(egui::RichText::new(sample).small().weak());
            ui.horizontal_wrapped(|ui| {
                let (open, open_with) = labels(item.kind, env.front);
                if ui.button(open).clicked() {
                    events.push(Event::Open {
                        path: path.clone(),
                        with_sample: false,
                    });
                }
                if item.sample.is_some()
                    && ui
                        .button(open_with)
                        .on_hover_text("Also open its sample as the Source, and run it")
                        .clicked()
                {
                    events.push(Event::Open {
                        path: path.clone(),
                        with_sample: true,
                    });
                }
            });
        } else if let Some(folder) = workspace.tree().find_folder(&path) {
            ui.label(egui::RichText::new(&folder.name).strong());
            ui.label(
                egui::RichText::new(format!(
                    "{}, {}",
                    super::counted(folder.item_count(), "item", "items"),
                    super::counted(folder.folder_count(), "folder", "folders")
                ))
                .small()
                .weak(),
            );
        }
    }

    /// The first lines of what `item` holds (a web address: the address), read once for
    /// each version of it.
    fn preview_of(&mut self, item: &Item) -> String {
        if let Some((sha, text)) = &self.preview {
            if *sha == item.sha256 {
                return text.clone();
            }
        }
        let text = self
            .workspace
            .as_ref()
            .and_then(|ws| ws.read_text(&item.path, 64 * 1024).ok())
            .unwrap_or_default();
        let address = address_in(&text).filter(|_| item.kind == ItemKind::Url);
        let shown = address.map_or_else(|| first_lines(&text), str::to_owned);
        let shown: String = shown.chars().take(240).collect();
        self.preview = Some((item.sha256.clone(), shown.clone()));
        shown
    }

    /// Do what a click in the pane asked for.
    pub(super) fn apply(&mut self, ctx: &egui::Context, env: &Env<'_>, events: Vec<Event>) {
        for event in events {
            match event {
                Event::Toggle(path) => {
                    if !self.open_folders.remove(&path) {
                        self.open_folders.insert(path);
                    }
                }
                Event::Expand(path) => {
                    self.open_folders.insert(path);
                }
                Event::Select(path) => self.selected = Some(path),
                Event::Open { path, with_sample } => self.open_item(ctx, &path, with_sample),
                Event::StartRename(path) => {
                    let name = self
                        .workspace
                        .as_ref()
                        .and_then(|ws| {
                            ws.item(&path)
                                .map(|i| i.name.clone())
                                .or_else(|| ws.tree().find_folder(&path).map(|f| f.name.clone()))
                        })
                        .unwrap_or_default();
                    self.renaming = Some((path, name));
                }
                Event::CancelRename => self.renaming = None,
                Event::FinishRename { path, name } => {
                    self.renaming = None;
                    let result = self.workspace.as_mut().map(|ws| ws.rename(&path, &name));
                    match result {
                        Some(Ok(new_path)) => {
                            self.follow_open_folders(&path, &new_path);
                            self.selected = Some(new_path);
                            self.after_change();
                        }
                        Some(Err(e)) => self.say(ctx, e, true),
                        None => {}
                    }
                }
                Event::Duplicate(path) => {
                    let result = self.workspace.as_mut().map(|ws| ws.duplicate(&path));
                    match result {
                        Some(Ok(copy)) => {
                            self.selected = Some(copy.path.clone());
                            self.say(ctx, format!("Copied as “{}”", copy.name), false);
                            self.after_change();
                        }
                        Some(Err(e)) => self.say(ctx, e, true),
                        None => {}
                    }
                }
                Event::MoveTo { path, folder } => {
                    let name = self.name_of(&path);
                    let result = self.workspace.as_mut().map(|ws| ws.move_to(&path, &folder));
                    match result {
                        Some(Ok(new_path)) => {
                            self.follow_open_folders(&path, &new_path);
                            self.selected = Some(new_path);
                            if !folder.is_empty() {
                                self.open_folders.insert(folder.clone());
                            }
                            self.after_change();
                            self.say(ctx, drag::moved_text(&name, &folder), false);
                        }
                        Some(Err(e)) => self.say(ctx, e, true),
                        None => {}
                    }
                }
                Event::Delete(path) => {
                    self.form = forms::Form::delete(self.workspace.as_ref(), &path);
                }
                Event::CopyText(path) => self.copy_text_of(ctx, &path),
                Event::Reveal(path) => {
                    if let Some(ws) = &self.workspace {
                        self.actions.push(Action::Reveal(ws.abs(&path)));
                    }
                }
                Event::SaveInto(folder) => {
                    self.selected = Some(folder);
                    self.wants_save = true;
                }
                Event::SaveUrlInto(folder) => {
                    self.selected = Some(folder);
                    self.prefer = Some(forms::What::Url);
                    self.wants_save = true;
                }
                Event::NewFolderIn(folder) => self.form = forms::Form::folder(folder),
                Event::ExportFolder(folder) => {
                    self.form =
                        forms::Form::export(Some(folder), self.root().map(|r| r.to_path_buf()));
                }
                Event::ImportInto(folder) => self.form = forms::Form::import(folder),
            }
        }
        let _ = env;
    }

    /// Copy the text of the item at `path` to the clipboard: what its file holds, or, for
    /// a web address, the address.
    fn copy_text_of(&mut self, ctx: &egui::Context, path: &str) {
        let Some(ws) = &self.workspace else { return };
        let is_address = ws.item(path).is_some_and(|item| item.kind == ItemKind::Url);
        match ws.read_text(path, QUERY_LIMIT * 16) {
            Ok(text) if is_address => match address_in(&text) {
                Some(address) => ctx.copy_text(address.to_owned()),
                None => self.say(ctx, NO_ADDRESS, true),
            },
            Ok(text) => ctx.copy_text(text),
            Err(e) => self.say(ctx, e, true),
        }
    }

    /// Put the item at `path` where it belongs; with its sample, as the Source (or
    /// as the document of the page), and run it. A web address is loaded from the web
    /// as the Source.
    fn open_item(&mut self, ctx: &egui::Context, path: &str, with_sample: bool) {
        let Some(ws) = &self.workspace else { return };
        let Some(item) = ws.item(path).cloned() else {
            return;
        };
        let sample = if with_sample {
            item.sample.as_ref().map(|s| ws.abs(&s.path))
        } else {
            None
        };
        let file = ws.abs(&item.path);
        let action = match item.kind {
            ItemKind::Query(engine) => match ws.read_text(&item.path, QUERY_LIMIT) {
                Ok(text) => Action::OpenQuery {
                    id: item.id.clone(),
                    engine,
                    text,
                    sample,
                },
                Err(e) => {
                    self.say(ctx, e, true);
                    return;
                }
            },
            ItemKind::Schema => Action::UseFiles {
                tool: Tool::Validate,
                path: file,
                sample,
            },
            ItemKind::Patch => Action::UseFiles {
                tool: Tool::Patch,
                path: file,
                sample,
            },
            ItemKind::Document => Action::OpenDocument(file),
            ItemKind::Url => match ws.read_text(&item.path, QUERY_LIMIT) {
                Ok(text) => match address_in(&text) {
                    Some(address) => Action::OpenUrl(address.to_owned()),
                    None => {
                        self.say(ctx, NO_ADDRESS, true);
                        return;
                    }
                },
                Err(e) => {
                    self.say(ctx, e, true);
                    return;
                }
            },
        };
        self.actions.push(action);
    }
}

/// The first two lines of `text` that have anything in them, trimmed, on one line.
fn first_lines(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(2)
        .collect::<Vec<_>>()
        .join("  ")
}

/// What the two buttons under an item say, by what it is.
fn labels(kind: ItemKind, _front: Front) -> (&'static str, &'static str) {
    match kind {
        ItemKind::Query(_) => ("Open", "Open with sample"),
        ItemKind::Schema => ("Use as schema", "With its sample"),
        ItemKind::Patch => ("Use as patch", "With its sample"),
        ItemKind::Document => ("Open as Source", "Open as Source"),
        ItemKind::Url => ("Load from the web", "Load from the web"),
    }
}

/// The menu of a right click on a row.
fn row_menu(ui: &mut egui::Ui, row: &Row, folders: &[String], events: &mut Vec<Event>) {
    let path = row.path.clone();
    match &row.what {
        What::Item { kind, has_sample } => {
            let (open, open_with) = labels(*kind, Front::Query);
            if ui.button(open).clicked() {
                events.push(Event::Open {
                    path: path.clone(),
                    with_sample: false,
                });
                ui.close();
            }
            if *has_sample && ui.button(open_with).clicked() {
                events.push(Event::Open {
                    path: path.clone(),
                    with_sample: true,
                });
                ui.close();
            }
            ui.separator();
            if ui.button("Rename").clicked() {
                events.push(Event::StartRename(path.clone()));
                ui.close();
            }
            if ui.button("Duplicate").clicked() {
                events.push(Event::Duplicate(path.clone()));
                ui.close();
            }
            move_menu(ui, &path, "", folders, events);
            if ui.button("Copy text").clicked() {
                events.push(Event::CopyText(path.clone()));
                ui.close();
            }
            if ui.button("Show in the file manager").clicked() {
                events.push(Event::Reveal(path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button("Delete…").clicked() {
                events.push(Event::Delete(path));
                ui.close();
            }
        }
        What::Folder { .. } => {
            if ui.button("Save the query here…").clicked() {
                events.push(Event::SaveInto(path.clone()));
                ui.close();
            }
            if ui.button("Save a web address here…").clicked() {
                events.push(Event::SaveUrlInto(path.clone()));
                ui.close();
            }
            if ui.button("New folder…").clicked() {
                events.push(Event::NewFolderIn(path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button("Rename").clicked() {
                events.push(Event::StartRename(path.clone()));
                ui.close();
            }
            move_menu(ui, &path, &path, folders, events);
            ui.separator();
            if ui.button("Import into this folder…").clicked() {
                events.push(Event::ImportInto(path.clone()));
                ui.close();
            }
            if ui.button("Export this folder…").clicked() {
                events.push(Event::ExportFolder(path.clone()));
                ui.close();
            }
            if ui.button("Show in the file manager").clicked() {
                events.push(Event::Reveal(path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button("Delete…").clicked() {
                events.push(Event::Delete(path));
                ui.close();
            }
        }
    }
}

/// The submenu that moves `path` to another folder. `own_folder` is the folder
/// that `path` is (empty for an item), whose own folders it cannot be moved to.
fn move_menu(
    ui: &mut egui::Ui,
    path: &str,
    own_folder: &str,
    folders: &[String],
    events: &mut Vec<Event>,
) {
    ui.menu_button("Move to", |ui| {
        let here = crate::workspace::parent_of(path);
        let mut offer = |ui: &mut egui::Ui, target: &str, label: &str| {
            let inside_itself = !own_folder.is_empty()
                && (target == own_folder || target.starts_with(&format!("{own_folder}/")));
            let enabled = target != here && !inside_itself;
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                events.push(Event::MoveTo {
                    path: path.to_owned(),
                    folder: target.to_owned(),
                });
                ui.close();
            }
        };
        offer(ui, "", "(top)");
        for folder in folders {
            let depth = folder.matches('/').count();
            let label = format!(
                "{}{}",
                "    ".repeat(depth),
                crate::workspace::last_part(folder)
            );
            offer(ui, folder, &label);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonquery_query::Kind::{JmesPath, Jq, JsonPath, JsonPointer};

    const KINDS: [ItemKind; 8] = [
        ItemKind::Query(Jq),
        ItemKind::Query(JsonPath),
        ItemKind::Query(JmesPath),
        ItemKind::Query(JsonPointer),
        ItemKind::Schema,
        ItemKind::Patch,
        ItemKind::Document,
        ItemKind::Url,
    ];

    /// How well `text` can be read on `background`: the ratio of their lightness, 1 to 21.
    fn contrast(background: Color32, text: Color32) -> f32 {
        fn lightness(color: Color32) -> f32 {
            let channel = |c: u8| {
                let c = f32::from(c) / 255.0;
                if c <= 0.03928 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
        }
        let (a, b) = (lightness(background), lightness(text));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn every_kind_has_a_tag_of_its_own_in_the_light_theme_and_in_the_dark_one() {
        for dark in [false, true] {
            let tags: Vec<(Color32, Color32)> =
                KINDS.iter().map(|kind| badge_colors(*kind, dark)).collect();
            for (i, one) in tags.iter().enumerate() {
                for other in &tags[i + 1..] {
                    assert_ne!(one.0, other.0, "two kinds have the background {:?}", one.0);
                    assert_ne!(one, other);
                }
            }
        }
        // …and the dark theme's is not the light theme's.
        for kind in KINDS {
            assert_ne!(
                badge_colors(kind, false),
                badge_colors(kind, true),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn the_text_of_a_tag_can_be_read_on_its_background_in_both_themes() {
        for dark in [false, true] {
            for kind in KINDS {
                let (background, text) = badge_colors(kind, dark);
                assert!(
                    contrast(background, text) >= 4.5,
                    "{kind:?} (dark: {dark}) is {:.1} to 1",
                    contrast(background, text)
                );
            }
        }
    }

    #[test]
    fn what_an_item_holds_is_shown_as_its_first_two_lines_that_have_anything_in_them() {
        assert_eq!(first_lines("\n  a  \n\n b \r\nc"), "a  b");
        assert_eq!(first_lines("only"), "only");
        assert_eq!(first_lines(" \n\t\n"), "");
        assert_eq!(
            first_lines("[InternetShortcut]\r\nURL=https://example.com/a\r\nIconIndex=0"),
            "[InternetShortcut]  URL=https://example.com/a"
        );
    }

    #[test]
    fn the_buttons_under_an_item_say_what_it_does_by_what_it_is() {
        assert_eq!(
            labels(ItemKind::Url, Front::Query).0,
            "Load from the web",
            "the button of a web address"
        );
        assert_eq!(labels(ItemKind::Query(Jq), Front::Query).0, "Open");
        assert_eq!(labels(ItemKind::Schema, Front::Query).0, "Use as schema");
        assert_eq!(labels(ItemKind::Patch, Front::Query).0, "Use as patch");
        assert_eq!(labels(ItemKind::Document, Front::Query).0, "Open as Source");
    }
}
