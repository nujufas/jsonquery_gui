//! The Tools tab of the tutorial: a lesson's examples, each with its samples —
//! the documents the tool's page takes, in its boxes or, for Merge, as files —
//! what the tool makes of them (run by the real code, see
//! `jsonquery_query::tutorial::tools`), and what to look at in that. ▶ Try it
//! puts the samples in the tool's page, sets it as the lesson has it, and runs
//! it; Load samples does that without running it.

use eframe::egui::{self, RichText, TextWrapMode};
use jsonquery_query::tutorial::tools::{self as tool_lessons, Setting, Tool, ToolExample};

use super::{code_frame, data_box, prose, tips, Request, Tab, Tutorial};

/// The most a result is shown at a time; it scrolls past that.
const RESULT_HEIGHT: f32 = 320.0;

impl Tutorial {
    /// A lesson of the Tools tab.
    pub(super) fn tools_lesson_view(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let tab = Tab::Tools;
        let slot = tab.slot();
        let topics = tool_lessons::TOPICS;
        let (t, l) = self.selected[slot];
        let (t, l) = if topics.get(t).is_some_and(|topic| l < topic.lessons.len()) {
            (t, l)
        } else {
            (0, 0)
        };
        let topic = &topics[t];
        let lesson = &topic.lessons[l];

        let mut request = None;
        let mut go_to = None;
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("tutorial_lesson", tab))
            .auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_to_top) {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| {
            ui.add_space(4.0);
            ui.weak(format!("{} › {}", tab.title(), topic.title));
            ui.heading(lesson.title);
            ui.add_space(4.0);
            prose(ui, lesson.summary);
            ui.add_space(10.0);

            for (i, example) in lesson.examples.iter().enumerate() {
                let card = self.tool_card(ui, example, (t, l, i));
                request = request.take().or(card);
                ui.add_space(10.0);
            }

            if !lesson.tips.is_empty() {
                tips(ui, lesson.tips);
                ui.add_space(10.0);
            }

            ui.separator();
            go_to = self.pager(ui, tab, (t, l));
            ui.add_space(8.0);
        });

        if let Some((nt, nl)) = go_to {
            self.selected[slot] = (nt, nl);
            self.reveal = Some((tab, nt));
            self.scroll_to_top = true;
        }
        request
    }

    /// One example: what it does, the tool and how it is set, the buttons, the
    /// samples, what the tool made of them, and what to look at.
    fn tool_card(
        &mut self,
        ui: &mut egui::Ui,
        example: &'static ToolExample,
        key: (usize, usize, usize),
    ) -> Option<Request> {
        let mut request = None;
        let outcome = self
            .tool_outcomes
            .entry(key)
            .or_insert_with(|| tool_lessons::run(example))
            .clone();

        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(example.caption).strong());
                ui.add_space(4.0);
                setting_line(ui, example);

                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    // The one primary action, filled as a query lesson's is.
                    let try_it = egui::Button::new(RichText::new("▶ Try it").strong())
                        .fill(ui.visuals().selection.bg_fill);
                    let label = example.tool().label();
                    if ui
                        .add(try_it)
                        .on_hover_text(format!("Put the samples in {label} and run it."))
                        .clicked()
                    {
                        request = Some(Request::Tool { example, run: true });
                    }
                    if ui
                        .button("Load samples")
                        .on_hover_text(format!(
                            "Put the samples in {label}, set as here, without running it."
                        ))
                        .clicked()
                    {
                        request = Some(Request::Tool {
                            example,
                            run: false,
                        });
                    }
                });

                for (i, sample) in example.samples.iter().enumerate() {
                    ui.add_space(4.0);
                    let title = match example.tool() {
                        Tool::Merge => format!("File {}", sample.name),
                        _ => sample.name.to_owned(),
                    };
                    data_box(ui, &title, sample.text, ("tool_sample", key, i));
                }

                ui.add_space(6.0);
                let heading = match (&outcome.error, example.fails) {
                    (Some(_), true) => "Result — an error, on purpose".to_owned(),
                    (Some(_), false) => "Result — error".to_owned(),
                    (None, _) => format!("Result — {}", outcome.summary),
                };
                ui.label(RichText::new(heading).strong());
                code_frame(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if let Some(error) = &outcome.error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        return;
                    }
                    egui::ScrollArea::vertical()
                        .id_salt(("tool_result", key))
                        .max_height(RESULT_HEIGHT)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&outcome.text).monospace())
                                    .wrap_mode(TextWrapMode::Wrap),
                            );
                        });
                });

                if !example.notes.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("What to look at").strong());
                    for note in example.notes {
                        ui.horizontal_top(|ui| {
                            ui.label("•");
                            prose(ui, note);
                        });
                    }
                }
            });

        request
    }
}

/// The tool, and how its page is set: a Merge's filter as code, the options of
/// the others as the command row has them.
fn setting_line(ui: &mut egui::Ui, example: &ToolExample) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(example.tool().label()).color(ui.visuals().hyperlink_color));
        match example.setting {
            Setting::Merge { filter } => {
                ui.weak("· filter");
                ui.add(
                    egui::Label::new(
                        RichText::new(filter)
                            .monospace()
                            .background_color(ui.visuals().code_bg_color),
                    )
                    .wrap_mode(TextWrapMode::Wrap),
                );
            }
            setting => {
                let options = setting.describe();
                if !options.is_empty() {
                    ui.weak(format!("· {options}"));
                }
            }
        }
    });
}
