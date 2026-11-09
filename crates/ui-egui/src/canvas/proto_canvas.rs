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
    /// Dragging the arrow end of the source's interaction at this index, from where it was
    /// grabbed: dropped on a frame it goes there, on empty canvas the connection goes.
    rewire: Option<(usize, Pos2)>,
}

/// How close (screen points) to a connection's arrow tip a press picks up its end.
const END_GRAB: f32 = 8.0;

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

/// A dragged arrow end let go: over a frame other than the source's own the connection goes there
/// ("drag the arrow end to another frame"); over empty canvas it goes away ("click and drag the
/// connection to an empty space on the canvas, and release it"); over the source's own frame, or
/// where it was, nothing changes.
fn rewire(app: &mut FigApp, ctx: &egui::Context, doc: &Document, page: NodeId, d: &Drag, index: usize) {
    let Some(it) = doc.node(d.source).and_then(|n| n.interactions.get(index)) else { return };
    match frame_at(app, doc, page, d.pos) {
        Some(dest) if doc.top_level_frame(d.source) == Some(dest) => {}
        Some(dest) => {
            let mut done = false;
            let actions: Vec<Value> = it
                .actions
                .iter()
                .map(|a| {
                    let mut v = serde_json::to_value(a).unwrap_or(Value::Null);
                    if !done && a.dest().is_some_and(|was| was != dest) {
                        v["dest"] = json!(dest);
                        done = true;
                    }
                    v
                })
                .collect();
            if done && app.run("proto.interaction.set", json!({"ids": [d.source], "index": index, "actions": actions})).is_some() {
                crate::proto_panel::open_editor(ctx, Some((d.source, index)));
            }
        }
        None => {
            if app.run("proto.interaction.remove", json!({"ids": [d.source], "index": index})).is_some() {
                crate::proto_panel::open_editor(ctx, None);
            }
        }
    }
}

/// Another variant of the component set `source` is a variant of (or inside one), under a screen
/// point: where a drag from it makes a Change to.
fn sibling_variant_at(app: &FigApp, doc: &Document, source: NodeId, p: Pos2) -> Option<NodeId> {
    let mut variant = source;
    for _ in 0..64 {
        let parent = doc.node(variant)?.parent?;
        if matches!(doc.node(parent)?.kind, figcraft_doc::NodeKind::ComponentSet { .. }) {
            let pt = app.to_page(p);
            return doc.node(parent)?.children.iter().rev().copied().find(|v| *v != variant && doc.world_bounds(*v).is_some_and(|b| b.contains(pt)));
        }
        variant = parent;
    }
    None
}

fn arrow(painter: &egui::Painter, pts: &[Pos2; 4], col: Color32) {
    let tip = pts[3];
    let dir = (pts[3] - bezier(pts, 0.9)).normalized();
    let dir = if dir.is_finite() && dir.length() > 0.5 { dir } else { vec2(1.0, 0.0) };
    let n = vec2(-dir.y, dir.x);
    painter.add(Shape::convex_polygon(vec![tip, tip - dir * 8.0 + n * 4.5, tip - dir * 8.0 - n * 4.5], col, Stroke::NONE));
}

/// The page's flow starting points as their canvas tags: the start frame, the tag's box above the
/// frame's top-left corner and the play button at its left end.
fn flow_tags(app: &FigApp, ctx: &egui::Context, doc: &Document, page: NodeId) -> Vec<(NodeId, String, Rect, Rect)> {
    let figcraft_doc::NodeKind::Page { flows, .. } = &doc.node(page).map(|p| p.kind.clone()).unwrap_or(figcraft_doc::NodeKind::Group) else {
        return Vec::new();
    };
    flows
        .iter()
        .take(64)
        .filter_map(|f| {
            let frame = screen_rect(app, doc, f.start)?;
            let text = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(f.name.clone(), crate::theme::semibold(11.0), Color32::WHITE)).size().x;
            let tag = Rect::from_min_size(pos2(frame.min.x, frame.min.y - 26.0), vec2(20.0 + 6.0 + text.min(240.0) + 8.0, 20.0));
            let play = Rect::from_min_size(tag.min, vec2(20.0, 20.0));
            Some((f.start, f.name.clone(), tag, play))
        })
        .collect()
}

/// A flow's tag: a play button and its name, in the accent colour.
fn flow_tag(painter: &egui::Painter, name: &str, tag: Rect, play: Rect, hot: bool) {
    let col = crate::theme::canvas_accent(painter.ctx());
    painter.rect_filled(tag, CornerRadius::same(4), if hot { col } else { col.gamma_multiply(0.9) });
    let c = play.center();
    painter.add(Shape::convex_polygon(vec![c + vec2(-3.0, -5.0), c + vec2(5.0, 0.0), c + vec2(-3.0, 5.0)], Color32::WHITE, Stroke::NONE));
    let galley = painter.layout_no_wrap(name.to_string(), crate::theme::semibold(11.0), Color32::WHITE);
    let at = pos2(play.max.x + 6.0, tag.center().y - galley.size().y / 2.0);
    painter.with_clip_rect(tag).galley(at, galley, Color32::WHITE);
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
    // "A small blue label… Flow 1" over each flow's starting frame, with its play button.
    for (_, name, tag, play) in flow_tags(app, ctx, doc, page) {
        flow_tag(painter, &name, tag, play, hover.is_some_and(|q| tag.contains(q)));
    }
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
            if d.rewire.is_none() {
                plus_handle(painter, d.from, true);
            }
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
            if let Some((index, grabbed)) = d.rewire {
                if d.pos.distance(grabbed) < 4.0 {
                    // A click on the arrow end opens the connection, as a click on its line does.
                    app.session.set_selection(vec![d.source]);
                    crate::proto_panel::open_editor(&ctx, Some((d.source, index)));
                } else {
                    rewire(app, &ctx, &doc, page, &d, index);
                }
                return true;
            }
            let added = if let Some(variant) = sibling_variant_at(app, &doc, d.source, d.pos) {
                // Onto another variant of the source's own set: Change to (interactive components).
                app.run("proto.interaction.add", json!({"ids": [d.source], "action": {"type": "changeTo", "dest": variant}}))
            } else if let Some(dest) = frame_at(app, &doc, page, d.pos)
                && doc.top_level_frame(d.source) != Some(dest)
            {
                // From a selection of hotspots, every one connects ("select multiple starting
                // hotspots on your canvas, then click and drag the icon to the destination").
                let from: Vec<NodeId> = if sel.contains(&d.source) { sel.clone() } else { vec![d.source] };
                let from: Vec<NodeId> = from.into_iter().filter(|i| *i == d.source || doc.top_level_frame(*i) != Some(dest)).collect();
                app.run("proto.interaction.add", json!({"ids": from, "dest": dest}))
            } else {
                None
            };
            if let Some(r) = added {
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
        ctx.data_mut(|m| m.insert_temp(drag_id(), Drag { source, from, pos, rewire: None }));
        return true;
    }
    // A flow's tag: its play button presents the flow, the rest selects its starting frame.
    if let Some((start, _, _, play)) = flow_tags(app, &ctx, &doc, page).into_iter().find(|(_, _, tag, _)| tag.contains(pos)) {
        if play.contains(pos) {
            app.run("proto.present", json!({"start": start}));
        } else {
            app.session.set_selection(vec![start]);
        }
        return true;
    }
    // The arrow end of a connection: drag it to another frame or off onto the canvas.
    let end = noodles(app, &ctx, &doc, page)
        .into_iter()
        .filter(|n| n.pts[3].distance(pos) <= END_GRAB)
        .min_by(|a, b| a.pts[3].distance(pos).total_cmp(&b.pts[3].distance(pos)));
    if let Some(n) = end {
        ctx.data_mut(|m| m.insert_temp(drag_id(), Drag { source: n.source, from: n.pts[0], pos, rewire: Some((n.index, pos)) }));
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

    /// Drag a connection from `source`'s handle to page point `to`.
    fn drag_connection(u: &mut crate::present_tests::Ui, source: NodeId, to: (f64, f64)) {
        let doc = u.app.session.doc().unwrap().clone();
        let sel = u.app.session.selection();
        let (_, at) = *handles(&u.app, &doc, &sel).iter().find(|(i, _)| *i == source).unwrap();
        u.button(at, true);
        let end = u.app.to_screen(to.0, to.1);
        u.frame(vec![egui::Event::PointerMoved(at + (end - at) * 0.5)]);
        u.frame(vec![egui::Event::PointerMoved(end)]);
        u.button(end, false);
        u.frames(2, Vec::new);
    }

    fn connections(u: &crate::present_tests::Ui, id: NodeId) -> Vec<figcraft_doc::proto::Action> {
        u.app.session.doc().unwrap().node(id).unwrap().interactions.iter().flat_map(|i| i.actions.clone()).collect()
    }

    /// "To create multiple connections at once, select multiple starting hotspots on your canvas,
    /// then click and drag the icon to the destination": every selected layer connects.
    #[test]
    fn a_drag_from_a_selection_connects_every_layer() {
        let mut s = figcraft_engine::Session::new();
        let a = crate::present_tests::id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap());
        let b = crate::present_tests::id(&s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 200, "height": 300})).unwrap());
        let btns: Vec<NodeId> = (0..3)
            .map(|k| {
                let r = s.execute("shape.rectangle", &json!({"x": 20, "y": 20 + k * 60, "width": 80, "height": 30, "parent": a})).unwrap();
                crate::present_tests::id(&r)
            })
            .collect();
        s.execute("select.set", &json!({"ids": btns})).unwrap();
        let mut u = crate::present_tests::Ui::new(s);
        u.app.ui.right_tab = RightTab::Prototype;
        u.run("view.zoomFit", json!({}));
        u.frames(2, Vec::new);
        drag_connection(&mut u, btns[1], (400.0, 150.0));
        for k in &btns {
            assert!(
                matches!(connections(&u, *k)[..], [figcraft_doc::proto::Action::Navigate { dest, .. }] if dest == b),
                "{k} connects to B: {:?}",
                connections(&u, *k)
            );
        }
    }

    /// A drag from a variant onto another variant of its set makes a Change to ("drag it to the
    /// destination variant"): the set was taken for the variant's own frame and the drop refused.
    #[test]
    fn a_drag_onto_a_sibling_variant_makes_a_change_to() {
        let mut s = figcraft_engine::Session::new();
        let mut variants = Vec::new();
        for (k, state) in ["Off", "On"].iter().enumerate() {
            let f = s
                .execute("frame.create", &json!({"x": k as f64 * 150.0, "y": 0, "width": 100, "height": 40, "name": format!("State={state}")}))
                .unwrap();
            let f = crate::present_tests::id(&f);
            variants.push(crate::present_tests::id(&s.execute("component.create", &json!({"ids": [f]})).unwrap()));
        }
        s.execute("component.combineVariants", &json!({"ids": variants.clone()})).unwrap();
        s.execute("select.set", &json!({"ids": [variants[0]]})).unwrap();
        let mut u = crate::present_tests::Ui::new(s);
        u.app.ui.right_tab = RightTab::Prototype;
        u.run("view.zoomFit", json!({}));
        u.frames(2, Vec::new);
        let on = u.app.session.doc().unwrap().world_bounds(variants[1]).unwrap().center();
        drag_connection(&mut u, variants[0], (on.x, on.y));
        assert!(
            matches!(connections(&u, variants[0])[..], [figcraft_doc::proto::Action::ChangeTo { dest }] if dest == variants[1]),
            "{:?}",
            connections(&u, variants[0])
        );
    }

    /// Frames A, B and C with a button in A connected to B, selected, on the Prototype tab.
    fn connected() -> (crate::present_tests::Ui, NodeId, NodeId, NodeId) {
        let mut s = figcraft_engine::Session::new();
        let at = |s: &mut figcraft_engine::Session, x: f64| {
            crate::present_tests::id(&s.execute("frame.create", &json!({"x": x, "y": 0, "width": 200, "height": 300})).unwrap())
        };
        let (a, b, c) = (at(&mut s, 0.0), at(&mut s, 300.0), at(&mut s, 600.0));
        let btn =
            crate::present_tests::id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 80, "height": 30, "parent": a})).unwrap());
        s.execute("proto.interaction.add", &json!({"ids": [btn], "dest": b})).unwrap();
        s.execute("select.set", &json!({"ids": [btn]})).unwrap();
        let mut u = crate::present_tests::Ui::new(s);
        u.app.ui.right_tab = RightTab::Prototype;
        u.run("view.zoomFit", json!({}));
        u.frames(2, Vec::new);
        (u, btn, b, c)
    }

    /// Press the connection's arrow end and let go at page point `to` (or where it was).
    fn drag_end(u: &mut crate::present_tests::Ui, to: Option<(f64, f64)>) {
        let ds = u.app.session.ds().unwrap();
        let (doc, page) = (ds.doc.clone(), ds.page);
        let tip = noodles(&u.app, &u.ctx, &doc, page).first().unwrap().pts[3];
        u.button(tip, true);
        let end = to.map_or(tip, |(x, y)| u.app.to_screen(x, y));
        u.frame(vec![egui::Event::PointerMoved(tip + (end - tip) * 0.5)]);
        u.frame(vec![egui::Event::PointerMoved(end)]);
        u.button(end, false);
        u.frames(2, Vec::new);
    }

    /// The arrow end of a connection drags to another frame ("the connection now goes there"), or
    /// off onto empty canvas to remove it ("click and drag the connection to an empty space on the
    /// canvas, and release it"); a click on it keeps it and opens it.
    #[test]
    fn a_connection_end_drags_to_another_frame_or_away() {
        let (mut u, btn, b, c) = connected();
        drag_end(&mut u, None);
        assert!(matches!(connections(&u, btn)[..], [figcraft_doc::proto::Action::Navigate { dest, .. }] if dest == b), "a click keeps it");
        assert_eq!(crate::proto_panel::editor(&u.ctx), Some((btn, 0)), "and opens it");
        drag_end(&mut u, Some((700.0, 150.0)));
        assert!(matches!(connections(&u, btn)[..], [figcraft_doc::proto::Action::Navigate { dest, .. }] if dest == c), "{:?}", connections(&u, btn));
        drag_end(&mut u, Some((700.0, 600.0)));
        assert!(connections(&u, btn).is_empty(), "dropped on empty canvas: gone");
    }

    /// Each flow shows its tag over its starting frame: a click selects the frame, its play button
    /// presents the flow from there.
    #[test]
    fn a_flow_tag_selects_its_frame_and_plays_the_flow() {
        let (mut u, btn, b, _) = connected();
        u.run("proto.flow.add", json!({"start": b, "name": "Checkout"}));
        u.frames(2, Vec::new);
        let ds = u.app.session.ds().unwrap();
        let (doc, page) = (ds.doc.clone(), ds.page);
        let tags = flow_tags(&u.app, &u.ctx, &doc, page);
        let (_, name, tag, play) = tags.iter().find(|t| t.0 == b).cloned().expect("a tag over B");
        assert_eq!(name, "Checkout");
        let frame = u.app.session.doc().unwrap().world_bounds(b).unwrap();
        assert!(tag.max.y <= u.app.to_screen(frame.x0, frame.y0).y, "above the frame");
        assert!(u.app.session.selection().contains(&btn));
        u.button(pos2(tag.max.x - 4.0, tag.center().y), true);
        u.button(pos2(tag.max.x - 4.0, tag.center().y), false);
        assert_eq!(u.app.session.selection(), vec![b], "the tag selects its frame");
        u.button(play.center(), true);
        u.button(play.center(), false);
        u.frames(2, Vec::new);
        assert!(u.app.ui.present.is_some(), "the play button presents the flow");
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
