//! The windows besides the main window and the panes' windows: the Tools
//! window, the tutorial, About and the Settings.
//!
//! Each is a viewport of its own, with its state in the app (`Tools`,
//! `Tutorial`, `InfoWindow`, `SettingsWindow`). They are drawn the way a pane's window is
//! (`App::show_popped_panes` has the reasons): on a desktop each is a
//! *deferred* viewport, which eframe redraws by itself and whose frame calls
//! back into the app through its lock (`Shared`), so that it goes on working
//! while the main window is not being redrawn. A compositor sends no redraw
//! callbacks to a window that is completely covered (GNOME does), which is what
//! a window maximized over the main window does to it; drawn inside the main
//! window's frame, as an immediate viewport is, such a window would stop
//! responding altogether. Without real windows (embedded viewports, and the
//! headless layout tests) each is an immediate viewport instead, drawn inside
//! the main window's frame.

use super::*;
use crate::tutorial;

/// A window of the app other than the main window and the panes' windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Satellite {
    Tools,
    Tutorial,
    About,
    Settings,
}

impl Satellite {
    pub(super) const ALL: [Satellite; 4] = [
        Satellite::Tools,
        Satellite::Tutorial,
        Satellite::About,
        Satellite::Settings,
    ];

    fn index(self) -> usize {
        match self {
            Satellite::Tools => 0,
            Satellite::Tutorial => 1,
            Satellite::About => 2,
            Satellite::Settings => 3,
        }
    }

    /// The id of the viewport the window is shown in.
    pub(super) fn viewport_id(self) -> egui::ViewportId {
        match self {
            Satellite::Tools => tools::viewport_id(),
            Satellite::Tutorial => tutorial::viewport_id(),
            Satellite::About => info_viewport_id(),
            Satellite::Settings => crate::settings_window::viewport_id(),
        }
    }
}

/// How long each of those windows that has been closed has waited for the main
/// window to drop it: nothing but the main window's next frame ends a window
/// (it stops showing it).
#[derive(Default)]
pub(super) struct Leaving([Option<f64>; Satellite::ALL.len()]);

impl Leaving {
    /// How long (in seconds, `now` being egui's clock) the closed window has
    /// been waiting to be dropped. Zero the first time it is asked.
    fn waited(&mut self, window: Satellite, now: f64) -> f64 {
        now - *self.0[window.index()].get_or_insert(now)
    }

    /// The window is open: the next time it is closed it starts waiting afresh.
    fn reset(&mut self, window: Satellite) {
        self.0[window.index()] = None;
    }
}

impl App {
    pub(super) fn satellite_open(&self, which: Satellite) -> bool {
        match which {
            Satellite::Tools => self.tools.is_open(),
            Satellite::Tutorial => self.tutorial.is_open(),
            Satellite::About => self.info_window.is_open(),
            Satellite::Settings => self.settings_window.is_open(),
        }
    }

    fn close_satellite(&mut self, which: Satellite) {
        match which {
            Satellite::Tools => self.tools.close(),
            Satellite::Tutorial => self.tutorial.close(),
            Satellite::About => self.info_window.close(),
            // What was typed in a box, and is a limit, is kept.
            Satellite::Settings => {
                if let Some(settings) = self.settings_window.close(&self.settings) {
                    self.apply_settings(settings);
                }
            }
        }
    }

    fn satellite_builder(&mut self, which: Satellite) -> egui::ViewportBuilder {
        match which {
            Satellite::Tools => self.tools.builder(),
            Satellite::Tutorial => self.tutorial.builder(),
            Satellite::About => self.info_window.builder(),
            Satellite::Settings => self.settings_window.builder(),
        }
    }

    /// Make each of these windows that is open show.
    ///
    /// With real windows each is a *deferred* viewport: eframe redraws it by
    /// itself and calls back into the app through its lock (`Shared`), so it
    /// goes on working while the main window is not being redrawn — which is
    /// the case once the main window is hidden. Without real windows (embedded;
    /// the layout tests) it is an immediate viewport, drawn inside the main
    /// window's frame with the same `&mut self` the main window has. Either
    /// way a window the user closes is not shown again until it is reopened.
    pub(super) fn show_satellites(
        &mut self,
        ctx: &egui::Context,
        shared: Option<&Arc<Mutex<App>>>,
    ) {
        let Some(shared) = shared.filter(|_| !ctx.embed_viewports()) else {
            self.info_window.show(ctx);
            if let Some(settings) = self.settings_window.show(ctx, &self.settings, &self.store) {
                self.apply_settings(settings);
            }
            if let Some(request) = self.tutorial.show(ctx) {
                self.apply_tutorial_request(request);
            }
            if let Some(request) = self.tools.show(ctx, self.doc.as_ref()) {
                self.apply_tools_request(ctx, request);
            }
            return;
        };
        for which in Satellite::ALL {
            if !self.satellite_open(which) {
                continue;
            }
            let builder = self.satellite_builder(which);
            let shared = Arc::clone(shared);
            ctx.show_viewport_deferred(which.viewport_id(), builder, move |ui, _class| {
                lock(&shared).satellite_frame(ui, which);
            });
            // That window is redrawn on its own, so say that this frame may
            // have changed what it shows (the theme, the open document).
            ctx.request_repaint_of(which.viewport_id());
        }
    }

    /// One frame of one of these windows, redrawn by eframe on its own (see
    /// `show_satellites`). It does what the main window's frame would have done
    /// for it, since that frame doesn't happen while the main window is hidden:
    /// takes in what the worker has said, applies what the window asks for and
    /// — only the main window's next frame can end a window — gets out of the
    /// main window's way once it is closed.
    fn satellite_frame(&mut self, ui: &mut egui::Ui, which: Satellite) {
        let ctx = ui.ctx().clone();
        self.drain_events(&ctx);

        let (info, clicked) = ctx.input(|i| (i.viewport().clone(), i.pointer.any_click()));
        if self.satellite_open(which) && info.close_requested() {
            self.close_satellite(which);
            // Closed from over the main window (maximized, or fullscreen):
            // uncover it, so that its frames resume and it drops this window.
            if info.fullscreen == Some(true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            if info.maximized == Some(true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            }
        }
        if !self.satellite_open(which) {
            self.wait_to_be_dropped_satellite(ui, which);
            return;
        }
        self.leaving.reset(which);

        match which {
            Satellite::Tools => {
                if let Some(request) = self.tools.window_frame(ui, self.doc.as_ref()) {
                    self.apply_tools_request(&ctx, request);
                }
            }
            Satellite::Tutorial => {
                if let Some(request) = self.tutorial.window_frame(ui) {
                    self.apply_tutorial_request(request);
                    // Already in front, so the result of pressing ▶ is visible
                    // straight away.
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                }
            }
            Satellite::About => info_window_frame(ui),
            Satellite::Settings => {
                if let Some(settings) =
                    self.settings_window
                        .window_frame(ui, &self.settings, &self.store)
                {
                    self.apply_settings(settings);
                }
            }
        }

        if clicked {
            // What was done here may show in the main window (a document
            // opened in it, a status message). Should that window be hidden,
            // the request stays open and eframe polls until it is met; a frame
            // of this window soon after ends the polling.
            ctx.request_repaint_of(egui::ViewportId::ROOT);
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    /// The window of one that was closed, until the main window's next frame
    /// leaves it out and egui closes it. A main window that is hidden has no
    /// frames, so after a moment the window gets out of the way itself; it is
    /// closed already.
    fn wait_to_be_dropped_satellite(&mut self, ui: &mut egui::Ui, which: Satellite) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| ui.weak("Closing…"));
        });
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        if self.leaving.waited(which, ctx.input(|i| i.time)) > WINDOW_DROP_GRACE_SECS {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        } else {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_window_has_a_viewport_of_its_own() {
        let mut ids: Vec<_> = Satellite::ALL.iter().map(|w| w.viewport_id()).collect();
        ids.extend(Pane::ALL.iter().map(|p| p.viewport_id()));
        ids.push(egui::ViewportId::ROOT);
        let distinct: std::collections::HashSet<_> = ids.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            ids.len(),
            "no two windows (panes, the main window) share one"
        );
    }

    #[test]
    fn a_closed_window_counts_the_time_from_when_it_was_first_seen() {
        let mut leaving = Leaving::default();
        assert_eq!(leaving.waited(Satellite::Tools, 10.0), 0.0);
        assert_eq!(leaving.waited(Satellite::Tools, 10.75), 0.75);
        // Each window has its own clock, and being open again starts it over.
        assert_eq!(leaving.waited(Satellite::About, 12.0), 0.0);
        leaving.reset(Satellite::Tools);
        assert_eq!(leaving.waited(Satellite::Tools, 20.0), 0.0);
        assert_eq!(leaving.waited(Satellite::About, 12.5), 0.5);
    }

    #[test]
    fn every_window_is_in_the_list_once() {
        let mut indexes: Vec<_> = Satellite::ALL.iter().map(|w| w.index()).collect();
        indexes.sort_unstable();
        assert_eq!(indexes, [0, 1, 2, 3]);
    }
}
