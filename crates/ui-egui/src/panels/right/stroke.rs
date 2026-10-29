use super::menu::{menu_item, menu_item_with, menu_popup, row};
use super::{
    Brush, Color32, Document, FigApp, Icon, Id, Lead, NodeId, NodeKind, PAD, Pos2, Rect, Stroke, StrokeAlign, StrokeCap, StrokeJoin, StrokeProps,
    Tokens, Ui, Value, WidthPoint, WidthPreset, WidthProfile, a11y, common, icons, json, pos2, tr, widgets,
};

/// A width profile drawn as a filled band across `rect` (thickest point = full height); `None`
/// draws a uniform stroke.
pub(crate) fn profile_art(painter: &egui::Painter, rect: Rect, profile: Option<&WidthProfile>, col: Color32) {
    const STEPS: u32 = 40;
    let max = profile.map_or(1.0, WidthProfile::max_scale).max(1e-6);
    let (cy, half) = (rect.center().y, rect.height() / 2.0);
    let mut mesh = egui::Mesh::default();
    let (mut top, mut bottom) = (Vec::new(), Vec::new());
    for i in 0..=STEPS {
        let t = f64::from(i) / f64::from(STEPS);
        // The path runs left to right: its left side is drawn on top.
        let (l, r) = profile.map_or((0.5, 0.5), |p| {
            let (l, r) = p.sides_at(t);
            (l / max, r / max)
        });
        let x = rect.min.x + rect.width() * t as f32;
        let (hl, hr) = (((l as f32) * half).max(0.5), ((r as f32) * half).max(0.5));
        top.push(pos2(x, cy - hl));
        bottom.push(pos2(x, cy + hr));
        mesh.colored_vertex(pos2(x, cy - hl), col);
        mesh.colored_vertex(pos2(x, cy + hr), col);
    }
    for i in 0..STEPS {
        let k = 2 * i;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
    // The mesh isn't anti-aliased: smooth its edges with hairlines.
    for edge in [top, bottom] {
        painter.add(egui::Shape::line(edge, Stroke::new(0.75, col)));
    }
}

/// Stroke settings ▸ Width profile: the presets (each with its shape) and a flip.
fn width_profile_row(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], profile: Option<Option<WidthProfile>>) {
    let t = Tokens::get(ui.ctx());
    widgets::sublabels(ui, tr("Width profile"), None);
    row(ui, 32.0, |ui| {
        let label = match &profile {
            None => tr("Mixed"),
            Some(None) => tr(WidthPreset::Uniform.label()),
            Some(Some(p)) => p.preset().map_or(tr("Custom"), |w| tr(w.label())),
        };
        let resp = widgets::dropdown(ui, label, ui.available_width() - 32.0, None);
        if let Some(p) = &profile {
            let r = resp.rect;
            let art = Rect::from_min_max(pos2(r.max.x - 22.0 - 56.0, r.center().y - 5.0), pos2(r.max.x - 22.0, r.center().y + 5.0));
            profile_art(ui.painter(), art, p.as_ref(), t.icon_secondary);
        }
        menu_popup(&resp).show(|ui| {
            for preset in WidthPreset::ALL {
                let prof = preset.profile();
                let art = |pt: &egui::Painter, r: Rect, col: Color32| {
                    let a = Rect::from_min_max(pos2(r.max.x - 8.0 - 64.0, r.center().y - 5.0), pos2(r.max.x - 8.0, r.center().y + 5.0));
                    profile_art(pt, a, prof.as_ref(), col);
                };
                if menu_item_with(ui, tr(preset.label()), "", 8.0 + 64.0, art) {
                    app.run("stroke.widthProfile", json!({"ids": ids, "preset": preset.id()}));
                    ui.close();
                }
            }
        });
        // Flip: the profile runs from the other end of the path.
        let custom = profile.clone().flatten();
        let flip = icons::button(ui, Icon::FlipH, 24.0, false, tr("Flip width profile"));
        if flip.clicked()
            && let Some(p) = custom
        {
            let pts: Vec<Value> =
                p.points.iter().rev().map(|q| figcraft_engine::cmd::props::width_point_json(&WidthPoint { t: 1.0 - q.t, ..*q })).collect();
            app.run("stroke.widthProfile", json!({"ids": ids, "points": pts}));
        }
    });
}

/// A brush drawn along a short line across `rect`, as outlines (egui fills only convex shapes).
pub(crate) fn brush_art(painter: &egui::Painter, rect: Rect, preset: &str, col: Color32) {
    use figcraft_geom::kurbo::{self, PathEl};
    let mut line = figcraft_geom::BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((f64::from(rect.width()), 0.0));
    let brush = Brush::new(preset);
    let props = StrokeProps { weight: f64::from(rect.height()), align: StrokeAlign::Center, brush: Some(brush.clone()), ..Default::default() };
    let Some(out) = figcraft_pathops::brush::outline(&line, props.weight, &props, &brush) else { return };
    for sub in figcraft_geom::split_subpaths(&out) {
        let mut pts = Vec::new();
        kurbo::flatten(sub.iter(), 0.25, |el| {
            if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
                pts.push(pos2(rect.min.x + p.x as f32, rect.center().y + p.y as f32));
            }
        });
        painter.add(egui::Shape::closed_line(pts, Stroke::new(1.0, col)));
    }
}

/// Stroke settings ▸ Brush: a plain stroke, a stretch brush or a scatter brush, each with its look.
fn brush_row(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], brush: Option<Option<Brush>>, panel_w: f32) {
    use figcraft_pathops::brush::{PRESETS, preset};
    let t = Tokens::get(ui.ctx());
    widgets::sublabels(ui, tr("Brush"), None);
    row(ui, 32.0, |ui| {
        let label = match &brush {
            None => tr("Mixed"),
            Some(b) => b.as_ref().and_then(|b| preset(&b.preset)).map_or(tr("None"), |p| tr(p.label)),
        };
        // The row's own interior: the section's width less its two margins.
        let resp = widgets::dropdown(ui, label, panel_w - PAD - widgets::PAD_RIGHT, None);
        a11y::combo(&resp, tr("Brush"), label, a11y::menu_open(&resp));
        if let Some(Some(b)) = &brush {
            let r = resp.rect;
            brush_art(
                ui.painter(),
                Rect::from_min_max(pos2(r.max.x - 22.0 - 56.0, r.center().y - 5.0), pos2(r.max.x - 22.0, r.center().y + 5.0)),
                &b.preset,
                t.icon_secondary,
            );
        }
        menu_popup(&resp).show(|ui| {
            if menu_item(ui, tr("None"), "") {
                app.run("stroke.brush", json!({"ids": ids, "preset": null}));
                ui.close();
            }
            for p in PRESETS {
                let art = |pt: &egui::Painter, r: Rect, col: Color32| {
                    brush_art(
                        pt,
                        Rect::from_min_max(pos2(r.max.x - 8.0 - 64.0, r.center().y - 5.0), pos2(r.max.x - 8.0, r.center().y + 5.0)),
                        p.id,
                        col,
                    );
                };
                if menu_item_with(ui, tr(p.label), "", 8.0 + 64.0, art) {
                    app.run("stroke.brush", json!({"ids": ids, "preset": p.id}));
                    ui.close();
                }
            }
        });
    });
    // A scatter brush's spacing and jitter.
    let Some(Some(b)) =
        brush.filter(|b| b.as_ref().and_then(|b| preset(&b.preset)).is_some_and(|p| p.kind == figcraft_pathops::brush::BrushKind::Scatter))
    else {
        return;
    };
    let half = widgets::col_width(panel_w);
    let with = |f: &dyn Fn(&mut Brush)| {
        let mut nb = b.clone();
        f(&mut nb);
        serde_json::to_value(nb.sanitized()).unwrap_or_default()
    };
    widgets::sublabels(ui, tr("Spacing"), Some(tr("Scatter")));
    row(ui, 32.0, |ui| {
        let e = widgets::number_field(ui, Id::unique("brush_spacing"), Lead::Icon(Icon::Gap, tr("Brush spacing")), Some(b.spacing), "×", half, 0.1);
        app.field(ids, "brush", e, |v| with(&|nb| nb.spacing = v));
        let e = widgets::number_field(ui, Id::unique("brush_scatter"), Lead::Icon(Icon::Dots, tr("Brush scatter")), Some(b.scatter), "×", half, 0.1);
        app.field(ids, "brush", e, |v| with(&|nb| nb.scatter = v));
    });
    widgets::sublabels(ui, tr("Size jitter"), Some(tr("Rotation jitter")));
    row(ui, 32.0, |ui| {
        let e = widgets::number_field(
            ui,
            Id::unique("brush_size_jitter"),
            Lead::Icon(Icon::Scale, tr("Brush size jitter")),
            Some(b.size_jitter * 100.0),
            "%",
            half,
            1.0,
        );
        app.field(ids, "brush", e, |v| with(&|nb| nb.size_jitter = v / 100.0));
        let e = widgets::number_field(
            ui,
            Id::unique("brush_rotation_jitter"),
            Lead::Icon(Icon::Rotate, tr("Brush rotation jitter")),
            Some(b.rotation_jitter * 100.0),
            "%",
            half,
            1.0,
        );
        app.field(ids, "brush", e, |v| with(&|nb| nb.rotation_jitter = v / 100.0));
    });
}

/// Advanced stroke settings: dash pattern, width profile, join and the end points of open paths.
pub(super) fn stroke_settings(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids = sel.to_vec();
    let half = widgets::col_width(ui.available_width());
    let st = common(doc, sel, |n| n.stroke.clone());
    let dashes = st.as_ref().map(|s| s.dashes.clone());
    let weight = st.as_ref().map(|s| s.weight).unwrap_or(1.0).max(1.0);
    widgets::sublabels(ui, tr("Stroke style"), None);
    row(ui, 32.0, |ui| {
        let dashed = dashes.as_ref().is_some_and(|d| !d.is_empty());
        let resp = widgets::dropdown(ui, if dashed { tr("Dash") } else { tr("Solid") }, half, None);
        menu_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Solid"), "") {
                app.set_props(&ids, json!({"dashes": []}));
                ui.close();
            }
            if menu_item(ui, tr("Dash"), "") {
                app.set_props(&ids, json!({"dashes": [weight * 4.0, weight * 4.0]}));
                ui.close();
            }
        });
    });
    if let Some(d) = dashes.filter(|d| !d.is_empty()) {
        let (dash, gap) = (d.first().copied().unwrap_or(4.0), d.get(1).copied().unwrap_or_else(|| d.first().copied().unwrap_or(4.0)));
        widgets::sublabels(ui, tr("Dash"), Some(tr("Gap")));
        row(ui, 32.0, |ui| {
            let e = widgets::number_field(ui, Id::unique("stroke_dash"), Lead::None, Some(dash), "", half, 1.0);
            app.field(&ids, "dashes", e, |v| json!([v.max(0.0), gap]));
            let e = widgets::number_field(ui, Id::unique("stroke_gap"), Lead::None, Some(gap), "", half, 1.0);
            app.field(&ids, "dashes", e, |v| json!([dash, v.max(0.0)]));
        });
    }
    width_profile_row(app, ui, &ids, st.as_ref().map(|s| s.profile().cloned()));
    brush_row(app, ui, &ids, st.as_ref().map(|s| s.brush.clone()), ui.available_width());
    let join = st.as_ref().map(|s| s.join);
    widgets::sublabels(ui, tr("Join|stroke"), (join == Some(StrokeJoin::Miter)).then_some(tr("Miter angle")));
    row(ui, 32.0, |ui| {
        let resp = widgets::dropdown(ui, join.map(join_label).unwrap_or(tr("Mixed")), half, None);
        menu_popup(&resp).show(|ui| {
            for j in StrokeJoin::ALL {
                if menu_item(ui, join_label(j), "") {
                    app.set_props(&ids, json!({"strokeJoin": j}));
                    ui.close();
                }
            }
        });
        if join == Some(StrokeJoin::Miter) {
            let angle = st.as_ref().map(|s| (s.miter_angle() * 100.0).round() / 100.0);
            let e = widgets::number_field(ui, Id::unique("stroke_miter"), Lead::None, angle, "°", half, 1.0);
            app.field(&ids, "miterAngle", e, |v| json!(v));
        }
    });
    // End points only mean something on open paths.
    let open_paths = sel.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Line | NodeKind::Vector { .. })));
    if open_paths {
        widgets::sublabels(ui, tr("Start point"), Some(tr("End point")));
        row(ui, 32.0, |ui| {
            for (key, cap) in [("startCap", st.as_ref().map(|s| s.cap_start)), ("endCap", st.as_ref().map(|s| s.cap_end))] {
                let resp = widgets::dropdown(ui, cap.map(|c| tr(c.label())).unwrap_or(tr("Mixed")), half, None);
                menu_popup(&resp).show(|ui| {
                    for c in StrokeCap::ALL {
                        if menu_item_with(ui, tr(c.label()), "", CAP_ART_W, |p, r, col| cap_art(p, r, col, c, key == "startCap")) {
                            app.set_props(&ids, json!({key: format!("{c:?}")}));
                            ui.close();
                        }
                    }
                });
            }
        });
    }
    ui.add_space(4.0);
}

/// A stroke join's name in the interface.
pub(crate) fn join_label(j: StrokeJoin) -> &'static str {
    match j {
        StrokeJoin::Miter => tr("Miter"),
        StrokeJoin::Bevel => tr("Bevel"),
        StrokeJoin::Round => tr("Round"),
    }
}

/// The width [`cap_art`] takes at the right of a menu row (its arrows reach past the line's ends).
pub(crate) const CAP_ART_W: f32 = 64.0;

/// A short stroke ending in `cap` (on its left end for a start point), drawn at the right of a menu
/// row.
pub(crate) fn cap_art(p: &egui::Painter, r: Rect, col: Color32, cap: StrokeCap, start: bool) {
    let y = r.center().y;
    let (left, right) = (r.max.x - 58.0, r.max.x - 14.0);
    // The capped end, the plain end, and the direction from the stroke into its cap.
    let (tip, tail, d) = if start { (left, right, -1.0) } else { (right, left, 1.0) };
    let at = |back: f32, dy: f32| pos2(tip - d * back, y + dy);
    let line = |to: Pos2, w: f32| {
        p.line_segment([pos2(tail, y), to], Stroke::new(w, col));
    };
    let fill = |pts: Vec<Pos2>| {
        p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
    };
    match cap {
        StrokeCap::None => line(at(0.0, 0.0), 4.0),
        StrokeCap::Round => {
            line(at(2.0, 0.0), 4.0);
            p.circle_filled(at(2.0, 0.0), 2.0, col);
        }
        StrokeCap::Square => line(at(-2.0, 0.0), 4.0),
        StrokeCap::LineArrow => {
            line(at(0.0, 0.0), 1.5);
            p.add(egui::Shape::line(vec![at(5.5, -5.5), at(0.0, 0.0), at(5.5, 5.5)], Stroke::new(1.5, col)));
        }
        StrokeCap::TriangleArrow => {
            line(at(7.0, 0.0), 1.5);
            fill(vec![at(0.0, 0.0), at(8.0, -4.5), at(8.0, 4.5)]);
        }
        StrokeCap::ReversedTriangle => {
            line(at(8.0, 0.0), 1.5);
            fill(vec![at(8.0, 0.0), at(0.0, -4.5), at(0.0, 4.5)]);
        }
        StrokeCap::CircleFilled => {
            line(at(4.0, 0.0), 1.5);
            p.circle_filled(at(4.0, 0.0), 4.0, col);
        }
        StrokeCap::DiamondFilled => {
            line(at(5.0, 0.0), 1.5);
            fill(vec![at(0.0, 0.0), at(5.0, -5.0), at(10.0, 0.0), at(5.0, 5.0)]);
        }
    }
}
