// Suppress the console window that would otherwise appear behind the GUI on
// a release Windows build; debug builds keep it so `println!`/panics are
// visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cli;
mod dock;
mod pane_header;
mod query_highlight;
mod query_suggest;
#[cfg(test)]
mod scratch_dir;
mod settings;
mod settings_window;
mod text_menu;
mod tools;
mod tree_view;
mod tutorial;
mod worker;
mod workspace;
mod workspace_pane;
mod x11_alert;
mod x11_launcher;

/// Must match the AppImage's `.desktop` file (`StartupWMClass=jsonquery_gui`)
/// so window managers associate the running window with the launcher icon —
/// otherwise "pin to taskbar" after launch doesn't stick.
const APP_ID: &str = "jsonquery_gui";

/// What the app is called in the title of its windows: the main window's own, and
/// the start of every other's ("JsonQuery — Tools", see [`title_of`]).
const APP_TITLE: &str = "JsonQuery";

/// The title of one of the app's other windows: the app, then what the window is.
fn title_of(window: &str) -> String {
    format!("{APP_TITLE} — {window}")
}

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
        .with_title(APP_TITLE)
        .with_app_id(APP_ID)
        .with_icon(app_icon());
    if interface.maximized {
        builder.with_maximized(true)
    } else {
        builder
    }
}

fn main() -> eframe::Result {
    let launch = read_the_command_line();

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
        Box::new(move |cc| Ok(Box::new(app::Shared::new(cc, store, settings, launch)))),
    )
}

/// What the command line asks the app to start with (`cli.rs`). `--help` and
/// `--version` are printed here, and end the program before any window opens; so
/// does a command line that cannot be done, which is said, and ends it with 2.
fn read_the_command_line() -> cli::Launch {
    use std::io::IsTerminal;

    let program = cli::program_name(
        std::env::args_os().next().as_deref(),
        std::env::var_os("SNAP_NAME").as_deref(),
        std::env::var_os("APPIMAGE").as_deref(),
    );
    let cwd = std::env::current_dir()
        .ok()
        .map(|physical| cli::working_folder(std::env::var_os("PWD").as_deref(), physical));
    let read_query =
        |path: &std::path::Path| workspace::read_text(path, workspace_pane::QUERY_LIMIT);
    match cli::parse(std::env::args_os().skip(1), cwd.as_deref(), read_query) {
        Ok(cli::Ask::Help) => print_and_end(&cli::help(&program)),
        Ok(cli::Ask::Version) => print_and_end(&cli::version(&program)),
        Err(error) => end_with_error(&program, &error),
        Ok(cli::Ask::Run(launch)) => {
            // A terminal is not a pipe: reading it would wait for whoever is at it.
            if launch.open == Some(cli::Source::Stdin) && std::io::stdin().is_terminal() {
                end_with_error(
                    &program,
                    "- opens what is piped in, and nothing is (the input is a terminal)",
                );
            }
            launch
        }
    }
}

/// Print `text`, and end the program.
fn print_and_end(text: &str) -> ! {
    use std::io::Write;
    cli::use_the_terminal();
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{}", text.trim_end());
    let _ = out.flush();
    std::process::exit(0)
}

/// Say what of the command line cannot be done, and end the program with 2.
fn end_with_error(program: &str, error: &str) -> ! {
    use std::io::Write;
    cli::use_the_terminal();
    let _ = writeln!(
        std::io::stderr(),
        "{program}: {error}\nSee {program} --help."
    );
    std::process::exit(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_opens_at_the_size_it_always_has_the_first_time() {
        let window = main_window(&settings::Interface::default());
        assert_eq!(window.inner_size, Some(egui::vec2(1200.0, 800.0)));
        assert_eq!(window.min_inner_size, Some(egui::vec2(700.0, 420.0)));
        assert_ne!(window.maximized, Some(true));
    }

    #[test]
    fn the_windows_are_titled_with_the_name_of_the_app() {
        let window = main_window(&settings::Interface::default());
        assert_eq!(window.title.as_deref(), Some("JsonQuery"));
        assert_eq!(title_of("Tools"), "JsonQuery — Tools");
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
        assert_eq!(window.inner_size, Some(egui::vec2(700.0, 420.0)));
    }
}
