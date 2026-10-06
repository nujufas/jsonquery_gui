//! The Settings window: from what size a file is memory-mapped and kept on disk,
//! which is the limit a person may want to change, and — folded away under
//! *Advanced* until they are asked for — the other eight limits on sizes. A second
//! native window (an egui *viewport*), like the tutorial and About;
//! `App::show_satellites` has the reasons it is drawn the way it is.
//!
//! The window is small, and says little: what a limit is for, and what it is by
//! default, is in a tooltip over its name. It grows to hold the Advanced limits
//! when they are opened, and goes back to the height it had when they are closed;
//! and it grows, once, when it opens with more in it than fits (warnings about the
//! settings file).
//!
//! A limit is typed as a size (`256 MB`, `4 GB`: see `settings::parse_size`) and
//! taken when the box loses the focus — Enter, Tab or a click elsewhere — or when
//! the window is closed. What is not a size is said so under the box and not
//! taken; Escape puts back what was there. What is taken is handed to the app,
//! which keeps it (`settings::Store`) and tells the worker.

use std::sync::Arc;

use eframe::egui;

use crate::settings::{
    check_range, parse_size, size_text, FileLimits, Group, Limit, Settings, Store,
};

pub(crate) fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("jsonquery_settings_window")
}

/// How wide the names of the limits are given room for, so that the boxes beside
/// them are in a column.
pub(crate) const LABEL_WIDTH: f32 = 196.0;

/// How wide the box a size is typed in is.
const BOX_WIDTH: f32 = 90.0;

/// The widest a tooltip is: the window is small, and a tooltip does not leave it.
const TIP_WIDTH: f32 = 330.0;

/// The size the window opens with: room for the one limit that is shown, the
/// Advanced header, the foot of the window, and a complaint about what was typed.
pub(crate) const OPENING_SIZE: [f32; 2] = [430.0, 190.0];

/// The least the window can be made.
const LEAST_SIZE: [f32; 2] = [370.0, 150.0];

/// The size of the window where it is not a window of its own but one inside the
/// main window (no real windows: the web): it cannot be made taller for the
/// Advanced limits, so it opens with room for them.
pub(crate) const EMBEDDED_SIZE: [f32; 2] = [430.0, 480.0];

/// How to type a size: the tooltip of every box.
const HOW_TO_TYPE: &str =
    "A size with its unit: KB, MB, GB or TB (1 MB is 1,048,576 bytes), as in \
     256 MB, 4 GB or 1.5 GB. Enter keeps it and Esc puts back what was there. It is kept for \
     the next time the app is started, and applies from the next file you open or the next \
     query you run.";

/// What the box of one limit holds while it is typed in.
#[derive(Default)]
struct Field {
    text: String,
    /// The text was typed in and has not been taken yet; until then it is not
    /// replaced by the limit's.
    edited: bool,
    /// Why the text is not a limit.
    problem: Option<String>,
}

impl Field {
    /// Take the text for a limit: what it says, or why it can't be one.
    fn take(&mut self) -> Result<u64, String> {
        let taken = parse_size(&self.text).and_then(check_range);
        match &taken {
            Ok(_) => self.forget(),
            Err(why) => self.problem = Some(why.clone()),
        }
        taken
    }

    /// Drop what was typed: the box shows the limit again.
    fn forget(&mut self) {
        self.edited = false;
        self.problem = None;
    }
}

/// What the window's height is doing about the Advanced limits being shown or
/// hidden. They slide open and shut (egui's animation of a header), and the window
/// is made another height when they have: when they are in full, it is known how
/// much room they need.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Height {
    #[default]
    Left,
    /// The window was just opened: once it has drawn itself a few times, it grows
    /// if it is too short for what is in it (warnings about the settings file, or
    /// a larger font, take room).
    Fit { frames: u8 },
    /// The limits were just shown: once they are drawn in full, the window grows
    /// if it is too short to hold them.
    Grow,
    /// The window grew, from this height, and goes back to it when the limits are
    /// hidden.
    Grown { from: f32 },
    /// The limits were just hidden: once they are gone, the window goes back to
    /// this height.
    Shrink { to: f32 },
}

#[derive(Default)]
pub struct SettingsWindow {
    open: bool,
    icon: Option<Arc<egui::IconData>>,
    /// One for each of [`Limit::ALL`], in its order.
    fields: [Field; Limit::ALL.len()],
    /// The Advanced limits are shown.
    advanced: bool,
    height: Height,
}

impl SettingsWindow {
    /// Open the window, or bring it to the front if it's already open.
    pub fn open_or_focus(&mut self, ctx: &egui::Context) {
        if self.open {
            ctx.send_viewport_cmd_to(viewport_id(), egui::ViewportCommand::Focus);
        } else {
            self.open = true;
            self.advanced = false;
            self.height = Height::Fit { frames: 0 };
            self.fields.iter_mut().for_each(Field::forget);
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The window was closed (by the user): stop showing it. What was typed in a
    /// box and is a limit, though the box has not lost the focus, is taken: the
    /// settings it makes are given, if they differ from `current`.
    pub fn close(&mut self, current: &Settings) -> Option<Settings> {
        let mut next = *current;
        for (limit, field) in Limit::ALL.into_iter().zip(self.fields.iter_mut()) {
            if field.edited {
                if let Ok(bytes) = parse_size(&field.text).and_then(check_range) {
                    next.limits.set(limit, bytes);
                }
            }
            field.forget();
        }
        self.open = false;
        (next != *current).then_some(next)
    }

    /// The window the settings are shown in. The same every frame it is open:
    /// egui patches the real window to match a builder that changed.
    pub fn builder(&mut self) -> egui::ViewportBuilder {
        let icon = self
            .icon
            .get_or_insert_with(|| Arc::new(crate::app_icon()))
            .clone();
        egui::ViewportBuilder::default()
            .with_title("jsonquery — Settings")
            .with_inner_size(OPENING_SIZE)
            .with_min_inner_size(LEAST_SIZE)
            .with_app_id(crate::APP_ID)
            .with_icon(icon)
    }

    /// One frame of the window when it is a window of its own, redrawn by eframe
    /// by itself. Returns the settings it changed, if it did.
    pub fn window_frame(
        &mut self,
        ui: &mut egui::Ui,
        current: &Settings,
        store: &Store,
    ) -> Option<Settings> {
        // What is wrong, and where the settings are kept, stays in view however
        // far the limits are scrolled.
        egui::Panel::bottom("settings_footer").show(ui, |ui| footer(ui, store));
        egui::CentralPanel::default()
            .show(ui, |ui| self.contents(ui, current))
            .inner
    }

    /// Draw the window (if open) for this frame as an immediate viewport, inside
    /// the main window's frame — where there are no real windows to redraw by
    /// themselves. Returns the settings it changed, if it did.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        current: &Settings,
        store: &Store,
    ) -> Option<Settings> {
        if !self.open {
            return None;
        }
        // Inside the main window: it cannot be resized later, so it starts at a
        // size for everything in it.
        let builder = self.builder().with_inner_size(EMBEDDED_SIZE);
        let mut changed = None;
        let mut close = false;
        ctx.show_viewport_immediate(viewport_id(), builder, |ui, _class| {
            close = ui.ctx().input(|i| i.viewport().close_requested());
            changed = self.window_frame(ui, current, store);
        });
        if close {
            let base = changed.unwrap_or(*current);
            changed = self.close(&base).or(changed);
        }
        changed
    }

    fn contents(&mut self, ui: &mut egui::Ui, current: &Settings) -> Option<Settings> {
        let mut next = *current;
        let any_edited = self.fields.iter().any(|f| f.edited);
        // (No least height to scroll in: `visible`, below, is what there is room for.)
        let shown = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("File size limits");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let any_changed = !current.limits.all_default() || any_edited;
                        if ui
                            .add_enabled(any_changed, egui::Button::new("Restore defaults"))
                            .on_hover_text("Put every limit back to what the app starts with")
                            .on_disabled_hover_text("Every limit is what the app starts with")
                            .clicked()
                        {
                            next.limits = FileLimits::default();
                            self.fields.iter_mut().for_each(Field::forget);
                        }
                    });
                });

                ui.add_space(4.0);
                for (limit, field) in Limit::ALL.into_iter().zip(self.fields.iter_mut()) {
                    if !limit.is_advanced() {
                        limit_row(ui, limit, field, &mut next, LABEL_WIDTH);
                    }
                }

                ui.add_space(4.0);
                self.advanced_limits(ui, &mut next)
            });
        self.fit_height(
            ui.ctx(),
            shown.inner,
            shown.content_size.y,
            shown.inner_rect.height(),
        );
        (next != *current).then_some(next)
    }

    /// The Advanced header, and under it, when it is open, the other limits, each
    /// group with its title. How far open they are, from 0 (shut) to 1 (in full).
    fn advanced_limits(&mut self, ui: &mut egui::Ui, next: &mut Settings) -> f32 {
        let changed = Limit::ALL
            .into_iter()
            .filter(|&limit| limit.is_advanced() && !next.limits.is_default(limit))
            .count();
        // What is hidden and not the default is said, so that it is not forgotten.
        let title = match changed {
            0 => "Advanced".to_owned(),
            n => format!("Advanced ({n} changed)"),
        };
        let header = egui::CollapsingHeader::new(title)
            .id_salt("settings_advanced")
            .open(Some(self.advanced))
            .show(ui, |ui| {
                for group in Group::ALL {
                    let mut titled = false;
                    for (limit, field) in Limit::ALL.into_iter().zip(self.fields.iter_mut()) {
                        if !limit.is_advanced() || limit.group() != group {
                            continue;
                        }
                        if !titled {
                            titled = true;
                            ui.add_space(4.0);
                            let title = ui.label(egui::RichText::new(group.title()).strong());
                            if let Some(note) = group.note() {
                                tip(title, |ui| {
                                    ui.label(note);
                                });
                            }
                        }
                        // Indented, and the boxes still in the column of the first one.
                        limit_row(ui, limit, field, next, LABEL_WIDTH - ui.spacing().indent);
                    }
                }
            });
        if header.header_response.clicked() {
            self.advanced = !self.advanced;
            self.height = match (self.advanced, self.height) {
                // Opened again before the window went back: it has not, yet.
                (true, Height::Shrink { to }) => Height::Grown { from: to },
                (true, _) => Height::Grow,
                (false, Height::Grown { from }) => Height::Shrink { to: from },
                (false, _) => Height::Left,
            };
            // The limits are drawn, or not, from the next frame (the header
            // follows `self.advanced`, which it was just given).
            ui.ctx().request_repaint();
        }
        header.openness
    }

    /// Make the window another height: as tall as what is in it needs, if it is
    /// not already, a few frames after it opened; as tall as the Advanced limits
    /// need, now that they are in full (`openness` 1); or as tall as it was, now
    /// that they are gone (0). `content` is how tall everything in the window is,
    /// `visible` how much of it there is room for.
    fn fit_height(&mut self, ctx: &egui::Context, openness: f32, content: f32, visible: f32) {
        // Only a window of its own can be made another size. One inside the main
        // window is no viewport, and the command would be for the main window.
        if ctx.viewport_id() != viewport_id() {
            self.height = Height::Left;
            return;
        }
        match self.height {
            // A few frames, for the window to have measured itself, and to have
            // been told how big it is.
            Height::Fit { frames } => {
                let known = ctx.input(|i| i.viewport().inner_rect.is_some());
                if frames < 3 || !known {
                    self.height = Height::Fit {
                        frames: frames.saturating_add(1).min(3),
                    };
                    ctx.request_repaint();
                } else {
                    self.height = Height::Left;
                    grow_to_hold(ctx, content, visible);
                }
            }
            Height::Grow if openness >= 1.0 => {
                self.height = match grow_to_hold(ctx, content, visible) {
                    Some(from) => Height::Grown { from },
                    None => Height::Left,
                };
            }
            Height::Shrink { to } if openness <= 0.0 => {
                self.height = Height::Left;
                resize(ctx, to);
            }
            _ => {}
        }
    }
}

/// Ask for a taller window if what is in it (`content` tall) does not fit in
/// what it shows (`visible`): as tall as it needs, and not taller than the screen
/// has room for, the rest being scrolled. The height it had, if it was asked.
fn grow_to_hold(ctx: &egui::Context, content: f32, visible: f32) -> Option<f32> {
    let (inner, monitor) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size));
    let inner = inner?;
    let missing = content - visible;
    if missing <= 1.0 {
        return None;
    }
    let room = monitor.map_or(f32::INFINITY, |size| size.y * 0.9);
    resize(
        ctx,
        (inner.height() + missing).min(room.max(inner.height())),
    );
    Some(inner.height())
}

/// Make the window `height` tall, as wide as it is.
fn resize(ctx: &egui::Context, height: f32) {
    let width = ctx
        .input(|i| i.viewport().inner_rect)
        .map_or(OPENING_SIZE[0], |rect| rect.width());
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
}

/// A tooltip over `response`, no wider than the window can hold.
fn tip(response: egui::Response, add: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(TIP_WIDTH);
        add(ui);
    })
}

/// The row of one limit: its name, given `label_width` (what it is for is in the
/// tooltip), the box to type it in, and the button that puts the default back; and
/// under them what is wrong with what was typed, if something is.
fn limit_row(
    ui: &mut egui::Ui,
    limit: Limit,
    field: &mut Field,
    next: &mut Settings,
    label_width: f32,
) {
    let default = FileLimits::default().get(limit);
    if !field.edited {
        field.text = size_text(next.limits.get(limit));
    }
    ui.horizontal(|ui| {
        // The boxes line up in a column, whatever the labels' widths.
        ui.allocate_ui_with_layout(
            egui::vec2(label_width, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(label_width);
                let name = ui
                    .label(limit.label())
                    .on_hover_cursor(egui::CursorIcon::Help);
                tip(name, |ui| {
                    ui.label(limit.help());
                    ui.label(
                        egui::RichText::new(format!("Default: {}", size_text(default))).weak(),
                    );
                });
            },
        );
        let edit = ui.add(
            egui::TextEdit::singleline(&mut field.text)
                .id(egui::Id::new(("settings_limit", limit.key())))
                .desired_width(BOX_WIDTH),
        );
        let edit = tip(edit, |ui| {
            ui.label(HOW_TO_TYPE);
        });
        if edit.changed() {
            field.edited = true;
            field.problem = None;
        }
        if edit.lost_focus() && field.edited {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                // Put back what was there.
                field.forget();
            } else if let Ok(bytes) = field.take() {
                next.limits.set(limit, bytes);
            }
        }
        let differs = !next.limits.is_default(limit) || field.edited;
        if ui
            .add_enabled(differs, egui::Button::new("Reset"))
            .on_hover_text(format!("Put back {}", size_text(default)))
            .on_disabled_hover_text(format!("{} is the default", size_text(default)))
            .clicked()
        {
            next.limits.set(limit, default);
            field.forget();
        }
    });
    if let Some(problem) = &field.problem {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!(
                "{} — the limit stays {}",
                sentence(problem),
                size_text(next.limits.get(limit))
            ),
        );
    }
}

/// `text` with a capital to begin it.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Where the settings are kept, and what is wrong with them if something is.
fn footer(ui: &mut egui::Ui, store: &Store) {
    for note in store.notes() {
        ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {note}"));
    }
    if let Some(error) = store.error() {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    match store.path() {
        Some(path) => {
            ui.label(egui::RichText::new(format!("Kept in {}", path.display())).weak());
        }
        None => {
            ui.label(
                egui::RichText::new(
                    "Not kept for next time: there is no folder for settings on this system.",
                )
                .weak(),
            );
        }
    }
}
