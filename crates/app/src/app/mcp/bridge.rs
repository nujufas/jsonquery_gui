//! How an assistant that starts its MCP servers itself (Claude Desktop, QuickAI's
//! llama-server) is to start `jsonquery-mcp`, the program that takes its calls to the
//! app. That depends on how JsonQuery was installed: the program beside the app, a
//! package manager's link to it on the `PATH`, the snap's command for it, or the
//! AppImage itself with `mcp` first (its AppRun then starts the program inside).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What starts `jsonquery-mcp`: a program, and what it is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Bridge {
    pub(super) command: PathBuf,
    pub(super) args: Vec<String>,
}

/// The program's name on this system.
pub(super) const FILE_NAME: &str = if cfg!(windows) {
    "jsonquery-mcp.exe"
} else {
    "jsonquery-mcp"
};

/// What starts `jsonquery-mcp` for the app that runs, if the program is beside it.
/// Looked for once: where the app was installed does not change while it runs.
pub(super) fn bridge() -> Option<&'static Bridge> {
    static FOUND: OnceLock<Option<Bridge>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            find(&exe, |name| std::env::var_os(name))
        })
        .as_ref()
}

/// [`bridge`], for the app's executable `exe` and the environment `var`.
fn find(exe: &Path, var: impl Fn(&str) -> Option<OsString>) -> Option<Bridge> {
    let program = exe.with_file_name(FILE_NAME);
    if !program.is_file() {
        return None;
    }
    let set = |name: &str| var(name).filter(|value| !value.is_empty());
    let runs_from = |name: &str| {
        set(name).is_some_and(|root| {
            exe.starts_with(&root)
                || std::fs::canonicalize(&root).is_ok_and(|root| exe.starts_with(root))
        })
    };
    // An AppImage runs from a mount that is gone once it ends: the AppImage stays.
    if runs_from("APPDIR") {
        if let Some(image) = set("APPIMAGE") {
            return Some(Bridge {
                command: image.into(),
                args: vec!["mcp".to_owned()],
            });
        }
    }
    // The snap's folder changes with each revision, and the program run from there
    // would not have the snap's home, where mcp.json is: the snap's command has both.
    if runs_from("SNAP") {
        if let Some(name) = set("SNAP_INSTANCE_NAME").or_else(|| set("SNAP_NAME")) {
            return Some(Bridge {
                command: format!("/snap/bin/{}.jsonquery-mcp", name.to_string_lossy()).into(),
                args: Vec::new(),
            });
        }
    }
    let command = set("PATH")
        .and_then(|path| link_on(&path, &program))
        .unwrap_or(program);
    Some(Bridge {
        command,
        args: Vec::new(),
    })
}

/// The first `jsonquery-mcp` in the folders of `path` that is `program`, or a link to
/// it: what a package manager puts on the `PATH` (Homebrew's `bin`, winget's `Links`)
/// stays where it is when an upgrade puts the program in a folder of the new version.
fn link_on(path: &OsString, program: &Path) -> Option<PathBuf> {
    let real = program.canonicalize().ok()?;
    std::env::split_paths(path)
        .map(|folder| folder.join(FILE_NAME))
        .find(|candidate| candidate.canonicalize().is_ok_and(|c| c == real))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch_dir::ScratchDir;

    /// An install: `<dir>/<under>/jsonquery_gui`, with `jsonquery-mcp` beside it if
    /// `bridge`; the app's path.
    fn install(dir: &Path, under: &str, bridge: bool) -> PathBuf {
        let folder = dir.join(under);
        std::fs::create_dir_all(&folder).unwrap();
        if bridge {
            std::fs::write(folder.join(FILE_NAME), "").unwrap();
        }
        folder.join("jsonquery_gui")
    }

    fn env<'a>(pairs: &'a [(&'a str, OsString)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        |name| {
            pairs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
        }
    }

    fn plain(command: PathBuf) -> Option<Bridge> {
        Some(Bridge {
            command,
            args: Vec::new(),
        })
    }

    #[test]
    fn the_program_beside_the_app_or_none_without_it() {
        let dir = ScratchDir::new("mcp-bridge", "beside");
        let exe = install(&dir, "a", true);
        assert_eq!(find(&exe, env(&[])), plain(exe.with_file_name(FILE_NAME)));
        let alone = install(&dir, "b", false);
        assert_eq!(find(&alone, env(&[])), None);
        // Neither the AppImage nor the snap gives one the app does not have.
        let image = [
            ("APPDIR", dir.join("b").into_os_string()),
            ("APPIMAGE", OsString::from("/home/u/jq.AppImage")),
        ];
        assert_eq!(find(&alone, env(&image)), None);
    }

    #[test]
    fn an_appimage_is_started_with_mcp_first() {
        let dir = ScratchDir::new("mcp-bridge", "appimage");
        let exe = install(&dir, "mount/usr/bin", true);
        let image = OsString::from("/home/u/Apps/jsonquery_gui-1.0.0-x86_64.AppImage");
        let vars = [
            ("APPDIR", dir.join("mount").into_os_string()),
            ("APPIMAGE", image.clone()),
        ];
        assert_eq!(
            find(&exe, env(&vars)),
            Some(Bridge {
                command: image.into(),
                args: vec!["mcp".to_owned()],
            })
        );
        // APPIMAGE handed down to an app that does not run from it (started from a
        // terminal that one opened) is not taken.
        let other = install(&dir, "elsewhere", true);
        assert_eq!(
            find(&other, env(&vars)),
            plain(other.with_file_name(FILE_NAME))
        );
    }

    #[test]
    fn a_snap_is_started_by_its_command() {
        let dir = ScratchDir::new("mcp-bridge", "snap");
        let exe = install(&dir, "snap/jsonquery-gui/12/bin", true);
        let snap = dir.join("snap/jsonquery-gui/12").into_os_string();
        let vars = [
            ("SNAP", snap.clone()),
            ("SNAP_NAME", OsString::from("jsonquery-gui")),
        ];
        assert_eq!(
            find(&exe, env(&vars)),
            plain("/snap/bin/jsonquery-gui.jsonquery-mcp".into())
        );
        // A second instance of the snap has a name of its own.
        let vars = [
            ("SNAP", snap),
            ("SNAP_NAME", OsString::from("jsonquery-gui")),
            ("SNAP_INSTANCE_NAME", OsString::from("jsonquery-gui_beta")),
        ];
        assert_eq!(
            find(&exe, env(&vars)),
            plain("/snap/bin/jsonquery-gui_beta.jsonquery-mcp".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_on_the_path_to_the_same_program_is_taken_over_its_versioned_folder() {
        let dir = ScratchDir::new("mcp-bridge", "path");
        let exe = install(&dir, "Cellar/jsonquery-gui/1.0.0/bin", true);
        let program = exe.with_file_name(FILE_NAME);
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(&program, bin.join(FILE_NAME)).unwrap();
        // Another jsonquery-mcp earlier on the PATH is another program: passed over.
        let other = install(&dir, "other", true).with_file_name(FILE_NAME);
        let path = std::env::join_paths([
            dir.join("nothing"),
            other.parent().unwrap().into(),
            bin.clone(),
        ])
        .unwrap();
        assert_eq!(
            find(&exe, env(&[("PATH", path)])),
            plain(bin.join(FILE_NAME))
        );
        let path = std::env::join_paths([other.parent().unwrap()]).unwrap();
        assert_eq!(find(&exe, env(&[("PATH", path)])), plain(program));
    }
}
