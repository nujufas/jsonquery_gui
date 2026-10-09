//! What the pages of the Tools window have in common that is not to do with how
//! they look: the line beside a result's buttons, the bookkeeping of a job that
//! runs on the worker thread, and what each page is handed to draw itself with.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use jsonquery_core::Document;

use super::Tool;

/// How long a line like "Saved to …" stays beside the result's buttons.
const NOTICE_FOR: Duration = Duration::from_secs(5);

/// A line beside a result's buttons ("Saved to …"), shown for a few seconds.
pub(super) struct Notice {
    pub tool: Tool,
    pub text: String,
    pub error: bool,
    at: Instant,
}

/// What every page shares: the line beside the buttons, and which save the
/// window asked for is under way and for which page (so that the answer, which
/// comes back through the app, can be told from a save of the main window's).
#[derive(Default)]
pub(super) struct Shared {
    pub saving: Option<(Tool, PathBuf)>,
    notice: Option<Notice>,
}

impl Shared {
    pub fn say(&mut self, tool: Tool, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            tool,
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    /// The line to show on `tool`'s page, if there is one.
    pub fn notice_for(&self, tool: Tool) -> Option<&Notice> {
        self.notice.as_ref().filter(|n| n.tool == tool)
    }

    /// Drop the line once it has been up long enough, and ask for a repaint
    /// for the moment that happens.
    pub fn expire(&mut self, ctx: &egui::Context) {
        let expired = self
            .notice
            .as_ref()
            .is_some_and(|n| n.at.elapsed() >= NOTICE_FOR);
        if expired {
            self.notice = None;
        } else if self.notice.is_some() {
            ctx.request_repaint_after(NOTICE_FOR);
        }
    }
}

/// What a page is given to draw itself with.
pub(super) struct Env<'a> {
    /// The document open in the main window, if any.
    pub open_doc: Option<&'a Arc<Document>>,
    /// Whether files dropped on the window are this page's to take — not when
    /// the window is embedded in the main one, which has taken them already.
    pub own_input: bool,
    /// The most the tools take, all of a job's documents together: the user's
    /// limit (`settings.rs`).
    pub tool_bytes: u64,
    /// From what size a result is kept on disk, in a temporary file, and not in
    /// memory: the user's limit on keeping a file on disk (`settings.rs`).
    pub keep_bytes: u64,
    pub shared: &'a mut Shared,
}

/// A job on the worker thread and the last answer to one: the bookkeeping the
/// Format, Diff, Patch and Validate pages share.
pub(super) struct Run<T> {
    /// Which job the page is waiting on (or last got an answer for). The worker
    /// answers with the `gen` it was given, and an answer to an older job is
    /// dropped.
    gen: u64,
    /// The cancel flag of the job in flight.
    cancel: Option<Arc<AtomicBool>>,
    pub result: Option<Result<T, String>>,
}

impl<T> Default for Run<T> {
    fn default() -> Self {
        Self {
            gen: 0,
            cancel: None,
            result: None,
        }
    }
}

impl<T> Run<T> {
    pub fn running(&self) -> bool {
        self.cancel.is_some()
    }

    /// Begin a job: drops the old result and gives the `gen` and cancel flag to
    /// send the worker.
    pub fn start(&mut self) -> (u64, Arc<AtomicBool>) {
        self.gen += 1;
        self.result = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        (self.gen, cancel)
    }

    pub fn cancel(&self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The worker's answer. One to a job that has since been replaced is
    /// dropped. True when it was taken.
    pub fn finish(&mut self, gen: u64, result: Result<T, String>) -> bool {
        if gen != self.gen {
            return false;
        }
        self.cancel = None;
        self.result = Some(result);
        true
    }

    /// What was computed no longer matches what is on the page.
    pub fn clear(&mut self) {
        self.result = None;
    }

    pub fn outcome(&self) -> Option<&T> {
        match &self.result {
            Some(Ok(outcome)) => Some(outcome),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_to_an_old_job_is_dropped() {
        let mut run = Run::<u32>::default();
        let (first, _) = run.start();
        let (second, _) = run.start();
        run.finish(first, Ok(1));
        assert!(run.running());
        assert!(run.result.is_none());
        run.finish(second, Ok(2));
        assert!(!run.running());
        assert_eq!(run.outcome(), Some(&2));
    }

    #[test]
    fn starting_drops_the_old_result_and_cancelling_sets_the_flag() {
        let mut run = Run::<u32>::default();
        let (gen, cancel) = run.start();
        run.finish(gen, Err("boom".to_owned()));
        assert!(run.outcome().is_none());
        assert!(run.result.is_some());
        let (_, cancel2) = run.start();
        assert!(run.result.is_none());
        assert!(!cancel.load(Ordering::Relaxed));
        run.cancel();
        assert!(cancel2.load(Ordering::Relaxed));
        assert!(
            !cancel.load(Ordering::Relaxed),
            "the old job's flag is its own"
        );
    }

    #[test]
    fn a_notice_belongs_to_one_page() {
        let mut shared = Shared::default();
        shared.say(Tool::Format, "Copied", false);
        assert!(shared.notice_for(Tool::Format).is_some());
        assert!(shared.notice_for(Tool::Merge).is_none());
    }
}
