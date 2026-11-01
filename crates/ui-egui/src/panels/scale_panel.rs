//! The Scale tool's panel (K): scale the selection by a factor or to a width or height, around an
//! anchor point (the engine's `tool.scaleAnchor`, which the canvas handles scale about too).
//! Everything goes through `transform.scale`, so strokes, effects, corner radii and text scale
//! with the layers.

use egui::{CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId};
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::right::row;
use crate::theme::Tokens;
use crate::widgets::{self, FieldEdit, Lead};

/// The Scale section, shown in place of Position while the Scale tool is active.
pub fn section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let Some(b) = doc.bounds_of(sel) else { return };
    widgets::section_header(ui, tr("Scale"), &[], false);
    let col = widgets::col_width(ui.available_width());
    // Anchor: column and row (0 start, 1 middle, 2 end) of the point that stays put.
    let anchor = app.session.tool.scale_anchor;
    let mut factor: Option<f64> = None;
    // A scrub of W or H scales live, in one undo step: `Some(done)`.
    let mut scrub: Option<bool> = None;
    let round = |v: f64| (v * 100.0).round() / 100.0;
    widgets::sublabels(ui, tr("Dimensions"), None);
    row(ui, 32.0, |ui| {
        for (key, lead, side) in [("scale_w", "W", b.width()), ("scale_h", "H", b.height())] {
            let (v, s) = match widgets::number_field(ui, Id::unique(key), Lead::Text(lead), Some(round(side)), "", col, 1.0) {
                Some(FieldEdit::Set(v)) => (v, None),
                Some(FieldEdit::Scrub(v, done)) => (v, Some(done)),
                None => continue,
            };
            if side > 1e-9 {
                factor = Some(v / side);
                scrub = s;
            }
        }
    });
    widgets::sublabels(ui, tr("Scale"), Some(tr("Anchor")));
    crate::right::block(ui, 52.0, 2.0, |ui| {
        // Relative to the layers as they are: a scrub applies once, when let go.
        if let Some(FieldEdit::Set(k) | FieldEdit::Scrub(k, true)) =
            widgets::number_field(ui, Id::unique("scale_k"), Lead::None, Some(1.0), "x", col, 0.1)
        {
            factor = Some(k);
        }
        ui.add_space(8.0);
        if let Some(a) = anchor_picker(ui, anchor) {
            app.run("tool.scaleAnchor", json!({"anchor": figcraft_engine::tools::anchor_name(a)}));
        }
    });
    let scale = factor.and_then(|k| scale_about(b, anchor, k)).map(|(k, cx, cy)| json!({"ids": sel, "factor": k, "cx": cx, "cy": cy}));
    match scrub {
        None => {
            if let Some(p) = scale {
                app.run("transform.scale", p);
            }
        }
        Some(done) => {
            if let Some(p) = scale {
                let _ = app.session.begin_interaction("Scale");
                if let Err(e) = app.session.execute("transform.scale", &p) {
                    app.failed("transform.scale", &e);
                }
            }
            if done {
                let _ = app.session.commit_interaction();
            }
        }
    }
}

/// The factor (clamped to 0.01–100) and pivot for scaling `b` by `k` around `anchor`; None when
/// nothing would change or `k` isn't a usable number.
fn scale_about(b: figcraft_geom::Rect, anchor: (u8, u8), k: f64) -> Option<(f64, f64, f64)> {
    let k = Some(k).filter(|k| k.is_finite() && *k > 0.0 && (*k - 1.0).abs() > 1e-9)?.clamp(0.01, 100.0);
    let (ax, ay) = (f64::from(anchor.0.min(2)) / 2.0, f64::from(anchor.1.min(2)) / 2.0);
    Some((k, b.x0 + b.width() * ax, b.y0 + b.height() * ay))
}

/// A 3 × 3 grid of anchor points; the chosen one is filled. Returns a newly clicked anchor.
fn anchor_picker(ui: &mut Ui, cur: (u8, u8)) -> Option<(u8, u8)> {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(5), t.field);
    let mut out = None;
    for row in 0..3u8 {
        for col in 0..3u8 {
            let c = pos2(r.min.x + 8.0 + f32::from(col) * 16.0, r.min.y + 8.0 + f32::from(row) * 16.0);
            let hit = Rect::from_center_size(c, vec2(16.0, 16.0));
            let resp = ui.interact(hit, Id::unique(("scale_anchor_dot", col, row)), Sense::click());
            let on = (col, row) == cur;
            if on {
                ui.painter().rect_filled(Rect::from_center_size(c, vec2(7.0, 7.0)), CornerRadius::same(2), t.accent);
            } else {
                let dot = Rect::from_center_size(c, vec2(if resp.hovered() { 6.0 } else { 4.0 }, if resp.hovered() { 6.0 } else { 4.0 }));
                ui.painter().rect_filled(dot, CornerRadius::same(1), if resp.hovered() { t.icon } else { t.icon_secondary });
            }
            if resp.clicked() {
                out = Some((col, row));
            }
        }
    }
    ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, t.border), StrokeKind::Inside);
    out
}

#[cfg(test)]
mod tests {
    use super::scale_about;
    use figcraft_geom::Rect;

    #[test]
    fn pivot_follows_the_anchor_and_bad_factors_do_nothing() {
        let b = Rect::new(10.0, 20.0, 110.0, 70.0);
        assert_eq!(scale_about(b, (0, 0), 2.0), Some((2.0, 10.0, 20.0)));
        assert_eq!(scale_about(b, (1, 1), 0.5), Some((0.5, 60.0, 45.0)));
        assert_eq!(scale_about(b, (2, 2), 3.0), Some((3.0, 110.0, 70.0)));
        assert_eq!(scale_about(b, (0, 0), 1e6).map(|s| s.0), Some(100.0));
        for k in [1.0, 0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(scale_about(b, (0, 0), k), None, "{k}");
        }
    }
}
