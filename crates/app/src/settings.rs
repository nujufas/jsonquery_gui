//! What the user can set, and the file it is kept in between runs.
//!
//! Two kinds of thing are kept:
//!
//! * the limits on the size of files (`limits.rs`; the Settings window,
//!   `settings_window.rs`): from what size a file is kept on disk instead of
//!   being loaded, how much the Tools window takes, and so on;
//! * how the window looks (`interface.rs`): the theme, whether the query box
//!   suggests, the size of the window and of its panes, as they were left.
//!
//! The defaults are what the app has always done, and only what the user changed
//! from them is written down, so that a default that is better one day reaches
//! everyone who never touched it.
//!
//! The file is `settings.json` in the folder `.jsonquery` of the user's home
//! (`Store::standard`; `JSONQUERY_HOME` names another folder for it). A file that
//! is not there, or that cannot be read, or has something in it that cannot be
//! used, never stops the app: what cannot be used is left out, the defaults stand
//! in for it, and the Settings window says so.

mod interface;
mod limits;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub use interface::{Interface, DEFAULT_WINDOW, MIN_WINDOW};
pub use limits::{
    check_range, parse_size, size_text, FileLimits, Group, Limit, DEFAULT_TOOLS_BYTES,
};

/// The name of the folder in the user's home that the app keeps its settings in.
const FOLDER: &str = ".jsonquery";

/// The name of the file in that folder.
const FILE: &str = "settings.json";

/// The variable that names the folder to keep the settings in, instead of
/// `.jsonquery` in the home: one that is a path from the root.
pub const HOME_VARIABLE: &str = "JSONQUERY_HOME";

/// Everything the user can set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Settings {
    pub limits: FileLimits,
    pub interface: Interface,
}

impl Settings {
    /// What `settings.json` holds for these: only what is not what the app has
    /// always used.
    pub fn to_json(self) -> Value {
        let mut root = Map::new();
        root.insert("limits".to_owned(), Value::Object(self.limits.to_json()));
        self.interface.write(&mut root);
        Value::Object(root)
    }

    /// The settings that `value`, the contents of a `settings.json`, stands for,
    /// and what was wrong with it. What is not there is the default; what cannot
    /// be used is left as the default and said; what is not known (from a newer
    /// version) is left alone.
    pub fn read(value: &Value) -> (Settings, Vec<String>) {
        let mut notes = Vec::new();
        let limits = FileLimits::read(value.get("limits"), &mut notes);
        let interface = Interface::read(value, &mut notes);
        (Settings { limits, interface }, notes)
    }
}

/// Where the settings are kept, and what went wrong with them. Nothing here
/// stops the app: a file that cannot be read gives the defaults, one that cannot
/// be written is said so in the Settings window and the settings are kept for as
/// long as the app runs.
#[derive(Debug)]
pub struct Store {
    path: Option<PathBuf>,
    /// What was wrong with the file when it was read.
    notes: Vec<String>,
    /// Why the last save failed.
    error: Option<String>,
}

impl Store {
    /// Settings kept in `path`; none, if there is nowhere to keep them.
    pub fn at(path: Option<PathBuf>) -> Self {
        Self {
            path,
            notes: Vec::new(),
            error: None,
        }
    }

    /// Nothing is read or written: what the tests use.
    #[cfg(test)]
    pub fn none() -> Self {
        Self::at(None)
    }

    /// The user's own: `settings.json` in `.jsonquery` in their home.
    pub fn standard() -> Self {
        Self::at(standard_path())
    }

    /// The file the settings are in, if there is one.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// What was wrong with the file when it was read: what was left out and why.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Why the settings could not be written, the last time they were tried.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Read the settings: what the file says, or the defaults for all that it
    /// does not (or cannot) say.
    pub fn load(&mut self) -> Settings {
        self.notes.clear();
        let Some(path) = &self.path else {
            return Settings::default();
        };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                self.notes.push(format!(
                    "{} could not be read ({e}): the defaults are used",
                    path.display()
                ));
                return Settings::default();
            }
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => {
                let (settings, notes) = Settings::read(&value);
                self.notes = notes;
                settings
            }
            Err(e) => {
                self.notes.push(format!(
                    "{} is not JSON ({e}): the defaults are used",
                    path.display()
                ));
                Settings::default()
            }
        }
    }

    /// Write `settings`. A failure is kept for [`Self::error`].
    pub fn save(&mut self, settings: &Settings) {
        self.error = None;
        let Some(path) = &self.path else { return };
        if let Err(e) = write_file(path, &settings.to_json()) {
            self.error = Some(format!("Could not save to {}: {e}", path.display()));
        }
    }
}

/// Write `value` to `path` in one step: to a file beside it that then takes its
/// place, so that a file that was there is never left half written.
fn write_file(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    text.push('\n');
    let mut beside = path.as_os_str().to_owned();
    beside.push(".tmp");
    let beside = PathBuf::from(beside);
    std::fs::write(&beside, text)?;
    std::fs::rename(&beside, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&beside);
    })
}

/// `settings.json` in the user's folder for it, where there is one.
fn standard_path() -> Option<PathBuf> {
    settings_path(|name| std::env::var_os(name), cfg!(windows))
}

/// The file the settings are kept in: `.jsonquery/settings.json` in the home —
/// `HOME`, or on Windows `USERPROFILE` (or the drive and path of the home, which
/// older systems have instead) — unless [`HOME_VARIABLE`] names the folder. A
/// folder that is not a path from the root is not one to go by. `var` reads the
/// environment.
fn settings_path(var: impl Fn(&str) -> Option<OsString>, windows: bool) -> Option<PathBuf> {
    let absolute = |name: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    if let Some(folder) = absolute(HOME_VARIABLE) {
        return Some(folder.join(FILE));
    }
    let home = if windows {
        absolute("USERPROFILE").or_else(|| {
            let mut path = var("HOMEDRIVE").filter(|v| !v.is_empty())?;
            path.push(var("HOMEPATH").filter(|v| !v.is_empty())?);
            Some(PathBuf::from(path)).filter(|path| path.is_absolute())
        })?
    } else {
        absolute("HOME")?
    };
    Some(home.join(FOLDER).join(FILE))
}

#[cfg(test)]
mod tests {
    use super::limits::{GB, MB};
    use super::*;
    use serde_json::json;

    /// A folder of its own, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "jsonquery-settings-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> PathBuf {
            self.0.join(FOLDER).join(FILE)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_defaults_write_nothing_but_the_place_the_limits_go() {
        assert_eq!(Settings::default().to_json(), json!({"limits": {}}));
        let (settings, notes) = Settings::read(&json!({"limits": {}}));
        assert_eq!(settings, Settings::default());
        assert!(notes.is_empty());
    }

    #[test]
    fn the_limits_and_the_interface_are_in_one_file() {
        let mut settings = Settings::default();
        settings.limits.set(Limit::KeepOnDisk, 512 * MB);
        settings.interface.dark = false;
        settings.interface.window = Some([1400.0, 900.0]);
        let json = settings.to_json();
        assert_eq!(
            json,
            json!({
                "limits": {"keep_on_disk_from": "512 MB"},
                "theme": "light",
                "window": {"width": 1400.0, "height": 900.0}
            })
        );
        let (read, notes) = Settings::read(&json);
        assert_eq!(read, settings);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn what_is_wrong_in_either_part_is_said_and_the_rest_is_used() {
        let (settings, notes) = Settings::read(&json!({
            "limits": {"copy": "banana", "tools": "64 MB"},
            "theme": "purple",
            "autocomplete": true
        }));
        assert_eq!(settings.limits.tools(), 64 * 1024 * 1024);
        assert!(settings.limits.is_default(Limit::Copy));
        assert!(settings.interface.dark, "purple is not a theme");
        assert!(settings.interface.autocomplete);
        assert_eq!(notes.len(), 2, "{notes:#?}");
        assert!(notes.iter().any(|n| n.starts_with("copy: ")));
        assert!(notes.iter().any(|n| n.starts_with("theme: ")));
    }

    #[test]
    fn settings_are_kept_between_runs() {
        let dir = Scratch::new("round-trip");
        let mut store = Store::at(Some(dir.path()));
        assert_eq!(store.load(), Settings::default(), "no file yet");
        assert!(store.notes().is_empty() && store.error().is_none());

        let mut settings = Settings::default();
        settings.limits.set(Limit::KeepOnDisk, 64 * MB);
        settings.limits.set(Limit::Tools, 2 * GB);
        settings.interface.dark = false;
        store.save(&settings);
        assert!(store.error().is_none(), "{:?}", store.error());

        let text = std::fs::read_to_string(dir.path()).unwrap();
        assert!(text.ends_with('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({
                "limits": {"keep_on_disk_from": "64 MB", "tools": "2 GB"},
                "theme": "light"
            })
        );
        assert!(
            !dir.path().with_extension("json.tmp").exists(),
            "nothing is left beside it"
        );

        // Another run.
        let mut again = Store::at(Some(dir.path()));
        assert_eq!(again.load(), settings);
        assert!(again.notes().is_empty());
    }

    #[test]
    fn the_defaults_come_back_when_they_are_set_again() {
        let dir = Scratch::new("restore");
        let mut store = Store::at(Some(dir.path()));
        let mut settings = Settings::default();
        settings.limits.set(Limit::Copy, GB);
        settings.interface.dark = false;
        store.save(&settings);
        store.save(&Settings::default());
        assert_eq!(Store::at(Some(dir.path())).load(), Settings::default());
        let text = std::fs::read_to_string(dir.path()).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({"limits": {}})
        );
    }

    #[test]
    fn a_file_that_is_not_json_gives_the_defaults_and_is_left_alone() {
        let dir = Scratch::new("garbage");
        std::fs::create_dir_all(dir.path().parent().unwrap()).unwrap();
        std::fs::write(dir.path(), "{ this is not JSON").unwrap();
        let mut store = Store::at(Some(dir.path()));
        assert_eq!(store.load(), Settings::default());
        assert_eq!(store.notes().len(), 1);
        assert!(
            store.notes()[0].contains("is not JSON") && store.notes()[0].contains("defaults"),
            "{:?}",
            store.notes()
        );
        assert_eq!(
            std::fs::read_to_string(dir.path()).unwrap(),
            "{ this is not JSON",
            "reading it does not touch it"
        );
    }

    #[test]
    fn a_file_that_is_not_what_was_expected_gives_the_defaults() {
        for value in [json!(null), json!([]), json!("limits"), json!({}), json!(7)] {
            let (settings, _) = Settings::read(&value);
            assert_eq!(settings, Settings::default(), "{value}");
        }
    }

    #[test]
    fn a_file_that_cannot_be_read_gives_the_defaults_and_says_so() {
        // A directory where the file should be.
        let dir = Scratch::new("unreadable");
        std::fs::create_dir_all(dir.path()).unwrap();
        let mut store = Store::at(Some(dir.path()));
        assert_eq!(store.load(), Settings::default());
        assert!(
            store.notes()[0].contains("could not be read"),
            "{:?}",
            store.notes()
        );
    }

    #[test]
    fn a_file_that_cannot_be_written_is_an_error_to_show_and_not_a_panic() {
        let dir = Scratch::new("unwritable");
        // A file where the folder should be.
        std::fs::write(dir.0.join(FOLDER), "in the way").unwrap();
        let mut store = Store::at(Some(dir.path()));
        store.save(&Settings::default());
        let error = store.error().expect("it could not be saved");
        assert!(error.starts_with("Could not save to "), "{error}");
        // The next save that works clears it.
        std::fs::remove_file(dir.0.join(FOLDER)).unwrap();
        store.save(&Settings::default());
        assert!(store.error().is_none());
    }

    #[test]
    fn with_no_place_to_keep_them_nothing_is_read_or_written() {
        let mut store = Store::none();
        assert_eq!(store.path(), None);
        let mut settings = Settings::default();
        settings.limits.set(Limit::Copy, GB);
        store.save(&settings);
        assert!(store.error().is_none());
        assert_eq!(store.load(), Settings::default());
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    /// A path from the root as the platform has it: on Windows that is one
    /// where the drive is named, which `/home/a` is not.
    fn rooted(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", path.replace('/', "\\"))
        } else {
            path.to_string()
        }
    }

    #[test]
    fn settings_are_in_the_folder_dot_jsonquery_of_the_home() {
        let file = |windows, pairs: &[(&str, &str)]| settings_path(env(pairs), windows);
        let tail = PathBuf::from(".jsonquery").join("settings.json");
        let home = rooted("/home/a");

        assert_eq!(
            file(false, &[("HOME", &home)]),
            Some(PathBuf::from(&home).join(&tail))
        );
        // Nowhere that is not a path from the root, nor with no home.
        assert_eq!(file(false, &[]), None);
        assert_eq!(file(false, &[("HOME", "")]), None);
        assert_eq!(file(false, &[("HOME", "relative/home")]), None);
        // XDG's folder is not used: it is the folder of the home that is meant.
        assert_eq!(
            file(false, &[("XDG_CONFIG_HOME", "/cfg"), ("HOME", &home)]),
            Some(PathBuf::from(&home).join(&tail))
        );
    }

    #[test]
    fn on_windows_the_home_is_the_profile() {
        let file = |pairs: &[(&str, &str)]| settings_path(env(pairs), true);
        let tail = PathBuf::from(".jsonquery").join("settings.json");
        // (A path is one from the root where the drive is named, but only on
        // Windows; what these tests can know is the order they are tried in.)
        let profile = if cfg!(windows) {
            "C:\\Users\\a"
        } else {
            "/Users/a"
        };
        assert_eq!(
            file(&[("USERPROFILE", profile), ("HOME", "/other")]),
            Some(PathBuf::from(profile).join(&tail)),
            "the profile, not HOME"
        );
        assert_eq!(file(&[("HOME", "/other")]), None, "HOME is not asked there");
        assert_eq!(file(&[]), None);
        if cfg!(windows) {
            assert_eq!(
                file(&[("HOMEDRIVE", "D:"), ("HOMEPATH", "\\Users\\b")]),
                Some(PathBuf::from("D:\\Users\\b").join(&tail)),
                "the drive and path, where there is no profile"
            );
        }
    }

    #[test]
    fn a_folder_named_for_the_settings_takes_the_place_of_the_home() {
        let file = |pairs: &[(&str, &str)]| settings_path(env(pairs), false);
        let (folder, home) = (rooted("/data/jq"), rooted("/home/a"));
        assert_eq!(
            file(&[(HOME_VARIABLE, &folder), ("HOME", &home)]),
            Some(PathBuf::from(&folder).join("settings.json")),
            "in the folder itself, not in a .jsonquery inside it"
        );
        assert_eq!(
            file(&[(HOME_VARIABLE, &folder)]),
            Some(PathBuf::from(&folder).join("settings.json")),
            "and a home is not needed then"
        );
        // One that is not a path from the root, or empty, is as good as not set.
        for odd in ["", "relative", "./here"] {
            assert_eq!(
                file(&[(HOME_VARIABLE, odd), ("HOME", &home)]),
                Some(
                    PathBuf::from(&home)
                        .join(".jsonquery")
                        .join("settings.json")
                ),
                "{odd:?}"
            );
        }
    }
}
