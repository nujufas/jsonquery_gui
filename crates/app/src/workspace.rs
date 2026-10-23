//! The workspace: a folder of the user's own queries, schemas, patches, documents and
//! web addresses, each with an optional sample (but an address, which has none), to
//! come back to.
//!
//! It is a folder in the ordinary sense — folders inside folders, and files in
//! them — so that it can be opened in a file manager, kept in a repository, copied
//! to a stick or sent to a colleague, and so that other programs can use what is
//! in it (a `.jq` file is a jq program, a `.schema.json` file a JSON Schema). It is
//! in the folder `workspace` beside the settings (`~/.jsonquery/workspace`) unless
//! the user has chosen another.
//!
//! * `names.rs` — which file is which kind of item, and the names that may be given.
//! * `address.rs` — the web address that a `.url` item holds: what may be one, how a file
//!   keeps it, and the name it is saved under to begin with.
//! * `files.rs` — writing a file whole or not at all, and keeping to the workspace's
//!   folder.
//! * `ids.rs` — the SHA-256 of what an item holds, and the id it keeps.
//! * `index.rs` — the hidden file that remembers each item's id and SHA-256.
//! * `store.rs` — [`Workspace`]: the folder as a tree, and what is done to it.
//! * `transfer.rs` — export and import.
//!
//! What the user sees of it — the pane, its menus and forms — is `workspace_pane.rs`.

mod address;
mod files;
mod ids;
mod index;
mod names;
mod store;
mod transfer;

#[cfg(test)]
mod tests;

pub use address::{address_in, default_name, shortcut_text, web_address};
#[cfg(test)]
pub use ids::sha256_hex;
pub use names::{check_item_name, check_name, query_language_of, Group, ItemKind};
#[cfg(test)]
pub use store::Deleted;
pub use store::{read_text, Folder, Item, NewItem, SampleSource, Workspace};
pub use transfer::{preview, Clash, Imported, Place, Preview, Scope};

/// The folder that the relative path `rel` is in (the empty path for a path in the
/// workspace's own folder).
pub fn parent_of(rel: &str) -> &str {
    files::parent(rel)
}

/// The last part of the relative path `rel`.
pub fn last_part(rel: &str) -> &str {
    files::last(rel)
}

/// The file name of the sample that goes with the items called `name`.
pub fn sample_file(name: &str) -> String {
    names::sample_file_name(name)
}

/// The file name of the sample of `item`.
pub fn sample_name(item: &Item) -> String {
    names::sample_file_name(&item.name)
}
