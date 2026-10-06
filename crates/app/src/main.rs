// Suppress the console window that would otherwise appear behind the GUI on
// a release Windows build; debug builds keep it so `println!`/panics are
// visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod dock;
mod pane_header;
mod query_highlight;
mod query_suggest;
#[cfg(test)]
mod scratch_dir;
mod settings;
mod settings_window;
mod tools;
mod tree_view;
mod tutorial;
mod worker;
mod x11_alert;
mod x11_launcher;

/// Must match the AppImage's `.desktop` file (`StartupWMClass=jsonquery_gui`)
/// so window managers associate the running window with the launcher icon —
/// otherwise "pin to taskbar" after launch doesn't stick.
const APP_ID: &str = "jsonquery_gui";

/// The window icon, shared by the main window and every other window it opens.
fn app_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
        .expect("bundled icon should be a valid PNG")
}

/// The main window, as it was left: the size it had, and maximized if it was
/// (the first time, 1200 × 800).
fn main_window(interface: &settings::Interface) -> egui::ViewportBuilder {
    let builder = egui::ViewportBuilder::default()
        .with_inner_size(interface.window_size())
        .with_min_inner_size(settings::MIN_WINDOW)
        .with_title("jsonquery")
        .with_app_id(APP_ID)
        .with_icon(app_icon());
    if interface.maximized {
        builder.with_maximized(true)
    } else {
        builder
    }
}

fn main() -> eframe::Result {
    // What the user set and how they left the window (`settings.rs`) is read
    // before the window opens, which is opened as it was left.
    let mut store = settings::Store::standard();
    let settings = store.load();
    let native_options = eframe::NativeOptions {
        viewport: main_window(&settings.interface),
        ..Default::default()
    };

    eframe::run_native(
        "jsonquery_gui",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::Shared::new(cc, store, settings)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_opens_at_the_size_it_always_has_the_first_time() {
        let window = main_window(&settings::Interface::default());
        assert_eq!(window.inner_size, Some(egui::vec2(1200.0, 800.0)));
        assert_eq!(window.min_inner_size, Some(egui::vec2(640.0, 420.0)));
        assert_ne!(window.maximized, Some(true));
    }

    #[test]
    fn the_window_opens_as_it_was_left() {
        let mut interface = settings::Interface {
            window: Some([1500.0, 950.0]),
            ..Default::default()
        };
        let window = main_window(&interface);
        assert_eq!(window.inner_size, Some(egui::vec2(1500.0, 950.0)));
        assert_ne!(window.maximized, Some(true));

        // Maximized, and the size it goes back to when it is not.
        interface.maximized = true;
        let window = main_window(&interface);
        assert_eq!(window.maximized, Some(true));
        assert_eq!(window.inner_size, Some(egui::vec2(1500.0, 950.0)));
    }

    #[test]
    fn a_window_is_never_opened_smaller_than_it_can_be_used() {
        let interface = settings::Interface {
            window: Some([200.0, 150.0]),
            ..Default::default()
        };
        let window = main_window(&interface);
        assert_eq!(window.inner_size, Some(egui::vec2(640.0, 420.0)));
    }
}
