//! The right-click menu of a text box: Cut, Copy, Paste and Select All in a box
//! that can be typed in, Copy and Select All in one that only shows text.
//!
//! egui gives a text box the keys (Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+A) and nothing
//! else, so the menu is made here. Two things in egui get in its way, and are
//! dealt with here too:
//!
//! - A press of the secondary button puts the caret where it landed, which
//!   throws a selection away even when the press was inside it, and then Copy has
//!   nothing to copy. [`attach`] puts the selection back when that happens.
//! - A menu cannot read the clipboard. Paste is asked of the window system, which
//!   answers the next frame with the event the key makes, for the box that has
//!   the focus. Cut is asked the same way, so that the box does it as it does
//!   for the key (undo, change flag and all). The press on the menu took the
//!   focus from the box, so it is given back first.

use egui::text::{CCursor, CCursorRange};
use egui::widgets::text_edit::TextEditState;
use egui::{Context, Id, Key, KeyboardShortcut, Modifiers, PointerButton, Response, Ui};

/// What a box lets the menu do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Text that can be typed in: Cut, Copy, Paste, Select All.
    Typed,
    /// Text that is only shown: Copy, Select All.
    ReadOnly,
}

/// Give the box whose `TextEdit` gave `response` its right-click menu. `text` is
/// what the box holds, as of the frame (read after the box was drawn).
pub fn attach(response: &Response, text: &str, kind: Kind) {
    menu(response, text, kind, None::<fn(&mut Ui)>);
}

/// The same, with more items under the box's own, after a separator.
pub fn attach_with(response: &Response, text: &str, kind: Kind, more: impl FnOnce(&mut Ui)) {
    menu(response, text, kind, Some(more));
}

fn menu(response: &Response, text: &str, kind: Kind, more: Option<impl FnOnce(&mut Ui)>) {
    keep_selection(response);
    let ctx = response.ctx.clone();
    let id = response.id;
    response.context_menu(|ui| {
        let selected = selection(&ctx, id).map(|range| range.slice_str(text));
        let has_selection = selected.is_some_and(|s| !s.is_empty());
        if kind == Kind::Typed && item(ui, "Cut", Key::X, has_selection) {
            ask(&ctx, id, egui::ViewportCommand::RequestCut);
            ui.close();
        }
        if item(ui, "Copy", Key::C, has_selection) {
            if let Some(selected) = selected {
                ctx.copy_text(selected.to_owned());
            }
            if kind == Kind::Typed {
                // (the press on the item took the focus from the box)
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
            ui.close();
        }
        if kind == Kind::Typed && item(ui, "Paste", Key::V, true) {
            ask(&ctx, id, egui::ViewportCommand::RequestPaste);
            ui.close();
        }
        if item(ui, "Select All", Key::A, !text.is_empty()) {
            select_all(&ctx, id, text);
            ui.close();
        }
        if let Some(more) = more {
            ui.separator();
            more(ui);
        }
    });
}

/// Text that is only shown, in the monospace font, that the mouse selects as
/// in a box (and Ctrl+A, and the menu), because it is one. `wrap` breaks the
/// lines at the width there is; without it they run on, for the scroll area
/// around to scroll sideways. A plain label's selection cannot be kept through
/// a right click, nor read by a menu.
pub fn read_only(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    text: &str,
    wrap: bool,
) {
    shown(ui, id_salt, text, wrap, egui::TextStyle::Monospace, None);
}

/// A message that is only shown (what a tool says is wrong), in `color` and in the
/// font the rest of the window is set in, broken at the width there is.
pub fn read_message(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    text: &str,
    color: egui::Color32,
) {
    shown(ui, id_salt, text, true, egui::TextStyle::Body, Some(color));
}

fn shown(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    text: &str,
    wrap: bool,
    style: egui::TextStyle,
    color: Option<egui::Color32>,
) {
    let mut shown = text;
    let color = color.unwrap_or_else(|| ui.visuals().text_color());
    let font_style = style.clone();
    let mut no_wrap = |ui: &Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
        let font_id = font_style.resolve(ui.style());
        let row_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
        let mut job =
            egui::text::LayoutJob::simple(text.as_str().to_owned(), font_id, color, f32::INFINITY);
        job.keep_trailing_whitespace = true;
        for section in &mut job.sections {
            section.format.line_height = Some(row_height + ui.spacing().extra_text_line_spacing);
        }
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let mut edit = egui::TextEdit::multiline(&mut shown)
        .id_salt(id_salt)
        .font(style)
        .text_color(color)
        .frame(egui::Frame::NONE)
        .desired_rows(1)
        .desired_width(f32::INFINITY);
    if !wrap {
        edit = edit.layouter(&mut no_wrap);
    }
    let response = edit.show(ui).response;
    attach(&response, text, Kind::ReadOnly);
}

/// A menu with the one item Copy, for text that is not a box (a label, a row):
/// it puts `text` on the clipboard.
pub fn copy_only(response: &Response, text: &str) {
    let ctx = response.ctx.clone();
    response.context_menu(|ui| {
        if ui.button("Copy").clicked() {
            ctx.copy_text(text.to_owned());
            ui.close();
        }
    });
}

/// An item of the menu, with the key that does the same beside it.
fn item(ui: &mut Ui, label: &str, key: Key, enabled: bool) -> bool {
    let shortcut = ui
        .ctx()
        .format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, key));
    ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut))
        .clicked()
}

/// What is selected in the box with this id, if anything is.
fn selection(ctx: &Context, id: Id) -> Option<CCursorRange> {
    TextEditState::load(ctx, id)
        .and_then(|state| state.cursor.char_range())
        .filter(|range| !range.is_empty())
}

/// Hand the focus back to the box (the press on a menu item took it away) and
/// ask the window system for `command`, which it answers with an event for the
/// box. The window is drawn once more for it to be heard.
fn ask(ctx: &Context, id: Id, command: egui::ViewportCommand) {
    ctx.memory_mut(|memory| memory.request_focus(id));
    ctx.send_viewport_cmd(command);
    ctx.request_repaint();
}

fn select_all(ctx: &Context, id: Id, text: &str) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(0),
        CCursor::new(text.chars().count()),
    )));
    state.store(ctx, id);
    ctx.memory_mut(|memory| memory.request_focus(id));
}

/// A press of the secondary button inside what is selected leaves it selected, as
/// in other programs. (A press outside moves the caret there, as it does.)
fn keep_selection(response: &Response) {
    let ctx = &response.ctx;
    let id = response.id;
    let key = Id::new(("text_menu_selection", id));
    let now = selection(ctx, id);
    let before: Option<CCursorRange> = ctx.data(|data| data.get_temp(key));
    let pressed = ctx.input(|input| input.pointer.button_pressed(PointerButton::Secondary));
    let kept = match (pressed && response.contains_pointer(), before, now) {
        (true, Some(before), None) => {
            let caret = TextEditState::load(ctx, id)
                .and_then(|state| state.cursor.char_range())
                .map(|range| range.primary.index);
            let inside = caret.is_some_and(|caret| {
                let span = before.as_sorted_char_range();
                span.start <= caret && caret <= span.end
            });
            if inside {
                let mut state = TextEditState::load(ctx, id).unwrap_or_default();
                state.cursor.set_char_range(Some(before));
                state.store(ctx, id);
                Some(before)
            } else {
                None
            }
        }
        _ => now,
    };
    ctx.data_mut(|data| {
        if let Some(range) = kept {
            data.insert_temp(key, range);
        } else {
            data.remove_temp::<CCursorRange>(key);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2};

    const BOX: &str = "box";

    /// One text box in a window with nothing else, driven a frame at a time in a
    /// plain `egui::Context`.
    struct Bench {
        ctx: Context,
        text: String,
        kind: Kind,
        time: f64,
        shapes: Vec<egui::epaint::ClippedShape>,
        copied: Vec<String>,
        commands: Vec<egui::ViewportCommand>,
    }

    impl Bench {
        fn new(text: &str, kind: Kind) -> Self {
            Self::on(egui::os::OperatingSystem::Nix, text, kind)
        }

        /// The same on a system that writes the keys its own way (the build's is not the test's).
        fn on(os: egui::os::OperatingSystem, text: &str, kind: Kind) -> Self {
            let ctx = Context::default();
            ctx.set_os(os);
            let mut bench = Self {
                ctx,
                text: text.to_owned(),
                kind,
                time: 0.0,
                shapes: Vec::new(),
                copied: Vec::new(),
                commands: Vec::new(),
            };
            bench.settle();
            bench
        }

        fn frame_with(&mut self, events: Vec<Event>) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            self.time += 1.0 / 60.0;
            let (text, kind) = (&mut self.text, self.kind);
            let mut output = self.ctx.run_ui(input, |ui| {
                let response = egui::TextEdit::multiline(text)
                    .id(Id::new(BOX))
                    .desired_width(300.0)
                    .desired_rows(3)
                    .show(ui)
                    .response;
                attach(&response, text, kind);
            });
            output.textures_delta.clear();
            self.shapes = output.shapes;
            for viewport in output.viewport_output.values() {
                self.commands.extend(viewport.commands.iter().cloned());
            }
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

        fn button(&mut self, at: Pos2, button: PointerButton, pressed: bool) {
            self.frame_with(vec![Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }

        fn click_with(&mut self, at: Pos2, which: PointerButton) {
            self.frame_with(vec![Event::PointerMoved(at)]);
            self.button(at, which, true);
            self.button(at, which, false);
            self.settle();
        }

        fn click(&mut self, at: Pos2) {
            self.click_with(at, PointerButton::Primary);
        }

        fn right_click(&mut self, at: Pos2) {
            self.click_with(at, PointerButton::Secondary);
        }

        /// Where `label` is drawn (the first piece of text that is just that).
        fn at(&self, label: &str) -> Option<Pos2> {
            fn walk(shape: &egui::Shape, label: &str) -> Option<Pos2> {
                match shape {
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| walk(s, label)),
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                    }
                    _ => None,
                }
            }
            self.shapes.iter().find_map(|s| walk(&s.shape, label))
        }

        fn is_open(&self) -> bool {
            self.at("Select All").is_some()
        }

        fn select(&mut self, from: usize, to: usize) {
            let id = Id::new(BOX);
            let mut state = TextEditState::load(&self.ctx, id).unwrap_or_default();
            state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(from),
                CCursor::new(to),
            )));
            state.store(&self.ctx, id);
            self.ctx.memory_mut(|m| m.request_focus(id));
            self.settle();
        }

        fn selected(&self) -> Option<String> {
            selection(&self.ctx, Id::new(BOX)).map(|range| range.slice_str(&self.text).to_owned())
        }

        fn has_focus(&self) -> bool {
            self.ctx.memory(|m| m.has_focus(Id::new(BOX)))
        }

        /// The first letters of the first line, where a press lands on its text.
        const TEXT: Pos2 = Pos2::new(12.0, 10.0);
        /// Far to the right of the first line: the end of it.
        const END: Pos2 = Pos2::new(280.0, 10.0);
    }

    #[test]
    fn a_right_click_inside_what_is_selected_leaves_it_selected() {
        let mut b = Bench::new("hello world", Kind::Typed);
        b.select(0, 5);
        b.right_click(Bench::TEXT);
        assert_eq!(b.selected().as_deref(), Some("hello"));
        assert!(b.is_open(), "{:?}", b.shapes.len());
    }

    #[test]
    fn a_right_click_outside_what_is_selected_puts_the_caret_there() {
        let mut b = Bench::new("hello world", Kind::Typed);
        b.select(0, 5);
        b.right_click(Bench::END);
        assert_eq!(b.selected(), None);
        assert!(b.is_open());
    }

    #[test]
    fn a_left_click_inside_what_is_selected_still_moves_the_caret() {
        let mut b = Bench::new("hello world", Kind::Typed);
        b.select(0, 5);
        b.click(Bench::TEXT);
        assert_eq!(b.selected(), None);
        assert!(!b.is_open());
    }

    #[test]
    fn copy_puts_what_is_selected_on_the_clipboard() {
        let mut b = Bench::new("hello world", Kind::Typed);
        b.select(6, 11);
        b.right_click(Pos2::new(60.0, 10.0));
        let copy = b.at("Copy").expect("the menu has Copy");
        b.click(copy);
        assert_eq!(b.copied, ["world"]);
        assert!(!b.is_open(), "the menu closes");
        assert_eq!(b.text, "hello world", "copying leaves the text");
    }

    #[test]
    fn without_a_selection_there_is_nothing_to_copy_or_cut() {
        let mut b = Bench::new("hello world", Kind::Typed);
        for label in ["Copy", "Cut"] {
            b.right_click(Bench::END);
            let at = b.at(label).expect(label);
            b.click(at);
            // (a click on a disabled item only closes the menu)
            assert!(!b.is_open(), "{label}");
        }
        assert!(b.copied.is_empty());
        assert!(!b.commands.contains(&egui::ViewportCommand::RequestCut));
        assert_eq!(b.text, "hello world");
    }

    #[test]
    fn cut_asks_for_a_cut_and_the_box_does_it_with_the_answer() {
        let mut b = Bench::new("hello world", Kind::Typed);
        b.select(0, 6);
        b.right_click(Bench::TEXT);
        let cut = b.at("Cut").expect("the menu has Cut");
        b.click(cut);
        assert!(
            b.commands.contains(&egui::ViewportCommand::RequestCut),
            "{:?}",
            b.commands
        );
        assert!(b.has_focus(), "the box has the focus back for the answer");
        // What the window system answers with, as the key Ctrl+X makes it.
        b.frame_with(vec![Event::Cut]);
        b.settle();
        assert_eq!(b.text, "world");
        assert_eq!(b.copied, ["hello "]);
    }

    #[test]
    fn paste_asks_for_a_paste_and_the_box_takes_the_answer_in_place_of_the_selection() {
        let mut b = Bench::new("hello world", Kind::Typed);
        b.select(6, 11);
        b.right_click(Pos2::new(60.0, 10.0));
        let paste = b.at("Paste").expect("the menu has Paste");
        b.click(paste);
        assert!(
            b.commands.contains(&egui::ViewportCommand::RequestPaste),
            "{:?}",
            b.commands
        );
        assert!(b.has_focus());
        b.frame_with(vec![Event::Paste("there".to_owned())]);
        b.settle();
        assert_eq!(b.text, "hello there");
    }

    #[test]
    fn paste_is_offered_with_nothing_selected() {
        let mut b = Bench::new("", Kind::Typed);
        b.right_click(Bench::TEXT);
        let paste = b.at("Paste").expect("the menu has Paste");
        b.click(paste);
        assert!(b.commands.contains(&egui::ViewportCommand::RequestPaste));
        b.frame_with(vec![Event::Paste("{}".to_owned())]);
        b.settle();
        assert_eq!(b.text, "{}");
    }

    #[test]
    fn select_all_selects_the_whole_text() {
        let mut b = Bench::new("one\ntwo", Kind::Typed);
        b.right_click(Bench::TEXT);
        let all = b.at("Select All").expect("the menu has Select All");
        b.click(all);
        assert_eq!(b.selected().as_deref(), Some("one\ntwo"));
        assert!(b.has_focus());
        assert!(!b.is_open());
    }

    #[test]
    fn a_box_that_only_shows_text_has_no_cut_or_paste() {
        let mut b = Bench::new("hello world", Kind::ReadOnly);
        b.select(0, 5);
        b.right_click(Bench::TEXT);
        assert!(b.at("Copy").is_some() && b.at("Select All").is_some());
        assert!(b.at("Cut").is_none() && b.at("Paste").is_none());
        let copy = b.at("Copy").unwrap();
        b.click(copy);
        assert_eq!(b.copied, ["hello"]);
    }

    #[test]
    fn the_items_show_the_keys_that_do_the_same_as_the_system_writes_them() {
        use egui::os::OperatingSystem;
        for (os, keys) in [
            (
                OperatingSystem::Nix,
                ["Ctrl+X", "Ctrl+C", "Ctrl+V", "Ctrl+A"],
            ),
            (
                OperatingSystem::Windows,
                ["Ctrl+X", "Ctrl+C", "Ctrl+V", "Ctrl+A"],
            ),
            (OperatingSystem::Mac, ["Cmd+X", "Cmd+C", "Cmd+V", "Cmd+A"]),
        ] {
            let mut b = Bench::on(os, "x", Kind::Typed);
            b.right_click(Bench::TEXT);
            for key in keys {
                assert!(b.at(key).is_some(), "{os:?}: {key}");
            }
        }
    }
}
