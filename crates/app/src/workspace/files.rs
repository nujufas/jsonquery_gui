//! The small things the workspace does to files and paths: write a file so that
//! it is never half written, say where a path in the workspace is on disk, and
//! take care that nothing it does goes outside the workspace.
//!
//! A path *in* the workspace is written as text with `/` between its parts, on
//! every system (`Logs/Errors/slow requests.jq`), and is called a *relative path*
//! here: relative to the workspace's folder. The empty text is the folder itself.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

/// Write `bytes` to `path` in one step: to a file beside it that then takes its
/// place, so that what was there is never left half written. The folder is made
/// if it is not there.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let beside = temp_beside(path);
    if let Err(e) = fs::write(&beside, bytes) {
        let _ = fs::remove_file(&beside);
        return Err(e);
    }
    fs::rename(&beside, path).inspect_err(|_| {
        let _ = fs::remove_file(&beside);
    })
}

/// Copy the file `from` to `to` the same way: it is there whole or not at all.
pub fn copy_atomic(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(folder) = to.parent() {
        fs::create_dir_all(folder)?;
    }
    let beside = temp_beside(to);
    if let Err(e) = fs::copy(from, &beside) {
        let _ = fs::remove_file(&beside);
        return Err(e);
    }
    fs::rename(&beside, to).inspect_err(|_| {
        let _ = fs::remove_file(&beside);
    })
}

/// Move the file `from` to `to`, over what is there. Across two file systems a
/// rename is not possible, and the file is copied and then taken from where it was.
pub fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(folder) = to.parent() {
        fs::create_dir_all(folder)?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_atomic(from, to)?;
    fs::remove_file(from)
}

/// A name for the file that `path` is written to first: in the same folder (so
/// that it can be renamed into place), hidden (so that nothing lists it), and
/// not the same as any other that is being written.
fn temp_beside(path: &Path) -> PathBuf {
    static MADE: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().map_or_else(
        || "file".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    path.with_file_name(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        MADE.fetch_add(1, Ordering::Relaxed)
    ))
}

/// When the file was last changed, in milliseconds since 1970; 0 if the system
/// does not say.
pub fn modified_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0))
}

/// Whether `rel` is a path that stays inside a folder: parts of text with a `/`
/// between them, none empty, none `.` or `..`, none with a backslash, a colon or a
/// character that is not text in it. (The empty path, the folder itself, is one.)
pub fn is_relative_path(rel: &str) -> bool {
    rel.is_empty()
        || rel.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part
                    .chars()
                    .any(|c| c == '\\' || c == ':' || c.is_control())
        })
}

/// `name` in the folder `folder`, as a relative path (`folder` itself if there is no
/// name, and `name` if there is no folder).
pub fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else if name.is_empty() {
        folder.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// The folder that the relative path `rel` is in: the empty path for one that is
/// in the workspace's own folder.
pub fn parent(rel: &str) -> &str {
    rel.rfind('/').map_or("", |at| &rel[..at])
}

/// The last part of the relative path `rel`.
pub fn last(rel: &str) -> &str {
    rel.rfind('/').map_or(rel, |at| &rel[at + 1..])
}

/// Where the relative path `rel` is on disk, in the workspace at `root`.
pub fn on_disk(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in rel.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    path
}

/// Whether going to `rel` from `root` would pass through a link, which could lead
/// out of the workspace: a link is never followed to read, and never written
/// through. A part that is not there yet is no link.
pub fn passes_through_a_link(root: &Path, rel: &str) -> bool {
    let mut path = root.to_path_buf();
    for part in rel.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// Where `path` really is, links and `..` and all, as far as it is there: the part
/// of it that is not made yet is put after the place the rest really is.
pub fn resolved(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut here = path.to_path_buf();
    loop {
        if let Ok(real) = fs::canonicalize(&here) {
            return missing
                .iter()
                .rev()
                .fold(real, |real, part| real.join(part));
        }
        match (here.file_name().map(ToOwned::to_owned), here.parent()) {
            (Some(name), Some(above)) => {
                missing.push(name);
                here = above.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Whether `inner` is `outer` or in it, by where the two really are.
pub fn is_inside(inner: &Path, outer: &Path) -> bool {
    resolved(inner).starts_with(resolved(outer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch_dir::ScratchDir;

    #[test]
    fn a_file_is_written_whole_over_what_was_there_and_leaves_nothing_beside_it() {
        let dir = ScratchDir::new("files", "atomic");
        let path = dir.join("a").join("b.txt");
        write_atomic(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        write_atomic(&path, b"second, longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second, longer");
        let left: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("b.txt")]);
    }

    #[test]
    fn a_file_that_cannot_be_written_is_an_error_and_leaves_nothing_behind() {
        let dir = ScratchDir::new("files", "refused");
        // A folder where the file should go.
        fs::create_dir(dir.join("x")).unwrap();
        assert!(write_atomic(&dir.join("x"), b"data").is_err());
        let names: Vec<_> = fs::read_dir(&*dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["x".to_owned()], "no temporary file is left");
    }

    #[test]
    fn a_file_is_copied_and_moved_over_what_is_there() {
        let dir = ScratchDir::new("files", "copy");
        fs::write(dir.join("from"), "new").unwrap();
        fs::write(dir.join("to"), "old").unwrap();
        copy_atomic(&dir.join("from"), &dir.join("to")).unwrap();
        assert_eq!(fs::read_to_string(dir.join("to")).unwrap(), "new");
        assert!(dir.join("from").exists());
        fs::write(dir.join("to2"), "older").unwrap();
        move_file(&dir.join("from"), &dir.join("deep/er/to2")).unwrap();
        assert_eq!(fs::read_to_string(dir.join("deep/er/to2")).unwrap(), "new");
        assert!(!dir.join("from").exists());
        assert!(copy_atomic(&dir.join("missing"), &dir.join("x")).is_err());
        assert!(
            !dir.join("x").exists(),
            "nothing was made of a copy that failed"
        );
    }

    #[test]
    fn a_relative_path_stays_in_its_folder() {
        for good in ["", "a", "a/b", "Logs/Errors/slow requests.jq", "a b/c.d"] {
            assert!(is_relative_path(good), "{good:?}");
        }
        for bad in [
            "/a", "a/", "a//b", ".", "..", "a/..", "../a", "a/./b", "a\\b", "C:/a", "a/b:c", "a\nb",
        ] {
            assert!(!is_relative_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn paths_are_made_and_taken_apart_at_the_slashes() {
        assert_eq!(join("", "a.jq"), "a.jq");
        assert_eq!(join("Logs/Errors", "a.jq"), "Logs/Errors/a.jq");
        assert_eq!(join("Logs", ""), "Logs", "no name: the folder itself");
        assert_eq!(join("", ""), "");
        assert_eq!(parent("Logs/Errors/a.jq"), "Logs/Errors");
        assert_eq!(parent("a.jq"), "");
        assert_eq!(parent(""), "");
        assert_eq!(last("Logs/Errors/a.jq"), "a.jq");
        assert_eq!(last("a.jq"), "a.jq");
        assert_eq!(last("Logs"), "Logs");
    }

    #[test]
    fn a_relative_path_is_on_disk_where_its_parts_lead() {
        let root = Path::new("root");
        assert_eq!(on_disk(root, ""), PathBuf::from("root"));
        assert_eq!(
            on_disk(root, "Logs/a.jq"),
            Path::new("root").join("Logs").join("a.jq")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_found_in_the_way_and_nothing_else_is() {
        let dir = ScratchDir::new("files", "links");
        let outside = ScratchDir::new("files", "outside");
        fs::create_dir(dir.join("real")).unwrap();
        fs::write(dir.join("real/a.jq"), "x").unwrap();
        std::os::unix::fs::symlink(&*outside, dir.join("link")).unwrap();
        std::os::unix::fs::symlink(dir.join("real/a.jq"), dir.join("real/b.jq")).unwrap();
        assert!(!passes_through_a_link(&dir, "real/a.jq"));
        assert!(!passes_through_a_link(&dir, "real/new.jq"), "not there yet");
        assert!(!passes_through_a_link(&dir, ""));
        assert!(passes_through_a_link(&dir, "link"));
        assert!(passes_through_a_link(&dir, "link/anything.jq"));
        assert!(passes_through_a_link(&dir, "real/b.jq"));
    }

    #[test]
    fn a_folder_is_inside_another_by_where_it_really_is() {
        let dir = ScratchDir::new("files", "inside");
        fs::create_dir_all(dir.join("a/b")).unwrap();
        assert!(is_inside(&dir.join("a/b"), &dir.join("a")));
        assert!(is_inside(&dir.join("a"), &dir.join("a")));
        assert!(!is_inside(&dir.join("a"), &dir.join("a/b")));
        assert!(!is_inside(&dir.join("a"), &dir.join("ab")));
        // Spelled differently, the same place.
        assert!(is_inside(&dir.join("a/../a/b"), &dir.join("a")));
        // Not there yet: compared as written.
        assert!(is_inside(&dir.join("new/x"), &dir.join("new")));
    }

    #[test]
    fn a_time_is_in_milliseconds() {
        let dir = ScratchDir::new("files", "time");
        fs::write(dir.join("a"), "x").unwrap();
        let ms = modified_ms(&fs::metadata(dir.join("a")).unwrap());
        // After 2020, and not in the future by more than a minute.
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(ms > 1_577_836_800_000 && ms <= now + 60_000, "{ms}");
    }
}
