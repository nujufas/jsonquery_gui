//! The windows of the workspace pane: the one that saves what is in a box, the one
//! that makes a folder, the ones that import and export a workspace, and the one
//! that asks before something is deleted.
//!
//! Each is a small window over the main window, with its fields and its two
//! buttons; Enter does what the first says, and Esc the other. A form is a value of
//! [`Form`] that the pane holds while the window is open, and is dropped when it
//! closes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align2};
use jsonquery_core::{Document, DocumentSource};
use jsonquery_query::Kind as Engine;

use super::{Action, Env, Front, Waiting, WorkspacePane};
use crate::app::human_bytes;
use crate::tools::{Tool, ERROR_COLOR};
use crate::workspace::{
    check_item_name, check_name, default_name, shortcut_text, web_address, Clash, Imported,
    ItemKind, NewItem, Place, Preview, SampleSource, Scope, Workspace,
};

/// The window that is open, if one is.
pub(super) enum Form {
    None,
    Save(Box<SaveForm>),
    Folder(FolderForm),
    Import(Box<ImportForm>),
    Export(ExportForm),
    Delete(DeleteForm),
}

impl Form {
    pub(super) fn folder(parent: String) -> Self {
        Form::Folder(FolderForm {
            parent,
            name: String::new(),
            focus: true,
            error: None,
        })
    }

    pub(super) fn import(into: String) -> Self {
        Form::Import(Box::new(ImportForm {
            from: String::new(),
            preview: None,
            new_folder: true,
            new_name: String::new(),
            named: false,
            inside: into,
            clash: Clash::KeepBoth,
            error: None,
        }))
    }

    pub(super) fn export(folder: Option<String>, root: Option<PathBuf>) -> Self {
        let to = default_export_folder(root.as_deref());
        Form::Export(ExportForm {
            folder: folder.filter(|f| !f.is_empty()),
            whole: true,
            to,
            samples: true,
            error: None,
        })
    }

    pub(super) fn delete(workspace: Option<&Workspace>, path: &str) -> Self {
        let Some(workspace) = workspace else {
            return Form::None;
        };
        let (what, details) = if let Some(item) = workspace.item(path) {
            (
                format!("“{}”", item.name),
                if item.sample.is_some() {
                    "Its sample goes with it.".to_owned()
                } else {
                    String::new()
                },
            )
        } else if let Some(folder) = workspace.tree().find_folder(path) {
            (
                format!("the folder “{}”", folder.name),
                format!(
                    "It has {} and {} in it, with their samples.",
                    super::counted(folder.item_count(), "item", "items"),
                    super::counted(folder.folder_count(), "folder", "folders")
                ),
            )
        } else {
            return Form::None;
        };
        Form::Delete(DeleteForm {
            path: path.to_owned(),
            what,
            details,
            error: None,
        })
    }
}

/// What is saved: which box it is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum What {
    Query,
    Schema,
    Patch,
    Document,
    /// A web address, which is typed in the form.
    Url,
}

/// The texts that can be saved, as they were when the form was opened.
struct Sources {
    query: String,
    schema: Option<Arc<str>>,
    schema_document: Option<Arc<str>>,
    patch: Option<Arc<str>>,
    patch_document: Option<Arc<str>>,
}

pub(super) struct SaveForm {
    what: What,
    name: String,
    /// The name was typed, or is the name of the item the query came from: it does not
    /// follow the web address any more.
    named: bool,
    folder: String,
    /// A folder to make inside `folder`, for this item.
    subfolder: String,
    keep_sample: bool,
    /// The web address that is saved, when that is what is saved.
    address: String,
    focus: bool,
    /// The box of the address, and not the name's, is to take the focus.
    focus_address: bool,
    sources: Sources,
    engine: Engine,
    doc: Option<Arc<Document>>,
    sample_limit: u64,
    error: Option<String>,
}

impl SaveForm {
    /// The name is made from the address until the name is typed.
    fn follow_address(&mut self) {
        if !self.named {
            self.name = default_name(&self.address);
        }
    }
}

pub(super) struct FolderForm {
    parent: String,
    name: String,
    focus: bool,
    error: Option<String>,
}

pub(super) struct ImportForm {
    from: String,
    /// What the folder in `from` holds, or why it cannot be used; `None` while it has not
    /// been looked at (a folder that was only typed, or typed over).
    preview: Option<Result<Preview, String>>,
    new_folder: bool,
    new_name: String,
    /// The name of the new folder was typed, so it does not follow the folder that comes in.
    named: bool,
    inside: String,
    clash: Clash,
    error: Option<String>,
}

pub(super) struct ExportForm {
    /// The folder the person picked in the list, which can be exported alone.
    folder: Option<String>,
    whole: bool,
    to: String,
    samples: bool,
    error: Option<String>,
}

pub(super) struct DeleteForm {
    path: String,
    what: String,
    details: String,
    error: Option<String>,
}

/// A save that waits for the worker to write a document.
pub(super) struct PendingSave {
    pub name: String,
    folder: String,
    kind: ItemKind,
    /// The text of the item, if it is not the file that is written (a document is).
    body: Option<String>,
    replace: bool,
}

/// What a window does with itself after a frame.
enum Outcome {
    Keep,
    Close,
}

impl WorkspacePane {
    /// Draw the window that is open, if one is, and do what it is asked.
    pub fn windows(&mut self, ctx: &egui::Context, env: &Env<'_>) {
        if let Some(wish) = super::take_wish(ctx) {
            self.grant(ctx, wish);
        }
        if std::mem::take(&mut self.wants_save) {
            // The button that asked may be in a window of its own (the Query pane that was popped
            // out): the window that saves is the main window's, and it comes in front of that one.
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            self.open_save_form(env);
        }
        let form = std::mem::replace(&mut self.form, Form::None);
        self.form = match form {
            Form::None => Form::None,
            Form::Save(mut f) => match self.save_window(ctx, env, &mut f) {
                Outcome::Keep => Form::Save(f),
                Outcome::Close => Form::None,
            },
            Form::Folder(mut f) => match self.folder_window(ctx, &mut f) {
                Outcome::Keep => Form::Folder(f),
                Outcome::Close => Form::None,
            },
            Form::Import(mut f) => match self.import_window(ctx, &mut f) {
                Outcome::Keep => Form::Import(f),
                Outcome::Close => Form::None,
            },
            Form::Export(mut f) => match self.export_window(ctx, &mut f) {
                Outcome::Keep => Form::Export(f),
                Outcome::Close => Form::None,
            },
            Form::Delete(mut f) => match self.delete_window(ctx, &mut f) {
                Outcome::Keep => Form::Delete(f),
                Outcome::Close => Form::None,
            },
        };
    }

    /// What a page of the Tools asked for: the pane is shown, with the main window in
    /// front, and either the window that saves is opened or the list is narrowed to
    /// what the page uses.
    fn grant(&mut self, ctx: &egui::Context, wish: super::Wish) {
        use super::Wish;
        self.actions
            .push(Action::SetView(crate::settings::WorkspaceView::Open));
        match wish {
            Wish::SaveSchema => {
                self.prefer = Some(What::Schema);
                self.wants_save = true;
            }
            Wish::SavePatch => {
                self.prefer = Some(What::Patch);
                self.wants_save = true;
            }
            Wish::PickSchema => self.group = crate::workspace::Group::Schemas,
            Wish::PickPatch => self.group = crate::workspace::Group::Patches,
        }
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
    }

    /// Open the form that saves what is in the box in front, with what can be saved.
    fn open_save_form(&mut self, env: &Env<'_>) {
        let schema = env.tools.text_in_box(Tool::Validate, "Schema");
        let patch = env.tools.text_in_box(Tool::Patch, "Patch");
        let sources = Sources {
            query: env.query_text.to_owned(),
            schema,
            schema_document: env.tools.text_in_box(Tool::Validate, "Document"),
            patch,
            patch_document: env.tools.text_in_box(Tool::Patch, "Document"),
        };
        let query_ready = !env.query_text.trim().is_empty();
        let asked = self.prefer.take().filter(|what| match what {
            What::Schema => sources.schema.is_some(),
            What::Patch => sources.patch.is_some(),
            What::Query | What::Document | What::Url => true,
        });
        let what = match env.front {
            _ if asked.is_some() => asked.unwrap_or(What::Query),
            Front::Tool(Tool::Validate) if sources.schema.is_some() => What::Schema,
            Front::Tool(Tool::Patch) if sources.patch.is_some() => What::Patch,
            _ if query_ready => What::Query,
            _ if sources.schema.is_some() => What::Schema,
            _ if sources.patch.is_some() => What::Patch,
            _ if env.doc.is_some() => What::Document,
            _ => What::Query,
        };
        // A query that came from the workspace is saved back under its own name.
        let (name, folder) = match (&what, self.opened_item()) {
            (What::Query, Some((name, folder))) => (name, folder),
            _ => (String::new(), self.selected_folder()),
        };
        let named = !name.is_empty();
        let address = offered_address(env);
        // A web address is typed first, if there is none to begin with.
        let ask_for_address = what == What::Url && address.is_empty();
        let name = if what == What::Url && !named {
            default_name(&address)
        } else {
            name
        };
        self.form = Form::Save(Box::new(SaveForm {
            what,
            name,
            named,
            folder,
            subfolder: String::new(),
            keep_sample: false,
            address,
            focus: !ask_for_address,
            focus_address: ask_for_address,
            sources,
            engine: env.engine,
            doc: env.doc.cloned(),
            sample_limit: env.sample_limit,
            error: None,
        }));
    }

    /// The name and folder of the item the query box came from.
    fn opened_item(&self) -> Option<(String, String)> {
        let (ws, opened) = (self.workspace.as_ref()?, self.opened.as_ref()?);
        let item = ws.item_by_id(&opened.id)?;
        Some((item.name.clone(), item.folder().to_owned()))
    }

    fn save_window(&mut self, ctx: &egui::Context, env: &Env<'_>, form: &mut SaveForm) -> Outcome {
        let _ = env;
        let folders = self
            .workspace
            .as_ref()
            .map(|ws| ws.tree().folder_paths())
            .unwrap_or_default();
        let kind = match form.what {
            What::Query => ItemKind::Query(form.engine),
            What::Schema => ItemKind::Schema,
            What::Patch => ItemKind::Patch,
            What::Document => ItemKind::Document,
            What::Url => ItemKind::Url,
        };
        let name_problem = if form.name.trim().is_empty() {
            None
        } else {
            check_item_name(&form.name, kind).err()
        };
        let address_problem = (form.what == What::Url
            && !form.address.trim().is_empty()
            && web_address(&form.address).is_none())
        .then_some("That is not a web address (http:// or https://, no spaces)");
        let sub_problem = if form.subfolder.trim().is_empty() {
            None
        } else {
            check_name(&form.subfolder).err()
        };
        let target = target_folder(&form.folder, &form.subfolder);
        let file = format!("{}{}", form.name.trim(), kind.suffix());
        let rel = if target.is_empty() {
            file
        } else {
            format!("{target}/{file}")
        };
        let exists = self.workspace.as_ref().is_some_and(|ws| {
            ws.tree()
                .find_folder(&target)
                .is_some_and(|f| f.items.iter().any(|i| i.path.eq_ignore_ascii_case(&rel)))
        });
        let ready = !form.name.trim().is_empty()
            && name_problem.is_none()
            && sub_problem.is_none()
            && source_ready(form);

        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new("Save in the workspace")
            .id(egui::Id::new("workspace_save_form"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.horizontal(|ui| {
                    ui.label("Save");
                    for (what, label) in [
                        (What::Query, "the query"),
                        (What::Schema, "a schema"),
                        (What::Patch, "a patch"),
                        (What::Document, "the Source"),
                        (What::Url, "Web address"),
                    ] {
                        let available = what_is_available(form, what);
                        let response = ui.add_enabled(
                            available,
                            egui::Button::selectable(form.what == what, label),
                        );
                        if response.clicked() {
                            form.what = what;
                            if what == What::Url {
                                form.follow_address();
                                form.focus_address = form.address.is_empty();
                            }
                        }
                        let _ = response.on_disabled_hover_text(why_not(what));
                    }
                });
                ui.add_space(4.0);
                if form.what == What::Url {
                    submit |= address_box(ui, form, address_problem);
                }
                ui.label("Name");
                let name_id = egui::Id::new("workspace_save_name");
                if !ui.is_sizing_pass() && std::mem::take(&mut form.focus) {
                    ui.memory_mut(|m| m.request_focus(name_id));
                }
                let field = ui.add(
                    egui::TextEdit::singleline(&mut form.name)
                        .id(name_id)
                        .desired_width(f32::INFINITY),
                );
                crate::text_menu::attach(&field, &form.name, crate::text_menu::Kind::Typed);
                if field.changed() {
                    // (a name that was typed stays; one that was emptied is the address's again)
                    form.named = !form.name.trim().is_empty();
                }
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                ui.label("Folder");
                ui.horizontal(|ui| {
                    folder_combo(ui, "workspace_save_folder", &mut form.folder, &folders);
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("and a new folder in it:").weak());
                    ui.add(
                        egui::TextEdit::singleline(&mut form.subfolder)
                            .hint_text("(none)")
                            .desired_width(f32::INFINITY),
                    );
                });
                sample_choice(ui, form);
                ui.add_space(4.0);
                if let Some(problem) = name_problem.as_ref().or(sub_problem.as_ref()) {
                    ui.colored_label(ERROR_COLOR, problem);
                } else if !form.name.trim().is_empty() {
                    let mut text = format!("Saved as {rel}");
                    if form.keep_sample && what_has_sample(form) {
                        text.push_str(&format!(
                            "\nand its sample {}",
                            crate::workspace::sample_file(form.name.trim())
                        ));
                    }
                    ui.label(egui::RichText::new(text).small().monospace().weak());
                    if exists {
                        ui.label(
                            egui::RichText::new(format!(
                                "There is a {} of that name here: saving replaces it.",
                                kind.what()
                            ))
                            .small(),
                        );
                    }
                }
                if let Some(error) = &form.error {
                    ui.colored_label(ERROR_COLOR, error);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let label = if exists { "Replace" } else { "Save" };
                    if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        cancel = true;
                    }
                });
            });
        if cancel || !open {
            return Outcome::Close;
        }
        if submit && ready {
            return self.submit_save(ctx, form, kind, exists);
        }
        Outcome::Keep
    }

    /// Save what the form says: now, or, when the sample is a document that the
    /// worker has to write, once it has.
    fn submit_save(
        &mut self,
        ctx: &egui::Context,
        form: &mut SaveForm,
        kind: ItemKind,
        replace: bool,
    ) -> Outcome {
        let folder = target_folder(&form.folder, &form.subfolder);
        let name = form.name.trim().to_owned();
        let body: Option<String> = match form.what {
            What::Query => Some(form.sources.query.clone()),
            What::Schema => form.sources.schema.as_deref().map(str::to_owned),
            What::Patch => form.sources.patch.as_deref().map(str::to_owned),
            What::Document => None,
            What::Url => web_address(&form.address).map(shortcut_text),
        };
        // The sample: a text from the box next to it, or the Source, which the worker
        // writes.
        let sample_text: Option<String> = match (form.what, form.keep_sample) {
            (What::Schema, true) => form.sources.schema_document.as_deref().map(str::to_owned),
            (What::Patch, true) => form.sources.patch_document.as_deref().map(str::to_owned),
            _ => None,
        };
        let sample_doc = match (form.what, form.keep_sample) {
            (What::Query, true) => form.doc.clone(),
            _ => None,
        };
        let needs_worker = form.what == What::Document || sample_doc.is_some();
        if needs_worker {
            let Some(doc) = form.doc.clone() else {
                form.error = Some("There is no document open to keep".to_owned());
                return Outcome::Keep;
            };
            let Some(ws) = self.workspace() else {
                form.error = Some("There is nowhere to keep a workspace".to_owned());
                return Outcome::Keep;
            };
            let staged = ws.staging_path();
            let save = PendingSave {
                name: name.clone(),
                folder,
                kind,
                body,
                replace,
            };
            self.waiting = Some(Waiting {
                staged: staged.clone(),
                save,
            });
            self.say(ctx, format!("Saving “{name}”…"), false);
            self.actions.push(Action::WriteDocument { doc, to: staged });
            return Outcome::Close;
        }
        let Some(body) = body else {
            form.error = Some("There is nothing to save".to_owned());
            return Outcome::Keep;
        };
        let Some(ws) = self.workspace() else {
            form.error = Some("There is nowhere to keep a workspace".to_owned());
            return Outcome::Keep;
        };
        let result = ws.save(NewItem {
            folder: &folder,
            name: &name,
            kind,
            body: &body,
            sample: sample_text.map_or(SampleSource::Keep, SampleSource::Text),
            replace,
        });
        match result {
            Ok(item) => {
                self.after_save(ctx, &item.path, &item.name, &item.id, &body, kind);
                Outcome::Close
            }
            Err(e) => {
                form.error = Some(e);
                self.after_change();
                Outcome::Keep
            }
        }
    }

    /// An item was saved: it is picked, its folder is open, and, if it is the query
    /// of the box, the box came from it.
    fn after_save(
        &mut self,
        ctx: &egui::Context,
        path: &str,
        name: &str,
        id: &str,
        body: &str,
        kind: ItemKind,
    ) {
        let mut folder = crate::workspace::parent_of(path).to_owned();
        while !folder.is_empty() {
            self.open_folders.insert(folder.clone());
            folder = crate::workspace::parent_of(&folder).to_owned();
        }
        self.selected = Some(path.to_owned());
        if matches!(kind, ItemKind::Query(_)) {
            self.note_opened(id.to_owned(), body.to_owned());
        }
        self.after_change();
        self.say(ctx, format!("Saved “{name}”"), false);
    }

    /// The worker has written the document that a save was waiting for.
    pub(super) fn finish_save(&mut self, ctx: &egui::Context, waiting: Waiting) {
        let Waiting { staged, save } = waiting;
        let Some(ws) = self.workspace() else { return };
        let result = match &save.body {
            Some(body) => ws.save(NewItem {
                folder: &save.folder,
                name: &save.name,
                kind: save.kind,
                body,
                sample: SampleSource::Staged(staged.clone()),
                replace: save.replace,
            }),
            None => ws.save_staged(&save.folder, &save.name, save.kind, &staged, save.replace),
        };
        let _ = std::fs::remove_file(&staged);
        match result {
            Ok(item) => {
                let body = save.body.clone().unwrap_or_default();
                self.after_save(ctx, &item.path, &item.name, &item.id, &body, save.kind);
            }
            Err(e) => {
                self.after_change();
                self.say(ctx, e, true);
            }
        }
    }

    fn folder_window(&mut self, ctx: &egui::Context, form: &mut FolderForm) -> Outcome {
        let folders = self
            .workspace
            .as_ref()
            .map(|ws| ws.tree().folder_paths())
            .unwrap_or_default();
        let problem = if form.name.trim().is_empty() {
            None
        } else {
            check_name(&form.name).err()
        };
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new("New folder")
            .id(egui::Id::new("workspace_folder_form"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label("Name");
                let id = egui::Id::new("workspace_folder_name");
                if !ui.is_sizing_pass() && std::mem::take(&mut form.focus) {
                    ui.memory_mut(|m| m.request_focus(id));
                }
                let field = ui.add(
                    egui::TextEdit::singleline(&mut form.name)
                        .id(id)
                        .desired_width(f32::INFINITY),
                );
                crate::text_menu::attach(&field, &form.name, crate::text_menu::Kind::Typed);
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                ui.label("Inside");
                ui.horizontal(|ui| {
                    folder_combo(ui, "workspace_folder_parent", &mut form.parent, &folders);
                });
                if let Some(problem) = &problem {
                    ui.colored_label(ERROR_COLOR, problem);
                }
                if let Some(error) = &form.error {
                    ui.colored_label(ERROR_COLOR, error);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let ready = !form.name.trim().is_empty() && problem.is_none();
                    if ui
                        .add_enabled(ready, egui::Button::new("Make it"))
                        .clicked()
                    {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        cancel = true;
                    }
                });
            });
        if cancel || !open {
            return Outcome::Close;
        }
        if submit && problem.is_none() && !form.name.trim().is_empty() {
            let (parent, name) = (form.parent.clone(), form.name.clone());
            let result = self.workspace().map(|ws| ws.create_folder(&parent, &name));
            match result {
                Some(Ok(path)) => {
                    let mut folder = path.clone();
                    while !folder.is_empty() {
                        self.open_folders.insert(folder.clone());
                        folder = crate::workspace::parent_of(&folder).to_owned();
                    }
                    self.selected = Some(path);
                    self.after_change();
                    return Outcome::Close;
                }
                Some(Err(e)) => form.error = Some(e),
                None => form.error = Some("There is nowhere to keep a workspace".to_owned()),
            }
        }
        Outcome::Keep
    }

    fn import_window(&mut self, ctx: &egui::Context, form: &mut ImportForm) -> Outcome {
        let folders = self
            .workspace
            .as_ref()
            .map(|ws| ws.tree().folder_paths())
            .unwrap_or_default();
        let mut open = true;
        let mut cancel = false;
        let mut import = false;
        let mut look = false;
        let mut left_the_box = false;
        egui::Window::new("Import a workspace")
            .id(egui::Id::new("workspace_import_form"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label("From the folder");
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut form.from)
                            .desired_width(ui.available_width() - 110.0)
                            .hint_text("a folder of queries, schemas…"),
                    );
                    crate::text_menu::attach(&field, &form.from, crate::text_menu::Kind::Typed);
                    if field.changed() {
                        // (what was said of the folder before is not about this one)
                        form.preview = None;
                        form.error = None;
                    }
                    left_the_box = field.lost_focus();
                    if ui.button("Choose folder…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            form.from = path.display().to_string();
                            look = true;
                        }
                    }
                });
                match &form.preview {
                    Some(Ok(preview)) => {
                        ui.label(egui::RichText::new(describe(preview)).small());
                        for note in &preview.notes {
                            ui.label(egui::RichText::new(note).small().weak());
                        }
                    }
                    Some(Err(e)) => {
                        ui.colored_label(ERROR_COLOR, e);
                    }
                    None => {}
                }
                ui.add_space(6.0);
                ui.label("Put them");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut form.new_folder, true, "in a new folder");
                    let field = ui.add_enabled(
                        form.new_folder,
                        egui::TextEdit::singleline(&mut form.new_name).desired_width(160.0),
                    );
                    if field.changed() {
                        // (a name that was typed stays; one that was emptied is the folder's again)
                        form.named = !form.new_name.trim().is_empty();
                    }
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut form.new_folder, false, "among what is in");
                    ui.add_enabled_ui(!form.new_folder, |ui| {
                        folder_combo(ui, "workspace_import_inside", &mut form.inside, &folders);
                    });
                });
                ui.add_enabled_ui(!form.new_folder, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "If the name of an item is taken, and it holds something else",
                        )
                        .small()
                        .weak(),
                    );
                    ui.radio_value(
                        &mut form.clash,
                        Clash::KeepBoth,
                        "keep both (the new one gets a number)",
                    );
                    ui.radio_value(
                        &mut form.clash,
                        Clash::Replace,
                        "replace the one that is there",
                    );
                    ui.radio_value(&mut form.clash, Clash::Skip, "leave the new one out");
                });
                if let Some(error) = &form.error {
                    ui.colored_label(ERROR_COLOR, error);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    // (A folder that was only typed has not been looked at yet: Import looks.)
                    let ready = !form.from.trim().is_empty()
                        && (form.preview.is_none() || matches!(form.preview, Some(Ok(_))));
                    if ui.add_enabled(ready, egui::Button::new("Import")).clicked() {
                        import = true;
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        cancel = true;
                    }
                });
            });
        if to_look_at(form, left_the_box, import) {
            look = true;
        }
        if look {
            form.error = None;
            form.preview = Some(crate::workspace::preview(Path::new(form.from.trim())));
            // The new folder is named for the one that comes in, unless it was named.
            if !form.named {
                form.new_name = Path::new(form.from.trim())
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
        }
        if cancel || !open {
            return Outcome::Close;
        }
        if import && form.new_folder && form.new_name.trim().is_empty() {
            form.error = Some("Give the new folder a name".to_owned());
            import = false;
        }
        if import && !matches!(form.preview, Some(Ok(_))) {
            // (what was wrong with the folder is under it)
            import = false;
        }
        if import {
            let from = PathBuf::from(form.from.trim());
            let place = if form.new_folder {
                Place::NewFolder(form.new_name.clone())
            } else {
                Place::Inside(form.inside.clone())
            };
            let clash = if form.new_folder {
                Clash::KeepBoth
            } else {
                form.clash
            };
            let result = self.workspace().map(|ws| ws.import(&from, &place, clash));
            match result {
                Some(Ok(done)) => {
                    if let Place::NewFolder(_) = place {
                        self.open_folders.extend(
                            self.workspace
                                .as_ref()
                                .map(|ws| {
                                    ws.tree()
                                        .folders
                                        .iter()
                                        .map(|f| f.path.clone())
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default(),
                        );
                    }
                    self.after_change();
                    let message = imported_message(&done);
                    self.say(
                        ctx,
                        message,
                        !done.notes.is_empty()
                            && done.brought_in() == 0
                            && done.replaced == 0
                            && done.identical == 0
                            && done.skipped == 0,
                    );
                    return Outcome::Close;
                }
                Some(Err(e)) => form.error = Some(e),
                None => form.error = Some("There is nowhere to keep a workspace".to_owned()),
            }
        }
        Outcome::Keep
    }

    fn export_window(&mut self, ctx: &egui::Context, form: &mut ExportForm) -> Outcome {
        let (everything, folder, samples) = match &self.workspace {
            Some(ws) => {
                let folder = form
                    .folder
                    .as_ref()
                    .and_then(|path| ws.tree().find_folder(path))
                    .map(|f| (f.name.clone(), f.item_count()));
                let (mut count, mut bytes) = (0_usize, 0_u64);
                let mut seen = std::collections::HashSet::new();
                for item in ws.tree().all_items() {
                    if let Some(sample) = &item.sample {
                        if seen.insert(sample.path.clone()) {
                            count += 1;
                            bytes += sample.bytes;
                        }
                    }
                }
                (
                    (ws.tree().item_count(), ws.tree().folder_count()),
                    folder,
                    (count, bytes),
                )
            }
            None => ((0, 0), None, (0, 0)),
        };
        let mut open = true;
        let mut cancel = false;
        let mut export = false;
        egui::Window::new("Export the workspace")
            .id(egui::Id::new("workspace_export_form"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                let all = format!(
                    "everything: {} in {}",
                    super::counted(everything.0, "item", "items"),
                    super::counted(everything.1, "folder", "folders")
                );
                if let Some((name, count)) = &folder {
                    ui.label("What");
                    ui.radio_value(&mut form.whole, true, all);
                    ui.radio_value(
                        &mut form.whole,
                        false,
                        format!(
                            "only the folder “{name}”: {}",
                            super::counted(*count, "item", "items")
                        ),
                    );
                } else {
                    form.whole = true;
                    ui.label(format!(
                        "Everything: {} in {}",
                        super::counted(everything.0, "item", "items"),
                        super::counted(everything.1, "folder", "folders")
                    ));
                }
                ui.add_space(4.0);
                ui.label("To the folder");
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut form.to)
                            .desired_width(ui.available_width() - 110.0)
                            .hint_text("a new folder, or an empty one"),
                    );
                    crate::text_menu::attach(&field, &form.to, crate::text_menu::Kind::Typed);
                    if ui.button("Choose folder…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            form.to = path.display().to_string();
                        }
                    }
                });
                ui.checkbox(
                    &mut form.samples,
                    format!(
                        "include the samples ({}, {})",
                        super::counted(samples.0, "file", "files"),
                        human_bytes(samples.1)
                    ),
                );
                ui.label(
                    egui::RichText::new(
                        "A sample is real data: leave it out when you share the workspace.",
                    )
                    .small()
                    .weak(),
                );
                if let Some(error) = &form.error {
                    ui.colored_label(ERROR_COLOR, error);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!form.to.trim().is_empty(), egui::Button::new("Export"))
                        .clicked()
                    {
                        export = true;
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        cancel = true;
                    }
                });
            });
        if cancel || !open {
            return Outcome::Close;
        }
        if export {
            let to = PathBuf::from(form.to.trim());
            let scope = match (&form.folder, form.whole) {
                (Some(path), false) => Scope::Folder(path.clone()),
                _ => Scope::Everything,
            };
            let samples = form.samples;
            let result = self.workspace().map(|ws| ws.export(&scope, &to, samples));
            match result {
                Some(Ok(done)) => {
                    self.say(
                        ctx,
                        format!(
                            "Exported {}{} to {}",
                            super::counted(done.items, "item", "items"),
                            if done.samples > 0 {
                                format!(
                                    " and {}",
                                    super::counted(done.samples, "sample", "samples")
                                )
                            } else {
                                String::new()
                            },
                            super::abbreviate(&to)
                        ),
                        false,
                    );
                    return Outcome::Close;
                }
                Some(Err(e)) => form.error = Some(e),
                None => form.error = Some("There is nowhere to keep a workspace".to_owned()),
            }
        }
        Outcome::Keep
    }

    fn delete_window(&mut self, ctx: &egui::Context, form: &mut DeleteForm) -> Outcome {
        let mut open = true;
        let mut cancel = false;
        let mut delete = false;
        egui::Window::new("Delete")
            .id(egui::Id::new("workspace_delete_form"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(format!("Delete {}?", form.what));
                if !form.details.is_empty() {
                    ui.label(egui::RichText::new(&form.details).small().weak());
                }
                ui.label(egui::RichText::new("This cannot be undone.").small());
                if let Some(error) = &form.error {
                    ui.colored_label(ERROR_COLOR, error);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        delete = true;
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        cancel = true;
                    }
                });
            });
        if cancel || !open {
            return Outcome::Close;
        }
        if delete {
            let path = form.path.clone();
            let result = self.workspace().map(|ws| ws.delete(&path));
            match result {
                Some(Ok(done)) => {
                    self.selected = None;
                    self.after_change();
                    let message = if done.left > 0 {
                        format!(
                            "Deleted {}; {} in the folder {} left, and so is the folder",
                            super::counted(done.items, "item", "items"),
                            super::counted(done.left, "other file", "other files"),
                            if done.left == 1 { "was" } else { "were" }
                        )
                    } else {
                        format!("Deleted {}", super::counted(done.items, "item", "items"))
                    };
                    self.say(ctx, message, false);
                    return Outcome::Close;
                }
                Some(Err(e)) => form.error = Some(e),
                None => return Outcome::Close,
            }
        }
        Outcome::Keep
    }
}

/// The folder an item is saved in: the one that was picked, and the one made in it.
fn target_folder(folder: &str, subfolder: &str) -> String {
    let sub = subfolder.trim();
    if sub.is_empty() {
        folder.to_owned()
    } else if folder.is_empty() {
        sub.to_owned()
    } else {
        format!("{folder}/{sub}")
    }
}

/// A box that picks one of the workspace's folders; the empty path is the top.
fn folder_combo(ui: &mut egui::Ui, id: &str, chosen: &mut String, folders: &[String]) {
    let shown = if chosen.is_empty() {
        "(top)".to_owned()
    } else {
        chosen.replace('/', " / ")
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(240.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(chosen, String::new(), "(top)");
            for folder in folders {
                let depth = folder.matches('/').count();
                let label = format!(
                    "{}{}",
                    "    ".repeat(depth),
                    crate::workspace::last_part(folder)
                );
                ui.selectable_value(chosen, folder.clone(), label);
            }
        });
}

/// The web address that the box of the form starts with: the one the open document was
/// loaded from, or else the one in the Source field that has not been loaded (yet).
fn offered_address(env: &Env<'_>) -> String {
    let loaded = env.doc.and_then(|doc| match &doc.source {
        DocumentSource::Url(url) => web_address(url),
        _ => None,
    });
    loaded
        .or_else(|| web_address(env.source_text))
        .unwrap_or_default()
        .to_owned()
}

/// The box that takes the web address, with what is wrong with it under it. Gives
/// whether Enter was pressed in it.
fn address_box(ui: &mut egui::Ui, form: &mut SaveForm, problem: Option<&str>) -> bool {
    ui.label("Web address");
    let id = egui::Id::new("workspace_save_address");
    // (Not in the frame of a click, which was not on the box, and so takes the focus away from
    // a box that is made in it: this is the frame after the click on the choice.)
    if !ui.is_sizing_pass() && form.focus_address && !ui.input(|i| i.pointer.any_click()) {
        form.focus_address = false;
        ui.memory_mut(|m| m.request_focus(id));
    }
    let field = ui.add(
        egui::TextEdit::singleline(&mut form.address)
            .id(id)
            .hint_text("https://…")
            .desired_width(f32::INFINITY),
    );
    crate::text_menu::attach(&field, &form.address, crate::text_menu::Kind::Typed);
    if field.changed() {
        form.follow_address();
    }
    if let Some(problem) = problem {
        ui.colored_label(ERROR_COLOR, problem);
    }
    field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// Whether what the person wants to save is there to be saved. (A web address is typed
/// in the form: it is always there to be picked.)
fn what_is_available(form: &SaveForm, what: What) -> bool {
    match what {
        What::Query => !form.sources.query.trim().is_empty(),
        What::Schema => form.sources.schema.is_some(),
        What::Patch => form.sources.patch.is_some(),
        What::Document => form
            .doc
            .as_ref()
            .is_some_and(|doc| doc.byte_len <= form.sample_limit),
        What::Url => true,
    }
}

fn why_not(what: What) -> &'static str {
    match what {
        What::Query => "The query box is empty",
        What::Schema => "The Schema box of Validate has no text in it",
        What::Patch => "The Patch box of Patch has no text in it",
        What::Document => "There is no document open, or it is bigger than the Tools take",
        What::Url => "Type a web address",
    }
}

/// Whether there is what is to be saved: a web address has to be typed, and be one.
fn source_ready(form: &SaveForm) -> bool {
    match form.what {
        What::Url => web_address(&form.address).is_some(),
        what => what_is_available(form, what),
    }
}

/// Whether the thing saved can have a sample kept with it.
fn what_has_sample(form: &SaveForm) -> bool {
    match form.what {
        What::Query => form.doc.is_some(),
        What::Schema => form.sources.schema_document.is_some(),
        What::Patch => form.sources.patch_document.is_some(),
        What::Document | What::Url => false,
    }
}

/// The tick that keeps a sample, and what it is.
fn sample_choice(ui: &mut egui::Ui, form: &mut SaveForm) {
    let (label, available, why) = match form.what {
        What::Query => match &form.doc {
            Some(doc) if doc.byte_len <= form.sample_limit => (
                format!(
                    "Keep the Source as its sample ({})",
                    human_bytes(doc.byte_len)
                ),
                true,
                "",
            ),
            Some(_) => (
                "Keep the Source as its sample".to_owned(),
                false,
                "The Source is bigger than the Tools take",
            ),
            None => (
                "Keep the Source as its sample".to_owned(),
                false,
                "No document is open",
            ),
        },
        What::Schema => (
            "Keep the Document box of Validate as its sample".to_owned(),
            form.sources.schema_document.is_some(),
            "The Document box of Validate has no text in it",
        ),
        What::Patch => (
            "Keep the Document box of Patch as its sample".to_owned(),
            form.sources.patch_document.is_some(),
            "The Document box of Patch has no text in it",
        ),
        What::Document | What::Url => return,
    };
    if !available {
        form.keep_sample = false;
    }
    let response = ui.add_enabled(available, egui::Checkbox::new(&mut form.keep_sample, label));
    let _ = response.on_disabled_hover_text(why);
}

/// Whether the folder typed in the import form is to be looked at now: something is typed
/// that has not been looked at, and the box was left or Import was pressed.
fn to_look_at(form: &ImportForm, left_the_box: bool, import: bool) -> bool {
    form.preview.is_none() && !form.from.trim().is_empty() && (left_the_box || import)
}

/// What a folder that is to be imported holds, in a line.
fn describe(preview: &Preview) -> String {
    let mut parts = Vec::new();
    for (count, one, many) in [
        (preview.queries, "query", "queries"),
        (preview.schemas, "schema", "schemas"),
        (preview.patches, "patch", "patches"),
        (preview.documents, "document", "documents"),
        (preview.urls, "web address", "web addresses"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {}", if count == 1 { one } else { many }));
        }
    }
    let mut line = format!(
        "{} {} in {} {}: {}",
        preview.items,
        if preview.items == 1 { "item" } else { "items" },
        preview.folders,
        if preview.folders == 1 {
            "folder"
        } else {
            "folders"
        },
        parts.join(", ")
    );
    if preview.samples > 0 {
        line.push_str(&format!(
            "; {} with a sample ({})",
            preview.samples,
            human_bytes(preview.sample_bytes)
        ));
    }
    line
}

/// What an import did, in a line.
fn imported_message(done: &Imported) -> String {
    let mut parts = vec![format!(
        "Imported {}",
        super::counted(done.added + done.renamed + done.replaced, "item", "items")
    )];
    if done.renamed > 0 {
        parts.push(format!("{} under another name", done.renamed));
    }
    if done.replaced > 0 {
        parts.push(format!("{} replaced", done.replaced));
    }
    if done.identical > 0 {
        parts.push(format!("{} were there already", done.identical));
    }
    if done.skipped > 0 {
        parts.push(format!("{} left out", done.skipped));
    }
    let mut message = parts.join(", ");
    if !done.notes.is_empty() {
        message.push_str(&format!(" ({})", done.notes.join("; ")));
    }
    message
}

/// Where an export goes unless another is chosen: a new folder named for the day, in
/// the user's Documents, or in their home.
fn default_export_folder(root: Option<&Path>) -> String {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let Some(home) = home else {
        return String::new();
    };
    let documents = home.join("Documents");
    let base = if documents.is_dir() { documents } else { home };
    let _ = root;
    base.join(format!("jsonquery-workspace-{}", today()))
        .display()
        .to_string()
}

/// Today's date, as `2026-10-20` (in UTC).
fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The year, month and day of the day that is `days` days after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_part + 2) / 5 + 1) as u32;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    } as u32;
    (year + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_day_is_worked_out_from_the_days_since_1970() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_516), (2026, 3, 4));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // A leap day, and the day after.
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(19_783), (2024, 3, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn the_folder_an_item_goes_in_is_the_one_picked_and_the_one_made_in_it() {
        assert_eq!(target_folder("", ""), "");
        assert_eq!(target_folder("Logs", ""), "Logs");
        assert_eq!(target_folder("Logs", "  "), "Logs");
        assert_eq!(target_folder("", "New"), "New");
        assert_eq!(target_folder("Logs/Errors", " New "), "Logs/Errors/New");
    }

    #[test]
    fn a_typed_folder_is_looked_at_once_the_box_is_left_or_import_is_pressed() {
        let Form::Import(mut form) = Form::import(String::new()) else {
            unreachable!("an import form");
        };
        assert!(!to_look_at(&form, true, true), "nothing is typed");
        form.from = "  ".to_owned();
        assert!(!to_look_at(&form, true, true), "only spaces are typed");
        form.from = "/some/where".to_owned();
        assert!(!to_look_at(&form, false, false), "typing looks at nothing");
        assert!(to_look_at(&form, true, false), "the box was left");
        assert!(to_look_at(&form, false, true), "Import was pressed");
        form.preview = Some(Err("not a folder".to_owned()));
        assert!(!to_look_at(&form, true, true), "it was looked at already");
    }

    #[test]
    fn a_folder_that_is_to_be_imported_is_told_in_a_line() {
        let preview = Preview {
            folders: 5,
            items: 14,
            queries: 9,
            schemas: 3,
            patches: 1,
            documents: 1,
            urls: 0,
            samples: 6,
            sample_bytes: 22 * 1024,
            notes: Vec::new(),
        };
        assert_eq!(
            describe(&preview),
            "14 items in 5 folders: 9 queries, 3 schemas, 1 patch, 1 document; 6 with a sample (22.0 KB)"
        );
        let one = Preview {
            folders: 1,
            items: 1,
            queries: 1,
            ..Preview::default()
        };
        assert_eq!(describe(&one), "1 item in 1 folder: 1 query");
    }

    #[test]
    fn the_web_addresses_in_a_folder_are_told_in_a_line_too() {
        let some = Preview {
            folders: 1,
            items: 3,
            queries: 1,
            urls: 2,
            ..Preview::default()
        };
        assert_eq!(
            describe(&some),
            "3 items in 1 folder: 1 query, 2 web addresses"
        );
        let one = Preview {
            items: 1,
            urls: 1,
            ..Preview::default()
        };
        assert_eq!(describe(&one), "1 item in 0 folders: 1 web address");
    }

    #[test]
    fn what_an_import_did_is_told_in_a_line() {
        let done = Imported {
            added: 3,
            renamed: 1,
            replaced: 2,
            identical: 4,
            skipped: 1,
            notes: vec!["a note".to_owned()],
        };
        assert_eq!(
            imported_message(&done),
            "Imported 6 items, 1 under another name, 2 replaced, 4 were there already, 1 left out (a note)"
        );
        assert_eq!(imported_message(&Imported::default()), "Imported 0 items");
    }
}
