//! The code block of the Inspect panel: the code of the layer, coloured, in a box of its own.

use egui::text::LayoutJob;
use egui::{CornerRadius, FontFamily, FontId, TextFormat, Ui};
use figcraft_engine::codegen::CodeLang;

use crate::code_highlight::{self, Palette};
use crate::theme::Tokens;

/// The code of a layer, coloured, in the panel's own field colour.
pub fn show(ui: &mut Ui, code: &str, lang: CodeLang, t: &Tokens) {
    let palette = if t.dark { Palette::dark() } else { Palette::light() };
    let job = layout(code, lang, &palette);
    egui::Frame::NONE
        .fill(t.field)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(10))
        .outer_margin(egui::Margin::symmetric(16, 0))
        .show(ui, |ui| {
            // The panel is narrow: the code wraps instead of running off its box.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.add(egui::Label::new(job).selectable(true));
        });
}

/// The code as a layout job, every piece in the colour of its kind.
pub fn layout(code: &str, lang: CodeLang, palette: &Palette) -> LayoutJob {
    let font = FontId::new(11.0, FontFamily::Monospace);
    let mut job = LayoutJob::default();
    for (kind, text) in code_highlight::pieces(code, lang) {
        job.append(text, 0.0, TextFormat { font_id: font.clone(), color: kind.color(palette), ..Default::default() });
    }
    job
}
