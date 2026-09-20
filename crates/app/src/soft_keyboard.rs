//! Turns what a phone's on-screen keyboard reports into the events egui's text
//! fields understand.
//!
//! The window system (winit) delivers hardware key presses, but not the text a
//! soft keyboard commits or composes. A shell captures those from the Android
//! input-method service and feeds them here as [`Input`]s; the events that
//! come out are added to the frame's raw input (see
//! [`crate::platform::Platform::hook_raw_input`]).
//!
//! egui's `TextEdit` ignores a newline arriving as text (it wants an Enter
//! key press) and has no notion of "replace what the keyboard was composing",
//! so both are spelled out here.

use egui::{Event, Key, Modifiers};

/// One report from the on-screen keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Finished text. Replaces the text being composed, if any.
    Commit(String),
    /// Text still being composed (a word under autocorrect, say). Replaces
    /// the previous composing text.
    Compose(String),
    /// Composition is over; what is showing stays as it is.
    FinishComposing,
    /// Delete this many characters before / after the cursor.
    Delete { before: usize, after: usize },
    /// A key that is not text.
    Key(SoftKey),
}

/// The non-text keys a soft keyboard can press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftKey {
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

impl SoftKey {
    fn egui_key(self) -> Key {
        match self {
            SoftKey::Enter => Key::Enter,
            SoftKey::Backspace => Key::Backspace,
            SoftKey::Delete => Key::Delete,
            SoftKey::Tab => Key::Tab,
            SoftKey::Escape => Key::Escape,
            SoftKey::Left => Key::ArrowLeft,
            SoftKey::Right => Key::ArrowRight,
            SoftKey::Up => Key::ArrowUp,
            SoftKey::Down => Key::ArrowDown,
            SoftKey::Home => Key::Home,
            SoftKey::End => Key::End,
            SoftKey::PageUp => Key::PageUp,
            SoftKey::PageDown => Key::PageDown,
        }
    }
}

/// Remembers how much text the keyboard is still composing, so the next
/// report can replace it.
#[derive(Debug, Default)]
pub struct SoftKeyboard {
    /// Length, in characters, of the composing text currently in the field.
    composing: usize,
}

impl SoftKeyboard {
    /// Append the egui events that `input` amounts to.
    pub fn translate(&mut self, input: Input, events: &mut Vec<Event>) {
        match input {
            Input::Commit(text) => {
                self.erase_composing(events);
                push_text(&text, events);
            }
            Input::Compose(text) => {
                self.erase_composing(events);
                self.composing = text.chars().count();
                push_text(&text, events);
            }
            Input::FinishComposing => self.composing = 0,
            Input::Delete { before, after } => {
                self.composing = 0;
                for _ in 0..before {
                    push_key(SoftKey::Backspace, events);
                }
                for _ in 0..after {
                    push_key(SoftKey::Delete, events);
                }
            }
            Input::Key(key) => {
                self.composing = 0;
                push_key(key, events);
            }
        }
    }

    fn erase_composing(&mut self, events: &mut Vec<Event>) {
        for _ in 0..std::mem::take(&mut self.composing) {
            push_key(SoftKey::Backspace, events);
        }
    }
}

/// `text` as text events, with each newline as the Enter key press egui's
/// text fields expect.
fn push_text(text: &str, events: &mut Vec<Event>) {
    let mut lines = text.split('\n').peekable();
    while let Some(line) = lines.next() {
        if !line.is_empty() {
            events.push(Event::Text(line.to_string()));
        }
        if lines.peek().is_some() {
            push_key(SoftKey::Enter, events);
        }
    }
}

fn push_key(key: SoftKey, events: &mut Vec<Event>) {
    for pressed in [true, false] {
        events.push(Event::Key {
            key: key.egui_key(),
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(inputs: impl IntoIterator<Item = Input>) -> Vec<Event> {
        let mut keyboard = SoftKeyboard::default();
        let mut events = Vec::new();
        for input in inputs {
            keyboard.translate(input, &mut events);
        }
        events
    }

    fn text(s: &str) -> Event {
        Event::Text(s.to_string())
    }

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn committed_text_is_a_text_event() {
        assert_eq!(run([Input::Commit("abc".into())]), vec![text("abc")]);
    }

    #[test]
    fn a_newline_is_an_enter_key_press_not_text() {
        assert_eq!(
            run([Input::Commit("a\nb".into())]),
            vec![
                text("a"),
                key(Key::Enter, true),
                key(Key::Enter, false),
                text("b")
            ]
        );
        // A lone newline (the Enter key of a keyboard that commits it as text).
        assert_eq!(
            run([Input::Commit("\n".into())]),
            vec![key(Key::Enter, true), key(Key::Enter, false)]
        );
    }

    #[test]
    fn composing_text_is_replaced_by_the_next_report() {
        let events = run([
            Input::Compose("he".into()),
            Input::Compose("hel".into()),
            Input::Commit("help".into()),
        ]);
        let backspace = [key(Key::Backspace, true), key(Key::Backspace, false)];
        let mut expected = vec![text("he")];
        // "he" is erased (2 characters) and "hel" typed in its place...
        for _ in 0..2 {
            expected.extend(backspace.clone());
        }
        expected.push(text("hel"));
        // ...then "hel" erased (3) and the committed word typed.
        for _ in 0..3 {
            expected.extend(backspace.clone());
        }
        expected.push(text("help"));
        assert_eq!(events, expected);
    }

    #[test]
    fn composing_is_counted_in_characters_not_bytes() {
        let events = run([Input::Compose("é€".into()), Input::Commit("x".into())]);
        let backspaces = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Event::Key {
                        key: Key::Backspace,
                        pressed: true,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(backspaces, 2);
    }

    #[test]
    fn finishing_composition_keeps_the_text() {
        let events = run([
            Input::Compose("ab".into()),
            Input::FinishComposing,
            Input::Commit("c".into()),
        ]);
        assert_eq!(events, vec![text("ab"), text("c")]);
    }

    #[test]
    fn delete_becomes_backspaces_and_deletes() {
        assert_eq!(
            run([Input::Delete {
                before: 2,
                after: 1
            }]),
            vec![
                key(Key::Backspace, true),
                key(Key::Backspace, false),
                key(Key::Backspace, true),
                key(Key::Backspace, false),
                key(Key::Delete, true),
                key(Key::Delete, false),
            ]
        );
    }

    #[test]
    fn a_key_press_ends_any_composition() {
        let events = run([
            Input::Compose("ab".into()),
            Input::Key(SoftKey::Left),
            Input::Commit("c".into()),
        ]);
        // Nothing is erased for "ab": the arrow key moved on from it.
        assert_eq!(
            events,
            vec![
                text("ab"),
                key(Key::ArrowLeft, true),
                key(Key::ArrowLeft, false),
                text("c")
            ]
        );
    }
}
