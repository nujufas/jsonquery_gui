//! The *Known issues* section of the About window: a header that opens the list of
//! what runs differently here than in jq (`jsonquery_query::known_issues`), and what
//! the window does about its size while the list is open.
//!
//! The window is small, and says little until it is asked for more. When the list is
//! opened it slides open (egui's animation of a header), and then the window is made
//! as big as it needs to hold it: wider first, because the lines are long and wrap
//! into a great many more in a narrow window, and once it has that width, taller, no
//! taller than the screen has room for (the rest scrolls). When the list is closed
//! the window goes back to the size it had. The Settings window does the same with
//! its *Advanced* limits, in its height.

use super::*;
use crate::tutorial::prose;
use jsonquery_query::known_issues::{self, Issue};

/// How wide the window is made for the list, if it is not wider already.
const OPEN_WIDTH: f32 = 640.0;

/// The widest the tooltip with an example is.
const TIP_WIDTH: f32 = 420.0;

/// How many frames the window is given to arrive at the width it was asked for, so
/// that the height the list needs is worked out at that width. (A window manager
/// that does not resize windows, a tiling one, never gets there.)
const WIDTH_WAIT_FRAMES: u8 = 30;

/// What the window's size is doing about the list being open or shut.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Size {
    #[default]
    Left,
    /// The list was just opened, and the window of size `from` was asked to be
    /// `wide` (if it was narrower). Once the list is open in full, and the window has
    /// that width, it is made as tall as the list needs.
    Opening {
        from: egui::Vec2,
        wide: Option<f32>,
        frames: u8,
    },
    /// The window was made bigger, from this size, and goes back to it when the list
    /// is closed.
    Grown { from: egui::Vec2 },
    /// The list was just closed: once it is gone, the window goes back to this size.
    Closing { to: egui::Vec2 },
}

/// The section, and what is being done to the window's size for it.
#[derive(Default)]
pub(super) struct IssuesView {
    /// The list is shown (the header follows this).
    open: bool,
    size: Size,
    /// How many times the window has been opened. The header remembers how far open
    /// it was, and slides shut when the window is opened again with the list closed;
    /// a header of another id starts shut.
    opened: u32,
}

impl IssuesView {
    /// The window was opened afresh: the list is closed.
    pub(super) fn reset(&mut self) {
        *self = Self {
            opened: self.opened.wrapping_add(1),
            ..Self::default()
        };
    }

    /// The header and, under it, the list when it is open. How far open it is, from
    /// 0 (shut) to 1 (in full).
    pub(super) fn show(&mut self, ui: &mut egui::Ui) -> f32 {
        let title = format!("Known issues ({})", known_issues::count());
        let header = egui::CollapsingHeader::new(egui::RichText::new(title).strong())
            .id_salt(("info_known_issues", self.opened))
            .open(Some(self.open))
            .show(ui, list);
        if header.header_response.clicked() {
            self.open = !self.open;
            self.toggled(ui.ctx());
            // The list is drawn, or not, from the next frame (the header follows
            // `self.open`, which it was just given).
            ui.ctx().request_repaint();
        }
        header.openness
    }

    /// The list was just opened or closed: what the window is asked for first.
    fn toggled(&mut self, ctx: &egui::Context) {
        // Only a window of its own can be made another size. One inside the main
        // window is no viewport, and the command would be for the main window.
        if ctx.viewport_id() != info_viewport_id() {
            self.size = Size::Left;
            return;
        }
        if self.open {
            let (inner, screen) = window_and_screen(ctx);
            self.size = match (self.size, inner) {
                // Opened again before the window had gone back: it has not, yet.
                (Size::Closing { to }, _) => Size::Grown { from: to },
                (_, None) => Size::Left,
                (_, Some(inner)) => {
                    let from = inner.size();
                    let room = screen.map_or(f32::INFINITY, |screen| screen.x * 0.9);
                    let wide = OPEN_WIDTH.min(room).max(from.x);
                    let wider = wide > from.x + 1.0;
                    if wider {
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            wide, from.y,
                        )));
                    }
                    Size::Opening {
                        from,
                        wide: wider.then_some(wide),
                        frames: 0,
                    }
                }
            };
        } else {
            self.size = match self.size {
                Size::Opening {
                    from,
                    wide: Some(_),
                    ..
                }
                | Size::Grown { from } => Size::Closing { to: from },
                // Nothing was asked for, so there is nothing to put back.
                Size::Opening { wide: None, .. } | Size::Left => Size::Left,
                closing @ Size::Closing { .. } => closing,
            };
        }
    }

    /// Make the window another size, now that the list is `openness` open (0 to 1):
    /// as tall as the list needs when it is in full (`content` is how tall
    /// everything in the window is, `visible` how much of it there is room for), or
    /// as big as it was when the list is gone.
    pub(super) fn fit_window(
        &mut self,
        ctx: &egui::Context,
        openness: f32,
        content: f32,
        visible: f32,
    ) {
        if ctx.viewport_id() != info_viewport_id() {
            self.size = Size::Left;
            return;
        }
        match self.size {
            Size::Opening { from, wide, frames } if openness >= 1.0 => {
                let (inner, screen) = window_and_screen(ctx);
                let Some(inner) = inner else {
                    self.size = Size::Left;
                    return;
                };
                // Text wraps at the width the window has, so the window is given a
                // few frames to have the one it was asked for.
                if wide.is_some_and(|wide| inner.width() + 1.0 < wide) && frames < WIDTH_WAIT_FRAMES
                {
                    self.size = Size::Opening {
                        from,
                        wide,
                        frames: frames + 1,
                    };
                    ctx.request_repaint();
                    return;
                }
                let missing = content - visible;
                let asked = if missing > 1.0 {
                    let room = screen.map_or(f32::INFINITY, |screen| screen.y * 0.9);
                    let height = (inner.height() + missing).min(room.max(inner.height()));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                        inner.width(),
                        height,
                    )));
                    true
                } else {
                    wide.is_some()
                };
                self.size = if asked {
                    Size::Grown { from }
                } else {
                    Size::Left
                };
            }
            Size::Closing { to } if openness <= 0.0 => {
                self.size = Size::Left;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(to));
            }
            _ => {}
        }
    }
}

/// The window's size and the screen's, as the window manager last told.
fn window_and_screen(ctx: &egui::Context) -> (Option<egui::Rect>, Option<egui::Vec2>) {
    ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size))
}

/// The list: a line about it, then each group with its title and its issues, and
/// where the rest are.
fn list(ui: &mut egui::Ui) {
    ui.weak("Programs that run differently here than in jq. Hover a line for an example.");
    for group in known_issues::GROUPS {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(group.title).strong());
        for issue in group.main() {
            ui.horizontal_top(|ui| {
                ui.label("•");
                let line = prose(ui, issue.text).on_hover_ui(|ui| example_tip(ui, issue));
                crate::text_menu::copy_only(&line, issue.text);
            });
        }
    }
    ui.add_space(4.0);
    ui.hyperlink_to(
        format!(
            "{} smaller ones, and more examples, in the full list",
            known_issues::minor_count()
        ),
        format!("{REPO_URL}/blob/master/{}", known_issues::PAGE),
    );
}

/// What a line shows when it is hovered: the program of the issue's first example,
/// and what jq and this app give for it, in two lines.
fn example_tip(ui: &mut egui::Ui, issue: &Issue) {
    ui.set_max_width(TIP_WIDTH);
    let example = &issue.examples[0];
    ui.label(egui::RichText::new(example.program()).monospace());
    prose(
        ui,
        &format!("jq: {} — here: {}", example.jq, example.here.describe()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page `docs/known-issues.md` is made from the list, not written by hand.
    /// (To write it again after the list was changed:
    /// `JSONQUERY_WRITE_DOCS=1 cargo test -p jsonquery_gui the_known_issues_page`.)
    #[test]
    fn the_known_issues_page_is_what_the_list_says() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(known_issues::PAGE);
        let made = known_issues::markdown();
        if std::env::var_os("JSONQUERY_WRITE_DOCS").is_some() {
            std::fs::write(&path, &made).expect("the page can be written");
            return;
        }
        // (A checkout on Windows may have turned the line ends into CR LF.)
        let kept = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            kept == made,
            "{} is not what the list of known issues says: write it again with \
             `JSONQUERY_WRITE_DOCS=1 cargo test -p jsonquery_gui the_known_issues_page`",
            known_issues::PAGE
        );
    }
}
