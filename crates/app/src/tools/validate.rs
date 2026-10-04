//! Validate schema: check a document against a JSON Schema and list what does
//! not fit — where in the document, what is wrong and which keyword of the
//! schema said so (see `jsonquery_query::schema`). Both boxes take typed or
//! pasted JSON, a file, or the document open in the main window.
//!
//! The command row has the Validate button and the option to check formats; the
//! left pane is the document and the schema, one above the other; the right pane
//! is the list of problems. A problem in the document that is open in the main
//! window can be shown there: that puts its JSON Pointer in the query box.

use std::sync::Arc;

use eframe::egui::{self, Align, Layout, Rect, RichText};
use jsonquery_core::Document;
use jsonquery_query::schema::{Problem, MAX_PROBLEMS};

use super::jobs::{Job, Validated};
use super::operand::{deliver, dropped_files, stacked, Operand};
use super::shared::{Env, Run};
use super::widgets::{
    command_bar, halves, header, result_area, row_background, run_button, tint, PathColumn, Tint,
    RIGHT_MIN, ROW_HEIGHT,
};
use super::{Request, Tool};

pub(super) struct Validate {
    document: Operand,
    schema: Operand,
    check_formats: bool,
    /// The problem picked in the list.
    selected: Option<usize>,
    run: Run<Validated>,
}

impl Default for Validate {
    fn default() -> Self {
        Self {
            document: Operand::new(
                "Document",
                "validate_document",
                "Paste the JSON to check, or drop a file",
            ),
            schema: Operand::new(
                "Schema",
                "validate_schema",
                "Paste the JSON Schema, or drop a file",
            ),
            check_formats: true,
            selected: None,
            run: Run::default(),
        }
    }
}

impl Validate {
    #[cfg(test)]
    pub(super) fn operand(&mut self, title: &str) -> Option<&mut Operand> {
        [&mut self.document, &mut self.schema]
            .into_iter()
            .find(|o| o.title() == title)
    }

    /// The answer to a [`Request::Job`] for this page.
    pub(super) fn done(&mut self, gen: u64, result: Result<Validated, String>) {
        self.selected = None;
        self.run.finish(gen, result);
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        if env.own_input
            && deliver(
                &mut [&mut self.document, &mut self.schema],
                dropped_files(ui.ctx()),
            )
        {
            self.changed();
        }

        let mut request = None;
        command_bar(ui, "validate_command", |ui| request = self.command_ui(ui));
        let (left, right) = halves(ui, "validate_panes", RIGHT_MIN);
        left.show(ui, |ui| self.inputs_pane(ui, env));
        right.show(ui, |ui| {
            if let Some(r) = self.result_pane(ui, env) {
                request = Some(r);
            }
        });
        request
    }

    /// What was found no longer matches what is on the page.
    fn changed(&mut self) {
        self.selected = None;
        self.run.clear();
    }

    /// The Validate button and the option to check formats.
    fn command_ui(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let busy = self.run.running();
        let mut request = None;
        ui.horizontal(|ui| {
            let ready = !self.document.is_empty() && !self.schema.is_empty();
            if run_button(
                ui,
                &self.run,
                "Validate",
                ready,
                "Validate the document (Ctrl+Enter)",
            ) {
                request = self.start();
            }
            ui.add_enabled_ui(!busy, |ui| {
                if ui
                    .checkbox(&mut self.check_formats, "Check formats")
                    .on_hover_text("Hold strings to their format: email, date-time, uuid…")
                    .changed()
                {
                    self.changed();
                }
            });
        });
        request
    }

    fn inputs_pane(&mut self, ui: &mut egui::Ui, env: &Env) {
        ui.add_enabled_ui(!self.run.running(), |ui| {
            if stacked(ui, env.open_doc, &mut self.document, &mut self.schema) {
                self.changed();
            }
        });
    }

    fn start(&mut self) -> Option<Request> {
        let (document, schema) = (self.document.input()?, self.schema.input()?);
        self.selected = None;
        let (gen, cancel) = self.run.start();
        Some(Request::Job {
            tool: Tool::Validate,
            job: Job::Validate {
                document,
                schema,
                check_formats: self.check_formats,
            },
            gen,
            cancel,
        })
    }

    /// The problems, with Show in main window and Copy report in the header —
    /// there from the start, usable once there is something to act on.
    fn result_pane(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        enum Act {
            Show,
            Copy,
        }

        let validated = self.run.outcome();
        let in_main = validated.is_some_and(|v| shows_in_main_window(v, env.open_doc));
        let problems = validated.is_some_and(|v| !v.report.is_valid());
        let picked = self.selected.is_some();
        let mut act = None;
        header(
            ui,
            "Problems",
            |_| {},
            |ui| {
                let why_not = if validated.is_none() {
                    "Validate first"
                } else if !problems {
                    "There is no problem to show"
                } else if !in_main {
                    "Only for the document that is open in the main window"
                } else {
                    "Pick a problem in the list"
                };
                // Right to left: Show in main window is the right-most.
                if ui
                    .add_enabled(
                        problems && in_main && picked,
                        egui::Button::new("Show in main window"),
                    )
                    .on_hover_text("Put the problem's JSON Pointer in the main window's query box")
                    .on_disabled_hover_text(why_not)
                    .clicked()
                {
                    act = Some(Act::Show);
                }
                if ui
                    .add_enabled(problems, egui::Button::new("Copy report"))
                    .on_hover_text("Copy the list of problems")
                    .clicked()
                {
                    act = Some(Act::Copy);
                }
            },
        );

        let open_doc = env.open_doc;
        let mut shown = None;
        result_area(
            ui,
            "validate",
            &self.run,
            "What does not fit the schema appears here",
            |ui, validated| {
                if validated.report.is_valid() {
                    let green = tint(ui.visuals(), Tint::Good);
                    ui.colored_label(green, "The document is valid");
                    ui.weak(validated.report.draft);
                    return;
                }
                let list = problems_list(ui, validated, self.selected);
                if list.selected.is_some() {
                    self.selected = list.selected;
                }
                if let Some(index) = list.opened {
                    if shows_in_main_window(validated, open_doc) {
                        shown = Some(index);
                    }
                }
            },
        );
        if let Some(index) = shown {
            if let Some(Ok(validated)) = &self.run.result {
                return Some(Request::Show(validated.report.problems[index].path.clone()));
            }
        }

        let validated = validated?;
        match act? {
            Act::Show => {
                let problem = validated.report.problems.get(self.selected?)?;
                Some(Request::Show(problem.path.clone()))
            }
            Act::Copy => {
                ui.ctx().copy_text(report_text(validated));
                env.shared
                    .say(Tool::Validate, "Copied the report to the clipboard", false);
                None
            }
        }
    }

    /// What the status bar says about the result: valid, or how many problems,
    /// and against which draft, in how long. False when there is nothing to say.
    pub(super) fn status_line(&self, ui: &mut egui::Ui) -> bool {
        let Some(validated) = self.run.outcome() else {
            return false;
        };
        let (text, kind) = status(validated);
        let color = tint(ui.visuals(), kind);
        ui.colored_label(color, text);
        ui.separator();
        ui.weak(format!(
            "Checked against {} in {:.1?}",
            validated.report.draft, validated.elapsed
        ));
        true
    }
}

/// Whether the document that was checked is the one open in the main window,
/// which is where a problem's path means something.
fn shows_in_main_window(validated: &Validated, open_doc: Option<&Arc<Document>>) -> bool {
    match (&validated.open, open_doc) {
        (Some(checked), Some(open)) => Arc::ptr_eq(checked, open),
        _ => false,
    }
}

/// "Valid", "1 problem", "3 problems", "1,000+ problems".
fn status(validated: &Validated) -> (String, Tint) {
    let report = &validated.report;
    if report.is_valid() {
        return ("Valid".to_owned(), Tint::Good);
    }
    let n = report.problems.len();
    let text = match (n, report.more) {
        (_, true) => format!("{}+ problems", thousands(n)),
        (1, false) => "1 problem".to_owned(),
        _ => format!("{} problems", thousands(n)),
    };
    (text, Tint::Bad)
}

/// 1000 as "1,000".
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    grouped
}

/// The problems as text, one to a line, for the clipboard.
fn report_text(validated: &Validated) -> String {
    let report = &validated.report;
    let (summary, _) = status(validated);
    let mut text = format!("{summary} ({})\n", report.draft);
    for problem in &report.problems {
        text.push_str(&format!(
            "{}: {} [{}]\n",
            path_or_document(&problem.path),
            problem.message,
            problem.keyword
        ));
    }
    if report.more {
        text.push_str("…and more\n");
    }
    text
}

fn path_or_document(path: &str) -> &str {
    if path.is_empty() {
        "(document)"
    } else {
        path
    }
}

/// What a click in the list did.
struct ListAction {
    /// The row that was clicked.
    selected: Option<usize>,
    /// The row that was double-clicked.
    opened: Option<usize>,
}

fn problems_list(ui: &mut egui::Ui, validated: &Validated, selected: Option<usize>) -> ListAction {
    let problems = &validated.report.problems;
    // A last row says the list was cut short.
    let rows = problems.len() + usize::from(validated.report.more);
    let mut action = ListAction {
        selected: None,
        opened: None,
    };

    let paths = PathColumn::of(ui, problems.iter().map(|p| path_or_document(&p.path)));

    // Rows touch, so their highlights do. (Set before the scroll area, which
    // takes its idea of how tall a row is from this.)
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("validate_problems")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, rows, |ui, range| {
            for index in range {
                match problems.get(index) {
                    Some(problem) => {
                        let response = problem_row(ui, problem, selected == Some(index), &paths);
                        // egui counts a click as a triple click when the one before
                        // the last was recent, so a quick click-then-double-click
                        // is not a "double" click: take both.
                        if response.double_clicked() || response.triple_clicked() {
                            action.opened = Some(index);
                        }
                        if response.clicked() {
                            action.selected = Some(index);
                        }
                    }
                    None => {
                        ui.weak(format!(
                            "Only the first {} problems are listed.",
                            thousands(MAX_PROBLEMS)
                        ));
                    }
                }
            }
        });
    action
}

/// One problem: where it is and what is wrong with it. The keyword and the
/// place in the schema are in the tooltip.
fn problem_row(
    ui: &mut egui::Ui,
    problem: &Problem,
    selected: bool,
    paths: &PathColumn,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );
    row_background(ui, rect, selected, response.hovered());

    let inner = rect.shrink2(egui::vec2(4.0, 0.0));
    let path_width = paths.width(inner.width());
    let path_rect = Rect::from_min_size(inner.min, egui::vec2(path_width, ROW_HEIGHT));
    let message_rect =
        Rect::from_min_max(egui::pos2(path_rect.max.x + 12.0, inner.min.y), inner.max);
    let cell = |ui: &mut egui::Ui, rect: Rect| {
        ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        )
    };
    // The labels don't take clicks (selecting their text would), so that a click
    // anywhere on the row is a click on the row.
    cell(ui, path_rect).add(
        egui::Label::new(RichText::new(path_or_document(&problem.path)).monospace())
            .truncate()
            .selectable(false),
    );
    cell(ui, message_rect).add(
        egui::Label::new(RichText::new(&problem.message))
            .truncate()
            .selectable(false),
    );

    response
        .on_hover_text(format!(
            "{}\n\nAt {}\nKeyword: {}\nIn the schema: {}",
            problem.message,
            path_or_document(&problem.path),
            problem.keyword,
            if problem.schema_path.is_empty() {
                "(the schema itself)"
            } else {
                &problem.schema_path
            }
        ))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use jsonquery_query::schema::Report;

    use super::*;

    fn problem(path: &str, message: &str, keyword: &str) -> Problem {
        Problem {
            path: path.to_owned(),
            message: message.to_owned(),
            keyword: keyword.to_owned(),
            schema_path: format!("/{keyword}"),
        }
    }

    fn validated(problems: Vec<Problem>, more: bool, open: Option<Arc<Document>>) -> Validated {
        Validated {
            report: Report {
                draft: "Draft 2020-12",
                problems,
                more,
            },
            open,
            elapsed: Duration::ZERO,
        }
    }

    #[test]
    fn the_status_counts_the_problems() {
        assert_eq!(status(&validated(vec![], false, None)).0, "Valid");
        let one = vec![problem("", "x", "type")];
        assert_eq!(status(&validated(one, false, None)).0, "1 problem");
        let two = vec![problem("", "x", "type"), problem("/a", "y", "type")];
        assert_eq!(status(&validated(two.clone(), false, None)).0, "2 problems");
        assert_eq!(status(&validated(two, true, None)).0, "2+ problems");
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn the_report_is_one_problem_to_a_line() {
        let v = validated(
            vec![
                problem("", "\"name\" is a required property", "required"),
                problem("/age", "-1 is less than the minimum of 0", "minimum"),
            ],
            false,
            None,
        );
        assert_eq!(
            report_text(&v),
            "2 problems (Draft 2020-12)\n\
             (document): \"name\" is a required property [required]\n\
             /age: -1 is less than the minimum of 0 [minimum]\n"
        );
    }

    #[test]
    fn a_problem_can_be_shown_only_in_the_document_that_is_open() {
        let doc = Arc::new(jsonquery_core::load_text("{}").unwrap());
        let other = Arc::new(jsonquery_core::load_text("{}").unwrap());
        let v = validated(vec![problem("", "x", "type")], false, Some(doc.clone()));
        assert!(shows_in_main_window(&v, Some(&doc)));
        assert!(!shows_in_main_window(&v, Some(&other)));
        assert!(!shows_in_main_window(&v, None));
        let pasted = validated(vec![], false, None);
        assert!(!shows_in_main_window(&pasted, Some(&doc)));
    }

    #[test]
    fn both_boxes_are_needed() {
        let mut validate = Validate::default();
        validate.document.set_text("{}");
        assert!(validate.start().is_none());
        validate.schema.set_text("{}");
        assert!(matches!(
            validate.start(),
            Some(Request::Job {
                tool: Tool::Validate,
                gen: 1,
                ..
            })
        ));
    }
}
