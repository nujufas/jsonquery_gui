//! Headless layout tests: the whole app runs frame by frame in a plain
//! `egui::Context` (no window, no display), so panel sizes can be measured
//! exactly and the properties the GUI can only show by eye — "a long query
//! doesn't take over the window", "a dragged height sticks" — are checked
//! every time `cargo test` runs.

use super::*;
use egui::containers::panel::PanelState;
use std::collections::HashMap;

const SCREEN: egui::Vec2 = egui::vec2(1200.0, 800.0);

struct Harness {
    ctx: egui::Context,
    app: App,
    time: f64,
    shapes: Vec<egui::epaint::ClippedShape>,
    /// Every viewport command any frame has sent.
    commands: Vec<egui::ViewportCommand>,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        let app = App::with_context(&ctx);
        let mut h = Self {
            ctx,
            app,
            time: 0.0,
            shapes: Vec::new(),
            commands: Vec::new(),
        };
        // A couple of frames so every panel has measured itself.
        h.settle();
        h
    }

    /// One frame in which `f` runs instead of the whole app.
    fn run_with(&mut self, events: Vec<egui::Event>, mut f: impl FnMut(&mut App, &mut egui::Ui)) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        self.time += 1.0 / 60.0;
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| f(app, ui));
        // Nothing paints these, and egui panics if a texture update is dropped.
        output.textures_delta.clear();
        self.shapes = output.shapes;
        for viewport in output.viewport_output.values() {
            self.commands.extend(viewport.commands.iter().cloned());
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
    h.key(
        egui::Key::End,
        egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
    );
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
/// that drain its answer are run until it is in).
fn load(h: &mut Harness, json: &str) {
    h.app.open_text(json.to_owned());
    for _ in 0..300 {
        h.frame();
        if h.app.doc.is_some() {
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
        let ctx = egui::Context::default();
        // What eframe does wherever there are windows to open.
        ctx.set_embed_viewports(false);
        let shared = Arc::new(Mutex::new(App::with_context(&ctx)));
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
        let mut input = egui::RawInput {
            viewport_id: viewport,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
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
        for _ in 0..4 {
            self.window_frame(pane, egui::ViewportInfo::default(), Vec::new());
        }
    }

    fn window_click(&mut self, pane: Pane, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let info = egui::ViewportInfo::default;
        self.window_frame(pane, info(), vec![egui::Event::PointerMoved(pos)]);
        self.window_frame(pane, info(), vec![button(true)]);
        self.window_frame(pane, info(), vec![button(false)]);
        self.window_settle(pane);
    }

    /// The centre of `text` in the window's latest pass.
    fn window_text(&self, pane: Pane, text: &str) -> egui::Pos2 {
        text_rects(&self.shapes[&pane.viewport_id()])
            .into_iter()
            .find(|(t, _)| t == text)
            .unwrap_or_else(|| panic!("{text:?} is drawn in the {} window", pane.label()))
            .1
            .center()
    }

    fn pop_out(&mut self, pane: Pane) {
        self.app().dock.pop_out(pane, None, None);
        self.main_frame(Vec::new());
        self.window_settle(pane);
    }

    fn commands_for(&self, pane: Pane) -> Vec<egui::ViewportCommand> {
        self.commands
            .iter()
            .filter(|(id, _)| *id == pane.viewport_id())
            .map(|(_, c)| c.clone())
            .collect()
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
