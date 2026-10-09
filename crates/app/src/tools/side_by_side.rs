//! Diff's side-by-side view: the two documents next to each other, line by line,
//! as a file-comparison tool shows two files. What both have is on one line on
//! both sides; what only one has is red (removed) or green (added) with a blank
//! on the other side; what changed is amber on both, with the part of the line
//! that differs picked out. The rows come from `jsonquery_query::diff::compare`
//! already lined up and marked (see `diff/view.rs` there), so this only draws
//! them.
//!
//! Both sides are in one scroll area, a row of the view being a row of both
//! documents, so they can't drift apart; the text scrolls sideways together, with
//! a bar of its own under the rows (or Shift+wheel). Beside the scroll bar is an
//! overview of the whole document with a tick for each difference, that can be
//! clicked or dragged. Previous and Next step from one difference to the next,
//! and "Differences only" folds what is the same into a line that says how many
//! lines it hides.
//!
//! Moving. Between the two documents, at the first line of each difference, are
//! two arrows, one over the other: one press moves the difference into the document
//! the arrow points at. A line is picked with a click (Ctrl or Command adds one or
//! takes it away, Shift goes on from the last, dragging runs over several), and then
//! the arrows — and the buttons of the command row, and the menu of a line — move
//! only the lines that were picked. A value is picked whole, whatever its lines: half
//! an object is no JSON. Each document has a Save… over its column.
//!
//! Editing. A double click on a line puts a caret in it, on either side (a single click
//! only picks, so that several lines can be picked): what is typed takes the place of the
//! line when Enter is pressed or the caret goes elsewhere, and Esc puts back what was
//! there (see `line_edit.rs` for what typing over a line comes to). The comparison is made
//! again, and the document is said to be changed.

use std::collections::BTreeSet;
use std::ops::Range;

use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Key, Layout, Modifiers, Pos2, Rect, RichText,
    Sense, Shape, Stroke, StrokeKind, TextStyle,
};
use jsonquery_query::diff::{Block, Mark, Row, Side, SideBySide};

use super::line_edit::{self, Edit, Place};
use super::widgets::{tint, Tint, ERROR};
use crate::pane_header;

/// Height of a row: a monospace line and a hair.
const ROW: f32 = 17.0;
/// Space between the two documents: room for the arrows of a difference, which are
/// one over the other.
const GUTTER: f32 = 22.0;
/// The width of an arrow: it is as high as a row, and the second is in the row below
/// the first.
const ARROW_W: f32 = 16.0;
/// Width of the overview, and the room between it and the rows.
const OVERVIEW: f32 = 10.0;
const OVERVIEW_GAP: f32 = 4.0;
/// Space between the edge of a side and its text.
const TEXT_PAD: f32 = 6.0;
/// Rows of what is the same kept around a difference by "Differences only".
const CONTEXT: usize = 3;
/// Rows kept above a difference that is scrolled to.
const LEAD: f32 = 2.0;

/// What a line of the list on screen is: a row of the view, or a fold that
/// stands for rows that are the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Row(u32),
    Fold { first: u32, count: u32 },
}

impl Item {
    /// The first row of the view this is.
    fn first(self) -> u32 {
        match self {
            Item::Row(row) | Item::Fold { first: row, .. } => row,
        }
    }
}

/// What is on screen, in order: every row, or with "Differences only" the rows
/// near a difference, the rest folded. A view with no difference is shown whole.
fn items(view: &SideBySide, differences_only: bool) -> Vec<Item> {
    let len = view.rows.len();
    if !differences_only || view.blocks.is_empty() {
        return (0..len as u32).map(Item::Row).collect();
    }
    let mut near = vec![false; len];
    for block in &view.blocks {
        let from = block.start.saturating_sub(CONTEXT);
        let to = (block.end + CONTEXT).min(len);
        near[from..to].fill(true);
    }
    let mut items = Vec::new();
    let mut row = 0;
    while row < len {
        if near[row] {
            items.push(Item::Row(row as u32));
            row += 1;
        } else {
            let first = row;
            while row < len && !near[row] {
                row += 1;
            }
            items.push(Item::Fold {
                first: first as u32,
                count: (row - first) as u32,
            });
        }
    }
    items
}

/// Where in the list on screen `row` of the view is (a folded row is at its
/// fold).
fn position_of(items: &[Item], row: usize) -> usize {
    items
        .partition_point(|item| item.first() as usize <= row)
        .saturating_sub(1)
}

/// The block that has `row` in it.
fn block_at(blocks: &[Block], row: usize) -> Option<usize> {
    let at = blocks.partition_point(|b| b.end <= row);
    blocks.get(at).filter(|b| b.start <= row).map(|_| at)
}

/// Which characters of two lines differ: what is left when the start and the end
/// they have in common are taken off each.
fn differing(a: &str, b: &str) -> (Range<usize>, Range<usize>) {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    (prefix..a.len() - suffix, prefix..b.len() - suffix)
}

/// The two halves of a row `width` wide, with the gutter between them.
fn halves(rect: Rect) -> (Rect, Rect) {
    let half = ((rect.width() - GUTTER) / 2.0).floor().max(0.0);
    (
        Rect::from_min_size(rect.min, egui::vec2(half, rect.height())),
        Rect::from_min_size(
            Pos2::new(rect.max.x - half, rect.min.y),
            egui::vec2(half, rect.height()),
        ),
    )
}

/// What to move from one document into the other: which differences (by number, in
/// runs; see `diff::take_selected`), where in the view they are (the difference that is
/// shown as picked while they are moved), and into which document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Move {
    pub changes: Vec<Range<u32>>,
    pub block: usize,
    pub into: Side,
    /// These are lines that were picked, and not a whole difference.
    pub picked: bool,
}

/// What the view says over one of the two documents.
pub(super) struct Doc {
    /// What it is called (a file, the open document, and that a move changed it), if
    /// it is called anything.
    pub name: Option<String>,
    /// A move or an edit has changed it, and it has not been saved since.
    pub changed: bool,
    /// Whether this view can save it: it is a text that is in its box.
    pub can_save: bool,
}

/// What the view remembers between frames. It is made again (with
/// [`Viewer::reset`]) for each new comparison.
#[derive(Default)]
pub(super) struct Viewer {
    differences_only: bool,
    /// How far the text is scrolled to the left, in points, on both sides.
    h_off: f32,
    /// The difference Previous and Next are on.
    current: Option<usize>,
    /// The first item on screen, as of the last frame.
    top: usize,
    /// An item to scroll to at the next frame.
    jump: Option<usize>,
    /// What is on screen, and whether it was made for "Differences only".
    items: Vec<Item>,
    items_for: Option<bool>,
    /// The differences (by number) whose lines are picked. A value is picked whole,
    /// however many lines it has.
    selected: BTreeSet<u32>,
    /// How many lines of the view those are.
    selected_rows: usize,
    /// The line a Shift-click, or a drag, goes on from.
    anchor: Option<usize>,
    /// A drag over the lines, begun on this one, and the last line it was over.
    dragging: Option<(usize, usize)>,
    /// The line that has a caret in it: what is typed there is not yet in the document.
    editor: Option<Editor>,
    /// Why the line that was clicked was not opened for typing, to be said.
    refusal: Option<&'static str>,
    /// Where the arrows were drawn at the last frame, for the tests.
    #[cfg(test)]
    pub arrows: Vec<(usize, Side, Rect)>,
}

/// A line that has a caret in it.
struct Editor {
    side: Side,
    /// Which row of the view it is on.
    row: usize,
    /// Which line of its document it is, from 1.
    line: usize,
    place: Place,
    /// The line as it was, to know whether anything was typed.
    was: String,
    /// What is in the field.
    text: String,
    /// What is wrong with it, when it was tried and was not taken.
    problem: Option<String>,
    /// The field takes the focus (it has just come up, or its text was not taken), and
    /// puts the caret here, in characters, if it is said.
    wants_focus: Option<Option<usize>>,
    /// Whether it has had the focus: only then is losing it the end of the editing.
    had_focus: bool,
    /// Where the caret was when the view last made sure it was in sight, in characters.
    seen_at: Option<usize>,
    /// The frames the field has gone without the focus it asked for.
    tries: u8,
}

/// A change to a line, typed, for the page to ask the worker to make.
pub(super) struct Typed {
    pub edit: Edit,
    /// Which line of the document, from 1.
    pub line: usize,
}

/// What the view has to tell the page.
pub(super) struct Shown {
    /// The path of a line, to copy: its menu asked for it.
    pub copy_path: Option<String>,
    /// Differences that were asked, from an arrow, a menu or a line's menu, to be
    /// moved into the left document or the right one.
    pub moved: Option<Move>,
    /// A document that was asked to be saved.
    pub save: Option<Side>,
    /// A line that was typed over, and taken.
    pub typed: Option<Typed>,
}

/// What the rows of one frame tell: a click, a drag, a menu.
#[derive(Default)]
struct Touched {
    /// A click on a row.
    clicked: Option<usize>,
    /// A double click on a row, and where on screen: the second click of it, which is not a
    /// click of its own.
    double_clicked: Option<(usize, Pos2)>,
    right_clicked: Option<usize>,
    drag_from: Option<usize>,
    /// An arrow was pressed: at which difference, pointing to which side.
    arrow: Option<(usize, Side)>,
    menu: Option<Menu>,
}

enum Menu {
    Move(Side),
    CopyPath(String),
}

/// The text of a row on one of the two sides, if that side has a line there.
fn column(row: &Row, side: Side) -> Option<&str> {
    match side {
        Side::Left => row.left.as_deref(),
        Side::Right => row.right.as_deref(),
    }
}

/// The numbers in `sorted` (rising) as the fewest runs.
fn runs_of(sorted: impl IntoIterator<Item = u32>) -> Vec<Range<u32>> {
    let mut runs: Vec<Range<u32>> = Vec::new();
    for number in sorted {
        match runs.last_mut() {
            Some(last) if last.end == number => last.end += 1,
            _ => runs.push(number..number + 1),
        }
    }
    runs
}

/// The row of `drawn` (the rows on screen, with where) that `y` is level with, or
/// the first or the last when it is above or below them all.
fn row_at(drawn: &[(usize, Rect)], y: f32) -> Option<usize> {
    let (first, last) = (drawn.first()?, drawn.last()?);
    if y <= first.1.min.y {
        return Some(first.0);
    }
    if y >= last.1.max.y {
        return Some(last.0);
    }
    drawn
        .iter()
        .find(|(_, rect)| rect.y_range().contains(y))
        .map(|(row, _)| *row)
}

impl Viewer {
    /// A new comparison: forget where the old one was scrolled to, and what was picked.
    pub fn reset(&mut self) {
        let differences_only = self.differences_only;
        *self = Self {
            differences_only,
            ..Self::default()
        };
    }

    /// The comparison is made again after a line was typed over: forget what was picked,
    /// but stay where the text was scrolled to (the scroll area keeps how far down it was,
    /// as it does while the answer is waited for).
    pub fn reset_keeping_place(&mut self) {
        let (differences_only, h_off) = (self.differences_only, self.h_off);
        *self = Self {
            differences_only,
            h_off,
            ..Self::default()
        };
    }

    fn ensure_items(&mut self, view: &SideBySide) {
        if self.items_for == Some(self.differences_only) && !self.items.is_empty() {
            return;
        }
        // Switching "Differences only" on or off: stay at what is at the top.
        let top = self
            .items
            .get(self.top)
            .filter(|_| self.items_for.is_some())
            .map(|item| item.first() as usize);
        self.items = items(view, self.differences_only);
        self.items_for = Some(self.differences_only);
        if let Some(row) = top {
            self.jump = Some(position_of(&self.items, row) + LEAD as usize);
        }
    }

    /// There is a difference after (or before) the one the view is on.
    pub fn can_step(&self, view: &SideBySide, forward: bool) -> bool {
        self.target(view, forward).is_some()
    }

    /// The difference Previous or Next goes to: the one beside the current one,
    /// or, when none is, the first below (or above) the top of the screen.
    fn target(&self, view: &SideBySide, forward: bool) -> Option<usize> {
        let top = self
            .items
            .get(self.top)
            .map_or(0, |item| item.first() as usize);
        match (self.current, forward) {
            (Some(at), true) => (at + 1 < view.blocks.len()).then_some(at + 1),
            (Some(at), false) => at.checked_sub(1),
            (None, true) => view.blocks.iter().position(|b| b.start >= top),
            (None, false) => view.blocks.iter().rposition(|b| b.start < top),
        }
    }

    /// Go to the next (or the previous) difference. What was picked is let go: the
    /// difference that is stepped to is what a move acts on.
    pub fn step(&mut self, view: &SideBySide, forward: bool) {
        self.ensure_items(view);
        if let Some(at) = self.target(view, forward) {
            self.clear_selection();
            self.go_to(view, at);
        }
    }

    fn go_to(&mut self, view: &SideBySide, block: usize) {
        self.current = Some(block);
        self.jump = Some(position_of(&self.items, view.blocks[block].start));
    }

    /// "Difference 2 of 5", when one is picked.
    pub fn position(&self, view: &SideBySide) -> Option<String> {
        self.current
            .map(|at| format!("Difference {} of {}", at + 1, view.blocks.len()))
    }

    /// What the view says of a line that has a caret in it, for the status bar: what to
    /// do, or what is wrong with what was typed (which is true then); or why the line that
    /// was clicked has no caret.
    pub fn editing_text(&self) -> Option<(String, bool)> {
        if let Some(editor) = &self.editor {
            let which = match editor.side {
                Side::Left => "Left",
                Side::Right => "Right",
            };
            return Some(match &editor.problem {
                Some(problem) => (problem.clone(), true),
                None => (
                    format!(
                        "Editing line {} of {which}: Enter puts it in, Esc puts it back",
                        editor.line
                    ),
                    false,
                ),
            });
        }
        self.refusal.map(|text| (text.to_owned(), false))
    }

    /// Whether something was typed in a line that has a caret in it, and is not taken yet.
    pub fn has_typing(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.text != e.was)
    }

    /// Put a caret in the line of `row` on `side`, `caret` characters in, if that is a line
    /// to type over: what it is is read from the lines of its column.
    fn open_editor(&mut self, view: &SideBySide, row: usize, side: Side, caret: usize) {
        self.refusal = None;
        self.editor = None;
        let Some(shown) = view.rows.get(row).and_then(|r| column(r, side)) else {
            return;
        };
        let lines: Vec<&str> = view.rows.iter().filter_map(|r| column(r, side)).collect();
        let at = view.rows[..row]
            .iter()
            .filter_map(|r| column(r, side))
            .count();
        match line_edit::place_of(&lines, at) {
            Ok(place) => {
                self.editor = Some(Editor {
                    side,
                    row,
                    line: at + 1,
                    place,
                    was: shown.to_owned(),
                    text: shown.to_owned(),
                    problem: None,
                    wants_focus: Some(Some(caret.min(shown.chars().count()))),
                    had_focus: false,
                    seen_at: None,
                    tries: 0,
                });
            }
            Err(why) => self.refusal = why.said(),
        }
    }

    /// The caret leaves its line, with Enter or by a click elsewhere: what was typed is taken
    /// if it is JSON where it is, and is given back as the change to make; if it is not, it
    /// stays, to be put right. When nothing was typed the line is as it was.
    fn finish_editor(&mut self) -> Option<Typed> {
        let mut editor = self.editor.take()?;
        if editor.text == editor.was {
            return None;
        }
        match line_edit::parse(&editor.place, &editor.text) {
            Ok(fragment) => Some(Typed {
                edit: Edit {
                    into: editor.side,
                    path: editor.place.path.clone(),
                    fragment,
                },
                line: editor.line,
            }),
            Err(problem) => {
                editor.problem = Some(problem);
                editor.wants_focus = Some(None);
                editor.had_focus = false;
                editor.tries = 0;
                self.editor = Some(editor);
                None
            }
        }
    }

    /// Whether something can be moved: lines are picked, or a difference is.
    pub fn can_move(&self) -> bool {
        !self.selected.is_empty() || self.current.is_some()
    }

    /// "3 lines picked", when some are.
    pub fn selection_text(&self) -> Option<String> {
        let lines = self.selected_rows;
        (lines > 0).then(|| format!("{lines} line{} picked", if lines == 1 { "" } else { "s" }))
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.selected_rows = 0;
        self.anchor = None;
    }

    pub fn has_selection(&self) -> bool {
        !self.selected.is_empty()
    }

    fn recount(&mut self, view: &SideBySide) {
        self.selected_rows = if self.selected.is_empty() {
            0
        } else {
            view.rows
                .iter()
                .filter(|row| row.change.is_some_and(|c| self.selected.contains(&c)))
                .count()
        };
    }

    /// What the buttons of the command row, the keys and the menu of a line move: the
    /// lines that are picked, or, with none, the difference that is picked.
    pub fn to_move(&self, view: &SideBySide, into: Side) -> Option<Move> {
        if self.selected.is_empty() {
            let block = self.current?;
            let changes = vec![view.changes_of(view.blocks.get(block)?)];
            return Some(Move {
                changes,
                block,
                into,
                picked: false,
            });
        }
        let first = *self.selected.first()?;
        let block = view
            .blocks
            .iter()
            .position(|b| view.changes_of(b).contains(&first))?;
        Some(Move {
            changes: runs_of(self.selected.iter().copied()),
            block,
            into,
            picked: true,
        })
    }

    /// What an arrow beside difference `block` moves: the lines of it that are picked,
    /// or, when none is, all of it.
    pub fn to_move_in(&self, view: &SideBySide, block: usize, into: Side) -> Option<Move> {
        let whole = view.changes_of(view.blocks.get(block)?);
        let inside: Vec<u32> = self.selected.range(whole.clone()).copied().collect();
        let picked = !inside.is_empty();
        let changes = if picked { runs_of(inside) } else { vec![whole] };
        Some(Move {
            changes,
            block,
            into,
            picked,
        })
    }

    /// Pick the lines of `row` as a click on it does: only them; with Ctrl (Command)
    /// those are added to what is picked, or taken from it if they were; with Shift
    /// everything from the line the picking went on from.
    fn click(&mut self, view: &SideBySide, row: usize, modifiers: Modifiers) {
        let add = modifiers.command;
        let change = view.rows.get(row).and_then(|r| r.change);
        if modifiers.shift {
            let from = self.anchor.unwrap_or(row);
            self.pick_between(view, from, row, add);
        } else {
            match change {
                None if !add => self.selected.clear(),
                None => {}
                Some(change) if add => {
                    if !self.selected.remove(&change) {
                        self.selected.insert(change);
                    }
                }
                Some(change) => {
                    self.selected.clear();
                    self.selected.insert(change);
                }
            }
            self.anchor = Some(row);
        }
        if change.is_some() {
            self.current = block_at(&view.blocks, row);
        }
        self.recount(view);
    }

    /// Pick all the lines from `a` to `b`: instead of what was picked, or, with
    /// `add`, as well.
    fn pick_between(&mut self, view: &SideBySide, a: usize, b: usize, add: bool) {
        let end = (a.max(b) + 1).min(view.rows.len());
        let start = a.min(b).min(end);
        if !add {
            self.selected.clear();
        }
        for row in &view.rows[start..end] {
            if let Some(change) = row.change {
                self.selected.insert(change);
            }
        }
    }

    /// A press on a line that is not picked, with the secondary button: it is picked
    /// (alone) before its menu opens, as a file manager does.
    fn press_secondary(&mut self, view: &SideBySide, row: usize) {
        let Some(change) = view.rows.get(row).and_then(|r| r.change) else {
            return;
        };
        if !self.selected.contains(&change) {
            self.selected.clear();
            self.selected.insert(change);
            self.anchor = Some(row);
            self.recount(view);
        }
        self.current = block_at(&view.blocks, row);
    }

    /// The pointer is over `row` with the drag that began on `from` still going.
    fn drag_over(&mut self, view: &SideBySide, from: usize, row: usize) {
        if self.dragging == Some((from, row)) {
            return;
        }
        self.dragging = Some((from, row));
        self.pick_between(view, from, row, false);
        self.anchor = Some(from);
        self.recount(view);
    }

    /// Draw the view in the room that is left. `docs` say what the two documents are
    /// called, whether a move changed them and whether they can be saved.
    pub fn show(&mut self, ui: &mut egui::Ui, view: &SideBySide, docs: [Doc; 2]) -> Shown {
        self.ensure_items(view);
        let items = std::mem::take(&mut self.items);
        #[cfg(test)]
        self.arrows.clear();

        let full = ui.available_rect_before_wrap();
        let header_h = ui.spacing().interact_size.y;
        let solid = egui::style::ScrollStyle::solid();
        let bar = solid.allocated_width();
        let strip_h = bar + 1.0;

        let top = full.min.y + header_h + 2.0;
        let rows_right = full.max.x - OVERVIEW - OVERVIEW_GAP;
        let rows_rect = Rect::from_min_max(
            Pos2::new(full.min.x, top),
            Pos2::new(rows_right, (full.max.y - strip_h).max(top)),
        );
        let strip_rect = Rect::from_min_max(
            Pos2::new(full.min.x, rows_rect.max.y),
            Pos2::new(rows_right, rows_rect.max.y + strip_h),
        );
        let overview_rect = Rect::from_min_max(
            Pos2::new(full.max.x - OVERVIEW, top),
            Pos2::new(full.max.x, rows_rect.max.y),
        );
        // What the rows are laid out in: the scroll area keeps a bar's width at
        // the right (always, so that the header above lines up).
        let inner = Rect::from_min_max(
            rows_rect.min,
            Pos2::new(
                (rows_rect.max.x - bar).max(rows_rect.min.x),
                rows_rect.max.y,
            ),
        );
        let (left_half, right_half) = halves(inner);

        let font = TextStyle::Monospace.resolve(ui.style());
        let glyph = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        // (a line that is being typed over may be longer than all of them)
        let typed_len = self
            .editor
            .as_ref()
            .map_or(0, |e| e.text.chars().count() + 2);
        let text_width = view.width.max(typed_len) as f32 * glyph + 2.0 * TEXT_PAD;
        let max_off = (text_width - left_half.width()).max(0.0);

        // Sideways: the wheel over the rows, or the bar under them.
        if ui.rect_contains_pointer(rows_rect) {
            self.h_off -= ui.input(|i| i.smooth_scroll_delta.x);
        }
        self.h_off = self.h_off.clamp(0.0, max_off);
        let mut strip = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("diff_sbs_strip")
                .max_rect(strip_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        strip.spacing_mut().scroll = solid;
        let sideways = egui::ScrollArea::horizontal()
            .id_salt("diff_sbs_h")
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .horizontal_scroll_offset(self.h_off)
            .show(&mut strip, |ui| {
                ui.allocate_space(egui::vec2(strip_rect.width() + max_off, 1.0));
            });
        self.h_off = sideways.state.offset.x.clamp(0.0, max_off);

        let paint = Paint {
            font,
            h_off: self.h_off,
            text: ui.visuals().text_color(),
            good: tint(ui.visuals(), Tint::Good),
            bad: tint(ui.visuals(), Tint::Bad),
            warn: tint(ui.visuals(), Tint::Warn),
            filler: ui.visuals().text_color().gamma_multiply(0.06),
            select: ui.visuals().selection.bg_fill,
            weak: ui.visuals().weak_text_color(),
        };

        // The rows.
        let mut rows_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("diff_sbs_rows")
                .max_rect(rows_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        rows_ui.set_clip_rect(rows_rect.intersect(ui.clip_rect()));
        rows_ui.spacing_mut().scroll = solid;
        // Rows touch.
        rows_ui.spacing_mut().item_spacing.y = 0.0;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("diff_sbs")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
        if let Some(item) = self.jump.take() {
            area = area.vertical_scroll_offset((item as f32 - LEAD).max(0.0) * ROW);
        }
        let current = self.current.and_then(|at| view.blocks.get(at)).copied();
        let mut touched = Touched::default();
        let mut drawn: Vec<(usize, Rect)> = Vec::new();
        #[cfg(test)]
        let mut arrows_seen: Vec<(usize, Side, Rect)> = Vec::new();
        let selected = &self.selected;
        let editor = &mut self.editor;
        // How the caret left its line, if it did; whether that line was on screen; and
        // where the caret is, if it moved, to be brought into sight.
        let mut ending: Option<Ending> = None;
        let mut editor_on_screen = false;
        let mut caret_at: Option<f32> = None;
        let scrolled = area.show_rows(&mut rows_ui, ROW, items.len(), |ui, range| {
            // The difference whose arrows are drawn already, and whether the first row
            // that is wholly on screen is still to come (a difference that began above
            // it has its arrows there, so they stay in reach while the rest of it is in
            // view).
            let mut arrows_for: Option<usize> = None;
            let mut first_on_screen = true;
            let top_of_view = ui.clip_rect().min.y;
            // The arrows are drawn when all the rows are: the second is in the row below
            // the first, and has to be over it.
            let mut to_arrow: Vec<(usize, Rect, bool)> = Vec::new();
            for item in &items[range] {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(inner.width(), ROW), Sense::click_and_drag());
                match *item {
                    Item::Fold { count, .. } => paint_fold(ui, rect, count, &paint),
                    Item::Row(index) => {
                        let index = index as usize;
                        let row = &view.rows[index];
                        drawn.push((index, rect));
                        let in_current = current.is_some_and(|b| (b.start..b.end).contains(&index));
                        let picked = row.change.is_some_and(|c| selected.contains(&c));
                        let typing = editor
                            .as_ref()
                            .filter(|editor| editor.row == index)
                            .map(|editor| editor.side);
                        paint_row(ui, rect, row, in_current, picked, typing, &paint);
                        // (egui says the second click of a double click is a click too: it
                        // has picked the line already, with the first)
                        if response.double_clicked() {
                            let at = response.interact_pointer_pos().unwrap_or(rect.center());
                            touched.double_clicked = Some((index, at));
                        } else if response.clicked() {
                            touched.clicked = Some(index);
                        }
                        if response.secondary_clicked() {
                            touched.right_clicked = Some(index);
                        }
                        if response.drag_started() {
                            touched.drag_from = Some(index);
                        }
                        // The field is over the text of its side, and the menu of the line
                        // is its too.
                        if let Some(editor) = editor.as_mut().filter(|e| e.row == index) {
                            editor_on_screen = true;
                            let (done, at, field) = edit_line(ui, rect, editor, &paint, glyph);
                            ending = done;
                            caret_at = at;
                            if field.secondary_clicked() {
                                touched.right_clicked = Some(index);
                            }
                            if row.mark != Mark::Same {
                                field.context_menu(|ui| row_menu(ui, row, &mut touched));
                            }
                        }
                        let block = block_at(&view.blocks, index);
                        if row.mark != Mark::Same {
                            let response = response.on_hover_ui(|ui| {
                                let path = row.path.as_deref().unwrap_or_default();
                                ui.label(if path.is_empty() {
                                    "(whole document)"
                                } else {
                                    path
                                });
                                ui.weak(
                                    "Click to pick the line, Ctrl or Shift to pick more; \
                                     double-click to type over it",
                                );
                                ui.weak("Right-click for a menu");
                            });
                            response.context_menu(|ui| row_menu(ui, row, &mut touched));
                        }
                        // The arrows of a difference, at its first line (or at the first
                        // line of it that is on screen).
                        if let Some(block) = block {
                            let starts = view.blocks[block].start == index;
                            let whole = rect.min.y >= top_of_view - 0.5;
                            if arrows_for != Some(block) && (starts || (first_on_screen && whole)) {
                                arrows_for = Some(block);
                                let lines_picked = selected
                                    .range(view.changes_of(&view.blocks[block]))
                                    .next()
                                    .is_some();
                                to_arrow.push((block, rect, lines_picked));
                            }
                        }
                        if rect.min.y >= top_of_view - 0.5 {
                            first_on_screen = false;
                        }
                    }
                }
            }
            for (block, rect, lines_picked) in to_arrow {
                let pressed = arrows(ui, rect, block, lines_picked);
                if let Some(side) = pressed.pressed {
                    touched.arrow = Some((block, side));
                }
                #[cfg(test)]
                arrows_seen.extend(pressed.rects.map(|(s, r)| (block, s, r)));
            }
        });
        #[cfg(test)]
        {
            self.arrows = arrows_seen;
        }
        let first_shown = (scrolled.state.offset.y / ROW).floor().max(0.0) as usize;
        self.top = first_shown.min(items.len().saturating_sub(1));
        let visible = rows_rect.height() / ROW;

        // The overview: a tick for each difference, and what is on screen.
        let overview = ui.interact(
            overview_rect,
            ui.id().with("diff_overview"),
            Sense::click_and_drag(),
        );
        paint_overview(
            ui,
            overview_rect,
            view,
            &items,
            (first_shown as f32, visible),
            current,
            &paint,
        );
        if let Some(pos) = overview.interact_pointer_pos() {
            if overview.is_pointer_button_down_on() || overview.clicked() {
                let at = (pos.y - overview_rect.min.y) / overview_rect.height().max(1.0);
                let item = at * items.len() as f32 - visible / 2.0 + LEAD;
                self.jump = Some(item.max(0.0) as usize);
                ui.ctx().request_repaint();
            }
        }

        let save = self.header(
            ui,
            (left_half, right_half),
            view,
            &docs,
            header_h,
            full.min.y,
        );

        // What was done to the rows.
        let modifiers = ui.input(|i| i.modifiers);

        // The caret leaves its line: with Enter, or by a click elsewhere, what was typed is
        // taken; with Esc it is let go of; and a line that is no longer on screen is left as
        // by a click.
        let mut typed = None;
        if self.editor.is_some() {
            match ending {
                Some(Ending::Put) => typed = self.finish_editor(),
                Some(Ending::Back) => self.editor = None,
                None if !editor_on_screen => typed = self.finish_editor(),
                None => {}
            }
        }
        // The caret is kept in sight: the text scrolls, on both sides, when it is not.
        if let (Some(at), Some(editor)) = (caret_at, &self.editor) {
            let half = match editor.side {
                Side::Left => left_half,
                Side::Right => right_half,
            };
            let (near, far) = (half.min.x + TEXT_PAD, half.max.x - 3.0 * glyph);
            if at > far {
                self.h_off += at - far;
            } else if at < near {
                self.h_off -= near - at;
            }
        }
        // What is typed is to be made first. A line that is still being typed over (it was
        // not JSON) is put right before anything else is done.
        let busy = typed.is_some() || self.editor.is_some();

        if !busy {
            if let Some(row) = touched.right_clicked {
                self.press_secondary(view, row);
            }
            if let Some(from) = touched.drag_from {
                // The line the drag began on is picked, as a click would pick it.
                self.anchor = Some(from);
                self.dragging = Some((from, from));
                self.pick_between(view, from, from, modifiers.command);
                if view.rows.get(from).is_some_and(|row| row.change.is_some()) {
                    self.current = block_at(&view.blocks, from);
                }
                self.recount(view);
            }
            if let Some((from, _)) = self.dragging {
                if ui.input(|i| i.pointer.primary_down()) {
                    let pointer = ui.input(|i| i.pointer.latest_pos());
                    if let Some(row) = pointer.and_then(|pos| row_at(&drawn, pos.y)) {
                        self.drag_over(view, from, row);
                    }
                } else {
                    self.dragging = None;
                }
            }
            if let Some(row) = touched.clicked {
                self.click(view, row, modifiers);
                self.refusal = None;
            }
            if let Some((row, at)) = touched.double_clicked {
                // The first click of the pair has picked the line. With Ctrl or Shift held
                // the second is one more pick; without, it puts a caret in the line, on the
                // side it was on.
                let plain = !modifiers.command && !modifiers.shift && !modifiers.alt;
                let side = if left_half.x_range().contains(at.x) {
                    Some((Side::Left, left_half))
                } else if right_half.x_range().contains(at.x) {
                    Some((Side::Right, right_half))
                } else {
                    None
                };
                self.refusal = None;
                match (plain, side) {
                    (true, Some((side, half))) => {
                        let from = half.min.x + TEXT_PAD - self.h_off;
                        let caret = ((at.x - from) / glyph).round().max(0.0) as usize;
                        self.open_editor(view, row, side, caret);
                    }
                    (false, _) => self.click(view, row, modifiers),
                    (true, None) => {}
                }
            }
        }

        let mut copy_path = None;
        let mut moved = None;
        match touched.menu {
            Some(Menu::Move(into)) => moved = self.to_move(view, into),
            Some(Menu::CopyPath(path)) => copy_path = Some(path),
            None => {}
        }
        if let Some((block, into)) = touched.arrow {
            // The arrow moves what is picked in its difference, or all of it.
            moved = self.to_move_in(view, block, into);
        }
        if busy {
            moved = None;
        }
        if let Some(mv) = &moved {
            self.current = Some(mv.block);
        }
        self.items = items;
        ui.allocate_rect(full, Sense::hover());
        Shown {
            copy_path,
            moved,
            save,
            typed,
        }
    }

    /// What is over each side: which document it is, what it is called, and, at
    /// the right, "Differences only" and the button that saves the document.
    fn header(
        &mut self,
        ui: &mut egui::Ui,
        (left, right): (Rect, Rect),
        view: &SideBySide,
        docs: &[Doc; 2],
        height: f32,
        y: f32,
    ) -> Option<Side> {
        let side = |ui: &mut egui::Ui, id: &str, rect: Rect| {
            let rect =
                Rect::from_min_size(Pos2::new(rect.min.x, y), egui::vec2(rect.width(), height));
            ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(id)
                    .max_rect(rect)
                    .layout(Layout::left_to_right(Align::Center)),
            )
        };
        let describe = |doc: &Doc, lines: usize| match &doc.name {
            Some(name) => format!("{name} · {lines} lines"),
            None => format!("{lines} lines"),
        };
        // The Save… of a document, at the right end of its column.
        let save_button = |ui: &mut egui::Ui, doc: &Doc, which: &str| {
            let button = egui::Button::new(if doc.changed {
                RichText::new("Save…").strong()
            } else {
                RichText::new("Save…")
            });
            ui.add_enabled(doc.can_save, button)
                .on_hover_text(if doc.changed {
                    format!(
                        "Write the {which} document to a file. It was changed here and is not \
                         saved: the file it came from is as it was"
                    )
                } else {
                    format!("Write the {which} document to a file")
                })
                .on_disabled_hover_text(format!(
                    "The {which} document is read from its file when the tool runs, and has not \
                     been changed here: there is nothing to save"
                ))
                .clicked()
        };
        let mut save = None;

        let mut first = side(ui, "diff_sbs_left", left);
        pane_header::title(&mut first, "Left");
        let text = describe(&docs[0], view.left_lines);
        pane_header::pinned_right(&mut first, |ui| {
            if save_button(ui, &docs[0], "Left") {
                save = Some(Side::Left);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(text).weak()).truncate());
            });
        });

        let mut second = side(ui, "diff_sbs_right", right);
        pane_header::title(&mut second, "Right");
        let text = describe(&docs[1], view.right_lines);
        pane_header::pinned_right(&mut second, |ui| {
            if save_button(ui, &docs[1], "Right") {
                save = Some(Side::Right);
            }
            ui.checkbox(&mut self.differences_only, "Differences only")
                .on_hover_text("Hide what is the same, but a few lines around each difference");
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(text).weak()).truncate());
            });
        });
        save
    }
}

/// What [`arrows`] did.
struct Pressed {
    /// The arrow that was pressed, as the side it points to (and so moves into).
    pressed: Option<Side>,
    /// Where the two arrows are, for the tests.
    #[cfg(test)]
    rects: [(Side, Rect); 2],
}

/// The room of the two arrows of a difference, in the gutter: the one that points to the
/// left over the one that points to the right, each as high as a row, the second in the row
/// below `row`.
fn arrow_rects(row: Rect) -> [(Side, Rect); 2] {
    let (left, right) = halves(row);
    let x = (left.max.x + right.min.x) / 2.0 - ARROW_W / 2.0;
    let size = egui::vec2(ARROW_W, ROW - 1.0);
    [
        (
            Side::Left,
            Rect::from_min_size(Pos2::new(x, row.min.y + 0.5), size),
        ),
        (
            Side::Right,
            Rect::from_min_size(Pos2::new(x, row.min.y + ROW + 0.5), size),
        ),
    ]
}

/// The two arrows of a difference, in the gutter from `row` down: the one at the top
/// points to the left document, which takes what the right one has there; the one under it
/// the other way. `picked` says that lines of the difference are picked, and the arrows
/// move those and not all of it.
fn arrows(ui: &mut egui::Ui, row: Rect, block: usize, picked: bool) -> Pressed {
    let rects = arrow_rects(row);
    let mut pressed = None;
    for (into, rect) in rects {
        let response = ui.interact(
            rect,
            ui.id().with(("diff_arrow", block, into == Side::Left)),
            Sense::click(),
        );
        let widgets = &ui.visuals().widgets;
        let style = if response.is_pointer_button_down_on() {
            widgets.active
        } else if response.hovered() {
            widgets.hovered
        } else {
            widgets.inactive
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            3.0,
            style.bg_fill,
            style.bg_stroke,
            StrokeKind::Inside,
        );
        let (c, w, h) = (rect.center(), 3.5, 4.5);
        let points = match into {
            Side::Left => vec![
                Pos2::new(c.x - w, c.y),
                Pos2::new(c.x + w, c.y - h),
                Pos2::new(c.x + w, c.y + h),
            ],
            Side::Right => vec![
                Pos2::new(c.x + w, c.y),
                Pos2::new(c.x - w, c.y - h),
                Pos2::new(c.x - w, c.y + h),
            ],
        };
        painter.add(Shape::convex_polygon(
            points,
            style.fg_stroke.color,
            Stroke::NONE,
        ));
        let (to, takes) = match into {
            Side::Left => ("left", "Left takes what Right has there"),
            Side::Right => ("right", "Right takes what Left has there"),
        };
        let what = if picked {
            "the picked lines"
        } else {
            "this difference"
        };
        if response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(format!("Move {what} to the {to}: {takes}"))
            .clicked()
        {
            pressed = Some(into);
        }
    }
    Pressed {
        pressed,
        #[cfg(test)]
        rects,
    }
}

/// How the caret left its line.
enum Ending {
    /// With Enter, or by a click elsewhere: what was typed is to be taken.
    Put,
    /// With Esc: it is let go of.
    Back,
}

/// The field a line is typed over in, in the side of the row it is on, where the text of the
/// line was. Says how the caret left it, if it did, and where it is on screen, if it moved.
fn edit_line(
    ui: &mut egui::Ui,
    row: Rect,
    editor: &mut Editor,
    p: &Paint,
    glyph: f32,
) -> (Option<Ending>, Option<f32>, egui::Response) {
    let (left, right) = halves(row);
    let half = match editor.side {
        Side::Left => left,
        Side::Right => right,
    };
    let id = egui::Id::new("diff_sbs_editor");
    let ctx = ui.ctx().clone();

    // A ground of its own with an edge, so that it is plain where the typing goes (red
    // when what was typed was not taken).
    let visuals = ui.visuals();
    let edge = if editor.problem.is_some() {
        ERROR
    } else {
        visuals.selection.stroke.color
    };
    let ground = visuals.extreme_bg_color;
    ui.painter().rect_filled(half, 0.0, ground);
    ui.painter()
        .rect_stroke(half, 0.0, Stroke::new(1.0, edge), StrokeKind::Inside);

    if let Some(caret) = editor.wants_focus.take() {
        ctx.memory_mut(|memory| memory.request_focus(id));
        if let Some(caret) = caret {
            let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(caret),
                )));
            state.store(&ctx, id);
        }
    }

    let x = half.min.x + TEXT_PAD - p.h_off;
    let width = ((editor.text.chars().count() + 2) as f32 * glyph).max(half.width());
    let field = Rect::from_min_size(Pos2::new(x, row.min.y), egui::vec2(width, ROW));
    let mut cell = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(field)
            .layout(Layout::left_to_right(Align::Center)),
    );
    cell.set_clip_rect(half.intersect(ui.clip_rect()));
    let out = egui::TextEdit::singleline(&mut editor.text)
        .id(id)
        .font(p.font.clone())
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::ZERO)
        .text_color(p.text)
        .desired_width(width)
        .show(&mut cell);
    let response = &out.response;
    if response.changed() && editor.problem.take().is_some() {
        // (the status bar, which says what was wrong, was drawn before the field was)
        ctx.request_repaint();
    }
    if response.has_focus() {
        editor.had_focus = true;
        editor.tries = 0;
    }

    let ending = if response.lost_focus() {
        Some(if ui.input(|i| i.key_pressed(Key::Escape)) {
            Ending::Back
        } else {
            Ending::Put
        })
    } else if !editor.had_focus && !response.has_focus() {
        // It asked for the focus and has not got it: not for ever.
        editor.tries += 1;
        (editor.tries > 4).then_some(Ending::Back)
    } else {
        None
    };

    let mut caret_at = None;
    if let Some(range) = out.cursor_range {
        let at = range.primary.index.0;
        if editor.seen_at != Some(at) {
            editor.seen_at = Some(at);
            caret_at = Some(x + at as f32 * glyph);
        }
    }
    (ending, caret_at, out.response.response)
}

/// The menu of a line that is not the same on both sides: move it, or copy its path.
fn row_menu(ui: &mut egui::Ui, row: &Row, touched: &mut Touched) {
    for (label, tip, into) in [
        (
            "Move to the left",
            "Left takes what Right has here",
            Side::Left,
        ),
        (
            "Move to the right",
            "Right takes what Left has here",
            Side::Right,
        ),
    ] {
        if ui.button(label).on_hover_text(tip).clicked() {
            touched.menu = Some(Menu::Move(into));
            ui.close();
        }
    }
    ui.separator();
    if ui
        .button("Copy path")
        .on_hover_text("Put the JSON Pointer of this line on the clipboard")
        .clicked()
    {
        let path = row.path.as_deref().unwrap_or_default();
        touched.menu = Some(Menu::CopyPath(path.to_owned()));
        ui.close();
    }
}

/// The colours a row is drawn in.
struct Paint {
    font: FontId,
    h_off: f32,
    text: Color32,
    good: Color32,
    bad: Color32,
    warn: Color32,
    /// What the blank side of a line that only the other side has is filled
    /// with.
    filler: Color32,
    /// The gutter beside the difference Previous and Next are on.
    select: Color32,
    weak: Color32,
}

/// Draw a row. `typing` is the side that has a caret in its line: the field draws that one's
/// text.
fn paint_row(
    ui: &egui::Ui,
    rect: Rect,
    row: &Row,
    in_current: bool,
    picked: bool,
    typing: Option<Side>,
    p: &Paint,
) {
    let (left, right) = halves(rect);
    let painter = ui.painter();

    let colour = match row.mark {
        Mark::Same => None,
        Mark::Changed => Some(p.warn),
        Mark::Removed => Some(p.bad),
        Mark::Added => Some(p.good),
    };
    let tinted = |side: &Option<String>| {
        side.as_ref()
            .map(|_| colour.map(|c| c.gamma_multiply(0.16)))
            .unwrap_or(Some(p.filler))
    };
    for (half, fill) in [(left, tinted(&row.left)), (right, tinted(&row.right))] {
        if let Some(fill) = fill {
            painter.rect_filled(half, 0.0, fill);
        }
        // A line that is picked has the selection's colour over it, on both sides.
        if picked {
            painter.rect_filled(half, 0.0, p.select.gamma_multiply(0.55));
        }
    }

    // Between the sides: a bar in the colour of what differs, and where the
    // difference that is picked is, all of it.
    let gutter = Rect::from_min_max(
        Pos2::new(left.max.x, rect.min.y),
        Pos2::new(right.min.x, rect.max.y),
    );
    if in_current {
        // (narrower than the gutter: the arrows are in it)
        let inset = ((gutter.width() - 8.0) / 2.0).max(0.0);
        painter.rect_filled(gutter.shrink2(egui::vec2(inset, 0.0)), 0.0, p.select);
    } else if let Some(colour) = colour {
        let inset = ((gutter.width() - 4.0) / 2.0).max(0.0);
        painter.rect_filled(gutter.shrink2(egui::vec2(inset, 0.0)), 0.0, colour);
    }

    // What differs inside a line that changed (not in one that is being typed over: it is
    // not the text that was compared any more).
    if let (Mark::Changed, Some(a), Some(b), None) = (row.mark, &row.left, &row.right, typing) {
        let (range_a, range_b) = differing(a, b);
        for (half, text, range) in [(left, a, range_a), (right, b, range_b)] {
            let (from, to) = ui.fonts_mut(|fonts| {
                let mut x = 0.0;
                let (mut from, mut to) = (None, None);
                for (at, c) in text.chars().enumerate() {
                    if at == range.start {
                        from = Some(x);
                    }
                    if at == range.end {
                        to = Some(x);
                    }
                    x += fonts.glyph_width(&p.font, c);
                }
                (from.unwrap_or(x), to.unwrap_or(x))
            });
            let x = half.min.x + TEXT_PAD - p.h_off;
            let shaded = Rect::from_min_max(
                Pos2::new(x + from, rect.min.y + 1.0),
                Pos2::new(x + to.max(from + 2.0), rect.max.y - 1.0),
            )
            .intersect(half);
            if shaded.is_positive() {
                painter.rect_filled(shaded, 2.0, p.warn.gamma_multiply(0.45));
            }
        }
    }

    for (half, side, text) in [
        (left, Side::Left, &row.left),
        (right, Side::Right, &row.right),
    ] {
        if let Some(text) = text.as_ref().filter(|_| typing != Some(side)) {
            painter
                .with_clip_rect(half.intersect(painter.clip_rect()))
                .text(
                    Pos2::new(half.min.x + TEXT_PAD - p.h_off, half.center().y),
                    Align2::LEFT_CENTER,
                    text,
                    p.font.clone(),
                    p.text,
                );
        }
    }
}

/// A line that stands for rows that are the same on both sides.
fn paint_fold(ui: &egui::Ui, rect: Rect, count: u32, p: &Paint) {
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, p.filler);
    let text = if count == 1 {
        "… 1 line is the same".to_owned()
    } else {
        format!("… {count} lines are the same")
    };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        p.font.clone(),
        p.weak,
    );
}

/// The strip beside the rows: the whole view in a few points of height, a tick
/// where each difference is, the part on screen outlined, and the difference
/// that is picked in the selection colour.
fn paint_overview(
    ui: &egui::Ui,
    rect: Rect,
    view: &SideBySide,
    items: &[Item],
    (first, visible): (f32, f32),
    current: Option<Block>,
    p: &Paint,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, p.filler);
    let total = items.len().max(1) as f32;
    let y_of = |item: f32| rect.min.y + item / total * rect.height();
    for block in &view.blocks {
        let from = position_of(items, block.start) as f32;
        let to = position_of(items, block.end.saturating_sub(1)) as f32 + 1.0;
        let colour = match block.mark {
            Mark::Added => p.good,
            Mark::Removed => p.bad,
            _ => p.warn,
        };
        let tick = Rect::from_min_max(
            Pos2::new(rect.min.x + 1.0, y_of(from)),
            Pos2::new(rect.max.x - 1.0, y_of(to).max(y_of(from) + 2.0)),
        );
        let colour = if current == Some(*block) {
            p.select
        } else {
            colour
        };
        painter.rect_filled(tick, 0.0, colour);
    }
    let window = Rect::from_min_max(
        Pos2::new(rect.min.x, y_of(first)),
        Pos2::new(rect.max.x, y_of(first + visible).min(rect.max.y)),
    );
    painter.rect_stroke(
        window,
        2.0,
        egui::Stroke::new(1.0, p.weak),
        egui::StrokeKind::Inside,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonquery_query::diff::compare;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    fn view_of(before: serde_json::Value, after: serde_json::Value) -> SideBySide {
        compare(&before, &after, &AtomicBool::new(false))
            .unwrap()
            .view
            .unwrap()
    }

    /// Twenty numbers with the third and the eighteenth changed.
    fn spread() -> SideBySide {
        let before: Vec<u32> = (0..20).collect();
        let mut after = before.clone();
        after[2] = 100;
        after[17] = 200;
        view_of(json!(before), json!(after))
    }

    #[test]
    fn the_gutter_holds_one_arrow_across_and_two_down() {
        let row = Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, ROW));
        let (left, right) = halves(row);
        let gutter = right.min.x - left.max.x;
        assert!(
            (ARROW_W + 4.0..2.0 * ARROW_W).contains(&gutter),
            "room for one arrow, not two: {gutter}"
        );
        // The two are one over the other, in the middle of it, a row apart.
        let [(to_left, up), (to_right, down)] = arrow_rects(row);
        assert_eq!((to_left, to_right), (Side::Left, Side::Right));
        assert_eq!(up.center().x, down.center().x);
        assert!((up.center().x - (left.max.x + gutter / 2.0)).abs() < 0.5);
        assert!((down.center().y - up.center().y - ROW).abs() < 0.01);
        assert!(up.max.y <= down.min.y, "they do not overlap");
        assert!(up.left() > left.max.x && up.right() < right.min.x);
    }

    #[test]
    fn everything_is_shown_unless_asked_to_fold() {
        let view = spread();
        let all = items(&view, false);
        assert_eq!(all.len(), view.rows.len());
        assert!(all.iter().all(|i| matches!(i, Item::Row(_))));
    }

    #[test]
    fn folding_keeps_a_few_rows_around_each_difference() {
        let view = spread();
        // Rows: [ at 0, number n at n + 1, ] at 21. Changed rows are 3 and 18.
        assert_eq!(
            view.blocks.iter().map(|b| b.start).collect::<Vec<_>>(),
            [3, 18]
        );
        let folded = items(&view, true);
        // Rows 0 to 6 (the first difference and three after it), then the
        // eight rows that are the same, then rows 15 to 21 (three before the
        // second).
        let mut expected: Vec<Item> = (0..7).map(Item::Row).collect();
        expected.push(Item::Fold { first: 7, count: 8 });
        expected.extend((15..22).map(Item::Row));
        assert_eq!(folded, expected);
        let hidden: u32 = folded
            .iter()
            .map(|i| {
                if let Item::Fold { count, .. } = i {
                    *count
                } else {
                    0
                }
            })
            .sum();
        let shown = folded.iter().filter(|i| matches!(i, Item::Row(_))).count();
        assert_eq!(shown + hidden as usize, view.rows.len());
    }

    #[test]
    fn a_view_with_no_difference_is_never_folded() {
        let view = view_of(json!([1, 2, 3]), json!([1, 2, 3]));
        assert_eq!(items(&view, true).len(), view.rows.len());
    }

    #[test]
    fn a_row_is_found_in_the_list_even_when_folded_away() {
        let view = spread();
        let folded = items(&view, true);
        assert_eq!(position_of(&folded, 3), 3);
        // Row 10 is inside the fold that begins at row 7.
        assert_eq!(
            folded[position_of(&folded, 10)],
            Item::Fold { first: 7, count: 8 }
        );
        assert_eq!(position_of(&items(&view, false), 10), 10);
    }

    #[test]
    fn a_row_belongs_to_the_difference_it_is_in() {
        let view = spread();
        assert_eq!(block_at(&view.blocks, 3), Some(0));
        assert_eq!(block_at(&view.blocks, 18), Some(1));
        assert_eq!(block_at(&view.blocks, 4), None);
        assert_eq!(block_at(&view.blocks, 100), None);
    }

    #[test]
    fn the_part_that_differs_is_what_is_left_after_the_shared_ends() {
        assert_eq!(differing("  \"n\": \"b\"", "  \"n\": \"B\""), (8..9, 8..9));
        assert_eq!(differing("1,", "20,"), (0..1, 0..2));
        assert_eq!(differing("same", "same"), (4..4, 4..4));
        // Only one side has more: the other has nothing in between.
        assert_eq!(differing("ab", "abc"), (2..2, 2..3));
        assert_eq!(differing("", "x"), (0..0, 0..1));
        // Characters, not bytes.
        assert_eq!(differing("é1", "é2"), (1..2, 1..2));
    }

    #[test]
    fn previous_and_next_walk_the_differences() {
        let view = spread();
        let mut viewer = Viewer::default();
        viewer.ensure_items(&view);

        // Nothing picked, at the top: Next is the first, and nothing is before.
        assert!(viewer.can_step(&view, true));
        assert!(!viewer.can_step(&view, false));
        viewer.step(&view, true);
        assert_eq!(viewer.current, Some(0));
        assert_eq!(viewer.jump, Some(3));
        assert_eq!(viewer.position(&view).as_deref(), Some("Difference 1 of 2"));

        viewer.step(&view, true);
        assert_eq!(viewer.current, Some(1));
        assert!(!viewer.can_step(&view, true));
        viewer.step(&view, true);
        assert_eq!(viewer.current, Some(1), "there is no third");

        viewer.step(&view, false);
        assert_eq!(viewer.current, Some(0));
        assert!(!viewer.can_step(&view, false));
    }

    #[test]
    fn with_none_picked_the_screen_decides_where_next_and_previous_go() {
        let view = spread();
        let mut viewer = Viewer::default();
        viewer.ensure_items(&view);
        // Scrolled to row 10, between the two.
        viewer.top = 10;
        assert_eq!(viewer.target(&view, true), Some(1));
        assert_eq!(viewer.target(&view, false), Some(0));
        // Scrolled past both.
        viewer.top = 21;
        assert_eq!(viewer.target(&view, true), None);
        assert_eq!(viewer.target(&view, false), Some(1));
    }

    #[test]
    fn folding_changes_where_a_difference_is_on_screen() {
        let view = spread();
        let mut viewer = Viewer {
            differences_only: true,
            ..Viewer::default()
        };
        viewer.step(&view, true);
        viewer.step(&view, true);
        // Row 18 is item 11 now: the fold above it took seven rows out.
        assert_eq!(viewer.jump, Some(position_of(&viewer.items, 18)));
        assert_eq!(viewer.jump, Some(11));
    }

    #[test]
    fn switching_the_fold_on_or_off_keeps_what_was_at_the_top() {
        let view = spread();
        let mut viewer = Viewer::default();
        viewer.ensure_items(&view);
        assert_eq!(viewer.jump, None, "the first time there is nothing to keep");

        // Row 16 is at the top of the full list...
        viewer.top = 16;
        viewer.differences_only = true;
        viewer.ensure_items(&view);
        // ...and is item 9 of the folded one (the fold stands for rows 7 to 14).
        // A jump goes LEAD items above where it is sent, so it is sent that far
        // below.
        assert_eq!(viewer.jump, Some(9 + LEAD as usize));
        assert_eq!(viewer.items[9], Item::Row(16));

        // Back to the full list, with the fold at the top: its first row.
        viewer.top = 7;
        viewer.jump = None;
        viewer.differences_only = false;
        viewer.ensure_items(&view);
        assert_eq!(viewer.jump, Some(7 + LEAD as usize));
    }

    #[test]
    fn a_new_comparison_forgets_the_position_but_keeps_the_option() {
        let view = spread();
        let mut viewer = Viewer {
            differences_only: true,
            h_off: 40.0,
            ..Viewer::default()
        };
        viewer.step(&view, true);
        viewer.reset();
        assert!(viewer.differences_only);
        assert_eq!(
            (viewer.current, viewer.jump, viewer.h_off),
            (None, None, 0.0)
        );
        assert!(viewer.items.is_empty());
    }
}
