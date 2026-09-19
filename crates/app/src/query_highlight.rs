//! Colour-codes the query box the way the tutorial colours its examples: each
//! step of the query sits on its own tinted chip and the glue between steps
//! (pipes, commas, operators) stays plain. Where the tutorial's fragments are
//! hand-written, these come from `jsonquery_query::highlight`, which works on
//! whatever is typed so far — including half-finished queries.
//!
//! The palette lives here because both windows draw from it, so a chip's
//! colour means the same thing in the tutorial and in the query box.

use std::sync::Arc;

use eframe::egui::{self, text::LayoutJob, text::TextFormat, Color32};
use jsonquery_query::{highlight, Kind};

/// Tints for the chips, in order and then repeating. Deliberately few and well
/// separated: the first is regex101's orange, the second its blue.
const PALETTE: [(u8, u8, u8); 6] = [
    (255, 145, 40),
    (70, 150, 255),
    (70, 200, 120),
    (190, 120, 255),
    (255, 105, 150),
    (40, 195, 200),
];

/// Past this many bytes the query is drawn plain. Each chip is its own text
/// section in egui's layout, so a pasted megabyte of query would cost far more
/// per frame than the one section it used to be — and nobody reads colours in
/// text that long.
const MAX_HIGHLIGHTED_BYTES: usize = 20_000;

/// The background for chip `index`; `lit` brightens it (the tutorial does so
/// for the fragment whose explanation is hovered).
pub fn part_color(index: usize, dark: bool, lit: bool) -> Color32 {
    let (r, g, b) = PALETTE[index % PALETTE.len()];
    let alpha = match (dark, lit) {
        (true, false) => 85,
        (true, true) => 175,
        (false, false) => 80,
        (false, true) => 160,
    };
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

/// The query box's text laid out with its chips tinted, for
/// `TextEdit::layouter`. `engine` is the picker's choice; with none picked the
/// dialect is guessed from the text, as it is when the query runs.
///
/// Everything but the backgrounds copies `TextEdit`'s own default layout —
/// monospace font, its text colour, line height, kept trailing whitespace — so
/// the box looks and measures exactly as it did before it was coloured.
pub fn galley(
    ui: &egui::Ui,
    engine: Option<Kind>,
    text: &str,
    wrap_width: f32,
) -> Arc<egui::Galley> {
    let kind = engine.unwrap_or_else(|| Kind::detect(text));
    let dark = ui.visuals().dark_mode;
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.fonts_mut(|f| f.row_height(&font_id));
    let base = TextFormat {
        line_height: Some(row_height + ui.spacing().extra_text_line_spacing),
        color: ui
            .visuals()
            .override_text_color
            .unwrap_or_else(|| ui.visuals().widgets.inactive.text_color()),
        font_id,
        ..Default::default()
    };

    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    job.keep_trailing_whitespace = true;
    let pieces = if text.len() <= MAX_HIGHLIGHTED_BYTES {
        highlight::segments(kind, text)
    } else {
        vec![(text, None)]
    };
    for (piece, chip) in pieces {
        let mut format = base.clone();
        if let Some(i) = chip {
            format.background = part_color(i, dark, false);
        }
        job.append(piece, 0.0, format);
    }
    ui.fonts_mut(|f| f.layout_job(job))
}
