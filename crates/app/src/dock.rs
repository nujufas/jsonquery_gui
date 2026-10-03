//! Pop-out panes: the Query, Source and Results panes of the main window can
//! each be shown in a window of their own, and docked back again.
//!
//! This module is only the *state* — which panes are out, where each window
//! was last, where each pane sat in the main window — and the pure decisions
//! built on it (what the main window shows once some panes have left).
//! `app.rs` does the drawing: a docked pane in one of the main window's
//! panels, a popped-out one in a viewport of its own whose code runs with the
//! same `&mut self` access the docked panes have.

use std::sync::Arc;

use eframe::egui;

/// One of the three panes that can be popped out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Query,
    Source,
    Results,
}

impl Pane {
    pub const ALL: [Pane; 3] = [Pane::Query, Pane::Source, Pane::Results];

    fn index(self) -> usize {
        match self {
            Pane::Query => 0,
            Pane::Source => 1,
            Pane::Results => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Pane::Query => "Query",
            Pane::Source => "Source",
            Pane::Results => "Results",
        }
    }

    /// The id of the viewport the pane's window is shown in.
    pub fn viewport_id(self) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("jsonquery_pane_window", self.index()))
    }

    /// The smallest its window can be made.
    fn min_size(self) -> egui::Vec2 {
        match self {
            Pane::Query => egui::vec2(360.0, 150.0),
            Pane::Source | Pane::Results => egui::vec2(300.0, 200.0),
        }
    }
}

/// Where a pane's window is: its inner (client-area) size, and its top-left
/// corner on the desktop when that is known (it isn't on Wayland).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub pos: Option<egui::Pos2>,
    pub size: egui::Vec2,
}

/// What the main window shows under the query bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Central {
    /// Source on the left, Results filling the rest.
    Split,
    /// Results has left: Source fills the area.
    SourceOnly,
    /// Source has left: Results fills the area.
    ResultsOnly,
    /// Both have left: a note saying where they went.
    Empty,
}

/// A change to ask for in the middle of a frame (see [`Dock::apply_requests`]).
#[derive(Clone, Copy, Debug)]
enum Request {
    PopOut {
        pane: Pane,
        main_inner: Option<egui::Rect>,
        monitor: Option<egui::Vec2>,
    },
    Dock(Pane),
    DockAll,
}

/// Which panes are in a window of their own, and where.
#[derive(Default)]
pub struct Dock {
    popped: [bool; 3],
    /// Where each popped-out window is asked to open. Fixed for as long as
    /// the window exists: egui compares the window's builder from one frame
    /// to the next and would resize and move the real window to match a
    /// builder that kept changing, fighting the user's own dragging.
    opened_at: [Option<Placement>; 3],
    /// Where each window was last seen, so a pane popped out again reopens
    /// where the user left it.
    seen: [Option<Placement>; 3],
    /// Where each pane sits in the main window's usual arrangement — Query
    /// on top, Source and Results side by side — in the main window's points:
    /// a pane popped out for the first time opens over that spot. (Not where
    /// it is right now: with Source gone Results fills the whole width, and
    /// would open on top of Source's window.)
    home_rect: [Option<egui::Rect>; 3],
    /// When (egui's clock, in seconds) each window of a pane that is docked
    /// again was first seen waiting for the main window to drop it.
    leaving_since: [Option<f64>; 3],
    icon: Option<Arc<egui::IconData>>,
    /// Changes asked for during the frame, applied once it has been drawn.
    requests: Vec<Request>,
}

impl Dock {
    pub fn is_popped(&self, pane: Pane) -> bool {
        self.popped[pane.index()]
    }

    pub fn any_popped(&self) -> bool {
        self.popped.iter().any(|&p| p)
    }

    pub fn central(&self) -> Central {
        match (self.is_popped(Pane::Source), self.is_popped(Pane::Results)) {
            (false, false) => Central::Split,
            (false, true) => Central::SourceOnly,
            (true, false) => Central::ResultsOnly,
            (true, true) => Central::Empty,
        }
    }

    /// Remember where `pane` was drawn in the main window this frame.
    pub fn note_docked(&mut self, pane: Pane, rect: egui::Rect) {
        if pane == Pane::Query || self.central() == Central::Split {
            self.home_rect[pane.index()] = Some(rect);
        }
    }

    /// Remember where `pane`'s window is, from the viewport info egui hands
    /// its window each frame.
    pub fn note_window(&mut self, pane: Pane, info: &egui::ViewportInfo) {
        // A minimized window reports a meaningless size.
        if info.minimized == Some(true) {
            return;
        }
        if let Some(inner) = info
            .inner_rect
            .filter(|r| r.width() > 1.0 && r.height() > 1.0)
        {
            self.seen[pane.index()] = Some(Placement {
                pos: info.outer_rect.map(|r| r.min),
                size: inner.size(),
            });
        }
    }

    /// Ask for `pane` to be popped out ([`Dock::pop_out`]) once this frame is
    /// drawn. A pane is drawn in exactly one place each frame — its ids are
    /// the same wherever it is shown, so drawing it in two would make its
    /// widgets clash — and so it cannot move part-way through one.
    pub fn request_pop_out(
        &mut self,
        pane: Pane,
        main_inner: Option<egui::Rect>,
        monitor: Option<egui::Vec2>,
    ) {
        self.requests.push(Request::PopOut {
            pane,
            main_inner,
            monitor,
        });
    }

    /// Ask for `pane` to be docked back once this frame is drawn.
    pub fn request_dock(&mut self, pane: Pane) {
        self.requests.push(Request::Dock(pane));
    }

    /// Ask for every pane to be docked back once this frame is drawn.
    pub fn request_dock_all(&mut self) {
        self.requests.push(Request::DockAll);
    }

    /// Make the changes asked for during the frame — call it after the frame
    /// has been drawn.
    pub fn apply_requests(&mut self) {
        for request in std::mem::take(&mut self.requests) {
            match request {
                Request::PopOut {
                    pane,
                    main_inner,
                    monitor,
                } => self.pop_out(pane, main_inner, monitor),
                Request::Dock(pane) => self.dock(pane),
                Request::DockAll => self.dock_all(),
            }
        }
    }

    /// Put `pane` in a window of its own. `main_inner` is where the main
    /// window's client area is on the desktop (when the platform says) and
    /// `monitor` how big its screen is, so a pane's first window can open
    /// over the spot it left without running off the screen.
    pub fn pop_out(
        &mut self,
        pane: Pane,
        main_inner: Option<egui::Rect>,
        monitor: Option<egui::Vec2>,
    ) {
        let i = pane.index();
        if self.popped[i] {
            return;
        }
        let placement = self.seen[i]
            .unwrap_or_else(|| first_placement(pane, self.home_rect[i], main_inner, monitor));
        self.opened_at[i] = Some(placement);
        self.leaving_since[i] = None;
        self.popped[i] = true;
    }

    /// Bring `pane` back into the main window.
    pub fn dock(&mut self, pane: Pane) {
        let i = pane.index();
        self.popped[i] = false;
        self.opened_at[i] = None;
    }

    /// How long (in seconds, `now` being egui's clock) the window of `pane`,
    /// which is docked again, has been waiting to be dropped by the main
    /// window's frame — nothing else ends a window. Zero the first time it is
    /// asked.
    pub fn waited_to_leave(&mut self, pane: Pane, now: f64) -> f64 {
        now - *self.leaving_since[pane.index()].get_or_insert(now)
    }

    pub fn dock_all(&mut self) {
        for pane in Pane::ALL {
            self.dock(pane);
        }
    }

    /// The window `pane` is shown in. The same value every frame the window
    /// is open (see `opened_at`).
    pub fn window_builder(&mut self, pane: Pane) -> egui::ViewportBuilder {
        let placement =
            self.opened_at[pane.index()].unwrap_or_else(|| first_placement(pane, None, None, None));
        let icon = self
            .icon
            .get_or_insert_with(|| Arc::new(crate::app_icon()))
            .clone();
        let mut builder = egui::ViewportBuilder::default()
            .with_title(format!("jsonquery — {}", pane.label()))
            .with_inner_size(placement.size)
            .with_min_inner_size(pane.min_size())
            .with_app_id(crate::APP_ID)
            .with_icon(icon);
        if let Some(pos) = placement.pos {
            builder = builder.with_position(pos);
        }
        builder
    }
}

/// Where a pane's window opens the first time it is popped out: the size it
/// had in the main window (the query box gets room to grow, and for its
/// suggestion list to show), a little down and to the right of where it was
/// so it is plain that a new window opened.
fn first_placement(
    pane: Pane,
    home: Option<egui::Rect>,
    main_inner: Option<egui::Rect>,
    monitor: Option<egui::Vec2>,
) -> Placement {
    let docked_size = home.map(|r| r.size());
    let mut size = match (pane, docked_size) {
        (Pane::Query, Some(s)) => egui::vec2(s.x.min(760.0), 300.0),
        (Pane::Query, None) => egui::vec2(760.0, 300.0),
        (_, Some(s)) => s,
        (_, None) => egui::vec2(560.0, 600.0),
    };
    if let Some(monitor) = monitor {
        size = size.min(monitor * 0.9);
    }
    size = size.max(pane.min_size());
    let pos = match (main_inner, home) {
        (Some(main), Some(home)) => Some(main.min + home.min.to_vec2() + egui::vec2(32.0, 32.0)),
        _ => None,
    };
    Placement { pos, size }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
    }

    #[test]
    fn everything_starts_docked() {
        let dock = Dock::default();
        assert!(!dock.any_popped());
        assert_eq!(dock.central(), Central::Split);
        for pane in Pane::ALL {
            assert!(!dock.is_popped(pane));
        }
    }

    #[test]
    fn the_main_window_keeps_whichever_of_source_and_results_remain() {
        let mut dock = Dock::default();
        dock.pop_out(Pane::Query, None, None);
        assert_eq!(
            dock.central(),
            Central::Split,
            "the query bar isn't central"
        );
        dock.pop_out(Pane::Results, None, None);
        assert_eq!(dock.central(), Central::SourceOnly);
        dock.pop_out(Pane::Source, None, None);
        assert_eq!(dock.central(), Central::Empty);
        dock.dock(Pane::Results);
        assert_eq!(dock.central(), Central::ResultsOnly);
        dock.dock(Pane::Source);
        assert_eq!(dock.central(), Central::Split);
        assert!(dock.is_popped(Pane::Query));
    }

    #[test]
    fn dock_all_brings_every_pane_back() {
        let mut dock = Dock::default();
        for pane in Pane::ALL {
            dock.pop_out(pane, None, None);
        }
        assert!(Pane::ALL.iter().all(|&p| dock.is_popped(p)));
        dock.dock_all();
        assert!(!dock.any_popped());
        assert_eq!(dock.central(), Central::Split);
    }

    #[test]
    fn each_pane_has_its_own_viewport() {
        let ids: Vec<_> = Pane::ALL.iter().map(|p| p.viewport_id()).collect();
        assert_ne!(ids[0], ids[1]);
        assert_ne!(ids[1], ids[2]);
        assert_ne!(ids[0], ids[2]);
        assert!(ids.iter().all(|&id| id != egui::ViewportId::ROOT));
    }

    #[test]
    fn a_window_opens_over_the_spot_the_pane_left() {
        let mut dock = Dock::default();
        dock.note_docked(Pane::Results, rect(600.0, 130.0, 600.0, 640.0));
        let main_inner = rect(100.0, 80.0, 1200.0, 800.0);
        dock.pop_out(
            Pane::Results,
            Some(main_inner),
            Some(egui::vec2(1920.0, 1080.0)),
        );
        let b = dock.window_builder(Pane::Results);
        assert_eq!(b.inner_size, Some(egui::vec2(600.0, 640.0)));
        assert_eq!(
            b.position,
            Some(egui::pos2(100.0 + 600.0 + 32.0, 80.0 + 130.0 + 32.0))
        );
    }

    #[test]
    fn panes_popped_out_one_after_the_other_do_not_open_on_top_of_each_other() {
        let mut dock = Dock::default();
        let main_inner = Some(rect(0.0, 0.0, 1200.0, 800.0));
        dock.note_docked(Pane::Source, rect(0.0, 130.0, 592.0, 640.0));
        dock.note_docked(Pane::Results, rect(600.0, 130.0, 600.0, 640.0));
        dock.pop_out(Pane::Source, main_inner, None);
        // With Source gone, Results fills the whole width of the main window…
        dock.note_docked(Pane::Results, rect(0.0, 130.0, 1200.0, 640.0));
        dock.pop_out(Pane::Results, main_inner, None);
        // …but its window still opens in the slot it had beside Source's.
        let source = dock.window_builder(Pane::Source);
        let results = dock.window_builder(Pane::Results);
        assert_eq!(source.position, Some(egui::pos2(32.0, 162.0)));
        assert_eq!(results.position, Some(egui::pos2(632.0, 162.0)));
        assert_eq!(results.inner_size, Some(egui::vec2(600.0, 640.0)));
    }

    #[test]
    fn the_query_window_gets_room_for_a_long_query_and_its_suggestions() {
        let mut dock = Dock::default();
        dock.note_docked(Pane::Query, rect(0.0, 24.0, 1200.0, 101.0));
        dock.pop_out(Pane::Query, None, None);
        let b = dock.window_builder(Pane::Query);
        assert_eq!(b.inner_size, Some(egui::vec2(760.0, 300.0)));
        assert_eq!(
            b.position, None,
            "no desktop position known: the window manager chooses"
        );
    }

    #[test]
    fn a_window_never_opens_bigger_than_its_screen() {
        let mut dock = Dock::default();
        dock.note_docked(Pane::Source, rect(0.0, 130.0, 2000.0, 1500.0));
        dock.pop_out(Pane::Source, None, Some(egui::vec2(1000.0, 800.0)));
        let size = dock.window_builder(Pane::Source).inner_size.unwrap();
        assert_eq!(size, egui::vec2(900.0, 720.0));
    }

    #[test]
    fn the_builder_is_the_same_every_frame_the_window_is_open() {
        // egui patches the real window to match a changed builder, so a
        // builder that followed the window's live size would fight the user.
        let mut dock = Dock::default();
        dock.pop_out(Pane::Source, None, None);
        let first = dock.window_builder(Pane::Source);
        dock.note_window(
            Pane::Source,
            &egui::ViewportInfo {
                inner_rect: Some(rect(10.0, 10.0, 321.0, 456.0)),
                ..Default::default()
            },
        );
        assert_eq!(dock.window_builder(Pane::Source), first);
    }

    #[test]
    fn a_pane_popped_out_again_reopens_where_it_was_left() {
        let mut dock = Dock::default();
        dock.pop_out(Pane::Source, None, None);
        dock.note_window(
            Pane::Source,
            &egui::ViewportInfo {
                inner_rect: Some(rect(210.0, 160.0, 480.0, 500.0)),
                outer_rect: Some(rect(208.0, 130.0, 484.0, 532.0)),
                ..Default::default()
            },
        );
        dock.dock(Pane::Source);
        dock.pop_out(Pane::Source, None, None);
        let b = dock.window_builder(Pane::Source);
        assert_eq!(b.inner_size, Some(egui::vec2(480.0, 500.0)));
        assert_eq!(b.position, Some(egui::pos2(208.0, 130.0)));
    }

    #[test]
    fn a_minimized_window_is_not_remembered() {
        let mut dock = Dock::default();
        dock.pop_out(Pane::Query, None, None);
        dock.note_window(
            Pane::Query,
            &egui::ViewportInfo {
                inner_rect: Some(rect(0.0, 0.0, 1.0, 1.0)),
                minimized: Some(true),
                ..Default::default()
            },
        );
        dock.dock(Pane::Query);
        dock.pop_out(Pane::Query, None, None);
        let size = dock.window_builder(Pane::Query).inner_size.unwrap();
        assert!(size.x >= 360.0 && size.y >= 150.0);
    }

    #[test]
    fn requests_wait_for_the_end_of_the_frame() {
        let mut dock = Dock::default();
        dock.request_pop_out(Pane::Source, None, None);
        dock.request_pop_out(Pane::Query, None, None);
        assert!(!dock.any_popped(), "nothing moves while the frame is drawn");
        dock.apply_requests();
        assert!(dock.is_popped(Pane::Source) && dock.is_popped(Pane::Query));

        dock.request_dock(Pane::Source);
        assert!(dock.is_popped(Pane::Source));
        dock.apply_requests();
        assert!(!dock.is_popped(Pane::Source) && dock.is_popped(Pane::Query));

        dock.request_dock_all();
        dock.apply_requests();
        assert!(!dock.any_popped());
        // Applied once.
        dock.apply_requests();
        assert!(!dock.any_popped());
    }

    #[test]
    fn popping_out_twice_changes_nothing() {
        let mut dock = Dock::default();
        dock.pop_out(Pane::Results, None, None);
        let before = dock.window_builder(Pane::Results);
        dock.pop_out(Pane::Results, Some(rect(5.0, 5.0, 10.0, 10.0)), None);
        assert_eq!(dock.window_builder(Pane::Results), before);
    }

    #[test]
    fn a_window_that_outlives_its_docking_counts_the_time_from_when_it_was_first_seen() {
        let mut dock = Dock::default();
        dock.pop_out(Pane::Source, None, None);
        dock.dock(Pane::Source);
        assert_eq!(dock.waited_to_leave(Pane::Source, 10.0), 0.0);
        assert_eq!(dock.waited_to_leave(Pane::Source, 10.75), 0.75);
        // Each pane has its own clock, and popping out again starts it over.
        assert_eq!(dock.waited_to_leave(Pane::Results, 12.0), 0.0);
        dock.pop_out(Pane::Source, None, None);
        dock.dock(Pane::Source);
        assert_eq!(dock.waited_to_leave(Pane::Source, 20.0), 0.0);
    }
}
