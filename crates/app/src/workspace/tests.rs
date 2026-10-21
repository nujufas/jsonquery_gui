//! What the workspace does with a folder: how it reads one, how it keeps the id and
//! the SHA-256 of every item through what happens to the files, what it does with
//! them when it is asked to, and how a workspace is exported and imported.
//! (The names, the index and the small file operations have tests of their own,
//! beside them.)

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use jsonquery_query::Kind as Engine;

use super::ids::{sha256_file, SequentialIds};
use super::index::{Index, State};
use super::store::scan_up_to;
use super::*;
use crate::scratch_dir::ScratchDir;

const JQ: ItemKind = ItemKind::Query(Engine::Jq);

fn put(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn text(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

fn sha(text: &str) -> String {
    sha256_hex(text.as_bytes())
}

/// A workspace whose ids count up, so that a test can tell them apart.
fn open(dir: &Path) -> Workspace {
    Workspace::with_ids(dir.to_path_buf(), Box::new(SequentialIds::default()))
}

/// The same, with ids that count up from `after`, for a workspace that something is
/// imported into (whose items are not to have the ids of the ones that come in).
fn open_after(dir: &Path, after: u64) -> Workspace {
    Workspace::with_ids(dir.to_path_buf(), Box::new(SequentialIds(after)))
}

fn paths(workspace: &Workspace) -> Vec<String> {
    workspace
        .tree()
        .all_items()
        .iter()
        .map(|i| i.path.clone())
        .collect()
}

fn save(workspace: &mut Workspace, folder: &str, name: &str, body: &str) -> Item {
    workspace
        .save(NewItem {
            folder,
            name,
            kind: JQ,
            body,
            sample: SampleSource::Keep,
            replace: false,
        })
        .unwrap()
}

fn save_with_sample(
    workspace: &mut Workspace,
    folder: &str,
    name: &str,
    body: &str,
    sample: &str,
) -> Item {
    workspace
        .save(NewItem {
            folder,
            name,
            kind: JQ,
            body,
            sample: SampleSource::Text(sample.to_owned()),
            replace: false,
        })
        .unwrap()
}

fn id_of(workspace: &Workspace, path: &str) -> String {
    workspace
        .item(path)
        .unwrap_or_else(|| panic!("{path} is not in the workspace"))
        .id
        .clone()
}

fn set_mtime(path: &Path, time: SystemTime) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(time).unwrap();
}

// ---- reading a folder ---------------------------------------------------------

#[test]
fn a_folder_that_is_not_there_is_an_empty_workspace_and_stays_not_there() {
    let dir = ScratchDir::new("ws", "missing");
    let root = dir.join("not yet");
    let workspace = Workspace::open(root.clone());
    assert!(workspace.tree().all_items().is_empty());
    assert!(workspace.tree().folders.is_empty());
    assert!(!workspace.exists());
    assert!(workspace.notes().is_empty());
    assert!(!root.exists(), "reading a workspace makes nothing");
}

#[test]
fn an_empty_folder_is_an_empty_workspace_with_nothing_added_to_it() {
    let dir = ScratchDir::new("ws", "empty");
    let workspace = open(&dir);
    assert!(workspace.exists());
    assert_eq!(workspace.tree().item_count(), 0);
    assert!(
        !dir.join(".jsonquery").exists(),
        "no folder of the app's own yet"
    );
}

#[test]
fn changing_the_folders_of_a_workspace_that_has_no_items_adds_no_folder_of_the_apps_own() {
    let dir = ScratchDir::new("ws", "bare-folders");
    fs::create_dir_all(dir.join("Reports")).unwrap();
    fs::create_dir_all(dir.join("Old")).unwrap();
    let mut workspace = open(&dir);
    assert_eq!(workspace.rename("Reports", "Weekly").unwrap(), "Weekly");
    workspace.delete("Old").unwrap();
    assert_eq!(workspace.tree().folder_paths(), ["Weekly"]);
    assert!(
        !dir.join(".jsonquery").exists(),
        "there is nothing to give an id to yet"
    );
}

#[test]
fn a_folder_with_more_entries_than_the_limit_is_read_up_to_it_and_says_so() {
    let dir = ScratchDir::new("ws", "entry-limit");
    for n in 0..8 {
        put(&dir, &format!("q{n}.jq"), ".");
    }
    let mut ids = SequentialIds::default();
    let all = scan_up_to(&dir, &Index::default(), &mut ids, 8);
    assert_eq!(all.tree.item_count(), 8, "as many as the limit is not more");
    assert!(all.notes.is_empty(), "{:?}", all.notes);
    let some = scan_up_to(&dir, &Index::default(), &mut ids, 5);
    assert_eq!(some.tree.item_count(), 5);
    assert!(
        some.notes
            .iter()
            .any(|n| n.contains("more than 5 files and folders")),
        "{:?}",
        some.notes
    );
}

#[cfg(unix)]
#[test]
fn links_are_not_counted_among_the_entries_a_folder_may_have() {
    let dir = ScratchDir::new("ws", "entry-limit-links");
    let outside = ScratchDir::new("ws", "entry-limit-outside");
    for n in 0..3 {
        put(&dir, &format!("q{n}.jq"), ".");
        std::os::unix::fs::symlink(&*outside, dir.join(format!("link{n}"))).unwrap();
    }
    let scanned = scan_up_to(&dir, &Index::default(), &mut SequentialIds::default(), 3);
    assert_eq!(scanned.tree.item_count(), 3);
    assert!(
        !scanned.notes.iter().any(|n| n.contains("more than")),
        "{:?}",
        scanned.notes
    );
    assert!(
        scanned
            .notes
            .iter()
            .any(|n| n.starts_with("3 links in the workspace are not followed")),
        "{:?}",
        scanned.notes
    );
}

#[test]
fn what_is_in_a_folder_is_listed_by_what_its_name_says_it_is() {
    let dir = ScratchDir::new("ws", "kinds");
    put(&dir, "Logs/Errors/slow requests.jq", ".a");
    put(&dir, "Logs/failed logins.jmespath", "x");
    put(&dir, "Billing/customers.jsonpath", "$.a");
    put(&dir, "Billing/ptr.jsonptr", "/a");
    put(&dir, "Services/orders-api.schema.json", "{}");
    put(&dir, "Services/bump replicas.patch.json", "[]");
    put(&dir, "invoices.json", "[]");
    put(&dir, "events.ndjson", "{}\n");
    // Not the workspace's.
    put(&dir, ".hidden.jq", "x");
    put(&dir, ".git/config.jq", "x");
    put(&dir, "notes.txt", "x");
    put(&dir, "README.md", "x");
    put(&dir, "x.jqx", "x");
    put(&dir, "orphan.sample.json", "[]");
    let workspace = open(&dir);
    let found: Vec<(String, ItemKind)> = workspace
        .tree()
        .all_items()
        .iter()
        .map(|i| (i.path.clone(), i.kind))
        .collect();
    use Engine::*;
    assert_eq!(
        found,
        vec![
            ("events.ndjson".to_owned(), ItemKind::Document),
            ("invoices.json".to_owned(), ItemKind::Document),
            (
                "Billing/customers.jsonpath".to_owned(),
                ItemKind::Query(JsonPath)
            ),
            (
                "Billing/ptr.jsonptr".to_owned(),
                ItemKind::Query(JsonPointer)
            ),
            (
                "Logs/failed logins.jmespath".to_owned(),
                ItemKind::Query(JmesPath)
            ),
            (
                "Logs/Errors/slow requests.jq".to_owned(),
                ItemKind::Query(Jq)
            ),
            (
                "Services/bump replicas.patch.json".to_owned(),
                ItemKind::Patch
            ),
            (
                "Services/orders-api.schema.json".to_owned(),
                ItemKind::Schema
            ),
        ]
    );
    assert_eq!(workspace.tree().folder_count(), 4);
    assert_eq!(
        workspace.tree().folder_paths(),
        ["Billing", "Logs", "Logs/Errors", "Services"]
    );
}

#[test]
fn folders_come_first_and_everything_is_in_the_order_of_its_name_whatever_the_case() {
    let dir = ScratchDir::new("ws", "order");
    for rel in [
        "B.jq",
        "a.jq",
        "C/x.jq",
        "b folder/y.jq",
        "A/z.jq",
        "a.schema.json",
    ] {
        put(&dir, rel, "x");
    }
    let workspace = open(&dir);
    let tree = workspace.tree();
    let folders: Vec<&str> = tree.folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(folders, ["A", "b folder", "C"]);
    let items: Vec<&str> = tree.items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(items, ["a.jq", "a.schema.json", "B.jq"]);
}

#[test]
fn a_folder_with_nothing_in_it_is_in_the_tree() {
    let dir = ScratchDir::new("ws", "empty-folders");
    fs::create_dir_all(dir.join("Empty/Deeper")).unwrap();
    put(&dir, "a.jq", "x");
    let workspace = open(&dir);
    assert_eq!(workspace.tree().folder_paths(), ["Empty", "Empty/Deeper"]);
    assert!(workspace
        .tree()
        .find_folder("Empty/Deeper")
        .unwrap()
        .items
        .is_empty());
}

#[test]
fn the_folders_are_read_to_a_depth_and_what_is_left_out_is_said() {
    let dir = ScratchDir::new("ws", "depth");
    let deep = |levels: usize| vec!["d"; levels].join("/");
    put(&dir, &format!("{}/in.jq", deep(16)), "x");
    put(&dir, &format!("{}/out.jq", deep(17)), "x");
    let workspace = open(&dir);
    let found = paths(&workspace);
    assert_eq!(found, vec![format!("{}/in.jq", deep(16))]);
    assert!(
        workspace.notes().iter().any(|n| n.contains("16 levels")),
        "{:?}",
        workspace.notes()
    );
}

#[cfg(unix)]
#[test]
fn a_link_is_not_followed_and_it_is_said() {
    let dir = ScratchDir::new("ws", "links");
    let outside = ScratchDir::new("ws", "outside");
    put(&outside, "secret.jq", "x");
    put(&dir, "real.jq", "x");
    std::os::unix::fs::symlink(&*outside, dir.join("folder link")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret.jq"), dir.join("file link.jq")).unwrap();
    let workspace = open(&dir);
    assert_eq!(paths(&workspace), ["real.jq"]);
    assert!(workspace.tree().folders.is_empty());
    assert!(
        workspace.notes().iter().any(|n| n.starts_with("2 links")),
        "{:?}",
        workspace.notes()
    );
}

#[test]
fn an_item_has_a_sample_when_a_file_beside_it_is_named_for_it() {
    let dir = ScratchDir::new("ws", "samples");
    put(&dir, "f/a.jq", ".a");
    put(&dir, "f/a.sample.json", "[1, 2]");
    put(&dir, "f/b.jq", ".b");
    put(&dir, "f/z.sample.json", "[]");
    let workspace = open(&dir);
    let a = workspace.item("f/a.jq").unwrap();
    let sample = a.sample.as_ref().expect("a has a sample");
    assert_eq!(sample.path, "f/a.sample.json");
    assert_eq!(sample.bytes, 6);
    assert_eq!(sample.sha256, sha("[1, 2]"));
    assert!(workspace.item("f/b.jq").unwrap().sample.is_none());
    assert_eq!(workspace.read_sample(a, 1000).unwrap(), "[1, 2]");
    assert!(workspace
        .read_sample(workspace.item("f/b.jq").unwrap(), 1000)
        .is_err());
}

#[test]
fn the_items_of_one_name_in_a_folder_share_their_sample() {
    let dir = ScratchDir::new("ws", "shared-sample");
    put(&dir, "orders.jq", ".a");
    put(&dir, "orders.schema.json", "{}");
    put(&dir, "orders.sample.json", "[]");
    let workspace = open(&dir);
    for path in ["orders.jq", "orders.schema.json"] {
        let sample = workspace.item(path).unwrap().sample.as_ref().unwrap();
        assert_eq!(sample.path, "orders.sample.json", "{path}");
    }
}

// ---- the id and the SHA-256 -----------------------------------------------------

#[test]
fn every_item_has_an_id_of_its_own_and_the_sha_of_what_it_holds_and_they_are_kept() {
    let dir = ScratchDir::new("ws", "ids");
    put(&dir, "a.jq", ".a");
    put(&dir, "f/b.schema.json", "{}");
    put(&dir, "f/b.sample.json", "[1]");
    let workspace = open(&dir);
    let a = workspace.item("a.jq").unwrap();
    let b = workspace.item("f/b.schema.json").unwrap();
    assert_eq!(a.sha256, sha(".a"));
    assert_eq!(b.sha256, sha("{}"));
    assert_ne!(a.id, b.id);
    assert!(!a.id.is_empty() && !b.id.is_empty());

    // They are in the index, in the workspace's folder, with the sample's.
    let (index, state, notes) = Index::read(&dir);
    assert_eq!(state, State::Read);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(index.items["a.jq"].id, a.id);
    assert_eq!(index.items["a.jq"].sha256, sha(".a"));
    assert_eq!(index.items["a.jq"].bytes, 2);
    assert!(index.items["a.jq"].modified > 0);
    assert_eq!(
        index.items["f/b.schema.json"]
            .sample
            .as_ref()
            .unwrap()
            .sha256,
        sha("[1]")
    );
    assert_eq!(index.items.len(), 2);
}

#[test]
fn the_ids_are_the_same_the_next_time_the_workspace_is_read() {
    let dir = ScratchDir::new("ws", "ids-kept");
    put(&dir, "a.jq", ".a");
    put(&dir, "f/b.jq", ".b");
    let first = Workspace::open(dir.to_path_buf());
    let ids: Vec<String> = first
        .tree()
        .all_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    drop(first);
    let second = Workspace::open(dir.to_path_buf());
    let again: Vec<String> = second
        .tree()
        .all_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(again, ids);
}

#[test]
fn every_id_in_a_workspace_is_different() {
    let dir = ScratchDir::new("ws", "ids-unique");
    for n in 0..60 {
        put(&dir, &format!("f{}/q{n}.jq", n % 5), &format!(".{n}"));
    }
    let workspace = Workspace::open(dir.to_path_buf());
    let ids: HashSet<String> = workspace
        .tree()
        .all_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(ids.len(), 60);
}

#[test]
fn a_file_that_another_program_changed_keeps_its_id_and_has_a_new_sha() {
    let dir = ScratchDir::new("ws", "edited");
    put(&dir, "a.jq", ".a");
    let mut workspace = open(&dir);
    let id = id_of(&workspace, "a.jq");
    put(&dir, "a.jq", ".a | .b | .c");
    workspace.rescan();
    let item = workspace.item("a.jq").unwrap();
    assert_eq!(item.id, id);
    assert_eq!(item.sha256, sha(".a | .b | .c"));
    assert_eq!(item.bytes, 12);
    let (index, _, _) = Index::read(&dir);
    assert_eq!(
        index.items["a.jq"].sha256,
        sha(".a | .b | .c"),
        "and the index says so"
    );
}

#[test]
fn a_file_that_is_the_same_size_and_time_as_it_was_is_not_read_again() {
    let dir = ScratchDir::new("ws", "cache");
    put(&dir, "a.jq", "AAAA");
    let mut workspace = open(&dir);
    let file = dir.join("a.jq");
    let time = fs::metadata(&file).unwrap().modified().unwrap();
    fs::write(&file, "BBBB").unwrap();
    set_mtime(&file, time);
    workspace.rescan();
    // The time and the size said that nothing changed, so the sha is what it was.
    assert_eq!(workspace.item("a.jq").unwrap().sha256, sha("AAAA"));
    // A change of the time is a change.
    set_mtime(&file, time + Duration::from_secs(5));
    workspace.rescan();
    assert_eq!(workspace.item("a.jq").unwrap().sha256, sha("BBBB"));
}

#[test]
fn a_file_that_another_program_renamed_or_moved_is_found_by_what_it_holds_and_keeps_its_id() {
    let dir = ScratchDir::new("ws", "moved");
    put(&dir, "a.jq", "the first query");
    put(&dir, "f/b.jq", "the second query");
    put(&dir, "f/b.sample.json", "[1]");
    let mut workspace = open(&dir);
    let (a, b) = (id_of(&workspace, "a.jq"), id_of(&workspace, "f/b.jq"));

    fs::rename(dir.join("a.jq"), dir.join("renamed.jq")).unwrap();
    fs::create_dir_all(dir.join("g")).unwrap();
    fs::rename(dir.join("f/b.jq"), dir.join("g/b.jq")).unwrap();
    fs::rename(dir.join("f/b.sample.json"), dir.join("g/b.sample.json")).unwrap();
    workspace.rescan();
    assert_eq!(id_of(&workspace, "renamed.jq"), a);
    assert_eq!(id_of(&workspace, "g/b.jq"), b);
    assert!(workspace.item("g/b.jq").unwrap().sample.is_some());
    let (index, _, _) = Index::read(&dir);
    let keys: Vec<&str> = index.items.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["g/b.jq", "renamed.jq"],
        "the old places are gone from the index"
    );
}

#[test]
fn two_files_with_the_same_text_that_were_both_moved_keep_an_id_each() {
    let dir = ScratchDir::new("ws", "twins");
    put(&dir, "x.jq", ".same");
    put(&dir, "y.jq", ".same");
    let mut workspace = open(&dir);
    let before: HashSet<String> = [id_of(&workspace, "x.jq"), id_of(&workspace, "y.jq")].into();
    assert_eq!(before.len(), 2);
    fs::rename(dir.join("x.jq"), dir.join("p.jq")).unwrap();
    fs::rename(dir.join("y.jq"), dir.join("q.jq")).unwrap();
    workspace.rescan();
    let after: HashSet<String> = [id_of(&workspace, "p.jq"), id_of(&workspace, "q.jq")].into();
    assert_eq!(after, before, "the same two ids, one each");
}

#[test]
fn a_copy_of_a_file_made_by_another_program_is_a_new_item_with_an_id_of_its_own() {
    let dir = ScratchDir::new("ws", "copied");
    put(&dir, "a.jq", ".a");
    let mut workspace = open(&dir);
    let id = id_of(&workspace, "a.jq");
    fs::copy(dir.join("a.jq"), dir.join("b.jq")).unwrap();
    workspace.rescan();
    assert_eq!(id_of(&workspace, "a.jq"), id, "the original keeps its id");
    assert_ne!(id_of(&workspace, "b.jq"), id, "the copy has another");
    assert_eq!(
        workspace.item("b.jq").unwrap().sha256,
        workspace.item("a.jq").unwrap().sha256
    );
}

#[test]
fn a_file_that_is_gone_is_taken_out_of_the_index_and_its_id_is_not_given_again() {
    let dir = ScratchDir::new("ws", "gone");
    put(&dir, "a.jq", ".a");
    put(&dir, "b.jq", ".b");
    let mut workspace = open(&dir);
    let gone = id_of(&workspace, "a.jq");
    fs::remove_file(dir.join("a.jq")).unwrap();
    workspace.rescan();
    assert_eq!(paths(&workspace), ["b.jq"]);
    let (index, _, _) = Index::read(&dir);
    assert!(!index.items.contains_key("a.jq"));
    put(&dir, "c.jq", ".c");
    workspace.rescan();
    assert_ne!(id_of(&workspace, "c.jq"), gone);
}

#[test]
fn two_entries_with_the_same_id_in_the_index_are_made_different() {
    let dir = ScratchDir::new("ws", "same-id");
    put(&dir, "a.jq", ".a");
    put(&dir, "b.jq", ".b");
    let workspace = open(&dir);
    let id = id_of(&workspace, "a.jq");
    drop(workspace);
    let (mut index, _, _) = Index::read(&dir);
    index.items.get_mut("b.jq").unwrap().id = id.clone();
    index.write(&dir).unwrap();
    let workspace = open(&dir);
    assert_eq!(id_of(&workspace, "a.jq"), id, "the first keeps it");
    assert_ne!(id_of(&workspace, "b.jq"), id, "the second is given another");
}

#[test]
fn an_index_that_cannot_be_used_is_set_aside_and_made_again() {
    let dir = ScratchDir::new("ws", "bad-index");
    put(&dir, "a.jq", ".a");
    put(&dir, ".jsonquery/index.json", "{ this is not JSON");
    let workspace = open(&dir);
    assert_eq!(workspace.item("a.jq").unwrap().sha256, sha(".a"));
    assert_eq!(workspace.notes().len(), 1, "{:?}", workspace.notes());
    assert_eq!(
        text(&dir, ".jsonquery/index.json.bad"),
        "{ this is not JSON"
    );
    let (index, state, _) = Index::read(&dir);
    assert_eq!(state, State::Read);
    assert!(index.items.contains_key("a.jq"));
}

#[test]
fn an_index_of_a_newer_version_is_never_written_over() {
    let dir = ScratchDir::new("ws", "newer-index");
    put(&dir, "a.jq", ".a");
    let newer = r#"{"version": 2, "items": {"a.jq": {"id": "from the future"}}}"#;
    put(&dir, ".jsonquery/index.json", newer);
    let mut workspace = open(&dir);
    assert_eq!(
        paths(&workspace),
        ["a.jq"],
        "the items are listed all the same"
    );
    assert!(!workspace.notes().is_empty());
    // Not even when something is saved.
    save(&mut workspace, "", "b", ".b");
    assert_eq!(text(&dir, ".jsonquery/index.json"), newer);
}

// ---- save --------------------------------------------------------------------------

#[test]
fn an_item_is_saved_in_a_folder_that_is_made_and_has_an_id_and_a_sha() {
    let dir = ScratchDir::new("ws", "save");
    let mut workspace = open(&dir);
    let item = workspace
        .save(NewItem {
            folder: "Logs/Errors",
            name: "slow requests",
            kind: JQ,
            body: "map(select(.ms > 500))",
            sample: SampleSource::Text("[{\"ms\": 600}]".to_owned()),
            replace: false,
        })
        .unwrap();
    assert_eq!(item.path, "Logs/Errors/slow requests.jq");
    assert_eq!(item.name, "slow requests");
    assert_eq!(item.kind, JQ);
    assert_eq!(item.sha256, sha("map(select(.ms > 500))"));
    assert!(!item.id.is_empty());
    assert_eq!(
        text(&dir, "Logs/Errors/slow requests.jq"),
        "map(select(.ms > 500))"
    );
    assert_eq!(
        text(&dir, "Logs/Errors/slow requests.sample.json"),
        "[{\"ms\": 600}]"
    );
    let sample = item.sample.as_ref().unwrap();
    assert_eq!(sample.sha256, sha("[{\"ms\": 600}]"));
    // …and what the index says of it is what the item says.
    let (index, _, _) = Index::read(&dir);
    assert_eq!(index.items[&item.path].id, item.id);
    assert_eq!(index.items[&item.path].sha256, item.sha256);
    // Nothing else is left in the folder (no half-written files).
    let left: Vec<String> = fs::read_dir(dir.join("Logs/Errors"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left.len(), 2, "{left:?}");
}

#[test]
fn each_kind_is_saved_with_the_end_that_says_what_it_is() {
    let dir = ScratchDir::new("ws", "save-kinds");
    let mut workspace = open(&dir);
    for (kind, file) in [
        (ItemKind::Query(Engine::JsonPath), "a.jsonpath"),
        (ItemKind::Query(Engine::JmesPath), "b.jmespath"),
        (ItemKind::Query(Engine::JsonPointer), "c.jsonptr"),
        (ItemKind::Schema, "d.schema.json"),
        (ItemKind::Patch, "e.patch.json"),
        (ItemKind::Document, "f.json"),
    ] {
        let name = file.split('.').next().unwrap();
        let item = workspace
            .save(NewItem {
                folder: "",
                name,
                kind,
                body: "x",
                sample: SampleSource::Keep,
                replace: false,
            })
            .unwrap();
        assert_eq!((item.path.as_str(), item.kind), (file, kind));
        assert!(dir.join(file).is_file());
    }
}

#[test]
fn an_item_is_not_saved_over_one_that_is_there_unless_that_is_asked_for() {
    let dir = ScratchDir::new("ws", "save-replace");
    let mut workspace = open(&dir);
    let first = save(&mut workspace, "", "slow", ".first");
    let refused = workspace.save(NewItem {
        folder: "",
        name: "slow",
        kind: JQ,
        body: ".second",
        sample: SampleSource::Keep,
        replace: false,
    });
    assert!(refused.unwrap_err().contains("already"));
    assert_eq!(text(&dir, "slow.jq"), ".first");

    let replaced = workspace
        .save(NewItem {
            folder: "",
            name: "slow",
            kind: JQ,
            body: ".second",
            sample: SampleSource::Keep,
            replace: true,
        })
        .unwrap();
    assert_eq!(text(&dir, "slow.jq"), ".second");
    assert_eq!(replaced.id, first.id, "it is the same item, saved again");
    assert_eq!(replaced.sha256, sha(".second"));
    assert_ne!(replaced.sha256, first.sha256);
    let (index, _, _) = Index::read(&dir);
    assert_eq!(index.items.len(), 1);
    assert_eq!(index.items["slow.jq"].sha256, sha(".second"));
}

#[test]
fn a_name_that_differs_only_in_its_case_is_the_item_that_is_there() {
    let dir = ScratchDir::new("ws", "save-case");
    let mut workspace = open(&dir);
    let first = save(&mut workspace, "", "Slow", ".first");
    assert!(workspace
        .save(NewItem {
            folder: "",
            name: "slow",
            kind: JQ,
            body: "x",
            sample: SampleSource::Keep,
            replace: false
        })
        .is_err());
    let replaced = workspace
        .save(NewItem {
            folder: "",
            name: "slow",
            kind: JQ,
            body: ".second",
            sample: SampleSource::Keep,
            replace: true,
        })
        .unwrap();
    assert_eq!(replaced.path, "Slow.jq", "in the case it had");
    assert_eq!(replaced.id, first.id);
}

#[test]
fn a_sample_is_kept_replaced_or_moved_in_when_an_item_is_saved() {
    let dir = ScratchDir::new("ws", "save-sample");
    let mut workspace = open(&dir);
    save_with_sample(&mut workspace, "", "a", ".a", "[1]");
    // Saved again with Keep: the sample is as it was.
    workspace
        .save(NewItem {
            folder: "",
            name: "a",
            kind: JQ,
            body: ".a2",
            sample: SampleSource::Keep,
            replace: true,
        })
        .unwrap();
    assert_eq!(text(&dir, "a.sample.json"), "[1]");
    // Saved with another sample: it takes its place.
    let item = workspace
        .save(NewItem {
            folder: "",
            name: "a",
            kind: JQ,
            body: ".a2",
            sample: SampleSource::Text("[2]".into()),
            replace: true,
        })
        .unwrap();
    assert_eq!(text(&dir, "a.sample.json"), "[2]");
    assert_eq!(item.sample.unwrap().sha256, sha("[2]"));
    // A file that was written elsewhere is moved in, and is gone from there.
    let staged = workspace.staging_path();
    fs::write(&staged, "[3]").unwrap();
    let item = workspace
        .save(NewItem {
            folder: "",
            name: "b",
            kind: JQ,
            body: ".b",
            sample: SampleSource::Staged(staged.clone()),
            replace: false,
        })
        .unwrap();
    assert_eq!(text(&dir, "b.sample.json"), "[3]");
    assert!(!staged.exists());
    assert_eq!(item.sample.unwrap().bytes, 3);
}

#[test]
fn a_name_the_file_system_would_not_take_is_refused_and_nothing_is_written() {
    let dir = ScratchDir::new("ws", "save-names");
    let mut workspace = open(&dir);
    for (folder, name, kind) in [
        ("", "a/b", JQ),
        ("", "", JQ),
        ("", "  ", JQ),
        ("", ".hidden", JQ),
        ("", "CON", JQ),
        ("", "a.schema", ItemKind::Document),
        ("../out", "a", JQ),
        ("/abs", "a", JQ),
        ("a//b", "a", JQ),
        ("a:b", "a", JQ),
        (".git", "a", JQ),
    ] {
        let result = workspace.save(NewItem {
            folder,
            name,
            kind,
            body: "x",
            sample: SampleSource::Keep,
            replace: false,
        });
        assert!(result.is_err(), "{folder:?} / {name:?} was saved");
    }
    assert_eq!(fs::read_dir(&*dir).unwrap().count(), 0, "nothing was made");
}

#[cfg(unix)]
#[test]
fn nothing_is_written_through_a_link() {
    let dir = ScratchDir::new("ws", "save-links");
    let outside = ScratchDir::new("ws", "save-outside");
    std::os::unix::fs::symlink(&*outside, dir.join("link")).unwrap();
    put(&dir, "real/x.jq", "x");
    std::os::unix::fs::symlink(dir.join("real/x.jq"), dir.join("real/y.jq")).unwrap();
    let mut workspace = open(&dir);
    for (folder, name) in [("link", "a"), ("link/deeper", "a")] {
        let result = workspace.save(NewItem {
            folder,
            name,
            kind: JQ,
            body: "x",
            sample: SampleSource::Keep,
            replace: false,
        });
        assert!(result.is_err(), "{folder}");
    }
    assert_eq!(fs::read_dir(&*outside).unwrap().count(), 0);
    // A link as the item's own file is not an item, and is not replaced either.
    let result = workspace.save(NewItem {
        folder: "real",
        name: "y",
        kind: JQ,
        body: "new",
        sample: SampleSource::Keep,
        replace: true,
    });
    assert!(result.is_err());
    assert_eq!(text(&dir, "real/x.jq"), "x");
}

// ---- folders -------------------------------------------------------------------------

#[test]
fn a_folder_is_made_in_the_workspace_and_in_a_folder_of_it() {
    let dir = ScratchDir::new("ws", "folders");
    let mut workspace = open(&dir);
    assert_eq!(workspace.create_folder("", "Logs").unwrap(), "Logs");
    assert_eq!(
        workspace.create_folder("Logs", "Errors").unwrap(),
        "Logs/Errors"
    );
    assert_eq!(workspace.tree().folder_paths(), ["Logs", "Logs/Errors"]);
    assert!(dir.join("Logs/Errors").is_dir());
    for (parent, name) in [
        ("", "logs"),
        ("Logs", "Errors"),
        ("", "a/b"),
        ("", ""),
        ("", ".x"),
        ("Nope", "a"),
    ] {
        assert!(
            workspace.create_folder(parent, name).is_err(),
            "{parent:?} / {name:?}"
        );
    }
    assert_eq!(workspace.tree().folder_count(), 2);
}

#[test]
fn the_folder_of_a_workspace_that_is_not_there_is_made_with_its_first_folder() {
    let dir = ScratchDir::new("ws", "folders-first");
    let root = dir.join("not/there/yet");
    let mut workspace = Workspace::with_ids(root.clone(), Box::new(SequentialIds::default()));
    workspace.create_folder("", "Logs").unwrap();
    assert!(root.join("Logs").is_dir());
    assert!(workspace.exists());
}

// ---- rename, move, copy, delete ---------------------------------------------------------

#[test]
fn an_item_that_is_renamed_keeps_its_id_and_its_sample_goes_with_it() {
    let dir = ScratchDir::new("ws", "rename");
    let mut workspace = open(&dir);
    let item = save_with_sample(&mut workspace, "f", "old", ".a", "[1]");
    let new = workspace.rename("f/old.jq", "new name").unwrap();
    assert_eq!(new, "f/new name.jq");
    assert!(!dir.join("f/old.jq").exists() && !dir.join("f/old.sample.json").exists());
    assert_eq!(text(&dir, "f/new name.jq"), ".a");
    assert_eq!(text(&dir, "f/new name.sample.json"), "[1]");
    let renamed = workspace.item("f/new name.jq").unwrap();
    assert_eq!(renamed.id, item.id);
    assert_eq!(renamed.sha256, item.sha256);
    assert!(renamed.sample.is_some());
    let (index, _, _) = Index::read(&dir);
    assert_eq!(index.items.keys().collect::<Vec<_>>(), ["f/new name.jq"]);
    assert_eq!(index.items["f/new name.jq"].id, item.id);
}

#[test]
fn a_rename_is_refused_when_the_name_is_taken_or_is_no_name() {
    let dir = ScratchDir::new("ws", "rename-refused");
    let mut workspace = open(&dir);
    save(&mut workspace, "", "a", ".a");
    save(&mut workspace, "", "b", ".b");
    save_with_sample(&mut workspace, "", "c", ".c", "[1]");
    for bad in ["b", "B", "", "x/y", ".h"] {
        assert!(workspace.rename("a.jq", bad).is_err(), "{bad:?}");
    }
    assert_eq!(text(&dir, "a.jq"), ".a");
    assert_eq!(text(&dir, "b.jq"), ".b");
    // A name whose sample is there already (it is some other item's) is refused too.
    put(&dir, "orphan.sample.json", "[]");
    assert!(workspace.rename("c.jq", "orphan").is_err());
    assert_eq!(text(&dir, "c.sample.json"), "[1]");
    // The same name is no change.
    assert_eq!(workspace.rename("a.jq", "a").unwrap(), "a.jq");
    // Something that is not there.
    assert!(workspace.rename("nope.jq", "x").is_err());
}

#[test]
fn an_item_that_is_renamed_leaves_the_sample_that_another_item_of_its_name_needs() {
    let dir = ScratchDir::new("ws", "rename-shared");
    let mut workspace = open(&dir);
    save_with_sample(&mut workspace, "", "orders", ".a", "[1]");
    workspace
        .save(NewItem {
            folder: "",
            name: "orders",
            kind: ItemKind::Schema,
            body: "{}",
            sample: SampleSource::Keep,
            replace: false,
        })
        .unwrap();
    workspace.rename("orders.jq", "orders query").unwrap();
    assert!(
        dir.join("orders.sample.json").exists(),
        "the schema still has its sample"
    );
    assert!(workspace
        .item("orders.schema.json")
        .unwrap()
        .sample
        .is_some());
    assert!(workspace.item("orders query.jq").unwrap().sample.is_none());
}

#[test]
fn a_folder_that_is_renamed_keeps_the_ids_of_what_is_in_it() {
    let dir = ScratchDir::new("ws", "rename-folder");
    let mut workspace = open(&dir);
    save(&mut workspace, "Logs", "a", ".a");
    save(&mut workspace, "Logs/Errors", "b", ".b");
    save(&mut workspace, "", "c", ".c");
    let ids: Vec<String> = ["Logs/a.jq", "Logs/Errors/b.jq"]
        .iter()
        .map(|p| id_of(&workspace, p))
        .collect();
    assert_eq!(workspace.rename("Logs", "Diary").unwrap(), "Diary");
    assert_eq!(
        paths(&workspace),
        ["c.jq", "Diary/a.jq", "Diary/Errors/b.jq"]
    );
    assert_eq!(id_of(&workspace, "Diary/a.jq"), ids[0]);
    assert_eq!(id_of(&workspace, "Diary/Errors/b.jq"), ids[1]);
    assert!(workspace.rename("Diary", "Diary").is_ok());
    assert!(workspace.rename("Diary", "a/b").is_err());
    workspace.create_folder("", "Other").unwrap();
    assert!(
        workspace.rename("Diary", "other").is_err(),
        "a folder of that name is there"
    );
}

#[test]
fn an_item_is_moved_to_another_folder_with_its_id_and_its_sample() {
    let dir = ScratchDir::new("ws", "move");
    let mut workspace = open(&dir);
    let item = save_with_sample(&mut workspace, "from", "a", ".a", "[1]");
    workspace.create_folder("", "to").unwrap();
    assert_eq!(workspace.move_to("from/a.jq", "to").unwrap(), "to/a.jq");
    assert!(!dir.join("from/a.jq").exists() && !dir.join("from/a.sample.json").exists());
    assert_eq!(text(&dir, "to/a.sample.json"), "[1]");
    assert_eq!(id_of(&workspace, "to/a.jq"), item.id);
    // To the workspace's own folder, and back where it is already.
    assert_eq!(workspace.move_to("to/a.jq", "").unwrap(), "a.jq");
    assert_eq!(workspace.move_to("a.jq", "").unwrap(), "a.jq");
    assert!(workspace.move_to("a.jq", "nowhere").is_err());
}

#[test]
fn an_item_is_not_moved_over_one_that_is_there() {
    let dir = ScratchDir::new("ws", "move-clash");
    let mut workspace = open(&dir);
    save(&mut workspace, "x", "a", ".x");
    save_with_sample(&mut workspace, "y", "a", ".y", "[1]");
    let error = workspace.move_to("x/a.jq", "y").unwrap_err();
    assert!(error.contains("already"), "{error}");
    assert_eq!(text(&dir, "x/a.jq"), ".x");
    assert_eq!(text(&dir, "y/a.jq"), ".y");
    // Nor onto the sample of another item of that name.
    save_with_sample(&mut workspace, "x", "s", ".s", "[2]");
    put(&dir, "z/s.sample.json", "[]");
    assert!(workspace.move_to("x/s.jq", "z").is_err());
    assert_eq!(text(&dir, "x/s.sample.json"), "[2]");
}

#[test]
fn an_item_that_is_moved_leaves_a_sample_that_another_item_needs_and_takes_a_copy() {
    let dir = ScratchDir::new("ws", "move-shared");
    let mut workspace = open(&dir);
    save_with_sample(&mut workspace, "", "orders", ".a", "[1]");
    workspace
        .save(NewItem {
            folder: "",
            name: "orders",
            kind: ItemKind::Schema,
            body: "{}",
            sample: SampleSource::Keep,
            replace: false,
        })
        .unwrap();
    workspace.create_folder("", "to").unwrap();
    workspace.move_to("orders.jq", "to").unwrap();
    assert_eq!(text(&dir, "orders.sample.json"), "[1]");
    assert_eq!(text(&dir, "to/orders.sample.json"), "[1]");
}

#[test]
fn a_folder_is_moved_with_everything_in_it_and_not_into_itself() {
    let dir = ScratchDir::new("ws", "move-folder");
    let mut workspace = open(&dir);
    save(&mut workspace, "a/b", "x", ".x");
    workspace.create_folder("", "c").unwrap();
    let id = id_of(&workspace, "a/b/x.jq");
    assert!(workspace.move_to("a", "a").is_err());
    assert!(workspace.move_to("a", "a/b").is_err());
    assert_eq!(workspace.move_to("a", "c").unwrap(), "c/a");
    assert_eq!(id_of(&workspace, "c/a/b/x.jq"), id);
    assert!(!dir.join("a").exists());
    workspace.create_folder("", "a").unwrap();
    assert!(
        workspace.move_to("c/a", "").is_err(),
        "a folder of that name is there"
    );
}

#[test]
fn a_copy_is_made_with_the_sample_a_number_after_the_name_and_an_id_of_its_own() {
    let dir = ScratchDir::new("ws", "duplicate");
    let mut workspace = open(&dir);
    let original = save_with_sample(&mut workspace, "f", "slow", ".a", "[1]");
    let copy = workspace.duplicate("f/slow.jq").unwrap();
    assert_eq!(copy.path, "f/slow (2).jq");
    assert_ne!(copy.id, original.id);
    assert_eq!(copy.sha256, original.sha256, "the same text, the same sha");
    assert_eq!(text(&dir, "f/slow (2).jq"), ".a");
    assert_eq!(text(&dir, "f/slow (2).sample.json"), "[1]");
    let third = workspace.duplicate("f/slow.jq").unwrap();
    assert_eq!(third.path, "f/slow (3).jq");
    let copy_of_copy = workspace.duplicate("f/slow (2).jq").unwrap();
    assert_eq!(copy_of_copy.path, "f/slow (2) (2).jq");
    assert!(workspace.duplicate("f/nope.jq").is_err());
    let ids: HashSet<String> = workspace
        .tree()
        .all_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(ids.len(), 4);
}

#[test]
fn an_item_is_deleted_with_its_sample_unless_another_item_needs_it() {
    let dir = ScratchDir::new("ws", "delete-item");
    let mut workspace = open(&dir);
    save_with_sample(&mut workspace, "", "a", ".a", "[1]");
    save(&mut workspace, "", "b", ".b");
    assert_eq!(
        workspace.delete("a.jq").unwrap(),
        Deleted { items: 1, left: 0 }
    );
    assert!(!dir.join("a.jq").exists() && !dir.join("a.sample.json").exists());
    let (index, _, _) = Index::read(&dir);
    assert_eq!(index.items.keys().collect::<Vec<_>>(), ["b.jq"]);
    assert!(workspace.delete("a.jq").is_err());

    save_with_sample(&mut workspace, "", "orders", ".a", "[1]");
    workspace
        .save(NewItem {
            folder: "",
            name: "orders",
            kind: ItemKind::Schema,
            body: "{}",
            sample: SampleSource::Keep,
            replace: false,
        })
        .unwrap();
    workspace.delete("orders.jq").unwrap();
    assert!(
        dir.join("orders.sample.json").exists(),
        "the schema still has its sample"
    );
    workspace.delete("orders.schema.json").unwrap();
    assert!(
        !dir.join("orders.sample.json").exists(),
        "and when it goes the sample goes"
    );
}

#[test]
fn a_folder_is_deleted_as_far_as_the_workspace_put_things_in_it() {
    let dir = ScratchDir::new("ws", "delete-folder");
    let mut workspace = open(&dir);
    save_with_sample(&mut workspace, "a", "x", ".x", "[1]");
    save(&mut workspace, "a/b", "y", ".y");
    save(&mut workspace, "", "keep", ".keep");
    let deleted = workspace.delete("a").unwrap();
    assert_eq!(deleted, Deleted { items: 2, left: 0 });
    assert!(!dir.join("a").exists());
    assert_eq!(paths(&workspace), ["keep.jq"]);

    // A folder that has something that is not the workspace's keeps it, and itself.
    save(&mut workspace, "m/n", "z", ".z");
    put(&dir, "m/notes.txt", "mine");
    put(&dir, "m/n/.hidden.jq", "hidden");
    let deleted = workspace.delete("m").unwrap();
    assert_eq!(deleted, Deleted { items: 1, left: 2 });
    assert_eq!(text(&dir, "m/notes.txt"), "mine");
    assert_eq!(text(&dir, "m/n/.hidden.jq"), "hidden");
    assert!(
        workspace.tree().find_folder("m").is_some(),
        "the folder is still there"
    );
    assert!(
        workspace.delete("").is_err(),
        "the workspace's own folder is not deleted"
    );

    // What is hidden is not the workspace's, whatever it is called or holds: a sample
    // that starts with a dot, and a folder that does (a repository's, say) with files
    // in it that look like items.
    save(&mut workspace, "h", "q", ".q");
    put(&dir, "h/.old.sample.json", "mine");
    put(&dir, "h/.git/config.json", "mine");
    let deleted = workspace.delete("h").unwrap();
    assert_eq!(deleted, Deleted { items: 1, left: 2 });
    assert_eq!(text(&dir, "h/.old.sample.json"), "mine");
    assert_eq!(text(&dir, "h/.git/config.json"), "mine");
}

// ---- reading what is stored ---------------------------------------------------------------

#[test]
fn a_text_is_read_without_its_byte_order_mark_and_only_if_it_is_text_and_not_too_big() {
    let dir = ScratchDir::new("ws", "read");
    put(&dir, "a.jq", "\u{feff}.a");
    fs::write(dir.join("b.jq"), [0xff_u8, 0xfe, 0xfd]).unwrap();
    put(&dir, "c.jq", "0123456789");
    let workspace = open(&dir);
    assert_eq!(workspace.read_text("a.jq", 100).unwrap(), ".a");
    assert!(workspace.read_text("b.jq", 100).is_err(), "not text");
    assert_eq!(workspace.read_text("c.jq", 10).unwrap(), "0123456789");
    let too_big = workspace.read_text("c.jq", 9).unwrap_err();
    assert!(too_big.contains("too big"), "{too_big}");
    assert!(workspace.read_text("nope.jq", 100).is_err());
}

#[test]
fn a_file_waiting_to_be_moved_in_has_a_place_in_the_apps_own_folder_and_old_ones_are_swept() {
    let dir = ScratchDir::new("ws", "staging");
    let workspace = open(&dir);
    let one = workspace.staging_path();
    let two = workspace.staging_path();
    assert_ne!(one, two);
    assert_eq!(one.parent().unwrap(), dir.join(".jsonquery"));
    assert!(dir.join(".jsonquery").is_dir());

    fs::write(&one, "left over").unwrap();
    set_mtime(&one, SystemTime::now() - Duration::from_secs(3 * 24 * 3600));
    fs::write(&two, "new").unwrap();
    // Another file of the app's folder, as old as the left-over one: it is not staged, so
    // it is not swept.
    put(&dir, ".jsonquery/index.json.bad", "keep");
    set_mtime(
        &dir.join(".jsonquery/index.json.bad"),
        SystemTime::now() - Duration::from_secs(3 * 24 * 3600),
    );
    drop(workspace);
    let _again = open(&dir);
    assert!(!one.exists(), "a day old and more: swept");
    assert!(two.exists(), "this one is new");
    assert!(
        dir.join(".jsonquery/index.json.bad").exists(),
        "and nothing else is touched"
    );
}

#[test]
fn an_item_is_found_by_its_id_wherever_it_has_gone() {
    let dir = ScratchDir::new("ws", "by-id");
    let mut workspace = open(&dir);
    let item = save(&mut workspace, "a", "x", ".x");
    workspace.move_to("a/x.jq", "").unwrap();
    assert_eq!(workspace.item_by_id(&item.id).unwrap().path, "x.jq");
    assert!(workspace.item_by_id("no such id").is_none());
}

// ---- export and import ----------------------------------------------------------------------

/// A workspace to move around: two folders, a schema, and three samples.
fn something(dir: &Path) -> Workspace {
    let mut workspace = open(dir);
    save_with_sample(&mut workspace, "Logs/Errors", "slow", ".slow", "[1]");
    save(&mut workspace, "Logs", "logins", ".logins");
    workspace
        .save(NewItem {
            folder: "Services",
            name: "orders",
            kind: ItemKind::Schema,
            body: "{\"type\": \"object\"}",
            sample: SampleSource::Text("{}".to_owned()),
            replace: false,
        })
        .unwrap();
    save(&mut workspace, "", "scratch", ".scratch");
    workspace
}

#[test]
fn a_workspace_is_exported_to_a_new_folder_with_its_samples_and_its_ids() {
    let dir = ScratchDir::new("ws", "export");
    let out = ScratchDir::new("ws", "export-to");
    let mut workspace = something(&dir);
    let to = out.join("copy");
    let done = workspace.export(&Scope::Everything, &to, true).unwrap();
    assert_eq!((done.items, done.samples, done.folders), (4, 2, 3));
    assert_eq!(text(&to, "Logs/Errors/slow.jq"), ".slow");
    assert_eq!(text(&to, "Logs/Errors/slow.sample.json"), "[1]");
    assert_eq!(
        text(&to, "Services/orders.schema.json"),
        "{\"type\": \"object\"}"
    );
    assert_eq!(text(&to, "scratch.jq"), ".scratch");

    // What is there is a workspace, and every item has the id and the sha it had.
    let copy = open(&to);
    for item in workspace.tree().all_items() {
        let there = copy
            .item(&item.path)
            .unwrap_or_else(|| panic!("{} was not exported", item.path));
        assert_eq!(
            (&there.id, &there.sha256),
            (&item.id, &item.sha256),
            "{}",
            item.path
        );
        assert_eq!(
            there.sample.as_ref().map(|s| &s.sha256),
            item.sample.as_ref().map(|s| &s.sha256)
        );
    }
    assert!(copy.notes().is_empty(), "{:?}", copy.notes());
    // The workspace itself is as it was.
    assert_eq!(paths(&workspace).len(), 4);
}

#[test]
fn an_export_can_leave_the_samples_out() {
    let dir = ScratchDir::new("ws", "export-bare");
    let out = ScratchDir::new("ws", "export-bare-to");
    let mut workspace = something(&dir);
    let to = out.join("bare");
    let done = workspace.export(&Scope::Everything, &to, false).unwrap();
    assert_eq!((done.items, done.samples), (4, 0));
    assert!(!to.join("Logs/Errors/slow.sample.json").exists());
    let (index, _, _) = Index::read(&to);
    assert!(index.items.values().all(|e| e.sample.is_none()));
    assert!(open(&to)
        .tree()
        .all_items()
        .iter()
        .all(|i| i.sample.is_none()));
}

#[test]
fn a_folder_is_exported_as_the_top_of_what_comes_out() {
    let dir = ScratchDir::new("ws", "export-folder");
    let out = ScratchDir::new("ws", "export-folder-to");
    let mut workspace = something(&dir);
    let done = workspace
        .export(&Scope::Folder("Logs".to_owned()), &out.join("logs"), true)
        .unwrap();
    assert_eq!((done.items, done.samples, done.folders), (2, 1, 1));
    let copy = open(&out.join("logs"));
    assert_eq!(paths(&copy), ["logins.jq", "Errors/slow.jq"]);
    assert_eq!(
        copy.item("Errors/slow.jq").unwrap().id,
        id_of(&workspace, "Logs/Errors/slow.jq")
    );
    assert!(workspace
        .export(&Scope::Folder("Nope".to_owned()), &out.join("x"), true)
        .is_err());
    assert!(workspace
        .export(&Scope::Folder(String::new()), &out.join("y"), true)
        .is_err());
}

#[test]
fn an_export_goes_only_to_a_folder_that_is_new_or_empty_and_outside_the_workspace() {
    let dir = ScratchDir::new("ws", "export-refused");
    let out = ScratchDir::new("ws", "export-refused-to");
    let mut workspace = something(&dir);
    // A folder that has something in it.
    put(&out, "full/a.txt", "mine");
    let error = workspace
        .export(&Scope::Everything, &out.join("full"), true)
        .unwrap_err();
    assert!(error.contains("not empty"), "{error}");
    assert_eq!(text(&out, "full/a.txt"), "mine");
    // A file.
    put(&out, "file", "x");
    assert!(workspace
        .export(&Scope::Everything, &out.join("file"), true)
        .is_err());
    // An empty one is fine.
    fs::create_dir(out.join("empty")).unwrap();
    assert!(workspace
        .export(&Scope::Everything, &out.join("empty"), true)
        .is_ok());
    // Inside the workspace, or the workspace inside it (and it is said why: a folder
    // that has the workspace in it is not empty either, but that is not the reason).
    let reason = "not in the workspace, nor has it in it";
    for to in [dir.join("Logs/export"), dir.to_path_buf()] {
        let error = workspace.export(&Scope::Everything, &to, true).unwrap_err();
        assert!(error.contains(reason), "{to:?}: {error}");
    }
    let parent = ScratchDir::new("ws", "export-parent");
    let inner = parent.join("workspace");
    fs::create_dir(&inner).unwrap();
    let mut nested = Workspace::with_ids(inner, Box::new(SequentialIds::default()));
    save(&mut nested, "", "a", ".a");
    let error = nested
        .export(&Scope::Everything, &parent, true)
        .unwrap_err();
    assert!(error.contains(reason), "{error}");
    assert!(!dir.join("Logs/export").exists());
}

#[test]
fn an_export_that_cannot_make_its_folder_says_so_and_leaves_nothing() {
    let dir = ScratchDir::new("ws", "export-failed");
    let out = ScratchDir::new("ws", "export-failed-to");
    let mut workspace = something(&dir);
    put(&out, "a file", "x");
    let result = workspace.export(&Scope::Everything, &out.join("a file/inside"), true);
    assert!(result.is_err());
    assert_eq!(text(&out, "a file"), "x");
    assert_eq!(paths(&workspace).len(), 4, "the workspace is as it was");
}

#[test]
fn a_folder_is_imported_into_a_folder_of_its_own_with_the_ids_it_had() {
    let source = ScratchDir::new("ws", "import-from");
    let dir = ScratchDir::new("ws", "import-into");
    let mut theirs = something(&source);
    let mut workspace = open_after(&dir, 100);
    save(&mut workspace, "", "mine", ".mine");
    let done = workspace
        .import(
            &source,
            &Place::NewFolder("team".to_owned()),
            Clash::KeepBoth,
        )
        .unwrap();
    assert_eq!(
        (
            done.added,
            done.replaced,
            done.renamed,
            done.skipped,
            done.identical
        ),
        (4, 0, 0, 0, 0)
    );
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    assert_eq!(text(&dir, "team/Logs/Errors/slow.jq"), ".slow");
    assert_eq!(text(&dir, "team/Logs/Errors/slow.sample.json"), "[1]");
    assert_eq!(
        text(&dir, "team/Services/orders.schema.json"),
        "{\"type\": \"object\"}"
    );
    assert_eq!(text(&dir, "mine.jq"), ".mine");
    for item in theirs.tree().all_items() {
        let here = workspace.item(&format!("team/{}", item.path)).unwrap();
        assert_eq!(
            (&here.id, &here.sha256),
            (&item.id, &item.sha256),
            "{}",
            item.path
        );
    }
    // The folder that was imported is as it was: nothing was written in it.
    theirs.rescan();
    assert_eq!(paths(&theirs).len(), 4);
    // And what is in the workspace has its index in order.
    let (index, state, _) = Index::read(&dir);
    assert_eq!(state, State::Read);
    assert_eq!(index.items.len(), 5);
}

#[test]
fn the_folder_for_an_import_is_a_new_one_with_a_number_when_its_name_is_taken() {
    let source = ScratchDir::new("ws", "import-new-from");
    let dir = ScratchDir::new("ws", "import-new-into");
    something(&source);
    let mut workspace = open(&dir);
    workspace.create_folder("", "team").unwrap();
    workspace
        .import(
            &source,
            &Place::NewFolder("team".to_owned()),
            Clash::KeepBoth,
        )
        .unwrap();
    assert!(dir.join("team (2)/scratch.jq").exists());
    workspace
        .import(
            &source,
            &Place::NewFolder("TEAM".to_owned()),
            Clash::KeepBoth,
        )
        .unwrap();
    assert!(dir.join("TEAM (3)/scratch.jq").exists() || dir.join("TEAM/scratch.jq").exists());
    assert!(workspace
        .import(
            &source,
            &Place::NewFolder("a/b".to_owned()),
            Clash::KeepBoth
        )
        .is_err());
}

#[test]
fn what_is_the_same_as_what_is_there_is_not_brought_in_again() {
    let source = ScratchDir::new("ws", "import-twice-from");
    let dir = ScratchDir::new("ws", "import-twice-into");
    something(&source);
    let mut workspace = open(&dir);
    let first = workspace
        .import(&source, &Place::Inside(String::new()), Clash::KeepBoth)
        .unwrap();
    assert_eq!(first.added, 4);
    let ids: Vec<String> = workspace
        .tree()
        .all_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    let second = workspace
        .import(&source, &Place::Inside(String::new()), Clash::KeepBoth)
        .unwrap();
    assert_eq!((second.added, second.renamed, second.identical), (0, 0, 4));
    assert_eq!(paths(&workspace).len(), 4, "no copies");
    let after: Vec<String> = workspace
        .tree()
        .all_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(after, ids);
}

#[test]
fn an_item_with_the_name_of_one_that_is_there_and_another_text_is_dealt_with_as_asked() {
    let source = ScratchDir::new("ws", "import-clash-from");
    put(&source, "a.jq", ".theirs");
    put(&source, "a.sample.json", "[9]");
    put(&source, "b.jq", ".other");

    // Keep both: theirs gets a number, with its sample.
    let dir = ScratchDir::new("ws", "import-clash-keep");
    let mut workspace = open(&dir);
    let mine = save_with_sample(&mut workspace, "", "a", ".mine", "[1]");
    let done = workspace
        .import(&source, &Place::Inside(String::new()), Clash::KeepBoth)
        .unwrap();
    assert_eq!((done.added, done.renamed), (1, 1));
    assert_eq!(text(&dir, "a.jq"), ".mine");
    assert_eq!(text(&dir, "a.sample.json"), "[1]");
    assert_eq!(text(&dir, "a (2).jq"), ".theirs");
    assert_eq!(text(&dir, "a (2).sample.json"), "[9]");
    assert_eq!(id_of(&workspace, "a.jq"), mine.id);

    // Replace: theirs takes its place, and the id that was there stays.
    let dir = ScratchDir::new("ws", "import-clash-replace");
    let mut workspace = open(&dir);
    let mine = save_with_sample(&mut workspace, "", "a", ".mine", "[1]");
    let done = workspace
        .import(&source, &Place::Inside(String::new()), Clash::Replace)
        .unwrap();
    assert_eq!((done.added, done.replaced), (1, 1));
    assert_eq!(text(&dir, "a.jq"), ".theirs");
    assert_eq!(text(&dir, "a.sample.json"), "[9]");
    assert_eq!(id_of(&workspace, "a.jq"), mine.id);
    assert_eq!(workspace.item("a.jq").unwrap().sha256, sha(".theirs"));

    // Skip: what is there stays, and the rest comes in.
    let dir = ScratchDir::new("ws", "import-clash-skip");
    let mut workspace = open(&dir);
    save(&mut workspace, "", "a", ".mine");
    let done = workspace
        .import(&source, &Place::Inside(String::new()), Clash::Skip)
        .unwrap();
    assert_eq!((done.added, done.skipped), (1, 1));
    assert_eq!(text(&dir, "a.jq"), ".mine");
    assert!(!dir.join("a.sample.json").exists());
    assert_eq!(text(&dir, "b.jq"), ".other");
}

#[test]
fn an_id_that_is_in_use_here_is_not_taken_a_second_time() {
    let source = ScratchDir::new("ws", "import-ids-from");
    put(&source, "a.jq", ".a");
    put(&source, "b.jq", ".b");
    let theirs = open(&source);
    let (their_a, their_b) = (id_of(&theirs, "a.jq"), id_of(&theirs, "b.jq"));
    let dir = ScratchDir::new("ws", "import-ids-into");
    // The workspace has an item with the id of their `a`.
    put(&dir, "mine.jq", ".mine");
    let mut workspace = open_after(&dir, 100);
    let (mut index, _, _) = Index::read(&dir);
    index.items.get_mut("mine.jq").unwrap().id = their_a.clone();
    index.write(&dir).unwrap();
    workspace.rescan();
    workspace
        .import(&source, &Place::NewFolder("in".to_owned()), Clash::KeepBoth)
        .unwrap();
    assert_eq!(id_of(&workspace, "mine.jq"), their_a);
    assert_ne!(id_of(&workspace, "in/a.jq"), their_a, "that id is in use");
    assert_eq!(id_of(&workspace, "in/b.jq"), their_b, "this one is free");
    let ids: HashSet<String> = workspace
        .tree()
        .all_items()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(ids.len(), 3);
}

#[test]
fn a_folder_with_no_index_is_imported_too_and_its_items_are_given_ids() {
    let source = ScratchDir::new("ws", "import-plain-from");
    put(&source, "Queries/a.jq", ".a");
    put(&source, "Queries/a.sample.json", "[1]");
    put(&source, "notes.txt", "not an item");
    put(&source, "schema.schema.json", "{}");
    let dir = ScratchDir::new("ws", "import-plain-into");
    let mut workspace = open(&dir);
    let done = workspace
        .import(&source, &Place::Inside(String::new()), Clash::KeepBoth)
        .unwrap();
    assert_eq!(done.added, 2);
    assert_eq!(paths(&workspace), ["schema.schema.json", "Queries/a.jq"]);
    assert!(
        !dir.join("notes.txt").exists(),
        "what is not the workspace's is not brought in"
    );
    assert!(!workspace.item("Queries/a.jq").unwrap().id.is_empty());
    assert_eq!(
        workspace
            .item("Queries/a.jq")
            .unwrap()
            .sample
            .as_ref()
            .unwrap()
            .bytes,
        3
    );
    assert!(
        !source.join(".jsonquery").exists(),
        "nothing is written in the folder that is imported"
    );
}

#[test]
fn an_import_is_put_into_a_folder_of_the_workspace_when_asked() {
    let source = ScratchDir::new("ws", "import-inside-from");
    put(&source, "a.jq", ".a");
    let dir = ScratchDir::new("ws", "import-inside-into");
    let mut workspace = open(&dir);
    workspace.create_folder("", "Billing").unwrap();
    workspace
        .import(
            &source,
            &Place::Inside("Billing".to_owned()),
            Clash::KeepBoth,
        )
        .unwrap();
    assert_eq!(paths(&workspace), ["Billing/a.jq"]);
    assert!(workspace
        .import(&source, &Place::Inside("Nope".to_owned()), Clash::KeepBoth)
        .is_err());
    assert!(workspace
        .import(&source, &Place::Inside("../x".to_owned()), Clash::KeepBoth)
        .is_err());
}

#[test]
fn a_folder_that_cannot_be_imported_says_why() {
    let dir = ScratchDir::new("ws", "import-refused");
    let mut workspace = open(&dir);
    save(&mut workspace, "Logs", "a", ".a");
    let empty = ScratchDir::new("ws", "import-refused-empty");
    put(&empty, "notes.txt", "x");
    let place = Place::NewFolder("x".to_owned());
    assert!(workspace
        .import(&empty, &place, Clash::KeepBoth)
        .unwrap_err()
        .contains("nothing"));
    assert!(workspace
        .import(&dir.join("missing"), &place, Clash::KeepBoth)
        .is_err());
    assert!(
        workspace
            .import(&dir.join("Logs/a.jq"), &place, Clash::KeepBoth)
            .is_err(),
        "a file"
    );
    assert!(
        workspace
            .import(&dir.join("Logs"), &place, Clash::KeepBoth)
            .is_err(),
        "inside it"
    );
    assert!(
        workspace.import(&dir, &place, Clash::KeepBoth).is_err(),
        "itself"
    );
    let parent = dir.parent().unwrap();
    assert!(
        workspace.import(parent, &place, Clash::KeepBoth).is_err(),
        "what has it inside"
    );
    assert_eq!(paths(&workspace), ["Logs/a.jq"]);
}

#[test]
fn what_a_folder_holds_is_counted_before_it_is_imported() {
    let source = ScratchDir::new("ws", "preview");
    let mut theirs = something(&source);
    theirs
        .save(NewItem {
            folder: "",
            name: "bump",
            kind: ItemKind::Patch,
            body: "[]",
            sample: SampleSource::Keep,
            replace: false,
        })
        .unwrap();
    theirs
        .save(NewItem {
            folder: "",
            name: "data",
            kind: ItemKind::Document,
            body: "[]",
            sample: SampleSource::Keep,
            replace: false,
        })
        .unwrap();
    put(&source, "Logs/more.jq", "x");
    let preview = preview(&source).unwrap();
    assert_eq!(preview.items, 7);
    assert_eq!(
        (
            preview.queries,
            preview.schemas,
            preview.patches,
            preview.documents
        ),
        (4, 1, 1, 1)
    );
    assert_eq!(preview.folders, 3);
    assert_eq!(preview.samples, 2);
    assert_eq!(preview.sample_bytes, 3 + 2);
    assert!(super::preview(&source.join("missing")).is_err());
}

#[test]
fn the_hash_of_a_file_that_was_exported_is_the_hash_of_the_item() {
    let dir = ScratchDir::new("ws", "hash-of-copy");
    let out = ScratchDir::new("ws", "hash-of-copy-to");
    let mut workspace = something(&dir);
    workspace
        .export(&Scope::Everything, &out.join("c"), true)
        .unwrap();
    for item in workspace.tree().all_items() {
        let (hash, _) = sha256_file(
            &out.join("c")
                .join(item.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
        )
        .unwrap();
        assert_eq!(hash, item.sha256, "{}", item.path);
    }
}

#[test]
fn a_document_that_was_written_elsewhere_is_moved_in_as_an_item() {
    let dir = ScratchDir::new("ws", "save-staged");
    let mut workspace = open(&dir);
    let staged = workspace.staging_path();
    fs::write(&staged, "[1, 2, 3]").unwrap();
    let item = workspace
        .save_staged("Data", "numbers", ItemKind::Document, &staged, false)
        .unwrap();
    assert_eq!(item.path, "Data/numbers.json");
    assert_eq!(item.sha256, sha("[1, 2, 3]"));
    assert_eq!(item.bytes, 9);
    assert_eq!(text(&dir, "Data/numbers.json"), "[1, 2, 3]");
    assert!(!staged.exists());
    let again = workspace.staging_path();
    fs::write(&again, "[4]").unwrap();
    assert!(workspace
        .save_staged("Data", "numbers", ItemKind::Document, &again, false)
        .is_err());
    assert!(
        again.exists(),
        "a file that was not taken is left where it was"
    );
    let replaced = workspace
        .save_staged("Data", "numbers", ItemKind::Document, &again, true)
        .unwrap();
    assert_eq!(
        (replaced.id.as_str(), replaced.sha256.as_str()),
        (item.id.as_str(), sha("[4]").as_str())
    );
}

#[test]
fn a_file_that_has_the_time_it_had_but_another_size_is_read_again() {
    let dir = ScratchDir::new("ws", "cache-size");
    put(&dir, "a.jq", "AAAA");
    let mut workspace = open(&dir);
    let file = dir.join("a.jq");
    let time = fs::metadata(&file).unwrap().modified().unwrap();
    fs::write(&file, "BBBBBB").unwrap();
    set_mtime(&file, time);
    workspace.rescan();
    assert_eq!(workspace.item("a.jq").unwrap().sha256, sha("BBBBBB"));
}

#[test]
fn a_sample_that_was_changed_is_read_again_even_when_it_is_the_same_size() {
    let dir = ScratchDir::new("ws", "sample-changed");
    put(&dir, "a.jq", ".a");
    put(&dir, "a.sample.json", "[1]");
    let mut workspace = open(&dir);
    assert_eq!(
        workspace
            .item("a.jq")
            .unwrap()
            .sample
            .as_ref()
            .unwrap()
            .sha256,
        sha("[1]")
    );
    let file = dir.join("a.sample.json");
    let time = fs::metadata(&file).unwrap().modified().unwrap();
    fs::write(&file, "[2]").unwrap();
    set_mtime(&file, time + Duration::from_secs(5));
    workspace.rescan();
    assert_eq!(
        workspace
            .item("a.jq")
            .unwrap()
            .sample
            .as_ref()
            .unwrap()
            .sha256,
        sha("[2]")
    );
    let (index, _, _) = Index::read(&dir);
    assert_eq!(
        index.items["a.jq"].sample.as_ref().unwrap().sha256,
        sha("[2]")
    );
}

#[test]
fn the_id_of_an_item_that_is_gone_is_not_given_to_one_that_is_new() {
    let dir = ScratchDir::new("ws", "id-not-reused");
    put(&dir, "a.jq", ".a");
    let gone = id_of(&open(&dir), "a.jq");
    fs::remove_file(dir.join("a.jq")).unwrap();
    put(&dir, "b.jq", ".something else entirely");
    // The index still has the id of `a.jq` (nothing has read the folder since), and
    // the ids that are made next start from the same place as the first ones did.
    let workspace = open(&dir);
    assert_ne!(id_of(&workspace, "b.jq"), gone);
}

#[test]
fn a_sample_that_two_items_share_is_one_sample_in_what_an_import_would_bring() {
    let source = ScratchDir::new("ws", "preview-shared");
    put(&source, "orders.jq", ".a");
    put(&source, "orders.schema.json", "{}");
    put(&source, "orders.sample.json", "[1, 2]");
    let preview = preview(&source).unwrap();
    assert_eq!(
        (preview.items, preview.samples, preview.sample_bytes),
        (2, 1, 6)
    );
}
