//! A section header's icon buttons (Appearance, Fill, Stroke, Effects...) draw their glyphs at the
//! same size as the panel's other 24 pt icon buttons (#2227).

use egui::{Context, Rect, pos2, vec2};

use crate::icons::{self, Icon};
use crate::widgets::{icon_button_at, icon_glyph_rect, section_header};

/// The bounding box of every glyph-like shape painted in one frame of `add`, one per shape, with
/// the pointer at `pointer` (hover-only buttons) when given.
pub(crate) fn painted_rects(pointer: Option<egui::Pos2>, add: impl FnOnce(&mut egui::Ui)) -> Vec<Rect> {
    let ctx = Context::default();
    let mut add = Some(add);
    let events = pointer.map(egui::Event::PointerMoved).into_iter().collect();
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 600.0))), events, ..Default::default() };
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
        .collect()
}

/// Bounding box of everything painted in one frame of `add`.
fn painted_box(add: impl FnOnce(&mut egui::Ui)) -> Rect {
    painted_rects(None, add).into_iter().fold(Rect::NOTHING, |a, b| a.union(b))
}

/// Whether `painted` holds `icon` drawn at the size of a 24 pt `icons::button`'s glyph (16 pt): the
/// reference glyph's shapes, moved by one offset, are all among `painted` (same sizes and spacing,
/// so a three-dot glyph's spread counts too).
pub(crate) fn paints_icon_at_button_size(painted: &[Rect], icon: Icon) -> bool {
    let reference = painted_rects(None, |ui| {
        icons::button(ui, icon, 24.0, false, "reference");
    });
    let same = |a: Rect, b: Rect| (a.min - b.min).length() < 0.5 && (a.size() - b.size()).length() < 0.5;
    let Some(first) = reference.first() else { return false };
    painted.iter().any(|p| {
        let shift = p.min - first.min;
        reference.iter().all(|r| painted.iter().any(|q| same(*q, r.translate(shift))))
    })
}

/// The header's eye, `icon_button_at`'s and a fill row's (`icons::button`, the reporter's
/// reference) are drawn at one size.
#[test]
fn header_eye_draws_as_large_as_a_fill_rows_eye() {
    let header = painted_box(|ui| {
        section_header(ui, "Appearance", &[(Icon::Eye, "Hide")], false);
    });
    let at = painted_box(|ui| {
        let r = Rect::from_min_size(pos2(20.0, 20.0), vec2(24.0, 24.0));
        icon_button_at(ui, ui.scope_id().with("b"), r, Icon::Eye, "Hide");
    });
    let row = painted_box(|ui| {
        icons::button(ui, Icon::Eye, 24.0, false, "Toggle visibility");
    });
    for (name, b) in [("icon_button_at", at), ("icons::button", row)] {
        assert!(header.is_finite() && b.is_finite(), "{header:?} {b:?}");
        assert!((header.width() - b.width()).abs() < 0.5, "{name}: widths {} vs {}", header.width(), b.width());
        assert!((header.height() - b.height()).abs() < 0.5, "{name}: heights {} vs {}", header.height(), b.height());
    }
}

#[test]
fn glyph_rect_is_the_shared_inset_of_the_button() {
    let b = Rect::from_min_size(pos2(0.0, 0.0), vec2(24.0, 24.0));
    assert_eq!(icon_glyph_rect(b).size(), vec2(16.0, 16.0));
}
