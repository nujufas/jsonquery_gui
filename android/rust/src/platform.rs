//! `jsonquery_gui::platform::Platform` on Android: each method is a call into
//! Kotlin ([`crate::jvm`]) or a look in the queues Kotlin fills
//! ([`crate::shared`]).

use std::path::Path;
use std::sync::Mutex;

use android_activity::AndroidApp;
use eframe::egui;
use jsonquery_gui::platform::{Incoming, PathRequest, Platform};
use jsonquery_gui::soft_keyboard::SoftKeyboard;

use crate::jvm::Jvm;
use crate::shared::SHARED;

pub struct AndroidPlatform {
    jvm: Jvm,
    keyboard: Mutex<SoftKeyboard>,
}

impl AndroidPlatform {
    pub fn new(app: &AndroidApp) -> Option<Self> {
        Some(Self {
            jvm: Jvm::new(app)?,
            keyboard: Mutex::default(),
        })
    }
}

impl Platform for AndroidPlatform {
    fn attach(&self, ctx: &egui::Context) {
        SHARED.attach(ctx);
    }

    fn is_touch(&self) -> bool {
        true
    }

    fn pick_file_to_open(&self) -> PathRequest {
        let (answer, request) = PathRequest::pending();
        let id = SHARED.register_picker(answer);
        if !self.jvm.pick_open(id) {
            SHARED.answer_picker(id, None, None);
        }
        request
    }

    fn pick_file_to_save(&self, suggested_name: &str) -> PathRequest {
        let (answer, request) = PathRequest::pending();
        let id = SHARED.register_picker(answer);
        if !self.jvm.pick_save(id, suggested_name) {
            SHARED.answer_picker(id, None, None);
        }
        request
    }

    fn file_saved(&self, path: &Path) {
        self.jvm.file_saved(&path.to_string_lossy());
    }

    fn display_name(&self, path: &Path) -> Option<String> {
        SHARED.name_of(path)
    }

    fn clipboard_text(&self) -> Option<String> {
        self.jvm.clipboard_text()
    }

    fn set_clipboard_text(&self, text: &str) {
        self.jvm.set_clipboard_text(text);
    }

    fn take_incoming(&self) -> Option<Incoming> {
        SHARED.take_incoming()
    }

    fn hook_raw_input(&self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let inputs = SHARED.take_keyboard();
        if !inputs.is_empty() {
            let mut keyboard = self
                .keyboard
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for input in inputs {
                keyboard.translate(input, &mut raw.events);
            }
        }

        // Kotlin reports pixels; egui lays out in points.
        let insets = SHARED.insets();
        let points_per_pixel = 1.0 / ctx.pixels_per_point();
        let safe_area = egui::SafeAreaInsets(egui::epaint::MarginF32 {
            left: insets.left * points_per_pixel,
            top: insets.top * points_per_pixel,
            right: insets.right * points_per_pixel,
            bottom: insets.bottom * points_per_pixel,
        });
        raw.safe_area_insets = Some(safe_area);
        log_when_changed(ctx, insets.top, safe_area);
    }

    fn set_keyboard_visible(&self, visible: bool) {
        log::debug!("keyboard visible: {visible}");
        self.jvm.set_keyboard_visible(visible);
    }

    fn set_dark_theme(&self, dark: bool) {
        log::debug!("dark theme: {dark}");
        self.jvm.set_dark_theme(dark);
    }
}

/// Insets are the one thing here that only a screenshot on a device would
/// reveal going wrong, so what is sent and what egui made of it can go to
/// logcat — once per change, not per frame. Debug level: to see it, raise
/// `with_max_level` in `android_main`.
fn log_when_changed(ctx: &egui::Context, top_px: f32, sent: egui::SafeAreaInsets) {
    use std::sync::Mutex;
    static LAST: Mutex<Option<String>> = Mutex::new(None);
    let line = format!(
        "safe area: sent top={:.1}pt (from {top_px}px) bottom={:.1}pt; egui content_rect={:?} of viewport_rect={:?}; ppp={}",
        sent.0.top,
        sent.0.bottom,
        ctx.content_rect(),
        ctx.viewport_rect(),
        ctx.pixels_per_point()
    );
    let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
    if last.as_deref() != Some(line.as_str()) {
        log::debug!("{line}");
        *last = Some(line);
    }
}
