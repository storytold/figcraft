//! Names in right-to-left and mixed-direction scripts (Hebrew, Arabic…) in the panels: layer, page,
//! style, component and file names. egui 0.37's epaint shapes the logical (typing-order) text with
//! the Unicode bidirectional algorithm (UAX #9), lays every run out in visual order, joins Arabic
//! letters through the font's OpenType features, mirrors brackets, and keeps one glyph per char so
//! that caret, selection and click offsets stay logical. Names are therefore passed to egui in
//! logical order and elided at their visual end. The rename editor is egui's own single-line
//! `TextEdit`, whose arrow keys move the caret on screen while its state remains a logical range.

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, FontId, Galley, Id, Painter, Pos2, Rect, Response, Ui, pos2, vec2};

/// `name` on one line in `font`, at most `max_w` wide (ending in an ellipsis when cut), ready to
/// paint. The text stays in logical order; egui shapes, joins and reorders it, and puts the
/// ellipsis at the row's visual end (the left end of a right-to-left name).
pub fn name_galley(painter: &Painter, name: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let max_w = max_w.max(1.0);
    let mut job = LayoutJob::simple_singleline(name.to_string(), font, color);
    job.wrap = TextWrapping::truncate_at_width(max_w);
    painter.layout_job(job)
}

/// Paint `name` (see [`name_galley`]) with its left end at `left_center`'s x, centred on its y;
/// returns the painted rectangle.
pub fn paint_name(painter: &Painter, left_center: Pos2, name: &str, font: FontId, color: Color32, max_w: f32) -> Rect {
    let galley = name_galley(painter, name, font, color, max_w);
    let rect = Rect::from_min_size(pos2(left_center.x, left_center.y - galley.size().y / 2.0), galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

/// A frameless one-line text editor for a name (a [`egui::TextEdit`] with id `id`). The editor's
/// text is logical; egui's galley paints it in visual order, and its caret, selection, clicks,
/// drags and left and right arrows follow what is on screen while reporting logical char offsets.
pub fn single_line(ui: &mut Ui, id: Id, text: &mut String, font: FontId, width: f32) -> Response {
    // Centred in the height it's given (a rename row), like the name it replaces.
    let height = ui.available_height().max(0.0);
    egui::TextEdit::singleline(text)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(font)
        .desired_width(width)
        .align(egui::Align2::LEFT_CENTER)
        .min_size(vec2(0.0, height))
        .show(ui)
        .response
        .response
}

/// One letter per script the UI's own fonts may lack: Hebrew, Arabic, Ethiopic, Devanagari,
/// Gurmukhi, Gujarati, Tamil, Telugu, Kannada, Malayalam, Sinhala, Lao, Myanmar, Khmer, Armenian
/// and Georgian. egui's shaper selects the joined contextual forms itself. Bengali and Thai are
/// not here: [`crate::cjk`] picks their faces, in its own order of preference, after this list.
pub(crate) const SCRIPT_SAMPLES: [char; 16] = [
    '\u{05D0}', '\u{0628}', '\u{1200}', '\u{0915}', '\u{0A15}', '\u{0A95}', '\u{0B95}', '\u{0C15}', '\u{0C95}', '\u{0D15}', '\u{0D9A}', '\u{0E81}',
    '\u{1000}', '\u{1780}', '\u{0531}', '\u{10D0}',
];

/// System families that draw those scripts, in order of preference: the first installed one that
/// has a script's letter is taken, so a family covering several scripts (Segoe UI, Nirmala UI,
/// Kohinoor, Noto Sans) joins once. Windows, macOS and the common Linux fonts for each script.
pub(crate) const SCRIPT_FAMILIES: [&str; 46] = [
    // Hebrew and Arabic.
    "Segoe UI",
    "Arial",
    "Tahoma",
    "Noto Sans Hebrew",
    "Noto Sans Arabic",
    "Arial Hebrew",
    "Geeza Pro",
    "Lucida Grande",
    "DejaVu Sans",
    "FreeSans",
    "Noto Sans",
    // Ethiopic (#2014).
    "Ebrima",
    "Nyala",
    "Kefa",
    "Noto Sans Ethiopic",
    "Abyssinica SIL",
    // Devanagari, Gurmukhi, Gujarati, Tamil, Telugu, Kannada, Malayalam and Sinhala.
    "Nirmala UI",
    "Kohinoor Devanagari",
    "Kohinoor Gujarati",
    "Kohinoor Telugu",
    "Gurmukhi MN",
    "Tamil Sangam MN",
    "Kannada Sangam MN",
    "Malayalam Sangam MN",
    "Sinhala Sangam MN",
    "Noto Sans Devanagari",
    "Noto Sans Gurmukhi",
    "Noto Sans Gujarati",
    "Noto Sans Tamil",
    "Noto Sans Telugu",
    "Noto Sans Kannada",
    "Noto Sans Malayalam",
    "Noto Sans Sinhala",
    "Lohit Devanagari",
    // Lao, Myanmar and Khmer.
    "Leelawadee UI",
    "Lao Sangam MN",
    "Myanmar Text",
    "Myanmar Sangam MN",
    "Khmer Sangam MN",
    "Noto Sans Lao",
    "Noto Sans Myanmar",
    "Noto Sans Khmer",
    // Armenian and Georgian.
    "Sylfaen",
    "Mshtakan",
    "Noto Sans Armenian",
    "Noto Sans Georgian",
];

/// Faces the UI adds for scripts its own fonts lack (desktop: from the system), as egui font data;
/// `taken` as for [`crate::theme::system_fallbacks`]. Hebrew and Arabic names were covered first;
/// Ethiopic and the other scripts in [`SCRIPT_SAMPLES`] drew as boxes in the panels (a String
/// variable's Amharic value, #2014) until the system face for each was taken too. Bengali and
/// Thai keep [`crate::cjk`]'s choice, made after this list.
pub fn script_fallbacks(have: &dyn Fn(char) -> bool, taken: &mut Vec<Arc<figcraft_text::Face>>) -> Vec<(String, egui::FontData)> {
    crate::theme::system_fallbacks(SCRIPT_SAMPLES, &SCRIPT_FAMILIES, have, taken)
}

#[cfg(test)]
mod tests;
