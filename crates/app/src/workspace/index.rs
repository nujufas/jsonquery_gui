//! The index: what the workspace knows about each of its items that is not in the
//! item's own file — the id it was given when it was first saved, the SHA-256 of
//! what it holds, how big it was and when it was last changed (so that a file that
//! has not changed is not read again), and the same for its sample.
//!
//! It is one file, `.jsonquery/index.json` in the workspace's folder, so that the
//! items themselves stay what other tools expect of them: a `.jq` file is a jq
//! program, a `.schema.json` file a JSON Schema. It is written whenever it changes
//! and is made right again, from the folder, whenever the workspace is read:
//!
//! * a file that is new gets an id;
//! * a file that was changed (by another program) keeps its id, and has a new
//!   SHA-256;
//! * a file that was moved or renamed (by another program) is found again by its
//!   SHA-256 and keeps its id;
//! * an entry whose file is gone is dropped.
//!
//! ```json
//! {
//!   "format": "jsonquery-workspace",
//!   "version": 1,
//!   "items": {
//!     "Logs/Errors/slow requests.jq": {
//!       "id": "6f1c0e52-9a4b-4c1e-8d52-3b7a1c9e0f21",
//!       "sha256": "9f2c…",
//!       "bytes": 118,
//!       "modified": 1792528206123,
//!       "sample": { "sha256": "ab12…", "bytes": 2355, "modified": 1792528206000 }
//!     }
//!   }
//! }
//! ```
//!
//! A key that a later version of the app has put in the file, or in an entry, is
//! kept as it is when this version writes it again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Number, Value};

use super::files::{is_relative_path, write_atomic};
use super::ids::{is_id, is_sha256};

/// The folder in the workspace that is the app's own, and so not listed.
pub const FOLDER: &str = ".jsonquery";

/// The file in that folder.
pub const FILE: &str = "index.json";

/// What `format` says in the file.
const FORMAT: &str = "jsonquery-workspace";

/// The version of the file that this app reads and writes.
pub const VERSION: u64 = 1;

/// The sample of an item, as far as the index knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleEntry {
    pub sha256: String,
    pub bytes: u64,
    pub modified: u64,
}

/// An item, as far as the index knows it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Entry {
    /// Empty for an entry that had none, which is given one.
    pub id: String,
    /// Empty for an entry that had none, which is worked out again.
    pub sha256: String,
    pub bytes: u64,
    pub modified: u64,
    pub sample: Option<SampleEntry>,
    /// What a later version put in the entry, kept as it was.
    extra: Map<String, Value>,
}

impl Entry {
    /// An entry for a file that has just been written or read.
    pub fn new(id: String, sha256: String, bytes: u64, modified: u64) -> Self {
        Self {
            id,
            sha256,
            bytes,
            modified,
            sample: None,
            extra: Map::new(),
        }
    }

    /// This entry for another file (a copy, a move): the same keys that are not
    /// known to this version, and what is given.
    pub fn renewed(&self, id: String, sha256: String, bytes: u64, modified: u64) -> Self {
        Self {
            extra: self.extra.clone(),
            ..Self::new(id, sha256, bytes, modified)
        }
    }

    fn to_json(&self) -> Value {
        let mut entry = Map::new();
        entry.insert("id".to_owned(), Value::from(self.id.as_str()));
        entry.insert("sha256".to_owned(), Value::from(self.sha256.as_str()));
        entry.insert("bytes".to_owned(), Value::from(self.bytes));
        entry.insert("modified".to_owned(), Value::from(self.modified));
        if let Some(sample) = &self.sample {
            let mut object = Map::new();
            object.insert("sha256".to_owned(), Value::from(sample.sha256.as_str()));
            object.insert("bytes".to_owned(), Value::from(sample.bytes));
            object.insert("modified".to_owned(), Value::from(sample.modified));
            entry.insert("sample".to_owned(), Value::Object(object));
        }
        for (key, value) in &self.extra {
            entry.entry(key.clone()).or_insert_with(|| value.clone());
        }
        Value::Object(entry)
    }

    /// The entry that `value` stands for, with what was wrong with it (it is not
    /// thrown away for that: what is usable is kept, and the rest is made again).
    fn from_json(path: &str, value: &Value, notes: &mut Vec<String>) -> Option<Self> {
        let Some(object) = value.as_object() else {
            notes.push(format!(
                "{path}: its entry is not an object, so it is left out"
            ));
            return None;
        };
        let mut entry = Entry::default();
        if let Some(id) = object.get("id").and_then(Value::as_str) {
            if is_id(id) {
                entry.id = id.to_owned();
            }
        }
        if let Some(sha) = object.get("sha256").and_then(Value::as_str) {
            if is_sha256(sha) {
                entry.sha256 = sha.to_owned();
            }
        }
        entry.bytes = object.get("bytes").and_then(Value::as_u64).unwrap_or(0);
        entry.modified = object.get("modified").and_then(Value::as_u64).unwrap_or(0);
        if let Some(sample) = object.get("sample").and_then(Value::as_object) {
            let sha = sample.get("sha256").and_then(Value::as_str).unwrap_or("");
            if is_sha256(sha) {
                entry.sample = Some(SampleEntry {
                    sha256: sha.to_owned(),
                    bytes: sample.get("bytes").and_then(Value::as_u64).unwrap_or(0),
                    modified: sample.get("modified").and_then(Value::as_u64).unwrap_or(0),
                });
            }
        }
        for (key, value) in object {
            if !matches!(
                key.as_str(),
                "id" | "sha256" | "bytes" | "modified" | "sample"
            ) {
                entry.extra.insert(key.clone(), value.clone());
            }
        }
        Some(entry)
    }
}

/// How the index in a folder was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// There is none yet.
    Fresh,
    /// It was read.
    Read,
    /// There is a file, and it is not an index that can be used: it is set aside
    /// (as `index.json.bad`) before a new one is written.
    Unusable,
    /// It was written by a later version of the app: it is read as far as it is
    /// understood, and never written over.
    Newer,
}

impl State {
    /// Whether the index may be written.
    pub fn writable(self) -> bool {
        self != State::Newer
    }
}

/// What the workspace knows of its items: the entry of each, by the relative path
/// of its file.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Index {
    pub items: BTreeMap<String, Entry>,
    /// What a later version put in the file, kept as it was.
    extra: Map<String, Value>,
}

/// Where the index of the workspace at `root` is.
pub fn path(root: &Path) -> PathBuf {
    root.join(FOLDER).join(FILE)
}

impl Index {
    /// The index in the workspace at `root`, how it was found, and what was
    /// wrong with it.
    pub fn read(root: &Path) -> (Index, State, Vec<String>) {
        let file = path(root);
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Index::default(), State::Fresh, Vec::new());
            }
            Err(e) => {
                return (
                    Index::default(),
                    State::Unusable,
                    vec![format!(
                        "{} could not be read ({e}): the ids of the items are made again",
                        file.display()
                    )],
                );
            }
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => Index::from_json(&value, &file),
            Err(e) => (
                Index::default(),
                State::Unusable,
                vec![format!(
                    "{} is not JSON ({e}): the ids of the items are made again",
                    file.display()
                )],
            ),
        }
    }

    fn from_json(value: &Value, file: &Path) -> (Index, State, Vec<String>) {
        let mut notes = Vec::new();
        let Some(object) = value.as_object() else {
            return (
                Index::default(),
                State::Unusable,
                vec![format!(
                    "{} is not an object: the ids of the items are made again",
                    file.display()
                )],
            );
        };
        match object.get("version").and_then(Value::as_u64) {
            Some(version) if version > VERSION => {
                return (
                    Index::default(),
                    State::Newer,
                    vec![format!(
                        "{} is from a newer version of the app, so the items' ids are not kept \
                         until that one is used",
                        file.display()
                    )],
                );
            }
            Some(_) => {}
            None => notes.push("the index has no version: it is taken to be version 1".to_owned()),
        }
        let mut index = Index::default();
        for (key, value) in object {
            if !matches!(key.as_str(), "format" | "version" | "items") {
                index.extra.insert(key.clone(), value.clone());
            }
        }
        match object.get("items") {
            None => {}
            Some(Value::Object(items)) => {
                for (path, value) in items {
                    if !is_relative_path(path) || path.is_empty() {
                        notes.push(format!(
                            "{path:?} is not a place in the workspace, so it is left out"
                        ));
                        continue;
                    }
                    if let Some(entry) = Entry::from_json(path, value, &mut notes) {
                        index.items.insert(path.clone(), entry);
                    }
                }
            }
            Some(_) => {
                return (
                    Index::default(),
                    State::Unusable,
                    vec![format!(
                        "the items in {} are not an object: their ids are made again",
                        file.display()
                    )],
                );
            }
        }
        (index, State::Read, notes)
    }

    /// An index of `items`, which keeps what this one has of its own that is not
    /// known to this version.
    pub fn rebuilt(&self, items: BTreeMap<String, Entry>) -> Index {
        Index {
            items,
            extra: self.extra.clone(),
        }
    }

    /// The file's contents.
    pub fn to_json(&self) -> Value {
        let mut root = Map::new();
        root.insert("format".to_owned(), Value::from(FORMAT));
        root.insert("version".to_owned(), Value::Number(Number::from(VERSION)));
        for (key, value) in &self.extra {
            root.entry(key.clone()).or_insert_with(|| value.clone());
        }
        let items: Map<String, Value> = self
            .items
            .iter()
            .map(|(path, entry)| (path.clone(), entry.to_json()))
            .collect();
        root.insert("items".to_owned(), Value::Object(items));
        Value::Object(root)
    }

    /// Write the index to the workspace at `root`, whole or not at all.
    pub fn write(&self, root: &Path) -> std::io::Result<()> {
        let mut text =
            serde_json::to_string_pretty(&self.to_json()).map_err(std::io::Error::other)?;
        text.push('\n');
        write_atomic(&path(root), text.as_bytes())
    }

    /// The entry of `path`, and every entry below it if it is a folder, moved to
    /// `to`: for the item or folder that was renamed or moved.
    pub fn remap(&mut self, from: &str, to: &str) {
        let below = format!("{from}/");
        let moved: Vec<String> = self
            .items
            .keys()
            .filter(|key| key.as_str() == from || key.starts_with(&below))
            .cloned()
            .collect();
        for key in moved {
            if let Some(entry) = self.items.remove(&key) {
                let new_key = format!("{to}{}", &key[from.len()..]);
                self.items.insert(new_key, entry);
            }
        }
    }

    /// The entry of `path`, and every entry below it if it is a folder, taken out.
    pub fn forget(&mut self, path: &str) {
        let below = format!("{path}/");
        self.items
            .retain(|key, _| key != path && !key.starts_with(&below));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch_dir::ScratchDir;
    use serde_json::json;

    fn sha(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn entry(id: &str, n: u8) -> Entry {
        Entry::new(
            id.to_owned(),
            sha(n),
            10 + u64::from(n),
            1_000 + u64::from(n),
        )
    }

    #[test]
    fn a_workspace_that_has_no_index_has_a_fresh_one() {
        let dir = ScratchDir::new("index", "fresh");
        let (index, state, notes) = Index::read(&dir);
        assert_eq!(state, State::Fresh);
        assert!(index.items.is_empty() && notes.is_empty());
        assert!(state.writable());
    }

    #[test]
    fn what_is_written_is_what_is_read_and_it_is_the_file_that_the_doc_shows() {
        let dir = ScratchDir::new("index", "round-trip");
        let mut index = Index::default();
        let mut slow = entry("6f1c0e52-9a4b-4c1e-8d52-3b7a1c9e0f21", 1);
        slow.sample = Some(SampleEntry {
            sha256: sha(2),
            bytes: 2355,
            modified: 3_000,
        });
        index
            .items
            .insert("Logs/Errors/slow requests.jq".to_owned(), slow);
        index
            .items
            .insert("scratch.jq".to_owned(), entry("id-2", 3));
        index.write(&dir).unwrap();

        let text = std::fs::read_to_string(path(&dir)).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["format"], "jsonquery-workspace");
        assert_eq!(value["version"], 1);
        assert_eq!(
            value["items"]["Logs/Errors/slow requests.jq"]["id"],
            "6f1c0e52-9a4b-4c1e-8d52-3b7a1c9e0f21"
        );
        assert_eq!(
            value["items"]["Logs/Errors/slow requests.jq"]["sample"]["bytes"],
            2355
        );
        assert!(value["items"]["scratch.jq"].get("sample").is_none());
        assert!(text.ends_with('\n'));

        let (read, state, notes) = Index::read(&dir);
        assert_eq!(state, State::Read);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(read, index);
    }

    #[test]
    fn the_items_are_written_in_the_order_of_their_paths() {
        let mut index = Index::default();
        for path in ["b.jq", "a/z.jq", "a.jq", "a/b.jq"] {
            index.items.insert(path.to_owned(), entry(path, 1));
        }
        let value = index.to_json();
        let keys: Vec<&String> = value["items"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["a.jq", "a/b.jq", "a/z.jq", "b.jq"]);
    }

    #[test]
    fn what_a_later_version_wrote_is_kept_when_this_one_writes_again() {
        let dir = ScratchDir::new("index", "extra");
        let file = path(&dir);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            json!({
                "format": "jsonquery-workspace",
                "version": 1,
                "synced": "2027-01-01",
                "items": {
                    "a.jq": {
                        "id": "id-1", "sha256": sha(1), "bytes": 5, "modified": 7,
                        "tags": ["slow"], "sample": {"sha256": sha(2), "bytes": 1, "modified": 2, "note": "n"}
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
        let (index, state, notes) = Index::read(&dir);
        assert_eq!(state, State::Read);
        assert!(notes.is_empty(), "{notes:?}");
        index.write(&dir).unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(value["synced"], "2027-01-01");
        assert_eq!(value["items"]["a.jq"]["tags"], json!(["slow"]));
        assert_eq!(value["items"]["a.jq"]["id"], "id-1");
    }

    #[test]
    fn an_entry_with_something_wrong_in_it_is_kept_as_far_as_it_is_usable() {
        let dir = ScratchDir::new("index", "lenient");
        let file = path(&dir);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            json!({
                "version": 1,
                "items": {
                    "no id.jq": {"sha256": sha(1), "bytes": 3, "modified": 4},
                    "bad hash.jq": {"id": "id-2", "sha256": "xyz"},
                    "bad id.jq": {"id": "has a space", "sha256": sha(3)},
                    "not an object.jq": 5,
                    "../escape.jq": {"id": "id-3", "sha256": sha(4)},
                    "a//b.jq": {"id": "id-4", "sha256": sha(4)},
                    "": {"id": "id-5"}
                }
            })
            .to_string(),
        )
        .unwrap();
        let (index, state, notes) = Index::read(&dir);
        assert_eq!(state, State::Read);
        assert_eq!(index.items["no id.jq"].id, "");
        assert_eq!(index.items["no id.jq"].sha256, sha(1));
        assert_eq!(index.items["bad hash.jq"].id, "id-2");
        assert_eq!(index.items["bad hash.jq"].sha256, "");
        assert_eq!(index.items["bad id.jq"].id, "");
        assert!(!index.items.contains_key("not an object.jq"));
        assert!(!index.items.contains_key("../escape.jq"));
        assert!(!index.items.contains_key("a//b.jq"));
        assert!(!index.items.contains_key(""));
        assert!(notes.len() >= 4, "each thing left out was said: {notes:?}");
    }

    #[test]
    fn a_file_that_is_no_index_is_unusable_and_says_why() {
        for (name, text) in [
            ("not json", "{ nope"),
            ("an array", "[1, 2]"),
            ("items that are a list", r#"{"version": 1, "items": [1]}"#),
        ] {
            let dir = ScratchDir::new("index", "unusable");
            let file = path(&dir);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, text).unwrap();
            let (index, state, notes) = Index::read(&dir);
            assert_eq!(state, State::Unusable, "{name}");
            assert!(index.items.is_empty());
            assert_eq!(notes.len(), 1, "{name}: {notes:?}");
            assert!(state.writable());
        }
    }

    #[test]
    fn an_index_of_a_later_version_is_read_as_nothing_and_never_written_over() {
        let dir = ScratchDir::new("index", "newer");
        let file = path(&dir);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{"version": 2, "items": {"a.jq": {"id": "x"}}}"#).unwrap();
        let (index, state, notes) = Index::read(&dir);
        assert_eq!(state, State::Newer);
        assert!(!state.writable());
        assert!(index.items.is_empty());
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn a_file_with_no_version_is_taken_as_the_first() {
        let dir = ScratchDir::new("index", "unversioned");
        let file = path(&dir);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{"items": {"a.jq": {"id": "id-1"}}}"#).unwrap();
        let (index, state, notes) = Index::read(&dir);
        assert_eq!(state, State::Read);
        assert_eq!(index.items["a.jq"].id, "id-1");
        assert_eq!(notes.len(), 1, "said that it had none");
    }

    #[test]
    fn an_entry_and_what_is_below_a_folder_move_with_it() {
        let mut index = Index::default();
        for path in ["a/x.jq", "a/b/y.jq", "ab/z.jq", "a.jq", "c.jq"] {
            index.items.insert(path.to_owned(), entry(path, 1));
        }
        index.remap("a", "renamed");
        let keys: Vec<&str> = index.items.keys().map(String::as_str).collect();
        // `ab/z.jq` and `a.jq` are not below `a`, and are where they were.
        assert_eq!(
            keys,
            ["a.jq", "ab/z.jq", "c.jq", "renamed/b/y.jq", "renamed/x.jq"]
        );
        assert_eq!(
            index.items["renamed/x.jq"].id, "a/x.jq",
            "the entry moved, not a copy"
        );
        index.remap("c.jq", "d/c.jq");
        assert!(index.items.contains_key("d/c.jq") && !index.items.contains_key("c.jq"));
    }

    #[test]
    fn forgetting_a_folder_forgets_what_is_in_it_and_only_that() {
        let mut index = Index::default();
        for path in ["a/x.jq", "a/b/y.jq", "ab/z.jq", "a.jq"] {
            index.items.insert(path.to_owned(), entry(path, 1));
        }
        index.forget("a");
        let keys: Vec<&str> = index.items.keys().map(String::as_str).collect();
        assert_eq!(keys, ["a.jq", "ab/z.jq"]);
        index.forget("a.jq");
        assert_eq!(index.items.len(), 1);
    }

    #[test]
    fn an_entry_made_again_for_a_copy_keeps_only_what_this_version_does_not_know() {
        let mut old = entry("id-1", 1);
        old.extra.insert("tags".to_owned(), json!(["x"]));
        old.sample = Some(SampleEntry {
            sha256: sha(9),
            bytes: 1,
            modified: 1,
        });
        let copy = old.renewed("id-2".to_owned(), sha(5), 7, 8);
        assert_eq!(
            (copy.id.as_str(), copy.bytes, copy.modified),
            ("id-2", 7, 8)
        );
        assert_eq!(copy.extra["tags"], json!(["x"]));
        assert!(copy.sample.is_none());
    }
}
