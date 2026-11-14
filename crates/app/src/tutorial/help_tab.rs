//! The tutorial's tabs of help (`help.rs`): AI assistants in the builds that have the
//! MCP server, and General. A lesson is what to do, the commands that do it — each in
//! a box with a Copy button at its right — pictures of it, and what is good to know.

use std::time::Instant;

use eframe::egui::{self, RichText, TextWrapMode};

use super::help::{HelpTopic, Picture, Snippet};
use super::{code_frame, prose, tips, Tab, Tutorial};

/// The room the Copy button takes at the right of a command's box.
const COPY_ROOM: f32 = 64.0;

/// Where a lesson's text has it, the status bar's icon for the MCP server (the plug
/// with a small "AI") is drawn: the lesson shows the icon instead of saying what it
/// looks like.
#[cfg(feature = "mcp")]
pub(super) const PLUG: char = '🔌';

/// A lesson's prose, with the MCP server's icon drawn for each [`PLUG`] in it, in the
/// flow of the text.
fn prose_with_icons(ui: &mut egui::Ui, text: &str) {
    #[cfg(feature = "mcp")]
    if text.contains(PLUG) {
        ui.horizontal_wrapped(|ui| {
            // The text's own spaces part the words from the icon.
            ui.spacing_mut().item_spacing.x = 0.0;
            for (i, part) in text.split(PLUG).enumerate() {
                if i > 0 {
                    let color = ui.visuals().text_color();
                    crate::app::mcp_icon(ui, color);
                }
                if !part.is_empty() {
                    prose(ui, part);
                }
            }
        });
        return;
    }
    prose(ui, text);
}

impl Tutorial {
    /// A lesson of a help tab.
    pub(super) fn help_lesson_view(
        &mut self,
        ui: &mut egui::Ui,
        tab: Tab,
        topics: &'static [HelpTopic],
    ) {
        let slot = tab.slot();
        let (t, l) = self.selected[slot];
        let (t, l) = if topics.get(t).is_some_and(|topic| l < topic.lessons.len()) {
            (t, l)
        } else {
            (0, 0)
        };
        let topic = &topics[t];
        let lesson = &topic.lessons[l];

        let mut go_to = None;
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("tutorial_lesson", tab))
            .auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_to_top) {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| {
            ui.add_space(4.0);
            ui.weak(format!("{} › {}", tab.title(), topic.title));
            ui.heading(lesson.title);
            ui.add_space(4.0);
            prose_with_icons(ui, lesson.summary);
            ui.add_space(10.0);

            for link in lesson.links {
                crate::app::link(ui, link.text, link.url.to_owned());
                ui.add_space(4.0);
            }
            if !lesson.links.is_empty() {
                ui.add_space(6.0);
            }

            for snippet in lesson.snippets {
                self.snippet(ui, snippet);
                ui.add_space(8.0);
            }
            if !lesson.snippets.is_empty() {
                ui.add_space(2.0);
            }

            // The words first, what is good to know with them; then what it looks like.
            if !lesson.tips.is_empty() {
                tips(ui, lesson.tips);
                ui.add_space(10.0);
            }

            for picture in lesson.pictures {
                self.picture(ui, picture);
                ui.add_space(10.0);
            }

            ui.separator();
            go_to = self.pager(ui, tab, (t, l));
            ui.add_space(8.0);
        });

        if let Some((nt, nl)) = go_to {
            self.selected[slot] = (nt, nl);
            self.reveal = Some((tab, nt));
            self.scroll_to_top = true;
        }
    }

    /// A picture, as wide as the lesson (and no wider than it is), its caption under
    /// it. Its texture is made the first time it is shown, and kept.
    fn picture(&mut self, ui: &mut egui::Ui, picture: &'static Picture) {
        let ctx = ui.ctx().clone();
        let texture = self.pictures.entry(picture.name).or_insert_with(|| {
            let image = eframe::icon_data::from_png_bytes(picture.png).ok()?;
            let size = [image.width as usize, image.height as usize];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba);
            Some(ctx.load_texture(picture.name, image, egui::TextureOptions::LINEAR))
        });
        if let Some(texture) = texture {
            let size = texture.size_vec2();
            let width = size.x.min(ui.available_width());
            let shown = egui::vec2(width, size.y * width / size.x);
            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                texture.id(),
                shown,
            )));
        }
        ui.weak(picture.caption);
    }

    /// A command to copy: what it does over it, the command in a box, Copy at its
    /// right.
    fn snippet(&mut self, ui: &mut egui::Ui, snippet: &Snippet) {
        ui.label(RichText::new(snippet.caption).strong());
        ui.horizontal_top(|ui| {
            let room = (ui.available_width() - COPY_ROOM).max(120.0);
            code_frame(ui).show(ui, |ui| {
                // (The frame's margins are inside the room it is given.)
                ui.set_width(room - 20.0);
                ui.add(
                    egui::Label::new(RichText::new(snippet.text).monospace())
                        .wrap_mode(TextWrapMode::Wrap),
                );
            });
            if ui
                .button("Copy")
                .on_hover_text("Copy it to the clipboard")
                .clicked()
            {
                ui.ctx().copy_text(snippet.text.to_owned());
                self.notice = Some(("Copied.".to_owned(), Instant::now()));
            }
        });
    }
}
