//! jsonquery's GUI as a library: the desktop binary (`main.rs`) and the
//! Android shell (`android/rust`) each open one [`App`] in their own window
//! and differ only in the [`platform::Platform`] they install.

mod app;
pub mod platform;
mod query_highlight;
mod query_suggest;
pub mod soft_keyboard;
mod tree_view;
mod tutorial;
mod worker;

pub use app::App;

/// Must match the AppImage's `.desktop` file (`StartupWMClass=jsonquery_gui`,
/// see `build/appimage.sh`) so window managers associate the running window
/// with the launcher icon — otherwise "pin to taskbar" after launch doesn't
/// stick.
pub const APP_ID: &str = "jsonquery_gui";

/// The window icon, shared by the main window and the tutorial window.
pub fn app_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
        .expect("bundled icon should be a valid PNG")
}
