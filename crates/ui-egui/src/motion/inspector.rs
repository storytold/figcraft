//! The Motion inspector: the right panel while Motion is on.
//!
//! Sections: the layer's name, Animations, Transform, Layout, Appearance and Fill. Every animatable
//! field shows its value at the playhead and carries a diamond that adds or removes a keyframe there.

use egui::{CornerRadius, Rect, RichText, Sense, Shape, Stroke, Ui, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{self, AnimProperty};

use crate::FigApp;
use crate::motion::SelectedKey;
use crate::theme::{Tokens, regular, semibold};

/// The inspector's width (the right panel widens to it in Motion mode).
pub const WIDTH: f32 = 413.0;
const ROW_H: f32 = 24.0;
const FIELD_W: f32 = 68.0;

fn selected_layer(app: &FigApp) -> Option<NodeId> {
    app.session.selection().first().copied()
}

pub fn panel(app: &mut FigApp, ui: &mut Ui, rect: Rect, t: &Tokens) {
    if app.ui.motion.easing_target {
        crate::motion::easing::panel(app, ui, rect, t);
        return;
    }
    ui.painter().line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0, t.border));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(12.0, 10.0))).layout(egui::Layout::top_down(egui::Align::LEFT)));
    egui::ScrollArea::vertical().id_salt("motion_inspector").show(&mut child, |ui| {
        let Some(layer) = selected_layer(app) else {
            ui.label(RichText::new("Select a layer").font(regular(11.0)).color(t.text_tertiary));
            return;
        };
        let name = app.session.doc().ok().and_then(|d| d.node(layer)).map(|n| n.name.clone()).unwrap_or_default();
        ui.label(RichText::new(name).font(semibold(12.0)).color(t.text));
        ui.add_space(8.0);

        clip_section(app, ui, layer, t);
        ui.add_space(8.0);
        transform(app, ui, layer, t);
        ui.add_space(8.0);
        layout(app, ui, layer, t);
        ui.add_space(8.0);
        appearance(app, ui, layer, t);
        ui.add_space(8.0);
        fill(app, ui, layer, t);
    });
}

fn section_title(ui: &mut Ui, text: &str, t: &Tokens) {
    ui.label(RichText::new(text).font(semibold(10.5)).color(t.text_secondary));
}

fn clip_section(app: &mut FigApp, ui: &mut Ui, layer: NodeId, t: &Tokens) {
    let Some((n, cid)) = app.ui.motion.selected_clip else { return };
    if n != layer {
        return;
    }
    let clip = app.session.doc().ok().and_then(|d| d.node(layer)).and_then(|nd| nd.motion.clips.iter().find(|c| c.id == cid)).cloned();
    let Some(c) = clip else { return };
    section_title(ui, "Clip", t);
    ui.label(RichText::new(format!("{} · {}", c.kind.name(), c.mode.label(c.kind))).font(regular(10.0)).color(t.text));
    let mut set_delay = None;
    let mut set_dur = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("delay").font(regular(9.5)).color(t.text_tertiary));
        if let Some(v) = num_box(ui, ("clip_delay", cid), c.delay_ms as f64 / 1000.0, t) {
            set_delay = Some((v * 1000.0).max(0.0) as u32);
        }
        ui.label(RichText::new("duration").font(regular(9.5)).color(t.text_tertiary));
        if let Some(v) = num_box(ui, ("clip_dur", cid), c.duration_ms as f64 / 1000.0, t) {
            set_dur = Some((v * 1000.0).max(1.0) as u32);
        }
    });
    if set_delay.is_some() || set_dur.is_some() {
        super::edited(app, |app| {
            let Ok(doc) = app.session.doc_mut() else { return };
            if let Some(cl) = doc.node_mut(layer).and_then(|nd| nd.motion.clips.iter_mut().find(|c| c.id == cid)) {
                if let Some(d) = set_delay {
                    cl.delay_ms = d;
                }
                if let Some(d) = set_dur {
                    cl.duration_ms = d;
                }
            }
            doc.mark_dirty([layer]);
        });
    }
}

/// A compact editable number box (seconds); returns the typed value on Enter / focus loss.
fn num_box(ui: &mut Ui, key: (&str, u32), value: f64, t: &Tokens) -> Option<f64> {
    let id = ui.scope_id().with(key);
    let text = format!("{:.2}", value);
    let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| text.clone());
    let (rect, _) = ui.allocate_exact_size(vec2(52.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), t.field);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 1.0))));
    let resp = child.add_sized(
        rect.size() - vec2(8.0, 2.0),
        egui::TextEdit::singleline(&mut buf).font(egui::FontId::proportional(10.0)).frame(egui::Frame::NONE),
    );
    let lost = resp.lost_focus();
    if !resp.has_focus() && !lost {
        buf = text;
    }
    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    let commit = lost || (resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
    commit.then(|| buf.trim().parse::<f64>().ok()).flatten()
}

fn transform(app: &mut FigApp, ui: &mut Ui, layer: NodeId, t: &Tokens) {
    section_title(ui, "Transform", t);
    ui.label(RichText::new("Position").font(regular(9.5)).color(t.text_tertiary));
    ui.horizontal(|ui| {
        field(app, ui, layer, AnimProperty::Position, 0, "X", t);
        field(app, ui, layer, AnimProperty::Position, 1, "Y", t);
        diamond(app, ui, layer, AnimProperty::Position, t);
    });
    ui.label(RichText::new("Scale").font(regular(9.5)).color(t.text_tertiary));
    ui.horizontal(|ui| {
        field(app, ui, layer, AnimProperty::Scale, 0, "X", t);
        field(app, ui, layer, AnimProperty::Scale, 1, "Y", t);
        diamond(app, ui, layer, AnimProperty::Scale, t);
    });
    ui.label(RichText::new("Rotation").font(regular(9.5)).color(t.text_tertiary));
    ui.horizontal(|ui| {
        field(app, ui, layer, AnimProperty::Rotation, 0, "°", t);
        diamond(app, ui, layer, AnimProperty::Rotation, t);
    });
}

fn layout(app: &mut FigApp, ui: &mut Ui, layer: NodeId, t: &Tokens) {
    section_title(ui, "Layout", t);
    ui.label(RichText::new("Dimensions").font(regular(9.5)).color(t.text_tertiary));
    ui.horizontal(|ui| {
        field(app, ui, layer, AnimProperty::Width, 0, "W", t);
        diamond(app, ui, layer, AnimProperty::Width, t);
        field(app, ui, layer, AnimProperty::Height, 0, "H", t);
        diamond(app, ui, layer, AnimProperty::Height, t);
    });
}

fn appearance(app: &mut FigApp, ui: &mut Ui, layer: NodeId, t: &Tokens) {
    section_title(ui, "Appearance", t);
    ui.label(RichText::new("Opacity / Corner radius").font(regular(9.5)).color(t.text_tertiary));
    ui.horizontal(|ui| {
        field(app, ui, layer, AnimProperty::Opacity, 0, "%", t);
        diamond(app, ui, layer, AnimProperty::Opacity, t);
        field(app, ui, layer, AnimProperty::CornerRadius, 0, "R", t);
        diamond(app, ui, layer, AnimProperty::CornerRadius, t);
    });
}

fn fill(app: &mut FigApp, ui: &mut Ui, layer: NodeId, t: &Tokens) {
    section_title(ui, "Fill", t);
    ui.horizontal(|ui| {
        let paint = app.session.doc().ok().and_then(|d| d.node(layer)).and_then(|n| n.fills.first().cloned());
        if let Some(paint) = paint {
            let _ = crate::widgets::swatch(ui, &paint, 20.0);
        }
        diamond(app, ui, layer, AnimProperty::FillColor, t);
    });
}

/// An editable field: typing a number and pressing Enter (or leaving it) keys the property at the
/// playhead, so every animatable value can be edited from the inspector.
fn field(app: &mut FigApp, ui: &mut Ui, layer: NodeId, prop: AnimProperty, idx: usize, prefix: &str, t: &Tokens) {
    let text = value_text(app, layer, prop, idx);
    let id = ui.scope_id().with(("mfield", layer, prop as u8, idx));
    let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| text.clone());
    ui.horizontal(|ui| {
        ui.label(RichText::new(prefix).font(regular(9.0)).color(t.text_tertiary));
        let resp = ui.add_sized(
            vec2(FIELD_W - 12.0, ROW_H),
            egui::TextEdit::singleline(&mut buf).font(egui::FontId::proportional(10.0)).margin(vec2(6.0, 3.0)),
        );
        // `lost_focus()` is also true on the frame Enter commits, so never reset the buffer then:
        // resetting first made every commit write the old value back.
        let lost = resp.lost_focus();
        if !resp.has_focus() && !lost {
            buf = text.clone();
        }
        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
        let commit = lost || (resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if commit && let Ok(v) = buf.trim().trim_end_matches(|c: char| c.is_alphabetic() || c == '%' || c == '°').parse::<f32>() {
            commit_value(app, layer, prop, idx, v);
        }
    });
}

/// Writes `value` into the property at the playhead (creating the track when needed).
fn commit_value(app: &mut FigApp, layer: NodeId, prop: AnimProperty, idx: usize, value: f32) {
    super::edited(app, |app| commit_value_in(app, layer, prop, idx, value));
}

fn commit_value_in(app: &mut FigApp, layer: NodeId, prop: AnimProperty, idx: usize, value: f32) {
    let time = app.ui.motion.playhead_ms.max(0.0) as u32;
    let Ok(doc) = app.session.doc_mut() else { return };
    let Some(n) = doc.node(layer) else { return };
    let mut m = n.motion.clone();
    let current = m.track(prop).and_then(|tr| tr.key_at(time)).map(|k| k.value).or_else(|| motion::keyframed_value_at(n, prop, time as f64));
    let next = match current {
        Some(motion::AnimValue::Vec2(p)) => {
            motion::AnimValue::Vec2(if idx == 0 { figcraft_geom::Vec2::new(value as f64, p.y) } else { figcraft_geom::Vec2::new(p.x, value as f64) })
        }
        Some(motion::AnimValue::Color(c)) => motion::AnimValue::Color(c),
        None if matches!(prop, AnimProperty::Position | AnimProperty::Scale) => {
            motion::AnimValue::Vec2(figcraft_geom::Vec2::new(if idx == 0 { value as f64 } else { 0.0 }, if idx == 1 { value as f64 } else { 0.0 }))
        }
        _ => motion::AnimValue::Scalar(value as f64 / prop.display_factor().max(1e-6)),
    };
    m.ensure_track(prop).set_key(time, next, None);
    m.normalize();
    if let Some(n) = doc.node_mut(layer) {
        n.motion = m;
    }
    doc.mark_dirty([layer]);
    app.ui.motion.folded.remove(&layer);
}

fn value_text(app: &FigApp, layer: NodeId, prop: AnimProperty, idx: usize) -> String {
    let t = app.ui.motion.playhead_ms;
    let v = app.session.doc().ok().and_then(|d| d.node(layer)).and_then(|n| motion::value_at(n, prop, t));
    let Some(v) = v else { return "—".into() };
    match v {
        motion::AnimValue::Scalar(x) => format!("{:.2}{}", x * prop.display_factor(), prop.unit()),
        motion::AnimValue::Vec2(p) => format!("{:.1}", if idx == 0 { p.x } else { p.y }),
        motion::AnimValue::Color(_) => String::new(),
    }
}

/// The keyframe diamond: filled when a keyframe sits at the playhead, clicking adds or removes one.
fn diamond(app: &mut FigApp, ui: &mut Ui, layer: NodeId, prop: AnimProperty, t: &Tokens) {
    let time = app.ui.motion.playhead_ms.max(0.0) as u32;
    let on = app.session.doc().ok().and_then(|d| d.node(layer)).and_then(|n| n.motion.track(prop)).is_some_and(|tr| tr.key_at(time).is_some());
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, ROW_H), Sense::click());
    let c = rect.center();
    let col = if on {
        t.accent
    } else if resp.hovered() {
        t.icon
    } else {
        t.icon_secondary
    };
    ui.painter().add(Shape::convex_polygon(
        vec![c + vec2(0.0, -4.0), c + vec2(4.0, 0.0), c + vec2(0.0, 4.0), c + vec2(-4.0, 0.0)],
        if on { t.accent } else { egui::Color32::TRANSPARENT },
        Stroke::new(1.0, col),
    ));
    if resp.clicked() {
        if on {
            remove_key(app, layer, prop, time);
        } else {
            add_key(app, layer, prop, time);
        }
    }
}

fn add_key(app: &mut FigApp, layer: NodeId, prop: AnimProperty, time: u32) {
    super::edited(app, |app| add_key_in(app, layer, prop, time));
}

fn add_key_in(app: &mut FigApp, layer: NodeId, prop: AnimProperty, time: u32) {
    let Ok(doc) = app.session.doc_mut() else { return };
    let value = doc.node(layer).and_then(|n| motion::keyframed_value_at(n, prop, time as f64));
    let Some(value) = value else { return };
    if let Some(n) = doc.node_mut(layer) {
        n.motion.ensure_track(prop).set_key(time, value, None);
    }
    doc.mark_dirty([layer]);
    app.ui.motion.folded.remove(&layer);
    app.ui.motion.selected = Some(SelectedKey { node: layer, property: prop, time_ms: time });
}

fn remove_key(app: &mut FigApp, layer: NodeId, prop: AnimProperty, time: u32) {
    super::edited(app, |app| remove_key_in(app, layer, prop, time));
}

fn remove_key_in(app: &mut FigApp, layer: NodeId, prop: AnimProperty, time: u32) {
    let Ok(doc) = app.session.doc_mut() else { return };
    if let Some(n) = doc.node_mut(layer) {
        if let Some(tr) = n.motion.track_mut(prop) {
            tr.remove_key(time);
        }
        n.motion.normalize();
    }
    doc.mark_dirty([layer]);
    if app.ui.motion.selected.is_some_and(|k| k.node == layer && k.property == prop && k.time_ms == time) {
        app.ui.motion.selected = None;
    }
}
