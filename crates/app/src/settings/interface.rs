//! How the window looks and where its parts are, kept from one run to the next:
//! the theme, whether the query box suggests as you type, the size of the window
//! and the sizes of its panes.
//!
//! Like the limits, only what differs from what the app starts with is written
//! down: a size is there once the user has changed it, and not before.

use std::ops::RangeInclusive;

use serde_json::{Map, Number, Value};

use crate::query_suggest::QuerySuggest;

/// The size of the main window, in points, the first time the app starts.
pub const DEFAULT_WINDOW: [f32; 2] = [1200.0, 800.0];

/// The smallest the main window can be made, in points.
pub const MIN_WINDOW: [f32; 2] = [640.0, 420.0];

/// What a saved width or height of the window may be. Beyond the largest there is
/// no screen, and it is a number that something else has put there.
const WINDOW_SIDE: RangeInclusive<f32> = 120.0..=16_384.0;

/// What a saved height of the query panel may be, in points.
const QUERY_HEIGHT: RangeInclusive<f32> = 40.0..=4_000.0;

/// What a saved share of the width for Source (against Results) may be.
const SOURCE_SHARE: RangeInclusive<f32> = 0.05..=0.95;

/// What the user sees of the window, as it was left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interface {
    /// The dark theme, which the app starts in; the light one if not.
    pub dark: bool,
    /// The query box suggests as you type (💡).
    pub autocomplete: bool,
    /// The size (width, height, in points) the window had when it was not
    /// maximized; none while it is the one the app starts with.
    pub window: Option<[f32; 2]>,
    /// The window was maximized.
    pub maximized: bool,
    /// The height of the query panel, if the user dragged it from the one the app
    /// starts with.
    pub query_height: Option<f32>,
    /// How much of the width Source and Results share Source takes, from 0 to 1,
    /// if the user dragged it from a half.
    pub source_share: Option<f32>,
}

impl Default for Interface {
    fn default() -> Self {
        Self {
            dark: true,
            autocomplete: QuerySuggest::default().enabled,
            window: None,
            maximized: false,
            query_height: None,
            source_share: None,
        }
    }
}

impl Interface {
    /// The size the main window opens with: the one it had, but no smaller than
    /// it can be made.
    pub fn window_size(&self) -> [f32; 2] {
        let [width, height] = self.window.unwrap_or(DEFAULT_WINDOW);
        [width.max(MIN_WINDOW[0]), height.max(MIN_WINDOW[1])]
    }

    /// Put what differs from the default into `root`, the object `settings.json`
    /// is.
    pub fn write(&self, root: &mut Map<String, Value>) {
        let default = Self::default();
        if self.dark != default.dark {
            root.insert("theme".to_owned(), Value::from("light"));
        }
        if self.autocomplete != default.autocomplete {
            root.insert("autocomplete".to_owned(), Value::from(self.autocomplete));
        }

        let mut window = Map::new();
        if let Some([width, height]) = self.window {
            window.insert("width".to_owned(), number(width));
            window.insert("height".to_owned(), number(height));
        }
        if self.maximized {
            window.insert("maximized".to_owned(), Value::from(true));
        }
        if !window.is_empty() {
            root.insert("window".to_owned(), Value::Object(window));
        }

        let mut panes = Map::new();
        if let Some(height) = self.query_height {
            panes.insert("query_height".to_owned(), number(height));
        }
        if let Some(share) = self.source_share {
            panes.insert("source_share".to_owned(), number(share));
        }
        if !panes.is_empty() {
            root.insert("panes".to_owned(), Value::Object(panes));
        }
    }

    /// What `root`, the contents of a `settings.json`, says of the interface; what
    /// is not there is the default, and what cannot be used (with the reason in
    /// `notes`) is left as it.
    pub fn read(root: &Value, notes: &mut Vec<String>) -> Self {
        let mut interface = Self::default();

        match root.get("theme") {
            None => {}
            Some(Value::String(name)) if name.eq_ignore_ascii_case("dark") => {}
            Some(Value::String(name)) if name.eq_ignore_ascii_case("light") => {
                interface.dark = false;
            }
            Some(other) => notes.push(format!(
                "theme: {other} is not \"dark\" or \"light\", so dark is used"
            )),
        }
        match root.get("autocomplete") {
            None => {}
            Some(Value::Bool(on)) => interface.autocomplete = *on,
            Some(other) => notes.push(format!(
                "autocomplete: {other} is not true or false, so the default is used"
            )),
        }

        if let Some(window) = root.get("window") {
            if let Some(window) = object(window, "window", notes) {
                let width = side(window.get("width"), "window.width", notes);
                let height = side(window.get("height"), "window.height", notes);
                match (width, height) {
                    (Some(width), Some(height)) => interface.window = Some([width, height]),
                    _ if window.contains_key("width") != window.contains_key("height") => {
                        notes.push(
                            "window needs both a width and a height, so its size is left out"
                                .to_owned(),
                        );
                    }
                    _ => {}
                }
                match window.get("maximized") {
                    None => {}
                    Some(Value::Bool(on)) => interface.maximized = *on,
                    Some(other) => notes.push(format!(
                        "window.maximized: {other} is not true or false, so it is not"
                    )),
                }
            }
        }

        if let Some(panes) = root.get("panes") {
            if let Some(panes) = object(panes, "panes", notes) {
                interface.query_height = ranged(
                    panes.get("query_height"),
                    "panes.query_height",
                    &QUERY_HEIGHT,
                    notes,
                );
                interface.source_share = ranged(
                    panes.get("source_share"),
                    "panes.source_share",
                    &SOURCE_SHARE,
                    notes,
                );
            }
        }
        interface
    }
}

/// A number as `settings.json` has it: to hundredths, which is more than a
/// size on a screen needs and keeps `0.42` from being `0.41999998`.
fn number(value: f32) -> Value {
    let hundredths = (f64::from(value) * 100.0).round() / 100.0;
    Number::from_f64(hundredths).map_or(Value::Null, Value::Number)
}

fn object<'a>(
    value: &'a Value,
    name: &str,
    notes: &mut Vec<String>,
) -> Option<&'a Map<String, Value>> {
    let found = value.as_object();
    if found.is_none() {
        notes.push(format!("{name} is not an object, so it is left out"));
    }
    found
}

/// A width or a height of the window that `value` gives, if it gives one.
fn side(value: Option<&Value>, name: &str, notes: &mut Vec<String>) -> Option<f32> {
    ranged(value, name, &WINDOW_SIDE, notes)
}

/// The number that `value` is, if it is one in `range`; if there is a value and
/// it is not, say so.
fn ranged(
    value: Option<&Value>,
    name: &str,
    range: &RangeInclusive<f32>,
    notes: &mut Vec<String>,
) -> Option<f32> {
    let value = value?;
    let found = value
        .as_f64()
        .map(|n| n as f32)
        .filter(|n| range.contains(n));
    if found.is_none() {
        notes.push(format!(
            "{name}: {value} is not a number from {} to {}, so it is left out",
            range.start(),
            range.end()
        ));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn written(interface: &Interface) -> Value {
        let mut root = Map::new();
        interface.write(&mut root);
        Value::Object(root)
    }

    fn read(value: Value) -> (Interface, Vec<String>) {
        let mut notes = Vec::new();
        let interface = Interface::read(&value, &mut notes);
        (interface, notes)
    }

    #[test]
    fn the_app_starts_dark_with_suggestions_as_it_has_been_and_the_window_it_has_been() {
        let interface = Interface::default();
        assert!(interface.dark);
        assert!(!interface.autocomplete, "experimental, so off");
        assert_eq!(interface.window_size(), [1200.0, 800.0]);
        assert!(!interface.maximized);
        assert_eq!(
            (interface.query_height, interface.source_share),
            (None, None)
        );
        assert_eq!(written(&interface), json!({}), "nothing to write down");
    }

    #[test]
    fn only_what_was_changed_is_written() {
        let mut interface = Interface {
            dark: false,
            ..Default::default()
        };
        assert_eq!(written(&interface), json!({"theme": "light"}));
        interface.dark = true;
        interface.autocomplete = true;
        assert_eq!(written(&interface), json!({"autocomplete": true}));
        interface.autocomplete = false;
        interface.window = Some([1400.0, 900.0]);
        assert_eq!(
            written(&interface),
            json!({"window": {"width": 1400.0, "height": 900.0}})
        );
        interface.maximized = true;
        assert_eq!(
            written(&interface),
            json!({"window": {"width": 1400.0, "height": 900.0, "maximized": true}})
        );
        interface.window = None;
        assert_eq!(written(&interface), json!({"window": {"maximized": true}}));
        interface.maximized = false;
        interface.query_height = Some(140.5);
        interface.source_share = Some(0.42);
        assert_eq!(
            written(&interface),
            json!({"panes": {"query_height": 140.5, "source_share": 0.42}})
        );
    }

    #[test]
    fn what_is_written_is_what_is_read() {
        let interface = Interface {
            dark: false,
            autocomplete: true,
            window: Some([1366.0, 768.0]),
            maximized: true,
            query_height: Some(150.0),
            source_share: Some(0.35),
        };
        let (read, notes) = read(written(&interface));
        assert_eq!(read, interface);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_number_is_kept_to_hundredths_so_that_it_reads_as_it_was_typed_by_the_eye() {
        // 0.42 in an f32 is 0.4199999868…: not what a person who opens the file
        // wants to find in it.
        let interface = Interface {
            source_share: Some(0.42),
            ..Default::default()
        };
        let text = serde_json::to_string(&written(&interface)).unwrap();
        assert!(text.contains("0.42") && !text.contains("0.419"), "{text}");
    }

    #[test]
    fn the_theme_is_dark_or_light_in_any_case() {
        assert!(read(json!({"theme": "dark"})).0.dark);
        assert!(!read(json!({"theme": "light"})).0.dark);
        assert!(!read(json!({"theme": "LIGHT"})).0.dark);
        let (interface, notes) = read(json!({"theme": "purple"}));
        assert!(interface.dark);
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].contains("theme") && notes[0].contains("purple"),
            "{notes:?}"
        );
        let (_, notes) = read(json!({"theme": 3}));
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn autocomplete_is_true_or_false() {
        assert!(read(json!({"autocomplete": true})).0.autocomplete);
        assert!(!read(json!({"autocomplete": false})).0.autocomplete);
        let (interface, notes) = read(json!({"autocomplete": "yes"}));
        assert_eq!(interface.autocomplete, Interface::default().autocomplete);
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn a_size_that_is_no_size_of_a_window_is_left_out_and_said() {
        for window in [
            json!({"width": 1400, "height": "tall"}),
            json!({"width": 5, "height": 900}),
            json!({"width": 1400, "height": 99999}),
            json!({"width": -1400, "height": 900}),
            json!({"width": 1400}),
        ] {
            let (interface, notes) = read(json!({"window": window}));
            assert_eq!(interface.window, None, "{window}");
            assert!(
                interface.window_size() == DEFAULT_WINDOW,
                "the start is what it was: {window}"
            );
            assert!(!notes.is_empty(), "{window}: said why");
        }
        let (interface, notes) = read(json!({"window": [1, 2]}));
        assert_eq!(interface.window, None);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("window is not an object"), "{notes:?}");
    }

    #[test]
    fn a_window_may_be_maximized_without_a_size_and_a_size_without_being_maximized() {
        let (interface, notes) = read(json!({"window": {"maximized": true}}));
        assert!(interface.maximized && interface.window.is_none());
        assert!(notes.is_empty());
        let (interface, _) = read(json!({"window": {"width": 800, "height": 600}}));
        assert!(!interface.maximized);
        assert_eq!(interface.window, Some([800.0, 600.0]));
        let (interface, notes) = read(json!({"window": {"maximized": "yes"}}));
        assert!(!interface.maximized);
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn the_sizes_of_the_panes_are_held_to_what_a_window_has_room_for() {
        let (interface, notes) = read(json!({"panes": {"query_height": 120, "source_share": 0.3}}));
        assert_eq!(interface.query_height, Some(120.0));
        assert_eq!(interface.source_share, Some(0.3));
        assert!(notes.is_empty());
        for panes in [
            json!({"query_height": 0}),
            json!({"query_height": 100000}),
            json!({"query_height": "tall"}),
            json!({"source_share": 0}),
            json!({"source_share": 1}),
            json!({"source_share": 40}),
        ] {
            let (interface, notes) = read(json!({"panes": panes}));
            assert_eq!(
                (interface.query_height, interface.source_share),
                (None, None),
                "{panes}"
            );
            assert_eq!(notes.len(), 1, "{panes}: {notes:?}");
        }
        let (_, notes) = read(json!({"panes": 7}));
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn a_file_that_says_nothing_of_the_interface_leaves_it_as_it_starts() {
        for value in [json!({}), json!({"limits": {}}), json!(null), json!([1])] {
            let (interface, notes) = read(value);
            assert_eq!(interface, Interface::default());
            assert!(notes.is_empty(), "{notes:?}");
        }
    }
}
