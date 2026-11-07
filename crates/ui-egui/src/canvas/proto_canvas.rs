//! Prototype connections on the canvas (Prototype tab): noodles from layers to the frames they
//! act on, a "+" connection handle on selected layers that drags out a new Navigate interaction,
//! and clicking a noodle to edit its interaction.

use std::sync::Arc;

use egui::{Color32, CornerRadius, Id, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use figcraft_doc::{Document, NodeId};
use serde_json::{Value, json};

use crate::{FigApp, RightTab};

mod overlay_ghost;

const HANDLE_R: f32 = 7.0;

/// A connection being dragged out of a layer's handle.
#[derive(Clone, Copy)]
struct Drag {
    source: NodeId,
    from: Pos2,
    pos: Pos2,
}

/// Stable egui id for prototype connection dragging.
fn drag_id() -> Id {
    Id::unique("proto_canvas_drag")
}

/// One drawn connection.
struct Noodle {
    source: NodeId,
    index: usize,
    pts: [Pos2; 4],
}

fn active(app: &FigApp) -> bool {
    app.ui.right_tab == RightTab::Prototype && app.ui.present.is_none()
}

/// Screen rect of a node's axis-aligned page bounds.
fn screen_rect(app: &FigApp, doc: &Document, id: NodeId) -> Option<Rect> {
    let b = doc.world_bounds(id)?;
    Some(Rect::from_min_max(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)))
}

/// Curve from `a` (a layer) to `b` (a frame): leaves the side facing the destination.
fn curve_between(a: Rect, b: Rect) -> [Pos2; 4] {
    let right = b.center().x >= a.center().x;
    let (p0, p3) = if right { (a.right_center(), b.left_center()) } else { (a.left_center(), b.right_center()) };
    let s = if right { 1.0 } else { -1.0 };
    let dx = ((p3.x - p0.x).abs() * 0.5).max(40.0) * s;
    [p0, p0 + vec2(dx, 0.0), p3 - vec2(dx, 0.0), p3]
}

/// The layers on `page` with interactions, in tree order, found once per document version (the
/// tab redraws every frame; walking a large page each time would cost milliseconds).
#[derive(Clone, Default)]
struct Sources {
    doc: Option<Arc<Document>>,
    page: Option<NodeId>,
    ids: Arc<Vec<NodeId>>,
}

/// Return the cached subtree nodes with interactions for `page`, recomputing only when the
/// document pointer or page changes.
pub(crate) fn sources(ctx: &egui::Context, doc: &Arc<Document>, page: NodeId) -> Arc<Vec<NodeId>> {
    ctx.data_mut(|d| {
        let c = d.get_temp_mut_or_default::<Sources>(Id::unique("proto_canvas_sources"));
        if !(c.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, doc)) && c.page == Some(page)) {
            let ids = doc.subtree(page).into_iter().filter(|i| doc.node(*i).is_some_and(|n| !n.interactions.is_empty())).collect();
            *c = Sources { doc: Some(doc.clone()), page: Some(page), ids: Arc::new(ids) };
        }
        c.ids.clone()
    })
}

fn noodles(app: &FigApp, ctx: &egui::Context, doc: &Arc<Document>, page: NodeId) -> Vec<Noodle> {
    let mut out = Vec::new();
    for &id in sources(ctx, doc, page).iter() {
        let Some(n) = doc.node(id) else { continue };
        let Some(from) = screen_rect(app, doc, id) else { continue };
        for (k, it) in n.interactions.iter().enumerate() {
            let Some(dest) = figcraft_doc::proto::walk(&it.actions).find_map(|a| a.dest()) else { continue };
            // Change-to targets are variants; draw to them as they are.
            let Some(to) = screen_rect(app, doc, dest) else { continue };
            if doc.page_of(dest) != Some(page) {
                continue;
            }
            out.push(Noodle { source: id, index: k, pts: curve_between(from, to) });
        }
    }
    out
}

fn bezier(p: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    pos2(w[0] * p[0].x + w[1] * p[1].x + w[2] * p[2].x + w[3] * p[3].x, w[0] * p[0].y + w[1] * p[1].y + w[2] * p[2].y + w[3] * p[3].y)
}

/// Distance from `q` to a cubic (sampled).
fn bezier_distance(p: &[Pos2; 4], q: Pos2) -> f32 {
    let mut best = f32::INFINITY;
    let mut prev = p[0];
    for i in 1..=32 {
        let cur = bezier(p, i as f32 / 32.0);
        best = best.min(segment_distance(prev, cur, q));
        prev = cur;
    }
    best
}

fn segment_distance(a: Pos2, b: Pos2, q: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 > 0.0 { ((q - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(q)
}

/// Handle centres of the selected layers (right edge, vertically centred).
fn handles(app: &FigApp, doc: &Document, sel: &[NodeId]) -> Vec<(NodeId, Pos2)> {
    sel.iter()
        .take(32)
        .filter(|i| doc.node(**i).is_some_and(|n| !n.is_page()))
        .filter_map(|i| screen_rect(app, doc, *i).map(|r| (*i, r.right_center() + vec2(HANDLE_R + 4.0, 0.0))))
        .collect()
}

/// The top-level frame under a screen point (top-most first), else the innermost section there
/// (a connection to a section opens the frame of it shown last).
fn frame_at(app: &FigApp, doc: &Document, page: NodeId, p: Pos2) -> Option<NodeId> {
    let pt = app.to_page(p);
    let under = |i: &NodeId| doc.world_bounds(*i).is_some_and(|b| b.contains(pt));
    doc.top_frames(page).into_iter().rev().find(under).or_else(|| doc.sections_in(page).into_iter().rev().find(under))
}

fn arrow(painter: &egui::Painter, pts: &[Pos2; 4], col: Color32) {
    let tip = pts[3];
    let dir = (pts[3] - bezier(pts, 0.9)).normalized();
    let dir = if dir.is_finite() && dir.length() > 0.5 { dir } else { vec2(1.0, 0.0) };
    let n = vec2(-dir.y, dir.x);
    painter.add(Shape::convex_polygon(vec![tip, tip - dir * 8.0 + n * 4.5, tip - dir * 8.0 - n * 4.5], col, Stroke::NONE));
}

fn plus_handle(painter: &egui::Painter, c: Pos2, hot: bool) {
    painter.circle_filled(c, HANDLE_R + if hot { 1.0 } else { 0.0 }, crate::theme::canvas_accent(painter.ctx()));
    painter.circle_stroke(c, HANDLE_R + if hot { 1.0 } else { 0.0 }, Stroke::new(1.5, Color32::WHITE));
    painter.line_segment([c - vec2(3.5, 0.0), c + vec2(3.5, 0.0)], Stroke::new(1.5, Color32::WHITE));
    painter.line_segment([c - vec2(0.0, 3.5), c + vec2(0.0, 3.5)], Stroke::new(1.5, Color32::WHITE));
}

/// Draw connections, handles and a connection being dragged.
pub fn draw(app: &FigApp, painter: &egui::Painter) {
    if !active(app) {
        return;
    }
    let Ok(ds) = app.session.ds() else { return };
    let (doc, page, sel) = (&ds.doc, ds.page, &ds.selection);
    let ctx = painter.ctx();
    let open = crate::proto_panel::editor(ctx);
    let hover = ctx.input(|i| i.pointer.hover_pos());
    let dragging: Option<Drag> = ctx.data(|d| d.get_temp(drag_id()));
    overlay_ghost::draw(app, painter, doc, page);
    for nd in noodles(app, ctx, doc, page) {
        let selected = sel.contains(&nd.source);
        let is_open = open == Some((nd.source, nd.index));
        let near = dragging.is_none() && hover.is_some_and(|q| bezier_distance(&nd.pts, q) < 5.0);
        let col = if selected || is_open || near {
            crate::theme::canvas_accent(painter.ctx())
        } else {
            crate::theme::canvas_accent(painter.ctx()).gamma_multiply(0.55)
        };
        let w = if is_open || near {
            2.5
        } else if selected {
            2.0
        } else {
            1.5
        };
        // Destination outline for the selected layer's connections.
        if (selected || is_open)
            && let Some(dest) = doc
                .node(nd.source)
                .and_then(|n| n.interactions.get(nd.index))
                .and_then(|it| figcraft_doc::proto::walk(&it.actions).find_map(|a| a.dest()))
            && let Some(r) = screen_rect(app, doc, dest)
        {
            painter.rect_stroke(
                r,
                CornerRadius::ZERO,
                Stroke::new(1.0, crate::theme::canvas_accent(painter.ctx()).gamma_multiply(0.7)),
                StrokeKind::Outside,
            );
        }
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(nd.pts, false, Color32::TRANSPARENT, Stroke::new(w, col)));
        painter.circle_filled(nd.pts[0], 3.5, col);
        painter.circle_stroke(nd.pts[0], 3.5, Stroke::new(1.0, Color32::WHITE));
        arrow(painter, &nd.pts, col);
    }
    match dragging {
        Some(d) => {
            if let Some(f) = frame_at(app, doc, page, d.pos)
                && doc.top_level_frame(d.source) != Some(f)
                && let Some(r) = screen_rect(app, doc, f)
            {
                painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, crate::theme::canvas_accent(painter.ctx())), StrokeKind::Outside);
            }
            let dx = ((d.pos.x - d.from.x).abs() * 0.5).max(30.0) * if d.pos.x >= d.from.x { 1.0 } else { -1.0 };
            let pts = [d.from, d.from + vec2(dx, 0.0), d.pos - vec2(dx, 0.0), d.pos];
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                pts,
                false,
                Color32::TRANSPARENT,
                Stroke::new(2.0, crate::theme::canvas_accent(painter.ctx())),
            ));
            arrow(painter, &pts, crate::theme::canvas_accent(painter.ctx()));
            plus_handle(painter, d.from, true);
        }
        None => {
            if ds.text_edit.is_none() {
                for (_, c) in handles(app, doc, sel) {
                    plus_handle(painter, c, hover.is_some_and(|q| q.distance(c) <= HANDLE_R + 3.0));
                }
            }
        }
    }
}

/// Canvas input while the Prototype tab is open. Returns true when it consumed the gesture
/// (dragging from a connection handle, or clicking a noodle).
pub fn input(app: &mut FigApp, ui: &egui::Ui, resp: &egui::Response) -> bool {
    let ctx = ui.ctx().clone();
    let drag: Option<Drag> = ctx.data(|d| d.get_temp(drag_id()));
    if !active(app) {
        if drag.is_some() {
            ctx.data_mut(|d| d.remove::<Drag>(drag_id()));
        }
        overlay_ghost::end(app, &ctx);
        return false;
    }
    let (pos, pressed, down, released) = ctx.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.primary_down(),
            i.pointer.button_released(egui::PointerButton::Primary),
        )
    });
    if drag.is_none() && overlay_ghost::input(app, &ctx, pos, pos.is_some_and(|p| app.canvas.rect.contains(p)) && resp.hovered()) {
        return true;
    }
    let Ok(ds) = app.session.ds() else { return false };
    let (doc, page, sel) = (ds.doc.clone(), ds.page, ds.selection.clone());
    if let Some(mut d) = drag {
        if let Some(p) = pos {
            d.pos = p;
        }
        ctx.request_repaint();
        if released || !down {
            ctx.data_mut(|m| m.remove::<Drag>(drag_id()));
            if let Some(dest) = frame_at(app, &doc, page, d.pos)
                && doc.top_level_frame(d.source) != Some(dest)
                && let Some(r) = app.run("proto.interaction.add", json!({"ids": [d.source], "dest": dest}))
            {
                let k = r.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                crate::proto_panel::open_editor(&ctx, Some((d.source, k)));
            }
        } else {
            ctx.data_mut(|m| m.insert_temp(drag_id(), d));
        }
        return true;
    }
    let Some(pos) = pos.filter(|p| app.canvas.rect.contains(*p)) else { return false };
    if !resp.hovered() || ds.text_edit.is_some() {
        return false;
    }
    let over_handle = handles(app, &doc, &sel).into_iter().find(|(_, c)| c.distance(pos) <= HANDLE_R + 3.0);
    if over_handle.is_some() {
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if !pressed {
        return false;
    }
    if let Some((source, from)) = over_handle {
        ctx.data_mut(|m| m.insert_temp(drag_id(), Drag { source, from, pos }));
        return true;
    }
    let hit = noodles(app, &ctx, &doc, page)
        .into_iter()
        .filter(|n| bezier_distance(&n.pts, pos) < 5.0)
        .min_by(|a, b| bezier_distance(&a.pts, pos).total_cmp(&bezier_distance(&b.pts, pos)));
    if let Some(n) = hit {
        app.session.set_selection(vec![n.source]);
        crate::proto_panel::open_editor(&ctx, Some((n.source, n.index)));
        return true;
    }
    if pressed && crate::proto_panel::editor(&ctx).is_some() {
        crate::proto_panel::open_editor(&ctx, None);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A press on a connection handle takes the keyboard from the Pages list, as a press anywhere
    /// else on the canvas does, so a later Delete can't take the page (#859).
    #[test]
    fn a_press_on_a_connection_handle_ends_the_pages_list_focus() {
        let mut s = figcraft_engine::Session::new();
        let a = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
        let a = crate::present_tests::id(&a);
        s.execute("select.set", &json!({"ids": [a]})).unwrap();
        let mut u = crate::present_tests::Ui::new(s);
        u.app.ui.right_tab = RightTab::Prototype;
        u.run("view.zoomSelection", json!({}));
        u.frames(2, Vec::new);
        u.app.ui.pages_focus = u.app.session.ds().ok().map(|d| d.uid);
        let doc = u.app.session.doc().unwrap().clone();
        let (_, at) = *handles(&u.app, &doc, &[a]).first().unwrap();
        u.button(at, true);
        assert!(u.app.ui.pages_focus.is_none());
        u.button(at, false);
    }

    #[test]
    fn curves_leave_the_facing_side() {
        let a = Rect::from_min_size(pos2(0.0, 0.0), vec2(50.0, 20.0));
        let b = Rect::from_min_size(pos2(200.0, 100.0), vec2(100.0, 200.0));
        let c = curve_between(a, b);
        assert_eq!(c[0], pos2(50.0, 10.0));
        assert_eq!(c[3], pos2(200.0, 200.0));
        let back = curve_between(b, a);
        assert_eq!(back[0], pos2(200.0, 200.0));
        assert_eq!(back[3], pos2(50.0, 10.0));
        assert!(bezier_distance(&c, bezier(&c, 0.5)) < 0.5);
        assert!(bezier_distance(&c, pos2(0.0, 300.0)) > 50.0);
    }
}
