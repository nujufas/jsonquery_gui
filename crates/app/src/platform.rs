//! Where a desktop window and a phone differ, behind one small interface.
//!
//! The app is written against [`Platform`]; a shell installs its own
//! implementation with [`install`] before opening the window. With no shell
//! (the desktop binary) the native `rfd` dialogs are used and every other
//! method keeps its no-op default, so desktop behaviour is exactly what it
//! was before this seam existed.
//!
//! Almost everything here is asynchronous-friendly: a phone's file picker is
//! another app, whose answer arrives later, so pickers hand back a
//! [`PathRequest`] to poll rather than a path.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crossbeam_channel::{Receiver, Sender, TryRecvError};

/// The user's answer to a file picker — a path, or `None` for "cancelled" —
/// which may not be known yet.
pub struct PathRequest(Receiver<Option<PathBuf>>);

impl PathRequest {
    /// Already answered. A desktop dialog blocks until the user has answered
    /// it, so its request is born complete.
    pub fn ready(answer: Option<PathBuf>) -> Self {
        let (tx, request) = Self::pending();
        let _ = tx.send(answer);
        request
    }

    /// Answered later: the shell sends the answer through the returned
    /// sender. Dropping the sender unanswered counts as a cancel.
    pub fn pending() -> (Sender<Option<PathBuf>>, Self) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        (tx, Self(rx))
    }

    /// `None` while the user is still choosing; the answer, once.
    pub fn poll(&self) -> Option<Option<PathBuf>> {
        match self.0.try_recv() {
            Ok(answer) => Some(answer),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(None),
        }
    }
}

/// Something another app handed us: "Open with jsonquery" or "Share to
/// jsonquery".
pub enum Incoming {
    File(PathBuf),
    Text(String),
}

/// Everything the app needs from the device it runs on.
pub trait Platform: Send + Sync {
    /// The context the window runs in, handed over once as the app starts —
    /// the way for a shell to wake the UI when an answer arrives from
    /// another thread.
    fn attach(&self, _ctx: &egui::Context) {}

    /// A touch-first device: bigger touch targets, a long press for the
    /// context menu, and the on-screen keyboard.
    fn is_touch(&self) -> bool {
        false
    }

    /// Ask which JSON file to open.
    fn pick_file_to_open(&self) -> PathRequest;

    /// Ask where to save a file, suggesting `suggested_name`. The app then
    /// writes to the path it is given and calls [`Platform::file_saved`].
    fn pick_file_to_save(&self, suggested_name: &str) -> PathRequest;

    /// The file at `path` (from [`Platform::pick_file_to_save`]) is fully
    /// written. A shell that handed out a staging path moves it to the
    /// destination the user really chose.
    fn file_saved(&self, _path: &Path) {}

    /// What to call the file at `path` when the path itself means nothing to
    /// the user (a staged copy of a picked document).
    fn display_name(&self, _path: &Path) -> Option<String> {
        None
    }

    fn clipboard_text(&self) -> Option<String> {
        None
    }

    fn set_clipboard_text(&self, _text: &str) {}

    /// A document (or text) another app asked us to open, oldest first.
    fn take_incoming(&self) -> Option<Incoming> {
        None
    }

    /// Called with each frame's raw input, for a shell to add what the window
    /// system doesn't deliver (soft-keyboard text, safe-area insets).
    fn hook_raw_input(&self, _ctx: &egui::Context, _raw: &mut egui::RawInput) {}

    /// A text field has (or no longer has) focus: show or hide the on-screen
    /// keyboard.
    fn set_keyboard_visible(&self, _visible: bool) {}

    /// The light/dark theme changed, so system bars can follow it.
    fn set_dark_theme(&self, _dark: bool) {}
}

static PLATFORM: OnceLock<Box<dyn Platform>> = OnceLock::new();

/// Install the shell's implementation. Call once, before the window opens;
/// later calls (and calls after the app has already asked for the default)
/// are ignored.
pub fn install(platform: Box<dyn Platform>) {
    let _ = PLATFORM.set(platform);
}

pub(crate) fn current() -> &'static dyn Platform {
    PLATFORM.get_or_init(default_platform).as_ref()
}

#[cfg(feature = "desktop")]
fn default_platform() -> Box<dyn Platform> {
    Box::new(Desktop)
}

#[cfg(not(feature = "desktop"))]
fn default_platform() -> Box<dyn Platform> {
    Box::new(Unsupported)
}

/// The native open/save dialogs of a desktop OS.
#[cfg(feature = "desktop")]
struct Desktop;

#[cfg(feature = "desktop")]
impl Platform for Desktop {
    fn pick_file_to_open(&self) -> PathRequest {
        PathRequest::ready(
            rfd::FileDialog::new()
                .add_filter("JSON", &["json", "ndjson", "jsonl", "log", "txt"])
                .pick_file(),
        )
    }

    fn pick_file_to_save(&self, suggested_name: &str) -> PathRequest {
        PathRequest::ready(
            rfd::FileDialog::new()
                .set_file_name(suggested_name)
                .add_filter("JSON", &["json"])
                .save_file(),
        )
    }
}

/// No shell installed and no native dialogs compiled in: every picker is
/// cancelled at once.
#[cfg(not(feature = "desktop"))]
struct Unsupported;

#[cfg(not(feature = "desktop"))]
impl Platform for Unsupported {
    fn pick_file_to_open(&self) -> PathRequest {
        PathRequest::ready(None)
    }

    fn pick_file_to_save(&self, _suggested_name: &str) -> PathRequest {
        PathRequest::ready(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ready_request_answers_once() {
        let request = PathRequest::ready(Some(PathBuf::from("/tmp/a.json")));
        assert_eq!(request.poll(), Some(Some(PathBuf::from("/tmp/a.json"))));
        // The answer has been taken; the sender is gone, which reads as a cancel.
        assert_eq!(request.poll(), Some(None));
    }

    #[test]
    fn a_pending_request_waits_for_its_answer() {
        let (tx, request) = PathRequest::pending();
        assert_eq!(request.poll(), None);
        tx.send(Some(PathBuf::from("/tmp/b.json"))).unwrap();
        assert_eq!(request.poll(), Some(Some(PathBuf::from("/tmp/b.json"))));
    }

    #[test]
    fn a_dropped_sender_is_a_cancel() {
        let (tx, request) = PathRequest::pending();
        drop(tx);
        assert_eq!(request.poll(), Some(None));
    }
}
