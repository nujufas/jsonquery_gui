//! A folder of a test's own in the temp dir, taken away when it is dropped, so
//! that `cargo test` leaves nothing behind.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// An empty folder `jsonquery-<kind>-<name>-<process>-<count>` in the temp dir,
/// removed with what is in it when this is dropped (on Windows a file that is
/// still open keeps its folder: a leftover, no more). It stands for its own
/// path: `dir.join(..)`, and `&dir` where a `&Path` is wanted. Every one is a
/// folder of its own, so tests running side by side never share a file, even
/// when they give the same name.
pub struct ScratchDir(PathBuf);

impl ScratchDir {
    pub fn new(kind: &str, name: &str) -> Self {
        static MADE: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "jsonquery-{kind}-{name}-{}-{}",
            std::process::id(),
            MADE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// The path `name` in this folder, which keeps the folder for as long as it
    /// is kept. Nothing is made: the file is the caller's to write.
    pub fn into_file(self, name: &str) -> ScratchFile {
        ScratchFile {
            path: self.0.join(name),
            _dir: self,
        }
    }
}

impl Deref for ScratchDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for ScratchDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A path in a [`ScratchDir`] that holds the folder, so that the folder goes
/// when the path does.
pub struct ScratchFile {
    path: PathBuf,
    _dir: ScratchDir,
}

impl Deref for ScratchFile {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for ScratchFile {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_goes_with_the_guard_and_what_is_in_it() {
        let dir = ScratchDir::new("scratch", "goes");
        let path = dir.to_path_buf();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        assert!(path.join("a.txt").exists());
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn two_that_are_given_the_same_name_are_two_folders() {
        let a = ScratchDir::new("scratch", "same");
        let b = ScratchDir::new("scratch", "same");
        assert_ne!(a.to_path_buf(), b.to_path_buf());
    }

    #[test]
    fn a_file_keeps_its_folder_until_it_is_dropped() {
        let file = ScratchDir::new("scratch", "file").into_file("a.json");
        std::fs::write(&file, "[]").unwrap();
        let folder = file.parent().unwrap().to_path_buf();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[]");
        drop(file);
        assert!(!folder.exists());
    }
}
