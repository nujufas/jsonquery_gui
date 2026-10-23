//! Taking a workspace somewhere else, and bringing one in: **export** copies the
//! workspace, or one folder of it, to a folder; **import** copies a folder into the
//! workspace. A workspace is a folder, so a folder is what goes in either direction
//! — to a stick, to a colleague, into a repository.
//!
//! What is copied is the files as they are, with the same folders, and (unless it
//! is asked not to) the samples; and, in the folder that is exported to, the
//! index, so that each item keeps its id and its SHA-256 where it goes. An import
//! reads those back: an item whose file is the same as one that is there already
//! (it has the same SHA-256) is not copied again, and one whose id is already in
//! use here is given a new one.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use super::files::{
    copy_atomic, is_inside, join, modified_ms, on_disk, parent, passes_through_a_link,
};
use super::ids::RandomIds;
use super::index::{Entry, Index, SampleEntry};
use super::names::{check_name, sample_file_name, unique_name, ItemKind};
use super::store::{scan, Folder, Item, Workspace};

/// What is exported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The whole workspace.
    Everything,
    /// One folder of it, by its relative path: what is in it becomes the top of
    /// what is exported.
    Folder(String),
}

/// What an export did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub items: usize,
    pub samples: usize,
    pub folders: usize,
    pub bytes: u64,
}

/// Where an import puts what it brings in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// In a new folder of this name in the workspace's own folder (with a number
    /// after the name if that is taken).
    NewFolder(String),
    /// In this folder of the workspace, among what is there ("" is the
    /// workspace's own folder).
    Inside(String),
}

/// What to do with an item that comes in under a name that an item of a different
/// content has already.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    /// Keep both: the one that comes in gets a number after its name.
    KeepBoth,
    /// The one that comes in takes the place of the one that is there.
    Replace,
    /// Leave the one that is there, and do not bring the other in.
    Skip,
}

/// What a folder holds, as far as an import would take it: for the window that asks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub folders: usize,
    pub items: usize,
    pub queries: usize,
    pub schemas: usize,
    pub patches: usize,
    pub documents: usize,
    pub urls: usize,
    pub samples: usize,
    pub sample_bytes: u64,
    pub notes: Vec<String>,
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    /// Items that were not there.
    pub added: usize,
    /// Items that took the place of one of the same name.
    pub replaced: usize,
    /// Items that came in under another name, because theirs was taken.
    pub renamed: usize,
    /// Items that were left out because the workspace had one of that name already.
    pub skipped: usize,
    /// Items that were left out because the workspace had the very same one.
    pub identical: usize,
    /// What went wrong with some of them, and what was left out.
    pub notes: Vec<String>,
}

impl Imported {
    /// How many items are in the workspace now that were not before.
    pub fn brought_in(&self) -> usize {
        self.added + self.renamed
    }
}

/// What a folder holds, as an import would see it.
pub fn preview(from: &Path) -> Result<Preview, String> {
    if !from.is_dir() {
        return Err(format!("{} is not a folder", from.display()));
    }
    let (old, _, mut notes) = Index::read(from);
    let scanned = scan(from, &old, &mut RandomIds::new());
    notes.extend(scanned.notes);
    let mut preview = Preview {
        folders: scanned.tree.folder_count(),
        notes,
        ..Preview::default()
    };
    // A sample that two items of a name share is one sample.
    let mut counted = HashSet::new();
    for item in scanned.tree.all_items() {
        preview.items += 1;
        match item.kind {
            ItemKind::Query(_) => preview.queries += 1,
            ItemKind::Schema => preview.schemas += 1,
            ItemKind::Patch => preview.patches += 1,
            ItemKind::Document => preview.documents += 1,
            ItemKind::Url => preview.urls += 1,
        }
        if let Some(sample) = &item.sample {
            if counted.insert(sample.path.clone()) {
                preview.samples += 1;
                preview.sample_bytes += sample.bytes;
            }
        }
    }
    Ok(preview)
}

/// What came of one item of an import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Added,
    Replaced,
    Renamed,
    Skipped,
    Identical,
}

/// Every folder below `folder`, each before its own.
fn folders_below(folder: &Folder) -> Vec<&Folder> {
    let mut found = Vec::new();
    for child in &folder.folders {
        found.push(child);
        found.extend(folders_below(child));
    }
    found
}

impl Workspace {
    /// Copy what `scope` says to the folder `to`, which is not there or is empty.
    pub fn export(
        &mut self,
        scope: &Scope,
        to: &Path,
        with_samples: bool,
    ) -> Result<Exported, String> {
        self.rescan();
        let source = match scope {
            Scope::Everything => self.tree.clone(),
            Scope::Folder(path) => match self.tree.find_folder(path) {
                Some(folder) if !folder.path.is_empty() => folder.clone(),
                _ => return Err("That folder is not in the workspace any more".to_owned()),
            },
        };
        if is_inside(to, &self.root) || is_inside(&self.root, to) {
            return Err(
                "Choose a folder that is not in the workspace, nor has it in it".to_owned(),
            );
        }
        let existed = to.exists();
        if existed {
            if !to.is_dir() {
                return Err(format!("{} is not a folder", to.display()));
            }
            let empty = fs::read_dir(to).is_ok_and(|mut entries| entries.next().is_none());
            if !empty {
                return Err(
                    "That folder is not empty: choose a new folder, or an empty one".to_owned(),
                );
            }
        }
        let result = self.copy_out(&source, to, with_samples);
        if result.is_err() && !existed {
            let _ = fs::remove_dir_all(to);
        }
        result
    }

    fn copy_out(&self, source: &Folder, to: &Path, with_samples: bool) -> Result<Exported, String> {
        let wrong = |what: &str, e: std::io::Error| format!("{what}: {e}");
        fs::create_dir_all(to).map_err(|e| wrong("The folder could not be made", e))?;
        // Where a path of the workspace is in what is exported: below the folder
        // that is its top.
        let below = |rel: &str| -> String {
            if source.path.is_empty() {
                rel.to_owned()
            } else if rel == source.path {
                String::new()
            } else {
                rel[source.path.len() + 1..].to_owned()
            }
        };
        let mut done = Exported::default();
        let mut index = Index::default();
        for folder in std::iter::once(source).chain(folders_below(source)) {
            let here = below(&folder.path);
            if !here.is_empty() {
                fs::create_dir_all(on_disk(to, &here))
                    .map_err(|e| wrong("A folder could not be made", e))?;
                done.folders += 1;
            }
            for item in &folder.items {
                let rel = below(&item.path);
                let target = on_disk(to, &rel);
                copy_atomic(&self.abs(&item.path), &target)
                    .map_err(|e| wrong(&format!("{} could not be copied", item.name), e))?;
                done.items += 1;
                done.bytes += item.bytes;
                let meta =
                    fs::metadata(&target).map_err(|e| wrong("A copy could not be read", e))?;
                let mut entry = Entry::new(
                    item.id.clone(),
                    item.sha256.clone(),
                    meta.len(),
                    modified_ms(&meta),
                );
                if let (true, Some(sample)) = (with_samples, &item.sample) {
                    let sample_target =
                        on_disk(to, &join(parent(&rel), &sample_file_name(&item.name)));
                    if !sample_target.exists() {
                        copy_atomic(&self.abs(&sample.path), &sample_target).map_err(|e| {
                            wrong(
                                &format!("The sample of {} could not be copied", item.name),
                                e,
                            )
                        })?;
                        done.samples += 1;
                        done.bytes += sample.bytes;
                    }
                    if let Ok(meta) = fs::metadata(&sample_target) {
                        entry.sample = Some(SampleEntry {
                            sha256: sample.sha256.clone(),
                            bytes: meta.len(),
                            modified: modified_ms(&meta),
                        });
                    }
                }
                index.items.insert(rel, entry);
            }
        }
        if !index.items.is_empty() {
            index
                .write(to)
                .map_err(|e| wrong("The index could not be written", e))?;
        }
        Ok(done)
    }

    /// Copy the folder `from` into the workspace, as `place` says, and say what was done.
    pub fn import(&mut self, from: &Path, place: &Place, clash: Clash) -> Result<Imported, String> {
        self.rescan();
        if !from.is_dir() {
            return Err(format!("{} is not a folder", from.display()));
        }
        if is_inside(from, &self.root) || is_inside(&self.root, from) {
            return Err(
                "Choose a folder that is not in the workspace, nor has it in it".to_owned(),
            );
        }
        let base = match place {
            Place::NewFolder(name) => {
                let name = check_name(name)?;
                unique_name(
                    |n| {
                        self.tree
                            .folders
                            .iter()
                            .any(|f| f.name.eq_ignore_ascii_case(n))
                            || self.abs(n).exists()
                    },
                    &name,
                )
            }
            Place::Inside(folder) => {
                let folder = self.checked_folder(folder)?;
                if !folder.is_empty() && self.tree.find_folder(&folder).is_none() {
                    return Err(format!("There is no folder {folder:?} any more"));
                }
                folder
            }
        };

        let (old, _, mut notes) = Index::read(from);
        let scanned = scan(from, &old, self.ids.as_mut());
        notes.extend(scanned.notes);
        let source = scanned.tree;
        if source.item_count() == 0 && source.folder_count() == 0 {
            return Err("There is nothing in that folder that the workspace can use".to_owned());
        }

        let mut done = Imported::default();
        // The ids that came in, which the next are not to take again.
        let mut ids_used: HashSet<String> = HashSet::new();
        if matches!(place, Place::NewFolder(_)) {
            fs::create_dir_all(self.abs(&base))
                .map_err(|e| format!("The folder could not be made: {e}"))?;
        }
        for folder in std::iter::once(&source).chain(folders_below(&source)) {
            let dest_folder = join(&base, &folder.path);
            if passes_through_a_link(&self.root, &dest_folder) {
                notes.push(format!(
                    "{dest_folder} is a link: what is for it is left out"
                ));
                continue;
            }
            if !folder.path.is_empty() {
                fs::create_dir_all(self.abs(&dest_folder))
                    .map_err(|e| format!("A folder could not be made: {e}"))?;
            }
            for item in &folder.items {
                match self.bring_in(from, item, &dest_folder, clash, &mut ids_used) {
                    Ok(Outcome::Added) => done.added += 1,
                    Ok(Outcome::Replaced) => done.replaced += 1,
                    Ok(Outcome::Renamed) => done.renamed += 1,
                    Ok(Outcome::Skipped) => done.skipped += 1,
                    Ok(Outcome::Identical) => done.identical += 1,
                    Err(e) => notes.push(e),
                }
            }
        }
        done.notes = notes;
        self.commit();
        Ok(done)
    }

    /// Bring one item (and its sample) into `dest_folder`.
    fn bring_in(
        &mut self,
        from: &Path,
        item: &Item,
        dest_folder: &str,
        clash: Clash,
        ids_used: &mut HashSet<String>,
    ) -> Result<Outcome, String> {
        let there = self
            .tree
            .find_folder(dest_folder)
            .and_then(|f| {
                f.items
                    .iter()
                    .find(|i| i.file_name().eq_ignore_ascii_case(item.file_name()))
            })
            .cloned();
        if there.as_ref().is_some_and(|i| i.sha256 == item.sha256) {
            return Ok(Outcome::Identical);
        }
        let taken = there.is_some() || self.abs(&join(dest_folder, item.file_name())).exists();
        // Under which name the item goes in, and what that is called.
        let (name, file_name, outcome) = match (&there, taken, clash) {
            (_, false, _) => (
                item.name.clone(),
                item.file_name().to_owned(),
                Outcome::Added,
            ),
            (_, true, Clash::Skip) => return Ok(Outcome::Skipped),
            // Over an item that is there, in the case its name has. A file that is
            // there and is not an item is never written over.
            (Some(there), true, Clash::Replace) => (
                there.name.clone(),
                there.file_name().to_owned(),
                Outcome::Replaced,
            ),
            _ => {
                let name = unique_name(
                    |n| {
                        self.abs(&join(dest_folder, &format!("{n}{}", item.ext)))
                            .exists()
                            || (item.kind.takes_sample()
                                && self.abs(&join(dest_folder, &sample_file_name(n))).exists())
                    },
                    &item.name,
                );
                let file_name = format!("{name}{}", item.ext);
                (name, file_name, Outcome::Renamed)
            }
        };
        let rel = join(dest_folder, &file_name);
        let target = self.abs(&rel);
        copy_atomic(&on_disk(from, &item.path), &target)
            .map_err(|e| format!("{} could not be copied: {e}", item.name))?;
        if let Some(sample) = &item.sample {
            let sample_target = self.abs(&join(dest_folder, &sample_file_name(&name)));
            // A sample that is there goes with another item of the name, and is kept
            // unless this is a replacement.
            if !sample_target.exists() || outcome == Outcome::Replaced {
                copy_atomic(&on_disk(from, &sample.path), &sample_target)
                    .map_err(|e| format!("The sample of {} could not be copied: {e}", item.name))?;
            }
        }
        // Its id: the one it had, unless that is in use here; one that takes the
        // place of another item keeps that item's.
        let kept = match (outcome, self.index.items.get(&rel)) {
            (Outcome::Replaced, Some(entry)) if !entry.id.is_empty() => Some(entry.id.clone()),
            _ => None,
        };
        let id = match kept {
            Some(id) => id,
            None if self.index.items.values().any(|e| e.id == item.id)
                || ids_used.contains(&item.id) =>
            {
                self.new_id()
            }
            None => item.id.clone(),
        };
        ids_used.insert(id.clone());
        if let Ok(meta) = fs::metadata(&target) {
            let entry = Entry::new(id, item.sha256.clone(), meta.len(), modified_ms(&meta));
            self.index.items.insert(rel, entry);
        }
        Ok(outcome)
    }
}
