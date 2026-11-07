//! Design panel in vector edit mode: the selected points' position and dimensions (their
//! transform box), corner radius, handle mirroring and the stroke's cap and join at them, or the
//! selected width point with the Variable width tool (in place of the layer's position and
//! dimensions).

use egui::{Id, Ui};
use figcraft_doc::{Document, HandleMirroring, StrokeCap, StrokeJoin};
use figcraft_engine::vectoredit::{self, VectorEdit};
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::tr;
use crate::icons::Icon;
use crate::right::{CAP_ART_W, cap_art, join_label, menu_item, menu_item_with, menu_popup, row};
use crate::widgets::{self, FieldEdit, Lead};

/// Run a point command from a field: typed values are one step, scrubs one step when released.
fn point_field(app: &mut FigApp, cmd: &str, edit: Option<FieldEdit>, params: impl Fn(f64) -> Value) {
    match edit {
        Some(FieldEdit::Set(v)) => {
            app.run(cmd, params(v));
        }
        Some(FieldEdit::Scrub(v, done)) => {
            let _ = app.session.begin_interaction("Edit points");
            let _ = app.session.execute(cmd, &params(v));
            if done {
                let _ = app.session.commit_interaction();
            }
        }
        None => {}
    }
}

/// The Variable width tool's selected width point: where it is along the path and its width.
fn width_point(app: &mut FigApp, ui: &mut Ui, doc: &Document, ve: &VectorEdit) {
    widgets::section_header(ui, tr("Width point"), &[], false);
    let stroke = doc.node(ve.id).map(|n| &n.stroke);
    let sel = ve.width_point.and_then(|i| Some((i, *stroke?.profile()?.points.get(i)?)));
    let Some((i, p)) = sel else {
        let t = crate::theme::Tokens::get(ui.ctx());
        widgets::note(ui, tr("Drag the stroke to add a width point"), t.text_secondary);
        return;
    };
    let weight = stroke.map_or(1.0, |s| s.weight);
    let col = widgets::col_width(ui.available_width());
    let round = |v: f64| (v * 100.0).round() / 100.0;
    widgets::sublabels(ui, tr("Position"), Some(tr("Width")));
    row(ui, 32.0, |ui| {
        let e = widgets::number_field(ui, Id::unique("wpt_t"), Lead::None, Some(round(p.t * 100.0)), "%", col, 1.0);
        point_field(app, "vector.edit.widthPoint", e, |v| json!({"index": i, "t": v / 100.0}));
        let e =
            widgets::number_field(ui, Id::unique("wpt_w"), Lead::Icon(Icon::StrokeIcon, tr("Width")), Some(round(p.scale * weight)), "", col, 1.0);
        point_field(app, "vector.edit.widthPoint", e, |v| json!({"index": i, "width": v.max(0.0)}));
    });
    // Each side's reach from the centre line (Alt-drag a handle on the canvas for one side).
    let (l, r) = p.sides();
    widgets::sublabels(ui, tr("Left side"), Some(tr("Right side")));
    row(ui, 32.0, |ui| {
        for (side, scale, name) in [("left", l, tr("Left side width")), ("right", r, tr("Right side width"))] {
            let e = widgets::number_field(
                ui,
                Id::unique(("wpt_side", side)),
                Lead::Icon(Icon::StrokeIcon, name),
                Some(round(scale * weight / 2.0)),
                "",
                col,
                1.0,
            );
            point_field(app, "vector.edit.widthPoint", e, |v| json!({"index": i, "side": side, "width": v.max(0.0)}));
        }
    });
    ui.add_space(8.0);
}

/// Draw the selected points' position, dimensions, corner radius, mirroring and per-point
/// stroke cap/join controls; the width tool delegates to its own panel.
pub fn points(app: &mut FigApp, ui: &mut Ui, doc: &Document, ve: &VectorEdit) {
    if ve.tool == vectoredit::VTool::Width {
        width_point(app, ui, doc, ve);
        return;
    }
    let Some(net) = vectoredit::network(doc, ve.id) else { return };
    let pts = vectoredit::selected_points(net, ve);
    widgets::section_header(ui, if pts.len() > 1 { tr("Points") } else { tr("Point") }, &[], false);
    if pts.is_empty() {
        let t = crate::theme::Tokens::get(ui.ctx());
        widgets::note(ui, tr("Select points to edit their position, radius and mirroring"), t.text_secondary);
        return;
    }
    let col = widgets::col_width(ui.available_width());
    let pos = vectoredit::selection_position(doc, ve);
    let round = |v: f64| (v * 100.0).round() / 100.0;
    widgets::sublabels(ui, tr("Position"), None);
    row(ui, 32.0, |ui| {
        let e = widgets::number_field(ui, Id::unique("vpt_x"), Lead::Text("X"), pos.map(|p| round(p.x)), "", col, 1.0);
        point_field(app, "vector.edit.position", e, |v| json!({"x": v}));
        let e = widgets::number_field(ui, Id::unique("vpt_y"), Lead::Text("Y"), pos.map(|p| round(p.y)), "", col, 1.0);
        point_field(app, "vector.edit.position", e, |v| json!({"y": v}));
    });
    if let Some(b) = vectoredit::transform::point_box(doc, ve).filter(|b| b.points.len() > 1) {
        widgets::sublabels(ui, tr("Dimensions"), None);
        row(ui, 32.0, |ui| {
            let e = widgets::number_field(ui, Id::unique("vpt_w"), Lead::Text("W"), Some(round(b.width)), "", col, 1.0);
            point_field(app, "vector.edit.resize", e, |v| json!({"width": v.max(0.0)}));
            let e = widgets::number_field(ui, Id::unique("vpt_h"), Lead::Text("H"), Some(round(b.height)), "", col, 1.0);
            point_field(app, "vector.edit.resize", e, |v| json!({"height": v.max(0.0)}));
        });
    }
    let vertex = |v: &usize| net.vertices.get(*v).copied();
    let radius = same(pts.iter().map(|v| vertex(v).map(|x| x.radius))).flatten();
    let mirror = same(pts.iter().map(|v| net.mirroring(*v)));
    // A point's own cap and join (`Some(None)`: the layer's; `None`: mixed).
    let cap = same(pts.iter().map(|v| vertex(v).map(|x| x.cap))).flatten();
    let join = same(pts.iter().map(|v| vertex(v).map(|x| x.join))).flatten();
    widgets::sublabels(ui, tr("Corner radius"), None);
    row(ui, 32.0, |ui| {
        let e =
            widgets::number_field(ui, Id::unique("vpt_radius"), Lead::Icon(Icon::CornerRadius, tr("Corner radius")), radius.map(round), "", col, 1.0);
        point_field(app, "vector.edit.cornerRadius", e, |v| json!({"radius": v.max(0.0)}));
    });
    widgets::sublabels(ui, tr("Mirroring"), None);
    row(ui, 32.0, |ui| {
        let label = |m: HandleMirroring| match m {
            HandleMirroring::None => tr("No mirroring"),
            HandleMirroring::Angle => tr("Mirror angle"),
            HandleMirroring::AngleAndLength => tr("Mirror angle and length"),
        };
        let resp = widgets::dropdown(ui, mirror.map(label).unwrap_or(tr("Mixed")), col * 2.0 + 8.0, None);
        menu_popup(&resp).show(|ui| {
            for m in [HandleMirroring::None, HandleMirroring::Angle, HandleMirroring::AngleAndLength] {
                if menu_item(ui, label(m), "") {
                    app.run("vector.edit.mirroring", json!({"mode": m.id()}));
                    ui.close();
                }
            }
        });
    });
    // The stroke where it ends and turns at these points.
    let own = |set: Option<Option<&'static str>>| match set {
        None => tr("Mixed"),
        Some(None) => tr("Same as layer"),
        Some(Some(l)) => l,
    };
    widgets::sublabels(ui, tr("End point"), Some(tr("Join|stroke")));
    row(ui, 32.0, |ui| {
        let resp = widgets::dropdown(ui, own(cap.map(|c| c.map(|c| tr(c.label())))), col, None);
        menu_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Same as layer"), "") {
                app.run("vector.edit.strokeCap", json!({"cap": null}));
                ui.close();
            }
            for c in StrokeCap::ALL {
                if menu_item_with(ui, tr(c.label()), "", CAP_ART_W, |p, r, col| cap_art(p, r, col, c, false)) {
                    app.run("vector.edit.strokeCap", json!({"cap": c}));
                    ui.close();
                }
            }
        });
        let resp = widgets::dropdown(ui, own(join.map(|j| j.map(join_label))), col, None);
        menu_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Same as layer"), "") {
                app.run("vector.edit.strokeJoin", json!({"join": null}));
                ui.close();
            }
            for j in StrokeJoin::ALL {
                if menu_item(ui, join_label(j), "") {
                    app.run("vector.edit.strokeJoin", json!({"join": j}));
                    ui.close();
                }
            }
        });
    });
    ui.add_space(8.0);
}

/// The value every item shares, if they all share one.
fn same<T: PartialEq>(mut it: impl Iterator<Item = T>) -> Option<T> {
    let first = it.next()?;
    it.all(|x| x == first).then_some(first)
}
