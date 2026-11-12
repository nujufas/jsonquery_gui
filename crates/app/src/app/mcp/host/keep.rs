//! What changes what is kept, with "Edit" allowed: the workspace (save an item, make
//! a folder, move, delete), a file (save_file), and the Source from a web address
//! (open_url). What deletes or replaces, writes outside the workspace, or fetches, is
//! asked of the user first, in the app (`ask.rs`), and done only if they allow it —
//! unless they chose "Always allow".

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonquery_mcp::{Input, SaveKind};
use jsonquery_query::reformat::{self, Indent};
use serde_json::{json, Value};

use super::super::ask::ASK_TIME;
use super::{full_path, AppHost, POLL};
use crate::workspace::{
    check_item_name, shortcut_text, web_address, ItemKind, NewItem, SampleSource,
};

/// `name` in `folder` (`""`: the top).
fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

fn parent(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(folder, _)| folder)
}

fn no_workspace() -> String {
    "the app has no workspace folder".to_owned()
}

impl AppHost {
    /// Ask the user, in the app, and wait for the answer: `Ok` if they allow it (or
    /// always allow it, and are not asked). The question is `title` (a sentence that
    /// ends in a colon) about `what`.
    pub(super) fn ask(
        &self,
        title: &str,
        what: &str,
        detail: Option<String>,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let id = self.with_app(|app| {
            (!app.mcp.always_allow())
                .then(|| app.mcp.asks.put(title.to_owned(), what.to_owned(), detail))
        })?;
        let Some(id) = id else {
            return Ok(());
        };
        // A window that is hidden or minimized flashes in the task bar.
        self.ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Critical),
        );
        self.shown();
        let deadline = Instant::now() + ASK_TIME;
        loop {
            if let Some(allowed) = self.with_app(|app| app.mcp.asks.take_answer(id))? {
                self.shown();
                return if allowed {
                    Ok(())
                } else {
                    Err(format!(
                        "the user did not allow it ({} {what}): it was not done",
                        title.trim_end_matches(':').to_lowercase()
                    ))
                };
            }
            let gave_up = if cancel.load(Ordering::Relaxed) {
                Some("cancelled".to_owned())
            } else if Instant::now() >= deadline {
                Some(format!(
                    "the user did not answer within {} s: it was not done",
                    ASK_TIME.as_secs()
                ))
            } else {
                None
            };
            if let Some(why) = gave_up {
                self.with_app(|app| app.mcp.asks.withdraw(id))?;
                self.shown();
                return Err(why);
            }
            std::thread::sleep(POLL * 5);
        }
    }

    /// `input` as JSON text, laid out as `indent` says.
    fn json_text(&self, input: &Input, indent: Indent) -> Result<(String, String), String> {
        let limit = self.with_app(|app| app.settings.limits.tools())?;
        let (value, name) = self.value_of(input, limit)?;
        let options = reformat::Options {
            indent,
            ..reformat::Options::default()
        };
        Ok((reformat::render(&value, &options), name))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn workspace_save(
        &self,
        folder: &str,
        name: &str,
        kind: SaveKind,
        content: &Input,
        sample: Option<&Input>,
        replace: bool,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let item_kind = match kind {
            SaveKind::Query(language) => ItemKind::Query(language),
            SaveKind::Schema => ItemKind::Schema,
            SaveKind::Patch => ItemKind::Patch,
            SaveKind::Document => ItemKind::Document,
            SaveKind::Address => ItemKind::Url,
        };
        // What goes in the files, read before the app is asked anything.
        let body = match (kind, content) {
            (SaveKind::Address, Input::Text(text)) => {
                let address = web_address(text).ok_or_else(|| {
                    format!("\"{}\" is not a web address (http or https)", text.trim())
                })?;
                shortcut_text(address)
            }
            (SaveKind::Query(_), Input::Text(text)) => text.clone(),
            _ => self.json_text(content, Indent::Spaces(2))?.0,
        };
        let sample = sample
            .map(|s| self.json_text(s, Indent::Spaces(2)).map(|(text, _)| text))
            .transpose()?;
        let name = check_item_name(name, item_kind)?;
        let rel = join(folder, &format!("{name}{}", item_kind.suffix()));
        let exists = self.with_app(|app| {
            let ws = app.workspace_pane.workspace().ok_or_else(no_workspace)?;
            Ok::<_, String>(ws.item(&rel).is_some())
        })??;
        if exists && !replace {
            return Err(format!(
                "the workspace has an item \"{rel}\" already: give replace true to replace it \
                 (the user is asked first), or another name"
            ));
        }
        if exists {
            self.ask(
                "Replace in the workspace:",
                &rel,
                Some("What it holds now is lost.".to_owned()),
                cancel,
            )?;
        }
        let saved = self.with_app(|app| {
            let ws = app.workspace_pane.workspace().ok_or_else(no_workspace)?;
            ws.save(NewItem {
                folder,
                name: &name,
                kind: item_kind,
                body: &body,
                sample: sample.map_or(SampleSource::Keep, SampleSource::Text),
                replace: exists,
            })
        })??;
        self.shown();
        Ok(json!({
            "saved": saved.path,
            "kind": item_kind.what(),
            "replaced": exists,
            "sample": saved.sample.is_some(),
        }))
    }

    pub(super) fn workspace_new_folder(&self, path: &str) -> Result<Value, String> {
        let made = self.with_app(|app| {
            let ws = app.workspace_pane.workspace().ok_or_else(no_workspace)?;
            let mut made = Vec::new();
            let mut at = String::new();
            for part in path.split('/') {
                let next = join(&at, part);
                if ws.tree().find_folder(&next).is_none() {
                    if ws.item(&next).is_some() {
                        return Err(format!("\"{next}\" is an item, not a folder"));
                    }
                    made.push(ws.create_folder(&at, part)?);
                }
                at = next;
            }
            Ok::<_, String>(made)
        })??;
        self.shown();
        Ok(json!({"folder": path, "made": made}))
    }

    pub(super) fn workspace_move(
        &self,
        path: &str,
        to: Option<&str>,
        name: Option<&str>,
    ) -> Result<Value, String> {
        let now = self.with_app(|app| {
            let ws = app.workspace_pane.workspace().ok_or_else(no_workspace)?;
            if ws.item(path).is_none() && ws.tree().find_folder(path).is_none() {
                return Err(format!(
                    "the workspace has no item or folder \"{path}\": workspace_list gives their paths"
                ));
            }
            let mut at = path.to_owned();
            if let Some(to) = to {
                if parent(&at) != to {
                    if !to.is_empty() && ws.tree().find_folder(to).is_none() {
                        return Err(format!(
                            "the workspace has no folder \"{to}\": workspace_new_folder makes one"
                        ));
                    }
                    at = ws.move_to(&at, to)?;
                }
            }
            if let Some(name) = name {
                at = ws.rename(&at, name)?;
            }
            Ok::<_, String>(at)
        })??;
        self.shown();
        Ok(json!({"moved": path, "now": now}))
    }

    pub(super) fn workspace_delete(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let detail = self.with_app(|app| {
            let ws = app.workspace_pane.workspace().ok_or_else(no_workspace)?;
            if let Some(item) = ws.item(path) {
                let sample = if item.sample.is_some() { ", and its sample" } else { "" };
                return Ok(format!("The {}{sample}.", item.kind.what()));
            }
            match ws.tree().find_folder(path) {
                Some(folder) => Ok(format!(
                    "The folder and all that is in it ({} item(s)).",
                    folder.item_count()
                )),
                None => Err(format!(
                    "the workspace has no item or folder \"{path}\": workspace_list gives their paths"
                )),
            }
        })??;
        self.ask("Delete from the workspace:", path, Some(detail), cancel)?;
        let deleted = self.with_app(|app| {
            let ws = app.workspace_pane.workspace().ok_or_else(no_workspace)?;
            ws.delete(path)
        })??;
        self.shown();
        Ok(json!({"deleted": path, "items": deleted.items, "left": deleted.left}))
    }

    pub(super) fn save_file(
        &self,
        input: &Input,
        path: &Path,
        indent: Indent,
        replace: bool,
        cancel: &AtomicBool,
    ) -> Result<Value, String> {
        let path = full_path(path)?;
        if path.is_dir() {
            return Err(format!(
                "{} is a folder: give a file's path",
                path.display()
            ));
        }
        let folder = path
            .parent()
            .filter(|p| p.is_dir())
            .ok_or_else(|| format!("the folder of {} is not there", path.display()))?;
        let exists = path.exists();
        if exists && !replace {
            return Err(format!(
                "{} is there already: give replace true to replace it (the user is asked first)",
                path.display()
            ));
        }
        let (text, name) = self.json_text(input, indent)?;
        let detail = format!(
            "{}: {name}, {} bytes.",
            if exists {
                "It replaces the file that is there"
            } else {
                "A new file"
            },
            text.len()
        );
        self.ask(
            "Save a file:",
            &path.display().to_string(),
            Some(detail),
            cancel,
        )?;
        // Beside it first, then in its place: never half written.
        let staged = folder.join(format!(
            ".{}.jsonquery-{}",
            path.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            std::process::id()
        ));
        std::fs::write(&staged, text.as_bytes())
            .and_then(|()| std::fs::rename(&staged, &path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&staged);
                format!("{} could not be written: {e}", path.display())
            })?;
        Ok(json!({
            "saved": path.display().to_string(),
            "of": name,
            "bytes": text.len(),
            "replaced": exists,
        }))
    }

    pub(super) fn open_url(
        &self,
        url: &str,
        timeout: Duration,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, String> {
        self.ask(
            "Load a web address as the Source:",
            url,
            Some("JsonQuery fetches it from this computer.".to_owned()),
            cancel,
        )?;
        let before = self.with_app(|app| {
            let before = app.doc.as_ref().map(Arc::as_ptr);
            app.tools.show_query();
            app.load_error = None;
            app.open_url(url.to_owned());
            before
        })?;
        self.shown();
        self.loaded_since(before, timeout, cancel)
    }
}
