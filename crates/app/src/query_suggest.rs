//! Egui-facing state for the query box's autocomplete popup: which
//! candidates (from `jsonquery_query::suggest`) are current, which one is
//! keyboard-selected, and the plumbing to intercept navigation keys before
//! the `TextEdit` widget itself would otherwise consume them. Rendering and
//! wiring live in `App::query_text_edit` (`app.rs`); this module only holds
//! state and the pieces that don't need the rest of `App`.

use eframe::egui;
use jsonquery_query::{Kind, Suggestion};
use serde_json::Value;

/// Per-frame state for the query box's suggestion popup.
#[derive(Default)]
pub struct QuerySuggest {
    pub open: bool,
    pub items: Vec<Suggestion>,
    pub selected: usize,
    /// Whether the popup should render its full `items` list (scrollable)
    /// rather than just the collapsed preview — set by clicking the "…
    /// more" row, or implicitly once keyboard navigation selects past the
    /// preview (see `App::query_text_edit`).
    pub expanded: bool,
    /// The feature's on/off switch — driven only by the query bar's toggle
    /// button (see `set_enabled`). Unlike `dismissed` below, this does *not*
    /// clear itself on the next keystroke: once off, suggestions stay off
    /// until the user explicitly turns them back on. Experimental, so off by
    /// default.
    pub enabled: bool,
    /// Set right after accepting a candidate, and by Escape; cleared as soon
    /// as the text or cursor changes again. So neither disables the feature
    /// for the rest of the query — accepting just suppresses the immediate
    /// re-suggestion of the thing just accepted (see `accepted`), and Escape
    /// just puts away the list that's showing until there's something new to
    /// suggest (see `dismiss`).
    dismissed: bool,
    /// `recompute` found a non-empty candidate list but held it back
    /// because the cursor sits right after a whitespace character (see
    /// `cursor_after_whitespace`) — set alongside `open = false` instead of
    /// `open = true` for that one case. Lets Escape distinguish "nothing to
    /// suggest here" (does nothing, same as always) from "there's a list,
    /// just not shown automatically" (reveals it — see `intercept_keys`).
    suppressed: bool,
    /// Set by an Escape-triggered reveal (`intercept_keys`) to override
    /// `suppressed` for the candidates at the *current* cursor position;
    /// cleared the moment the text or cursor changes again, same lifetime
    /// as `dismissed`. Whitespace suppression is opt-in per position, not a
    /// lasting mode — landing on the next whitespace-adjacent spot goes
    /// back to suppressed until asked for again.
    revealed: bool,
    last_cursor: Option<usize>,
    /// The row count the popup's `egui::Area` was last sized for (see
    /// `App::query_text_edit`). `egui::Area` remembers its rect by `Id`
    /// across frames — including across a full close/reopen cycle, since
    /// nothing clears that memory just because the popup wasn't drawn for a
    /// few frames — so reopening with a *different* number of rows than
    /// whatever the Area last happened to be sized for (e.g. a 3-item
    /// popup, then later a 50-item one) would otherwise silently stay
    /// clamped to the old, wrong size forever: `Ui::set_min_height` only
    /// changes what the frame *reports* as its size, not how much room its
    /// children (the `ScrollArea`) actually get to draw into this frame,
    /// which is still bounded by the Area's stale remembered rect. Tracking
    /// this lets the renderer ask egui for a one-frame invisible "sizing
    /// pass" (`Area::sizing_pass`) exactly when the target row count
    /// changes, so the Area re-measures instead of reusing a stale rect.
    pub popup_sized_for_rows: Option<usize>,
}

impl QuerySuggest {
    /// If the popup is open, consume Up/Down/Enter/Tab/Escape from the
    /// input queue *before* the `TextEdit` widget is shown this frame, so
    /// those keys drive the popup instead of moving the text cursor,
    /// inserting a newline/tab, or doing nothing. Returns the index into
    /// `self.items` to accept, if Enter/Tab was just consumed.
    ///
    /// `text_edit_id` is the query box's own widget id: egui's `Memory`
    /// unconditionally clears keyboard focus from *any* widget the instant
    /// it sees an Escape keypress, as part of its own frame-start
    /// processing — before this method (or anything else in `App::update`)
    /// ever runs, so `consume_key` here is too late to prevent it. Handling
    /// Escape as "close this list" rather than "leave the query box" means
    /// we have to explicitly ask for that focus back afterwards.
    pub fn intercept_keys(&mut self, ctx: &egui::Context, text_edit_id: egui::Id) -> Option<usize> {
        if !self.open {
            // Nothing showing. The one thing worth still checking for is a
            // whitespace-suppressed list (`suppressed`, set by `recompute`)
            // waiting on an explicit ask — plain typing never opens the
            // popup right after a space/tab/newline (so Tab is free to
            // indent/align instead of getting hijacked into "accept"), but
            // Escape or Ctrl+Space can still ask for it this once. Ctrl+Space
            // is the conventional "trigger autocomplete" shortcut, offered
            // alongside Escape since Escape doubles as "leave the query box"
            // muscle memory that some users won't want to press just to see
            // suggestions. Any other key falls through to the widget exactly
            // as if this method didn't exist.
            if self.suppressed && !self.items.is_empty() {
                let revealed = ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Space)
                });
                if revealed {
                    self.reveal();
                    ctx.memory_mut(|m| m.request_focus(text_edit_id));
                }
            }
            return None;
        }
        if self.items.is_empty() {
            return None;
        }
        let mut escaped = false;
        let accepted = ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                self.selected = (self.selected + 1) % self.items.len();
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                self.selected = (self.selected + self.items.len() - 1) % self.items.len();
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.dismiss();
                escaped = true;
            }
            let accept = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab);
            accept.then_some(self.selected)
        });
        if escaped {
            ctx.memory_mut(|m| m.request_focus(text_edit_id));
        }
        accepted
    }

    /// Recompute the candidate list for this frame from scratch. `cursor_char`
    /// is the `TextEdit`'s current char-offset cursor position (`None` if it
    /// doesn't have focus, or has no cursor yet). `text_changed` is whether
    /// the `TextEdit` widget itself edited `text` this frame (a
    /// keyboard-driven `apply` splices `text` *before* the widget runs, so
    /// that case is reported separately by the caller re-checking the
    /// cursor).
    ///
    /// A non-empty list is *computed* the same way regardless of what's
    /// right before the cursor, but isn't auto-*opened* when that's a
    /// whitespace character (space/tab/newline/…) — see
    /// `cursor_after_whitespace` — unless `reveal` was just called for this
    /// exact position. Without this, typing a space or tab to line up parts
    /// of a query reliably pops the keyword-dump list open (any word
    /// boundary is a valid, if empty, candidate site), and the very next
    /// Tab — pressed to keep indenting — gets eaten as "accept" instead of
    /// inserting another tab character.
    pub fn recompute(
        &mut self,
        text: &str,
        cursor_char: Option<usize>,
        text_changed: bool,
        explicit: Option<Kind>,
        doc_root: Option<&Value>,
    ) {
        if !self.enabled {
            self.open = false;
            self.items.clear();
            self.suppressed = false;
            return;
        }

        let Some(cursor_char) = cursor_char else {
            self.open = false;
            self.items.clear();
            self.suppressed = false;
            self.last_cursor = None;
            return;
        };

        if text_changed || Some(cursor_char) != self.last_cursor {
            self.dismissed = false;
            self.revealed = false;
        }
        self.last_cursor = Some(cursor_char);

        if self.dismissed {
            self.open = false;
            self.items.clear();
            self.suppressed = false;
            return;
        }

        let cursor_byte = byte_offset_for_char_index(text, cursor_char);
        let items = jsonquery_query::suggest(text, cursor_byte, explicit, doc_root);
        if items != self.items {
            self.selected = 0;
            self.expanded = false;
        }
        let suppress =
            !items.is_empty() && !self.revealed && cursor_after_whitespace(text, cursor_char);
        self.open = !items.is_empty() && !suppress;
        self.suppressed = suppress;
        self.items = items;
    }

    /// Close the popup outright (e.g. the box lost focus).
    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.suppressed = false;
    }

    /// Show a whitespace-suppressed list anyway (Escape, while `suppressed`
    /// — see `recompute`). Lasts only for the cursor position it was called
    /// at: `revealed` is cleared the moment the text or cursor changes,
    /// same lifetime as `dismissed`, so the next whitespace-adjacent spot
    /// needs its own explicit Escape.
    fn reveal(&mut self) {
        self.open = true;
        self.suppressed = false;
        self.revealed = true;
    }

    /// Put away the list that's showing (Escape), without turning the
    /// feature off: it stays closed while the text and cursor stay put, and
    /// the next keystroke or cursor move computes a fresh list — which shows
    /// as soon as there is anything to suggest. Same suppression as
    /// `accepted`, minus the cursor bookkeeping: nothing moved, so
    /// `last_cursor` (set by the `recompute` that opened the list) is
    /// already where the cursor is.
    pub fn dismiss(&mut self) {
        self.close();
        self.dismissed = true;
    }

    /// Turn the whole feature on or off — the query bar's toggle button
    /// calls this directly. Re-enabling also clears any lingering
    /// `dismissed` state so suggestions resume immediately, reflecting the
    /// query as it stands right now, rather than waiting for the next
    /// keystroke.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if enabled {
            self.dismissed = false;
        } else {
            self.close();
            self.revealed = false;
        }
    }

    /// Close the popup after accepting `new_cursor_char` as the candidate
    /// just spliced in, and keep it closed through the next `recompute`
    /// rather than reopening immediately. Without this, a fully-typed word
    /// like "users" still prefix-matches itself, so the next frame's
    /// `recompute` — seeing the cursor move to just past the accepted
    /// text, same as any other cursor move — would treat that as "resume
    /// suggesting" and reopen the popup showing the very candidate that
    /// was just accepted.
    pub fn accepted(&mut self, new_cursor_char: usize) {
        self.close();
        self.dismissed = true;
        self.revealed = false;
        self.last_cursor = Some(new_cursor_char);
    }
}

/// Whether `cursor_char` sits right after a whitespace character (space,
/// tab, newline, …) in `text` — the position `recompute` holds a non-empty
/// candidate list back from auto-opening at, absent an explicit `reveal`.
fn cursor_after_whitespace(text: &str, cursor_char: usize) -> bool {
    cursor_char > 0
        && text
            .chars()
            .nth(cursor_char - 1)
            .is_some_and(char::is_whitespace)
}

/// Splice `item` into `text` in place of the range it replaces, returning
/// the char-offset cursor position immediately after the inserted text.
pub fn apply_suggestion(text: &mut String, item: &Suggestion) -> usize {
    let chars_before = text[..item.replace.start].chars().count();
    text.replace_range(item.replace.clone(), &item.insert);
    chars_before + item.insert.chars().count()
}

fn byte_offset_for_char_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggesting() -> QuerySuggest {
        QuerySuggest {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn dismiss_puts_the_list_away_without_turning_the_feature_off() {
        let mut s = suggesting();
        s.recompute(".a | so", Some(7), true, None, None);
        assert!(s.open);

        s.dismiss();
        assert!(s.enabled, "Escape must not switch autocomplete off");
        assert!(!s.open && s.items.is_empty());

        // Frames while the box just sits there — same text, same cursor — must
        // not bring the list straight back.
        for _ in 0..3 {
            s.recompute(".a | so", Some(7), false, None, None);
            assert!(!s.open);
        }
    }

    #[test]
    fn typing_after_a_dismiss_brings_suggestions_back() {
        let mut s = suggesting();
        s.recompute(".a | so", Some(7), true, None, None);
        s.dismiss();

        s.recompute(".a | sor", Some(8), true, None, None);
        assert!(s.open, "the next keystroke should suggest again");
        assert!(s.items.iter().any(|i| i.label.starts_with("sort")));
    }

    #[test]
    fn a_dismiss_lasts_only_until_something_can_be_suggested_again() {
        let mut s = suggesting();
        s.recompute(".a | so", Some(7), true, None, None);
        s.dismiss();

        // Nothing matches yet — still closed, and no longer "dismissed"...
        s.recompute(".a | soz", Some(8), true, None, None);
        assert!(!s.open);
        // ...so as soon as there is a match, it opens.
        s.recompute(".a | so", Some(7), true, None, None);
        assert!(s.open);
    }

    #[test]
    fn moving_the_cursor_after_a_dismiss_suggests_again() {
        let mut s = suggesting();
        s.recompute(".a | so ", Some(7), true, None, None);
        s.dismiss();

        s.recompute(".a | so ", Some(6), false, None, None);
        assert!(s.open);
    }

    #[test]
    fn whitespace_right_before_the_cursor_computes_but_does_not_auto_open() {
        let mut s = suggesting();
        s.recompute(".a | so", Some(7), true, None, None);
        assert!(s.open && !s.suppressed);

        // Typing a space after "so" would, on its own, still match the
        // empty-word keyword-dump fallback — but auto-opening it here is
        // exactly what used to hijack the next Tab (pressed to keep
        // indenting/aligning the query) into "accept" instead.
        s.recompute(".a | so ", Some(8), true, None, None);
        assert!(
            !s.open,
            "auto-suggest must not pop up right after whitespace"
        );
        assert!(s.suppressed, "the list is held back, not thrown away");
        assert!(!s.items.is_empty(), "it's still there for Escape to reveal");
    }

    #[test]
    fn a_non_whitespace_position_is_never_suppressed() {
        let mut s = suggesting();
        s.recompute(".a | so", Some(7), true, None, None);
        assert!(s.open && !s.suppressed);
    }

    #[test]
    fn reveal_shows_a_whitespace_suppressed_list_once() {
        let mut s = suggesting();
        s.recompute(".a | so ", Some(8), true, None, None);
        assert!(!s.open && s.suppressed);

        s.reveal();
        assert!(s.open && !s.suppressed, "Escape asked for it explicitly");

        // Same position, nothing typed: the reveal holds.
        s.recompute(".a | so ", Some(8), false, None, None);
        assert!(s.open, "revealed list stays open at the same position");
    }

    #[test]
    fn a_reveal_does_not_outlive_the_position_it_was_asked_for() {
        let mut s = suggesting();
        s.recompute(".a | so ", Some(8), true, None, None);
        s.reveal();
        assert!(s.open);

        // Typing more moves to a new whitespace-adjacent position, which
        // goes back to suppressed-by-default — a reveal is a one-time
        // "yes, show it here", not a lasting mode change.
        s.recompute(".a | so  ", Some(9), true, None, None);
        assert!(!s.open && s.suppressed);
    }

    #[test]
    fn the_toggle_stays_the_only_way_to_turn_it_off() {
        let mut s = suggesting();
        s.recompute(".a | so", Some(7), true, None, None);
        s.set_enabled(false);
        assert!(!s.enabled && !s.open);
        s.recompute(".a | sor", Some(8), true, None, None);
        assert!(!s.open, "off means off, whatever is typed");
        s.set_enabled(true);
        s.recompute(".a | sor", Some(8), false, None, None);
        assert!(s.open);
    }
}
