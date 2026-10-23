//! The workspace as a folder on disk: reading it into a tree of folders and items,
//! keeping each item's id and SHA-256 (`index.rs`), and the things done to it —
//! save an item, make a folder, rename, move, copy and delete.
//!
//! Everything is made so that a mistake costs as little as it can: a file is
//! written whole or not at all, nothing is written through a link, a name that is
//! taken is not taken over unless that was asked for, a folder is deleted only as
//! far as what the workspace itself put in it, and a folder that holds files that
//! are not the workspace's keeps them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::files::{
    copy_atomic, is_relative_path, join, last, modified_ms, move_file, on_disk, parent,
    passes_through_a_link, write_atomic,
};
use super::ids::{sha256_file, sha256_hex, IdSource, RandomIds};
use super::index::{self, Entry, Index, SampleEntry, State};
use super::names::{
    check_item_name, check_name, parse_file_name, sample_file_name, unique_name, ItemKind,
    SAMPLE_SUFFIX,
};

/// How many folders deep the workspace is read.
pub const MAX_DEPTH: usize = 16;

/// How many files and folders the workspace is read up to: a folder that has
/// more is not a workspace, and reading it would only make the app slow.
pub const MAX_ENTRIES: usize = 100_000;

/// How long a file that was left in the app's own folder, to be moved into the
/// workspace, is kept before it is taken away as left over.
const STAGED_KEPT: Duration = Duration::from_secs(24 * 60 * 60);

/// A sample, as it is on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    /// The relative path of its file.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// What is stored: one file of the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The id it was given when it was first saved, which it keeps.
    pub id: String,
    /// The SHA-256 of what is in its file.
    pub sha256: String,
    /// What it is called: its file's name without the end that says what it is.
    pub name: String,
    pub kind: ItemKind,
    /// The relative path of its file.
    pub path: String,
    pub bytes: u64,
    /// The file `<name>.sample.json` beside it, if there is one and the kind of item has
    /// a sample to have (`ItemKind::takes_sample`).
    pub sample: Option<Sample>,
    /// The end its file has, as it is written.
    pub(super) ext: String,
}

impl Item {
    /// The name of its file.
    pub fn file_name(&self) -> &str {
        last(&self.path)
    }

    /// The folder it is in.
    pub fn folder(&self) -> &str {
        parent(&self.path)
    }
}

/// A folder of the workspace, with what is in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    /// Its name: empty for the workspace's own folder.
    pub name: String,
    /// Its relative path: empty for the workspace's own folder.
    pub path: String,
    pub folders: Vec<Folder>,
    pub items: Vec<Item>,
}

impl Folder {
    fn sort(&mut self) {
        self.folders.sort_by(|a, b| {
            (a.name.to_lowercase(), &a.name).cmp(&(b.name.to_lowercase(), &b.name))
        });
        self.items.sort_by(|a, b| {
            (a.name.to_lowercase(), &a.name, a.kind.suffix()).cmp(&(
                b.name.to_lowercase(),
                &b.name,
                b.kind.suffix(),
            ))
        });
    }

    /// The folder at the relative path `path` below this one.
    pub fn find_folder(&self, path: &str) -> Option<&Folder> {
        let mut here = self;
        if path.is_empty() {
            return Some(here);
        }
        for part in path.split('/') {
            here = here.folders.iter().find(|f| f.name == part)?;
        }
        Some(here)
    }

    /// The item whose file is at the relative path `path` below this folder.
    pub fn find_item(&self, path: &str) -> Option<&Item> {
        self.find_folder(parent(path))?
            .items
            .iter()
            .find(|item| item.path == path)
    }

    /// Every item in this folder and below it, a folder's own before its subfolders'.
    pub fn all_items(&self) -> Vec<&Item> {
        let mut found: Vec<&Item> = self.items.iter().collect();
        for folder in &self.folders {
            found.extend(folder.all_items());
        }
        found
    }

    /// How many items are in this folder and below it.
    pub fn item_count(&self) -> usize {
        self.items.len() + self.folders.iter().map(Folder::item_count).sum::<usize>()
    }

    /// How many folders are below this one.
    pub fn folder_count(&self) -> usize {
        self.folders.len() + self.folders.iter().map(Folder::folder_count).sum::<usize>()
    }

    /// The relative path of every folder below this one, each before its own
    /// subfolders.
    pub fn folder_paths(&self) -> Vec<String> {
        let mut found = Vec::new();
        for folder in &self.folders {
            found.push(folder.path.clone());
            found.extend(folder.folder_paths());
        }
        found
    }

    /// Whether this folder has a file called `file_name` in it, whatever the case,
    /// other than the one at `except`.
    fn has_file(&self, file_name: &str, except: &str) -> bool {
        self.items
            .iter()
            .any(|i| i.path != except && i.file_name().eq_ignore_ascii_case(file_name))
    }
}

/// What a read of the folder on disk found.
pub(super) struct Scanned {
    pub tree: Folder,
    /// The index as it should be, now that the files have been seen.
    pub index: Index,
    pub notes: Vec<String>,
    /// Whether that is not the index there was.
    pub changed: bool,
}

/// A file that is an item, as the folder shows it.
struct Found {
    rel: String,
    name: String,
    kind: ItemKind,
    ext: String,
    bytes: u64,
    modified: u64,
}

/// A sample file, as the folder shows it.
struct Meta {
    bytes: u64,
    modified: u64,
}

#[derive(Default)]
struct Walk {
    folders: Vec<String>,
    files: Vec<Found>,
    samples: HashMap<String, Meta>,
    notes: Vec<String>,
}

/// Look through the folder `root`: its subfolders, the files that are items, and
/// the samples. A name that starts with a dot is not the workspace's (the index is
/// in one), nor is a link, which is never followed. Up to `max_entries` files and
/// folders are read.
fn walk(root: &Path, max_entries: usize) -> Walk {
    let mut walk = Walk::default();
    let (mut links, mut odd_names, mut too_deep, mut unreadable) = (0, 0, 0, 0);
    let mut seen = 0_usize;
    let mut stack = vec![(String::new(), 0_usize)];
    'folders: while let Some((rel, depth)) = stack.pop() {
        let entries = match fs::read_dir(on_disk(root, &rel)) {
            Ok(entries) => entries,
            Err(e) if rel.is_empty() && e.kind() == std::io::ErrorKind::NotFound => return walk,
            Err(e) => {
                let place = if rel.is_empty() {
                    root.display().to_string()
                } else {
                    rel.clone()
                };
                walk.notes.push(format!("{place} could not be read: {e}"));
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else {
                unreadable += 1;
                continue;
            };
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                odd_names += 1;
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                unreadable += 1;
                continue;
            };
            if meta.file_type().is_symlink() {
                links += 1;
                continue;
            }
            seen += 1;
            if seen > max_entries {
                walk.notes.push(format!(
                    "The workspace has more than {max_entries} files and folders: the rest are left out"
                ));
                break 'folders;
            }
            let child = join(&rel, &name);
            if meta.is_dir() {
                if depth + 1 > MAX_DEPTH {
                    too_deep += 1;
                    continue;
                }
                walk.folders.push(child.clone());
                stack.push((child, depth + 1));
            } else if meta.is_file() {
                let (bytes, modified) = (meta.len(), modified_ms(&meta));
                if let Some(parsed) = parse_file_name(&name) {
                    walk.files.push(Found {
                        rel: child,
                        name: parsed.name,
                        kind: parsed.kind,
                        ext: parsed.ext,
                        bytes,
                        modified,
                    });
                } else if name.ends_with(SAMPLE_SUFFIX) {
                    walk.samples.insert(child, Meta { bytes, modified });
                }
            }
        }
    }
    for (count, said) in [
        (links, "links in the workspace are not followed"),
        (odd_names, "files with names that are not text are left out"),
        (too_deep, "folders more than 16 levels down are left out"),
        (unreadable, "entries could not be read and are left out"),
    ] {
        if count > 0 {
            walk.notes.push(format!("{count} {said}"));
        }
    }
    walk.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    walk
}

/// Read the folder `root` against `old`, the index there was: the tree it holds,
/// and the index that matches it. Nothing is written. See `index.rs` for what is
/// made of a file that is new, changed, moved or gone.
pub(super) fn scan(root: &Path, old: &Index, ids: &mut dyn IdSource) -> Scanned {
    scan_up_to(root, old, ids, MAX_ENTRIES)
}

/// [`scan`], reading up to `max_entries` files and folders.
pub(super) fn scan_up_to(
    root: &Path,
    old: &Index,
    ids: &mut dyn IdSource,
    max_entries: usize,
) -> Scanned {
    let mut walk = walk(root, max_entries);
    let mut notes = std::mem::take(&mut walk.notes);

    // The entries whose file is not where it was, by what they held: a file that
    // is new and holds the same is the one that was moved or renamed.
    let present: HashSet<&str> = walk.files.iter().map(|f| f.rel.as_str()).collect();
    let mut gone: HashMap<&str, Vec<&str>> = HashMap::new();
    for (path, entry) in &old.items {
        if !present.contains(path.as_str()) && !entry.sha256.is_empty() {
            gone.entry(entry.sha256.as_str())
                .or_default()
                .push(path.as_str());
        }
    }
    let old_ids: HashSet<&str> = old.items.values().map(|e| e.id.as_str()).collect();

    let mut used: HashSet<String> = HashSet::new();
    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut items: Vec<Item> = Vec::new();
    for found in &walk.files {
        let prior = old.items.get(&found.rel);
        let unchanged = prior.filter(|e| {
            !e.sha256.is_empty() && e.bytes == found.bytes && e.modified == found.modified
        });
        let sha = match unchanged {
            Some(entry) => entry.sha256.clone(),
            None => match sha256_file(&on_disk(root, &found.rel)) {
                Ok((sha, _)) => sha,
                Err(e) => {
                    notes.push(format!("{} could not be read: {e}", found.rel));
                    continue;
                }
            },
        };
        let base: Option<&Entry> = match prior {
            Some(entry) => Some(entry),
            None => match gone.get_mut(sha.as_str()) {
                Some(paths) if !paths.is_empty() => old.items.get(paths.remove(0)),
                _ => None,
            },
        };
        let mut id = base.map(|e| e.id.clone()).unwrap_or_default();
        if id.is_empty() || used.contains(&id) {
            id = loop {
                let fresh = ids.new_id();
                if !used.contains(&fresh) && !old_ids.contains(fresh.as_str()) {
                    break fresh;
                }
            };
        }
        used.insert(id.clone());

        let sample_path = join(parent(&found.rel), &sample_file_name(&found.name));
        // (A web address has none, whatever file is called after it.)
        let sample = walk
            .samples
            .get(&sample_path)
            .filter(|_| found.kind.takes_sample())
            .and_then(|meta| {
                let cached = base
                    .and_then(|e| e.sample.as_ref())
                    .filter(|s| s.bytes == meta.bytes && s.modified == meta.modified);
                match cached {
                    Some(sample) => Some(sample.clone()),
                    None => sha256_file(&on_disk(root, &sample_path))
                        .ok()
                        .map(|(sha, _)| SampleEntry {
                            sha256: sha,
                            bytes: meta.bytes,
                            modified: meta.modified,
                        }),
                }
            });

        let mut entry = match base {
            Some(base) => base.renewed(id.clone(), sha.clone(), found.bytes, found.modified),
            None => Entry::new(id.clone(), sha.clone(), found.bytes, found.modified),
        };
        entry.sample = sample.clone();
        items.push(Item {
            id,
            sha256: sha,
            name: found.name.clone(),
            kind: found.kind,
            path: found.rel.clone(),
            bytes: found.bytes,
            sample: sample.map(|s| Sample {
                path: sample_path,
                sha256: s.sha256,
                bytes: s.bytes,
            }),
            ext: found.ext.clone(),
        });
        entries.insert(found.rel.clone(), entry);
    }

    let changed = entries != old.items;
    Scanned {
        tree: build_tree(&walk.folders, items),
        index: old.rebuilt(entries),
        notes,
        changed,
    }
}

fn build_tree(folder_paths: &[String], items: Vec<Item>) -> Folder {
    let mut by_path: BTreeMap<String, Folder> = BTreeMap::new();
    by_path.insert(String::new(), Folder::default());
    for path in folder_paths {
        by_path.insert(
            path.clone(),
            Folder {
                name: last(path).to_owned(),
                path: path.clone(),
                ..Folder::default()
            },
        );
    }
    for item in items {
        if let Some(folder) = by_path.get_mut(parent(&item.path)) {
            folder.items.push(item);
        }
    }
    // The deepest first, so that a folder has what is in it when it is put in its own.
    let mut paths: Vec<String> = by_path.keys().filter(|p| !p.is_empty()).cloned().collect();
    paths.sort_by_key(|p| std::cmp::Reverse(p.matches('/').count()));
    for path in paths {
        if let Some(mut folder) = by_path.remove(&path) {
            folder.sort();
            if let Some(above) = by_path.get_mut(parent(&path)) {
                above.folders.push(folder);
            }
        }
    }
    let mut root = by_path.remove("").unwrap_or_default();
    root.sort();
    root
}

/// What to do with the sample of an item that is saved.
#[derive(Debug)]
pub enum SampleSource {
    /// Leave the sample the item has, if it has one.
    Keep,
    /// This text is the sample.
    Text(String),
    /// The sample is this file, written already (by the worker, which writes a
    /// document); it is moved to where the sample goes.
    Staged(PathBuf),
}

/// What an item's text comes from.
enum Body<'a> {
    Text(&'a str),
    /// A file that was written elsewhere, and is moved in.
    Staged(&'a Path),
}

/// What is saved: an item, in a folder, under a name.
pub struct NewItem<'a> {
    /// The folder, as a relative path (made if it is not there).
    pub folder: &'a str,
    pub name: &'a str,
    pub kind: ItemKind,
    pub body: &'a str,
    pub sample: SampleSource,
    /// Take the place of the item of that name, if there is one; if not, there
    /// is an error.
    pub replace: bool,
}

/// What a delete did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Deleted {
    /// How many items were taken away.
    pub items: usize,
    /// How many files there were in a folder that are not the workspace's, which
    /// are left (and so is the folder).
    pub left: usize,
}

/// Whether `a` and `b` are the same file: what two spellings of a name are on a
/// system that does not tell the cases apart.
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// A workspace: the folder, what it holds as a tree, and what is known of every item.
pub struct Workspace {
    pub(super) root: PathBuf,
    pub(super) ids: Box<dyn IdSource>,
    pub(super) index: Index,
    pub(super) state: State,
    pub(super) tree: Folder,
    pub(super) notes: Vec<String>,
}

impl Workspace {
    /// The workspace in the folder `root`, read. A folder that is not there is an
    /// empty workspace, and is made when something is saved in it.
    pub fn open(root: PathBuf) -> Self {
        Self::with_ids(root, Box::new(RandomIds::new()))
    }

    pub fn with_ids(root: PathBuf, ids: Box<dyn IdSource>) -> Self {
        let mut workspace = Self {
            root,
            ids,
            index: Index::default(),
            state: State::Fresh,
            tree: Folder::default(),
            notes: Vec::new(),
        };
        workspace.sweep_staging();
        workspace.rescan();
        workspace
    }

    pub fn tree(&self) -> &Folder {
        &self.tree
    }

    /// What was wrong with the last read: what was left out, and why.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Whether the folder is there.
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    pub fn item(&self, path: &str) -> Option<&Item> {
        self.tree.find_item(path)
    }

    pub fn item_by_id(&self, id: &str) -> Option<&Item> {
        self.tree.all_items().into_iter().find(|item| item.id == id)
    }

    /// Where the relative path `rel` is on disk.
    pub fn abs(&self, rel: &str) -> PathBuf {
        on_disk(&self.root, rel)
    }

    /// Read the folder again, as another program may have changed it, and make the
    /// index right (and write it, if it was not).
    pub fn rescan(&mut self) {
        let (old, state, mut notes) = Index::read(&self.root);
        self.state = state;
        let scanned = scan(&self.root, &old, self.ids.as_mut());
        notes.extend(scanned.notes);
        self.tree = scanned.tree;
        if scanned.changed {
            self.write_index(&scanned.index, &mut notes);
        }
        self.index = scanned.index;
        self.notes = notes;
    }

    fn write_index(&mut self, index: &Index, notes: &mut Vec<String>) {
        if !self.state.writable() {
            return;
        }
        // A folder that is empty and has no index has nothing to say, and is not
        // given a folder of the app's own.
        if index.items.is_empty() && self.state == State::Fresh {
            return;
        }
        if self.state == State::Unusable {
            let file = index::path(&self.root);
            let mut aside = file.clone().into_os_string();
            aside.push(".bad");
            let _ = fs::rename(&file, PathBuf::from(aside));
        }
        match index.write(&self.root) {
            Ok(()) => self.state = State::Read,
            Err(e) => notes.push(format!(
                "The ids of the items could not be kept ({e}): they are made again each time"
            )),
        }
    }

    /// Write the index as the change made it, and read the folder again.
    pub(super) fn commit(&mut self) {
        let index = std::mem::take(&mut self.index);
        let mut notes = Vec::new();
        self.write_index(&index, &mut notes);
        self.index = index;
        self.rescan();
        self.notes.extend(notes);
    }

    pub(super) fn new_id(&mut self) -> String {
        loop {
            let id = self.ids.new_id();
            if !self.index.items.values().any(|e| e.id == id) {
                return id;
            }
        }
    }

    /// Take away what was left in the app's own folder, to be moved into the
    /// workspace, by a run that ended before it was.
    fn sweep_staging(&self) {
        let Ok(entries) = fs::read_dir(self.root.join(index::FOLDER)) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let old = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age > STAGED_KEPT);
            if name.starts_with("staged-") && old {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    /// A folder path from the person at the keyboard or from the tree: one that
    /// is there is taken as it is; one that is to be made has parts that are all
    /// names a folder can have. None of it may go through a link.
    pub(super) fn checked_folder(&self, folder: &str) -> Result<String, String> {
        if folder.is_empty() {
            return Ok(String::new());
        }
        if !is_relative_path(folder) {
            return Err("That is not a folder in the workspace".to_owned());
        }
        if self.tree.find_folder(folder).is_none() {
            for part in folder.split('/') {
                if check_name(part)? != part {
                    return Err(format!("{part:?} is not a name that a folder can have"));
                }
            }
        }
        if passes_through_a_link(&self.root, folder) {
            return Err(
                "That folder is a link, which the workspace does not go through".to_owned(),
            );
        }
        Ok(folder.to_owned())
    }

    fn check_not_a_link(&self, rel: &str) -> Result<(), String> {
        if passes_through_a_link(&self.root, rel) {
            Err("That is a link, which the workspace does not go through".to_owned())
        } else {
            Ok(())
        }
    }

    /// Whether `folder` holds an item called `name`, of any kind, whatever the case.
    fn has_name(&self, folder: &str, name: &str) -> bool {
        self.tree
            .find_folder(folder)
            .is_some_and(|f| f.items.iter().any(|i| i.name.eq_ignore_ascii_case(name)))
    }

    /// Whether `folder` holds a folder called `name`, whatever the case.
    fn has_folder(&self, folder: &str, name: &str) -> bool {
        self.tree
            .find_folder(folder)
            .is_some_and(|f| f.folders.iter().any(|d| d.name.eq_ignore_ascii_case(name)))
    }

    /// Whether another item in the same folder has the name of `item` (and so the
    /// same sample). A web address has no sample, so it shares none, and is not one
    /// that keeps the sample of the items of its name.
    fn shares_sample(&self, item: &Item) -> bool {
        self.tree.find_folder(item.folder()).is_some_and(|f| {
            f.items
                .iter()
                .any(|i| i.path != item.path && i.name == item.name && i.kind.takes_sample())
        })
    }

    /// Save an item. Gives it as it is after.
    pub fn save(&mut self, new: NewItem<'_>) -> Result<Item, String> {
        self.save_body(
            new.folder,
            new.name,
            new.kind,
            Body::Text(new.body),
            new.sample,
            new.replace,
        )
    }

    /// Save an item whose text is in the file `staged`, written already (by the
    /// worker, which writes a document), which is moved to where the item goes.
    pub fn save_staged(
        &mut self,
        folder: &str,
        name: &str,
        kind: ItemKind,
        staged: &Path,
        replace: bool,
    ) -> Result<Item, String> {
        self.save_body(
            folder,
            name,
            kind,
            Body::Staged(staged),
            SampleSource::Keep,
            replace,
        )
    }

    fn save_body(
        &mut self,
        folder: &str,
        name: &str,
        kind: ItemKind,
        body: Body<'_>,
        sample: SampleSource,
        replace: bool,
    ) -> Result<Item, String> {
        // (Nothing is written, or taken from where it waits, for an item that has no sample.)
        if !kind.takes_sample() && !matches!(sample, SampleSource::Keep) {
            return Err(format!("A {} has no sample", kind.what()));
        }
        self.rescan();
        let name = check_item_name(name, kind)?;
        let folder = self.checked_folder(folder)?;
        let wanted = join(&folder, &format!("{name}{}", kind.suffix()));
        let existing = self
            .tree
            .find_folder(&folder)
            .and_then(|f| {
                f.items
                    .iter()
                    .find(|i| i.path.eq_ignore_ascii_case(&wanted))
            })
            .cloned();
        if existing.is_some() && !replace {
            return Err(format!(
                "There is already a {} called “{name}” here",
                kind.what()
            ));
        }
        // Over an item that is there, in the case its name has.
        let rel = existing.as_ref().map_or(wanted, |item| item.path.clone());
        self.check_not_a_link(&rel)?;
        let abs = self.abs(&rel);
        match &body {
            Body::Text(text) => write_atomic(&abs, text.as_bytes()),
            Body::Staged(staged) => move_file(staged, &abs),
        }
        .map_err(|e| format!("“{name}” could not be saved: {e}"))?;

        let sample_rel = join(&folder, &sample_file_name(&name));
        let sample_abs = self.abs(&sample_rel);
        let sample_result = match sample {
            SampleSource::Keep => Ok(()),
            _ if passes_through_a_link(&self.root, &sample_rel) => {
                Err("its sample's place is a link".to_owned())
            }
            SampleSource::Text(text) => {
                write_atomic(&sample_abs, text.as_bytes()).map_err(|e| e.to_string())
            }
            SampleSource::Staged(staged) => {
                move_file(&staged, &sample_abs).map_err(|e| e.to_string())
            }
        };

        let id = match &existing {
            Some(item) => item.id.clone(),
            None => self.new_id(),
        };
        if let Ok(meta) = fs::metadata(&abs) {
            let sha = match &body {
                Body::Text(text) => Some(sha256_hex(text.as_bytes())),
                Body::Staged(_) => sha256_file(&abs).ok().map(|(sha, _)| sha),
            };
            if let Some(sha) = sha {
                let entry = Entry::new(id, sha, meta.len(), modified_ms(&meta));
                self.index.items.insert(rel.clone(), entry);
            }
        }
        self.commit();
        if let Err(e) = sample_result {
            return Err(format!(
                "“{name}” was saved, but its sample could not be: {e}"
            ));
        }
        self.item(&rel)
            .cloned()
            .ok_or_else(|| format!("“{name}” was saved, but is not in the workspace's folder"))
    }

    /// Make the folder `name` in the folder `parent_folder`. Gives its relative path.
    pub fn create_folder(&mut self, parent_folder: &str, name: &str) -> Result<String, String> {
        self.rescan();
        let name = check_name(name)?;
        let parent_folder = self.checked_folder(parent_folder)?;
        if !parent_folder.is_empty() && self.tree.find_folder(&parent_folder).is_none() {
            return Err(format!("There is no folder {parent_folder:?} any more"));
        }
        if self.has_folder(&parent_folder, &name) {
            return Err(format!("There is already a folder called “{name}” here"));
        }
        let rel = join(&parent_folder, &name);
        fs::create_dir_all(self.abs(&rel))
            .map_err(|e| format!("The folder could not be made: {e}"))?;
        self.rescan();
        Ok(rel)
    }

    /// Give the item or folder at `rel` another name. Gives its new relative path.
    pub fn rename(&mut self, rel: &str, new_name: &str) -> Result<String, String> {
        self.rescan();
        self.check_not_a_link(rel)?;
        if let Some(item) = self.tree.find_item(rel).cloned() {
            let name = check_item_name(new_name, item.kind)?;
            let target_file = format!("{name}{}", item.ext);
            let target = join(item.folder(), &target_file);
            if target == item.path {
                return Ok(target);
            }
            // A file of that name, whatever the case, is there already — but the
            // item's own, for a change of case.
            let (from, to) = (self.abs(&item.path), self.abs(&target));
            let taken = self
                .tree
                .find_folder(item.folder())
                .is_some_and(|f| f.has_file(&target_file, &item.path));
            if taken || (to.exists() && !same_file(&from, &to)) {
                return Err(format!(
                    "There is already a {} called “{name}” here",
                    item.kind.what()
                ));
            }
            let sample_from = self.abs(&join(item.folder(), &sample_file_name(&item.name)));
            let sample_to = self.abs(&join(item.folder(), &sample_file_name(&name)));
            // The sample goes with the item, unless another item of the old name
            // still needs it.
            let moves_sample = item.sample.is_some() && !self.shares_sample(&item);
            if moves_sample && sample_to.exists() && !same_file(&sample_from, &sample_to) {
                return Err(format!("There is already a sample called “{name}” here"));
            }
            fs::rename(&from, &to).map_err(|e| format!("It could not be renamed: {e}"))?;
            if moves_sample {
                let _ = fs::rename(&sample_from, &sample_to);
            }
            self.index.remap(&item.path, &target);
            self.commit();
            return Ok(target);
        }
        if let Some(folder) = self
            .tree
            .find_folder(rel)
            .filter(|f| !f.path.is_empty())
            .cloned()
        {
            let name = check_name(new_name)?;
            let target = join(parent(&folder.path), &name);
            if target == folder.path {
                return Ok(target);
            }
            let (from, to) = (self.abs(&folder.path), self.abs(&target));
            let taken = self
                .tree
                .find_folder(parent(&folder.path))
                .is_some_and(|f| {
                    f.folders
                        .iter()
                        .any(|d| d.path != folder.path && d.name.eq_ignore_ascii_case(&name))
                });
            if taken || (to.exists() && !same_file(&from, &to)) {
                return Err(format!("There is already a folder called “{name}” here"));
            }
            fs::rename(&from, &to).map_err(|e| format!("It could not be renamed: {e}"))?;
            self.index.remap(&folder.path, &target);
            self.commit();
            return Ok(target);
        }
        Err("It is not in the workspace any more".to_owned())
    }

    /// Move the item or folder at `rel` into the folder `to`. Gives its new relative path.
    pub fn move_to(&mut self, rel: &str, to: &str) -> Result<String, String> {
        self.rescan();
        let to = self.checked_folder(to)?;
        if !to.is_empty() && self.tree.find_folder(&to).is_none() {
            return Err(format!("There is no folder {to:?} any more"));
        }
        self.check_not_a_link(rel)?;
        if let Some(item) = self.tree.find_item(rel).cloned() {
            if to == item.folder() {
                return Ok(item.path);
            }
            let target = join(&to, item.file_name());
            if self
                .tree
                .find_folder(&to)
                .is_some_and(|f| f.has_file(item.file_name(), ""))
                || self.abs(&target).exists()
            {
                return Err(format!(
                    "There is already a {} called “{}” in that folder",
                    item.kind.what(),
                    item.name
                ));
            }
            let sample_from = self.abs(&join(item.folder(), &sample_file_name(&item.name)));
            let sample_to = self.abs(&join(&to, &sample_file_name(&item.name)));
            if item.sample.is_some() && sample_to.exists() {
                return Err(format!(
                    "There is already a sample called “{}” in that folder",
                    item.name
                ));
            }
            let shared = self.shares_sample(&item);
            move_file(&self.abs(&item.path), &self.abs(&target))
                .map_err(|e| format!("It could not be moved: {e}"))?;
            if item.sample.is_some() {
                let _ = if shared {
                    copy_atomic(&sample_from, &sample_to)
                } else {
                    move_file(&sample_from, &sample_to)
                };
            }
            self.index.remap(&item.path, &target);
            self.commit();
            return Ok(target);
        }
        if let Some(folder) = self
            .tree
            .find_folder(rel)
            .filter(|f| !f.path.is_empty())
            .cloned()
        {
            if to == folder.path || to.starts_with(&format!("{}/", folder.path)) {
                return Err("A folder can't be moved into itself".to_owned());
            }
            if to == parent(&folder.path) {
                return Ok(folder.path);
            }
            let target = join(&to, &folder.name);
            if self.has_folder(&to, &folder.name) || self.abs(&target).exists() {
                return Err(format!(
                    "There is already a folder called “{}” in that folder",
                    folder.name
                ));
            }
            fs::rename(self.abs(&folder.path), self.abs(&target))
                .map_err(|e| format!("It could not be moved: {e}"))?;
            self.index.remap(&folder.path, &target);
            self.commit();
            return Ok(target);
        }
        Err("It is not in the workspace any more".to_owned())
    }

    /// Make a copy of the item at `rel`, with its sample, in the same folder, under
    /// a name with a number after it. It has an id of its own. Gives the copy.
    pub fn duplicate(&mut self, rel: &str) -> Result<Item, String> {
        self.rescan();
        let Some(item) = self.tree.find_item(rel).cloned() else {
            return Err("It is not in the workspace any more".to_owned());
        };
        self.check_not_a_link(rel)?;
        let folder = item.folder().to_owned();
        let name = unique_name(
            |candidate| {
                self.has_name(&folder, candidate)
                    || (item.kind.takes_sample()
                        && self
                            .abs(&join(&folder, &sample_file_name(candidate)))
                            .exists())
            },
            &item.name,
        );
        let name = check_item_name(&name, item.kind)?;
        let target = join(&folder, &format!("{name}{}", item.ext));
        copy_atomic(&self.abs(&item.path), &self.abs(&target))
            .map_err(|e| format!("It could not be copied: {e}"))?;
        if item.sample.is_some() {
            let _ = copy_atomic(
                &self.abs(&join(&folder, &sample_file_name(&item.name))),
                &self.abs(&join(&folder, &sample_file_name(&name))),
            );
        }
        if let Ok(meta) = fs::metadata(self.abs(&target)) {
            let id = self.new_id();
            let entry = Entry::new(id, item.sha256.clone(), meta.len(), modified_ms(&meta));
            self.index.items.insert(target.clone(), entry);
        }
        self.commit();
        self.item(&target)
            .cloned()
            .ok_or_else(|| "The copy is not in the workspace's folder".to_owned())
    }

    /// Delete the item at `rel` and its sample (unless another item has the name and
    /// so needs it), or the folder at `rel` and what the workspace put in it.
    pub fn delete(&mut self, rel: &str) -> Result<Deleted, String> {
        self.rescan();
        self.check_not_a_link(rel)?;
        if let Some(item) = self.tree.find_item(rel).cloned() {
            fs::remove_file(self.abs(&item.path))
                .map_err(|e| format!("It could not be deleted: {e}"))?;
            if item.sample.is_some() && !self.shares_sample(&item) {
                let _ =
                    fs::remove_file(self.abs(&join(item.folder(), &sample_file_name(&item.name))));
            }
            self.index.forget(&item.path);
            self.commit();
            return Ok(Deleted { items: 1, left: 0 });
        }
        if self
            .tree
            .find_folder(rel)
            .is_some_and(|f| !f.path.is_empty())
        {
            let mut deleted = Deleted::default();
            remove_known(&self.abs(rel), &mut deleted);
            self.index.forget(rel);
            self.commit();
            return Ok(deleted);
        }
        Err("It is not in the workspace any more".to_owned())
    }

    /// The text of the item at `rel`, up to `limit` bytes.
    pub fn read_text(&self, rel: &str, limit: u64) -> Result<String, String> {
        read_text(&self.abs(rel), limit)
    }

    /// The text of the sample of `item`, if it has one.
    #[cfg(test)]
    pub fn read_sample(&self, item: &Item, limit: u64) -> Result<String, String> {
        match &item.sample {
            Some(sample) => read_text(&self.abs(&sample.path), limit),
            None => Err("It has no sample".to_owned()),
        }
    }

    /// A path for a file that is to be moved into the workspace when it is written
    /// (by the worker, which writes a document): in the app's own folder, so that
    /// moving it is a rename. The folder is made.
    pub fn staging_path(&self) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static MADE: AtomicU64 = AtomicU64::new(0);
        let folder = self.root.join(index::FOLDER);
        let _ = fs::create_dir_all(&folder);
        folder.join(format!(
            "staged-{}-{}.json",
            std::process::id(),
            MADE.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

/// The text in the file at `path`, as long as it is not more than `limit` bytes
/// and is text, without a byte-order mark.
pub fn read_text(path: &Path, limit: u64) -> Result<String, String> {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let meta = fs::metadata(path).map_err(|e| format!("{name} could not be read: {e}"))?;
    if meta.len() > limit {
        return Err(format!("{name} is too big to put in a box"));
    }
    let text = fs::read_to_string(path).map_err(|e| format!("{name} could not be read: {e}"))?;
    Ok(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_owned(),
        None => text,
    })
}

/// Delete from the folder `dir`, and the folders in it, the files that the workspace
/// knows (items and samples), and then the folders that are empty. What is not the
/// workspace's is counted and left, and so are the folders it is in.
fn remove_known(dir: &Path, deleted: &mut Deleted) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if meta.file_type().is_symlink() || name.starts_with('.') {
            deleted.left += 1;
        } else if meta.is_dir() {
            // (the folder is taken away when it is empty; what keeps it is counted)
            remove_known(&path, deleted);
        } else if parse_file_name(&name).is_some() {
            if fs::remove_file(&path).is_ok() {
                deleted.items += 1;
            } else {
                deleted.left += 1;
            }
        } else if name.ends_with(SAMPLE_SUFFIX) {
            if fs::remove_file(&path).is_err() {
                deleted.left += 1;
            }
        } else {
            deleted.left += 1;
        }
    }
    let _ = fs::remove_dir(dir);
}
