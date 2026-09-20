//! State handed back and forth between the egui thread and the JVM's threads.
//!
//! Kotlin calls in on whichever thread the OS used (usually the main thread)
//! and drops what it has into these queues; the egui thread drains them once a
//! frame, in `Platform::hook_raw_input` / `take_incoming`, or when a picker's
//! answer arrives. Every producer calls [`Shared::wake`] so the UI notices
//! without waiting for the next input event.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock};

use crossbeam_channel::Sender;
use eframe::egui;
use jsonquery_gui::platform::Incoming;
use jsonquery_gui::soft_keyboard::Input;

/// How much of each screen edge the system bars and the on-screen keyboard
/// cover, in physical pixels.
#[derive(Clone, Copy, Default)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Default)]
pub struct Shared {
    ctx: OnceLock<egui::Context>,
    incoming: Mutex<VecDeque<Incoming>>,
    keyboard: Mutex<Vec<Input>>,
    insets: Mutex<Insets>,
    pickers: Mutex<HashMap<i32, Sender<Option<PathBuf>>>>,
    /// What to call a file in the app cache: its name in the document picker.
    names: Mutex<HashMap<PathBuf, String>>,
    next_request: AtomicI32,
}

pub static SHARED: LazyLock<Shared> = LazyLock::new(Shared::default);

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic elsewhere must not turn every later frame into a panic too.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Shared {
    pub fn attach(&self, ctx: &egui::Context) {
        let _ = self.ctx.set(ctx.clone());
    }

    /// Ask the UI to run a frame (thread-safe).
    pub fn wake(&self) {
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint();
        }
    }

    /// A number to tell Kotlin so its answer can be matched to `answer`.
    pub fn register_picker(&self, answer: Sender<Option<PathBuf>>) -> i32 {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        locked(&self.pickers).insert(id, answer);
        id
    }

    /// Kotlin's answer to picker `id`: the path (or `None` if cancelled) and
    /// what to call the file.
    pub fn answer_picker(&self, id: i32, path: Option<PathBuf>, name: Option<String>) {
        if let (Some(path), Some(name)) = (&path, name) {
            locked(&self.names).insert(path.clone(), name);
        }
        if let Some(answer) = locked(&self.pickers).remove(&id) {
            let _ = answer.send(path);
        }
        self.wake();
    }

    pub fn push_incoming(&self, incoming: Incoming, name: Option<String>) {
        if let (Incoming::File(path), Some(name)) = (&incoming, name) {
            locked(&self.names).insert(path.clone(), name);
        }
        locked(&self.incoming).push_back(incoming);
        self.wake();
    }

    pub fn take_incoming(&self) -> Option<Incoming> {
        locked(&self.incoming).pop_front()
    }

    pub fn push_keyboard(&self, input: Input) {
        locked(&self.keyboard).push(input);
        self.wake();
    }

    pub fn take_keyboard(&self) -> Vec<Input> {
        std::mem::take(&mut *locked(&self.keyboard))
    }

    pub fn set_insets(&self, insets: Insets) {
        log::debug!(
            "insets l={} t={} r={} b={}",
            insets.left,
            insets.top,
            insets.right,
            insets.bottom
        );
        *locked(&self.insets) = insets;
        self.wake();
    }

    pub fn insets(&self) -> Insets {
        *locked(&self.insets)
    }

    pub fn name_of(&self, path: &std::path::Path) -> Option<String> {
        locked(&self.names).get(path).cloned()
    }
}
