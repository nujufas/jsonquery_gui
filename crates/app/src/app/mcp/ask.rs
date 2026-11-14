//! What an assistant asks the user before it is done, with "Edit" allowed (and not
//! "Always allow"): deleting from the workspace, replacing an item, writing a file
//! outside it, loading a web address. The call waits (on the server's thread) while
//! the question is shown in the main window, one at a time, until the user answers or
//! [`ASK_TIME`] passes.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long a question waits for the user; then the call is refused.
pub(super) const ASK_TIME: Duration = if cfg!(test) {
    Duration::from_secs(3)
} else {
    Duration::from_secs(120)
};

/// One question.
pub(super) struct Ask {
    id: u64,
    /// What the assistant wants, as a sentence that ends in a colon: "Delete from
    /// the workspace:".
    title: String,
    /// What it is about: a path, an address.
    what: String,
    /// More, if there is more to say ("It replaces the file that is there.").
    detail: Option<String>,
    asked: Instant,
    answer: Option<bool>,
}

/// The questions that wait for the user, the first shown.
#[derive(Default)]
pub(super) struct Asks {
    queue: VecDeque<Ask>,
    next: u64,
}

impl Asks {
    /// Put a question; its number, to look for its answer by.
    pub(super) fn put(&mut self, title: String, what: String, detail: Option<String>) -> u64 {
        self.next += 1;
        self.queue.push_back(Ask {
            id: self.next,
            title,
            what,
            detail,
            asked: Instant::now(),
            answer: None,
        });
        self.next
    }

    /// The user's answer to question `id`, if they have given it (the question is
    /// then gone); `Some(false)` for one that is not there any more.
    pub(super) fn take_answer(&mut self, id: u64) -> Option<bool> {
        let at = self.queue.iter().position(|a| a.id == id);
        let Some(at) = at else {
            return Some(false);
        };
        let answer = self.queue[at].answer?;
        self.queue.remove(at);
        Some(answer)
    }

    /// Take back question `id` (the call gave up waiting).
    pub(super) fn withdraw(&mut self, id: u64) {
        self.queue.retain(|a| a.id != id);
    }

    /// Answer the first question that waits, as the user would: what it was about.
    #[cfg(test)]
    pub(super) fn answer_first(&mut self, allow: bool) -> Option<String> {
        let ask = self.queue.iter_mut().find(|a| a.answer.is_none())?;
        ask.answer = Some(allow);
        Some(format!("{} {}", ask.title, ask.what))
    }

    /// Questions are waiting for the user.
    pub(super) fn waiting(&self) -> usize {
        self.queue.iter().filter(|a| a.answer.is_none()).count()
    }

    /// Answer every question that waits.
    pub(super) fn answer_all(&mut self, allow: bool) {
        for ask in self.queue.iter_mut().filter(|a| a.answer.is_none()) {
            ask.answer = Some(allow);
        }
    }

    /// The first question that waits, over the main window, until it is answered:
    /// Allow, Always allow (this one, those that wait, and all that come: true is
    /// given back, for the panel to keep), or Don't allow (Escape too). Nothing else in
    /// the window can be used meanwhile, as with any dialog that must be answered.
    pub(super) fn show(&mut self, ctx: &egui::Context) -> bool {
        let waiting = self.waiting();
        let Some(ask) = self.queue.iter_mut().find(|a| a.answer.is_none()) else {
            return false;
        };
        let mut always = false;
        let left = ASK_TIME.saturating_sub(ask.asked.elapsed());
        let mut answer = None;
        egui::Modal::new(egui::Id::new("mcp_question")).show(ctx, |ui| {
            ui.set_width(QUESTION_WIDTH);
            ui.horizontal(|ui| {
                super::icon(ui);
                ui.strong("An assistant asks");
            });
            ui.add_space(4.0);
            ui.label(&ask.title);
            ui.add(egui::Label::new(egui::RichText::new(&ask.what).monospace()).wrap());
            if let Some(detail) = &ask.detail {
                ui.add_space(2.0);
                ui.weak(detail);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Allow").clicked() {
                    answer = Some(true);
                }
                if ui
                    .button("Always allow")
                    .on_hover_text(ALWAYS_TIP)
                    .clicked()
                {
                    always = true;
                }
                if ui.button("Don't allow").clicked() {
                    answer = Some(false);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let more = if waiting > 1 {
                        format!(" · {} more", waiting - 1)
                    } else {
                        String::new()
                    };
                    ui.weak(format!("{} s{more}", left.as_secs()));
                });
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(false);
        }
        if answer.is_some() {
            ask.answer = answer;
        }
        if always {
            self.answer_all(true);
        }
        // The seconds left go down, and the question goes when its time is up.
        ctx.request_repaint_after(Duration::from_millis(500));
        always
    }
}

/// How wide the question is.
const QUESTION_WIDTH: f32 = 400.0;

/// What "Always allow" says on hover (in the question and in the panel).
pub(super) const ALWAYS_TIP: &str =
    "Always allow — no more questions: deleting, replacing, saving files, loading web addresses\nTurn it off in the 🔌 panel";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_is_answered_once_and_then_gone() {
        let mut asks = Asks::default();
        let first = asks.put("Delete from the workspace:".into(), "a.jq".into(), None);
        let second = asks.put(
            "Open a web address:".into(),
            "https://example.com".into(),
            None,
        );
        assert_eq!(asks.waiting(), 2);
        assert_eq!(asks.take_answer(first), None, "not answered yet");
        asks.queue[0].answer = Some(true);
        assert_eq!(asks.waiting(), 1);
        assert_eq!(asks.take_answer(first), Some(true));
        assert_eq!(asks.take_answer(first), Some(false), "gone");
        asks.withdraw(second);
        assert_eq!(asks.waiting(), 0);
        assert_eq!(asks.take_answer(second), Some(false));
    }

    #[test]
    fn always_allow_answers_every_question_that_waits() {
        let mut asks = Asks::default();
        let first = asks.put("Delete from the workspace:".into(), "a.jq".into(), None);
        let second = asks.put("Save a file:".into(), "/tmp/a.json".into(), None);
        asks.answer_all(true);
        assert_eq!(asks.waiting(), 0);
        assert_eq!(asks.take_answer(first), Some(true));
        assert_eq!(asks.take_answer(second), Some(true));
    }
}
