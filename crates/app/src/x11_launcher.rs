//! The X11 launcher script, handed out from the About window on a Wayland
//! desktop.
//!
//! `winit` has no file drop on native Wayland, but under X11 (XWayland, which a
//! Wayland desktop provides) there is one. `scripts/jsonquery-x11.sh` puts a
//! "jsonquery (X11)" entry in the application menu that starts the program that
//! way. The script is carried inside the program, so every kind of install can
//! offer it: the tar.gz, the AppImage, the snap and the Arch package alike.

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// `scripts/jsonquery-x11.sh`, exactly as it is in the repository.
const SCRIPT: &str = include_str!("../../../scripts/jsonquery-x11.sh");

/// What the script is called when it is saved, as in the README.
pub const FILE_NAME: &str = "jsonquery-x11.sh";

/// Whether this process is a native Wayland client — the one case in which
/// dropping files on its window does not work. `winit` chooses Wayland when
/// `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set (and not empty), X11 otherwise,
/// so asking the same variables gives its answer. A Wayland *session* is not
/// enough: started through the "jsonquery (X11)" entry, or inside a snap, the
/// program runs under X11 there and files can be dropped on it.
pub fn on_native_wayland() -> bool {
    native_wayland(
        env::var_os("WAYLAND_DISPLAY"),
        env::var_os("WAYLAND_SOCKET"),
    )
}

fn native_wayland(display: Option<OsString>, socket: Option<OsString>) -> bool {
    [display, socket]
        .into_iter()
        .flatten()
        .any(|value| !value.is_empty())
}

/// Where a "save as" dialog for the script should start: the Downloads folder,
/// if there is one.
pub fn downloads_dir() -> Option<PathBuf> {
    let home = env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join("Downloads")).filter(|dir| dir.is_dir())
}

/// Write the script to `path`, replacing what is there, and make it
/// executable where there is such a thing. The executable bit is a courtesy
/// (`run_command` runs the script through `bash`), so a file that refuses it
/// is still saved: the snap's file dialog hands out paths in the document
/// portal's filesystem, which doesn't take a `chmod`.
pub fn save_script(path: &Path) -> io::Result<()> {
    std::fs::write(path, SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// The command that sets the saved script up, for a terminal. The script finds
/// the program by itself when it sits beside it, is on the `PATH` or is the
/// snap; a program kept anywhere else (the tar.gz unpacked in some folder, an
/// AppImage in `~/Apps`) is named, which the script accepts last on its
/// command line.
pub fn run_command(script: &Path) -> String {
    let mut command = format!("bash {}", shell_quote(&script.to_string_lossy()));
    if let Some(program) = program_path() {
        command.push(' ');
        command.push_str(&shell_quote(&program.to_string_lossy()));
    }
    command
}

/// The program that is running, as the script should be told to start it —
/// `None` where the script finds it by itself. An AppImage is its own file, not
/// the copy of the program mounted from it; the snap's program is at a path that
/// changes with every revision, where the script knows `/snap/bin/jsonquery-gui`.
fn program_path() -> Option<PathBuf> {
    if let Some(image) = env::var_os("APPIMAGE").filter(|image| !image.is_empty()) {
        return Some(PathBuf::from(image));
    }
    if env::var_os("SNAP").is_some() {
        return None;
    }
    env::current_exe().ok()
}

/// `text` as one word of a shell command line: left as it is when it holds
/// nothing the shell would read, in single quotes otherwise.
fn shell_quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:,@%~".contains(c));
    if plain {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    #[test]
    fn a_wayland_display_or_socket_makes_a_native_wayland_client() {
        assert!(native_wayland(os("wayland-0"), None));
        assert!(native_wayland(None, os("3")));
        assert!(native_wayland(os(""), os("3")));
    }

    #[test]
    fn nothing_or_empty_variables_are_not_wayland() {
        assert!(!native_wayland(None, None));
        assert!(!native_wayland(os(""), os("")));
    }

    #[test]
    fn the_embedded_script_is_the_one_the_readme_names() {
        assert!(SCRIPT.starts_with("#!/usr/bin/env bash\n# jsonquery-x11.sh"));
        assert!(SCRIPT.contains("ID=jsonquery-gui-x11"));
    }

    #[test]
    fn a_plain_path_is_left_alone_and_others_are_quoted() {
        assert_eq!(
            shell_quote("/home/u/Downloads/a.sh"),
            "/home/u/Downloads/a.sh"
        );
        assert_eq!(
            shell_quote("/home/u/My Apps/a.sh"),
            "'/home/u/My Apps/a.sh'"
        );
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("$HOME/x"), "'$HOME/x'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn a_path_with_backslashes_is_quoted_in_the_command() {
        // As a path on Windows is: the shell would eat the backslashes.
        let command = run_command(Path::new(r"C:\Users\a\jsonquery-x11.sh"));
        assert!(
            command.starts_with(r"bash 'C:\Users\a\jsonquery-x11.sh'"),
            "{command}"
        );
    }

    #[test]
    fn the_saved_script_is_the_embedded_one_and_executable() {
        let dir = std::env::temp_dir().join(format!("jq-x11-script-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        save_script(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SCRIPT);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
