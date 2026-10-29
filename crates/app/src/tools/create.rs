//! Create Schema: make a JSON Schema from a document — the types of its values,
//! the members of its objects and which of them every one has, and what its
//! strings are: a date, a uuid, an address, a code of one shape, a list of
//! them with commas between (see `jsonquery_query::infer`). The document can be
//! typed or pasted, a file, or the document open in the main window.
//!
//! The command row has the Create button and the options; the left pane is the
//! document; the right pane is the schema, with Use in Validate, Save…, Copy and
//! Reset in its header, and under it the patterns that were found, a row each.
//!
//! The schema made can be changed: typed over in its box, or a pattern at a time —
//! a click on a pattern opens it under the list, to write another expression in
//! its place, to make it plain (a string of any kind; for keys, any key), or to
//! put back what was found (`jsonquery_query::infer::change`). What is copied,
//! saved and checked is the schema as it is then; Reset goes back to the one made.
//!
//! The pattern that is open is pointed out where it is: in the schema, and in
//! the document — the first string (or key) it was found in, and with ⏶ and ⏷
//! the others (`jsonquery_query::infer::occurrences`).
//!
//! An enum (the Enums option) is in the list too. Open, it has its values, each
//! with a tick to keep it or leave it out, a box to put in another, and the
//! buttons that make it a plain string or put back the values found.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use eframe::egui::{self, Align, Layout, Rect, RichText};
use jsonquery_query::infer::{self, Draft, Found, Now, Options};
use jsonquery_query::reformat;
use serde_json::Value;

use super::format::COPY_LIMIT;
use super::jobs::{Created, Job};
use super::operand::{deliver, dropped_files, Operand, TEXT_LIMIT};
use super::shared::{Env, Run};
use super::widgets::{
    command_bar, halves, header, preview_box, result_area, row_background, run_button, text_box,
    tint, Mark, PathColumn, Tint, ERROR, RIGHT_MIN, ROW_HEIGHT, STACK_GAP,
};
use super::{Request, Tool};

/// The share of the right pane's height the schema has; the patterns found have
/// the rest.
const SCHEMA_SHARE: f32 = 0.6;

pub(super) struct Create {
    document: Operand,
    options: Options,
    run: Run<Created>,
    /// The schema made, as it has been changed since: what the page shows, and
    /// what Copy, Save… and Use in Validate take. None for one too big to edit in
    /// a box, which is shown as it was made.
    edits: Option<Edits>,
    /// The pattern picked in the list, which is open under it.
    selected: Option<usize>,
    /// Where in the document's text the pattern that is open was found: its
    /// strings, or its keys. None when the document is not shown in its box.
    places: Vec<Range<usize>>,
    /// The one of them the document's box points out.
    place: usize,
    /// Use in Validate was pressed: the schema to check the document against, on
    /// the Validate page.
    handoff: Option<Arc<str>>,
}

/// The schema made, and what has been done to it.
struct Edits {
    /// The schema as it was made.
    made: Value,
    /// The schema as it is now, as text: the box shows it, and it is typed in.
    text: String,
    /// What each pattern of the list says in it now.
    now: Vec<Now>,
    /// It is not what was made: it was typed in, or a pattern was changed.
    changed: bool,
    /// Why the text is not JSON, when it is not.
    broken: Option<String>,
    /// The expression in the box of the pattern that is open, as typed.
    typed: String,
    /// Why the last change of a pattern could not be made.
    problem: Option<String>,
    /// Where the pattern that is open is in the text.
    mark: Option<Mark>,
}

impl Edits {
    /// The schema `created` made, as yet unchanged: if it is small enough for a
    /// box to edit.
    fn of(created: &Created) -> Option<Self> {
        if created.text.len() as u64 > TEXT_LIMIT {
            return None;
        }
        Some(Self {
            made: serde_json::from_str(&created.text).ok()?,
            text: created.text.to_string(),
            now: vec![Now::Found; created.found.len()],
            changed: false,
            broken: None,
            typed: String::new(),
            problem: None,
            mark: None,
        })
    }

    /// Point out the pattern `found` (row `row` of the list) where it is in the
    /// text now; with `reveal`, scroll to it.
    fn point_at(&mut self, row: usize, found: &Found, reveal: bool) {
        self.mark = infer::place_in_schema(found, &self.now[row], &self.text)
            .map(|range| Mark { range, reveal });
    }

    /// The text was typed in.
    fn typed_in(&mut self) {
        self.changed = true;
        self.broken = serde_json::from_str::<Value>(&self.text)
            .err()
            .map(|e| e.to_string());
    }

    /// Make the pattern `found` (row `row` of the list) say `to`: in the schema
    /// as it is now, which is written out again. Says why when it cannot be made.
    fn change(&mut self, row: usize, found: &Found, to: Now) {
        let Ok(mut schema) = serde_json::from_str::<Value>(&self.text) else {
            self.problem = Some("The schema is not JSON: put it right, or Reset".to_owned());
            return;
        };
        // The expression that was found, typed again, is what was found; so are
        // the values found, ticked again.
        let to = match to {
            Now::Expression(expression) if expression == found.expression => Now::Found,
            Now::Values(values) if values == found.values => Now::Found,
            to => to,
        };
        match infer::change(&mut schema, &self.made, found, &self.now[row], &to) {
            Ok(()) => {
                self.text = reformat::render(&schema, &reformat::Options::default());
                if !found.is_enum() {
                    self.typed = infer::expression_now(found, &to)
                        .unwrap_or_default()
                        .to_owned();
                } else if infer::values_now(found, &to)
                    .is_some_and(|values| values.contains(&Value::from(self.typed.as_str())))
                {
                    // It was put in.
                    self.typed.clear();
                }
                self.now[row] = to;
                self.changed = true;
                self.broken = None;
                self.problem = None;
                self.point_at(row, found, true);
            }
            Err(problem) => self.problem = Some(problem),
        }
    }
}

impl Default for Create {
    fn default() -> Self {
        Self {
            document: Operand::new(
                "Document",
                "create_document",
                "Paste the JSON to make a schema of, or drop a file",
            ),
            options: Options::default(),
            run: Run::default(),
            edits: None,
            selected: None,
            places: Vec::new(),
            place: 0,
            handoff: None,
        }
    }
}

impl Create {
    #[cfg(test)]
    pub(super) fn operand(&mut self, title: &str) -> Option<&mut Operand> {
        [&mut self.document]
            .into_iter()
            .find(|o| o.title() == title)
    }

    /// The box the document is in.
    pub(super) fn document(&self) -> &Operand {
        &self.document
    }

    /// The schema Use in Validate asked to check the document against, once.
    pub(super) fn take_handoff(&mut self) -> Option<Arc<str>> {
        self.handoff.take()
    }

    /// The schema as it is now, as text.
    #[cfg(test)]
    pub(super) fn schema_text(&self) -> Option<String> {
        self.current_text().map(|t| t.to_string())
    }

    /// The sample of a lesson of the tutorial in the box, the options it has;
    /// and, with `run`, its schema made.
    pub(super) fn use_sample(
        &mut self,
        document: &str,
        options: Options,
        run: bool,
    ) -> Option<Request> {
        self.document.set_text(document);
        self.options = options;
        self.changed();
        if run {
            self.start()
        } else {
            None
        }
    }

    /// The answer to a [`Request::Job`] for this page.
    pub(super) fn done(&mut self, gen: u64, result: Result<Created, String>) {
        if self.run.finish(gen, result) {
            self.unselect();
            self.edits = self.run.outcome().and_then(Edits::of);
        }
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        if env.own_input && deliver(&mut [&mut self.document], dropped_files(ui.ctx())) {
            self.changed();
        }

        let mut request = None;
        command_bar(ui, "create_command", |ui| request = self.command_ui(ui));
        let (left, right) = halves(ui, "create_panes", RIGHT_MIN);
        left.show(ui, |ui| self.input_pane(ui, env));
        right.show(ui, |ui| {
            if let Some(r) = self.result_pane(ui, env) {
                request = Some(r);
            }
        });
        request
    }

    /// What was made no longer matches what is on the page.
    fn changed(&mut self) {
        self.unselect();
        self.edits = None;
        self.run.clear();
    }

    /// The schema as it is now, as text: as it was changed, or as it was made.
    fn current_text(&self) -> Option<Arc<str>> {
        match &self.edits {
            Some(edits) => Some(Arc::from(edits.text.as_str())),
            None => self.run.outcome().map(|c| c.text.clone()),
        }
    }

    /// Open the pattern of row `row` under the list, with the expression it has
    /// now in the box, and point it out: in the schema, and in the document the
    /// first place it was found.
    fn select(&mut self, row: usize) {
        self.selected = Some(row);
        let Some(found) = self.run.outcome().and_then(|c| c.found.get(row)) else {
            return;
        };
        self.places = self
            .document
            .shown_text()
            .map(|text| infer::occurrences(found, text))
            .unwrap_or_default();
        self.place = 0;
        self.document.set_mark(self.places.first().cloned());
        if let Some(edits) = self.edits.as_mut() {
            edits.typed = infer::expression_now(found, &edits.now[row])
                .unwrap_or_default()
                .to_owned();
            edits.problem = None;
            edits.point_at(row, found, true);
        }
    }

    /// No pattern is open, or pointed out.
    fn unselect(&mut self) {
        self.selected = None;
        self.places.clear();
        self.place = 0;
        self.document.set_mark(None);
    }

    /// Point out the place `by` after the one that is (before, when less than
    /// none), round from the last to the first.
    fn go(&mut self, by: isize) {
        let count = self.places.len();
        if count == 0 {
            return;
        }
        self.place = (self.place as isize + by).rem_euclid(count as isize) as usize;
        self.document.set_mark(self.places.get(self.place).cloned());
    }

    /// The Create button and the options. Changing one drops an old schema.
    fn command_ui(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let busy = self.run.running();
        let mut request = None;
        ui.horizontal(|ui| {
            let ready = !self.document.is_empty();
            if run_button(
                ui,
                &self.run,
                "Create",
                ready,
                "Create a schema of the document (Ctrl+Enter)",
            ) {
                request = self.start();
            }
            ui.add_enabled_ui(!busy, |ui| self.options_ui(ui));
        });
        request
    }

    fn options_ui(&mut self, ui: &mut egui::Ui) {
        let before = self.options;
        let options = &mut self.options;
        egui::ComboBox::from_id_salt("create_draft")
            .selected_text(options.draft.name())
            .width(110.0)
            .show_ui(ui, |ui| {
                for draft in Draft::ALL {
                    ui.selectable_value(&mut options.draft, draft, draft.name());
                }
            })
            .response
            .on_hover_text("The draft of JSON Schema to write it for");
        ui.checkbox(&mut options.patterns, "Patterns")
            .on_hover_text(
                "Say what strings are — a date, a uuid, a list… — as format and pattern",
            );
        ui.checkbox(&mut options.required, "Required")
            .on_hover_text("List the members every object has as required");
        ui.checkbox(&mut options.closed, "No other keys")
            .on_hover_text("Allow no members but those seen (additionalProperties: false)");
        ui.checkbox(&mut options.enums, "Enums")
            .on_hover_text("A few strings that come again and again are an enum");
        ui.checkbox(&mut options.bounds, "Bounds")
            .on_hover_text("The least and the most seen: minimum, maxLength, maxItems…");
        if self.options != before {
            self.changed();
        }
    }

    fn input_pane(&mut self, ui: &mut egui::Ui, env: &Env) {
        ui.add_enabled_ui(!self.run.running(), |ui| {
            if self.document.ui(ui, env, true) {
                self.changed();
            }
        });
    }

    fn start(&mut self) -> Option<Request> {
        let document = self.document.input()?;
        self.unselect();
        self.edits = None;
        let (gen, cancel) = self.run.start();
        Some(Request::Job {
            tool: Tool::Create,
            job: Job::Create {
                document,
                options: self.options,
            },
            gen,
            cancel,
        })
    }

    /// The schema, with Use in Validate, Save…, Copy and Reset in its header —
    /// there from the start, usable once there is a schema — and under it the
    /// patterns found, and the one that is open.
    fn result_pane(&mut self, ui: &mut egui::Ui, env: &mut Env) -> Option<Request> {
        enum Act {
            Validate,
            Save,
            Copy,
            CopyList,
            Reset,
        }

        let text = self.current_text();
        let too_big_to_copy = text.as_ref().is_some_and(|t| t.len() > COPY_LIMIT);
        let too_big_for_a_box = text.as_ref().is_some_and(|t| t.len() as u64 > TEXT_LIMIT);
        let broken = self.edits.as_ref().and_then(|e| e.broken.clone());
        let edited = self.edits.as_ref().is_some_and(|e| e.changed);
        let mut act = None;

        // Besides the gap, egui puts its item spacing after the first part.
        let spaces = STACK_GAP + ui.spacing().item_spacing.y;
        let width = ui.available_width();
        let schema_height = ((ui.available_height() - spaces) * SCHEMA_SHARE)
            .floor()
            .max(0.0);
        ui.allocate_ui(egui::vec2(width, schema_height), |ui| {
            // Its share whatever it holds, so the patterns stay where they are.
            ui.set_min_size(egui::vec2(width, schema_height));
            header(
                ui,
                "Schema",
                |ui| {
                    if edited {
                        ui.weak("edited");
                    }
                },
                |ui| {
                    // Right to left: Use in Validate is the right-most.
                    let why_not = if too_big_for_a_box {
                        "Too big for the Schema box; save it instead"
                    } else if broken.is_some() {
                        "The schema is not JSON: put it right first"
                    } else {
                        "Create a schema first"
                    };
                    if ui
                        .add_enabled(
                            text.is_some() && !too_big_for_a_box && broken.is_none(),
                            egui::Button::new("Use in Validate"),
                        )
                        .on_hover_text("Check the document against the schema, on Validate Schema")
                        .on_disabled_hover_text(why_not)
                        .clicked()
                    {
                        act = Some(Act::Validate);
                    }
                    if ui
                        .add_enabled(text.is_some(), egui::Button::new("Save…"))
                        .on_hover_text("Write the schema to a file")
                        .clicked()
                    {
                        act = Some(Act::Save);
                    }
                    if ui
                        .add_enabled(
                            text.is_some() && !too_big_to_copy,
                            egui::Button::new("Copy"),
                        )
                        .on_hover_text("Copy the schema")
                        .on_disabled_hover_text(if too_big_to_copy {
                            "Too big to copy; save it instead"
                        } else {
                            "Nothing to copy yet"
                        })
                        .clicked()
                    {
                        act = Some(Act::Copy);
                    }
                    if ui
                        .add_enabled(edited, egui::Button::new("Reset"))
                        .on_hover_text("Undo every change: the schema as it was made")
                        .on_disabled_hover_text("It is as it was made")
                        .clicked()
                    {
                        act = Some(Act::Reset);
                    }
                },
            );
            let edits = &mut self.edits;
            let open = self.selected;
            result_area(
                ui,
                "create",
                &self.run,
                "The schema appears here",
                |ui, created| match edits {
                    Some(edits) => {
                        if let Some(broken) = &edits.broken {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("Not JSON: {broken}")).color(ERROR),
                                )
                                .truncate(),
                            );
                        }
                        let mark = edits.mark.as_mut();
                        if text_box(ui, "create_schema", &mut edits.text, "", None, false, mark) {
                            edits.typed_in();
                            // Still pointed out, where it is now (if it is).
                            if let Some((row, found)) =
                                open.and_then(|row| Some((row, created.found.get(row)?)))
                            {
                                edits.point_at(row, found, false);
                            }
                        }
                    }
                    None => preview_box(
                        ui,
                        "create",
                        &created.preview.text,
                        created.preview.truncated,
                    ),
                },
            );
        });
        ui.add_space(STACK_GAP);

        let size = egui::vec2(width, ui.available_height());
        let mut picked = None;
        let mut changed = None;
        let mut go = None;
        ui.allocate_ui(size, |ui| {
            let created = self.run.outcome();
            let found = created.map(|c| c.found.as_slice());
            let title = list_title(found);
            header(
                ui,
                &title,
                |_| {},
                |ui| {
                    if ui
                        .add_enabled(
                            found.is_some_and(|f| !f.is_empty()),
                            egui::Button::new("Copy list"),
                        )
                        .on_hover_text("Copy the patterns, one to a line")
                        .clicked()
                    {
                        act = Some(Act::CopyList);
                    }
                },
            );
            let Some(found) = found else {
                ui.weak("What the strings were found to be appears here");
                return;
            };
            if found.is_empty() {
                ui.weak("No dates, ids, addresses, codes or lists were found in the strings");
                return;
            }
            let now = self.edits.as_ref().map(|e| e.now.as_slice());
            let open = self.selected.filter(|_| self.edits.is_some());
            // The pattern that is open has its room at the bottom; the list the rest.
            let editor = open
                .and_then(|row| found.get(row))
                .map_or(0.0, |f| editor_height(ui, f));
            let list = egui::vec2(width, (ui.available_height() - editor).max(ROW_HEIGHT));
            ui.allocate_ui(list, |ui| {
                ui.set_min_size(list);
                picked = found_list(ui, found, now, self.selected);
            });
            if let (Some(row), Some(edits)) = (open, self.edits.as_mut()) {
                if let Some(found) = found.get(row) {
                    let place =
                        (!self.places.is_empty()).then_some((self.place, self.places.len()));
                    let (now, typed, problem) = (&edits.now[row], &mut edits.typed, &edits.problem);
                    let shown = ui.scope(|ui| {
                        if found.is_enum() {
                            enum_editor(ui, found, now, typed, problem, place)
                        } else {
                            (pattern_editor(ui, found, now, typed, problem, place), 0.0)
                        }
                    });
                    // The list makes room for it as it was laid out, and for what
                    // it had no room for.
                    let (asked, short) = shown.inner;
                    let used = shown.response.rect.height() + short + ui.spacing().item_spacing.y;
                    keep_editor_height(ui, found, used);
                    match asked {
                        Some(Ask::Change(to)) => changed = Some((row, to)),
                        Some(Ask::Go(by)) => go = Some(by),
                        None => {}
                    }
                }
            }
        });
        if let Some(row) = picked {
            self.select(row);
        }
        if let Some(by) = go {
            self.go(by);
        }
        if let Some((row, to)) = changed {
            if let (Some(created), Some(edits)) = (self.run.outcome(), self.edits.as_mut()) {
                if let Some(found) = created.found.get(row) {
                    edits.change(row, found, to);
                }
            }
        }

        let text = self.current_text()?;
        match act? {
            Act::Validate => {
                self.handoff = Some(text);
                None
            }
            Act::Copy => {
                ui.ctx().copy_text(text.to_string());
                env.shared
                    .say(Tool::Create, "Copied the schema to the clipboard", false);
                None
            }
            Act::CopyList => {
                let created = self.run.outcome()?;
                let now = self.edits.as_ref().map(|e| e.now.as_slice());
                ui.ctx().copy_text(list_text(&created.found, now));
                env.shared
                    .say(Tool::Create, "Copied the patterns to the clipboard", false);
                None
            }
            Act::Reset => {
                self.edits = self.run.outcome().and_then(Edits::of);
                if let Some(row) = self.selected {
                    self.select(row);
                }
                None
            }
            Act::Save => {
                let path = rfd::FileDialog::new()
                    .set_file_name(save_name(self.document.source_file()))
                    .add_filter("JSON", &["json"])
                    .save_file()?;
                Some(Request::SaveText { text, path })
            }
        }
    }

    /// What the status bar says about the schema: how many members it names and
    /// patterns it has, that the document fits it — or that it was changed since —
    /// and how long it took. False when there is nothing to say.
    pub(super) fn status_line(&self, ui: &mut egui::Ui) -> bool {
        let Some(created) = self.run.outcome() else {
            return false;
        };
        ui.label(summary(created));
        ui.separator();
        let (text, kind) = if self.edits.as_ref().is_some_and(|e| e.changed) {
            ("Edited — Use in Validate checks it".to_owned(), Tint::Warn)
        } else if created.problems == 0 {
            ("The document fits it".to_owned(), Tint::Good)
        } else {
            (
                format!(
                    "The document does not fit it: {} problems",
                    created.problems
                ),
                Tint::Bad,
            )
        };
        ui.colored_label(tint(ui.visuals(), kind), text);
        ui.separator();
        ui.weak(format!("Created in {:.1?}", created.elapsed));
        true
    }
}

/// "17 properties · 5 patterns", and " · 2 enums" when there are enums.
fn summary(created: &Created) -> String {
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let enums = created.found.iter().filter(|f| f.is_enum()).count();
    let mut text = format!(
        "{} · {}",
        plural(created.properties, "property", "properties"),
        plural(created.found.len() - enums, "pattern", "patterns")
    );
    if enums > 0 {
        text.push_str(&format!(" · {}", plural(enums, "enum", "enums")));
    }
    text
}

/// "Patterns (5)" over the list; "Patterns and enums (7)", "Enums (2)" when there
/// are enums.
fn list_title(found: Option<&[Found]>) -> String {
    let Some(found) = found else {
        return "Patterns".to_owned();
    };
    let enums = found.iter().filter(|f| f.is_enum()).count();
    let name = match (found.len() - enums, enums) {
        (_, 0) => "Patterns",
        (0, _) => "Enums",
        _ => "Patterns and enums",
    };
    format!("{name} ({})", found.len())
}

/// The file name a save of the schema suggests: the document's, as a schema.
fn save_name(from: Option<&str>) -> String {
    match from.and_then(|name| Path::new(name).file_stem()) {
        Some(stem) => format!("{}.schema.json", stem.to_string_lossy()),
        None => "schema.json".to_owned(),
    }
}

/// What a row of the list says of a pattern: what it is, and its format — or
/// that it was changed, and how. Of an enum: its values.
fn found_text(found: &Found, now: &Now) -> String {
    if found.is_enum() {
        return match infer::values_now(found, now) {
            Some(values) => {
                let values: Vec<String> = values.iter().map(value_text).collect();
                let edited = if *now == Now::Found { "" } else { " · edited" };
                format!("enum: {}{edited}", values.join(", "))
            }
            None => "string · was enum".to_owned(),
        };
    }
    match (now, &found.format) {
        (Now::Found, Some(format)) => format!("{} · format {format}", found.what),
        (Now::Found, None) => found.what.clone(),
        (Now::Expression(_) | Now::Values(_), _) => format!("{} · edited", found.what),
        (Now::Plain, _) if found.keys => format!("any key · was {}", found.what),
        (Now::Plain, _) => format!("string · was {}", found.what),
    }
}

/// A value of an enum as a row shows it: a string without its quotes.
fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// What the schema says of `found` when it says `now`: its expression, or the
/// values of the enum as JSON; none when it is plain.
fn said_now(found: &Found, now: &Now) -> Option<String> {
    if found.is_enum() {
        infer::values_now(found, now).map(|values| Value::from(values.to_vec()).to_string())
    } else {
        infer::expression_now(found, now).map(str::to_owned)
    }
}

/// The patterns as text, one to a line: where, what, the expression it has now
/// (`now`, as changed; as found when there is none) or an enum's values.
fn list_text(found: &[Found], now: Option<&[Now]>) -> String {
    found
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let now = now.and_then(|n| n.get(i)).unwrap_or(&Now::Found);
            let said = said_now(f, now).unwrap_or_else(|| "(none)".to_owned());
            format!("{}\t{}\t{said}\n", f.path, found_text(f, now))
        })
        .collect()
}

/// The patterns, a row each. Gives the row that was clicked.
fn found_list(
    ui: &mut egui::Ui,
    found: &[Found],
    now: Option<&[Now]>,
    selected: Option<usize>,
) -> Option<usize> {
    let paths = PathColumn::of(ui, found.iter().map(|f| f.path.as_str()));
    let mut clicked = None;
    // Rows touch, so their highlights do. (Set before the scroll area, which
    // takes its idea of how tall a row is from this.)
    ui.spacing_mut().item_spacing.y = 0.0;
    let room = ui.available_height();
    let shown = egui::ScrollArea::vertical()
        .id_salt("create_found")
        .auto_shrink([false, false])
        // A row, when what is open under it leaves no more.
        .min_scrolled_height(ROW_HEIGHT)
        .show_rows(ui, ROW_HEIGHT, found.len(), |ui, range| {
            for index in range {
                let now = now.and_then(|n| n.get(index)).unwrap_or(&Now::Found);
                if found_row(ui, &found[index], now, selected == Some(index), &paths).clicked() {
                    clicked = Some(index);
                }
            }
        });
    // What opens under the list takes room from it: the row picked stays in
    // view when the list is made shorter (or taller), and is not held there
    // while it is scrolled.
    let room_id = egui::Id::new("create_found_room");
    let resized = ui
        .data(|d| d.get_temp::<f32>(room_id))
        .is_some_and(|before| (before - room).abs() > 0.5);
    ui.data_mut(|d| d.insert_temp(room_id, room));
    if let Some(row) = selected.filter(|_| resized) {
        let mut state = shown.state;
        let (offset, height) = (state.offset.y, shown.inner_rect.height());
        let top = row as f32 * ROW_HEIGHT;
        let wanted = if top < offset {
            top
        } else {
            offset.max(top + ROW_HEIGHT - height)
        };
        if (wanted - offset).abs() > 0.5 {
            state.offset.y = wanted;
            state.store(ui.ctx(), shown.id);
            ui.ctx().request_repaint();
        }
    }
    clicked
}

/// One pattern: where, and what was found there. The expression is in the
/// tooltip, and the right-click menu copies it; a click opens it, to change.
fn found_row(
    ui: &mut egui::Ui,
    found: &Found,
    now: &Now,
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
    let what_rect = Rect::from_min_max(egui::pos2(path_rect.max.x + 12.0, inner.min.y), inner.max);
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
        egui::Label::new(RichText::new(&found.path).monospace())
            .truncate()
            .selectable(false),
    );
    let what = RichText::new(found_text(found, now));
    cell(ui, what_rect).add(
        egui::Label::new(if *now == Now::Found {
            what
        } else {
            what.italics()
        })
        .truncate()
        .selectable(false),
    );

    let of = if found.keys { "Keys of" } else { "At" };
    let (name, copy) = if found.is_enum() {
        ("Enum", "Copy enum")
    } else {
        ("Pattern", "Copy pattern")
    };
    let said = said_now(found, now);
    let response = response
        .on_hover_text(format!(
            "{}\n\n{of} {}\n{name}: {}{}\n\nClick to find it and change it",
            found.what,
            found.path,
            said.as_deref().unwrap_or("none"),
            found
                .format
                .as_ref()
                .filter(|_| *now != Now::Plain)
                .map_or(String::new(), |f| format!("\nFormat: {f}"))
        ))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.context_menu(|ui| {
        let ctx = ui.ctx().clone();
        if ui
            .add_enabled(said.is_some(), egui::Button::new(copy))
            .clicked()
        {
            ctx.copy_text(said.clone().unwrap_or_default());
            ui.close();
        }
        if ui.button("Copy path").clicked() {
            ctx.copy_text(found.path.clone());
            ui.close();
        }
        if ui.button("Copy row").clicked() {
            let row = list_text(std::slice::from_ref(found), Some(std::slice::from_ref(now)));
            ctx.copy_text(row.trim_end().to_owned());
            ui.close();
        }
    });
    response
}

/// The height the open pattern takes under the list — a line that says which,
/// the box of its expression, and its buttons; an enum's values too — as it was
/// the last time one of its kind was laid out (an enum's values wrap); before
/// then, a guess.
fn editor_height(ui: &egui::Ui, found: &Found) -> f32 {
    ui.data(|d| d.get_temp(editor_height_id(found)))
        .unwrap_or_else(|| {
            let row = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            let rows = if found.is_enum() { 4.0 } else { 3.0 };
            rows * row + STACK_GAP
        })
}

/// Keep the height `used` the open pattern took, for the next frame's list; one
/// more frame now when it changed.
fn keep_editor_height(ui: &egui::Ui, found: &Found, used: f32) {
    if (used - editor_height(ui, found)).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(editor_height_id(found), used));
        ui.ctx().request_repaint();
    }
}

fn editor_height_id(found: &Found) -> egui::Id {
    egui::Id::new(("create_editor_height", found.is_enum()))
}

/// The most rows an enum's values take; more scroll.
const VALUE_ROWS: f32 = 3.0;

/// What the pattern that is open asks for.
enum Ask {
    /// That it say this.
    Change(Now),
    /// The place in the document so many after the one pointed out (before, when
    /// less than none).
    Go(isize),
}

/// The first line of what is open: which it is, and which of the `place`s (the
/// one pointed out, of how many) in the document, with ⏶ and ⏷ to go to another.
fn editor_heading(
    ui: &mut egui::Ui,
    found: &Found,
    now: &Now,
    place: Option<(usize, usize)>,
) -> Option<Ask> {
    let mut asked = None;
    ui.add_space(STACK_GAP);
    let line = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(line, Layout::right_to_left(Align::Center), |ui| {
        if let Some((at, count)) = place {
            let more = count > 1;
            if ui
                .add_enabled(more, egui::Button::new("⏷"))
                .on_hover_text("Next place in the document")
                .clicked()
            {
                asked = Some(Ask::Go(1));
            }
            if ui
                .add_enabled(more, egui::Button::new("⏶"))
                .on_hover_text("Previous place in the document")
                .clicked()
            {
                asked = Some(Ask::Go(-1));
            }
            ui.label(format!("{} of {count}", at + 1))
                .on_hover_text("The place in the document that is highlighted");
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.weak(if found.keys {
                "Keys of"
            } else if found.is_enum() {
                "Enum of"
            } else {
                "Pattern of"
            });
            ui.add(egui::Label::new(RichText::new(&found.path).monospace()).truncate());
            ui.add(egui::Label::new(RichText::new(found_text(found, now)).weak()).truncate());
        });
    });
    asked
}

/// The pattern that is open: which it is (see [`editor_heading`]); its
/// expression in a box to change, and the buttons that put it in the schema,
/// make it plain, or put back the one found.
fn pattern_editor(
    ui: &mut egui::Ui,
    found: &Found,
    now: &Now,
    typed: &mut String,
    problem: &Option<String>,
    place: Option<(usize, usize)>,
) -> Option<Ask> {
    let mut asked = editor_heading(ui, found, now, place);
    let field = ui.add(
        egui::TextEdit::singleline(typed)
            .id_salt("create_pattern_edit")
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .hint_text("A regular expression: ^[0-9]{5}$"),
    );
    crate::text_menu::attach(&field, typed, crate::text_menu::Kind::Typed);
    let current = infer::expression_now(found, now);
    let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    // A plain pattern has an empty box: Apply waits for an expression.
    let differs = !typed.is_empty() && current != Some(typed.as_str());
    ui.horizontal(|ui| {
        if ui
            .add_enabled(differs, egui::Button::new("Apply"))
            .on_hover_text("Put this expression in the schema (Enter)")
            .on_disabled_hover_text(if typed.is_empty() {
                "Write an expression first"
            } else {
                "It is the expression the schema has"
            })
            .clicked()
            || (entered && differs)
        {
            asked = Some(Ask::Change(Now::Expression(typed.clone())));
        }
        let (plain, tip) = if found.keys {
            (
                "Any key",
                "Allow any key, with these keys' values (additionalProperties)",
            )
        } else {
            (
                "Plain string",
                "No pattern: a string of any kind (the format goes too)",
            )
        };
        if ui
            .add_enabled(*now != Now::Plain, egui::Button::new(plain))
            .on_hover_text(tip)
            .on_disabled_hover_text("It is plain already")
            .clicked()
        {
            asked = Some(Ask::Change(Now::Plain));
        }
        if ui
            .add_enabled(*now != Now::Found, egui::Button::new("Restore"))
            .on_hover_text("Put back the pattern that was found")
            .on_disabled_hover_text("It is the pattern that was found")
            .clicked()
        {
            asked = Some(Ask::Change(Now::Found));
        }
        if let Some(problem) = problem {
            ui.add(egui::Label::new(RichText::new(problem).color(ERROR)).truncate())
                .on_hover_text(problem);
        }
    });
    asked
}

/// The enum that is open: which it is (see [`editor_heading`]); its values, the
/// ones found and the ones put in, each with a tick to keep it or leave it out;
/// a box to put in another; and the buttons that make it a plain string or put
/// back the values found. The values have [`VALUE_ROWS`] rows at most, fewer
/// when the room under the list is short, and scroll; how much taller they
/// would be is given with what is asked.
fn enum_editor(
    ui: &mut egui::Ui,
    found: &Found,
    now: &Now,
    typed: &mut String,
    problem: &Option<String>,
    place: Option<(usize, usize)>,
) -> (Option<Ask>, f32) {
    let mut asked = editor_heading(ui, found, now, place);
    let current = infer::values_now(found, now).unwrap_or_default();
    let put_in = current.iter().filter(|v| !found.values.contains(v));
    let shown: Vec<&Value> = found.values.iter().chain(put_in).collect();
    let spacing = ui.spacing();
    let row = spacing.interact_size.y;
    let most = VALUE_ROWS * row + (VALUE_ROWS - 1.0) * spacing.item_spacing.y;
    // The box and the buttons under the values, as high as they were.
    let below_id = egui::Id::new("create_enum_below");
    let below = ui
        .data(|d| d.get_temp(below_id))
        .unwrap_or(2.0 * (row + spacing.item_spacing.y));
    let room = (ui.available_height() - below).clamp(row, most);
    let values = egui::ScrollArea::vertical()
        .id_salt("create_enum_values")
        .max_height(room)
        .min_scrolled_height(room)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for value in shown {
                    let mut kept = current.contains(value);
                    let tip = match found.values.iter().position(|v| v == value) {
                        Some(i) if found.times[i] == 1 => "Once in the document".to_owned(),
                        Some(i) => format!("{} times in the document", found.times[i]),
                        None => "Put in: not in the document".to_owned(),
                    };
                    let text = RichText::new(value_text(value)).monospace();
                    if ui.checkbox(&mut kept, text).on_hover_text(tip).changed() {
                        let values = toggled(found, current, value);
                        asked = Some(Ask::Change(Now::Values(values)));
                    }
                }
            });
        });
    let short = (values.content_size.y.min(most) - values.inner_rect.height()).max(0.0);

    let top = ui.cursor().min.y;
    let field = ui.add(
        egui::TextEdit::singleline(typed)
            .id_salt("create_enum_add")
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .hint_text("Another value"),
    );
    crate::text_menu::attach(&field, typed, crate::text_menu::Kind::Typed);
    let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let can = !typed.is_empty();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can, egui::Button::new("Add"))
            .on_hover_text("Put this value in the enum (Enter)")
            .on_disabled_hover_text("Write a value first")
            .clicked()
            || (entered && can)
        {
            let mut values = current.to_vec();
            values.push(Value::from(typed.as_str()));
            asked = Some(Ask::Change(Now::Values(values)));
        }
        if entered {
            // The box stays open for the next one.
            field.request_focus();
        }
        if ui
            .add_enabled(*now != Now::Plain, egui::Button::new("Plain string"))
            .on_hover_text("No enum: a string of any kind")
            .on_disabled_hover_text("It is plain already")
            .clicked()
        {
            asked = Some(Ask::Change(Now::Plain));
        }
        if ui
            .add_enabled(*now != Now::Found, egui::Button::new("Restore"))
            .on_hover_text("Put back the values that were found")
            .on_disabled_hover_text("They are the values that were found")
            .clicked()
        {
            asked = Some(Ask::Change(Now::Found));
        }
        if let Some(problem) = problem {
            ui.add(egui::Label::new(RichText::new(problem).color(ERROR)).truncate())
                .on_hover_text(problem);
        }
    });
    let height = ui.min_rect().max.y - top;
    if (height - below).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(below_id, height));
        ui.ctx().request_repaint();
    }
    (asked, short)
}

/// The values of an enum that has `current` when `value` is ticked or
/// unticked: those found in their order, then those put in.
fn toggled(found: &Found, current: &[Value], value: &Value) -> Vec<Value> {
    if current.contains(value) {
        return current.iter().filter(|v| *v != value).cloned().collect();
    }
    let put_in = current.iter().filter(|v| !found.values.contains(v));
    found
        .values
        .iter()
        .filter(|v| current.contains(v) || *v == value)
        .chain(put_in)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_is_named_after_the_document() {
        assert_eq!(save_name(Some("orders.json")), "orders.schema.json");
        assert_eq!(save_name(Some("data")), "data.schema.json");
        assert_eq!(save_name(None), "schema.json");
    }

    #[test]
    fn a_document_is_needed() {
        let mut create = Create::default();
        assert!(create.start().is_none());
        create.document.set_text("{}");
        assert!(matches!(
            create.start(),
            Some(Request::Job {
                tool: Tool::Create,
                gen: 1,
                ..
            })
        ));
        assert!(create.run.running());
    }

    #[test]
    fn the_list_is_one_pattern_to_a_line_with_its_format() {
        let made = jsonquery_query::infer::create(
            &serde_json::json!({"id": "123e4567-e89b-12d3-a456-426614174000", "zip": "90210"}),
            Options::default(),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            list_text(&made.found, None),
            ".id\tuuid · format uuid\t^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\n\
             .zip\tinteger as text\t^[0-9]+$\n"
        );
        // As changed: an expression of one's own, and one made plain.
        let now = [Now::Expression("^[0-9a-f-]+$".to_owned()), Now::Plain];
        assert_eq!(
            list_text(&made.found, Some(&now)),
            ".id\tuuid · edited\t^[0-9a-f-]+$\n.zip\tstring · was integer as text\t(none)\n"
        );
    }
}
