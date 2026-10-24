//! The code block of the Inspect panel: the code of the layer, coloured, in a box of its own.

use std::hash::{Hash, Hasher};

use egui::text::LayoutJob;
use egui::{CornerRadius, FontFamily, FontId, TextFormat, Ui};
use figcraft_engine::codegen::CodeLang;

use crate::code_highlight::{self, Palette};
use crate::theme::Tokens;

/// The code of a layer, coloured, in the panel's own field colour.
pub fn show(ui: &mut Ui, code: &str, lang: CodeLang, t: &Tokens) {
    let palette = Palette::from_tokens(t);
    let job = cached_layout(ui, code, lang, &palette);
    egui::Frame::NONE
        .fill(t.field)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(10))
        .outer_margin(egui::Margin::symmetric(16, 0))
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.add(egui::Label::new(job).selectable(true));
        });
}

/// The code as a layout job, every piece in the colour of its kind. The tokenized result is
/// kept in egui's per-frame cache, so a selection is not re-tokenized on every frame.
pub fn layout(code: &str, lang: CodeLang, palette: &Palette) -> LayoutJob {
    let font = FontId::new(11.0, FontFamily::Monospace);
    let mut job = LayoutJob::default();
    for (kind, text) in code_highlight::pieces(code, lang) {
        job.append(text, 0.0, TextFormat { font_id: font.clone(), color: kind.color(palette), ..Default::default() });
    }
    job
}

/// The cached layout job for `(code, lang, palette)`.
fn cached_layout(ui: &mut Ui, code: &str, lang: CodeLang, palette: &Palette) -> LayoutJob {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut hasher);
    let key = (hasher.finish(), code.len(), lang as u8, palette.dark as u8);
    let id = egui::Id::new(("inspect_code_layout", key));
    if let Some(job) = ui.ctx().data(|d| d.get_temp::<LayoutJob>(id)) {
        return job;
    }
    let job = layout(code, lang, palette);
    ui.ctx().data_mut(|d| d.insert_temp(id, job.clone()));
    job
}
