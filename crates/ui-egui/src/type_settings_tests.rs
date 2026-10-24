//! Regression for #1029: a Type settings row whose label is wider than its column (French "Retrait
//! de paragraphe") stays inside the popover, which otherwise widened a little every frame.
use egui::{Area, Id, Pos2, RawInput, Rect, vec2};

#[test]
fn a_long_label_does_not_widen_the_type_settings_popover() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, false, 0);
    let id = Id::new("ty_row_test");
    let mut widths = Vec::new();
    for i in 0..30 {
        let raw =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), time: Some(i as f64 / 60.0), ..Default::default() };
        ctx.run_ui(raw, |ui| {
            // As `egui::Popup::width` shows it: a default width the area grows past to fit.
            Area::new(id).fixed_pos(Pos2::ZERO).default_width(264.0).show(ui.ctx(), |ui| {
                for label in ["List spacing", "Retrait de paragraphe, plus long que la colonne"] {
                    crate::typography::field_row(ui, label, |ui, fw| ui.allocate_exact_size(vec2(fw, 20.0), egui::Sense::hover()));
                }
            });
        })
        .textures_delta
        .clear();
        widths.push(ctx.memory(|m| m.area_rect(id)).expect("the area is shown").width());
    }
    let (first, last) = (widths[1], widths[widths.len() - 1]);
    assert!((last - first).abs() < 0.01, "the row widens the popover every frame: {first} → {last}");
    assert!((last - 264.0).abs() < 0.01, "{last}");
}
