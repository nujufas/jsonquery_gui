//! The tutorial window: a second native window (an egui *viewport*) with one
//! tab per query dialect, a topic tree on the left and the selected lesson on
//! the right. The lessons themselves are data in `jsonquery_query::tutorial`
//! (and unit-tested there against the real engines); this file is only
//! presentation plus the hand-off that lets a reader push an example's data
//! and/or query into the main window to practise with.
//!
//! It is an *immediate* viewport, so it runs inside the main window's frame
//! with plain `&mut self` access — no shared-state plumbing — and egui falls
//! back to an embedded floating window by itself if the backend can't open
//! native ones.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, text::LayoutJob, text::TextFormat, Color32, RichText, TextWrapMode};
use jsonquery_query::tutorial::{self as content, Example, Lesson, Outcome, Span};
use jsonquery_query::Kind;

use crate::query_highlight::part_color;

/// How long the "Loaded in the main window" confirmation stays in the
/// tutorial's status bar.
const NOTICE_FOR: Duration = Duration::from_secs(5);

/// What the reader asked to push into the main window. The window applies
/// it (`App::apply_tutorial_request`): pinning the engine to `kind`,
/// replacing the source document and/or the query text, and optionally
/// running the query once the data has loaded.
pub struct LoadRequest {
    pub kind: Kind,
    pub data: Option<&'static str>,
    pub query: Option<&'static str>,
    pub run: bool,
}

pub struct Tutorial {
    open: bool,
    tab: Kind,
    /// Selected (topic, lesson) for each tab, indexed like `Kind::ALL`, so
    /// switching tabs returns to where the reader left off.
    selected: [(usize, usize); 4],
    filter: String,
    /// Live results, computed once per (dialect, data, query) — the sample
    /// documents are tiny, but there's no reason to recompile every frame.
    outcomes: HashMap<(Kind, &'static str, &'static str), Outcome>,
    notice: Option<(String, Instant)>,
    /// Scroll the lesson pane back to the top on the next frame (a new
    /// lesson or tab was picked).
    scroll_to_top: bool,
    /// A topic to force open in the tree for one frame (Next/Previous moved
    /// into it), so the selected lesson is never hidden inside a collapsed
    /// topic.
    reveal: Option<(Kind, usize)>,
    /// The Explanation row under the pointer, as (example index, part
    /// index): its fragment is drawn brighter in the query. Read a frame
    /// late, which is imperceptible.
    hovered: Option<(usize, usize)>,
    icon: Option<Arc<egui::IconData>>,
}

impl Default for Tutorial {
    fn default() -> Self {
        Self {
            open: false,
            tab: Kind::Jq,
            selected: [(0, 0); 4],
            filter: String::new(),
            outcomes: HashMap::new(),
            notice: None,
            scroll_to_top: false,
            reveal: None,
            hovered: None,
            icon: None,
        }
    }
}

fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("jsonquery_tutorial_window")
}

fn tab_index(kind: Kind) -> usize {
    Kind::ALL.iter().position(|k| *k == kind).unwrap_or(0)
}

impl Tutorial {
    /// Open the window, or bring it to the front if it's already open.
    pub fn open_or_focus(&mut self, ctx: &egui::Context) {
        if self.open {
            ctx.send_viewport_cmd_to(viewport_id(), egui::ViewportCommand::Focus);
        } else {
            self.open = true;
        }
    }

    /// Draw the window (if open) for this frame. Returns what the reader
    /// asked to load into the main window, if anything — already focused,
    /// so the result of pressing ▶ is visible straight away.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<LoadRequest> {
        if !self.open {
            return None;
        }

        let icon = self
            .icon
            .get_or_insert_with(|| Arc::new(crate::app_icon()))
            .clone();
        let builder = egui::ViewportBuilder::default()
            .with_title("jsonquery — Tutorial")
            .with_inner_size([1040.0, 720.0])
            .with_min_inner_size([720.0, 460.0])
            .with_app_id(crate::APP_ID)
            .with_icon(icon);

        let mut request = None;
        let mut close = false;
        ctx.show_viewport_immediate(viewport_id(), builder, |ui, _class| {
            close = ui.ctx().input(|i| i.viewport().close_requested());
            request = self.contents(ui);
        });

        if close {
            self.open = false;
        }
        if request.is_some() {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
        request
    }

    fn contents(&mut self, ui: &mut egui::Ui) -> Option<LoadRequest> {
        let mut request = None;

        egui::Panel::top("tutorial_tabs").show(ui, |ui| self.tab_bar(ui));
        egui::Panel::bottom("tutorial_status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("tutorial_topics")
            .resizable(true)
            .default_size(270.0)
            .min_size(190.0)
            .show(ui, |ui| self.topic_tree(ui));
        egui::CentralPanel::default().show(ui, |ui| request = self.lesson_view(ui));

        if let Some(r) = &request {
            let what = match (r.data.is_some(), r.query.is_some(), r.run) {
                (true, true, true) => "Loaded the data and query into the main window and ran it.",
                (true, true, false) => "Loaded the data and query into the main window.",
                (true, false, _) => "Loaded the sample data into the main window.",
                (false, _, _) => "Loaded the query into the main window.",
            };
            self.notice = Some((what.to_owned(), Instant::now()));
        }
        request
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(14.0, 6.0);
            for kind in Kind::ALL {
                let text = RichText::new(content::title(kind)).size(15.0);
                if ui.selectable_label(self.tab == kind, text).clicked() && self.tab != kind {
                    self.tab = kind;
                    self.scroll_to_top = true;
                }
            }
        });
        ui.add_space(2.0);
        ui.weak(content::tagline(self.tab));
        ui.add_space(6.0);
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        self.notice = self
            .notice
            .take()
            .filter(|(_, at)| at.elapsed() < NOTICE_FOR);
        match &self.notice {
            Some((text, _)) => {
                ui.colored_label(ui.visuals().hyperlink_color, text);
                ui.ctx().request_repaint_after(NOTICE_FOR);
            }
            None => {
                ui.weak(
                    "▶ Try it loads the sample data and the query into the main window and runs \
                     it — then edit either one to experiment.",
                );
            }
        }
    }

    fn topic_tree(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter lessons…")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);

        let needle = self.filter.trim().to_lowercase();
        let kind = self.tab;
        let slot = tab_index(kind);
        let reveal = self.reveal.take();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut any = false;
                for (t, topic) in content::topics(kind).iter().enumerate() {
                    let matching: Vec<usize> = topic
                        .lessons
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| content::lesson_matches(l, &needle))
                        .map(|(i, _)| i)
                        .collect();
                    if matching.is_empty() {
                        continue;
                    }
                    any = true;

                    // Filtering shows every match, so force topics open while
                    // it's active; otherwise only the topic holding the
                    // selected lesson starts open.
                    let force_open = !needle.is_empty() || reveal == Some((kind, t));
                    egui::CollapsingHeader::new(RichText::new(topic.title).strong())
                        .id_salt(("tutorial_topic", kind, t))
                        .default_open(self.selected[slot].0 == t)
                        .open(force_open.then_some(true))
                        .show(ui, |ui| {
                            for l in matching {
                                let picked = self.selected[slot] == (t, l);
                                if ui
                                    .selectable_label(picked, topic.lessons[l].title)
                                    .clicked()
                                {
                                    self.selected[slot] = (t, l);
                                    self.scroll_to_top = true;
                                }
                            }
                        });
                }
                if !any {
                    ui.weak("No lessons match.");
                }
            });
    }

    fn lesson_view(&mut self, ui: &mut egui::Ui) -> Option<LoadRequest> {
        let kind = self.tab;
        let slot = tab_index(kind);
        let topics = content::topics(kind);
        let (t, l) = self.selected[slot];
        let (t, l) = if topics.get(t).is_some_and(|topic| l < topic.lessons.len()) {
            (t, l)
        } else {
            (0, 0)
        };
        let topic = &topics[t];
        let lesson = &topic.lessons[l];

        let hovered_before = self.hovered.take();
        let mut request = None;
        let mut go_to = None;

        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("tutorial_lesson", kind))
            .auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_to_top) {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| {
            ui.add_space(4.0);
            ui.weak(format!("{} › {}", content::title(kind), topic.title));
            ui.heading(lesson.title);
            ui.add_space(4.0);
            prose(ui, lesson.summary);
            ui.add_space(10.0);

            // One sample document per lesson when its examples share one.
            let shared_data = lesson
                .examples
                .first()
                .map(|e| e.data)
                .filter(|d| lesson.examples.iter().all(|e| e.data == *d));
            if let Some(data) = shared_data {
                data_box(ui, "Sample data", data, ("data", kind, t, l));
                ui.add_space(8.0);
            }

            for (i, example) in lesson.examples.iter().enumerate() {
                let key = (kind, t, l, i);
                let card = self.example_card(
                    ui,
                    kind,
                    example,
                    key,
                    shared_data.is_none(),
                    hovered_before,
                );
                request = request.take().or(card);
                ui.add_space(10.0);
            }

            if let Some(sheet) = lesson.cheat_sheet {
                data_box(ui, "Sample data", sheet.data, ("data", kind, t, l));
                ui.add_space(8.0);
                let picked = cheat_sheet(ui, kind, sheet.data, sheet.rows);
                request = request.take().or(picked);
                ui.add_space(10.0);
            }

            if !lesson.tips.is_empty() {
                tips(ui, lesson);
                ui.add_space(10.0);
            }

            ui.separator();
            go_to = self.pager(ui, kind, (t, l));
            ui.add_space(8.0);
        });

        if let Some((nt, nl)) = go_to {
            self.selected[slot] = (nt, nl);
            self.reveal = Some((kind, nt));
            self.scroll_to_top = true;
        }
        if self.hovered != hovered_before {
            ui.ctx().request_repaint();
        }
        request
    }

    /// One example: caption, the query with its explained fragments
    /// highlighted, the load buttons, the live result, and the fragment-by-
    /// fragment explanation.
    fn example_card(
        &mut self,
        ui: &mut egui::Ui,
        kind: Kind,
        example: &'static Example,
        key: (Kind, usize, usize, usize),
        show_own_data: bool,
        hovered_before: Option<(usize, usize)>,
    ) -> Option<LoadRequest> {
        let mut request = None;
        let example_idx = key.3;
        let lit_part = hovered_before.and_then(|(e, p)| (e == example_idx).then_some(p));
        let outcome = self
            .outcomes
            .entry((kind, example.data, example.query))
            .or_insert_with(|| content::run(kind, example.data, example.query))
            .clone();

        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(example.caption).strong());
                ui.add_space(4.0);

                code_frame(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(query_job(ui, example, lit_part))
                            .wrap_mode(TextWrapMode::Wrap),
                    );
                });

                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    // The one primary action: filled with the selection colour
                    // the active tab uses, so it reads as "the" button.
                    let try_it = egui::Button::new(RichText::new("▶ Try it").strong())
                        .fill(ui.visuals().selection.bg_fill);
                    if ui
                        .add(try_it)
                        .on_hover_text("Load this data and query into the main window and run it.")
                        .clicked()
                    {
                        request = Some(LoadRequest {
                            kind,
                            data: Some(example.data),
                            query: Some(example.query),
                            run: true,
                        });
                    }
                    if ui
                        .button("Load query")
                        .on_hover_text(
                            "Put this query in the main window's query box and select the \
                             engine, keeping whatever data is open.",
                        )
                        .clicked()
                    {
                        request = Some(LoadRequest {
                            kind,
                            data: None,
                            query: Some(example.query),
                            run: false,
                        });
                    }
                    if ui
                        .button("Load data")
                        .on_hover_text("Open this sample document in the main window.")
                        .clicked()
                    {
                        request = Some(LoadRequest {
                            kind,
                            data: Some(example.data),
                            query: None,
                            run: false,
                        });
                    }
                    if ui
                        .button("Copy query")
                        .on_hover_text("Copy the query text to the clipboard.")
                        .clicked()
                    {
                        ui.ctx().copy_text(example.query.to_owned());
                        self.notice = Some(("Copied the query.".to_owned(), Instant::now()));
                    }
                });

                if show_own_data {
                    ui.add_space(4.0);
                    data_box(
                        ui,
                        "Sample data",
                        example.data,
                        ("data", kind, key.1, key.2, key.3),
                    );
                }

                ui.add_space(6.0);
                result_box(ui, example, &outcome);

                ui.add_space(6.0);
                egui::CollapsingHeader::new(RichText::new("Explanation").strong())
                    .id_salt(("explanation", key))
                    .default_open(true)
                    .show(ui, |ui| {
                        self.explanation(ui, example, example_idx);
                    });
            });

        request
    }

    /// One row per explained fragment: its tinted chip, then what it does.
    /// Laid out as fixed-width rows rather than an `egui::Grid`: a grid feeds
    /// each frame's column widths back into the next frame's wrapping, and
    /// with wrapping labels in its cells that settles into wrong row heights.
    fn explanation(&mut self, ui: &mut egui::Ui, example: &Example, example_idx: usize) {
        let dark = ui.visuals().dark_mode;
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        let natural = example
            .parts
            .iter()
            .map(|(fragment, _)| text_width(ui, fragment, &mono))
            .fold(0.0, f32::max);
        // Wide enough for the longest fragment, but never more than ~40% of
        // the row, so a long fragment wraps instead of starving the note.
        let chip_width = (natural + 6.0).min((ui.available_width() * 0.4).max(150.0));

        for (i, (fragment, note)) in example.parts.iter().enumerate() {
            let lit = self.hovered == Some((example_idx, i));
            let row = ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(chip_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(chip_width);
                        ui.add(
                            egui::Label::new(
                                RichText::new(*fragment)
                                    .monospace()
                                    .background_color(part_color(i, dark, lit)),
                            )
                            .wrap_mode(TextWrapMode::Wrap),
                        );
                    },
                );
                prose(ui, note);
            });
            if ui.rect_contains_pointer(row.response.rect) {
                self.hovered = Some((example_idx, i));
            }
            ui.add_space(2.0);
        }
    }

    /// Previous / Next lesson buttons, walking the tab's lessons in order
    /// across topic boundaries. Returns the lesson to switch to.
    fn pager(
        &self,
        ui: &mut egui::Ui,
        kind: Kind,
        current: (usize, usize),
    ) -> Option<(usize, usize)> {
        let order: Vec<(usize, usize)> = content::topics(kind)
            .iter()
            .enumerate()
            .flat_map(|(t, topic)| (0..topic.lessons.len()).map(move |l| (t, l)))
            .collect();
        let pos = order.iter().position(|p| *p == current)?;
        let title_of = |(t, l): (usize, usize)| content::topics(kind)[t].lessons[l].title;

        let mut go_to = None;
        ui.horizontal(|ui| {
            if let Some(prev) = pos.checked_sub(1).map(|p| order[p]) {
                let label = with_arrows(ui, &format!("← {}", title_of(prev)));
                if ui.button(label).clicked() {
                    go_to = Some(prev);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(next) = order.get(pos + 1).copied() {
                    let label = with_arrows(ui, &format!("{} →", title_of(next)));
                    if ui.button(label).clicked() {
                        go_to = Some(next);
                    }
                }
            });
        });
        go_to
    }
}

/// The query as one wrapped line of text, each explained fragment on its own
/// tinted background (brighter for the one whose explanation is hovered).
fn query_job(ui: &egui::Ui, example: &Example, lit_part: Option<usize>) -> LayoutJob {
    let dark = ui.visuals().dark_mode;
    let font = egui::FontId::monospace(14.5);
    let mut job = LayoutJob::default();
    for (text, part) in content::segments(example.query, example.parts) {
        let mut format = TextFormat {
            font_id: font.clone(),
            color: ui.visuals().text_color(),
            ..Default::default()
        };
        if let Some(i) = part {
            format.background = part_color(i, dark, lit_part == Some(i));
        }
        job.append(text, 0.0, format);
    }
    job
}

/// The dark inset panel used for queries, data and results.
fn code_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(10, 8))
}

/// A collapsible, scrollable, read-only view of a sample document.
fn data_box(
    ui: &mut egui::Ui,
    title: &str,
    data: &str,
    id: impl std::hash::Hash + std::fmt::Debug,
) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt(id)
        .default_open(true)
        .show(ui, |ui| {
            code_frame(ui).show(ui, |ui| {
                egui::ScrollArea::both()
                    .max_height(210.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(data).monospace())
                                .wrap_mode(TextWrapMode::Extend),
                        );
                    });
            });
        });
}

/// The live result of running the example, or the error it produced.
fn result_box(ui: &mut egui::Ui, example: &Example, outcome: &Outcome) {
    let heading = match (&outcome.error, outcome.items.len()) {
        (Some(_), _) if example.fails => "Result — an error, on purpose".to_owned(),
        (Some(_), _) => "Result — error".to_owned(),
        (None, 1) => "Result — 1 result".to_owned(),
        (None, n) => format!("Result — {n} results"),
    };
    ui.label(RichText::new(heading).strong());
    code_frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if let Some(error) = &outcome.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
        if outcome.items.is_empty() {
            ui.weak("Nothing matched — an empty list, not an error.");
        }
        for value in &outcome.items {
            ui.add(
                egui::Label::new(RichText::new(content::format_value(value)).monospace())
                    .wrap_mode(TextWrapMode::Wrap),
            );
        }
        for error in &outcome.item_errors {
            ui.colored_label(ui.visuals().warn_fg_color, error);
        }
    });
}

/// "Good to know" bullets.
fn tips(ui: &mut egui::Ui, lesson: &Lesson) {
    ui.label(RichText::new("Good to know").strong());
    for tip in lesson.tips {
        ui.horizontal_top(|ui| {
            ui.label("•");
            prose(ui, tip);
        });
    }
}

/// The syntax-at-a-glance table, each row loadable with ▶. Fixed-width,
/// manually striped rows for the same reason as `Tutorial::explanation`.
fn cheat_sheet(
    ui: &mut egui::Ui,
    kind: Kind,
    data: &'static str,
    rows: &'static [(&'static str, &'static str)],
) -> Option<LoadRequest> {
    let mut request = None;
    let query_width = (ui.available_width() * 0.5).max(220.0);
    for (i, (query, meaning)) in rows.iter().enumerate() {
        let stripe = if i % 2 == 0 {
            ui.visuals().faint_bg_color
        } else {
            Color32::TRANSPARENT
        };
        egui::Frame::new()
            .fill(stripe)
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    if ui
                        .small_button("▶")
                        .on_hover_text(
                            "Load the sample data and this query into the main window and run it.",
                        )
                        .clicked()
                    {
                        request = Some(LoadRequest {
                            kind,
                            data: Some(data),
                            query: Some(*query),
                            run: true,
                        });
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(query_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(query_width);
                            ui.add(
                                egui::Label::new(RichText::new(*query).monospace())
                                    .wrap_mode(TextWrapMode::Wrap),
                            );
                        },
                    );
                    prose(ui, meaning);
                });
            });
    }
    request
}

/// The width `text` takes on one line in `font`.
fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x
}

/// Arrows are used freely in the lesson text, but egui's bundled proportional
/// font has no glyph for them (they render as empty boxes) while its
/// monospace font does — so they are drawn in that one.
const MONO_ONLY_GLYPHS: [char; 2] = ['→', '←'];

/// Appends `text` to `job` in `base`'s format, switching to the monospace
/// font for each of [`MONO_ONLY_GLYPHS`].
fn push_text(job: &mut LayoutJob, text: &str, base: &TextFormat, mono: &egui::FontId) {
    let mut run_start = 0;
    for (i, ch) in text.char_indices() {
        if !MONO_ONLY_GLYPHS.contains(&ch) {
            continue;
        }
        if run_start < i {
            job.append(&text[run_start..i], 0.0, base.clone());
        }
        let glyph = TextFormat {
            font_id: egui::FontId::monospace(mono.size),
            ..base.clone()
        };
        job.append(&text[i..i + ch.len_utf8()], 0.0, glyph);
        run_start = i + ch.len_utf8();
    }
    if run_start < text.len() {
        job.append(&text[run_start..], 0.0, base.clone());
    }
}

/// A button/label caption in the body font with real arrows (see
/// [`MONO_ONLY_GLYPHS`]); `Color32::PLACEHOLDER` lets the widget colour it.
fn with_arrows(ui: &egui::Ui, text: &str) -> egui::WidgetText {
    let body = egui::TextStyle::Button.resolve(ui.style());
    let base = TextFormat {
        font_id: body.clone(),
        color: Color32::PLACEHOLDER,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    push_text(&mut job, text, &base, &body);
    job.into()
}

/// Wrapped prose with `inline code` and *emphasis* (see
/// `content::parse_inline`).
fn prose(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let mono = egui::FontId::monospace(body.size * 0.95);
    let code_bg = ui.visuals().code_bg_color;
    let plain = TextFormat {
        font_id: body.clone(),
        color: Color32::PLACEHOLDER,
        ..Default::default()
    };

    let mut job = LayoutJob::default();
    for span in content::parse_inline(text) {
        match span {
            Span::Text(s) => push_text(&mut job, s, &plain, &body),
            Span::Emph(s) => {
                let italic = TextFormat {
                    italics: true,
                    ..plain.clone()
                };
                push_text(&mut job, s, &italic, &body);
            }
            Span::Code(s) => {
                let code = TextFormat {
                    font_id: mono.clone(),
                    background: code_bg,
                    ..plain.clone()
                };
                job.append(s, 0.0, code);
            }
        }
    }
    ui.add(egui::Label::new(job).wrap_mode(TextWrapMode::Wrap))
}
