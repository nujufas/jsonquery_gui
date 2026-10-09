//! Headless layout tests: the whole app runs frame by frame in a plain
//! `egui::Context` (no window, no display), so panel sizes can be measured
//! exactly and the properties the GUI can only show by eye — "a long query
//! doesn't take over the window", "a dragged height sticks" — are checked
//! every time `cargo test` runs.

use super::*;
use crate::scratch_dir::{ScratchDir, ScratchFile};
use egui::containers::panel::PanelState;
use std::collections::HashMap;

const SCREEN: egui::Vec2 = egui::vec2(1200.0, 800.0);

struct Harness {
    ctx: egui::Context,
    app: App,
    time: f64,
    /// How big the window is, which a test may change.
    screen: egui::Vec2,
    /// What the window says of itself (maximized, the monitor it is on…), which a
    /// test may change.
    window: egui::ViewportInfo,
    shapes: Vec<egui::epaint::ClippedShape>,
    /// Every viewport command any frame has sent.
    commands: Vec<egui::ViewportCommand>,
    /// Everything any frame has put on the clipboard, oldest first.
    copied: Vec<String>,
}

impl Harness {
    fn new() -> Self {
        Self::with_app(App::with_context, SCREEN, egui::ViewportInfo::default())
    }

    /// The app as it starts with its settings kept in `store`.
    fn with_store(store: Store) -> Self {
        Self::with_store_in(store, SCREEN, egui::ViewportInfo::default())
    }

    /// The same, in a window of this size that says this of itself from the first
    /// frame (as one opened at a size does).
    fn with_store_in(store: Store, screen: egui::Vec2, window: egui::ViewportInfo) -> Self {
        Self::with_app(|ctx| App::with_store(ctx, store), screen, window)
    }

    fn with_app(
        make: impl FnOnce(&egui::Context) -> App,
        screen: egui::Vec2,
        window: egui::ViewportInfo,
    ) -> Self {
        let ctx = egui::Context::default();
        let app = make(&ctx);
        let mut h = Self {
            ctx,
            app,
            time: 0.0,
            screen,
            window,
            shapes: Vec::new(),
            commands: Vec::new(),
            copied: Vec::new(),
        };
        // A couple of frames so every panel has measured itself.
        h.settle();
        h
    }

    /// One frame in which `f` runs instead of the whole app.
    fn run_with(&mut self, events: Vec<egui::Event>, mut f: impl FnMut(&mut App, &mut egui::Ui)) {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.screen)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        input
            .viewports
            .insert(egui::ViewportId::ROOT, self.window.clone());
        self.time += 1.0 / 60.0;
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| f(app, ui));
        // Nothing paints these, and egui panics if a texture update is dropped.
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

    fn frame_with(&mut self, events: Vec<egui::Event>) {
        self.run_with(events, |app, ui| app.show(ui));
    }

    fn frame(&mut self) {
        self.frame_with(Vec::new());
    }

    fn settle(&mut self) {
        for _ in 0..6 {
            self.frame();
        }
    }

    fn panel(&self, id: &'static str) -> Option<egui::Rect> {
        PanelState::load(&self.ctx, egui::Id::new(id)).map(|s| s.outer_rect)
    }

    fn query_panel(&self) -> egui::Rect {
        self.panel("query_bar").expect("query panel drawn")
    }

    /// Every piece of text drawn last frame, with where it starts.
    fn texts(&self) -> Vec<(String, egui::Pos2)> {
        text_rects(&self.shapes)
            .into_iter()
            .map(|(text, rect)| (text, rect.min))
            .collect()
    }

    /// Where `text` is drawn (the first, in paint order), if it is.
    fn text_pos(&self, text: &str) -> Option<egui::Pos2> {
        self.texts()
            .into_iter()
            .find(|(t, _)| t == text)
            .map(|(_, p)| p)
    }

    /// The centres of the pop-out/pop-in buttons, from the first (top-most) down.
    /// (The glyph is drawn centred in the button's square, so its centre is the
    /// button's.)
    fn pop_buttons(&self, glyph: &str) -> Vec<egui::Pos2> {
        let mut found: Vec<_> = text_rects(&self.shapes)
            .into_iter()
            .filter(|(t, _)| t == glyph)
            .map(|(_, rect)| rect.center())
            .collect();
        found.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap());
        found
    }

    fn pointer(&mut self, x: f32, y: f32) {
        self.frame_with(vec![egui::Event::PointerMoved(egui::pos2(x, y))]);
    }

    fn button(&mut self, x: f32, y: f32, pressed: bool) {
        self.frame_with(vec![egui::Event::PointerButton {
            pos: egui::pos2(x, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]);
    }

    fn click(&mut self, x: f32, y: f32) {
        self.pointer(x, y);
        self.button(x, y, true);
        self.button(x, y, false);
        self.settle();
    }

    /// A click with `modifiers` held down (Ctrl, Shift…).
    fn click_with(&mut self, x: f32, y: f32, modifiers: egui::Modifiers) {
        self.frame_with(vec![egui::Event::ModifiersChanged(modifiers)]);
        self.pointer(x, y);
        for pressed in [true, false] {
            self.frame_with(vec![egui::Event::PointerButton {
                pos: egui::pos2(x, y),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            }]);
        }
        self.frame_with(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        self.settle();
    }

    /// Let `seconds` go by, a frame at a time.
    fn pause(&mut self, seconds: f64) {
        for _ in 0..(seconds * 60.0) as usize {
            self.frame();
        }
    }

    /// Two clicks in a row, close enough in time and place to be a double click
    /// (and not, with an earlier click close behind, a triple one).
    /// Press and release the secondary button, as for a context menu.
    fn right_click(&mut self, x: f32, y: f32) {
        self.pointer(x, y);
        for pressed in [true, false] {
            self.frame_with(vec![egui::Event::PointerButton {
                pos: egui::pos2(x, y),
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        self.settle();
    }

    fn double_click(&mut self, x: f32, y: f32) {
        self.pause(1.0);
        self.pointer(x, y);
        for _ in 0..2 {
            self.button(x, y, true);
            self.button(x, y, false);
        }
        self.settle();
    }

    /// A double click with `modifiers` held down (Ctrl, Shift…).
    fn double_click_with(&mut self, x: f32, y: f32, modifiers: egui::Modifiers) {
        self.pause(1.0);
        self.frame_with(vec![egui::Event::ModifiersChanged(modifiers)]);
        self.pointer(x, y);
        for _ in 0..2 {
            for pressed in [true, false] {
                self.frame_with(vec![egui::Event::PointerButton {
                    pos: egui::pos2(x, y),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                }]);
            }
        }
        self.frame_with(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        self.settle();
    }

    /// Press at `(x, from)`, drag to `(x, to)` in small steps, release.
    fn drag_y(&mut self, x: f32, from: f32, to: f32) {
        self.pointer(x, from);
        self.button(x, from, true);
        let steps = 12;
        for i in 1..=steps {
            let y = from + (to - from) * i as f32 / steps as f32;
            self.pointer(x, y);
        }
        self.button(x, to, false);
        self.settle();
    }

    /// Press at `(from, y)`, drag to `(to, y)` in small steps, release.
    fn drag_x(&mut self, from: f32, y: f32, to: f32) {
        self.pointer(from, y);
        self.button(from, y, true);
        let steps = 12;
        for i in 1..=steps {
            let x = from + (to - from) * i as f32 / steps as f32;
            self.pointer(x, y);
        }
        self.button(to, y, false);
        self.settle();
    }

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            self.frame_with(vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            }]);
        }
    }

    fn type_text(&mut self, text: &str) {
        self.frame_with(vec![egui::Event::Text(text.to_owned())]);
    }

    /// How far the query box is scrolled: the top of its fixed frame (the
    /// visible part) against the top of the editor, which moves with the text.
    fn query_scrolled_by(&self) -> f32 {
        fn frames(shape: &egui::Shape, out: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| frames(s, out)),
                // The text-box colour, or that scaled down while a window fades in.
                egui::Shape::Rect(r)
                    if r.fill.a() > 0
                        && (3..=10).contains(&r.fill.r())
                        && r.fill.r() == r.fill.g()
                        && r.fill.g() == r.fill.b() =>
                {
                    out.push(r.rect)
                }
                _ => {}
            }
        }
        let editor = self.query_box();
        let mut all = Vec::new();
        for clipped in &self.shapes {
            frames(&clipped.shape, &mut all);
        }
        let frame = all
            .into_iter()
            .find(|r| (r.min.x - editor.min.x).abs() < 0.6 && (r.max.x - editor.max.x).abs() < 0.6)
            .expect("the query box's frame was painted");
        frame.top() - editor.top()
    }

    fn query_box(&self) -> egui::Rect {
        self.ctx
            .read_response(egui::Id::new("query_text_edit_box"))
            .expect("query box drawn")
            .rect
    }
}

/// Every piece of text in `shapes`, with the rectangle it fills.
fn text_rects(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::Shape::Text(t) => out.push((
                t.galley.text().to_owned(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

fn long_query(lines: usize) -> String {
    (0..lines)
        .map(|i| format!(".members[] | select(.age > {i}) | .name"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The Robot GUI suite clicks and OCRs fixed pixels below the query panel, all
/// calibrated against the panel ending here (see `QUERY_PANEL_DEFAULT_HEIGHT`).
const LEGACY_QUERY_PANEL_BOTTOM: f32 = 125.6;
const LEGACY_QUERY_BOX_HEIGHT: f32 = 64.5;

#[test]
fn the_default_layout_is_unchanged() {
    let h = Harness::new();
    assert!(
        (h.query_panel().bottom() - LEGACY_QUERY_PANEL_BOTTOM).abs() < 0.2,
        "query panel ends at {}",
        h.query_panel().bottom()
    );
    // Four rows of text fill the box exactly.
    assert!(
        (h.query_box().height() - LEGACY_QUERY_BOX_HEIGHT).abs() < 0.2,
        "query box is {} tall",
        h.query_box().height()
    );
}

#[test]
fn a_long_query_does_not_grow_the_panel() {
    for lines in [5, 40, 400] {
        let mut h = Harness::new();
        let before = h.query_panel();
        h.app.query_text = long_query(lines);
        h.settle();
        assert_eq!(h.query_panel(), before, "{lines} lines resized the panel");
        // …and it stays put, rather than creeping over later frames.
        for _ in 0..30 {
            h.frame();
        }
        assert_eq!(h.query_panel(), before, "{lines} lines: panel crept");
    }
}

#[test]
fn typing_a_long_query_scrolls_instead_of_growing_the_panel() {
    let mut h = Harness::new();
    let before = h.query_panel();
    h.click(300.0, 80.0);
    for i in 0..30 {
        h.type_text(&format!(".members[{i}] | select(.age > {i})"));
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(h.query_panel(), before, "line {i} resized the panel");
    }
    h.settle();
    assert_eq!(h.query_panel(), before);
    assert!(h.app.query_text.lines().count() >= 30, "the text was typed");
    assert_follows_cursor_to_the_end(&h);
}

/// The editor is much taller than the visible box (it holds all the text), and
/// is scrolled so that its last line — where the cursor is — is in view: its
/// bottom edge sits at the box's bottom edge.
fn assert_follows_cursor_to_the_end(h: &Harness) {
    let panel = h.query_panel();
    let editor = h.query_box();
    assert!(
        editor.height() > 3.0 * LEGACY_QUERY_BOX_HEIGHT,
        "the editor holds all the text ({} tall)",
        editor.height()
    );
    let box_bottom = panel.bottom() - 10.0;
    assert!(
        (editor.bottom() - box_bottom).abs() < 2.0,
        "editor ends at {}, the box at {box_bottom}: the cursor is out of view",
        editor.bottom()
    );
}

#[test]
fn pasting_a_long_query_scrolls_to_the_cursor() {
    let mut h = Harness::new();
    let before = h.query_panel();
    h.click(300.0, 80.0);
    let text = (0..40)
        .map(|i| format!(".members[] | select(.age > {i})"))
        .collect::<Vec<_>>()
        .join("\n");
    h.frame_with(vec![egui::Event::Paste(text)]);
    // The scroll is animated; give it time to arrive.
    for _ in 0..60 {
        h.frame();
    }
    assert_eq!(h.query_panel(), before);
    assert_follows_cursor_to_the_end(&h);
}

#[test]
fn a_dragged_height_sticks_for_a_long_query() {
    let mut h = Harness::new();
    h.app.query_text = long_query(60);
    h.settle();
    let top = h.query_panel().top();

    // Taller…
    let edge = h.query_panel().bottom();
    h.drag_y(600.0, edge - 1.0, 330.0);
    let tall = h.query_panel();
    assert!(
        (tall.bottom() - 330.0).abs() < 2.0,
        "dragged to 330, got {}",
        tall.bottom()
    );
    for _ in 0..20 {
        h.frame();
    }
    assert_eq!(h.query_panel(), tall, "the dragged height was reapplied");

    // …and shorter again — the case that used to snap back to the full height
    // of the query.
    h.drag_y(600.0, tall.bottom() - 1.0, 100.0);
    let short = h.query_panel();
    assert!(
        (short.bottom() - 100.0).abs() < 2.0,
        "dragged to 100, got {}",
        short.bottom()
    );
    for _ in 0..20 {
        h.frame();
    }
    assert_eq!(
        h.query_panel(),
        short,
        "shrinking snapped back to the query's height"
    );
    assert_eq!(h.query_panel().top(), top);

    // There is a floor.
    h.drag_y(600.0, short.bottom() - 1.0, 30.0);
    let floor = h.query_panel();
    assert!(
        (floor.height() - QUERY_PANEL_MIN_HEIGHT).abs() < 0.5,
        "floor is {}",
        floor.height()
    );
}

#[test]
fn the_box_fills_the_panel_at_any_height() {
    let mut h = Harness::new();
    for to in [140.0, 177.0, 233.5, 301.0] {
        let edge = h.query_panel().bottom();
        h.drag_y(600.0, edge - 1.0, to);
        let panel = h.query_panel();
        let gap = panel.bottom() - h.query_box().bottom();
        // The same ~10px of margin below the box that it has at the default
        // height; never a gap of up to a row.
        assert!(
            (gap - 10.0).abs() < 1.5,
            "panel ends {to}: box ends {gap} above the panel's edge"
        );
    }
}

// ---- popping panes out -----------------------------------------------------
//
// With no real windows to open, egui shows an immediate viewport as an
// embedded floating window inside the frame — the same code path a native
// window runs, minus the OS. What these check is what the main window does.

/// Where everything is drawn — to tell a layout from another, or the same.
fn layout_of(h: &Harness) -> Vec<(String, egui::Pos2)> {
    let mut texts = h.texts();
    // Timings change from run to run.
    texts.retain(|(t, _)| !t.starts_with("Parsed in") && !t.starts_with("Query ran in"));
    texts
}

#[test]
fn popping_the_query_out_gives_its_room_to_the_panes_below() {
    let mut h = Harness::new();
    let docked = layout_of(&h);
    let results_y = h.text_pos("Results").unwrap().y;

    let query_button = h.pop_buttons("⬈")[0];
    h.click(query_button.x, query_button.y);
    assert!(
        h.app.dock.is_popped(Pane::Query),
        "the button pops the query out"
    );

    let results_now = h.text_pos("Results").unwrap().y;
    assert!(
        results_y - results_now > 90.0,
        "Results moved up from {results_y} to {results_now}"
    );
    // The query pane is still drawn — in its window.
    assert!(h
        .ctx
        .read_response(egui::Id::new("query_text_edit_box"))
        .is_some());
    assert!(h.text_pos("Query:").is_some());

    // Its window has the button that docks it back.
    let back = h.pop_buttons("⬋");
    assert_eq!(back.len(), 1, "one pane is out, so one way back");
    h.click(back[0].x, back[0].y);
    assert!(!h.app.dock.is_popped(Pane::Query));
    assert_eq!(
        layout_of(&h),
        docked,
        "docking back restores the layout exactly"
    );
}

#[test]
fn popping_source_out_gives_results_the_whole_width() {
    let mut h = Harness::new();
    let docked = layout_of(&h);
    assert!(
        h.text_pos("Results").unwrap().x > 500.0,
        "Results starts on the right"
    );

    h.app.dock.pop_out(Pane::Source, None, None);
    h.settle();
    assert_eq!(h.app.dock.central(), Central::ResultsOnly);
    assert!(
        h.text_pos("Results").unwrap().x < 20.0,
        "Results now starts at the left edge, at {:?}",
        h.text_pos("Results")
    );

    h.app.dock.dock(Pane::Source);
    h.settle();
    assert_eq!(layout_of(&h), docked);
}

#[test]
fn popping_results_out_gives_source_the_whole_width() {
    let mut h = Harness::new();
    let docked = layout_of(&h);
    h.app.dock.pop_out(Pane::Results, None, None);
    h.settle();
    assert_eq!(h.app.dock.central(), Central::SourceOnly);
    // Source's paste box spans the window now, not half of it: its header's
    // pop-out button is at the far right.
    let rightmost = h
        .pop_buttons("⬈")
        .into_iter()
        .map(|p| p.x)
        .fold(0.0, f32::max);
    assert!(rightmost > 1100.0, "Source's button is at {rightmost}");
    h.app.dock.dock(Pane::Results);
    h.settle();
    assert_eq!(layout_of(&h), docked);
}

#[test]
fn with_source_and_results_both_out_the_main_window_says_where_they_went() {
    let mut h = Harness::new();
    let docked = layout_of(&h);
    h.app.dock.pop_out(Pane::Source, None, None);
    h.app.dock.pop_out(Pane::Results, None, None);
    h.settle();
    assert_eq!(h.app.dock.central(), Central::Empty);
    assert!(h
        .text_pos("Source and Results are in their own windows")
        .is_some());
    assert!(h.text_pos("Bring them back into this window").is_some());
    // The toolbar offers to bring everything back, too.
    assert!(h.text_pos("🗖").is_some(), "dock-all button in the toolbar");

    h.app.dock.dock_all();
    h.settle();
    assert_eq!(layout_of(&h), docked);
}

#[test]
fn the_note_in_the_empty_main_window_has_a_button_that_docks_everything() {
    // On its own: with no real windows the popped panes are floating windows
    // laid over the note, which would take the click.
    let mut h = Harness::new();
    h.app.dock.pop_out(Pane::Source, None, None);
    h.app.dock.pop_out(Pane::Results, None, None);
    let note = |app: &mut App, ui: &mut egui::Ui| app.nothing_docked_note(ui);
    for _ in 0..4 {
        h.run_with(vec![], note);
    }
    let label = h.text_pos("Bring them back into this window").unwrap();
    let at = egui::pos2(label.x + 20.0, label.y + 7.0);
    h.run_with(vec![egui::Event::PointerMoved(at)], note);
    for pressed in [true, false] {
        let click = egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.run_with(vec![click], note);
    }
    h.app.dock.apply_requests();
    assert!(!h.app.dock.any_popped());
}

#[test]
fn the_toolbar_button_docks_everything_and_only_shows_while_something_is_out() {
    let mut h = Harness::new();
    assert!(h.text_pos("🗖").is_none(), "nothing is out, so no button");
    for pane in Pane::ALL {
        h.app.dock.pop_out(pane, None, None);
    }
    h.settle();
    let button = h.text_pos("🗖").expect("something is out");
    h.click(button.x + 6.0, button.y + 7.0);
    assert!(!h.app.dock.any_popped());
    assert!(h.text_pos("🗖").is_none());
}

#[test]
fn a_popped_pane_keeps_what_the_user_had_in_it() {
    let mut h = Harness::new();
    h.app.query_text = long_query(40);
    h.click(300.0, 80.0);
    // To the end of the text. Not Ctrl+End: on macOS egui's text box reads
    // Ctrl and a letter as an Emacs key and ignores the rest, End included.
    h.key(egui::Key::ArrowDown, egui::Modifiers::COMMAND);
    for _ in 0..30 {
        h.frame();
    }
    let text = h.app.query_text.clone();
    let panel = h.query_panel();
    let docked = h.query_scrolled_by();
    assert!(docked > 100.0, "scrolled to the end: {docked}");

    // The pane's ids come from the pane, not from the panel or window around
    // it — so the window's box picks up the scroll position the docked one
    // had, instead of starting over at the top of the query.
    h.app.dock.pop_out(Pane::Query, None, None);
    h.settle();
    let popped = h.query_scrolled_by();
    assert!(
        popped > 100.0,
        "the window's box started over at the top: {popped}"
    );

    h.app.dock.dock(Pane::Query);
    h.settle();
    assert_eq!(h.app.query_text, text, "the query text is untouched");
    assert_eq!(h.query_panel(), panel, "the panel is as it was");
    assert!(
        h.query_scrolled_by() > 100.0,
        "docking it back started over"
    );
}

fn ctrl(key: egui::Key) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    }]
}

#[test]
fn shortcuts_in_a_pane_window_act_on_that_pane() {
    let mut h = Harness::new();
    h.app.focused_panel = PanelKind::Source;
    // Ctrl+F pressed in the Results window searches Results, whatever was
    // clicked last in the main window.
    h.run_with(ctrl(egui::Key::F), |app, ui| {
        app.handle_shortcuts(ui.ctx(), Some(PanelKind::Results))
    });
    assert!(h.app.show_search_dialog);
    assert_eq!(h.app.search_target, PanelKind::Results);
    assert_eq!(h.app.focused_panel, PanelKind::Results);
}

#[test]
fn a_pane_window_that_no_key_was_pressed_in_leaves_the_last_click_alone() {
    // A pane's window runs `handle_shortcuts` for its own input every frame.
    // With nothing pressed it must not claim `focused_panel`, or a click in
    // the main window would be forgotten by the very next window to be drawn.
    let mut h = Harness::new();
    h.app.focused_panel = PanelKind::Results;
    h.run_with(vec![], |app, ui| {
        app.handle_shortcuts(ui.ctx(), Some(PanelKind::Source))
    });
    assert_eq!(h.app.focused_panel, PanelKind::Results);
    assert!(!h.app.show_search_dialog);
}

#[test]
fn ctrl_f_in_the_query_window_searches_the_tree_last_clicked() {
    // The query pane has no tree of its own: it goes by the last click.
    let mut h = Harness::new();
    h.app.focused_panel = PanelKind::Results;
    h.run_with(ctrl(egui::Key::F), |app, ui| {
        app.handle_shortcuts(ui.ctx(), None)
    });
    assert!(h.app.show_search_dialog);
    assert_eq!(h.app.search_target, PanelKind::Results);
}

#[test]
fn ctrl_f_brings_forward_the_window_that_hosts_the_dialog() {
    // Results is in a window of its own, and Ctrl+F is pressed in the main
    // window: the dialog is drawn in Results' window, which is brought forward.
    let mut h = Harness::new();
    h.app.dock.pop_out(Pane::Results, None, None);
    h.app.focused_panel = PanelKind::Results;
    h.run_with(ctrl(egui::Key::F), |app, ui| {
        app.handle_shortcuts(ui.ctx(), None)
    });
    assert_eq!(h.app.raise_window, Some(Pane::Results.viewport_id()));
}

#[test]
fn ctrl_f_in_the_window_that_hosts_the_dialog_raises_nothing() {
    // Results docked, Ctrl+F pressed in the main window: the dialog is drawn
    // right there, so there is no other window to bring forward.
    let mut h = Harness::new();
    h.app.focused_panel = PanelKind::Results;
    h.run_with(ctrl(egui::Key::F), |app, ui| {
        app.handle_shortcuts(ui.ctx(), None)
    });
    assert!(h.app.show_search_dialog);
    assert_eq!(h.app.raise_window, None);
}

#[test]
fn a_dialog_is_drawn_only_by_the_window_that_hosts_it() {
    let mut h = Harness::new();
    h.app.dock.pop_out(Pane::Results, None, None);
    h.app.open_search_dialog(PanelKind::Results);
    // The main window's pass leaves it out…
    for _ in 0..6 {
        h.run_with(vec![], |app, ui| {
            app.dialogs(ui.ctx(), egui::ViewportId::ROOT)
        });
    }
    assert!(h.text_pos("Search — Results").is_none());
    // …the Results window's pass draws it.
    for _ in 0..6 {
        h.run_with(vec![], |app, ui| {
            app.dialogs(ui.ctx(), Pane::Results.viewport_id())
        });
    }
    assert!(h.text_pos("Search — Results").is_some());
}

#[test]
fn a_trees_window_is_the_one_its_dialogs_and_reveals_go_to() {
    let mut h = Harness::new();
    assert_eq!(
        h.app.viewport_of(PanelKind::Results),
        egui::ViewportId::ROOT
    );
    h.app.dock.pop_out(Pane::Results, None, None);
    assert_eq!(
        h.app.viewport_of(PanelKind::Results),
        Pane::Results.viewport_id()
    );
    assert_eq!(h.app.viewport_of(PanelKind::Source), egui::ViewportId::ROOT);

    // Revealing a row brings the window its tree is in forward — the main
    // window's too, when the reveal was asked for in another window.
    h.app.reveal_in_tree(PanelKind::Source, vec![]);
    assert_eq!(h.app.raise_window, Some(egui::ViewportId::ROOT));
    h.app.reveal_in_tree(PanelKind::Results, vec![]);
    assert_eq!(h.app.raise_window, Some(Pane::Results.viewport_id()));
}

#[derive(Debug)]
struct FakeDroppedFile(std::path::PathBuf);

impl egui::DroppedFile for FakeDroppedFile {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

#[test]
fn an_embedded_pane_window_does_not_open_the_dropped_file_twice() {
    // In an embedded window the pane shares the main window's input — and its
    // dropped file, which the main window has opened already.
    let mut h = Harness::new();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
        time: Some(h.time),
        dropped_files: vec![std::sync::Arc::new(FakeDroppedFile(
            "/nonexistent/dropped.json".into(),
        ))],
        ..Default::default()
    };
    let app = &mut h.app;
    let mut out = h.ctx.run_ui(input, |ui| {
        app.handle_drag_and_drop(ui, false);
        assert_eq!(app.source_input, "", "not consumed by an embedded window");
        app.handle_drag_and_drop(ui, true);
        assert_eq!(
            app.source_input, "/nonexistent/dropped.json",
            "consumed by a window that owns its input"
        );
    });
    out.textures_delta.clear();
}

// A pane's window, when eframe redraws it by itself and the main window may not
// be redrawn at all (it is not while it is hidden). `popped_window_frame` is
// what such a window runs; here it runs with no frame of the main window.

fn window_frame(h: &mut Harness, pane: Pane) {
    h.run_with(Vec::new(), move |app, ui| app.popped_window_frame(ui, pane));
}

#[test]
fn a_window_docks_its_own_pane_without_help_from_the_main_window() {
    let mut h = Harness::new();
    h.app.dock.pop_out(Pane::Query, None, None);
    window_frame(&mut h, Pane::Query);
    assert!(
        h.app.dock.is_popped(Pane::Query),
        "an ordinary frame changes nothing"
    );

    h.app.dock.request_dock(Pane::Query);
    window_frame(&mut h, Pane::Query);
    assert!(
        !h.app.dock.is_popped(Pane::Query),
        "the window's own frame applies the request; the main window's would not run"
    );

    // From then on it only waits for the main window to leave it out.
    window_frame(&mut h, Pane::Query);
    assert!(h.text_pos("Back in the main window").is_some());
}

#[test]
fn a_window_whose_pane_is_docked_gets_out_of_the_way_if_the_main_window_never_drops_it() {
    let mut h = Harness::new();
    h.app.dock.pop_out(Pane::Query, None, None);
    h.app.dock.request_dock(Pane::Query);
    window_frame(&mut h, Pane::Query);
    for _ in 0..10 {
        window_frame(&mut h, Pane::Query);
    }
    assert!(
        !h.commands.contains(&egui::ViewportCommand::Minimized(true)),
        "not at once: the main window usually drops it in its next frame"
    );
    for _ in 0..40 {
        window_frame(&mut h, Pane::Query);
    }
    assert!(
        h.commands.contains(&egui::ViewportCommand::Minimized(true)),
        "after a while it minimizes itself — the pane is docked, only the window lingers"
    );
}

// The icon in each pane header's top right corner.

/// Load `json` as the source document (the worker thread does it, so the frames
/// that drain its answer are run until it is in). When a document is already
/// shown, that one stays until the worker's answer replaces it, so "a document
/// is shown" is not the signal: waiting for it to be a different one is.
fn load(h: &mut Harness, json: &str) {
    let before = h.app.doc.clone();
    h.app.open_text(json.to_owned());
    for _ in 0..300 {
        h.frame();
        let replaced = match (&before, &h.app.doc) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(was), Some(now)) => !Arc::ptr_eq(was, now),
        };
        if replaced {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(h.app.doc.is_some(), "the document loaded");
    h.settle();
}

#[test]
fn each_headers_icon_is_in_its_top_right_corner_right_of_the_headers_own_controls() {
    let mut h = Harness::new();
    load(&mut h, r#"{"members":[{"name":"Ann"}]}"#);
    let icons = h.pop_buttons("⬈");
    assert_eq!(icons.len(), 3, "one per pane: {icons:?}");
    let (query, source, results) = (icons[0], icons[1], icons[2]);
    let texts = text_rects(&h.shapes);
    let right_edge_of = |text: &str| {
        texts
            .iter()
            .find(|(t, _)| t == text)
            .unwrap_or_else(|| panic!("{text:?} is drawn"))
            .1
            .max
            .x
    };
    // The square the icon is clicked in ends at the panel's content edge,
    // which is 8px in from the panel's own.
    let far_right = |centre: egui::Pos2, panel_right: f32| {
        let edge = centre.x + POP_BUTTON_SIZE / 2.0;
        assert!(
            (edge - (panel_right - 8.0)).abs() < 1.5,
            "icon at {centre:?} ends at {edge}, the content at {}",
            panel_right - 8.0
        );
    };

    let query_panel = h.query_panel();
    far_right(query, query_panel.right());
    assert!(
        query.y - query_panel.top() < 24.0,
        "the query icon is in the header row, at {query:?}"
    );
    for engine in ["jq", "Pointer", "JSONPath", "JMESPath", "Engine:"] {
        assert!(
            right_edge_of(engine) < query.x - POP_BUTTON_SIZE / 2.0,
            "{engine} is left of the query icon"
        );
    }

    let source_panel = h.panel("source_panel").expect("source panel drawn");
    far_right(source, source_panel.right());
    assert!((source.y - results.y).abs() < 1.0, "the two share a row");
    // The results' panel is the central one: what is left of the window.
    far_right(results, SCREEN.x);
    // "Save…" is in both headers, each just left of that header's icon.
    let saves: Vec<f32> = texts
        .iter()
        .filter(|(t, _)| t == "Save…")
        .map(|(_, r)| r.max.x)
        .collect();
    assert_eq!(saves.len(), 2, "Source's and Results': {saves:?}");
    for save in saves {
        assert!(
            [source, results]
                .iter()
                .any(|icon| save < icon.x - POP_BUTTON_SIZE / 2.0 && save > icon.x - 80.0),
            "a Save… ending at {save} sits left of its icon"
        );
    }
}

#[test]
fn the_icon_is_quiet() {
    let h = Harness::new();
    let glyph = text_rects(&h.shapes)
        .into_iter()
        .find(|(t, _)| t == "⬈")
        .expect("the icon is drawn")
        .1;
    assert!(
        glyph.width() < POP_BUTTON_SIZE && glyph.height() < POP_BUTTON_SIZE,
        "the glyph {glyph:?} fits well inside its {POP_BUTTON_SIZE}px square"
    );
}

#[test]
fn the_whole_square_around_the_icon_pops_the_pane_out_and_nothing_beside_it_does() {
    let mut h = Harness::new();
    let icon = h.pop_buttons("⬈")[0];
    // Just outside the square — and the few pixels around a widget that egui
    // still counts as on it — to its right (the panel's margin) and below.
    let reach = POP_BUTTON_SIZE / 2.0 + h.ctx.global_style().interaction.interact_radius + 1.0;
    for (dx, dy) in [(reach, 0.0), (0.0, reach)] {
        h.click(icon.x + dx, icon.y + dy);
        assert!(
            !h.app.dock.is_popped(Pane::Query),
            "a click {dx},{dy} from the icon's centre is not on it"
        );
    }
    // The square's own corner.
    let corner = POP_BUTTON_SIZE / 2.0 - 1.0;
    h.click(icon.x + corner, icon.y + corner);
    assert!(h.app.dock.is_popped(Pane::Query), "its corner is");
}

// The windows as they are on a desktop. eframe runs the app behind a lock, and
// a popped-out pane's window is a deferred viewport it redraws by itself — the
// main window's frame is not part of that. `Windows` drives it the way eframe
// does: passes of the main window, and passes of a window that never involve it.

struct Windows {
    ctx: egui::Context,
    shared: Arc<Mutex<App>>,
    time: f64,
    /// The windows there are, each with what eframe redraws it by (what the
    /// latest pass listed).
    open: HashMap<egui::ViewportId, Arc<egui::viewport::DeferredViewportUiCallback>>,
    /// What each window's latest pass drew.
    shapes: HashMap<egui::ViewportId, Vec<egui::epaint::ClippedShape>>,
    /// Every viewport command, with the window it was for.
    commands: Vec<(egui::ViewportId, egui::ViewportCommand)>,
    /// When each window asked to be redrawn, as of the latest pass.
    repaint: HashMap<egui::ViewportId, Duration>,
}

impl Windows {
    fn new() -> Self {
        Self::with_app(App::with_context)
    }

    /// The same, with the settings kept in `store`.
    fn with_store(store: Store) -> Self {
        Self::with_app(|ctx| App::with_store(ctx, store))
    }

    fn with_app(make: impl FnOnce(&egui::Context) -> App) -> Self {
        let ctx = egui::Context::default();
        // What eframe does wherever there are windows to open.
        ctx.set_embed_viewports(false);
        let shared = Arc::new(Mutex::new(make(&ctx)));
        let mut w = Self {
            ctx,
            shared,
            time: 0.0,
            open: HashMap::new(),
            shapes: HashMap::new(),
            commands: Vec::new(),
            repaint: HashMap::new(),
        };
        for _ in 0..6 {
            w.main_frame(Vec::new());
        }
        w
    }

    fn app(&self) -> MutexGuard<'_, App> {
        lock(&self.shared)
    }

    fn pass(
        &mut self,
        viewport: egui::ViewportId,
        info: egui::ViewportInfo,
        events: Vec<egui::Event>,
    ) {
        self.pass_sized(viewport, info, events, SCREEN);
    }

    /// A pass of a window that is `size` big.
    fn pass_sized(
        &mut self,
        viewport: egui::ViewportId,
        info: egui::ViewportInfo,
        events: Vec<egui::Event>,
        size: egui::Vec2,
    ) {
        let mut input = egui::RawInput {
            viewport_id: viewport,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        input.viewports.insert(viewport, info);
        self.time += 1.0 / 60.0;
        let shared = Arc::clone(&self.shared);
        let window = self.open.get(&viewport).cloned();
        let mut output = self.ctx.run_ui(input, |ui| match &window {
            Some(redraw) => redraw(ui),
            None => lock(&shared).show_with(ui, Some(&shared)),
        });
        output.textures_delta.clear();
        self.shapes.insert(viewport, output.shapes);
        self.open = output
            .viewport_output
            .iter()
            .filter_map(|(id, v)| Some((*id, v.viewport_ui_cb.clone()?)))
            .collect();
        for (id, v) in &output.viewport_output {
            self.commands
                .extend(v.commands.iter().map(|c| (*id, c.clone())));
        }
        self.repaint = output
            .viewport_output
            .iter()
            .map(|(id, v)| (*id, v.repaint_delay))
            .collect();
    }

    fn main_frame(&mut self, events: Vec<egui::Event>) {
        self.pass(
            egui::ViewportId::ROOT,
            egui::ViewportInfo::default(),
            events,
        );
    }

    fn window_frame(&mut self, pane: Pane, info: egui::ViewportInfo, events: Vec<egui::Event>) {
        self.pass(pane.viewport_id(), info, events);
    }

    /// A few passes of a window that is simply there.
    fn window_settle(&mut self, pane: Pane) {
        self.settle_in(pane.viewport_id());
    }

    fn window_click(&mut self, pane: Pane, pos: egui::Pos2) {
        self.click_in(pane.viewport_id(), pos);
    }

    /// The centre of `text` in the window's latest pass.
    fn window_text(&self, pane: Pane, text: &str) -> egui::Pos2 {
        self.text_in(pane.viewport_id(), text)
            .unwrap_or_else(|| panic!("{text:?} is drawn in the {} window", pane.label()))
    }

    fn pop_out(&mut self, pane: Pane) {
        self.app().dock.pop_out(pane, None, None);
        self.main_frame(Vec::new());
        self.window_settle(pane);
    }

    fn commands_for(&self, pane: Pane) -> Vec<egui::ViewportCommand> {
        self.commands_in(pane.viewport_id())
    }

    // The same for any window, a pane's or another's.

    /// A few passes of the window `viewport` that is simply there.
    fn settle_in(&mut self, viewport: egui::ViewportId) {
        for _ in 0..4 {
            self.pass(viewport, egui::ViewportInfo::default(), Vec::new());
        }
    }

    fn click_in(&mut self, viewport: egui::ViewportId, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let info = egui::ViewportInfo::default;
        self.pass(viewport, info(), vec![egui::Event::PointerMoved(pos)]);
        self.pass(viewport, info(), vec![button(true)]);
        self.pass(viewport, info(), vec![button(false)]);
        self.settle_in(viewport);
    }

    /// The centre of `text` in the latest pass of the window `viewport`, if it
    /// was drawn there.
    fn text_in(&self, viewport: egui::ViewportId, text: &str) -> Option<egui::Pos2> {
        text_rects(self.shapes.get(&viewport)?)
            .into_iter()
            .find(|(t, _)| t == text)
            .map(|(_, rect)| rect.center())
    }

    /// The rectangle `text` fills in the latest pass of the window `viewport`,
    /// if it was drawn there.
    fn rect_in(&self, viewport: egui::ViewportId, text: &str) -> Option<egui::Rect> {
        text_rects(self.shapes.get(&viewport)?)
            .into_iter()
            .find(|(t, _)| t == text)
            .map(|(_, rect)| rect)
    }

    /// Whether any text drawn in the latest pass of the window `viewport`
    /// satisfies `wanted`.
    fn drew_in(&self, viewport: egui::ViewportId, wanted: impl Fn(&str) -> bool) -> bool {
        self.shapes
            .get(&viewport)
            .is_some_and(|shapes| text_rects(shapes).iter().any(|(t, _)| wanted(t)))
    }

    fn commands_in(&self, viewport: egui::ViewportId) -> Vec<egui::ViewportCommand> {
        self.commands
            .iter()
            .filter(|(id, _)| *id == viewport)
            .map(|(_, c)| c.clone())
            .collect()
    }

    /// Open one of the windows besides the panes' the way its button does, and
    /// give the main window the frame in which it appears.
    fn open_satellite(&mut self, which: Satellite) {
        {
            let mut app = self.app();
            match which {
                Satellite::Tools => app.tools.open_or_focus(&self.ctx),
                Satellite::Tutorial => app.tutorial.open_or_focus(&self.ctx),
                Satellite::About => app.info_window.open_or_focus(&self.ctx),
                Satellite::Settings => app.settings_window.open_or_focus(&self.ctx),
            }
        }
        self.main_frame(Vec::new());
    }
}

#[test]
fn a_popped_pane_is_a_window_redrawn_on_its_own_not_part_of_the_main_windows_frame() {
    let mut w = Windows::new();
    assert!(w.open.is_empty(), "nothing is out");
    w.app().dock.pop_out(Pane::Query, None, None);
    w.main_frame(Vec::new());
    assert!(
        w.open.contains_key(&Pane::Query.viewport_id()),
        "the pane has a deferred viewport, which eframe redraws by itself"
    );
    assert_eq!(w.open.len(), 1);
    assert_eq!(
        w.repaint[&Pane::Query.viewport_id()],
        Duration::ZERO,
        "and the main window's frame says it may have something new to show"
    );
}

#[test]
fn a_window_works_while_the_main_window_is_not_redrawn_at_all() {
    // The main window is completely covered by the window — a compositor may
    // then stop sending it redraw callbacks (GNOME does) — so only the
    // window's own passes happen from here on.
    let mut w = Windows::new();
    w.pop_out(Pane::Query);

    assert_eq!(w.app().query_engine, None);
    let pointer = w.window_text(Pane::Query, "Pointer");
    w.window_click(Pane::Query, pointer);
    assert_eq!(
        w.app().query_engine,
        Some(jsonquery_query::Kind::JsonPointer),
        "a click on an engine button reaches the app"
    );

    // Typing too.
    let before = w.app().query_text.clone();
    let editor = w
        .ctx
        .read_response(egui::Id::new("query_text_edit_box"))
        .expect("the query box is drawn")
        .rect;
    w.window_click(Pane::Query, editor.center());
    w.window_frame(
        Pane::Query,
        egui::ViewportInfo::default(),
        vec![egui::Event::Text("zzz".to_owned())],
    );
    w.window_settle(Pane::Query);
    assert_eq!(w.app().query_text, format!("{before}zzz"));

    // And the way back: the window docks its own pane, then waits to be dropped.
    let back = w.window_text(Pane::Query, "⬋");
    w.window_click(Pane::Query, back);
    assert!(!w.app().dock.is_popped(Pane::Query));
    assert!(w
        .window_text(Pane::Query, "Back in the main window")
        .x
        .is_finite());
    // The main window, whenever it next gets a frame, leaves the window out.
    w.main_frame(Vec::new());
    assert!(w.open.is_empty(), "the window is gone: {:?}", w.open.keys());
}

#[test]
fn a_click_in_a_window_asks_the_main_window_to_redraw() {
    // What is done in a window can show in the main one (a row revealed in its
    // tree, a status message), which is told to look. (The window also asks
    // for a frame of its own shortly after, so that a main window that can't be
    // drawn doesn't leave the app polling for it; egui asks for one after any
    // click anyway, so no test can tell — that part was checked by hand.)
    let mut w = Windows::new();
    w.pop_out(Pane::Query);
    let root = egui::ViewportId::ROOT;
    let pos = w.window_text(Pane::Query, "Pointer");
    let pointer = |pos| egui::Event::PointerMoved(pos);
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let info = egui::ViewportInfo::default;
    w.window_frame(Pane::Query, info(), vec![pointer(pos)]);
    w.window_frame(Pane::Query, info(), vec![button(true)]);
    w.main_frame(Vec::new());
    assert_ne!(
        w.repaint[&root],
        Duration::ZERO,
        "nothing has been done yet, so the main window has nothing to look at"
    );
    w.window_frame(Pane::Query, info(), vec![button(false)]);
    assert_eq!(w.repaint[&root], Duration::ZERO, "the click asks for it");
}

#[test]
fn closing_a_maximized_window_docks_its_pane_and_uncovers_the_main_window() {
    let close = |maximized, fullscreen| egui::ViewportInfo {
        events: vec![egui::ViewportEvent::Close],
        maximized,
        fullscreen,
        ..Default::default()
    };

    let mut w = Windows::new();
    w.pop_out(Pane::Query);
    w.window_frame(Pane::Query, close(Some(true), Some(false)), Vec::new());
    assert!(!w.app().dock.is_popped(Pane::Query), "closing docks it");
    assert!(
        w.commands_for(Pane::Query)
            .contains(&egui::ViewportCommand::Maximized(false)),
        "the window gives up covering the main window, whose frames then resume \
         and drop it: {:?}",
        w.commands_for(Pane::Query)
    );

    let mut w = Windows::new();
    w.pop_out(Pane::Source);
    w.window_frame(Pane::Source, close(Some(false), Some(true)), Vec::new());
    assert!(!w.app().dock.is_popped(Pane::Source));
    assert!(w
        .commands_for(Pane::Source)
        .contains(&egui::ViewportCommand::Fullscreen(false)));

    // A window that is not covering anything has nothing to give up.
    let mut w = Windows::new();
    w.pop_out(Pane::Results);
    w.window_frame(Pane::Results, close(Some(false), Some(false)), Vec::new());
    assert!(!w.app().dock.is_popped(Pane::Results));
    assert!(w.commands_for(Pane::Results).iter().all(|c| !matches!(
        c,
        egui::ViewportCommand::Maximized(_) | egui::ViewportCommand::Fullscreen(_)
    )));
}

#[test]
fn the_dock_button_in_a_maximized_window_docks_its_pane_too() {
    let maximized = || egui::ViewportInfo {
        maximized: Some(true),
        ..Default::default()
    };
    let mut w = Windows::new();
    w.pop_out(Pane::Query);
    let back = w.window_text(Pane::Query, "⬋");
    let button = |pressed| egui::Event::PointerButton {
        pos: back,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    w.window_frame(
        Pane::Query,
        maximized(),
        vec![egui::Event::PointerMoved(back)],
    );
    w.window_frame(Pane::Query, maximized(), vec![button(true)]);
    w.window_frame(Pane::Query, maximized(), vec![button(false)]);
    w.window_frame(Pane::Query, maximized(), Vec::new());
    assert!(!w.app().dock.is_popped(Pane::Query));
    assert!(w
        .commands_for(Pane::Query)
        .contains(&egui::ViewportCommand::Maximized(false)));
}

// The windows besides the panes': the Tools window, the tutorial and About. Each
// is redrawn on its own as a pane's window is, so that it keeps working when the
// main window is not redrawn at all (a window maximized over it is why).
// `satellite_frame` is what such a window runs.

#[test]
fn the_worker_wakes_every_window_not_just_the_main_one() {
    // A window redrawn on its own would otherwise sit on the worker's answer
    // (the Tools window's job) until something else happened to it.
    let ctx = egui::Context::default();
    wake_windows(&ctx);
    let mut windows = vec![egui::ViewportId::ROOT];
    windows.extend(Pane::ALL.iter().map(|p| p.viewport_id()));
    windows.extend(Satellite::ALL.iter().map(|w| w.viewport_id()));
    for window in windows {
        assert!(
            ctx.has_requested_repaint_for(&window),
            "{window:?} is woken"
        );
    }
}

#[test]
fn the_tools_tutorial_and_about_windows_are_redrawn_on_their_own_not_as_part_of_the_main_windows_frame(
) {
    let mut w = Windows::new();
    assert!(w.open.is_empty(), "nothing is out");
    for which in Satellite::ALL {
        w.open_satellite(which);
        let id = which.viewport_id();
        assert!(
            w.open.contains_key(&id),
            "{which:?} has a deferred viewport, which eframe redraws by itself"
        );
        assert_eq!(
            w.repaint[&id],
            Duration::ZERO,
            "{which:?}: the main window's frame says it may have something new to show"
        );
    }
    assert_eq!(w.open.len(), 4, "and nothing else is a window");
}

#[test]
fn the_tools_window_takes_clicks_while_the_main_window_is_not_redrawn_at_all() {
    // The window covers the main window completely: a compositor may then send
    // the main window no redraw callbacks (GNOME does), so only the window's
    // own passes happen from here on.
    let mut w = Windows::new();
    let tools = Satellite::Tools.viewport_id();
    w.open_satellite(Satellite::Tools);
    w.settle_in(tools);
    assert!(
        w.text_in(tools, "Merge").is_some(),
        "the first page is Merge's"
    );

    let tab = w.text_in(tools, "Diff JSON").expect("the tabs are drawn");
    w.click_in(tools, tab);
    assert!(
        w.text_in(tools, "Compare").is_some(),
        "a click on a tab reaches the app: {:?}",
        w.shapes.get(&tools).map(|s| text_rects(s))
    );
    assert!(w.text_in(tools, "Merge").is_none(), "and the page changed");
}

#[test]
fn a_tool_takes_typing_and_runs_and_its_answer_arrives_while_the_main_window_is_not_redrawn() {
    let mut w = Windows::new();
    let tools = Satellite::Tools.viewport_id();
    {
        let mut app = w.app();
        app.tools.open_on(&w.ctx, Tool::Format);
    }
    w.main_frame(Vec::new());
    w.settle_in(tools);

    // Typing…
    let hint = "Paste JSON here, or drop a file";
    let at = w.text_in(tools, hint).expect("the box is drawn, empty");
    w.click_in(tools, at);
    w.pass(
        tools,
        egui::ViewportInfo::default(),
        vec![egui::Event::Text(r#"{"b":1,"a":[1,2]}"#.to_owned())],
    );
    w.settle_in(tools);
    assert!(
        w.text_in(tools, hint).is_none(),
        "what was typed is in the box"
    );

    // …running it: the worker answers in its own time, and the window's own
    // passes are what take the answer in (no pass of the main window happens).
    let run = w.text_in(tools, "Format").expect("the page's button");
    w.click_in(tools, run);
    let formatted = "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2\n  ]\n}";
    for _ in 0..300 {
        w.pass(tools, egui::ViewportInfo::default(), Vec::new());
        if w.drew_in(tools, |t| t == formatted) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(w.drew_in(tools, |t| t == formatted), "the answer is shown");
    assert!(
        w.drew_in(tools, |t| t.starts_with("Formatted in")),
        "and the status bar says so"
    );
}

#[test]
fn a_click_in_the_tools_window_asks_the_main_window_to_redraw() {
    // What is done in the window can show in the main one (a document opened in
    // it, a status message), which is told to look. (The window also asks for
    // a frame of its own shortly after, so that a main window that can't be
    // drawn doesn't leave the app polling for it; egui asks for one after any
    // click anyway, so no test can tell — that part was checked by hand.)
    let mut w = Windows::new();
    let tools = Satellite::Tools.viewport_id();
    w.open_satellite(Satellite::Tools);
    w.settle_in(tools);
    let root = egui::ViewportId::ROOT;
    let pos = w.text_in(tools, "Diff JSON").expect("the tabs are drawn");
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let info = egui::ViewportInfo::default;
    w.pass(tools, info(), vec![egui::Event::PointerMoved(pos)]);
    w.pass(tools, info(), vec![button(true)]);
    // (egui serves each request for a frame with two of them.)
    for _ in 0..3 {
        w.main_frame(Vec::new());
    }
    assert_ne!(
        w.repaint[&root],
        Duration::ZERO,
        "nothing has been done yet, so the main window has nothing to look at"
    );
    w.pass(tools, info(), vec![button(false)]);
    assert_eq!(w.repaint[&root], Duration::ZERO, "the click asks for it");
}

#[test]
fn the_tutorial_hands_a_query_to_the_main_window_without_a_frame_of_it() {
    let mut w = Windows::new();
    let tutorial = Satellite::Tutorial.viewport_id();
    w.open_satellite(Satellite::Tutorial);
    w.settle_in(tutorial);
    assert_eq!(w.app().query_text, "");

    let load = w
        .text_in(tutorial, "Load query")
        .expect("an example has its buttons");
    w.click_in(tutorial, load);
    assert_ne!(
        w.app().query_text,
        "",
        "the request is applied by the window's own frame"
    );
    assert!(
        w.commands_in(egui::ViewportId::ROOT)
            .contains(&egui::ViewportCommand::Focus),
        "and the main window is brought forward to show it"
    );
}

/// What a window's frame is given when the user has asked to close it.
fn close_request(maximized: bool, fullscreen: bool) -> egui::ViewportInfo {
    egui::ViewportInfo {
        events: vec![egui::ViewportEvent::Close],
        maximized: Some(maximized),
        fullscreen: Some(fullscreen),
        ..Default::default()
    }
}

#[test]
fn closing_a_maximized_window_uncovers_the_main_window_and_the_main_window_then_drops_it() {
    for which in Satellite::ALL {
        let id = which.viewport_id();

        let mut w = Windows::new();
        w.open_satellite(which);
        w.settle_in(id);
        w.pass(id, close_request(true, false), Vec::new());
        assert!(
            !w.app().satellite_open(which),
            "{which:?}: closing closes it"
        );
        assert!(
            w.commands_in(id)
                .contains(&egui::ViewportCommand::Maximized(false)),
            "{which:?}: the window gives up covering the main window, whose frames then \
             resume and drop it: {:?}",
            w.commands_in(id)
        );
        assert!(
            w.text_in(id, "Closing…").is_some(),
            "{which:?}: until then it says so"
        );
        // The main window, whenever it next gets a frame, leaves the window out.
        w.main_frame(Vec::new());
        assert!(!w.open.contains_key(&id), "{which:?}: the window is gone");

        let mut w = Windows::new();
        w.open_satellite(which);
        w.settle_in(id);
        w.pass(id, close_request(false, true), Vec::new());
        assert!(!w.app().satellite_open(which));
        assert!(
            w.commands_in(id)
                .contains(&egui::ViewportCommand::Fullscreen(false)),
            "{which:?}: a fullscreen window covers the main window too"
        );

        // A window that is not covering anything has nothing to give up.
        let mut w = Windows::new();
        w.open_satellite(which);
        w.settle_in(id);
        w.pass(id, close_request(false, false), Vec::new());
        assert!(!w.app().satellite_open(which));
        assert!(
            w.commands_in(id).iter().all(|c| !matches!(
                c,
                egui::ViewportCommand::Maximized(_) | egui::ViewportCommand::Fullscreen(_)
            )),
            "{which:?}: {:?}",
            w.commands_in(id)
        );
    }
}

#[test]
fn a_closed_window_gets_out_of_the_way_if_the_main_window_never_drops_it() {
    for which in Satellite::ALL {
        let id = which.viewport_id();
        let mut w = Windows::new();
        w.open_satellite(which);
        w.settle_in(id);
        w.pass(id, close_request(false, false), Vec::new());
        for _ in 0..10 {
            w.pass(id, egui::ViewportInfo::default(), Vec::new());
        }
        assert!(
            !w.commands_in(id)
                .contains(&egui::ViewportCommand::Minimized(true)),
            "{which:?}: not at once — the main window usually drops it in its next frame"
        );
        for _ in 0..40 {
            w.pass(id, egui::ViewportInfo::default(), Vec::new());
        }
        assert!(
            w.commands_in(id)
                .contains(&egui::ViewportCommand::Minimized(true)),
            "{which:?}: after a while it minimizes itself — it is closed already, only \
             the window lingers"
        );
    }
}

#[test]
fn a_closed_window_opens_again() {
    for which in Satellite::ALL {
        let id = which.viewport_id();

        // After the main window dropped it: a new window.
        let mut w = Windows::new();
        w.open_satellite(which);
        w.settle_in(id);
        w.pass(id, close_request(false, false), Vec::new());
        w.main_frame(Vec::new());
        assert!(!w.open.contains_key(&id), "{which:?} was dropped");
        w.open_satellite(which);
        assert!(w.open.contains_key(&id), "{which:?} is a window again");
        w.settle_in(id);
        assert!(w.app().satellite_open(which));
        assert!(
            w.text_in(id, "Closing…").is_none(),
            "{which:?} shows its own content"
        );

        // Before it did: the window that still is there is the one.
        let mut w = Windows::new();
        w.open_satellite(which);
        w.settle_in(id);
        w.pass(id, close_request(false, false), Vec::new());
        w.open_satellite(which);
        assert!(w.open.contains_key(&id), "{which:?} was never dropped");
        w.settle_in(id);
        assert!(w.app().satellite_open(which));
        assert!(
            w.text_in(id, "Closing…").is_none(),
            "{which:?} shows its own content again"
        );
    }
}

// The headers of the panes — a small title, and no line under it — and the
// source field the Source pane's window has.

/// The horizontal lines at least `min_len` long in `shapes`, as `(y, left, right)`.
fn horizontal_lines(shapes: &[egui::epaint::ClippedShape], min_len: f32) -> Vec<(f32, f32, f32)> {
    fn walk(shape: &egui::Shape, min_len: f32, out: &mut Vec<(f32, f32, f32)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, min_len, out)),
            egui::Shape::LineSegment { points, .. }
                if (points[0].y - points[1].y).abs() < 0.01
                    && (points[0].x - points[1].x).abs() >= min_len =>
            {
                let (a, b) = (points[0].x, points[1].x);
                out.push((points[0].y, a.min(b), a.max(b)));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, min_len, &mut out);
    }
    out
}

/// The lines drawn just under a header's title (in the next 14 points): there
/// should be none — what is under a header is its contents.
fn lines_under(shapes: &[egui::epaint::ClippedShape], title: egui::Rect) -> Vec<(f32, f32, f32)> {
    horizontal_lines(shapes, 100.0)
        .into_iter()
        .filter(|(y, ..)| *y > title.bottom() && *y < title.bottom() + 14.0)
        .collect()
}

#[test]
fn a_panes_title_is_smaller_than_a_heading_and_has_no_line_under_it() {
    let mut h = Harness::new();
    let heading = h.ctx.global_style().text_styles[&egui::TextStyle::Heading].size;
    for loaded in [false, true] {
        if loaded {
            load(&mut h, r#"{"members":[{"name":"Ann"}]}"#);
        }
        for title in ["Source", "Results"] {
            let rect = rect_of(&h, title);
            assert!(
                rect.height() < heading,
                "loaded={loaded}: {title:?} is {} tall, no smaller than a heading ({heading})",
                rect.height()
            );
            assert_eq!(
                lines_under(&h.shapes, rect),
                vec![],
                "loaded={loaded}: {title:?} has a line under it"
            );
        }
    }
}

#[test]
fn the_empty_source_panes_hint_comes_straight_under_its_header() {
    let h = Harness::new();
    let title = rect_of(&h, "Source");
    let hint = rect_of(
        &h,
        "Drag & drop a file anywhere, or enter a URL or path above.",
    );
    let gap = hint.top() - title.bottom();
    assert!(
        (0.0..8.0).contains(&gap),
        "{gap} between the title {title:?} and the hint {hint:?}"
    );
}

#[test]
fn the_tools_boxes_have_the_same_small_titles_and_no_line_under_their_headers() {
    for (tool, titles) in [
        (Tool::Merge, ["Files", "Result"]),
        (Tool::Format, ["Input", "Result"]),
        (Tool::Diff, ["Left", "Right"]),
        (Tool::Patch, ["Document", "Result"]),
        (Tool::Validate, ["Document", "Problems"]),
    ] {
        let mut h = Harness::new();
        open_tool(&mut h, tool);
        let heading = h.ctx.global_style().text_styles[&egui::TextStyle::Heading].size;
        let main = rect_of(&h, "Results");
        for title in titles {
            let rect = rect_of(&h, title);
            assert!(
                rect.height() < heading,
                "{tool:?}: {title:?} is {} tall, no smaller than a heading ({heading})",
                rect.height()
            );
            assert!(
                (rect.height() - main.height()).abs() < 0.01,
                "{tool:?}: {title:?} is as big as the main window's titles ({})",
                main.height()
            );
            assert_eq!(
                lines_under(&h.shapes, rect),
                vec![],
                "{tool:?}: {title:?} has a line under it"
            );
        }
    }
}

/// The texts along the top of the Source window — its row with the source
/// field — left to right, as `(text, left, right)`.
fn source_row_of(w: &Windows) -> Vec<(String, f32, f32)> {
    let mut row: Vec<_> = text_rects(&w.shapes[&Pane::Source.viewport_id()])
        .into_iter()
        .filter(|(t, r)| !t.is_empty() && r.center().y < 24.0)
        .map(|(t, r)| (t, r.min.x, r.max.x))
        .collect();
    row.sort_by(|a, b| a.1.total_cmp(&b.1));
    row
}

/// A document of the given kind, as the toolbar's line says something about
/// each differently: where it came from, how big it is, how many records.
fn a_document(source: DocumentSource, records: usize) -> Arc<Document> {
    let mut doc = Document::from_value(
        serde_json::json!([1, 2, 3]),
        source,
        123_456,
        Duration::from_millis(5),
    );
    doc.top_level_values = records;
    Arc::new(doc)
}

/// Passes of the window `viewport` — and none of the main window — until `done`:
/// the worker thread answers in its own time.
fn wait_in(
    w: &mut Windows,
    viewport: egui::ViewportId,
    what: &str,
    done: impl Fn(&Windows) -> bool,
) {
    for _ in 0..300 {
        w.pass(viewport, egui::ViewportInfo::default(), Vec::new());
        if done(w) {
            w.settle_in(viewport);
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("waited for {what}");
}

#[test]
fn the_window_of_a_popped_out_source_has_a_source_field_of_its_own() {
    let mut w = Windows::new();
    let (root, source) = (egui::ViewportId::ROOT, Pane::Source.viewport_id());
    assert!(w.text_in(root, "Source:").is_some(), "the toolbar's");

    w.pop_out(Pane::Source);
    // A row along the window's top, as the toolbar's: the label, the field (its
    // hint, while it is empty) and the three buttons…
    for text in ["Source:", "URL or local path…", "…", "Load", "Clear"] {
        let rect = w
            .rect_in(source, text)
            .unwrap_or_else(|| panic!("{text:?} is in the Source window"));
        assert!(
            rect.center().y < 24.0,
            "{text:?} is in the row along the top: {rect:?}"
        );
    }
    // …above the pane's own header.
    let row = w.rect_in(source, "Source:").unwrap();
    let title = w.rect_in(source, "Source").expect("the pane's title");
    assert!(
        title.top() > row.bottom(),
        "the title {title:?} is under the row {row:?}"
    );

    // The main window keeps its field, and the other panes' windows have none.
    w.main_frame(Vec::new());
    assert!(
        w.text_in(root, "Source:").is_some(),
        "the toolbar still has its field"
    );
    for pane in [Pane::Query, Pane::Results] {
        w.pop_out(pane);
        assert!(
            w.text_in(pane.viewport_id(), "Source:").is_none(),
            "the {} window has no source field",
            pane.label()
        );
    }
}

#[test]
fn the_source_window_has_its_field_where_there_are_no_native_windows_too() {
    let mut h = Harness::new();
    let fields = |h: &Harness| h.texts().iter().filter(|(t, _)| t == "Source:").count();
    assert_eq!(fields(&h), 1, "the toolbar's");
    h.app.dock.pop_out(Pane::Source, None, None);
    h.settle();
    assert_eq!(
        fields(&h),
        2,
        "the toolbar's and the floating window's: {:?}",
        h.texts()
    );
    h.app.dock.dock(Pane::Source);
    h.settle();
    assert_eq!(fields(&h), 1, "docked, the window and its field are gone");
}

#[test]
fn what_is_typed_in_the_source_windows_field_is_the_toolbars_text_too() {
    let mut w = Windows::new();
    let (root, source) = (egui::ViewportId::ROOT, Pane::Source.viewport_id());
    w.pop_out(Pane::Source);
    let hint = w
        .text_in(source, "URL or local path…")
        .expect("the field is drawn, empty");
    w.click_in(source, hint);

    // (egui serves each request for a frame with two, so the click's — it asked
    // the main window to look — takes a few frames of it to be over.)
    for _ in 0..3 {
        w.main_frame(Vec::new());
    }
    assert_ne!(
        w.repaint[&root],
        Duration::ZERO,
        "nothing has been typed yet, so the main window has nothing new to show"
    );
    w.pass(
        source,
        egui::ViewportInfo::default(),
        vec![egui::Event::Text("/some/where/data.json".to_owned())],
    );
    assert_eq!(w.app().source_input, "/some/where/data.json");
    assert_eq!(
        w.repaint[&root],
        Duration::ZERO,
        "the window asks the main window to show it in its own field"
    );
    w.main_frame(Vec::new());
    assert!(
        w.drew_in(root, |t| t == "/some/where/data.json"),
        "the toolbar's field shows it: {:?}",
        w.shapes.get(&root).map(|s| text_rects(s))
    );
}

#[test]
fn a_file_is_opened_from_the_source_windows_field_and_cleared_from_it() {
    // Only the window's own passes happen: nothing here involves the main
    // window's frame, which a covered window would not get.
    let mut w = Windows::new();
    let source = Pane::Source.viewport_id();
    let path = temp_json(
        "source_window_field.json",
        r#"{"opened":"from its window"}"#,
    );
    let typed = path.display().to_string();
    w.pop_out(Pane::Source);

    let hint = w.text_in(source, "URL or local path…").unwrap();
    w.click_in(source, hint);
    w.pass(
        source,
        egui::ViewportInfo::default(),
        vec![egui::Event::Text(typed.clone())],
    );
    w.settle_in(source);
    // Enter loads it…
    for pressed in [true, false] {
        w.pass(
            source,
            egui::ViewportInfo::default(),
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: Some(egui::Key::Enter),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    wait_in(&mut w, source, "the document", |w| w.app().doc.is_some());
    assert!(
        w.drew_in(source, |t| t == "Save…"),
        "the window's pane shows what was loaded: {:?}",
        w.shapes.get(&source).map(|s| text_rects(s))
    );
    assert_eq!(w.app().source_input, typed, "the field names what is open");

    // …and Clear unloads it, emptying the field.
    let clear = w.text_in(source, "Clear").unwrap();
    w.click_in(source, clear);
    assert!(w.app().doc.is_none(), "Clear unloads the document");
    assert_eq!(w.app().source_input, "");
    assert!(
        w.text_in(source, "URL or local path…").is_some(),
        "the field is empty again"
    );

    // The Load button does the same as Enter.
    w.click_in(source, w.text_in(source, "URL or local path…").unwrap());
    w.pass(
        source,
        egui::ViewportInfo::default(),
        vec![egui::Event::Text(typed)],
    );
    w.settle_in(source);
    let load = w.text_in(source, "Load").unwrap();
    w.click_in(source, load);
    wait_in(&mut w, source, "the document, again", |w| {
        w.app().doc.is_some()
    });
}

#[test]
fn a_load_that_fails_is_said_in_the_source_window_and_goes_with_clear() {
    // The window has no status bar, and the main window may be out of sight.
    let mut w = Windows::new();
    let source = Pane::Source.viewport_id();
    w.pop_out(Pane::Source);
    let hint = w.text_in(source, "URL or local path…").unwrap();
    w.click_in(source, hint);
    w.pass(
        source,
        egui::ViewportInfo::default(),
        vec![egui::Event::Text("/no/such/dir/missing.json".to_owned())],
    );
    w.settle_in(source);
    let load = w.text_in(source, "Load").unwrap();
    w.click_in(source, load);
    wait_in(&mut w, source, "the error", |w| {
        w.app().load_error.is_some()
    });
    assert!(
        w.drew_in(source, |t| t.starts_with("Load error: ")),
        "{:?}",
        w.shapes.get(&source).map(|s| text_rects(s))
    );

    let clear = w.text_in(source, "Clear").unwrap();
    w.click_in(source, clear);
    assert!(w.app().load_error.is_none());
    assert!(
        !w.drew_in(source, |t| t.starts_with("Load error")),
        "Clear takes the error away"
    );
}

#[test]
fn the_source_windows_row_has_room_for_everything_it_says() {
    let source = Pane::Source.viewport_id();
    let min_width = Dock::default()
        .window_builder(Pane::Source)
        .min_inner_size
        .expect("the window has a least size")
        .x;
    // The window as it opens from a pane docked at its usual width, and as
    // narrow as it can be made.
    for width in [592.0, min_width] {
        let mut w = Windows::new();
        w.pop_out(Pane::Source);
        let size = egui::vec2(width, 600.0);
        let mut states: Vec<(&str, Option<Arc<Document>>)> = vec![("empty", None)];
        for (state, doc) in [
            ("pasted", a_document(DocumentSource::Pasted, 1)),
            (
                "a file",
                a_document(DocumentSource::File("/a/data.json".into()), 1),
            ),
            ("NDJSON", a_document(DocumentSource::Pasted, 120)),
            (
                "merged",
                a_document(
                    DocumentSource::Merged(vec!["a".into(), "b".into(), "c".into()]),
                    1,
                ),
            ),
            (
                "patched",
                a_document(
                    DocumentSource::Derived {
                        label: "(patched)",
                        file_name: "patched.json",
                    },
                    1,
                ),
            ),
        ] {
            states.push((state, Some(doc)));
        }
        for (state, doc) in states {
            if let Some(doc) = doc {
                w.app().document_loaded(doc);
            }
            for _ in 0..6 {
                w.pass_sized(source, egui::ViewportInfo::default(), Vec::new(), size);
            }
            let row = source_row_of(&w);
            let (_, _, clear_right) = row
                .iter()
                .find(|(t, ..)| t == "Clear")
                .unwrap_or_else(|| panic!("{state}, {width} wide: Clear is in the row: {row:?}"))
                .clone();
            assert!(
                clear_right <= width,
                "{state}, {width} wide: Clear ends at {clear_right}: {row:?}"
            );
            // What it says about what is loaded fits as well, unless the window is
            // as narrow as it goes (when it is cut off at the edge).
            if width > min_width {
                let right = row.iter().map(|(_, _, r)| *r).fold(0.0, f32::max);
                assert!(
                    right <= width,
                    "{state}, {width} wide: ends at {right}: {row:?}"
                );
            }
        }
    }
}

// The Tools window (the 🛠 button) and its Merge JSON.

/// A JSON file with `json` in it, in a folder of its own that goes when it does.
fn temp_json(name: &str, json: &str) -> ScratchFile {
    let file = ScratchDir::new("tools-test", "json").into_file(name);
    std::fs::write(&file, json).unwrap();
    file
}

/// The middle of `text`, wherever it was drawn last frame.
fn center_of(h: &Harness, text: &str) -> egui::Pos2 {
    text_rects(&h.shapes)
        .into_iter()
        .find(|(t, _)| t == text)
        .unwrap_or_else(|| panic!("{text:?} is drawn"))
        .1
        .center()
}

fn is_drawn(h: &Harness, text: &str) -> bool {
    h.texts().iter().any(|(t, _)| t == text)
}

/// Frames until `done` (the worker thread answers in its own time).
fn wait_for(h: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
    for _ in 0..300 {
        h.frame();
        if done(h) {
            h.settle();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("waited for {what}; drawn: {:?}", h.texts());
}

#[test]
fn the_tools_button_sits_beside_the_tutorial_button_and_opens_the_window() {
    let mut h = Harness::new();
    let tools = h.pop_buttons("🛠");
    let tutorial = h.pop_buttons("📖");
    let autocomplete = h.pop_buttons("💡");
    assert_eq!((tools.len(), tutorial.len(), autocomplete.len()), (1, 1, 1));
    let (tools, tutorial, autocomplete) = (tools[0], tutorial[0], autocomplete[0]);
    assert!(tools.x < tutorial.x, "left of 📖: {tools:?} {tutorial:?}");
    assert_eq!(tools.y, tutorial.y, "same row");
    assert!(
        ((tutorial.x - tools.x) - (autocomplete.x - tutorial.x)).abs() < 3.0,
        "spaced like its neighbours: {tools:?} {tutorial:?} {autocomplete:?}"
    );

    assert!(!h.app.tools.is_open());
    h.click(tools.x, tools.y);
    assert!(h.app.tools.is_open());
    assert!(is_drawn(&h, "Merge JSON"), "{:?}", h.texts());
}

#[test]
fn merging_two_files_and_opening_the_result_in_the_main_window() {
    let mut h = Harness::new();
    let a = temp_json("merge_a.json", "[1, 2]");
    let b = temp_json("merge_b.json", "[3]");
    let ctx = h.ctx.clone();
    h.app
        .tools
        .add_files(&ctx, vec![a.to_path_buf(), b.to_path_buf()]);
    h.settle();
    assert!(is_drawn(&h, "Files (2)"), "{:?}", h.texts());
    assert!(is_drawn(&h, "merge_a.json") && is_drawn(&h, "merge_b.json"));

    let merge = center_of(&h, "Merge");
    h.click(merge.x, merge.y);
    wait_for(&mut h, "the merged preview", |h| {
        is_drawn(h, "[\n  1,\n  2,\n  3\n]")
    });
    assert!(
        is_drawn(&h, "array · 2 items · 6 B"),
        "each file says what it was: {:?}",
        h.texts()
    );
    assert!(h.app.doc.is_none(), "nothing is opened until asked");

    let open = center_of(&h, "Open in main window");
    h.click(open.x, open.y);
    let doc = h.app.doc.clone().expect("the merge opened");
    assert_eq!(doc.tree(), Some(&serde_json::json!([1, 2, 3])));
    assert_eq!(doc.source.label(), "(merged from 2 files)");
    assert_eq!(h.app.source_input, "", "nothing to reload from the field");
}

#[test]
fn a_merge_result_as_big_as_files_are_kept_on_disk_from_is_kept_in_a_temporary_file() {
    let mut h = Harness::new();
    // About 5 KB pretty-printed (a line to an item): far under what a file is
    // kept on disk from, and over 2 KB.
    let a = temp_json("big_a.json", &format!("[{}1]", "1,".repeat(1000)));
    let b = temp_json("big_b.json", "[2]");
    let ctx = h.ctx.clone();
    h.app
        .tools
        .add_files(&ctx, vec![a.to_path_buf(), b.to_path_buf()]);
    h.settle();

    // As the limits are, it is a value in memory, and the status bar says only
    // what it was made from.
    press(&mut h, "Merge");
    wait_for(&mut h, "the merge", |h| {
        is_drawn_containing(h, "Merged 2 files")
    });
    assert!(
        !is_drawn_containing(&h, "temporary file"),
        "{:?}",
        h.texts()
    );

    // Kept on disk from 2 KB, the same merge is a file, and says so.
    open_settings(&mut h);
    set_limit(&mut h, Limit::KeepOnDisk, "2 KB");
    let current = h.app.settings;
    h.app.settings_window.close(&current);
    h.settle();
    press(&mut h, "Merge");
    wait_for(&mut h, "the merge, kept in a file", |h| {
        is_drawn_containing(h, "kept in a temporary file")
    });
    assert!(is_drawn_containing(&h, "Merged 2 files"), "{:?}", h.texts());
    assert!(
        is_drawn_containing(&h, "array · 1002 items"),
        "and still says what it is: {:?}",
        h.texts()
    );
    assert!(h.app.doc.is_none(), "nothing is opened until asked");

    // The main window gets the file, as it would a big one opened from disk.
    press(&mut h, "Open in main window");
    let doc = h.app.doc.clone().expect("the merge opened");
    assert!(doc.is_lazy(), "kept as its file, not parsed");
    assert!(
        doc.byte_len > 2048,
        "the size of the result: {}",
        doc.byte_len
    );
    assert_eq!(doc.source.label(), "(merged from 2 files)");
    assert_eq!(h.app.source_input, "", "nothing to reload from the field");
}

#[test]
fn a_failed_merge_says_why_and_editing_the_files_clears_it() {
    let mut h = Harness::new();
    let a = temp_json("fail_a.json", "[1]");
    let b = temp_json("fail_b.json", r#"{"a": 1}"#);
    let ctx = h.ctx.clone();
    h.app
        .tools
        .add_files(&ctx, vec![a.to_path_buf(), b.to_path_buf()]);
    h.settle();

    let merge = center_of(&h, "Merge");
    h.click(merge.x, merge.y);
    wait_for(&mut h, "the error", |h| {
        h.texts()
            .iter()
            .any(|(t, _)| t.contains("cannot calculate"))
    });

    // Removing a file makes the old answer stale.
    let remove = h.pop_buttons("×");
    assert_eq!(remove.len(), 2, "one per file");
    h.click(remove[1].x, remove[1].y);
    assert!(is_drawn(&h, "Files (1)"), "{:?}", h.texts());
    assert!(
        !h.texts()
            .iter()
            .any(|(t, _)| t.contains("cannot calculate")),
        "the error is gone"
    );
}

#[test]
fn dropping_several_files_on_the_main_window_lists_them_for_merging() {
    let mut h = Harness::new();
    let files = [
        temp_json("drop_a.json", "[1]"),
        temp_json("drop_b.json", "[2]"),
    ];
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
        time: Some(h.time),
        dropped_files: files
            .iter()
            .map(|f| {
                std::sync::Arc::new(FakeDroppedFile(f.to_path_buf()))
                    as std::sync::Arc<dyn egui::DroppedFile + Send + Sync>
            })
            .collect(),
        ..Default::default()
    };
    let app = &mut h.app;
    let mut out = h.ctx.run_ui(input, |ui| app.show(ui));
    out.textures_delta.clear();
    h.settle();

    assert!(
        h.app.doc.is_none() && !h.app.loading,
        "none of them was opened"
    );
    assert!(h.app.tools.is_open());
    assert!(is_drawn(&h, "Files (2)"), "{:?}", h.texts());
}

/// The toolbar's own texts (and the icons pinned at its right end), left to
/// right, as `(text, left, right)`.
fn toolbar_row(h: &Harness) -> Vec<(String, f32, f32)> {
    let mut row: Vec<_> = text_rects(&h.shapes)
        .into_iter()
        .filter(|(t, r)| !t.is_empty() && r.center().y > 5.0 && r.center().y < 18.0)
        .map(|(t, r)| (t, r.min.x, r.max.x))
        .collect();
    row.sort_by(|a, b| a.1.total_cmp(&b.1));
    row
}

fn assert_toolbar_does_not_overlap(h: &Harness, state: &str) {
    for pair in toolbar_row(h).windows(2) {
        assert!(
            pair[0].2 <= pair[1].1,
            "{state}: {:?} runs into {:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn the_toolbar_has_room_for_its_labels_and_buttons_in_every_state() {
    // The new 🛠 button took room from the labels beside it ("(pasted JSON)",
    // "(merged from 2 files)", the size) once the "dock all" button joined it.
    let mut h = Harness::new();
    assert_toolbar_does_not_overlap(&h, "empty");

    load(&mut h, r#"{"a":1}"#);
    assert_toolbar_does_not_overlap(&h, "pasted");
    h.app.dock.pop_out(Pane::Query, None, None);
    h.settle();
    assert_toolbar_does_not_overlap(&h, "pasted, a pane out");
    assert!(
        h.pop_buttons("🗖").len() == 1,
        "the dock-all button is there"
    );

    let a = temp_json("bar_a.json", "[1]");
    let b = temp_json("bar_b.json", "[2]");
    let ctx = h.ctx.clone();
    h.app
        .tools
        .add_files(&ctx, vec![a.to_path_buf(), b.to_path_buf()]);
    h.settle();
    let merge = center_of(&h, "Merge");
    h.click(merge.x, merge.y);
    wait_for(&mut h, "the preview", |h| is_drawn(h, "[\n  1,\n  2\n]"));
    let open = center_of(&h, "Open in main window");
    h.click(open.x, open.y);
    assert!(is_drawn(&h, "(merged from 2 files)"), "{:?}", h.texts());
    assert_toolbar_does_not_overlap(&h, "merged, a pane out");
}

#[test]
fn a_cancelled_merge_is_not_shown_as_an_error() {
    let mut h = Harness::new();
    let a = temp_json("cancel_a.json", "[1]");
    let ctx = h.ctx.clone();
    h.app.tools.add_files(&ctx, vec![a.to_path_buf()]);
    h.settle();
    h.app.tools.merge_done(0, Err("cancelled".to_owned()));
    h.settle();
    assert!(is_drawn(&h, "Cancelled."), "{:?}", h.texts());
    assert!(!is_drawn(&h, "cancelled"));
}

// The other tools of the Tools window: Format, Diff, Patch and Validate.

use crate::tools::Tool;
use jsonquery_query::diff::Side;

fn open_tool(h: &mut Harness, tool: Tool) {
    let ctx = h.ctx.clone();
    h.app.tools.open_on(&ctx, tool);
    h.settle();
}

/// Click the button (or label) drawn as `text`.
fn press(h: &mut Harness, text: &str) {
    let at = center_of(h, text);
    h.click(at.x, at.y);
}

fn rect_of(h: &Harness, text: &str) -> egui::Rect {
    text_rects(&h.shapes)
        .into_iter()
        .find(|(t, _)| t == text)
        .unwrap_or_else(|| panic!("{text:?} is drawn: {:?}", h.texts()))
        .1
}

fn is_drawn_containing(h: &Harness, part: &str) -> bool {
    h.texts().iter().any(|(t, _)| t.contains(part))
}

#[test]
fn the_tools_window_lists_every_tool_and_each_opens_its_page() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Merge);
    // Every tool is a tab; the page of the one picked has its own main button,
    // and none of the others' is there.
    let pages = [
        ("Merge JSON", "Merge"),
        ("Format JSON", "Format"),
        ("Diff JSON", "Compare"),
        ("Patch JSON", "Apply"),
        ("Validate schema", "Validate"),
    ];
    for (tab, _) in pages {
        assert!(is_drawn(&h, tab), "{tab}: {:?}", h.texts());
    }
    for (tab, button) in pages {
        press(&mut h, tab);
        assert!(
            is_drawn(&h, button),
            "{tab} opens its page: {:?}",
            h.texts()
        );
        for (_, other) in pages.iter().filter(|(t, _)| *t != tab) {
            assert!(
                !is_drawn(&h, other),
                "{tab}: {other:?} belongs to another page"
            );
        }
    }
}

#[test]
fn formatting_json_typed_into_the_box_and_changing_an_option() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Format);
    press(&mut h, "Paste JSON here, or drop a file");
    h.type_text(r#"{"b":1,"a":[1,2]}"#);
    h.settle();
    press(&mut h, "Format");
    wait_for(&mut h, "the formatted text", |h| {
        is_drawn(h, "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2\n  ]\n}")
    });

    // An option changes what the answer would be, so the old one goes.
    press(&mut h, "Sort keys");
    assert!(
        is_drawn(&h, "The formatted document appears here"),
        "{:?}",
        h.texts()
    );
    press(&mut h, "Format");
    wait_for(&mut h, "the sorted text", |h| {
        is_drawn(h, "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": 1\n}")
    });
}

#[test]
fn text_that_is_not_json_is_reported_with_its_line() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Format);
    h.app.tools.fill(Tool::Format, "Input", "{\n  \"a\": \n}");
    h.settle();
    press(&mut h, "Format");
    wait_for(&mut h, "the error", |h| {
        h.texts()
            .iter()
            .any(|(t, _)| t.starts_with("Input: ") && t.contains("line 3"))
    });
}

#[test]
fn diff_lists_what_changed_and_gives_the_patch() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app
        .tools
        .fill(Tool::Diff, "Left", r#"{"a":1,"b":[1,2,3]}"#);
    h.app
        .tools
        .fill(Tool::Diff, "Right", r#"{"a":2,"b":[1,3],"c":true}"#);
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the changes", |h| is_drawn(h, "Changes (3)"));
    assert!(
        is_drawn(&h, "1 added · 1 removed · 1 changed"),
        "{:?}",
        h.texts()
    );

    press(&mut h, "Changes (3)");
    for text in ["/a", "/b/1", "/c", "Changed", "Removed", "Added"] {
        assert!(is_drawn(&h, text), "{text:?}: {:?}", h.texts());
    }

    press(&mut h, "Patch");
    assert!(
        is_drawn_containing(&h, r#"{"op": "replace", "path": "/a", "value": 2}"#),
        "{:?}",
        h.texts()
    );
}

/// The Diff page with two documents compared, on its side-by-side view.
fn compared_diff(left: &str, right: &str, count: &str) -> Harness {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill(Tool::Diff, "Left", left);
    h.app.tools.fill(Tool::Diff, "Right", right);
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the changes", |h| is_drawn(h, count));
    h
}

#[test]
fn comparing_opens_the_two_documents_side_by_side_with_the_differences_marked() {
    let h = compared_diff(
        r#"{"a":1,"b":[1,2,3]}"#,
        r#"{"a":2,"b":[1,3],"c":true}"#,
        "Changes (3)",
    );
    // Each document under its own heading, Left on the left.
    let (left, right) = (rect_of(&h, "Left"), rect_of(&h, "Right"));
    assert!(left.min.x < right.min.x, "{left:?} {right:?}");
    assert!((left.center().y - right.center().y).abs() < 2.0);

    // A changed line is one line in both columns.
    let (in_left, in_right) = (rect_of(&h, "  \"a\": 1,"), rect_of(&h, "  \"a\": 2,"));
    assert!(in_left.max.x < in_right.min.x, "{in_left:?} {in_right:?}");
    assert!((in_left.center().y - in_right.center().y).abs() < 1.0);
    assert!(in_left.min.y > left.max.y, "under the headings");

    // A line only one document has leaves the other column blank: the 2 that
    // was taken out is only in the left one, the true that was put in only in
    // the right one.
    let count = |text: &str| h.texts().iter().filter(|(t, _)| t == text).count();
    assert_eq!(count("    2,"), 1);
    assert_eq!(count("  \"c\": true"), 1);
    assert!(
        (rect_of(&h, "    2,").min.x - in_left.min.x).abs() < 1.0,
        "on the left"
    );
    assert!(
        (rect_of(&h, "  \"c\": true").min.x - in_right.min.x).abs() < 1.0,
        "on the right"
    );
}

#[test]
fn previous_and_next_difference_walk_the_differences_and_say_which() {
    let mut h = compared_diff(
        r#"{"a":1,"b":[1,2,3]}"#,
        r#"{"a":2,"b":[1,3],"c":true}"#,
        "Changes (3)",
    );
    // Beside the Compare button, not the main window's own arrows.
    let row = center_of(&h, "Compare").y;
    let button = |h: &Harness, glyph: &str| {
        *h.pop_buttons(glyph)
            .iter()
            .find(|p| (p.y - row).abs() < 3.0)
            .unwrap_or_else(|| panic!("{glyph} is in the command row: {:?}", h.texts()))
    };
    assert!(!is_drawn(&h, "Difference 1 of 3"), "none is picked yet");

    let next = button(&h, "⏷");
    h.click(next.x, next.y);
    assert!(is_drawn(&h, "Difference 1 of 3"), "{:?}", h.texts());
    let next = button(&h, "⏷");
    h.click(next.x, next.y);
    assert!(is_drawn(&h, "Difference 2 of 3"), "{:?}", h.texts());
    let previous = button(&h, "⏶");
    h.click(previous.x, previous.y);
    assert!(is_drawn(&h, "Difference 1 of 3"), "{:?}", h.texts());
}

#[test]
fn only_differences_folds_what_is_the_same_into_a_line_that_counts_it() {
    let left: Vec<u32> = (0..30).collect();
    let mut right = left.clone();
    right[25] = 1000;
    let mut h = compared_diff(
        &serde_json::to_string(&left).unwrap(),
        &serde_json::to_string(&right).unwrap(),
        "Changes (1)",
    );
    assert!(!is_drawn_containing(&h, "lines are the same"));
    assert!(is_drawn(&h, "  0,"), "every line is there");

    press(&mut h, "Differences only");
    // The brackets and the 25 numbers before the three kept above it, and the
    // closing bracket and the number after the three kept below it.
    assert!(is_drawn(&h, "… 23 lines are the same"), "{:?}", h.texts());
    assert!(is_drawn(&h, "… 2 lines are the same"), "{:?}", h.texts());
    assert!(!is_drawn(&h, "  0,"));
    assert!(is_drawn(&h, "  1000,"), "the difference is shown");

    press(&mut h, "Differences only");
    assert!(!is_drawn_containing(&h, "lines are the same"));
    assert!(is_drawn(&h, "  0,"));
}

#[test]
fn clicking_a_line_picks_it_and_the_menu_of_a_line_copies_its_path() {
    let mut h = compared_diff(
        r#"{"a/b": 1, "list": [1, 2]}"#,
        r#"{"a/b": 2, "list": [1, 2, 3]}"#,
        "Changes (2)",
    );
    // A click picks the line (and says so); it takes nothing from the clipboard.
    let copies = h.copied.len();
    press(&mut h, "  \"a/b\": 2,");
    assert_eq!(h.copied.len(), copies, "{:?}", h.copied);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    assert!(is_drawn(&h, "Difference 1 of 2"), "{:?}", h.texts());

    // Its menu has the path.
    let line = center_of(&h, "  \"a/b\": 2,");
    h.right_click(line.x, line.y);
    press(&mut h, "Copy path");
    assert_eq!(h.copied.last().map(String::as_str), Some("/a~1b"));
    assert!(
        is_drawn_containing(&h, "Copied the path to /a~1b"),
        "{:?}",
        h.texts()
    );

    // A line that is the same is not a difference: clicking it lets go of what was
    // picked, and it has no menu to move or copy.
    press(&mut h, "  \"list\": [");
    assert!(!is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    let same = center_of(&h, "  \"list\": [");
    h.right_click(same.x, same.y);
    assert!(!is_drawn(&h, "Copy path"), "{:?}", h.texts());
}

/// The command row's button drawn as `glyph` (⏷, ⏴…): the one beside Compare,
/// not one of the main window's.
fn command_button(h: &Harness, glyph: &str) -> egui::Pos2 {
    let row = center_of(h, "Compare").y;
    *h.pop_buttons(glyph)
        .iter()
        .find(|p| (p.y - row).abs() < 3.0)
        .unwrap_or_else(|| panic!("{glyph} is in the command row: {:?}", h.texts()))
}

fn as_json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text:?}"))
}

#[test]
fn the_move_buttons_wait_for_a_difference_to_be_picked_and_leave_the_boxes_alone_until_then() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill(Tool::Diff, "Left", r#"{"a":1,"b":2}"#);
    h.app.tools.fill(Tool::Diff, "Right", r#"{"a":1,"b":3}"#);
    h.settle();
    // Not on the page of the two boxes, nor on the list of changes…
    assert!(!is_drawn(&h, "⏵"));
    press(&mut h, "Compare");
    wait_for(&mut h, "the changes", |h| is_drawn(h, "Changes (1)"));
    assert!(is_drawn(&h, "⏴") && is_drawn(&h, "⏵"), "{:?}", h.texts());
    press(&mut h, "Changes (1)");
    assert!(!is_drawn(&h, "⏵"), "only the side-by-side view moves");
    press(&mut h, "Side by side");

    // …and with none picked, pressing one does nothing.
    let right = command_button(&h, "⏵");
    h.click(right.x, right.y);
    h.settle();
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Right"),
        r#"{"a":1,"b":3}"#
    );
    assert!(!is_drawn_containing(&h, "Moved the difference"));
}

#[test]
fn moving_a_difference_to_the_right_gives_the_right_document_what_the_left_has() {
    // b changed, and, with a line that is the same between, c put in: two
    // differences.
    let mut h = compared_diff(
        r#"{"b":2,"s":0,"a":1}"#,
        r#"{"b":3,"s":0,"a":1,"c":true}"#,
        "Changes (2)",
    );
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    assert!(is_drawn(&h, "Difference 1 of 2"), "{:?}", h.texts());

    let right = command_button(&h, "⏵");
    h.click(right.x, right.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "1 added"));
    // The right box has b as the left has it, and still has c; the left box is
    // as it was, the way it was typed.
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"b": 2, "s": 0, "a": 1, "c": true})
    );
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Left"),
        r#"{"b":2,"s":0,"a":1}"#
    );
    assert!(
        is_drawn_containing(&h, "Moved the difference to the right"),
        "{:?}",
        h.texts()
    );
    // The new document is what is shown, with c the one difference left: the view does
    // not go on to it by itself.
    assert!(is_drawn(&h, "Changes (1)"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Difference "), "{:?}", h.texts());
}

#[test]
fn moving_a_difference_to_the_left_with_the_keyboard_changes_the_left_box() {
    let mut h = compared_diff(
        r#"{"a":1,"b":2}"#,
        r#"{"a":1,"b":3,"c":true}"#,
        "Changes (2)",
    );
    // Nothing is picked: the keys do nothing.
    h.key(egui::Key::ArrowLeft, egui::Modifiers::ALT);
    h.settle();
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);

    h.key(egui::Key::ArrowDown, egui::Modifiers::ALT);
    assert!(is_drawn(&h, "Difference 1 of 1"), "{:?}", h.texts());
    h.key(egui::Key::ArrowLeft, egui::Modifiers::ALT);
    wait_for(&mut h, "the new comparison", |h| {
        is_drawn(h, "Moved the difference to the left")
    });
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 3, "c": true}),
        "Left took what Right has: b changed and c put in"
    );
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Right"),
        r#"{"a":1,"b":3,"c":true}"#
    );
    assert!(is_drawn(&h, "same"), "{:?}", h.texts());
}

#[test]
fn moving_the_last_difference_makes_the_documents_the_same() {
    let mut h = compared_diff("[1, 2]", "[1, 3]", "Changes (1)");
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    let left = command_button(&h, "⏴");
    h.click(left.x, left.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        as_json("[1, 3]")
    );
    // Nothing is left to pick, so the buttons are not pressed on with no effect.
    assert!(!is_drawn_containing(&h, "Difference "), "{:?}", h.texts());
}

#[test]
fn after_a_move_nothing_is_picked_and_the_view_stays_where_it_was() {
    let mut h = compared_diff(
        r#"{"a":1,"s":0,"b":2,"t":0,"c":3}"#,
        r#"{"a":9,"s":0,"b":8,"t":0,"c":7}"#,
        "Changes (3)",
    );
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    assert!(is_drawn(&h, "Difference 2 of 3"), "{:?}", h.texts());
    // Moving the second leaves two, and the view does not go on to the third by itself:
    // nothing is picked, so the buttons have nothing to move until something is.
    let right = command_button(&h, "⏵");
    h.click(right.x, right.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (2)"));
    assert!(!is_drawn_containing(&h, "Difference "), "{:?}", h.texts());
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"a": 9, "s": 0, "b": 2, "t": 0, "c": 7})
    );
    // The next press of Next is the first difference from the top of the view.
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    assert!(is_drawn(&h, "Difference 1 of 2"), "{:?}", h.texts());
}

#[test]
fn after_a_move_the_text_is_where_it_was_scrolled_to_and_not_at_the_next_difference() {
    // Two differences far apart, in a document of 160 lines.
    let numbers = |first: i32, second: i32| {
        let items: Vec<String> = (0..160)
            .map(|n| match n {
                100 => first.to_string(),
                140 => second.to_string(),
                _ => format!("{}", 1000 + n),
            })
            .collect();
        format!("[{}]", items.join(","))
    };
    let mut h = compared_diff(&numbers(1, 3), &numbers(2, 4), "Changes (2)");
    // Down to the first of them.
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    assert!(is_drawn(&h, "Difference 1 of 2"), "{:?}", h.texts());
    let above = |h: &Harness| {
        h.texts()
            .into_iter()
            .find(|(t, _)| t == "  1099,")
            .map(|(_, p)| p.y.round())
    };
    let before = above(&h).expect("the line above the difference is in view");
    // Its arrow, into the right document.
    let arrow = h.app.tools.diff_arrow(0, Side::Right).unwrap();
    h.click(arrow.x, arrow.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (1)"));
    h.settle();
    // The same lines are in view, where they were, and the second difference (far below)
    // is not.
    assert_eq!(above(&h), Some(before), "{:?}", h.texts());
    assert!(!is_drawn(&h, "  4,"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Difference "), "{:?}", h.texts());
}

#[test]
fn a_difference_has_a_menu_that_moves_it_to_either_side() {
    let mut h = compared_diff(
        r#"{"a":1,"s":0,"b":2}"#,
        r#"{"a":9,"s":0,"b":8}"#,
        "Changes (2)",
    );
    // Nothing is picked; the menu of the line of the second difference moves
    // that one.
    let line = center_of(&h, "  \"b\": 8");
    h.right_click(line.x, line.y);
    assert!(is_drawn(&h, "Move to the left"), "{:?}", h.texts());
    assert!(is_drawn(&h, "Move to the right"), "{:?}", h.texts());
    press(&mut h, "Move to the left");
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (1)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "s": 0, "b": 8})
    );
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Right"),
        r#"{"a":9,"s":0,"b":8}"#
    );

    // A line that is the same has no such menu.
    let same = center_of(&h, "  \"s\": 0,");
    h.right_click(same.x, same.y);
    assert!(!is_drawn(&h, "Move to the right"), "{:?}", h.texts());
}

#[test]
fn a_box_that_held_a_file_holds_the_moved_text_and_says_it_is_changed() {
    let file = temp_json("moved-left.json", r#"{"a":1,"b":2}"#);
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill_file(Tool::Diff, "Left", &file);
    h.app.tools.fill(Tool::Diff, "Right", r#"{"a":1,"b":3}"#);
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the changes", |h| is_drawn(h, "Changes (1)"));
    assert!(
        is_drawn_containing(&h, "moved-left.json"),
        "{:?}",
        h.texts()
    );

    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    let left = command_button(&h, "⏴");
    h.click(left.x, left.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert!(
        is_drawn_containing(&h, "moved-left.json (changed)"),
        "the left document is called by its file, and said to be changed: {:?}",
        h.texts()
    );
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 3})
    );
    // The file itself is not touched.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), r#"{"a":1,"b":2}"#);
}

#[test]
fn a_move_that_makes_a_document_too_big_for_its_box_leaves_it_held_unseen() {
    // Two files of a megabyte and a half, which a text box can not edit, so they
    // are not shown but read when the tool runs. They differ in one string.
    let strings = |changed: &str| {
        let mut items: Vec<String> = (0..5_000)
            .map(|n| format!("\"{n:04}{}\"", "x".repeat(300)))
            .collect();
        items[2_500] = format!("\"{changed}\"");
        format!("[{}]", items.join(","))
    };
    let (left, right) = (strings("left"), strings("right"));
    assert!(left.len() as u64 > 1024 * 1024);
    let (left_file, right_file) = (
        temp_json("held-left.json", &left),
        temp_json("held-right.json", &right),
    );
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill_file(Tool::Diff, "Left", &left_file);
    h.app.tools.fill_file(Tool::Diff, "Right", &right_file);
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the changes", |h| is_drawn(h, "Changes (1)"));

    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    let left_button = command_button(&h, "⏴");
    h.click(left_button.x, left_button.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));

    // The left box holds the new text, all of it, though a text box would not.
    let held = h.app.tools.box_text(Tool::Diff, "Left");
    assert!(held.len() as u64 > 1024 * 1024, "{} bytes", held.len());
    assert_eq!(as_json(&held), as_json(&right));
    // It is said to be changed, and the file is as it was.
    assert!(
        is_drawn_containing(&h, "held-left.json (changed)"),
        "{:?}",
        h.texts()
    );
    assert_eq!(std::fs::read_to_string(&left_file).unwrap(), left);
    // Comparing again uses it as it is.
    press(&mut h, "Compare");
    wait_for(&mut h, "the comparison", |h| {
        is_drawn_containing(h, "Compared in")
    });
    assert!(is_drawn(&h, "same"), "{:?}", h.texts());
}

// Comparing by itself, the arrows between the documents, picking lines, saving.

/// Whether the status bar says that lines are picked ("1 line picked", "3 lines picked"; not
/// the notice that picked lines were moved).
fn lines_are_picked(h: &Harness) -> bool {
    is_drawn_containing(h, "line picked") || is_drawn_containing(h, "lines picked")
}

/// The Diff page with both boxes filled and nothing compared.
fn diff_with(left: &str, right: &str) -> Harness {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill(Tool::Diff, "Left", left);
    h.app.tools.fill(Tool::Diff, "Right", right);
    h.settle();
    h
}

#[test]
fn a_view_of_the_comparison_compares_the_documents_by_itself() {
    for (view, shown) in [
        ("Side by side", "  \"b\": 3"),
        ("Changes", "Changed"),
        ("Patch", r#"{"op": "replace", "path": "/b", "value": 3}"#),
    ] {
        let mut h = diff_with(r#"{"a":1,"b":2}"#, r#"{"a":1,"b":3}"#);
        // Compare is not pressed: the tab asks for the comparison.
        press(&mut h, view);
        wait_for(&mut h, "the comparison", |h| is_drawn(h, "Changes (1)"));
        assert!(
            is_drawn_containing(&h, shown),
            "{view} shows its answer: {:?}",
            h.texts()
        );
        assert!(is_drawn(&h, "1 changed"), "{view}: {:?}", h.texts());
    }
}

#[test]
fn a_view_does_not_compare_until_both_documents_are_there_and_does_once_they_are() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill(Tool::Diff, "Left", "[1]");
    h.settle();
    press(&mut h, "Side by side");
    h.pause(0.5);
    let hint = "Put a document in both boxes to see what differs";
    assert!(is_drawn(&h, hint), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Working"), "{:?}", h.texts());

    press(&mut h, "Documents");
    h.app.tools.fill(Tool::Diff, "Right", "[2]");
    h.settle();
    press(&mut h, "Side by side");
    wait_for(&mut h, "the comparison", |h| is_drawn(h, "Changes (1)"));
    assert!(!is_drawn(&h, hint));
}

#[test]
fn a_comparison_that_was_made_is_not_made_again_by_going_to_a_view() {
    let mut h = compared_diff("[1]", "[2]", "Changes (1)");
    press(&mut h, "Patch");
    press(&mut h, "Side by side");
    h.pause(0.3);
    assert!(
        !is_drawn_containing(&h, "Working"),
        "no new comparison: {:?}",
        h.texts()
    );
    assert!(is_drawn(&h, "Changes (1)"));
}

#[test]
fn an_arrow_between_the_documents_moves_the_difference_it_is_beside() {
    // Two differences, with a line that is the same between them.
    let mut h = compared_diff(
        r#"{"a":1,"s":0,"b":2}"#,
        r#"{"a":9,"s":0,"b":8}"#,
        "Changes (2)",
    );
    for block in 0..2 {
        for into in [Side::Left, Side::Right] {
            assert!(
                h.app.tools.diff_arrow(block, into).is_some(),
                "difference {block} has an arrow to {into:?}"
            );
        }
    }
    // The one at the top points to the left document, the one under it to the right.
    let (to_left, to_right) = (
        h.app.tools.diff_arrow(1, Side::Left).unwrap(),
        h.app.tools.diff_arrow(1, Side::Right).unwrap(),
    );
    assert!(
        (to_left.x - to_right.x).abs() < 0.5 && (to_right.y - to_left.y - 17.0).abs() < 0.5,
        "one over the other, a row apart: {to_left:?} {to_right:?}"
    );
    // Both lie between the two documents' text.
    let (left_text, right_text) = (rect_of(&h, "  \"b\": 2"), rect_of(&h, "  \"b\": 8"));
    assert!(left_text.max.x < to_left.x && to_right.x < right_text.min.x);

    // The second difference into the right document: Right takes what Left has.
    h.click(to_right.x, to_right.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (1)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"a": 9, "s": 0, "b": 2})
    );
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Left"),
        r#"{"a":1,"s":0,"b":2}"#
    );
    assert!(
        is_drawn(&h, "Moved the difference to the right"),
        "{:?}",
        h.texts()
    );

    // The one that is left, into the left document.
    let arrow = h.app.tools.diff_arrow(0, Side::Left).unwrap();
    h.click(arrow.x, arrow.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 9, "s": 0, "b": 2})
    );
    assert!(
        is_drawn(&h, "Moved the difference to the left"),
        "{:?}",
        h.texts()
    );
    // Nothing is left to have arrows.
    assert!(h.app.tools.diff_arrow(0, Side::Left).is_none());
}

/// Three changes in a row, which are one difference of the view, and a fourth.
fn four_in_a_row() -> Harness {
    compared_diff(
        r#"{"a":1,"b":2,"c":3,"d":4}"#,
        r#"{"a":9,"b":8,"c":7,"d":6}"#,
        "Changes (4)",
    )
}

#[test]
fn a_line_that_is_picked_is_moved_by_its_arrow_and_the_rest_of_the_difference_stays() {
    let mut h = four_in_a_row();
    // Nothing is picked: the arrow is for the whole difference.
    let right = h.app.tools.diff_arrow(0, Side::Right).unwrap();
    // Pick b, and say so.
    press(&mut h, "  \"b\": 2,");
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    // The arrow moves that line only.
    h.click(right.x, right.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (3)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"a": 9, "b": 2, "c": 7, "d": 6})
    );
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Left"),
        r#"{"a":1,"b":2,"c":3,"d":4}"#
    );
    assert!(
        is_drawn(&h, "Moved the picked lines to the right"),
        "{:?}",
        h.texts()
    );
    // Nothing is picked in what is shown now.
    assert!(!lines_are_picked(&h), "{:?}", h.texts());
}

#[test]
fn an_arrow_with_nothing_picked_moves_the_whole_difference() {
    let mut h = four_in_a_row();
    let left = h.app.tools.diff_arrow(0, Side::Left).unwrap();
    h.click(left.x, left.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 9, "b": 8, "c": 7, "d": 6})
    );
    assert!(is_drawn(&h, "Moved the difference to the left"));
}

#[test]
fn lines_are_picked_with_ctrl_and_shift_and_by_dragging_and_let_go_of_with_escape() {
    let mut h = four_in_a_row();
    let at = |h: &Harness, name: &str, value: u32| center_of(h, &format!("  \"{name}\": {value},"));
    let a = at(&h, "a", 1);
    let b = at(&h, "b", 2);
    let c = at(&h, "c", 3);
    let command = egui::Modifiers::COMMAND;
    let shift = egui::Modifiers::SHIFT;

    h.click(a.x, a.y);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    // A plain click on another line picks that one instead.
    h.click(b.x, b.y);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    h.click(a.x, a.y);
    // Ctrl adds a line, and takes one away that is picked.
    h.click_with(c.x, c.y, command);
    assert!(is_drawn(&h, "2 lines picked"), "{:?}", h.texts());
    h.click_with(a.x, a.y, command);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    // Shift picks from the line that was clicked last, to this one.
    let d = center_of(&h, "  \"d\": 4");
    h.click_with(d.x, d.y, shift);
    assert!(is_drawn(&h, "4 lines picked"), "{:?}", h.texts());
    // Escape lets go of them all.
    h.key(egui::Key::Escape, egui::Modifiers::NONE);
    h.settle();
    assert!(!lines_are_picked(&h), "{:?}", h.texts());

    // Dragging over lines picks them: b to d.
    let d = center_of(&h, "  \"d\": 4");
    h.drag_y(b.x, b.y, d.y);
    assert!(is_drawn(&h, "3 lines picked"), "{:?}", h.texts());
    // A click on a line that is the same lets go (there is none here: the line
    // above them all is the opening bracket).
    let bracket = center_of(&h, "{");
    h.click(bracket.x, bracket.y);
    assert!(!lines_are_picked(&h), "{:?}", h.texts());
}

#[test]
fn the_buttons_and_the_keys_of_the_command_row_move_the_lines_that_are_picked() {
    let mut h = four_in_a_row();
    let b = center_of(&h, "  \"b\": 2,");
    let c = center_of(&h, "  \"c\": 3,");
    h.click(b.x, b.y);
    h.click_with(c.x, c.y, egui::Modifiers::COMMAND);
    assert!(is_drawn(&h, "2 lines picked"), "{:?}", h.texts());

    // ⏴: Left takes b and c from Right.
    let left = command_button(&h, "⏴");
    h.click(left.x, left.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (2)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 8, "c": 7, "d": 4})
    );
    assert!(
        is_drawn(&h, "Moved the picked lines to the left"),
        "{:?}",
        h.texts()
    );

    // And the keys do the same for what is picked now.
    let a = center_of(&h, "  \"a\": 1,");
    h.click(a.x, a.y);
    h.key(egui::Key::ArrowRight, egui::Modifiers::ALT);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (1)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"a": 1, "b": 8, "c": 7, "d": 6})
    );
}

#[test]
fn a_picked_line_beats_a_difference_that_was_stepped_to_for_the_buttons() {
    // The buttons act on the lines that are picked, and, with none, on the difference that
    // Next went to.
    let mut h = compared_diff(
        r#"{"a":1,"s":0,"b":2,"t":0,"c":3}"#,
        r#"{"a":9,"s":0,"b":8,"t":0,"c":7}"#,
        "Changes (3)",
    );
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    assert!(is_drawn(&h, "Difference 1 of 3"));
    // b is picked, and is what ⏵ moves, and not a, which Next went to.
    let b = center_of(&h, "  \"b\": 2,");
    h.click(b.x, b.y);
    assert!(is_drawn(&h, "Difference 2 of 3"), "{:?}", h.texts());
    let right = command_button(&h, "⏵");
    h.click(right.x, right.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (2)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"a": 9, "s": 0, "b": 2, "t": 0, "c": 7})
    );
    // Stepping lets go of what was picked: what the buttons move is the difference that
    // was stepped to.
    let a = center_of(&h, "  \"a\": 1,");
    h.click(a.x, a.y);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    assert!(!lines_are_picked(&h), "{:?}", h.texts());
    assert!(is_drawn(&h, "Difference 2 of 2"), "{:?}", h.texts());
}

#[test]
fn a_line_of_a_value_that_has_several_is_picked_with_all_of_them_and_moved_whole() {
    // x is only in Right, and has four lines: picking one of them picks the value, as half
    // an object is no JSON.
    let mut h = compared_diff(r#"{"a":1}"#, r#"{"a":1,"x":{"p":1,"q":2}}"#, "Changes (1)");
    press(&mut h, "    \"p\": 1,");
    assert!(is_drawn(&h, "4 lines picked"), "{:?}", h.texts());
    // Into the left document: Left takes the whole of x.
    let arrow = h.app.tools.diff_arrow(0, Side::Left).unwrap();
    h.click(arrow.x, arrow.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "x": {"p": 1, "q": 2}})
    );
}

#[test]
fn the_menu_of_a_picked_line_moves_all_that_is_picked() {
    let mut h = four_in_a_row();
    let a = center_of(&h, "  \"a\": 1,");
    let d = center_of(&h, "  \"d\": 4");
    h.click(a.x, a.y);
    h.click_with(d.x, d.y, egui::Modifiers::COMMAND);
    // The menu of a line that is picked is for all of them.
    h.right_click(d.x, d.y);
    assert!(is_drawn(&h, "2 lines picked"), "{:?}", h.texts());
    press(&mut h, "Move to the right");
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (2)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"a": 1, "b": 8, "c": 7, "d": 4})
    );

    // The menu of a line that is not picked picks it, and is for it alone.
    let b = center_of(&h, "  \"b\": 2,");
    h.right_click(b.x, b.y);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
}

#[test]
fn the_menu_of_a_picked_line_moves_the_lines_picked_in_every_difference() {
    let mut h = compared_diff(
        r#"{"a":1,"s":0,"b":2}"#,
        r#"{"a":9,"s":0,"b":8}"#,
        "Changes (2)",
    );
    let a = center_of(&h, "  \"a\": 1,");
    let b = center_of(&h, "  \"b\": 2");
    h.click(a.x, a.y);
    h.click_with(b.x, b.y, egui::Modifiers::COMMAND);
    assert!(is_drawn(&h, "2 lines picked"), "{:?}", h.texts());
    h.right_click(b.x, b.y);
    press(&mut h, "Move to the right");
    // Both were moved, though they are two differences of the view.
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"a": 1, "s": 0, "b": 2})
    );
}

#[test]
fn each_document_has_a_save_that_writes_what_a_move_made_where_the_file_was() {
    // Left is a file, Right is typed.
    let file = temp_json("saved-left.json", r#"{"a":1,"b":2}"#);
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill_file(Tool::Diff, "Left", &file);
    h.app.tools.fill(Tool::Diff, "Right", r#"{"a":1,"b":3}"#);
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the changes", |h| is_drawn(h, "Changes (1)"));
    // A Save… over each column, at the right end of its half.
    let saves = h.pop_buttons("Save…");
    let (left_save, right_save) = (
        *saves.iter().find(|p| p.x < 450.0).expect("Left's Save…"),
        *saves.iter().find(|p| p.x >= 450.0).expect("Right's Save…"),
    );

    // The file in a box was read, not changed: nothing to be said of it. Move the
    // difference into Left.
    let arrow = h.app.tools.diff_arrow(0, Side::Left).unwrap();
    h.click(arrow.x, arrow.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert!(
        is_drawn_containing(&h, "saved-left.json (changed)"),
        "{:?}",
        h.texts()
    );

    // Save… asks where, proposing the file's folder and name (the dialog is
    // answered by the test), and writes what is in the box.
    let target = file.parent().unwrap().join("moved.json");
    h.app.tools.answer_save_dialogs_with(Some(target.clone()));
    h.click(left_save.x, left_save.y);
    wait_for(&mut h, "the save", |h| is_drawn_containing(h, "Saved to"));
    assert_eq!(
        as_json(&std::fs::read_to_string(&target).unwrap()),
        serde_json::json!({"a": 1, "b": 3})
    );
    // The original file is as it was; the document is the file it was saved as, and is
    // no longer said to be changed.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), r#"{"a":1,"b":2}"#);
    assert!(is_drawn_containing(&h, "moved.json · "), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "(changed)"), "{:?}", h.texts());

    // A dialog that is cancelled writes nothing.
    h.app.tools.answer_save_dialogs_with(None);
    let saved_at = std::fs::metadata(&target).unwrap().modified().unwrap();
    h.click(right_save.x, right_save.y);
    h.pause(0.3);
    assert_eq!(
        std::fs::metadata(&target).unwrap().modified().unwrap(),
        saved_at
    );
}

#[test]
fn a_file_that_was_not_read_into_its_box_has_nothing_to_save_from_here() {
    // Too big for a text box (over a megabyte), so that it is read when the tool runs, and
    // short enough to be laid out side by side.
    let strings: Vec<String> = (0..4_000)
        .map(|n| format!("\"{n:04}{}\"", "x".repeat(300)))
        .collect();
    let big = format!("[{}]", strings.join(","));
    assert!(big.len() as u64 > 1024 * 1024);
    let file = temp_json("big-left.json", &big);
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill_file(Tool::Diff, "Left", &file);
    h.app.tools.fill(Tool::Diff, "Right", "[1]");
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the comparison", |h| {
        is_drawn_containing(h, "Compared in")
    });
    // Its Save… is dim: pressing it asks for nothing.
    h.app
        .tools
        .answer_save_dialogs_with(Some(file.parent().unwrap().join("x.json")));
    let saves = h.pop_buttons("Save…");
    let left_save = *saves.iter().find(|p| p.x < 450.0).expect("Left's Save…");
    h.click(left_save.x, left_save.y);
    h.pause(0.3);
    assert!(!file.parent().unwrap().join("x.json").exists());
}

// Typing over a line of either column.

/// Double-click the text drawn as `line`, take what is in the line all, and type `typed` over it.
fn type_over(h: &mut Harness, line: &str, typed: &str) {
    let at = center_of(h, line);
    h.double_click(at.x, at.y);
    h.key(egui::Key::A, egui::Modifiers::COMMAND);
    h.type_text(typed);
}

fn enter(h: &mut Harness) {
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
}

/// The Diff page's side-by-side view of `left` and `right`, which differ in `b`.
fn two_with_b() -> Harness {
    compared_diff(r#"{"a":1,"b":2}"#, r#"{"a":1,"b":3}"#, "Changes (1)")
}

#[test]
fn a_double_click_puts_a_caret_in_a_line_and_what_is_typed_takes_its_place_on_enter() {
    let mut h = two_with_b();
    type_over(&mut h, "  \"b\": 2", "\"b\": 3");
    assert!(
        is_drawn(
            &h,
            "Editing line 3 of Left: Enter puts it in, Esc puts it back"
        ),
        "{:?}",
        h.texts()
    );
    // Nothing is done until Enter.
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 3})
    );
    assert!(
        is_drawn(&h, "Changed line 3 of the left document"),
        "{:?}",
        h.texts()
    );
    assert!(is_drawn_containing(&h, "(changed)"), "{:?}", h.texts());
    // The other document is as it was, and nothing has a caret.
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Right"),
        r#"{"a":1,"b":3}"#
    );
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
}

#[test]
fn the_right_column_is_typed_over_as_well() {
    let mut h = two_with_b();
    type_over(&mut h, "  \"b\": 3", "\"b\": 2");
    assert!(
        is_drawn_containing(&h, "Editing line 3 of Right"),
        "{:?}",
        h.texts()
    );
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"a": 1, "b": 2})
    );
    assert!(
        is_drawn(&h, "Changed line 3 of the right document"),
        "{:?}",
        h.texts()
    );
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);
}

#[test]
fn a_line_that_is_the_same_on_both_sides_is_typed_over_too() {
    let mut h = two_with_b();
    // `"a": 1,` is drawn on both sides: the first is the left one.
    type_over(&mut h, "  \"a\": 1,", "\"a\": 10,");
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "2 changed"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 10, "b": 2})
    );
}

#[test]
fn escape_puts_the_line_back_and_nothing_is_changed() {
    let mut h = two_with_b();
    type_over(&mut h, "  \"b\": 2", "\"b\": 99");
    h.key(egui::Key::Escape, egui::Modifiers::NONE);
    h.settle();
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
    assert!(
        is_drawn(&h, "  \"b\": 2"),
        "the line is as it was: {:?}",
        h.texts()
    );
    h.pause(0.3);
    assert!(!is_drawn_containing(&h, "Working"), "{:?}", h.texts());
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);
    assert!(!is_drawn_containing(&h, "(changed)"), "{:?}", h.texts());
}

#[test]
fn a_line_that_was_not_typed_in_makes_no_change_whatever_leaves_it() {
    let mut h = two_with_b();
    let at = center_of(&h, "  \"b\": 2");
    h.double_click(at.x, at.y);
    assert!(
        is_drawn_containing(&h, "Editing line 3 of Left"),
        "{:?}",
        h.texts()
    );
    enter(&mut h);
    h.pause(0.3);
    assert!(!is_drawn_containing(&h, "Working"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "(changed)"), "{:?}", h.texts());
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);
}

#[test]
fn what_is_not_json_where_it_is_stays_to_be_put_right_and_changes_nothing() {
    let mut h = two_with_b();
    type_over(&mut h, "  \"b\": 2", "\"b\": ");
    enter(&mut h);
    h.pause(0.3);
    // It says what is wrong, in the status bar, and the line still has the caret.
    assert!(
        is_drawn_containing(&h, "Not valid JSON: "),
        "{:?}",
        h.texts()
    );
    assert!(!is_drawn_containing(&h, "Working"), "{:?}", h.texts());
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);
    // Put right, it is taken.
    h.type_text("7");
    h.frame();
    assert!(
        !is_drawn_containing(&h, "Not valid JSON"),
        "{:?}",
        h.texts()
    );
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (1)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 7})
    );
}

#[test]
fn a_click_elsewhere_takes_what_was_typed_and_is_not_a_click_on_what_it_lands_on() {
    let mut h = two_with_b();
    type_over(&mut h, "  \"b\": 2", "\"b\": 5");
    // A click on the other side's line: the change is made, and the line that was clicked is
    // not picked (the view is the new one by then).
    let other = center_of(&h, "  \"b\": 3");
    h.click(other.x, other.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (1)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 5})
    );
    assert!(!lines_are_picked(&h), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
}

#[test]
fn a_line_can_be_taken_out_and_several_put_in_where_one_was() {
    let mut h = two_with_b();
    // Taken out: nothing is typed.
    let at = center_of(&h, "  \"a\": 1,");
    h.double_click(at.x, at.y);
    h.key(egui::Key::A, egui::Modifiers::COMMAND);
    h.key(egui::Key::Backspace, egui::Modifiers::NONE);
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| {
        is_drawn(h, "Took line 2 out of the left document")
    });
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"b": 2})
    );

    // Two in the place of one, with a comma between them.
    type_over(&mut h, "  \"b\": 2", "\"b\": 3, \"c\": 4");
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| {
        is_drawn(h, "Changed line 2 of the left document")
    });
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"b": 3, "c": 4})
    );
}

#[test]
fn the_name_of_a_line_that_opens_an_object_is_changed_and_what_is_in_it_stays() {
    let mut h = compared_diff(
        r#"{"home":{"city":"London"}}"#,
        r#"{"home":{"city":"Paris"}}"#,
        "Changes (1)",
    );
    type_over(&mut h, "  \"home\": {", "\"address\": {");
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (2)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"address": {"city": "London"}})
    );
}

#[test]
fn a_name_that_another_member_has_is_not_taken() {
    let mut h = compared_diff(r#"{"a":1,"b":2}"#, r#"{"a":1,"b":3}"#, "Changes (1)");
    type_over(&mut h, "  \"b\": 2", "\"a\": 2");
    enter(&mut h);
    h.pause(0.3);
    assert!(
        is_drawn(&h, "There already is a member called \"a\" here"),
        "{:?}",
        h.texts()
    );
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);
}

#[test]
fn the_members_of_the_right_document_keep_their_order_when_a_line_of_it_is_typed_over() {
    // The right column shows the members in the left one's order; the document is not
    // put in it.
    let mut h = compared_diff(r#"{"a":1,"b":2}"#, r#"{"b":3,"a":1}"#, "Changes (1)");
    type_over(&mut h, "  \"b\": 3", "\"b\": 2");
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    let text = h.app.tools.box_text(Tool::Diff, "Right");
    assert!(
        text.find("\"b\"").unwrap() < text.find("\"a\"").unwrap(),
        "b stays before a: {text}"
    );
}

#[test]
fn lines_that_close_something_or_are_too_long_have_no_caret_and_the_long_one_says_so() {
    let long = "x".repeat(500);
    let mut h = compared_diff(
        &format!(r#"{{"s":"{long}","n":1}}"#),
        &format!(r#"{{"s":"{long}","n":2}}"#),
        "Changes (1)",
    );
    // The bracket that closes the document, and the one that opens it.
    let closing = center_of(&h, "}");
    h.double_click(closing.x, closing.y);
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "too long"), "{:?}", h.texts());
    // The line that was cut short: what is shown is not all of it.
    let cut = h
        .texts()
        .into_iter()
        .find(|(t, _)| t.starts_with("  \"s\": \"xxx") && t.ends_with('…'))
        .map(|(_, p)| p)
        .expect("the long line is shown cut");
    h.double_click(cut.x + 40.0, cut.y + 6.0);
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
    assert!(
        is_drawn_containing(&h, "too long to edit here"),
        "{:?}",
        h.texts()
    );
    // A click on another line takes the message away.
    let closing = center_of(&h, "}");
    h.click(closing.x, closing.y);
    assert!(!is_drawn_containing(&h, "too long"), "{:?}", h.texts());
}

#[test]
fn escape_after_a_double_click_lets_go_of_the_pick_as_it_did_and_a_pick_with_ctrl_has_no_caret() {
    let mut h = two_with_b();
    let b = center_of(&h, "  \"b\": 2");
    h.double_click(b.x, b.y);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    assert!(
        is_drawn_containing(&h, "Editing line 3 of Left"),
        "{:?}",
        h.texts()
    );
    h.key(egui::Key::Escape, egui::Modifiers::NONE);
    h.settle();
    assert!(!lines_are_picked(&h), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
    // Ctrl picks, and puts no caret.
    h.click_with(b.x, b.y, egui::Modifiers::COMMAND);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
}

#[test]
fn a_single_click_only_picks_so_that_more_lines_can_be_picked_by_clicks_and_by_a_drag() {
    let mut h = compared_diff(
        r#"{"a":1,"s":0,"b":2,"t":0,"c":3}"#,
        r#"{"a":9,"s":0,"b":8,"t":0,"c":7}"#,
        "Changes (3)",
    );
    let a = center_of(&h, "  \"a\": 1,");
    let b = center_of(&h, "  \"b\": 2,");
    let c = center_of(&h, "  \"c\": 3");
    h.click(a.x, a.y);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
    // Ctrl adds the others, one by one.
    h.click_with(b.x, b.y, egui::Modifiers::COMMAND);
    h.click_with(c.x, c.y, egui::Modifiers::COMMAND);
    assert!(is_drawn(&h, "3 lines picked"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
    // A drag over lines picks them, in the place of those picked before, after a click too.
    h.click(a.x, a.y);
    h.drag_y(b.x, b.y, c.y);
    assert!(is_drawn(&h, "2 lines picked"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
    // ...and all the lines picked are moved, in one press.
    let right = command_button(&h, "⏵");
    h.click(right.x, right.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (1)"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!({"a": 9, "s": 0, "b": 2, "t": 0, "c": 3})
    );
}

#[test]
fn a_double_click_with_ctrl_is_two_picks_and_puts_no_caret() {
    let mut h = two_with_b();
    let b = center_of(&h, "  \"b\": 2");
    h.double_click_with(b.x, b.y, egui::Modifiers::COMMAND);
    assert!(!is_drawn_containing(&h, "Editing line"), "{:?}", h.texts());
    // Added, and taken away again, as by two clicks.
    assert!(!lines_are_picked(&h), "{:?}", h.texts());
}

#[test]
fn a_double_click_on_a_line_that_is_picked_keeps_it_picked_and_puts_a_caret_in_it() {
    let mut h = two_with_b();
    let b = center_of(&h, "  \"b\": 2");
    h.click(b.x, b.y);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    h.double_click(b.x, b.y);
    assert!(is_drawn(&h, "1 line picked"), "{:?}", h.texts());
    assert!(
        is_drawn_containing(&h, "Editing line 3 of Left"),
        "{:?}",
        h.texts()
    );
}

#[test]
fn a_difference_is_moved_by_its_arrow_after_a_double_click_that_put_a_caret_in_it() {
    let mut h = two_with_b();
    let b = center_of(&h, "  \"b\": 2");
    h.double_click(b.x, b.y);
    // The caret is in b of the left document; nothing was typed, so the arrow is not held up.
    let arrow = h.app.tools.diff_arrow(0, Side::Left).unwrap();
    h.click(arrow.x, arrow.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 3})
    );
}

#[test]
fn an_arrow_pressed_with_something_typed_takes_that_first_and_the_move_is_pressed_again() {
    let mut h = two_with_b();
    type_over(&mut h, "  \"b\": 2", "\"b\": 5");
    let arrow = h.app.tools.diff_arrow(0, Side::Right).unwrap();
    h.click(arrow.x, arrow.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (1)"));
    // Left has what was typed, and Right was not moved into.
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 5})
    );
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Right"),
        r#"{"a":1,"b":3}"#
    );
}

#[test]
fn the_view_stays_where_it_was_scrolled_to_when_a_line_far_down_is_typed_over() {
    let numbers = |changed: i32| {
        let items: Vec<String> = (0..120)
            .map(|n| {
                if n == 100 {
                    changed.to_string()
                } else {
                    format!("{}", 1000 + n)
                }
            })
            .collect();
        format!("[{}]", items.join(","))
    };
    let mut h = compared_diff(&numbers(1), &numbers(2), "Changes (1)");
    // Down to where line 100 is: Next goes to the one difference.
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    assert!(is_drawn(&h, "Difference 1 of 1"), "{:?}", h.texts());
    assert!(is_drawn(&h, "  1,"), "{:?}", h.texts());
    let before = h.texts().into_iter().find(|(t, _)| t == "  1,").unwrap().1;
    type_over(&mut h, "  1,", "9,");
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "Changes (1)"));
    // The same lines are in view, the changed one where it was.
    let after = h
        .texts()
        .into_iter()
        .find(|(t, _)| t == "  9,")
        .map(|(_, p)| p);
    assert_eq!(
        after.map(|p| p.y.round()),
        Some(before.y.round()),
        "{:?}",
        h.texts()
    );
}

#[test]
fn nothing_is_moved_while_a_line_has_something_typed_in_it_that_was_not_put_in() {
    // By an arrow while it is not JSON (the line keeps its caret to be put right)...
    let mut h = two_with_b();
    type_over(&mut h, "  \"b\": 2", "\"b\": ");
    enter(&mut h);
    h.pause(0.2);
    assert!(is_drawn_containing(&h, "Not valid JSON"), "{:?}", h.texts());
    let arrow = h.app.tools.diff_arrow(0, Side::Right).unwrap();
    h.click(arrow.x, arrow.y);
    h.pause(0.3);
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Right"),
        r#"{"a":1,"b":3}"#
    );
    assert!(is_drawn_containing(&h, "Not valid JSON"), "{:?}", h.texts());

    // ...and by a key, while it is typed and not put in.
    let mut h = two_with_b();
    type_over(&mut h, "  \"b\": 2", "\"b\": 9");
    h.key(egui::Key::ArrowRight, egui::Modifiers::ALT);
    h.pause(0.3);
    assert_eq!(
        h.app.tools.box_text(Tool::Diff, "Right"),
        r#"{"a":1,"b":3}"#
    );
    assert_eq!(h.app.tools.box_text(Tool::Diff, "Left"), r#"{"a":1,"b":2}"#);
    assert!(
        is_drawn_containing(&h, "Editing line 3 of Left"),
        "{:?}",
        h.texts()
    );
    // Put in, it is what is moved by the key afterwards: b is 9 in Left now.
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| {
        is_drawn_containing(h, "Changed line 3")
    });
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 9})
    );
}

#[test]
fn the_caret_goes_where_the_double_click_was() {
    let mut h = two_with_b();
    let line = rect_of(&h, "  \"b\": 2");
    let glyph = line.width() / "  \"b\": 2".chars().count() as f32;
    // Just before the 2 (the seventh character): what is typed goes in front of it.
    h.double_click(line.min.x + 7.0 * glyph + 1.0, line.center().y);
    h.type_text("1");
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| {
        is_drawn_containing(h, "Changed line 3")
    });
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 12})
    );
    // Past the end of the line: at the end.
    let line = rect_of(&h, "  \"b\": 12");
    h.double_click(line.max.x + 40.0, line.center().y);
    h.type_text("3");
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| {
        is_drawn_containing(h, "Changed line 3")
    });
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Left")),
        serde_json::json!({"a": 1, "b": 123})
    );
}

#[test]
fn a_line_that_is_scrolled_out_of_view_takes_what_was_typed_in_it() {
    let numbers = |last: i32| {
        let items: Vec<String> = (0..150)
            .map(|n| {
                if n == 149 {
                    last.to_string()
                } else {
                    format!("{}", 1000 + n)
                }
            })
            .collect();
        format!("[{}]", items.join(","))
    };
    let mut h = compared_diff(&numbers(1), &numbers(2), "Changes (1)");
    // Down to the last difference, and a caret in it with a number typed.
    let next = command_button(&h, "⏷");
    h.click(next.x, next.y);
    type_over(&mut h, "  1", "7");
    assert!(
        is_drawn_containing(&h, "Editing line 151 of Left"),
        "{:?}",
        h.texts()
    );
    // The wheel takes the text far away from it: it is put in, as a click elsewhere would.
    let header = rect_of(&h, "Left").center();
    h.pointer(header.x + 200.0, header.y + 120.0);
    h.frame_with(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 6000.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }]);
    h.settle();
    wait_for(&mut h, "the new comparison", |h| {
        is_drawn_containing(h, "Changed line 151")
    });
    let text = h.app.tools.box_text(Tool::Diff, "Left");
    assert!(
        as_json(&text).as_array().unwrap().last() == Some(&serde_json::json!(7)),
        "{text}"
    );
}

#[test]
fn the_text_stays_scrolled_sideways_when_a_line_is_typed_over() {
    let (x, y, z) = ("x".repeat(150), "y".repeat(150), "z".repeat(150));
    let mut h = compared_diff(
        &format!(r#"{{"a":"{x}","b":"{y}"}}"#),
        &format!(r#"{{"a":"{x}","b":"{z}"}}"#),
        "Changes (1)",
    );
    let line_a = format!("  \"a\": \"{x}\",");
    let before = rect_of(&h, &line_a).min.x;
    // The wheel, sideways, over the rows.
    h.pointer(300.0, 200.0);
    h.frame_with(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-120.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }]);
    h.settle();
    let scrolled = rect_of(&h, &line_a).min.x;
    assert!(scrolled < before - 50.0, "scrolled: {before} to {scrolled}");

    // Type over b (the end of its line is in view), as Right has it.
    let line_b = format!("  \"b\": \"{y}\"");
    let at = rect_of(&h, &line_b);
    h.double_click(at.min.x + 250.0, at.center().y);
    h.key(egui::Key::A, egui::Modifiers::COMMAND);
    h.type_text(&format!("\"b\": \"{z}\""));
    h.settle();
    // (the text follows the caret to the end of what was typed)
    let typing = rect_of(&h, &line_a).min.x;
    assert!(
        typing < scrolled,
        "the caret is kept in sight: {scrolled} to {typing}"
    );
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    let after = rect_of(&h, &line_a).min.x;
    assert!(
        (after - typing).abs() < 1.0,
        "it is where it was scrolled to: {typing}, and now {after} (not {before})"
    );
}

#[test]
fn the_caret_goes_where_the_double_click_was_when_the_text_is_scrolled_sideways() {
    let (x, y, z) = ("x".repeat(150), "y".repeat(150), "z".repeat(150));
    let mut h = compared_diff(
        &format!(r#"{{"a":"{x}","b":"{y}"}}"#),
        &format!(r#"{{"a":"{x}","b":"{z}"}}"#),
        "Changes (1)",
    );
    h.pointer(300.0, 200.0);
    h.frame_with(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-120.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }]);
    // (the wheel is eased in over some frames: let it come to rest)
    h.pause(1.0);
    let line_a = format!("  \"a\": \"{x}\",");
    let line = rect_of(&h, &line_a);
    assert!(line.min.x < 0.0, "the text is scrolled: {line:?}");
    let glyph = line.width() / line_a.chars().count() as f32;
    // Twenty characters into the line, which is twelve into the string.
    h.double_click(line.min.x + 20.0 * glyph + 1.0, line.center().y);
    h.type_text("1");
    enter(&mut h);
    wait_for(&mut h, "the new comparison", |h| {
        is_drawn_containing(h, "Changed line 2")
    });
    let left = as_json(&h.app.tools.box_text(Tool::Diff, "Left"));
    assert_eq!(
        left["a"].as_str(),
        Some(format!("{}1{}", "x".repeat(12), "x".repeat(138)).as_str())
    );
}

#[test]
fn the_text_stays_scrolled_sideways_when_a_difference_is_moved() {
    let (x, y, z) = ("x".repeat(150), "y".repeat(150), "z".repeat(150));
    let mut h = compared_diff(
        &format!(r#"{{"a":"{x}","b":"{y}"}}"#),
        &format!(r#"{{"a":"{x}","b":"{z}"}}"#),
        "Changes (1)",
    );
    let line_a = format!("  \"a\": \"{x}\",");
    let before = rect_of(&h, &line_a).min.x;
    h.pointer(300.0, 200.0);
    h.frame_with(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-120.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }]);
    // (the wheel is eased in over some frames: let it come to rest)
    h.pause(1.0);
    let scrolled = rect_of(&h, &line_a).min.x;
    assert!(scrolled < before - 50.0, "scrolled: {before} to {scrolled}");

    let arrow = h.app.tools.diff_arrow(0, Side::Left).unwrap();
    h.click(arrow.x, arrow.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    let after = rect_of(&h, &line_a).min.x;
    assert!(
        (after - scrolled).abs() < 1.0,
        "it is where it was scrolled to: {scrolled}, and now {after} (not {before})"
    );
}

#[test]
fn the_two_arrows_of_a_difference_on_the_only_line_are_both_there_and_both_work() {
    let mut h = compared_diff("5", "6", "Changes (1)");
    let (up, down) = (
        h.app.tools.diff_arrow(0, Side::Left).unwrap(),
        h.app.tools.diff_arrow(0, Side::Right).unwrap(),
    );
    assert!((down.y - up.y - 17.0).abs() < 0.5, "{up:?} {down:?}");
    h.click(down.x, down.y);
    wait_for(&mut h, "the new comparison", |h| is_drawn(h, "same"));
    assert_eq!(
        as_json(&h.app.tools.box_text(Tool::Diff, "Right")),
        serde_json::json!(5)
    );
}

#[test]
fn the_patch_can_be_copied_from_every_view() {
    let mut h = compared_diff("[1]", "[1, 2]", "Changes (1)");
    for view in ["Side by side", "Changes (1)", "Patch"] {
        press(&mut h, view);
        h.copied.clear();
        press(&mut h, "Copy patch");
        let patch = h
            .copied
            .last()
            .unwrap_or_else(|| panic!("{view}: the patch was copied"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(patch).unwrap(),
            serde_json::json!([{"op": "add", "path": "/1", "value": 2}]),
            "{view}"
        );
    }
}

#[test]
fn documents_too_long_to_lay_out_say_so_and_leave_the_list_and_the_patch() {
    // Too big for a text box, so it goes in as a file, which the box does not
    // show (a text box that size would take seconds a frame to lay out).
    let long = format!(
        "[{}]",
        (0..=jsonquery_query::diff::MAX_ROWS)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let file = temp_json("too-long.json", &long);
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill_file(Tool::Diff, "Left", &file);
    h.app.tools.fill(Tool::Diff, "Right", "[1]");
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the answer", |h| is_drawn(h, "Changes (200000)"));
    assert!(
        is_drawn_containing(&h, "Too long to show side by side"),
        "{:?}",
        h.texts()
    );
    press(&mut h, "Patch");
    assert!(is_drawn_containing(&h, r#"{"op": "remove""#));
}

#[test]
fn documents_that_are_the_same_are_said_to_be() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    // Another order of keys, and another way to write a number.
    h.app.tools.fill(Tool::Diff, "Left", r#"{"a":1,"b":2}"#);
    h.app.tools.fill(Tool::Diff, "Right", r#"{"b":2,"a":1.0}"#);
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the answer", |h| is_drawn(h, "Changes (0)"));
    // Said in the status bar, and in the list; the side-by-side view shows
    // two columns with nothing marked.
    assert!(is_drawn(&h, "same"));
    press(&mut h, "Changes (0)");
    assert!(is_drawn(&h, "The documents are the same"));
}

#[test]
fn patching_a_document_and_opening_the_result_in_the_main_window() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Patch);
    h.app.tools.fill(Tool::Patch, "Document", r#"{"a":1}"#);
    h.app.tools.fill(
        Tool::Patch,
        "Patch",
        r#"[{"op":"add","path":"/b","value":2}]"#,
    );
    h.settle();
    press(&mut h, "Apply");
    wait_for(&mut h, "the patched document", |h| {
        is_drawn(h, "{\n  \"a\": 1,\n  \"b\": 2\n}")
    });
    assert!(h.app.doc.is_none(), "nothing is opened until asked");

    press(&mut h, "Open in main window");
    let doc = h.app.doc.clone().expect("the result opened");
    assert_eq!(doc.tree(), Some(&serde_json::json!({"a": 1, "b": 2})));
    assert_eq!(doc.source.label(), "(patched)");
    assert_eq!(h.app.source_input, "", "nothing to reload from the field");
    assert!(is_drawn(&h, "(patched)"), "{:?}", h.texts());
    assert_toolbar_does_not_overlap(&h, "patched");
}

#[test]
fn a_patch_that_fails_names_the_operation() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Patch);
    h.app.tools.fill(Tool::Patch, "Document", r#"{"a":1}"#);
    h.app
        .tools
        .fill(Tool::Patch, "Patch", r#"[{"op":"remove","path":"/zzz"}]"#);
    h.settle();
    press(&mut h, "Apply");
    wait_for(&mut h, "the error", |h| {
        is_drawn_containing(h, "operation 1 (remove /zzz)")
    });
}

#[test]
fn validating_the_open_document_and_showing_a_problem_in_it() {
    let mut h = Harness::new();
    load(&mut h, r#"{"age": -1}"#);
    open_tool(&mut h, Tool::Validate);
    h.app.tools.fill(
        Tool::Validate,
        "Schema",
        r#"{"properties":{"age":{"minimum":0}},"required":["name"]}"#,
    );
    h.settle();
    let open = h.pop_buttons("Open document");
    assert_eq!(open.len(), 2, "one in each box: {:?}", h.texts());
    // The document's box is the upper one.
    h.click(open[0].x, open[0].y);

    press(&mut h, "Validate");
    wait_for(&mut h, "the problems", |h| is_drawn(h, "2 problems"));
    assert!(is_drawn(&h, "(document)") && is_drawn(&h, "/age"));

    press(&mut h, "/age");
    press(&mut h, "Show in main window");
    assert_eq!(h.app.query_text, "/age");
    assert_eq!(h.app.query_engine, Some(jsonquery_query::Kind::JsonPointer));
}

#[test]
fn a_document_that_fits_the_schema_is_valid() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Validate);
    h.app.tools.fill(Tool::Validate, "Document", r#"{"n": 1}"#);
    h.app
        .tools
        .fill(Tool::Validate, "Schema", r#"{"type":"object"}"#);
    h.settle();
    press(&mut h, "Validate");
    wait_for(&mut h, "the answer", |h| {
        is_drawn(h, "The document is valid")
    });
    assert!(is_drawn(&h, "Valid"));
}

#[test]
fn a_schema_that_cannot_be_used_is_explained() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Validate);
    h.app.tools.fill(Tool::Validate, "Document", "1");
    h.app.tools.fill(Tool::Validate, "Schema", r#"{"type": 5}"#);
    h.settle();
    press(&mut h, "Validate");
    wait_for(&mut h, "the error", |h| {
        is_drawn_containing(h, "The schema can't be used")
    });
}

#[test]
fn the_two_boxes_of_a_page_are_stacked_under_the_command_row() {
    for (tool, first, second, button) in [
        (Tool::Patch, "Document", "Patch", "Apply"),
        (Tool::Validate, "Document", "Schema", "Validate"),
    ] {
        let mut h = Harness::new();
        open_tool(&mut h, tool);
        let (a, b, c) = (rect_of(&h, first), rect_of(&h, second), rect_of(&h, button));
        assert!(
            c.min.y < a.min.y && a.min.y < b.min.y,
            "{tool:?}: the button is in the row above both boxes: {c:?} {a:?} {b:?}"
        );
        assert!(
            (a.min.x - b.min.x).abs() < 1.0,
            "{tool:?}: both titles start at the left edge: {a:?} {b:?}"
        );
        // The second box starts about half way down the room there is.
        assert!(
            (b.min.y - a.min.y) > 100.0,
            "{tool:?}: the boxes have room: {a:?} {b:?}"
        );
    }
}

#[test]
fn the_headers_of_the_two_halves_are_on_one_line_under_the_command_row() {
    for (tool, run, left_title, right_title, left, right) in [
        (
            Tool::Merge,
            "Merge",
            "Files",
            "Result",
            "Add files…",
            "Open in main window",
        ),
        (
            Tool::Format,
            "Format",
            "Input",
            "Result",
            "Open file…",
            "Copy",
        ),
        (
            Tool::Patch,
            "Apply",
            "Document",
            "Result",
            "Open file…",
            "Open in main window",
        ),
        (
            Tool::Validate,
            "Validate",
            "Document",
            "Problems",
            "Open file…",
            "Show in main window",
        ),
    ] {
        let mut h = Harness::new();
        open_tool(&mut h, tool);
        // The first of each: the upper box's, where there are two.
        let first = |text: &str| *h.pop_buttons(text).first().expect("is drawn");
        let (button, l, r) = (first(run), first(left), first(right));
        assert!(
            (l.y - r.y).abs() < 3.0,
            "{tool:?}: {left:?} at {l:?} and {right:?} at {r:?} are one line"
        );
        assert!(l.x < r.x, "{tool:?}: the right half is on the right");
        assert!(
            button.y < l.y,
            "{tool:?}: {run:?} at {button:?} is in the row above the headers"
        );

        let (lt, rt) = (rect_of(&h, left_title), rect_of(&h, right_title));
        assert!(
            (lt.center().y - rt.center().y).abs() < 2.0,
            "{tool:?}: {left_title:?} and {right_title:?} are one line: {lt:?} {rt:?}"
        );
        assert!(
            lt.min.x < rt.min.x,
            "{tool:?}: and the right one is on the right"
        );
    }
}

#[test]
fn the_tools_window_uses_the_main_windows_text_sizes() {
    // The headings and the buttons are the main window's own, not bigger ones.
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Format);
    let height_of = |h: &Harness, text: &str| -> Vec<f32> {
        text_rects(&h.shapes)
            .into_iter()
            .filter(|(t, _)| t == text)
            .map(|(_, r)| r.height())
            .collect()
    };
    let (main, tools) = (height_of(&h, "Results"), height_of(&h, "Result"));
    assert_eq!((main.len(), tools.len()), (1, 1), "{:?}", h.texts());
    assert!(
        (main[0] - tools[0]).abs() < 0.01,
        "one heading size: {main:?} {tools:?}"
    );
    // "Clear" is in the main window's toolbar and in the box's header.
    let clears = height_of(&h, "Clear");
    assert!(clears.len() >= 2, "{:?}", h.texts());
    assert!(
        clears.windows(2).all(|w| (w[0] - w[1]).abs() < 0.01),
        "one button text size: {clears:?}"
    );
}

#[test]
fn swapping_the_documents_swaps_what_the_diff_says() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app.tools.fill(Tool::Diff, "Left", "[1]");
    h.app.tools.fill(Tool::Diff, "Right", "[1, 2]");
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the answer", |h| is_drawn(h, "1 added"));

    // Swapping drops the old answer and goes back to the documents; comparing
    // again says the opposite.
    press(&mut h, "Swap");
    assert!(!is_drawn(&h, "1 added"), "{:?}", h.texts());
    assert!(is_drawn(&h, "Left") && is_drawn(&h, "Right"));
    press(&mut h, "Compare");
    wait_for(&mut h, "the opposite answer", |h| is_drawn(h, "1 removed"));
    assert!(!is_drawn(&h, "1 added"));
}

#[test]
fn the_diff_page_puts_its_two_documents_next_to_each_other_under_the_command_row() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    let (left, right) = (rect_of(&h, "Left"), rect_of(&h, "Right"));
    let compare = rect_of(&h, "Compare");
    assert!(compare.min.y < left.min.y, "{compare:?} {left:?}");
    assert!((left.center().y - right.center().y).abs() < 2.0, "one line");
    assert!(left.min.x < right.min.x, "Left is on the left");
    // Both have room: about half the window each.
    assert!(right.min.x - left.min.x > 300.0, "{left:?} {right:?}");

    // The buttons of each box are on its own header; the patch buttons are in
    // the command row, there from the start and not usable yet.
    let opens = h.pop_buttons("Open file…");
    assert_eq!(opens.len(), 2, "{:?}", h.texts());
    assert!(opens[0].x < opens[1].x && (opens[0].y - opens[1].y).abs() < 3.0);
    for button in ["Copy patch", "Save patch…"] {
        let at = rect_of(&h, button);
        assert!(
            (at.center().y - compare.center().y).abs() < 3.0,
            "{button} is in the command row: {at:?} {compare:?}"
        );
        assert!(at.min.x > right.min.x, "at the right of the row");
    }
}

#[test]
fn clicking_a_change_copies_its_path_and_the_patch_can_be_copied() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    h.app
        .tools
        .fill(Tool::Diff, "Left", r#"{"a/b": 1, "list": [1, 2]}"#);
    h.app
        .tools
        .fill(Tool::Diff, "Right", r#"{"a/b": 2, "list": [1, 2, 3]}"#);
    h.settle();
    press(&mut h, "Compare");
    wait_for(&mut h, "the changes", |h| is_drawn(h, "Changes (2)"));

    // A path is copied the way a JSON Pointer writes it.
    press(&mut h, "Changes (2)");
    press(&mut h, "/a~1b");
    assert_eq!(h.copied.last().map(String::as_str), Some("/a~1b"));
    assert!(
        is_drawn_containing(&h, "Copied the path to /a~1b"),
        "{:?}",
        h.texts()
    );

    press(&mut h, "Copy patch");
    let patch = h.copied.last().expect("the patch was copied");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(patch).unwrap(),
        serde_json::json!([
            {"op": "replace", "path": "/a~1b", "value": 2},
            {"op": "add", "path": "/list/2", "value": 3},
        ])
    );
}

#[test]
fn the_formatted_text_can_be_copied() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Format);
    h.app
        .tools
        .fill(Tool::Format, "Input", r#"{"b":[1,2],"a":null}"#);
    h.settle();

    // Nothing to copy before there is a result.
    press(&mut h, "Copy");
    assert!(h.copied.is_empty());

    press(&mut h, "Format");
    wait_for(&mut h, "the formatted text", |h| {
        is_drawn(h, "{\n  \"b\": [\n    1,\n    2\n  ],\n  \"a\": null\n}")
    });
    press(&mut h, "Copy");
    assert_eq!(
        h.copied.last().map(String::as_str),
        Some("{\n  \"b\": [\n    1,\n    2\n  ],\n  \"a\": null\n}")
    );
    assert!(is_drawn(&h, "Copied to the clipboard"));
}

#[test]
fn a_patched_document_can_be_copied() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Patch);
    h.app
        .tools
        .fill(Tool::Patch, "Document", r#"{"a":1,"b":2}"#);
    h.app
        .tools
        .fill(Tool::Patch, "Patch", r#"{"a":null,"c":3}"#);
    h.settle();
    // The default kind of patch is a list of operations, which this isn't.
    press(&mut h, "Apply");
    wait_for(&mut h, "the error", |h| {
        is_drawn_containing(h, "array of operations")
    });
}

#[test]
fn a_report_can_be_copied_and_a_problem_shown_by_double_clicking() {
    let mut h = Harness::new();
    load(&mut h, r#"{"age": -1}"#);
    open_tool(&mut h, Tool::Validate);
    h.app.tools.fill(
        Tool::Validate,
        "Schema",
        r#"{"properties":{"age":{"minimum":0}}}"#,
    );
    h.settle();
    let open = h.pop_buttons("Open document");
    h.click(open[0].x, open[0].y);
    press(&mut h, "Validate");
    wait_for(&mut h, "the problem", |h| is_drawn(h, "1 problem"));

    press(&mut h, "Copy report");
    let report = h.copied.last().expect("the report was copied").clone();
    assert!(
        report.starts_with("1 problem (Draft 2020-12)\n"),
        "{report}"
    );
    assert!(report.contains("/age: -1 is less than the minimum of 0 [minimum]"));

    // Nothing is picked yet, so Show is not available...
    press(&mut h, "Show in main window");
    assert_eq!(h.app.query_text, "");
    // ...but a double click on the problem shows it.
    let row = center_of(&h, "/age");
    h.double_click(row.x, row.y);
    assert_eq!(h.app.query_text, "/age");
    assert_eq!(h.app.query_engine, Some(jsonquery_query::Kind::JsonPointer));
}

#[test]
fn a_problem_is_shown_in_the_main_window_only_for_the_document_that_is_open() {
    let mut h = Harness::new();
    load(&mut h, r#"{"age": -1}"#);
    open_tool(&mut h, Tool::Validate);
    // The same document, but typed in rather than the open one.
    h.app
        .tools
        .fill(Tool::Validate, "Document", r#"{"age": -1}"#);
    h.app.tools.fill(
        Tool::Validate,
        "Schema",
        r#"{"properties":{"age":{"minimum":0}}}"#,
    );
    h.settle();
    press(&mut h, "Validate");
    wait_for(&mut h, "the problem", |h| is_drawn(h, "1 problem"));

    press(&mut h, "/age");
    press(&mut h, "Show in main window");
    assert_eq!(
        h.app.query_text, "",
        "a pointer means nothing to another document"
    );
}

#[test]
fn a_cancelled_job_is_not_shown_as_an_error_on_any_page() {
    for (tool, hint) in [
        (Tool::Format, "The formatted document appears here"),
        (
            Tool::Diff,
            "Put a document in both boxes to see what differs",
        ),
        (Tool::Patch, "The patched document appears here"),
        (Tool::Validate, "What does not fit the schema appears here"),
    ] {
        let mut h = Harness::new();
        open_tool(&mut h, tool);
        if tool == Tool::Diff {
            // Its answers are on the views, not on the page with the documents.
            press(&mut h, "Side by side");
        }
        assert!(is_drawn(&h, hint), "{tool:?}: {:?}", h.texts());
        h.app.tools.job_done(tool, 0, Err("cancelled".to_owned()));
        h.settle();
        assert!(is_drawn(&h, "Cancelled."), "{tool:?}: {:?}", h.texts());
        assert!(!is_drawn(&h, "cancelled"), "{tool:?}");
    }
}

#[test]
fn an_answer_for_another_tool_is_reported_not_trusted() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Diff);
    let formatted = crate::tools::jobs::run(
        crate::tools::jobs::Job::Format {
            input: crate::tools::jobs::Input::Text("[1]".to_owned()),
            options: jsonquery_query::reformat::Options::default(),
        },
        &std::sync::atomic::AtomicBool::new(false),
        &crate::settings::FileLimits::default(),
    );
    h.app.tools.job_done(Tool::Diff, 0, formatted);
    h.settle();
    assert!(
        is_drawn_containing(&h, "answer was for another tool"),
        "{:?}",
        h.texts()
    );
}

#[test]
fn clearing_a_box_brings_back_its_hint_and_the_open_document_needs_one_to_be_open() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Format);
    h.app.tools.fill(Tool::Format, "Input", "[1]");
    h.settle();
    assert!(!is_drawn(&h, "Paste JSON here, or drop a file"));
    // The main window has a Clear of its own, higher up: the box's is the lowest.
    let clear = *h.pop_buttons("Clear").last().expect("is drawn");
    h.click(clear.x, clear.y);
    assert!(
        is_drawn(&h, "Paste JSON here, or drop a file"),
        "{:?}",
        h.texts()
    );

    // With no document open, the button does nothing.
    assert!(h.app.doc.is_none());
    press(&mut h, "Open document");
    assert!(is_drawn(&h, "Paste JSON here, or drop a file"));

    // With one open, the box holds it, and says so.
    load(&mut h, r#"{"k": [true]}"#);
    press(&mut h, "Open document");
    assert!(is_drawn(&h, "The open document"), "{:?}", h.texts());
    press(&mut h, "Format");
    wait_for(&mut h, "the formatted open document", |h| {
        is_drawn(h, "{\n  \"k\": [\n    true\n  ]\n}")
    });
}

#[test]
fn a_patched_document_leaves_room_in_the_toolbar() {
    let mut h = Harness::new();
    open_tool(&mut h, Tool::Patch);
    h.app.tools.fill(Tool::Patch, "Document", "[1]");
    h.app.tools.fill(
        Tool::Patch,
        "Patch",
        r#"[{"op":"add","path":"/-","value":2}]"#,
    );
    h.settle();
    press(&mut h, "Apply");
    wait_for(&mut h, "the patched document", |h| {
        is_drawn(h, "[\n  1,\n  2\n]")
    });
    press(&mut h, "Open in main window");
    assert!(is_drawn(&h, "(patched)"), "{:?}", h.texts());
    assert_toolbar_does_not_overlap(&h, "patched");
    h.app.dock.pop_out(Pane::Query, None, None);
    h.settle();
    assert_toolbar_does_not_overlap(&h, "patched, a pane out");
}

// Results that are rows of CSV or TSV: a query ending in `@csv` or `@tsv`.

const PEOPLE: &str = r#"[{"name":"Ada","age":36},{"name":"Linus, L.","age":28}]"#;
const CSV_QUERY: &str = ".[] | [.name, .age] | @csv";
const TSV_QUERY: &str = ".[] | [.name, .age] | @tsv";

/// Run `query` in the query box and wait for its results.
fn run_and_wait(h: &mut Harness, query: &str) {
    h.app.query_text = query.to_owned();
    h.app.run_query();
    wait_for(h, "the query to finish", |h| !h.app.query_running);
}

/// What `copy` puts on the clipboard, once the worker has made the text.
fn copied_after(h: &mut Harness, copy: impl FnOnce(&mut App)) -> String {
    let before = h.copied.len();
    copy(&mut h.app);
    wait_for(h, "something on the clipboard", |h| h.copied.len() > before);
    h.copied.last().cloned().expect("something was copied")
}

fn copy_row(h: &mut Harness, row: usize) -> String {
    copied_after(h, |app| {
        app.copy_results_node(vec![PathSegment::Index(row)])
    })
}

fn copy_all(h: &mut Harness) -> String {
    copied_after(h, |app| app.copy_results_node(Vec::new()))
}

#[test]
fn csv_rows_are_copied_as_csv() {
    let mut h = Harness::new();
    load(&mut h, PEOPLE);
    run_and_wait(&mut h, CSV_QUERY);
    assert_eq!(h.app.results_format, OutputFormat::Csv);
    // A row is copied as the row, not as a quoted, escaped JSON string…
    assert_eq!(copy_row(&mut h, 0), "\"Ada\",36");
    assert_eq!(copy_row(&mut h, 1), "\"Linus, L.\",28");
    // …and the results root, which is all of them, as a line each.
    assert_eq!(copy_all(&mut h), "\"Ada\",36\n\"Linus, L.\",28");
}

#[test]
fn tsv_rows_are_copied_as_tsv() {
    let mut h = Harness::new();
    load(&mut h, PEOPLE);
    run_and_wait(&mut h, TSV_QUERY);
    assert_eq!(h.app.results_format, OutputFormat::Tsv);
    assert_eq!(copy_row(&mut h, 1), "Linus, L.\t28");
    assert_eq!(copy_all(&mut h), "Ada\t36\nLinus, L.\t28");
}

#[test]
fn results_of_any_other_query_are_still_copied_as_json() {
    let mut h = Harness::new();
    load(&mut h, PEOPLE);
    run_and_wait(&mut h, ".[] | .name");
    assert_eq!(h.app.results_format, OutputFormat::Json);
    assert_eq!(copy_row(&mut h, 0), "\"Ada\"");
    assert_eq!(copy_all(&mut h), "[\n  \"Ada\",\n  \"Linus, L.\"\n]");
    // Using `@csv` on the way to something else is not asking for CSV.
    run_and_wait(&mut h, "[.[] | [.name] | @csv] | length");
    assert_eq!(h.app.results_format, OutputFormat::Json);
    assert_eq!(copy_row(&mut h, 0), "2");
}

#[test]
fn the_format_follows_the_latest_query_and_a_new_document_starts_over() {
    let mut h = Harness::new();
    load(&mut h, PEOPLE);
    run_and_wait(&mut h, CSV_QUERY);
    assert_eq!(h.app.results_format, OutputFormat::Csv);
    run_and_wait(&mut h, TSV_QUERY);
    assert_eq!(h.app.results_format, OutputFormat::Tsv);
    run_and_wait(&mut h, ".[0]");
    assert_eq!(h.app.results_format, OutputFormat::Json);
    run_and_wait(&mut h, CSV_QUERY);
    load(&mut h, "[1]");
    assert_eq!(h.app.results_format, OutputFormat::Json);
    // A JSON Pointer or JSONPath query has no `@csv`, whatever it says.
    h.app.query_engine = Some(jsonquery_query::Kind::JsonPath);
    run_and_wait(&mut h, "$[0] | @csv");
    assert_eq!(h.app.results_format, OutputFormat::Json);
}

#[test]
fn the_text_view_shows_the_rows_as_they_will_be_copied() {
    let mut h = Harness::new();
    load(&mut h, PEOPLE);
    run_and_wait(&mut h, CSV_QUERY);
    h.app.results_view = ViewMode::Text;
    wait_for(&mut h, "the text view", |h| {
        !h.app.results_text_pending && !h.app.results_text_dirty
    });
    assert_eq!(h.app.results_text_cache, "\"Ada\",36\n\"Linus, L.\",28");
    assert!(!h.app.results_text_truncated);

    // The same view over JSON results is JSON.
    run_and_wait(&mut h, ".[] | .name");
    wait_for(&mut h, "the text view", |h| {
        !h.app.results_text_pending && !h.app.results_text_dirty
    });
    assert_eq!(
        h.app.results_text_cache,
        "[\n  \"Ada\",\n  \"Linus, L.\"\n]"
    );
}

#[test]
fn the_results_header_says_when_the_results_are_csv_or_tsv() {
    let mut h = Harness::new();
    load(&mut h, PEOPLE);
    run_and_wait(&mut h, ".[] | .name");
    assert!(
        !is_drawn(&h, "CSV") && !is_drawn(&h, "TSV"),
        "{:?}",
        h.texts()
    );
    run_and_wait(&mut h, CSV_QUERY);
    assert!(is_drawn(&h, "CSV"), "{:?}", h.texts());
    assert!(!is_drawn(&h, "TSV"));
    run_and_wait(&mut h, TSV_QUERY);
    assert!(is_drawn(&h, "TSV"), "{:?}", h.texts());
    assert!(!is_drawn(&h, "CSV"));
}

#[test]
fn the_default_results_header_is_not_changed_by_any_of_this() {
    // Where the header texts of both panes are, to the pixel.
    fn header(h: &Harness) -> Vec<(String, i32, i32)> {
        let mut found: Vec<_> = h
            .texts()
            .into_iter()
            .filter(|(t, _)| matches!(t.as_str(), "Results" | "Tree" | "Text" | "Save…"))
            .map(|(t, at)| (t, at.x.round() as i32, at.y.round() as i32))
            .collect();
        found.sort();
        found
    }
    let mut h = Harness::new();
    load(&mut h, PEOPLE);
    run_and_wait(&mut h, ".[]");
    let json = header(&h);
    assert_eq!(json.len(), 7, "{json:?}"); // Source: Tree, Text, Save…; Results: all four
    run_and_wait(&mut h, CSV_QUERY);
    // The note goes after the Tree and Text buttons and before the right edge;
    // nothing that was there moves.
    assert_eq!(header(&h), json);
}

#[test]
fn saving_after_expanding_writes_the_rows_in_the_queries_format() {
    // "Save…" over capped results re-runs the query without the cap and saves
    // when that finishes; the file must be rows, not a JSON array of strings.
    let dir = std::env::temp_dir().join(format!("jsonquery-save-rows-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (query, name, expected) in [
        (CSV_QUERY, "rows.csv", "\"Ada\",36\n\"Linus, L.\",28\n"),
        (TSV_QUERY, "rows.tsv", "Ada\t36\nLinus, L.\t28\n"),
    ] {
        let mut h = Harness::new();
        load(&mut h, PEOPLE);
        run_and_wait(&mut h, query);
        let path = dir.join(name);
        h.app.expand_results();
        h.app.pending_save_results = Some(path.clone());
        wait_for(&mut h, "the file to be saved", |h| {
            h.app.last_saved.is_some()
        });
        assert_eq!(h.app.last_saved.as_deref(), Some(path.as_path()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// The ⚠ beside the (i), on a Wayland desktop (`x11_alert.rs` tests the widget on
// its own). The tests never read the session they run from: the app starts with
// the alert hidden and these turn it on.

fn text_rect(h: &Harness, wanted: &str) -> Option<egui::Rect> {
    text_rects(&h.shapes)
        .into_iter()
        .find(|(t, _)| t == wanted)
        .map(|(_, rect)| rect)
}

#[test]
fn on_wayland_the_alert_sits_beside_the_info_button_and_moves_nothing() {
    let mut h = Harness::new();
    let bar = h.panel("status_bar").expect("the status bar");
    let info = text_rect(&h, "ℹ").expect("the (i) button");
    assert!(
        text_rect(&h, "⚠").is_none(),
        "no alert unless the desktop is Wayland"
    );

    h.app.x11_alert = X11Alert::new(true);
    h.settle();
    let alert = text_rect(&h, "⚠").expect("the alert");
    assert!(
        (alert.center().y - info.center().y).abs() < 2.0,
        "on the (i)'s row: {alert:?} vs {info:?}"
    );
    assert!(alert.max.x < info.min.x, "to the left of it: {alert:?}");
    assert!(
        info.min.x - alert.max.x < 24.0,
        "next to it, not across the bar: {alert:?} vs {info:?}"
    );
    assert_eq!(
        text_rect(&h, "ℹ"),
        Some(info),
        "the (i) stays in the corner"
    );
    assert_eq!(h.panel("status_bar"), Some(bar), "the bar is as it was");
}

#[test]
fn clicking_the_alert_opens_its_popup_above_the_status_bar() {
    let mut h = Harness::new();
    h.app.x11_alert = X11Alert::new(true);
    h.settle();
    let bar = h.panel("status_bar").expect("the status bar");
    let alert = text_rect(&h, "⚠").expect("the alert");
    assert!(
        text_rect(&h, "Wayland detected").is_none(),
        "closed at first"
    );

    h.click(alert.center().x, alert.center().y);
    let heading = text_rect(&h, "Wayland detected").expect("the popup is open");
    assert!(heading.max.y <= bar.min.y, "above the bar: {heading:?}");
    for (text, rect) in text_rects(&h.shapes) {
        if text.contains("Wayland detected") || text.contains("Download") {
            assert!(
                rect.max.x <= SCREEN.x && rect.min.x >= 0.0,
                "{text:?} is inside the window: {rect:?}"
            );
        }
    }

    h.click(alert.center().x, alert.center().y);
    assert!(text_rect(&h, "Wayland detected").is_none(), "closed again");
}

// ---- a document kept on disk ------------------------------------------------

/// `text` as a document that is indexed, as one of 256 MiB or more is, and shown
/// in the window the way a loaded one is.
fn show_lazily(h: &mut Harness, text: &str) -> std::fs::File {
    // A directory of its own for each: the tests run side by side, and a file that
    // another test writes over while this one has it mapped ends the process.
    static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "jsonquery-layout-lazy-{}-{}",
        std::process::id(),
        MADE.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("doc.json");
    std::fs::write(&path, text).unwrap();
    // Opened to be written too, for the test that cuts the file short.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let doc = jsonquery_core::load_open_file(&file, DocumentSource::File(path), 1).unwrap();
    assert!(doc.is_lazy());
    // The mapping holds the file once it is unlinked.
    let _ = std::fs::remove_dir_all(&dir);
    h.app.document_loaded(Arc::new(doc));
    h.settle();
    file
}

fn thousand_numbers(count: usize) -> String {
    format!(
        "[{}]",
        (0..count)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// The row text of a run, as the tree draws it.
fn run_row(h: &Harness, text: &str) -> Option<egui::Rect> {
    text_rects(&h.shapes)
        .into_iter()
        .find(|(t, _)| t.contains(text))
        .map(|(_, rect)| rect)
}

#[test]
fn a_long_list_of_a_document_kept_on_disk_is_shown_in_runs_that_open() {
    let mut h = Harness::new();
    show_lazily(&mut h, &thousand_numbers(2_500));

    assert!(run_row(&h, "(2500 items)").is_some(), "the list itself");
    let first = run_row(&h, "[0 … 999]").expect("the first run");
    assert!(run_row(&h, "[1000 … 1999]").is_some(), "the second run");
    assert!(run_row(&h, "[2000 … 2499]  (500 items)").is_some());
    assert!(
        h.texts().iter().all(|(t, _)| t != "7: "),
        "none of the children until a run is opened"
    );

    // The arrow is just left of the row's text.
    h.click(first.min.x - 8.0, first.center().y);
    assert!(
        h.texts().iter().any(|(t, _)| t == "7: "),
        "the first run is open: {:?}",
        h.texts()
    );
    // A thousand rows are under it now, and the other runs below them, out of sight.
    assert!(run_row(&h, "[1000 … 1999]").is_none());

    h.click(first.min.x - 8.0, first.center().y);
    assert!(h.texts().iter().all(|(t, _)| t != "7: "), "closed again");
    assert!(
        run_row(&h, "[1000 … 1999]").is_some(),
        "and the others are back"
    );
}

#[test]
fn a_parsed_document_shows_every_row_as_it_always_did() {
    let mut h = Harness::new();
    load(&mut h, &thousand_numbers(2_500));
    assert!(run_row(&h, "[0 … 999]").is_none(), "no runs");
    assert!(run_row(&h, "(2500 items)").is_some());
    assert!(
        h.texts().iter().any(|(t, _)| t == "7: "),
        "the rows are there"
    );
}

#[test]
fn a_document_kept_on_disk_says_it_was_indexed() {
    let mut h = Harness::new();
    show_lazily(&mut h, "[1, 2, 3]");
    assert!(
        h.texts().iter().any(|(t, _)| t.starts_with("Indexed in ")),
        "{:?}",
        h.texts()
    );
    let mut h = Harness::new();
    load(&mut h, "[1, 2, 3]");
    assert!(h.texts().iter().any(|(t, _)| t.starts_with("Parsed in ")));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_document_whose_file_was_cut_short_says_so_in_the_status_bar() {
    let mut h = Harness::new();
    let file = show_lazily(&mut h, &thousand_numbers(40_000));
    assert!(
        !h.texts().iter().any(|(t, _)| t.contains("changed on disk")),
        "nothing is wrong yet"
    );

    // Another program cuts the file short; reading past the cut is what finds out.
    file.set_len(16_384).unwrap();
    let doc = h.app.doc.clone().unwrap();
    let tree = doc.lazy().unwrap();
    assert_eq!(tree.bytes()[tree.len() - 1], 0);
    h.settle();
    assert!(
        h.texts().iter().any(|(t, _)| t.contains("changed on disk")),
        "{:?}",
        h.texts()
    );
}

// ---- the Settings window ----------------------------------------------------

use crate::settings::{FileLimits, Group, Limit};

/// A folder of its own for the settings of one test, and where the file goes.
fn settings_file(name: &str) -> std::path::PathBuf {
    static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    std::env::temp_dir()
        .join(format!(
            "jsonquery-layout-settings-{name}-{}-{}",
            std::process::id(),
            MADE.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ))
        .join("jsonquery_gui")
        .join("settings.json")
}

fn forget_settings(file: &std::path::Path) {
    let _ = std::fs::remove_dir_all(file.parent().and_then(|p| p.parent()).unwrap());
}

fn open_settings(h: &mut Harness) {
    let gear = text_rect(h, "⚙").expect("the ⚙ button is in the status bar");
    h.click(gear.center().x, gear.center().y);
    assert!(h.app.settings_window.is_open());
    h.settle();
}

/// The header of the Advanced limits (it says how many of them are not what they
/// were when some are not), where it is drawn.
fn advanced_header(h: &Harness) -> Option<egui::Rect> {
    text_rects(&h.shapes)
        .into_iter()
        .find(|(t, _)| t.starts_with("Advanced"))
        .map(|(_, rect)| rect)
}

/// Open the Advanced limits, which the window shows closed (or close them).
fn toggle_advanced(h: &mut Harness) {
    let header = advanced_header(h).unwrap_or_else(|| panic!("Advanced is drawn: {:?}", h.texts()));
    h.click(header.center().x, header.center().y);
    // They slide open or shut.
    h.pause(0.3);
}

/// Open the Advanced limits, which the window shows closed.
fn open_advanced(h: &mut Harness) {
    toggle_advanced(h);
    assert!(
        is_drawn(h, Limit::Copy.label()),
        "the Advanced limits are shown: {:?}",
        h.texts()
    );
}

/// Where the box of `limit` is.
fn limit_box(h: &Harness, limit: Limit) -> egui::Rect {
    h.ctx
        .read_response(egui::Id::new(("settings_limit", limit.key())))
        .unwrap_or_else(|| panic!("the box of {limit:?} is drawn: {:?}", h.texts()))
        .rect
}

/// Type `text` over what is in the box of `limit`, without confirming it.
fn type_in_limit(h: &mut Harness, limit: Limit, text: &str) {
    let at = limit_box(h, limit).center();
    h.click(at.x, at.y);
    h.key(egui::Key::A, egui::Modifiers::COMMAND);
    h.type_text(text);
    h.settle();
}

/// Type `text` over what is in the box of `limit` and press Enter.
fn set_limit(h: &mut Harness, limit: Limit, text: &str) {
    type_in_limit(h, limit, text);
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
}

/// What the box of `limit` shows.
fn limit_text(h: &Harness, limit: Limit) -> String {
    let at = limit_box(h, limit);
    // The last that is drawn there is the top one: the window is over the main
    // window's own texts.
    text_rects(&h.shapes)
        .into_iter()
        .filter(|(_, rect)| at.contains(rect.min))
        .map(|(text, _)| text)
        .next_back()
        .unwrap_or_default()
}

/// `bytes` of JSON in a file of its own, opened and waited for.
fn open_file_of(h: &mut Harness, bytes: usize) {
    static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "jsonquery-layout-open-{}-{}",
        std::process::id(),
        MADE.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("doc.json");
    let mut text = b"[1, 2, 3]".to_vec();
    text.resize(bytes, b' ');
    std::fs::write(&path, text).unwrap();
    let before = h.app.doc.clone();
    h.app.open_file(path);
    for _ in 0..300 {
        h.frame();
        let replaced = match (&before, &h.app.doc) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(was), Some(now)) => !Arc::ptr_eq(was, now),
        };
        if replaced {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_dir_all(&dir);
    h.settle();
    assert!(h.app.doc.is_some(), "the file opened");
}

#[test]
fn the_settings_button_is_beside_the_info_button_and_opens_the_window() {
    let mut h = Harness::new();
    let info = text_rect(&h, "ℹ").expect("the (i) button");
    let gear = text_rect(&h, "⚙").expect("the settings button");
    assert!(
        (gear.center().y - info.center().y).abs() < 2.0,
        "on the (i)'s row: {gear:?} vs {info:?}"
    );
    assert!(
        gear.max.x < info.min.x && info.min.x - gear.max.x < 24.0,
        "just left of it: {gear:?} vs {info:?}"
    );
    assert!(!is_drawn(&h, "File size limits"), "closed at first");

    open_settings(&mut h);
    assert!(is_drawn(&h, "File size limits"), "{:?}", h.texts());
    assert!(is_drawn(&h, Limit::KeepOnDisk.label()));
    assert_eq!(
        limit_text(&h, Limit::KeepOnDisk),
        "256 MB",
        "the limit shows what it is"
    );
    assert_eq!(
        h.app.settings,
        Settings::default(),
        "nothing changed by looking"
    );
}

#[test]
fn the_settings_button_moves_neither_the_info_button_nor_the_status_bar() {
    let mut h = Harness::new();
    let bar = h.panel("status_bar").expect("the status bar");
    let info = text_rect(&h, "ℹ").expect("the (i) button");
    open_settings(&mut h);
    assert_eq!(h.panel("status_bar"), Some(bar));
    assert_eq!(
        text_rect(&h, "ℹ"),
        Some(info),
        "the (i) stays in the corner"
    );
}

#[test]
fn a_limit_typed_in_its_box_is_taken_on_enter_and_kept_for_the_next_start() {
    let file = settings_file("enter");
    let mut h = Harness::with_store(Store::at(Some(file.clone())));
    open_settings(&mut h);
    assert!(is_drawn_containing(&h, "Kept in "), "{:?}", h.texts());

    type_in_limit(&mut h, Limit::KeepOnDisk, "512 MB");
    assert_eq!(
        h.app.settings,
        Settings::default(),
        "typing alone does not change it"
    );
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert_eq!(h.app.settings.limits.keep_on_disk(), 512 * 1024 * 1024);
    assert_eq!(limit_text(&h, Limit::KeepOnDisk), "512 MB");
    let kept: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).expect("it was written")).unwrap();
    assert_eq!(
        kept,
        serde_json::json!({"limits": {"keep_on_disk_from": "512 MB"}})
    );

    // The next start finds it there.
    let mut again = Harness::with_store(Store::at(Some(file.clone())));
    assert_eq!(again.app.settings.limits.keep_on_disk(), 512 * 1024 * 1024);
    open_settings(&mut again);
    assert_eq!(limit_text(&again, Limit::KeepOnDisk), "512 MB");
    forget_settings(&file);
}

#[test]
fn a_limit_is_taken_when_the_box_loses_the_focus_by_a_click_elsewhere_too() {
    let mut h = Harness::new();
    open_settings(&mut h);
    open_advanced(&mut h);
    type_in_limit(&mut h, Limit::Copy, "8 MB");
    let heading = text_rect(&h, "File size limits").expect("the heading");
    h.click(heading.center().x, heading.center().y);
    assert_eq!(h.app.settings.limits.copy(), 8 * 1024 * 1024);
}

#[test]
fn the_worker_opens_files_by_the_limit_that_was_set() {
    let mut h = Harness::new();
    open_file_of(&mut h, 4096);
    assert!(!h.app.doc.as_ref().unwrap().is_lazy(), "parsed, as always");

    open_settings(&mut h);
    set_limit(&mut h, Limit::KeepOnDisk, "2 KB");
    open_file_of(&mut h, 4096);
    assert!(
        h.app.doc.as_ref().unwrap().is_lazy(),
        "kept on disk from 2 KB"
    );
    assert!(
        is_drawn_containing(&h, "Indexed in "),
        "and the status bar says it was indexed: {:?}",
        h.texts()
    );

    // Put back, it is parsed again.
    press(&mut h, "Reset");
    assert!(h.app.settings.limits.all_default());
    open_file_of(&mut h, 4096);
    assert!(!h.app.doc.as_ref().unwrap().is_lazy());
}

#[test]
fn what_is_not_a_size_is_said_so_and_not_taken() {
    let mut h = Harness::new();
    open_settings(&mut h);
    open_advanced(&mut h);
    set_limit(&mut h, Limit::Tools, "banana");
    assert_eq!(h.app.settings, Settings::default());
    assert!(
        is_drawn_containing(&h, "Not a size") && is_drawn_containing(&h, "the limit stays 128 MB"),
        "{:?}",
        h.texts()
    );
    assert_eq!(
        limit_text(&h, Limit::Tools),
        "banana",
        "what was typed is left to fix"
    );

    // A size below what a limit can be.
    set_limit(&mut h, Limit::Tools, "10 B");
    assert_eq!(h.app.settings, Settings::default());
    assert!(is_drawn_containing(&h, "Too small"), "{:?}", h.texts());

    // Fixed, it goes through and the complaint is gone.
    set_limit(&mut h, Limit::Tools, "64 MB");
    assert_eq!(h.app.settings.limits.tools(), 64 * 1024 * 1024);
    assert!(
        !is_drawn_containing(&h, "the limit stays"),
        "{:?}",
        h.texts()
    );
}

#[test]
fn escape_puts_back_what_was_there() {
    let mut h = Harness::new();
    open_settings(&mut h);
    open_advanced(&mut h);
    type_in_limit(&mut h, Limit::Download, "12 GB");
    h.key(egui::Key::Escape, egui::Modifiers::NONE);
    h.settle();
    assert_eq!(h.app.settings, Settings::default());
    assert_eq!(limit_text(&h, Limit::Download), "4 GB");
}

#[test]
fn reset_puts_one_limit_back_and_restore_defaults_puts_them_all_back() {
    let mut h = Harness::new();
    open_settings(&mut h);
    open_advanced(&mut h);
    set_limit(&mut h, Limit::Copy, "8 MB");
    set_limit(&mut h, Limit::Tools, "64 MB");
    assert!(!h.app.settings.limits.all_default());

    // Reset is in the row of the limit; the first one that can be pressed is
    // the first limit that is not the default (Tools), above Copy.
    let resets: Vec<_> = h.pop_buttons("Reset");
    assert_eq!(resets.len(), Limit::ALL.len().min(resets.len()));
    let tools = limit_box(&h, Limit::Tools).center();
    let reset = resets
        .iter()
        .find(|p| (p.y - tools.y).abs() < 4.0)
        .expect("Reset is on the row of the limit");
    h.click(reset.x, reset.y);
    assert!(h.app.settings.limits.is_default(Limit::Tools));
    assert_eq!(
        h.app.settings.limits.copy(),
        8 * 1024 * 1024,
        "only that one"
    );

    press(&mut h, "Restore defaults");
    assert_eq!(h.app.settings, Settings::default());
    assert_eq!(limit_text(&h, Limit::Copy), "64 MB");
}

#[test]
fn restore_defaults_is_there_to_press_only_when_something_differs() {
    let mut h = Harness::new();
    open_settings(&mut h);
    open_advanced(&mut h);
    let restore = center_of(&h, "Restore defaults");
    h.click(restore.x, restore.y);
    assert_eq!(h.app.settings, Settings::default(), "nothing to restore");
    set_limit(&mut h, Limit::Copy, "8 MB");
    let restore = center_of(&h, "Restore defaults");
    h.click(restore.x, restore.y);
    assert!(h.app.settings.limits.all_default());
}

#[test]
fn a_settings_file_with_something_wrong_in_it_says_so_in_the_window() {
    let file = settings_file("wrong");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        r#"{"limits": {"download": "banana", "copy": "8 MB"}}"#,
    )
    .unwrap();
    let mut h = Harness::with_store(Store::at(Some(file.clone())));
    assert_eq!(
        h.app.settings.limits.copy(),
        8 * 1024 * 1024,
        "what is fine is used"
    );
    assert_eq!(
        h.app.settings.limits.download(),
        FileLimits::default().download()
    );
    open_settings(&mut h);
    assert!(
        is_drawn_containing(&h, "⚠ download: not a size"),
        "{:?}",
        h.texts()
    );
    forget_settings(&file);
}

#[test]
fn a_settings_file_that_cannot_be_written_says_so_and_the_limit_still_applies() {
    let file = settings_file("unwritable");
    // A file where the folder should be.
    std::fs::create_dir_all(file.parent().unwrap().parent().unwrap()).unwrap();
    std::fs::write(file.parent().unwrap(), "in the way").unwrap();
    let mut h = Harness::with_store(Store::at(Some(file.clone())));
    open_settings(&mut h);
    open_advanced(&mut h);
    set_limit(&mut h, Limit::Copy, "8 MB");
    assert_eq!(
        h.app.settings.limits.copy(),
        8 * 1024 * 1024,
        "applies for this run"
    );
    assert!(
        is_drawn_containing(&h, "Could not save to"),
        "{:?}",
        h.texts()
    );
    forget_settings(&file);
}

#[test]
fn without_a_folder_for_settings_the_window_says_they_are_not_kept() {
    let mut h = Harness::new();
    open_settings(&mut h);
    assert!(
        is_drawn_containing(&h, "Not kept for next time"),
        "{:?}",
        h.texts()
    );
    // They still work for the run.
    open_advanced(&mut h);
    set_limit(&mut h, Limit::Copy, "8 MB");
    assert_eq!(h.app.settings.limits.copy(), 8 * 1024 * 1024);
}

#[test]
fn the_tools_window_goes_by_the_limit_on_what_it_takes() {
    let mut h = Harness::new();
    load(&mut h, &format!("[{}1]", "1,".repeat(3000)));
    // (The windows are floating over one another here, and the one clicked last
    // is in front: each is put away when it has been used.)
    open_settings(&mut h);
    open_advanced(&mut h);
    set_limit(&mut h, Limit::Tools, "1 KB");
    let current = h.app.settings;
    h.app.settings_window.close(&current);
    open_tool(&mut h, Tool::Format);
    press(&mut h, "Open document");
    assert!(
        is_drawn_containing(&h, "more than the 1.0 KB these tools take"),
        "{:?}",
        h.texts()
    );
    assert!(!is_drawn(&h, "The open document"), "so it was not taken");

    // Higher, it is.
    open_settings(&mut h);
    open_advanced(&mut h);
    set_limit(&mut h, Limit::Tools, "1 MB");
    let current = h.app.settings;
    h.app.settings_window.close(&current);
    h.settle();
    press(&mut h, "Open document");
    assert!(is_drawn(&h, "The open document"), "{:?}", h.texts());
}

#[test]
fn a_limit_typed_but_not_confirmed_is_taken_when_the_window_is_closed() {
    let mut w = Windows::new();
    let id = Satellite::Settings.viewport_id();
    w.open_satellite(Satellite::Settings);
    w.settle_in(id);
    // Copy is one of the Advanced limits, which are closed at first.
    let header = w.rect_in(id, "Advanced").expect("the header is drawn");
    w.click_in(id, header.center());
    // The box is on the row of its label, a label's width (and a gap) to the
    // right of it; the row is indented, and the width is made less by that.
    let label = w.rect_in(id, "Largest copy").expect("the row is drawn");
    let width = crate::settings_window::LABEL_WIDTH - egui::Spacing::default().indent;
    let at = egui::pos2(label.min.x + width + 30.0, label.center().y);
    w.click_in(id, at);
    let info = egui::ViewportInfo::default;
    w.pass(
        id,
        info(),
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: Some(egui::Key::A),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    w.pass(id, info(), vec![egui::Event::Text("8 MB".to_owned())]);
    w.settle_in(id);
    assert_eq!(
        w.app().settings,
        Settings::default(),
        "typed over what was there, and not confirmed"
    );

    w.pass(id, close_request(false, false), Vec::new());
    assert!(!w.app().satellite_open(Satellite::Settings));
    assert_eq!(
        w.app().settings.limits.copy(),
        8 * 1024 * 1024,
        "what was typed is taken as the window goes"
    );
}

#[test]
fn the_window_shows_one_limit_and_the_others_when_advanced_is_opened() {
    let mut h = Harness::new();
    open_settings(&mut h);
    assert!(is_drawn(&h, Limit::MAIN.label()), "{:?}", h.texts());
    assert!(is_drawn(&h, "Advanced"));
    for limit in Limit::ALL.into_iter().filter(|l| l.is_advanced()) {
        assert!(
            !is_drawn(&h, limit.label()),
            "{limit:?} is not shown at first"
        );
    }
    assert_eq!(h.pop_buttons("Reset").len(), 1, "one limit, one Reset");

    open_advanced(&mut h);
    for limit in Limit::ALL {
        assert!(is_drawn(&h, limit.label()), "{limit:?}: {:?}", h.texts());
    }
    assert_eq!(h.pop_buttons("Reset").len(), Limit::ALL.len());
    for group in Group::ALL {
        // The first group has the one that is not Advanced, and the one that is.
        assert!(is_drawn(&h, group.title()), "{group:?}: {:?}", h.texts());
    }

    toggle_advanced(&mut h);
    for limit in Limit::ALL.into_iter().filter(|l| l.is_advanced()) {
        assert!(!is_drawn(&h, limit.label()), "{limit:?} is hidden again");
    }
    assert!(is_drawn(&h, Limit::MAIN.label()));
}

#[test]
fn advanced_is_closed_whenever_the_window_is_opened() {
    let mut h = Harness::new();
    open_settings(&mut h);
    open_advanced(&mut h);
    let current = h.app.settings;
    h.app.settings_window.close(&current);
    h.settle();
    assert!(!is_drawn(&h, Limit::Copy.label()), "the window is gone");
    open_settings(&mut h);
    assert!(
        !is_drawn(&h, Limit::Copy.label()),
        "and when it is open again, Advanced is closed: {:?}",
        h.texts()
    );
}

#[test]
fn advanced_says_how_many_of_its_limits_are_not_the_default() {
    let file = settings_file("hidden");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    // Two of the Advanced limits, and the one that is shown.
    std::fs::write(
        &file,
        r#"{"limits": {"tools": "64 MB", "query_gather": "512 MB", "keep_on_disk_from": "300 MB"}}"#,
    )
    .unwrap();
    let mut h = Harness::with_store(Store::at(Some(file.clone())));
    open_settings(&mut h);
    assert!(
        is_drawn(&h, "Advanced (2 changed)"),
        "what is hidden and is not the default is said: {:?}",
        h.texts()
    );
    press(&mut h, "Restore defaults");
    assert!(is_drawn(&h, "Advanced"), "{:?}", h.texts());
    forget_settings(&file);
}

#[test]
fn what_a_limit_is_for_is_not_written_in_the_window() {
    let mut h = Harness::new();
    open_settings(&mut h);
    open_advanced(&mut h);
    for limit in Limit::ALL {
        let first_words = limit
            .help()
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            !is_drawn_containing(&h, &first_words),
            "{limit:?} is explained in the window: {:?}",
            h.texts()
        );
    }
    assert!(!is_drawn_containing(&h, "default 256 MB"));
    assert!(!is_drawn_containing(&h, "Type a size"));
}

#[test]
fn hovering_the_name_of_a_limit_says_what_it_is_for_and_what_it_is_by_default() {
    let mut h = Harness::new();
    open_settings(&mut h);
    let name = rect_of(&h, Limit::MAIN.label());
    // Still over it for a while: a tooltip does not come at once.
    h.pointer(name.center().x, name.center().y);
    assert!(!is_drawn_containing(&h, "memory-mapped"), "not at once");
    for _ in 0..90 {
        h.frame();
    }
    assert!(
        is_drawn_containing(&h, "memory-mapped, indexed once"),
        "what it is for: {:?}",
        h.texts()
    );
    assert!(is_drawn(&h, "Default: 256 MB"), "{:?}", h.texts());
}

/// A pass of the Settings window, `size` big on a screen of `monitor`, as the
/// window manager tells it.
fn settings_pass(w: &mut Windows, size: egui::Vec2, monitor: egui::Vec2, events: Vec<egui::Event>) {
    let info = egui::ViewportInfo {
        inner_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        monitor_size: Some(monitor),
        ..Default::default()
    };
    w.pass_sized(Satellite::Settings.viewport_id(), info, events, size);
}

/// A click in the Settings window, and the passes after it.
fn settings_click(w: &mut Windows, at: egui::Pos2, size: egui::Vec2, monitor: egui::Vec2) {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    settings_pass(w, size, monitor, vec![egui::Event::PointerMoved(at)]);
    settings_pass(w, size, monitor, vec![button(true)]);
    settings_pass(w, size, monitor, vec![button(false)]);
    // The limits slide open or shut in a twelfth of a second.
    for _ in 0..12 {
        settings_pass(w, size, monitor, Vec::new());
    }
}

/// The sizes the Settings window was asked to be, in order.
fn sizes_asked(w: &Windows) -> Vec<egui::Vec2> {
    w.commands_in(Satellite::Settings.viewport_id())
        .into_iter()
        .filter_map(|command| match command {
            egui::ViewportCommand::InnerSize(size) => Some(size),
            _ => None,
        })
        .collect()
}

/// A Settings window opened and drawn at `size` (and the Advanced header's centre).
fn settings_window_at(size: egui::Vec2, monitor: egui::Vec2) -> (Windows, egui::Pos2) {
    let mut w = Windows::new();
    w.open_satellite(Satellite::Settings);
    for _ in 0..4 {
        settings_pass(&mut w, size, monitor, Vec::new());
    }
    assert!(
        sizes_asked(&w).is_empty(),
        "no size is asked for by looking"
    );
    let header = w
        .rect_in(Satellite::Settings.viewport_id(), "Advanced")
        .expect("the Advanced header is drawn");
    (w, header.center())
}

#[test]
fn the_window_grows_to_hold_the_advanced_limits_and_goes_back_when_they_are_closed() {
    let opening = egui::vec2(
        crate::settings_window::OPENING_SIZE[0],
        crate::settings_window::OPENING_SIZE[1],
    );
    let monitor = egui::vec2(1920.0, 1080.0);
    let (mut w, header) = settings_window_at(opening, monitor);

    settings_click(&mut w, header, opening, monitor);
    let asked = sizes_asked(&w);
    assert_eq!(asked.len(), 1, "once: {asked:?}");
    assert_eq!(asked[0].x, opening.x, "as wide as it was");
    assert!(
        asked[0].y > opening.y + 100.0,
        "taller, to hold eight limits and four titles: {asked:?}"
    );

    // The window manager gives it the size, and the limits are all there.
    let id = Satellite::Settings.viewport_id();
    let grown = asked[0];
    for _ in 0..4 {
        settings_pass(&mut w, grown, monitor, Vec::new());
    }
    assert!(w.rect_in(id, "Largest copy").is_some());
    assert!(w.rect_in(id, "Keys of a sort or group").is_some());
    assert_eq!(sizes_asked(&w).len(), 1, "asked for once");

    // A click on the header closes the limits: the height it had is asked for.
    settings_click(&mut w, header, grown, monitor);
    let asked = sizes_asked(&w);
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(asked[1], opening, "back to what it was");
    assert!(w.rect_in(id, "Largest copy").is_none());
}

#[test]
fn a_window_that_is_tall_enough_for_the_advanced_limits_is_not_resized() {
    let (size, monitor) = (egui::vec2(430.0, 800.0), egui::vec2(1920.0, 1080.0));
    let (mut w, header) = settings_window_at(size, monitor);
    settings_click(&mut w, header, size, monitor);
    assert!(w
        .rect_in(Satellite::Settings.viewport_id(), "Largest copy")
        .is_some());
    settings_click(&mut w, header, size, monitor);
    assert!(sizes_asked(&w).is_empty(), "{:?}", sizes_asked(&w));
}

#[test]
fn the_window_does_not_grow_past_what_the_screen_has_room_for() {
    let (size, monitor) = (egui::vec2(430.0, 190.0), egui::vec2(1920.0, 300.0));
    let (mut w, header) = settings_window_at(size, monitor);
    settings_click(&mut w, header, size, monitor);
    let asked = sizes_asked(&w);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        (asked[0].y - 270.0).abs() < 0.5,
        "nine tenths of the screen's height: {asked:?}"
    );
}

#[test]
fn a_window_opened_with_warnings_about_the_file_grows_to_hold_them() {
    let file = settings_file("warned");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        r#"{"limits": {"download": "banana", "copy": "grape", "tools": "pear"}, "theme": "purple"}"#,
    )
    .unwrap();
    let mut w = Windows::with_store(Store::at(Some(file.clone())));
    let opening = egui::vec2(
        crate::settings_window::OPENING_SIZE[0],
        crate::settings_window::OPENING_SIZE[1],
    );
    let monitor = egui::vec2(1920.0, 1080.0);
    w.open_satellite(Satellite::Settings);
    for _ in 0..8 {
        settings_pass(&mut w, opening, monitor, Vec::new());
    }
    let asked = sizes_asked(&w);
    assert_eq!(asked.len(), 1, "four warnings need room: {asked:?}");
    assert_eq!(asked[0].x, opening.x);
    assert!(asked[0].y > opening.y, "{asked:?}");

    // Given the room, it stays: nothing more is asked for, and it does not go back
    // when the Advanced limits are opened and closed.
    let grown = asked[0];
    for _ in 0..8 {
        settings_pass(&mut w, grown, monitor, Vec::new());
    }
    assert_eq!(sizes_asked(&w).len(), 1, "once");
    forget_settings(&file);
}

// ---- what the interface looks like, kept for the next start --------------------

/// The settings file of a test that starts with `contents` in it (or none).
fn start_with(name: &str, contents: Option<&str>) -> (Harness, std::path::PathBuf) {
    let file = settings_file(name);
    if let Some(contents) = contents {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, contents).unwrap();
    }
    (Harness::with_store(Store::at(Some(file.clone()))), file)
}

/// What the settings file says, if there is one.
fn kept(file: &std::path::Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

#[test]
fn starting_and_looking_at_the_app_writes_no_file() {
    let (mut h, file) = start_with("no-file", None);
    h.pause(2.0);
    assert!(!file.exists(), "nothing has been changed: nothing is kept");
    h.app.flush_settings();
    assert!(!file.exists());
    forget_settings(&file);
}

#[test]
fn the_app_starts_in_the_theme_it_was_left_in() {
    let (h, file) = start_with("theme-start", Some(r#"{"theme": "light"}"#));
    assert_eq!(h.ctx.theme(), egui::Theme::Light);
    forget_settings(&file);
    let (h, file) = start_with("theme-start-dark", Some(r#"{"theme": "dark"}"#));
    assert_eq!(h.ctx.theme(), egui::Theme::Dark);
    forget_settings(&file);
    let (h, file) = start_with("theme-start-none", None);
    assert_eq!(h.ctx.theme(), egui::Theme::Dark, "dark the first time");
    forget_settings(&file);
}

#[test]
fn a_change_of_theme_is_kept_once_it_has_stayed_for_a_moment_and_changing_back_unkeeps_it() {
    let (mut h, file) = start_with("theme-toggle", None);
    let sun = text_rect(&h, "☀").expect("the theme button");
    h.click(sun.center().x, sun.center().y);
    assert_eq!(h.ctx.theme(), egui::Theme::Light);
    assert!(
        !file.exists(),
        "not written the instant it changed (a moment is waited for)"
    );
    h.pause(1.0);
    assert_eq!(
        kept(&file),
        Some(serde_json::json!({"limits": {}, "theme": "light"}))
    );

    // And back: dark is what the app starts in, so there is nothing to keep.
    let moon = text_rect(&h, "🌙").expect("the theme button, now a moon");
    h.click(moon.center().x, moon.center().y);
    h.pause(1.0);
    assert_eq!(kept(&file), Some(serde_json::json!({"limits": {}})));
    forget_settings(&file);
}

#[test]
fn a_change_is_not_written_while_it_is_still_changing() {
    let (mut h, file) = start_with("still-changing", None);
    for width in [1100.0, 1000.0, 900.0, 950.0, 1000.0, 1050.0] {
        h.screen = egui::vec2(width, 700.0);
        h.pause(0.2);
        assert!(!file.exists(), "still being resized at {width}");
    }
    h.pause(1.0);
    assert_eq!(
        kept(&file).unwrap()["window"],
        serde_json::json!({"width": 1050.0, "height": 700.0})
    );
    forget_settings(&file);
}

#[test]
fn the_size_of_the_window_is_kept_when_it_is_not_the_size_the_app_starts_with() {
    let (mut h, file) = start_with("window-size", None);
    h.screen = egui::vec2(1500.0, 900.0);
    h.pause(1.0);
    assert_eq!(
        kept(&file).unwrap()["window"],
        serde_json::json!({"width": 1500.0, "height": 900.0})
    );
    // Back to the usual size: nothing to keep.
    h.screen = SCREEN;
    h.pause(1.0);
    assert!(
        kept(&file).unwrap().get("window").is_none(),
        "{:?}",
        kept(&file)
    );
    forget_settings(&file);
}

#[test]
fn a_maximized_window_keeps_the_size_it_had_before_and_that_it_was_maximized() {
    let (mut h, file) = start_with("window-maximized", None);
    h.screen = egui::vec2(1500.0, 900.0);
    h.pause(1.0);
    // Maximized: the screen is as big as the monitor, which is not a size to go
    // back to.
    h.window.maximized = Some(true);
    h.screen = egui::vec2(1920.0, 1080.0);
    h.pause(1.0);
    assert_eq!(
        kept(&file).unwrap()["window"],
        serde_json::json!({"width": 1500.0, "height": 900.0, "maximized": true})
    );
    // Restored, it is not.
    h.window.maximized = Some(false);
    h.screen = egui::vec2(1500.0, 900.0);
    h.pause(1.0);
    assert_eq!(
        kept(&file).unwrap()["window"],
        serde_json::json!({"width": 1500.0, "height": 900.0})
    );
    forget_settings(&file);
}

#[test]
fn the_size_a_window_has_for_a_moment_on_its_way_to_being_maximized_is_not_kept() {
    let (mut h, file) = start_with("window-transient", None);
    h.screen = egui::vec2(1500.0, 900.0);
    h.pause(1.0);
    // The window grows to the size of the screen, and only then says it is
    // maximized: that size is not one to go back to.
    h.screen = egui::vec2(1920.0, 1080.0);
    h.pause(0.1);
    h.window.maximized = Some(true);
    h.pause(1.0);
    assert_eq!(
        kept(&file).unwrap()["window"],
        serde_json::json!({"width": 1500.0, "height": 900.0, "maximized": true})
    );
    // And the other way: it says it is maximized, and has not grown yet; then it
    // is restored, and shrinks a moment later.
    h.window.maximized = Some(false);
    h.pause(0.1);
    h.screen = egui::vec2(1500.0, 900.0);
    h.pause(1.0);
    assert_eq!(
        kept(&file).unwrap()["window"],
        serde_json::json!({"width": 1500.0, "height": 900.0})
    );
    forget_settings(&file);
}

#[test]
fn a_window_left_maximized_is_asked_to_be_when_the_app_starts() {
    // Some window managers do not maximize a window that asks as it opens.
    let file = settings_file("window-ask-maximize");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, r#"{"window": {"maximized": true}}"#).unwrap();
    let h = Harness::with_store_in(Store::at(Some(file.clone())), SCREEN, Default::default());
    assert!(
        h.commands.contains(&egui::ViewportCommand::Maximized(true)),
        "{:?}",
        h.commands
    );
    forget_settings(&file);

    // One that is maximized already is not asked.
    let file = settings_file("window-asked-maximize");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, r#"{"window": {"maximized": true}}"#).unwrap();
    let h = Harness::with_store_in(
        Store::at(Some(file.clone())),
        SCREEN,
        egui::ViewportInfo {
            maximized: Some(true),
            ..Default::default()
        },
    );
    assert!(
        !h.commands.contains(&egui::ViewportCommand::Maximized(true)),
        "{:?}",
        h.commands
    );
    // And one that was not left maximized is not either.
    let (h, file2) = start_with("window-not-maximized", None);
    assert!(!h.commands.contains(&egui::ViewportCommand::Maximized(true)));
    forget_settings(&file);
    forget_settings(&file2);
}

#[test]
fn a_window_that_is_minimized_or_full_screen_keeps_nothing_of_its_size() {
    let (mut h, file) = start_with("window-minimized", None);
    h.window.minimized = Some(true);
    h.screen = egui::vec2(0.0, 0.0);
    h.pause(1.0);
    assert!(!file.exists(), "a minimized window has no size");
    h.window.minimized = Some(false);
    h.window.fullscreen = Some(true);
    h.screen = egui::vec2(1920.0, 1080.0);
    h.pause(1.0);
    assert!(!file.exists(), "nor does one that fills the screen");
    forget_settings(&file);
}

#[test]
fn a_window_too_small_to_use_is_not_a_size_to_keep() {
    let (mut h, file) = start_with("window-tiny", None);
    h.screen = egui::vec2(300.0, 200.0);
    h.pause(1.0);
    assert!(!file.exists());
    forget_settings(&file);
}

#[test]
fn a_saved_size_bigger_than_the_screen_it_opens_on_is_brought_down_to_it() {
    // Opened at the size that was saved, on a screen that is smaller.
    let file = settings_file("window-too-big");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, r#"{"window": {"width": 3000, "height": 2000}}"#).unwrap();
    let mut h = Harness::with_store_in(
        Store::at(Some(file.clone())),
        egui::vec2(3000.0, 2000.0),
        egui::ViewportInfo {
            monitor_size: Some(egui::vec2(1920.0, 1080.0)),
            ..Default::default()
        },
    );
    h.pause(0.5);
    assert!(
        h.commands
            .contains(&egui::ViewportCommand::InnerSize(egui::vec2(
                1920.0, 1080.0
            ))),
        "{:?}",
        h.commands
    );
    forget_settings(&file);

    // The size of a window that was never changed is not the app's to bring down.
    let (mut h, file) = start_with("window-default", None);
    h.window.monitor_size = Some(egui::vec2(1024.0, 768.0));
    h.pause(0.5);
    assert!(
        !h.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::InnerSize(_))),
        "{:?}",
        h.commands
    );
    forget_settings(&file);
}

#[test]
fn the_panes_open_as_big_as_they_were_left() {
    let (h, file) = start_with(
        "panes-start",
        Some(r#"{"panes": {"query_height": 150, "source_share": 0.3}}"#),
    );
    assert!(
        (h.query_panel().height() - 150.0).abs() < 1.0,
        "{:?}",
        h.query_panel()
    );
    let source = h.panel("source_panel").expect("the source panel");
    assert!(
        (source.width() - 0.3 * SCREEN.x).abs() < 2.0,
        "Source takes 30% of the width: {source:?}"
    );
    forget_settings(&file);
}

#[test]
fn the_panes_are_as_they_always_were_when_nothing_was_left() {
    let (h, file) = start_with("panes-default", None);
    assert!((h.query_panel().bottom() - LEGACY_QUERY_PANEL_BOTTOM).abs() < 0.2);
    let source = h.panel("source_panel").expect("the source panel");
    assert!((source.width() - 0.5 * SCREEN.x).abs() < 2.0, "{source:?}");
    forget_settings(&file);
}

#[test]
fn dragging_the_query_panel_is_kept() {
    let (mut h, file) = start_with("panes-drag", None);
    let panel = h.query_panel();
    // The edge under the query panel is what is dragged.
    h.drag_y(300.0, panel.bottom(), panel.bottom() + 60.0);
    h.pause(1.0);
    let height = kept(&file).expect("it was written")["panes"]["query_height"]
        .as_f64()
        .expect("a height");
    assert!(
        (height - f64::from(h.query_panel().height())).abs() < 1.0,
        "kept {height}, is {}",
        h.query_panel().height()
    );
    assert!(height > f64::from(panel.height()) + 40.0, "{height}");
    forget_settings(&file);
}

#[test]
fn dragging_the_edge_between_source_and_results_is_kept() {
    let (mut h, file) = start_with("panes-split", None);
    let source = h.panel("source_panel").expect("the source panel");
    h.drag_x(source.right(), 400.0, 300.0);
    h.pause(1.0);
    let share = kept(&file).expect("it was written")["panes"]["source_share"]
        .as_f64()
        .expect("a share");
    let now = f64::from(h.panel("source_panel").unwrap().width() / SCREEN.x);
    assert!((share - now).abs() < 0.01, "kept {share}, is {now}");
    assert!(share < 0.45, "Source got narrower: {share}");
    forget_settings(&file);
}

#[test]
fn the_light_theme_and_suggestions_are_what_the_app_starts_with_when_they_were_left_so() {
    let (h, file) = start_with(
        "start-suggest",
        Some(r#"{"theme": "light", "autocomplete": true}"#),
    );
    assert!(h.app.query_suggest.enabled);
    assert_eq!(h.ctx.theme(), egui::Theme::Light);
    forget_settings(&file);
}

#[test]
fn turning_suggestions_on_is_kept() {
    let (mut h, file) = start_with("suggest-toggle", None);
    let bulb = text_rect(&h, "💡").expect("the suggestions button");
    h.click(bulb.center().x, bulb.center().y);
    assert!(h.app.query_suggest.enabled);
    h.pause(1.0);
    assert_eq!(
        kept(&file),
        Some(serde_json::json!({"limits": {}, "autocomplete": true}))
    );
    h.click(bulb.center().x, bulb.center().y);
    h.pause(1.0);
    assert_eq!(kept(&file), Some(serde_json::json!({"limits": {}})));
    forget_settings(&file);
}

#[test]
fn what_was_changed_a_moment_ago_is_written_when_the_app_closes() {
    let (mut h, file) = start_with("flush", None);
    let sun = text_rect(&h, "☀").expect("the theme button");
    h.click(sun.center().x, sun.center().y);
    assert!(!file.exists(), "not yet");
    // The app is closing before the moment has gone by.
    h.app.flush_settings();
    assert_eq!(
        kept(&file),
        Some(serde_json::json!({"limits": {}, "theme": "light"}))
    );
    forget_settings(&file);
}

#[test]
fn the_limits_and_the_interface_are_kept_together_and_neither_loses_the_other() {
    let (mut h, file) = start_with("together", Some(r#"{"theme": "light"}"#));
    open_settings(&mut h);
    open_advanced(&mut h);
    set_limit(&mut h, Limit::Copy, "8 MB");
    assert_eq!(
        kept(&file),
        Some(serde_json::json!({"limits": {"copy": "8 MB"}, "theme": "light"}))
    );
    // And a change of the interface after it keeps the limit.
    let moon = text_rect(&h, "🌙").expect("the theme button, a moon in the light theme");
    h.click(moon.center().x, moon.center().y);
    h.pause(1.0);
    assert_eq!(
        kept(&file),
        Some(serde_json::json!({"limits": {"copy": "8 MB"}}))
    );
    forget_settings(&file);
}
