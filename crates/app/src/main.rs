// Suppress the console window that would otherwise appear behind the GUI on
// a release Windows build; debug builds keep it so `println!`/panics are
// visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod tree_view;
mod worker;

/// Must match the AppImage's `.desktop` file (`StartupWMClass=jsonquery`, see
/// `build/appimage.sh`) so window managers associate the running window with
/// the launcher icon — otherwise "pin to taskbar" after launch doesn't stick.
const APP_ID: &str = "jsonquery";

fn main() -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
        .expect("bundled icon should be a valid PNG");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([640.0, 420.0])
            .with_title("jsonquery")
            .with_app_id(APP_ID)
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "jsonquery",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
