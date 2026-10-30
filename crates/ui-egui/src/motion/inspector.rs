//! The Motion inspector: the right panel while Motion is on.
//!
//! Laid out like the Design panel: flat `section_header`s, two fields per grid row with a caption
//! above. Sections: the layer's name, Animations, Transform, Layout,
//! Appearance and Fill (and Clip while one is selected). Every animatable field shows its value at
//! the playhead and carries a diamond that adds or removes a keyframe there.

use egui::{Align2, CornerRadius, Rect, RichText, Sense, Shape, Stroke, Ui, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{self, AnimProperty, ClipKind};
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::icons::Icon;
use crate::motion::SelectedKey;
use crate::panels::right::{menu_item, menu_popup};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, FieldEdit, Lead};

/// The inspector's width (the right panel widens to it in Motion mode).
pub const WIDTH: f32 = 413.0;
const ROW_H: f32 = 32.0;

fn selected_layer(app: &FigApp) -> Option<NodeId> {
    app.session.selection().first().copied()
}

/// A grid row of the Motion panel. A field and its keyframe diamond sit 4 pt apart,
/// so the diamond reads as part of the field's group (a combo's button) rather than a separate cell;
/// the Design panel's own rows keep the wider 8 pt gutter.
fn row(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui)) {
    const PAD: f32 = 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new().max_rect(rect.shrink2(vec2(PAD, (h - 24.0) / 2.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing = vec2(4.0, 0.0);
    add(&mut child);
}

pub fn panel(app: &mut FigApp, ui: &mut Ui, rect: Rect, t: &Tokens) {
    if app.ui.motion.easing_target {
        crate::motion::easing::panel(app, ui, rect, t);
        return;
    }
    ui.painter().line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0, t.border));
    egui::ScrollArea::vertical().id_salt("motion_inspector").auto_shrink([false, false]).show(ui, |ui| {
        // The Design panel's grid: 8 pt between items, no extra outer margin (the section headers
        // and rows carry their own 16 pt), a 1 pt separator between sections.
        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
        let Some(layer) = selected_layer(app) else {
            ui.label(RichText::new(tr("Select a layer")).font(regular(11.0)).color(t.text_tertiary));
            return;
        };
        // The layer's name only: Motion mode has none of the component / mask / boolean actions or
        // the component instance fields the Design header carries.
        let name = app.session.doc().ok().and_then(|d| d.node(layer)).map(|n| n.name.clone()).unwrap_or_default();
        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        ui.painter().text(pos2(head.min.x + 16.0, head.center().y), Align2::LEFT_CENTER, name, semibold(13.0), t.text);
        widgets::separator(ui);

        // The Animations header: a "+" whose menu picks the animation kind.
        let (_hdr, header) = widgets::section_header_resp(ui, tr("Animation"), &[], false);
        let hr = header.rect;
        let plus = Rect::from_min_size(pos2(hr.max.x - 8.0 - 24.0, hr.center().y - 12.0), vec2(24.0, 24.0));
        let presp = crate::motion::icon_button(ui, plus, crate::icons::Icon::Plus, "motion_add_anim", tr("Animation"), false, t);
        menu_popup(&presp).show(|ui| {
            for kind in ClipKind::ALL {
                if menu_item(ui, tr(kind.name()), "") {
                    app.ui.motion.clip_anchor = Some(plus);
                    add_clip(app, layer, kind);
                    ui.close();
                }
            }
        });
        animations_body(app, ui, layer, t);
        widgets::separator(ui);
        widgets::section_header(ui, tr("Transform"), &[], false);
        transform_body(app, ui, layer, t);
        widgets::separator(ui);
        widgets::section_header(ui, tr("Layout"), &[], false);
        layout_body(app, ui, layer, t);
        widgets::separator(ui);
        widgets::section_header(ui, tr("Appearance"), &[], false);
        appearance_body(app, ui, layer, t);
        widgets::separator(ui);
        // Fill, Stroke and Effects are the Design panel's own rows: the same colour picker, style
        // chips and effect list, so there is nothing to re-implement here.
        let (doc, sel) = {
            let Ok(ds) = app.session.ds() else { return };
            (ds.doc.clone(), ds.selection.clone())
        };
        crate::panels::right::paints::paints(app, ui, &doc, &sel, true, Some(layer));
        widgets::separator(ui);
        crate::panels::right::paints::paints(app, ui, &doc, &sel, false, Some(layer));
        widgets::separator(ui);
        crate::panels::right::effects::effects(app, ui, &doc, &sel);
        widgets::separator(ui);
    });
    // The clip's settings float over the canvas, anchored to the inspector's left edge — the
    // right panel keeps the layer's own fields.
    crate::motion::clip_popover::show(app, ui.ctx(), rect, t);
}

/// The keyframe diamond of `prop`, for the reused Design-panel rows (Fill, Stroke) that do not
/// live in this file.
pub(crate) fn keyframe_diamond(app: &mut FigApp, ui: &mut Ui, layer: NodeId, prop: AnimProperty, t: &Tokens) {
    diamond(app, ui, layer, prop, t);
}

fn animations_body(app: &mut FigApp, ui: &mut Ui, layer: NodeId, t: &Tokens) {
    // The clips applied to this layer: a name row with its eye and remove buttons. The "+" in
    // the section header adds one.
    let clips: Vec<motion::Clip> = app.session.doc().ok().and_then(|d| d.node(layer)).map(|n| n.motion.clips.clone()).unwrap_or_default();
    let wide = widgets::col_width(ui.available_width()) * 2.0 + 8.0;
    if clips.is_empty() {
        widgets::sublabels(ui, tr("None"), None);
    }
    for c in &clips {
        row(ui, ROW_H, |ui| {
            let selected = app.ui.motion.selected_clip == Some((layer, c.id));
            let (r, resp) = ui.allocate_exact_size(vec2(wide, 24.0), Sense::click());
            // A clip row takes the selected fill as a whole block — the same one the layer rows
            // use — instead of keeping the field colour and drawing a border around it.
            let bg = if selected {
                t.selected_row
            } else if resp.hovered() {
                t.hover
            } else {
                t.field
            };
            ui.painter().rect_filled(r, CornerRadius::same(5), bg);
            let label = format!("{} · {}", tr(c.kind.name()), c.mode.label(c.kind));
            ui.painter().text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, &label, regular(10.0), t.text);
            let eye = Rect::from_center_size(pos2(r.max.x - 46.0, r.center().y), vec2(20.0, 20.0));
            let eye_icon = if c.visible { Icon::Eye } else { Icon::EyeOff };
            let eresp = crate::motion::icon_button(ui, eye, eye_icon, ("anim_eye", c.id), tr("Show/Hide"), false, t);
            if eresp.clicked() {
                toggle_clip_visible(app, layer, c.id);
            }
            let minus = Rect::from_center_size(pos2(r.max.x - 22.0, r.center().y), vec2(20.0, 20.0));
            let mresp = crate::motion::icon_button(ui, minus, Icon::Minus, ("anim_del", c.id), tr("Remove"), false, t);
            if mresp.clicked() {
                remove_clip(app, layer, c.id);
                if app.ui.motion.selected_clip == Some((layer, c.id)) {
                    app.ui.motion.selected_clip = None;
                }
            }
            if resp.clicked() && !eresp.clicked() && !mresp.clicked() {
                // The row opens (or closes) the clip's floating settings window, anchored here.
                let open = app.ui.motion.clip_popover == Some((layer, c.id));
                app.ui.motion.selected_clip = Some((layer, c.id));
                app.ui.motion.clip_popover = if open { None } else { Some((layer, c.id)) };
                app.ui.motion.clip_just_opened = !open;
                app.ui.motion.clip_anchor = Some(r);
                app.ui.motion.easing_target = false;
            }
        });
    }
}

/// Shows or hides a clip — the eye in the Animations list.
fn toggle_clip_visible(app: &mut FigApp, layer: NodeId, id: u32) {
    let Some(mut clip) = app.session.doc().ok().and_then(|d| d.node(layer)).and_then(|n| n.motion.clip(id)).cloned() else { return };
    clip.visible = !clip.visible;
    if app.session.begin_interaction("Show animation clip").is_ok() {
        app.run("motion.clipUpdate", json!({"node": layer.0, "id": id, "clip": clip}));
        let _ = app.session.commit_interaction();
    }
}

fn remove_clip(app: &mut FigApp, layer: NodeId, id: u32) {
    if app.session.begin_interaction("Delete animation clip").is_ok() {
        app.run("motion.clipDelete", json!({"node": layer.0, "id": id}));
        let _ = app.session.commit_interaction();
    }
}

fn transform_body(app: &mut FigApp, ui: &mut Ui, layer: NodeId, t: &Tokens) {
    let col = widgets::col_width(ui.available_width());
    widgets::sublabels(ui, tr("Position"), None);
    row(ui, ROW_H, |ui| {
        field(app, ui, layer, AnimProperty::Position, 0, "X", col);
        field(app, ui, layer, AnimProperty::Position, 1, "Y", col);
        diamond(app, ui, layer, AnimProperty::Position, t);
    });
    widgets::sublabels(ui, tr("Scale"), None);
    row(ui, ROW_H, |ui| {
        field(app, ui, layer, AnimProperty::Scale, 0, "X", col);
        field(app, ui, layer, AnimProperty::Scale, 1, "Y", col);
        diamond(app, ui, layer, AnimProperty::Scale, t);
    });
    widgets::sublabels(ui, tr("Rotation"), None);
    row(ui, ROW_H, |ui| {
        field(app, ui, layer, AnimProperty::Rotation, 0, "°", col);
        diamond(app, ui, layer, AnimProperty::Rotation, t);
    });
}

fn layout_body(app: &mut FigApp, ui: &mut Ui, layer: NodeId, t: &Tokens) {
    // Two fields, their two diamonds and the proportions link in one row: the link is the last
    // cell, so both fields give up the room for it (and its 4 pt gutter).
    let content = ui.available_width() - 32.0;
    let col = ((content - 24.0 * 3.0 - 4.0 * 4.0) / 2.0).max(40.0);
    widgets::sublabels(ui, tr("Width"), Some(tr("Height")));
    row(ui, ROW_H, |ui| {
        field(app, ui, layer, AnimProperty::Width, 0, "W", col);
        diamond(app, ui, layer, AnimProperty::Width, t);
        field(app, ui, layer, AnimProperty::Height, 0, "H", col);
        diamond(app, ui, layer, AnimProperty::Height, t);
        // The proportions lock: W and H then scale together — typed into either field or dragged
        // by a corner handle on the canvas (the document's `constrainProportions`).
        let locked = app.session.doc().ok().and_then(|d| d.node(layer)).is_some_and(|n| n.constrain_proportions);
        let (lr, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
        let link_icon = if locked { Icon::LinkOn } else { Icon::LinkOff };
        let link = crate::motion::icon_button(ui, lr, link_icon, ("motion_link", layer), tr("Constrain proportions"), locked, t);
        if link.clicked() {
            app.set_props(&[layer], serde_json::json!({"constrainProportions": !locked}));
        }
    });
}

fn appearance_body(app: &mut FigApp, ui: &mut Ui, layer: NodeId, t: &Tokens) {
    let col = widgets::col_width(ui.available_width()) - 16.0;
    widgets::sublabels(ui, tr("Opacity"), Some(tr("Corner radius")));
    row(ui, ROW_H, |ui| {
        field(app, ui, layer, AnimProperty::Opacity, 0, "%", col);
        diamond(app, ui, layer, AnimProperty::Opacity, t);
        field(app, ui, layer, AnimProperty::CornerRadius, 0, "R", col);
        diamond(app, ui, layer, AnimProperty::CornerRadius, t);
    });
}

/// A grid field: the value at the playhead, committed as a keyframe there.
fn field(app: &mut FigApp, ui: &mut Ui, layer: NodeId, prop: AnimProperty, idx: usize, lead: &str, col: f32) {
    let id = ui.scope_id().with(("mfield", layer, prop as u8, idx));
    let value = value_of(app, layer, prop, idx);
    if let Some(e) = widgets::number_field_mixed(ui, id, Lead::Text(lead), value, "", col, 1.0)
        && let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, _)) = e.settled()
    {
        commit_value(app, layer, prop, idx, v as f32);
    }
}

fn value_of(app: &FigApp, layer: NodeId, prop: AnimProperty, idx: usize) -> Option<f64> {
    let t = app.ui.motion.playhead_ms;
    let v = app.session.doc().ok().and_then(|d| d.node(layer)).and_then(|n| motion::value_at(n, prop, t))?;
    match v {
        motion::AnimValue::Scalar(x) => Some(x * prop.display_factor()),
        motion::AnimValue::Vec2(p) => Some(if idx == 0 { p.x } else { p.y }),
        motion::AnimValue::Color(_) => None,
    }
}

/// Writes `value` into the property at the playhead (creating the track when needed).
fn commit_value(app: &mut FigApp, layer: NodeId, prop: AnimProperty, idx: usize, value: f32) {
    let time = app.ui.motion.playhead_ms.max(0.0) as u32;
    let current = app.session.doc().ok().and_then(|d| d.node(layer)).and_then(|n| {
        n.motion.track(prop).and_then(|tr| tr.key_at(time)).map(|k| k.value).or_else(|| motion::keyframed_value_at(n, prop, time as f64))
    });
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
    if app.session.begin_interaction("Set keyframe value").is_ok() {
        app.run("motion.keyframe", json!({"ids": [layer.0], "property": prop.name(), "time": time, "value": next}));
        let _ = app.session.commit_interaction();
    }
    app.ui.motion.folded.remove(&layer);
}

/// Writes a typed value (a colour, a shadow's number) into `prop` at the playhead, creating the
/// track when needed: the Fill, Stroke and Effects rows' keyframe edit.
pub(crate) fn commit_anim_value(app: &mut FigApp, layer: NodeId, prop: AnimProperty, value: motion::AnimValue) {
    let time = app.ui.motion.playhead_ms.max(0.0) as u32;
    if app.session.begin_interaction("Set keyframe value").is_ok() {
        app.run("motion.keyframe", json!({"ids": [layer.0], "property": prop.name(), "time": time, "value": value}));
        let _ = app.session.commit_interaction();
    }
    app.ui.motion.folded.remove(&layer);
}

/// The keyframe diamond: filled when a keyframe sits at the playhead, clicking adds or removes one.
fn diamond(app: &mut FigApp, ui: &mut Ui, layer: NodeId, prop: AnimProperty, t: &Tokens) {
    let time = app.ui.motion.playhead_ms.max(0.0) as u32;
    let on = app.session.doc().ok().and_then(|d| d.node(layer)).and_then(|n| n.motion.track(prop)).is_some_and(|tr| tr.key_at(time).is_some());
    // A 24×24 button (the field row's content height), not the full 32 pt row: a taller box was
    // what pushed the diamond out of line with the fields beside it.
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let resp = resp.on_hover_text(if on { tr("Remove keyframe") } else { tr("Add keyframe") });
    let c = rect.center();
    // The keyed state gets the selected fill, hover its own: the button reads like any other cell.
    if on {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.selected_row);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.hover);
    }
    let col = if on {
        t.accent
    } else if resp.hovered() {
        t.text
    } else {
        t.icon_secondary
    };
    ui.painter().add(Shape::convex_polygon(
        vec![c + vec2(0.0, -4.5), c + vec2(4.5, 0.0), c + vec2(0.0, 4.5), c + vec2(-4.5, 0.0)],
        if on { t.accent } else { egui::Color32::TRANSPARENT },
        Stroke::new(1.25, col),
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
    if app.session.begin_interaction("Add keyframe").is_ok() {
        app.run("motion.keyframe", json!({"ids": [layer.0], "property": prop.name(), "time": time}));
        let _ = app.session.commit_interaction();
    }
    app.ui.motion.folded.remove(&layer);
    app.ui.motion.selected = Some(SelectedKey { node: layer, property: prop, time_ms: time });
}

fn remove_key(app: &mut FigApp, layer: NodeId, prop: AnimProperty, time: u32) {
    if app.session.begin_interaction("Delete keyframe").is_ok() {
        app.run("motion.keyframeDelete", json!({"node": layer.0, "property": prop.name(), "time": time}));
        let _ = app.session.commit_interaction();
    }
    if app.ui.motion.selected.is_some_and(|k| k.node == layer && k.property == prop && k.time_ms == time) {
        app.ui.motion.selected = None;
    }
}

/// Adds one clip of `kind` (a default "in" at the playhead) — the Animations "+" menu.
pub(crate) fn add_clip(app: &mut FigApp, layer: NodeId, kind: ClipKind) {
    let start = app.ui.motion.playhead_ms.max(0.0) as u32;
    let id = if app.session.begin_interaction("Add animation clip").is_ok() {
        let r = app
            .run("motion.clip", json!({"ids": [layer.0], "kind": kind.name(), "time": start}))
            .and_then(|v| v.get("clips")?.as_array()?.first()?.get("id")?.as_u64())
            .map(|id| id as u32);
        let _ = app.session.commit_interaction();
        r
    } else {
        None
    };
    app.ui.motion.folded.remove(&layer);
    if let Some(id) = id {
        app.ui.motion.selected_clip = Some((layer, id));
        app.ui.motion.clip_popover = Some((layer, id));
        app.ui.motion.clip_just_opened = true;
        app.ui.motion.easing_target = false;
    }
}
