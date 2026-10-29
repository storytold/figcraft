//! The easing panel: shown in the inspector when a segment between two keyframes is selected.
//!
//! Lists the preset curves and applies the chosen one to the selected keyframe (the segment's
//! start), so the interpolation of that segment changes immediately.

use egui::{Align2, CornerRadius, Rect, RichText, Sense, Shape, Stroke, Ui, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::Ease;

use crate::FigApp;
use crate::motion::SelectedKey;
use crate::theme::{Tokens, regular, semibold};

pub fn panel(app: &mut FigApp, ui: &mut Ui, rect: Rect, t: &Tokens) {
    ui.painter().line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0, t.border));
    let Some(k) = app.ui.motion.selected else {
        ui.label(RichText::new("Select a segment").font(regular(11.0)).color(t.text_tertiary));
        return;
    };
    let current = easing_of(app, k);
    let mut back = false;
    ui.horizontal(|ui| {
        if ui.add(egui::Button::new(RichText::new("←").font(regular(12.0))).frame(false)).clicked() {
            back = true;
        }
        ui.label(RichText::new("Easing").font(semibold(11.5)).color(t.text));
    });
    ui.add_space(6.0);
    curve(ui, &current, t);
    ui.add_space(8.0);
    for (name, ease) in Ease::PRESETS {
        let on = current == ease;
        if ui.add(egui::Button::new(RichText::new(name).font(regular(10.5)).color(if on { t.accent_text } else { t.text })).frame(on)).clicked() {
            apply(app, k, ease);
        }
    }
    if back {
        app.ui.motion.easing_target = false;
    }
}

fn easing_of(app: &FigApp, k: SelectedKey) -> Ease {
    app.session
        .doc()
        .ok()
        .and_then(|d| d.node(k.node))
        .and_then(|n| n.motion.track(k.property))
        .and_then(|tr| tr.key_at(k.time_ms))
        .map(|key| key.easing)
        .unwrap_or(Ease::Linear)
}

fn apply(app: &mut FigApp, k: SelectedKey, ease: Ease) {
    super::edited(app, |app| {
        let Ok(doc) = app.session.doc_mut() else { return };
        if let Some(tr) = doc.node_mut(k.node).and_then(|n| n.motion.track_mut(k.property))
            && let Some(idx) = tr.key_index(k.time_ms)
        {
            tr.keys[idx].easing = ease;
        }
        doc.mark_dirty([k.node]);
    });
}

/// A small graph of the curve: x is time, y is progress.
fn curve(ui: &mut Ui, ease: &Ease, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(180.0, 110.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), t.field);
    let (lo, hi) = ease.value_range();
    let span = (hi - lo).max(1e-3);
    let pts: Vec<_> = ease
        .sample(40)
        .iter()
        .map(|q| pos2(rect.left() + 6.0 + q[0] * (rect.width() - 12.0), rect.bottom() - 6.0 - ((q[1] - lo) / span) * (rect.height() - 12.0)))
        .collect();
    ui.painter().add(Shape::line(pts, Stroke::new(1.5, t.accent)));
    ui.painter().text(rect.left_top() + vec2(6.0, 6.0), Align2::LEFT_TOP, ease.name(), regular(9.0), t.text_tertiary);
}

/// Keeps `NodeId` visible to the module (used by tests elsewhere).
pub type Target = (NodeId, u32);
