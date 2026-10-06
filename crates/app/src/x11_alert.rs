//! The ⚠ in the status bar, next to the ℹ, that tells a user on a Wayland
//! desktop that files can't be dropped on the window, and what to do about it:
//! a click opens a small popup with the explanation and a button that saves the
//! X11 launcher script (`x11_launcher`), then shows the command to run.
//!
//! It is a widget of its own, with its state: it draws nothing at all off
//! native Wayland, so the status bar is exactly what it was everywhere else.

use std::path::{Path, PathBuf};

use eframe::egui;

use crate::x11_launcher;

const POPUP_WIDTH: f32 = 320.0;

pub struct X11Alert {
    /// Whether there is anything to warn about: running as a native Wayland
    /// client.
    shown: bool,
    /// What came of the last click on the download button: the command that
    /// sets the saved script up, or why it could not be saved.
    saved: Option<Result<String, String>>,
    /// Asks where to save the script. A save dialog, but for the tests.
    chooser: fn() -> Option<PathBuf>,
}

/// Hidden. (Also what `std::mem::take` leaves behind while the real one is
/// borrowed apart from the app.)
impl Default for X11Alert {
    fn default() -> Self {
        Self::new(false)
    }
}

impl X11Alert {
    pub fn new(shown: bool) -> Self {
        Self {
            shown,
            saved: None,
            chooser: choose_in_dialog,
        }
    }

    /// Shown if this process is a native Wayland client.
    pub fn detect() -> Self {
        Self::new(x11_launcher::on_native_wayland())
    }

    /// The ⚠ button and, once it was clicked, its popup. Nothing off Wayland.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.shown {
            return;
        }
        let warn = ui.visuals().warn_fg_color;
        let button = ui.add(egui::Button::new(egui::RichText::new("⚠").color(warn)).small());
        let popup_id = egui::Popup::default_response_id(&button);
        if !egui::Popup::is_id_open(ui.ctx(), popup_id) {
            button.clone().on_hover_text(
                "Wayland: files can't be dropped on this window. Click to see how to fix that.",
            );
        }
        egui::Popup::from_toggle_button_response(&button)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .align(egui::RectAlign::TOP_END)
            .width(POPUP_WIDTH)
            .show(|ui| self.contents(ui));
    }

    fn contents(&mut self, ui: &mut egui::Ui) {
        ui.strong("Wayland detected");
        ui.label(
            "Files can't be dropped on this window here, but they can under X11. \
             This script adds a \"jsonquery (X11)\" entry to your application menu \
             that starts jsonquery that way.",
        );
        ui.add_space(4.0);
        if ui
            .button(format!("⬇ Download {}", x11_launcher::FILE_NAME))
            .clicked()
        {
            if let Some(path) = (self.chooser)() {
                self.save_to(&path);
            }
        }
        match &self.saved {
            None => {}
            Some(Ok(command)) => {
                ui.add_space(4.0);
                ui.label("Saved. Run it once in a terminal:");
                ui.code(command);
                if ui.small_button("Copy command").clicked() {
                    ui.ctx().copy_text(command.clone());
                }
                ui.label("then start \"jsonquery (X11)\" from the application menu.");
            }
            Some(Err(why)) => {
                ui.add_space(4.0);
                ui.colored_label(ui.visuals().error_fg_color, why);
            }
        }
    }

    /// Save the script at `path` and remember what came of it.
    fn save_to(&mut self, path: &Path) {
        self.saved = Some(match x11_launcher::save_script(path) {
            Ok(()) => Ok(x11_launcher::run_command(path)),
            Err(error) => Err(format!("Could not save {}: {error}", path.display())),
        });
    }
}

/// The save dialog, starting in the Downloads folder if there is one.
fn choose_in_dialog() -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_file_name(x11_launcher::FILE_NAME)
        .add_filter("Shell script", &["sh"]);
    if let Some(dir) = x11_launcher::downloads_dir() {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: egui::Vec2 = egui::vec2(480.0, 360.0);

    /// A widget on its own in a headless `egui::Context`, frame by frame, with
    /// the pointer where the test puts it.
    struct Frames {
        ctx: egui::Context,
        alert: X11Alert,
        time: f64,
        shapes: Vec<egui::epaint::ClippedShape>,
        copied: Vec<String>,
    }

    impl Frames {
        fn new(alert: X11Alert) -> Self {
            let mut frames = Self {
                ctx: egui::Context::default(),
                alert,
                time: 0.0,
                shapes: Vec::new(),
                copied: Vec::new(),
            };
            frames.settle();
            frames
        }

        fn frame_with(&mut self, events: Vec<egui::Event>) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            self.time += 1.0 / 60.0;
            let alert = &mut self.alert;
            // At the bottom right, where the status bar has it.
            let mut output = self.ctx.run_ui(input, |ui| {
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| alert.ui(ui));
            });
            output.textures_delta.clear();
            self.shapes = output.shapes;
            for command in &output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    self.copied.push(text.clone());
                }
            }
        }

        fn settle(&mut self) {
            for _ in 0..6 {
                self.frame_with(Vec::new());
            }
        }

        fn click(&mut self, pos: egui::Pos2) {
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame_with(vec![egui::Event::PointerMoved(pos)]);
            self.frame_with(vec![button(true)]);
            self.frame_with(vec![button(false)]);
            self.settle();
        }

        /// Every piece of text drawn in the latest frame, with where.
        fn texts(&self) -> Vec<(String, egui::Rect)> {
            fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
                match shape {
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                    egui::Shape::Text(text) => out.push((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    )),
                    _ => {}
                }
            }
            let mut out = Vec::new();
            for clipped in &self.shapes {
                walk(&clipped.shape, &mut out);
            }
            out
        }

        fn text(&self, wanted: &str) -> Option<egui::Rect> {
            self.texts()
                .into_iter()
                .find(|(text, _)| text == wanted)
                .map(|(_, rect)| rect)
        }

        fn drew(&self, wanted: impl Fn(&str) -> bool) -> bool {
            self.texts().iter().any(|(text, _)| wanted(text))
        }

        fn click_text(&mut self, wanted: &str) {
            let rect = self
                .text(wanted)
                .unwrap_or_else(|| panic!("{wanted:?} was not drawn: {:?}", self.texts()));
            self.click(rect.center());
        }
    }

    const BUTTON: &str = "⚠";

    fn download_label() -> String {
        format!("⬇ Download {}", x11_launcher::FILE_NAME)
    }

    /// A folder of the test's own, so that tests running side by side don't
    /// share a file.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jq-x11-alert-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn it_is_hidden_unless_the_desktop_is_wayland() {
        let off = Frames::new(X11Alert::new(false));
        assert!(off.texts().is_empty(), "nothing drawn: {:?}", off.texts());
        let default = Frames::new(X11Alert::default());
        assert!(default.texts().is_empty(), "hidden by default");
        let on = Frames::new(X11Alert::new(true));
        assert!(on.text(BUTTON).is_some(), "the alert is drawn");
    }

    #[test]
    fn it_sits_at_the_bottom_right_and_its_popup_is_closed_at_first() {
        let frames = Frames::new(X11Alert::new(true));
        let rect = frames.text(BUTTON).unwrap();
        assert!(rect.center().x > SCREEN.x - 40.0, "right: {rect:?}");
        assert!(rect.center().y > SCREEN.y - 40.0, "bottom: {rect:?}");
        assert!(!frames.drew(|t| t.contains("Wayland detected")));
        assert!(frames.text(&download_label()).is_none());
    }

    #[test]
    fn clicking_it_opens_the_popup_with_the_download_button_and_clicking_again_closes_it() {
        let mut frames = Frames::new(X11Alert::new(true));
        frames.click_text(BUTTON);
        assert!(
            frames.drew(|t| t == "Wayland detected"),
            "{:?}",
            frames.texts()
        );
        assert!(frames.text(&download_label()).is_some());
        // Nothing has been saved, so there is no command yet.
        assert!(!frames.drew(|t| t.starts_with("bash ")));

        frames.click_text(BUTTON);
        assert!(frames.text(&download_label()).is_none(), "closed again");
    }

    #[test]
    fn the_popup_stays_inside_the_window_above_the_button() {
        let mut frames = Frames::new(X11Alert::new(true));
        let button = frames.text(BUTTON).unwrap();
        frames.click_text(BUTTON);
        for (text, rect) in frames.texts() {
            assert!(
                rect.min.x >= 0.0 && rect.max.x <= SCREEN.x && rect.min.y >= 0.0,
                "{text:?} is outside the window: {rect:?}"
            );
        }
        let heading = frames.text("Wayland detected").unwrap();
        assert!(heading.max.y < button.min.y, "above the button");
    }

    #[test]
    fn clicking_outside_closes_the_popup_but_clicking_inside_it_does_not() {
        let mut frames = Frames::new(X11Alert::new(true));
        frames.click_text(BUTTON);
        // On the explanation: inside.
        frames.click_text("Wayland detected");
        assert!(frames.text(&download_label()).is_some(), "still open");
        frames.click(egui::pos2(10.0, 10.0));
        assert!(frames.text(&download_label()).is_none(), "closed");
    }

    #[test]
    fn download_saves_the_script_where_the_dialog_says_and_shows_the_command() {
        fn chosen() -> Option<PathBuf> {
            Some(scratch("saves").join(x11_launcher::FILE_NAME))
        }
        let mut alert = X11Alert::new(true);
        alert.chooser = chosen;
        let mut frames = Frames::new(alert);
        frames.click_text(BUTTON);
        frames.click_text(&download_label());

        let path = chosen().unwrap();
        let saved = std::fs::read_to_string(&path).expect("the script was written");
        assert!(saved.starts_with("#!/usr/bin/env bash\n# jsonquery-x11.sh"));

        assert!(
            frames.drew(|t| t.starts_with("Saved.")),
            "{:?}",
            frames.texts()
        );
        // What `run_command` makes of the path: `bash` and the path, quoted as a
        // shell needs it (a path on Windows has backslashes), and the program.
        let command = x11_launcher::run_command(&path);
        assert!(
            frames.drew(|t| t.starts_with(&command)),
            "the command is shown ({command}): {:?}",
            frames.texts()
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn copy_command_puts_the_command_on_the_clipboard() {
        fn chosen() -> Option<PathBuf> {
            Some(scratch("copies").join(x11_launcher::FILE_NAME))
        }
        let mut alert = X11Alert::new(true);
        alert.chooser = chosen;
        let mut frames = Frames::new(alert);
        frames.click_text(BUTTON);
        assert!(frames.text("Copy command").is_none(), "nothing to copy yet");
        frames.click_text(&download_label());
        frames.click_text("Copy command");

        let path = chosen().unwrap();
        assert_eq!(frames.copied.len(), 1);
        assert_eq!(frames.copied[0], x11_launcher::run_command(&path));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_dialog_that_is_cancelled_saves_nothing_and_says_nothing() {
        fn cancelled() -> Option<PathBuf> {
            None
        }
        let mut alert = X11Alert::new(true);
        alert.chooser = cancelled;
        let mut frames = Frames::new(alert);
        frames.click_text(BUTTON);
        frames.click_text(&download_label());
        assert!(frames.text(&download_label()).is_some(), "popup still open");
        assert!(!frames.drew(|t| t.starts_with("Saved.") || t.starts_with("Could not")));
    }

    #[test]
    fn a_save_that_fails_says_why() {
        fn nowhere() -> Option<PathBuf> {
            Some(PathBuf::from("/no/such/folder/jsonquery-x11.sh"))
        }
        let mut alert = X11Alert::new(true);
        alert.chooser = nowhere;
        let mut frames = Frames::new(alert);
        frames.click_text(BUTTON);
        frames.click_text(&download_label());
        assert!(
            frames.drew(|t| t.starts_with("Could not save /no/such/folder/jsonquery-x11.sh")),
            "{:?}",
            frames.texts()
        );
        assert!(!frames.drew(|t| t.starts_with("Saved.")));
    }

    #[test]
    fn saving_again_replaces_the_result_of_the_failure() {
        let mut alert = X11Alert::new(true);
        alert.save_to(Path::new("/no/such/folder/jsonquery-x11.sh"));
        assert!(matches!(alert.saved, Some(Err(_))));
        let dir = scratch("again");
        alert.save_to(&dir.join(x11_launcher::FILE_NAME));
        assert!(matches!(alert.saved, Some(Ok(_))));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
