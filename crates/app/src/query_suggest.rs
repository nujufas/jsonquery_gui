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
    /// The feature's on/off switch — the query bar's toggle button and
    /// Escape both drive this (see `set_enabled`). Unlike `dismissed`
    /// below, this does *not* clear itself on the next keystroke: once
    /// off, suggestions stay off until the user explicitly turns them back
    /// on. Experimental, so off by default.
    pub enabled: bool,
    /// Set right after accepting a candidate; cleared as soon as the text
    /// or cursor changes again, so accepting doesn't disable the feature
    /// for the rest of the query — just suppresses the immediate
    /// re-suggestion of the thing just accepted (see `accepted`).
    dismissed: bool,
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
    /// Escape as "turn suggestions off" rather than "leave the query box"
    /// means we have to explicitly ask for that focus back afterwards.
    pub fn intercept_keys(&mut self, ctx: &egui::Context, text_edit_id: egui::Id) -> Option<usize> {
        if !self.open || self.items.is_empty() {
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
                self.enabled = false;
                self.open = false;
                self.items.clear();
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
            return;
        }

        let Some(cursor_char) = cursor_char else {
            self.open = false;
            self.items.clear();
            self.last_cursor = None;
            return;
        };

        if text_changed || Some(cursor_char) != self.last_cursor {
            self.dismissed = false;
        }
        self.last_cursor = Some(cursor_char);

        if self.dismissed {
            self.open = false;
            self.items.clear();
            return;
        }

        let cursor_byte = byte_offset_for_char_index(text, cursor_char);
        let items = jsonquery_query::suggest(text, cursor_byte, explicit, doc_root);
        if items != self.items {
            self.selected = 0;
            self.expanded = false;
        }
        self.open = !items.is_empty();
        self.items = items;
    }

    /// Close the popup outright (e.g. the box lost focus).
    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
    }

    /// Turn the whole feature on or off — the query bar's toggle button
    /// calls this directly; Escape (in `intercept_keys`) calls the `false`
    /// half of it. Re-enabling also clears any lingering `dismissed` state
    /// so suggestions resume immediately, reflecting the query as it
    /// stands right now, rather than waiting for the next keystroke.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if enabled {
            self.dismissed = false;
        } else {
            self.close();
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
        self.last_cursor = Some(new_cursor_char);
    }
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
