//! A section header's icon buttons (Appearance, Fill, Stroke, Effects...) draw their glyphs at the
//! same size as the panel's other 24 pt icon buttons (#2227).

use egui::{Context, Rect, pos2, vec2};

use crate::icons::Icon;
use crate::widgets::{icon_button_at, icon_glyph_rect, section_header};

/// Bounding box of everything painted in one frame of `add`.
fn painted_box(add: impl FnOnce(&mut egui::Ui)) -> Rect {
    let ctx = Context::default();
    let mut add = Some(add);
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        if let Some(f) = add.take() {
            f(ui);
        }
    });
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter(|s| matches!(s.shape, egui::Shape::Path(_) | egui::Shape::Mesh(_) | egui::Shape::LineSegment { .. } | egui::Shape::Circle(_)))
        .map(|s| s.shape.visual_bounding_rect())
        .fold(Rect::NOTHING, |a, b| a.union(b))
}

#[test]
fn header_eye_draws_as_large_as_an_icon_buttons_eye() {
    let header = painted_box(|ui| {
        section_header(ui, "Appearance", &[(Icon::Eye, "Hide")], false);
    });
    let button = painted_box(|ui| {
        let r = Rect::from_min_size(pos2(20.0, 20.0), vec2(24.0, 24.0));
        icon_button_at(ui, ui.scope_id().with("b"), r, Icon::Eye, "Hide");
    });
    assert!(header.is_finite() && button.is_finite(), "{header:?} {button:?}");
    assert!((header.width() - button.width()).abs() < 0.5, "widths {} vs {}", header.width(), button.width());
    assert!((header.height() - button.height()).abs() < 0.5, "heights {} vs {}", header.height(), button.height());
}

#[test]
fn glyph_rect_is_the_shared_inset_of_the_button() {
    let b = Rect::from_min_size(pos2(0.0, 0.0), vec2(24.0, 24.0));
    assert_eq!(icon_glyph_rect(b).size(), vec2(14.0, 14.0));
}
