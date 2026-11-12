//! Vector multi-edit and edit-mode tail behaviour suite, written from the public documentation of
//! design tools: editing several vector layers at once, selecting several Bezier handles and
//! dragging them together, rounding a point next to a curved segment, a cap and a join per point,
//! and what the Pen does on existing points outside edit mode. Each rule is a table row (or a step)
//! with a one-line derivation; where the documentation is silent the row says so and pins today's
//! behaviour.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use figcraft_doc::{NodeId, NodeKind, StrokeCap, StrokeJoin, VectorNetwork};
use figcraft_geom::kurbo::{CubicBez, ParamCurve, ParamCurveDeriv, ParamCurveNearest};
use figcraft_geom::{BezPath, Point, Rect, Shape, Vec2};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, ToolId};
use crate::vectoredit::{VectorEdit, network};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn net(s: &Session, id: NodeId) -> VectorNetwork {
    network(s.doc().unwrap(), id).unwrap().clone()
}

fn editing(s: &Session) -> Option<VectorEdit> {
    s.ds().unwrap().vector_edit.clone()
}

#[derive(Clone, Copy)]
struct Mods {
    shift: bool,
    cmd: bool,
}

const NONE: Mods = Mods { shift: false, cmd: false };
const SHIFT: Mods = Mods { shift: true, cmd: false };
const CMD: Mods = Mods { shift: false, cmd: true };

fn ev(kind: PointerKind, p: (f64, f64), m: Mods) -> PointerEvent {
    let mut e = PointerEvent::at(kind, p.0, p.1, 1.0);
    (e.shift, e.cmd) = (m.shift, m.cmd);
    e
}

fn click(s: &mut Session, p: (f64, f64), m: Mods) {
    s.pointer(&ev(PointerKind::Down, p, m)).unwrap();
    s.pointer(&ev(PointerKind::Up, p, m)).unwrap();
}

fn drag(s: &mut Session, a: (f64, f64), b: (f64, f64), m: Mods) {
    s.pointer(&ev(PointerKind::Down, a, m)).unwrap();
    s.pointer(&ev(PointerKind::Move, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), m)).unwrap();
    s.pointer(&ev(PointerKind::Move, b, m)).unwrap();
    s.pointer(&ev(PointerKind::Up, b, m)).unwrap();
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    id_of(&ex(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 100, "height": 100}))["id"]).unwrap()
}

/// The point at page position `p` of vector `id`.
fn point_at(s: &Session, id: NodeId, p: (f64, f64)) -> usize {
    let w = s.doc().unwrap().world_transform(id);
    net(s, id).vertices.iter().position(|v| (w * v.point() - Point::new(p.0, p.1)).hypot() < 1e-6).unwrap()
}

fn page_pos(s: &Session, id: NodeId, v: usize) -> Point {
    s.doc().unwrap().world_transform(id) * net(s, id).vertices[v].point()
}

fn near(a: Point, b: (f64, f64)) -> bool {
    (a - Point::new(b.0, b.1)).hypot() < 1e-6
}

/// Each edited layer's selected points, the active layer first.
fn selected(s: &Session) -> Vec<(NodeId, Vec<usize>)> {
    let e = editing(s).unwrap();
    e.parts()
        .into_iter()
        .map(|p| {
            let mut v = p.vertices;
            v.sort_unstable();
            (p.id, v)
        })
        .collect()
}

fn selected_in(s: &Session, id: NodeId) -> Vec<usize> {
    selected(s).into_iter().find(|(i, _)| *i == id).map(|(_, v)| v).unwrap_or_default()
}

#[test]
fn editing_several_vectors_at_once() {
    several_vectors_enter_edit_mode();
    let (mut s, a, b, atl, btl) = points_across_both_layers();
    arrows_and_drags_move_every_layer(&mut s, a, b, atl, btl);
    corner_radius_mirroring_and_delete(&mut s, a, b, atl, btl);
    bend_select_all_pen_and_clicks(&mut s, a, b);
}

fn several_vectors_enter_edit_mode() {
    // (how edit mode is entered with two shapes selected, derivation)
    let rows: Vec<(&str, &str)> = vec![
        ("command", "the edit command takes every selected shape"),
        ("double-click", "double-clicking one of several selected shapes edits them all"),
        ("Enter", "Enter with several shapes selected edits them all"),
    ];
    for (how, why) in rows {
        let mut s = Session::new();
        let (a, b) = (rect(&mut s, 0.0), rect(&mut s, 200.0));
        s.set_selection(vec![a, b]);
        match how {
            "command" => {
                ex(&mut s, "vector.edit.begin", json!({}));
            }
            "double-click" => s.pointer(&ev(PointerKind::DoubleClick, (50.0, 50.0), NONE)).unwrap(),
            _ => {
                let ids = crate::vectoredit::enter_targets(s.doc().unwrap(), &s.selection()).unwrap();
                ex(&mut s, "vector.edit.begin", json!({"ids": ids}));
            }
        }
        let e = editing(&s).unwrap_or_else(|| panic!("{why}: in edit mode"));
        let mut layers: Vec<NodeId> = e.parts().iter().map(|p| p.id).collect();
        layers.sort_unstable();
        assert_eq!(layers, vec![a, b], "{why}");
        assert_eq!(e.id, a, "{why}: the layer clicked or first selected is the active one");
        let doc = s.doc().unwrap();
        assert!([a, b].iter().all(|i| matches!(doc.node(*i).unwrap().kind, NodeKind::Vector { .. })), "{why}: both are vectors now");
        s.undo().unwrap();
        let doc = s.doc().unwrap();
        assert!([a, b].iter().all(|i| matches!(doc.node(*i).unwrap().kind, NodeKind::Rectangle)), "{why}: converting both is one undo step");
    }
    // Only layers that can be edited as vectors take part: a frame in the selection keeps Enter
    // from editing points.
    let mut s = Session::new();
    let a = rect(&mut s, 0.0);
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 400, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    assert!(
        crate::vectoredit::enter_targets(s.doc().unwrap(), &[a, f]).is_none(),
        "silent: a frame in the selection leaves Enter to select its children (pinned)"
    );
}

fn points_across_both_layers() -> (Session, NodeId, NodeId, usize, usize) {
    let mut s = Session::new();
    let (a, b) = (rect(&mut s, 0.0), rect(&mut s, 200.0));
    ex(&mut s, "vector.edit.begin", json!({"ids": [a, b]}));
    let (atl, btl) = (point_at(&s, a, (0.0, 0.0)), point_at(&s, b, (200.0, 0.0)));
    // (step, active layer after, a's points, b's points, derivation)
    let steps: Vec<(&str, NodeId, Vec<usize>, Vec<usize>, &str)> = vec![
        ("click b", b, vec![], vec![btl], "a click on another layer's point selects it there"),
        ("shift-click a", a, vec![atl], vec![btl], "Shift-click adds points from any edited layer"),
        ("click a", a, vec![atl], vec![btl], "a press on a selected point keeps the selection to drag it"),
        ("marquee", a, vec![atl, point_at(&s, a, (100.0, 0.0))], vec![btl, point_at(&s, b, (300.0, 0.0))], "a marquee selects points in every layer"),
    ];
    for (what, active, sa, sb, why) in steps {
        match what {
            "click b" => click(&mut s, (200.0, 0.0), NONE),
            "shift-click a" => click(&mut s, (0.0, 0.0), SHIFT),
            "click a" => click(&mut s, (0.0, 0.0), NONE),
            _ => drag(&mut s, (-20.0, -20.0), (320.0, 10.0), NONE),
        }
        let mut sa = sa;
        let mut sb = sb;
        sa.sort_unstable();
        sb.sort_unstable();
        assert_eq!((editing(&s).unwrap().id, selected_in(&s, a), selected_in(&s, b)), (active, sa, sb), "{why}");
    }
    (s, a, b, atl, btl)
}

fn arrows_and_drags_move_every_layer(s: &mut Session, a: NodeId, b: NodeId, atl: usize, btl: usize) {
    // Arrows nudge every selected point, one undo step for all layers.
    ex(s, "vector.edit.select", json!({"vertices": [atl]}));
    ex(s, "vector.edit.select", json!({"id": b, "vertices": [btl], "add": true}));
    assert_eq!((selected_in(s, a), selected_in(s, b)), (vec![atl], vec![btl]), "the select command picks points in a named layer");
    assert!(s.tool_key("Right", false, false, false).unwrap());
    assert!(near(page_pos(s, a, atl), (1.0, 0.0)) && near(page_pos(s, b, btl), (201.0, 0.0)), "an arrow nudges the points in every layer");
    s.undo().unwrap();
    assert!(near(page_pos(s, a, atl), (0.0, 0.0)) && near(page_pos(s, b, btl), (200.0, 0.0)), "one undo step for every layer");
    // Dragging one selected point drags them all (one on the transform box's edge: its corners
    // scale, as with points of one layer).
    let atr = point_at(s, a, (100.0, 0.0));
    ex(s, "vector.edit.select", json!({"id": a, "vertices": [atr], "add": true}));
    drag(s, (100.0, 0.0), (110.0, 20.0), NONE);
    assert!(
        near(page_pos(s, a, atl), (10.0, 20.0)) && near(page_pos(s, a, atr), (110.0, 20.0)) && near(page_pos(s, b, btl), (210.0, 20.0)),
        "dragging one selected point moves them all"
    );
    s.undo().unwrap();
    assert!(near(page_pos(s, a, atl), (0.0, 0.0)) && near(page_pos(s, b, btl), (200.0, 0.0)), "the drag is one undo step");
    ex(s, "vector.edit.select", json!({"vertices": [atl]}));
    ex(s, "vector.edit.select", json!({"id": b, "vertices": [btl], "add": true}));
}

fn corner_radius_mirroring_and_delete(s: &mut Session, a: NodeId, b: NodeId, atl: usize, btl: usize) {
    // Corner radius, mirroring and deleting reach every layer's selected points.
    ex(s, "vector.edit.cornerRadius", json!({"radius": 10}));
    assert!(net(s, a).vertices[atl].radius == 10.0 && net(s, b).vertices[btl].radius == 10.0, "the corner radius applies to every layer");
    ex(s, "vector.edit.mirroring", json!({"mode": "angle"}));
    assert!(
        net(s, a).vertices[atl].mirror == Some(figcraft_doc::HandleMirroring::Angle)
            && net(s, b).vertices[btl].mirror == Some(figcraft_doc::HandleMirroring::Angle),
        "mirroring applies to every layer"
    );
    assert!(s.tool_key("Delete", false, false, false).unwrap());
    assert_eq!((net(s, a).vertices.len(), net(s, b).vertices.len()), (3, 3), "Delete removes the selected point in every layer");
    s.undo().unwrap();
    assert_eq!((net(s, a).vertices.len(), net(s, b).vertices.len()), (4, 4), "one undo step");
}

fn bend_select_all_pen_and_clicks(s: &mut Session, a: NodeId, b: NodeId) {
    // The Bend tool (held with Cmd/Ctrl) bends the segment under the pointer, in its own layer.
    drag(s, (250.0, 100.0), (250.0, 130.0), CMD);
    assert!(net(s, b).segments.iter().any(|sg| !sg.is_straight()), "the bend lands on the layer under the pointer");
    assert!(net(s, a).segments.iter().all(|sg| sg.is_straight()), "other layers keep their segments");
    // Select all takes every edited layer's points.
    let r = ex(s, "vector.edit.selectAll", json!({}));
    assert_eq!(r["count"], json!(8), "silent: select all takes the points of every edited layer (pinned)");
    // The pen draws in the active layer only.
    ex(s, "vector.edit.select", json!({"id": a, "vertices": []}));
    ex(s, "vector.edit.tool", json!({"tool": "pen"}));
    click(s, (50.0, 300.0), NONE);
    assert_eq!((net(s, a).vertices.len(), net(s, b).vertices.len()), (5, 4), "silent: the pen adds to the active layer (pinned)");
    ex(s, "vector.edit.penEnd", json!({}));
    ex(s, "vector.edit.tool", json!({"tool": "move"}));
    // A click on empty canvas inside an edited layer's area keeps edit mode; so does one far
    // away, which only clears the points (a double-click or Escape leaves).
    click(s, (250.0, 50.0), NONE);
    assert!(editing(s).is_some(), "a click inside an edited layer clears the points and stays");
    let st = ex(s, "vector.edit.state", json!({}));
    assert_eq!(st["layers"].as_array().map(Vec::len), Some(2), "the state lists every edited layer");
    click(s, (900.0, 900.0), NONE);
    assert!(editing(s).is_some(), "a click away from every edited layer keeps edit mode");
    s.pointer(&ev(PointerKind::DoubleClick, (900.0, 900.0), NONE)).unwrap();
    assert!(editing(s).is_none(), "a double-click away from every edited layer leaves edit mode");
}

#[test]
fn several_handles_drag_together() {
    // An open curve through (0,0), (100,0) and (200,0); its end handles point to (30,-30) and (170,30).
    let mut s = Session::new();
    let id = id_of(&ex(&mut s, "vector.create", json!({"path": "M0 0 C30 -30 70 -30 100 0 C130 30 170 30 200 0"}))["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.selectAll", json!({}));
    let handles = |s: &Session| {
        let mut h = editing(s).unwrap().handles;
        h.sort_unstable_by_key(|x| (x.0, x.1));
        h
    };
    // (gesture, handles selected after, derivation)
    let steps: Vec<(&str, Mods, Vec<(usize, bool)>, &str)> = vec![
        ("first", NONE, vec![(0, true)], "a click on a handle selects it"),
        ("last", SHIFT, vec![(0, true), (1, false)], "Shift-click adds another handle"),
        ("last", SHIFT, vec![(0, true)], "Shift-click on a selected handle takes it out"),
        ("last", SHIFT, vec![(0, true), (1, false)], "and adds it again"),
    ];
    for (which, m, want, why) in steps {
        click(&mut s, if which == "first" { (30.0, -30.0) } else { (170.0, 30.0) }, m);
        assert_eq!(handles(&s), want, "{why}");
    }
    // Dragging one selected handle moves the others the same way.
    drag(&mut s, (30.0, -30.0), (40.0, -20.0), NONE);
    let n = net(&s, id);
    let ok = |t: (f64, f64), w: (f64, f64)| (t.0 - w.0).abs() < 1e-6 && (t.1 - w.1).abs() < 1e-6;
    assert!(ok(n.segments[0].ta, (40.0, -20.0)), "the dragged handle follows the pointer: {:?}", n.segments[0].ta);
    assert!(ok(n.segments[1].tb, (-20.0, 40.0)), "the other selected handle moves by as much: {:?}", n.segments[1].tb);
    s.undo().unwrap();
    let n = net(&s, id);
    assert!(ok(n.segments[0].ta, (30.0, -30.0)) && ok(n.segments[1].tb, (-30.0, 30.0)), "dragging them is one undo step");
    // The command moves the selected handles by a page offset.
    ex(&mut s, "vector.edit.select", json!({"vertices": [0, 1, 2], "handles": [[0, true], [1, false]]}));
    ex(&mut s, "vector.edit.moveHandles", json!({"dx": 5, "dy": 0}));
    let n = net(&s, id);
    assert!(ok(n.segments[0].ta, (35.0, -30.0)) && ok(n.segments[1].tb, (-25.0, 30.0)), "moveHandles moves every selected handle");
    // A click on a point (not a handle) clears the handles.
    click(&mut s, (100.0, 0.0), NONE);
    assert!(handles(&s).is_empty(), "selecting a point clears the handle selection");
}

/// The smallest distance from `p` to `path`'s segments.
fn distance(path: &BezPath, p: Point) -> f64 {
    path.segments().map(|sg| sg.nearest(p, 1e-6).distance_sq.sqrt()).fold(f64::MAX, f64::min)
}

/// The direction of `c` at `t` (along the chord where a straight segment's handles vanish).
fn tangent(c: &CubicBez, t: f64) -> Vec2 {
    let d = c.deriv().eval(t).to_vec2();
    if d.hypot() > 1e-9 { d } else { c.p3 - c.p0 }
}

#[test]
fn rounding_points_next_to_curves() {
    // A corner at (100, 0) between a straight edge from (0, 0) and a curve down to (0, 100).
    let corner_path = "M0 0 L100 0 C100 60 60 100 0 100 Z";
    // (path, point, radius, rounded, derivation)
    let rows: Vec<(&str, (f64, f64), f64, bool, &str)> = vec![
        (corner_path, (100.0, 0.0), 20.0, true, "a sharp point rounds whatever its segments' shape"),
        ("M0 0 C100 0 100 100 100 100 L0 100 Z", (100.0, 100.0), 20.0, true, "a curve without a handle at the point still meets it at a corner"),
        ("M0 0 C0 -40 100 -40 100 0 C100 40 200 40 200 0", (100.0, 0.0), 20.0, false, "a smooth point has no corner to round"),
    ];
    for (path, at, r, rounded, why) in rows {
        let mut s = Session::new();
        let id = id_of(&ex(&mut s, "vector.create", json!({"path": path}))["id"]).unwrap();
        ex(&mut s, "vector.edit.begin", json!({"id": id}));
        let v = point_at(&s, id, at);
        let before = net(&s, id);
        ex(&mut s, "vector.edit.cornerRadius", json!({"radius": r, "vertices": [v]}));
        let n = net(&s, id);
        let w = s.doc().unwrap().world_transform(id);
        let mut stroke = n.stroke_path();
        stroke.apply_affine(w);
        let gap = distance(&stroke, Point::new(at.0, at.1));
        assert_eq!(gap > 2.0, rounded, "{why}: the outline passes {gap} from the point");
        if !rounded {
            continue;
        }
        // The arc meets each segment where the original passed and turns smoothly into it.
        let drawn = n.rounded().unwrap();
        let original: Vec<_> = (0..before.segments.len()).filter_map(|i| before.cubic(i)).collect();
        let on_original = |p: Point| original.iter().map(|c| c.nearest(p, 1e-9).distance_sq.sqrt()).fold(f64::MAX, f64::min);
        let arc = drawn.segments.len() - 1;
        let c = drawn.cubic(arc).unwrap();
        for (end, t) in [(c.p0, 0.0), (c.p3, 1.0)] {
            assert!(on_original(end) < 0.05, "{why}: the arc ends on the original outline ({})", on_original(end));
            let d = tangent(&c, t);
            // The neighbouring piece's tangent where the arc meets it.
            let piece = (0..arc)
                .filter_map(|i| drawn.cubic(i))
                .find_map(|q| {
                    if (q.p0 - end).hypot() < 1e-6 {
                        Some(tangent(&q, 0.0))
                    } else if (q.p3 - end).hypot() < 1e-6 {
                        Some(tangent(&q, 1.0))
                    } else {
                        None
                    }
                })
                .unwrap();
            let cross = (d.x * piece.y - d.y * piece.x) / (d.hypot() * piece.hypot());
            assert!(cross.abs() < 0.02, "{why}: the arc turns smoothly into the next segment ({cross})");
        }
        // The trimmed curve still follows the original one.
        for i in 0..arc {
            let q = drawn.cubic(i).unwrap();
            assert!(on_original(q.eval(0.5)) < 0.1, "{why}: segment {i} keeps to the original outline");
        }
        assert!(n.fill_contains(w.inverse() * Point::new(50.0, 40.0)), "{why}: the inside stays filled");
    }
}

/// Bounds of `id`'s painted stroke on the page.
fn stroke_bounds(s: &Session, id: NodeId) -> Rect {
    let doc = s.doc().unwrap();
    let n = doc.node(id).unwrap();
    let (area, _) = figcraft_render::draw::stroke_fill(doc, n).unwrap();
    let mut p = (*area).clone();
    p.apply_affine(doc.world_transform(id));
    p.bounding_box()
}

#[test]
fn a_cap_and_a_join_per_point() {
    // A 100-wide open line, stroke 10 centred, no caps on the layer.
    let mut s = Session::new();
    let id = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0]], "closed": false}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"ids": [id], "props": {"strokeWeight": 10, "strokeAlign": "center", "stroke": "#000000"}}));
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let (start, end) = (point_at(&s, id, (0.0, 0.0)), point_at(&s, id, (100.0, 0.0)));
    // (point, cap, left edge, right edge, derivation)
    let rows: Vec<(usize, Value, f64, f64, &str)> = vec![
        (start, json!("square"), -5.0, 100.0, "a point's cap applies at that end only"),
        (end, json!("round"), -5.0, 105.0, "the other end takes its own cap"),
        (start, Value::Null, 0.0, 105.0, "clearing a point's cap goes back to the layer's"),
    ];
    for (v, cap, x0, x1, why) in rows {
        ex(&mut s, "vector.edit.strokeCap", json!({"cap": cap, "vertices": [v]}));
        let b = stroke_bounds(&s, id);
        assert!((b.x0 - x0).abs() < 0.2 && (b.x1 - x1).abs() < 0.2, "{why}: {b:?}");
    }
    let st = s.doc().unwrap().node(id).unwrap().stroke.clone();
    assert_eq!((st.cap_start, st.cap_end), (StrokeCap::None, StrokeCap::None), "the layer's caps are untouched");
    ex(&mut s, "vector.edit.strokeCap", json!({"cap": "triangleArrow", "vertices": [end]}));
    let b = stroke_bounds(&s, id);
    assert!(b.x1 > 101.0 && b.y1 > 6.0, "an arrow cap on one point draws an arrowhead there: {b:?}");
    s.undo().unwrap();
    assert!((stroke_bounds(&s, id).x1 - 105.0).abs() < 0.2, "setting a point's cap is one undo step");

    // A V with a sharp tip at (50, 0); the layer joins with a miter.
    let mut s = Session::new();
    let id = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 100], [50, 0], [100, 100]], "closed": false}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"ids": [id], "props": {"strokeWeight": 10, "strokeAlign": "center", "stroke": "#000000", "strokeJoin": "miter"}}));
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let tip = point_at(&s, id, (50.0, 0.0));
    let half = (50f64 / 100f64).atan();
    // (join, how far the stroke reaches above the tip, derivation)
    let rows: Vec<(Value, f64, &str)> = vec![
        (Value::Null, 5.0 / half.sin(), "without a join of its own a point takes the layer's miter"),
        (json!("bevel"), 5.0 * half.sin(), "a point's bevel cuts its corner off"),
        (json!("round"), 5.0, "a point's round join reaches half the weight"),
    ];
    for (join, reach, why) in rows {
        ex(&mut s, "vector.edit.strokeJoin", json!({"join": join, "vertices": [tip]}));
        let got = -stroke_bounds(&s, id).y0;
        assert!((got - reach).abs() < 0.3, "{why}: {got} vs {reach}");
    }
    assert_eq!(s.doc().unwrap().node(id).unwrap().stroke.join, StrokeJoin::Miter, "the layer's join is untouched");
    // Points keep their cap and join in the file.
    let n = net(&s, id);
    let back: VectorNetwork = serde_json::from_value(serde_json::to_value(&n).unwrap()).unwrap();
    assert_eq!(back.vertices[tip].join, Some(StrokeJoin::Round), "a point's join is saved");
    // SVG writes such strokes as outlines (attributes would apply one join to the whole path).
    let svg = figcraft_svg::export_svg(s.doc().unwrap(), &[id], &figcraft_svg::SvgOptions::default()).unwrap();
    assert!(!svg.contains("stroke-width"), "the SVG stroke is an outline");
    // Several layers at once.
    let mut s = Session::new();
    let a = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0]], "closed": false}))["id"]).unwrap();
    let b = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 50], [100, 50]], "closed": false}))["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"ids": [a, b]}));
    ex(&mut s, "vector.edit.selectAll", json!({}));
    ex(&mut s, "vector.edit.strokeCap", json!({"cap": "round"}));
    assert!([a, b].iter().all(|i| net(&s, *i).vertices.iter().all(|v| v.cap == Some(StrokeCap::Round))), "caps reach every edited layer");
    assert!(s.execute("vector.edit.strokeCap", &json!({"cap": "zigzag"})).is_err(), "an unknown cap is refused");
}

#[test]
fn the_pen_on_existing_points_outside_edit_mode() {
    let pen = |s: &mut Session, pts: &[(f64, f64)]| {
        s.set_tool(ToolId::Pen);
        for p in pts {
            click(s, *p, NONE);
        }
    };
    let layers = |s: &Session| s.doc().unwrap().node(s.page().unwrap()).unwrap().children.clone();
    // Clicking the first point closes the path, which fills.
    let mut s = Session::new();
    pen(&mut s, &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 0.0)]);
    let id = layers(&s)[0];
    assert!(s.tool.pen.is_none() && net(&s, id).regions.len() == 1, "clicking the first point closes the path");
    // Clicking one of the path's own middle points connects there and ends the path.
    let mut s = Session::new();
    pen(&mut s, &[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (100.0, 0.0)]);
    let id = layers(&s)[0];
    let n = net(&s, id);
    let mid = point_at(&s, id, (100.0, 0.0));
    assert!(s.tool.pen.is_none(), "connecting to the path's own point ends it");
    assert_eq!((n.vertices.len(), n.incident(mid).len()), (4, 3), "the path branches at the point it reached (a vector network)");
    assert!(n.regions.is_empty(), "silent: the loop it makes isn't filled, as in edit mode (pinned)");
    // With an open path selected, the Pen continues it from an end point.
    let mut s = Session::new();
    let id = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 0], [100, 0]], "closed": false}))["id"]).unwrap();
    s.set_selection(vec![id]);
    pen(&mut s, &[(100.0, 0.0), (100.0, 100.0)]);
    let e = editing(&s).expect("the pen continues the path in edit mode");
    assert_eq!((e.id, net(&s, id).vertices.len(), layers(&s).len()), (id, 3, 1), "the new point extends the same layer");
    assert!(e.anchor.is_some(), "and the pen keeps going from it");
    // Another layer's end point while drawing: docs silent, the new path's point lands on it.
    let mut s = Session::new();
    let other = id_of(&ex(&mut s, "vector.create", json!({"points": [[0, 200], [100, 200]], "closed": false}))["id"]).unwrap();
    s.set_selection(vec![]);
    pen(&mut s, &[(0.0, 0.0), (100.0, 0.0), (100.0, 200.0)]);
    s.pen_finish().unwrap();
    assert_eq!((layers(&s).len(), net(&s, other).vertices.len()), (2, 2), "silent: the paths stay separate layers (pinned)");
}

#[test]
fn edit_mode_commands_survive_junk_params() {
    // Every vector command, run in multi-edit mode with points and handles selected, on junk.
    let junk = [
        json!({}),
        json!(null),
        json!({"vertices": [-1, 1e12, "a"], "segments": [1e300], "handles": [[1e300, "x"], [-1, true], "y", [0]], "id": "nope", "ids": [1e9]}),
        json!({"cap": 7, "join": [1], "dx": "NaN", "dy": 1e308, "x": null, "y": -1e308, "radius": -5, "mode": 3}),
        json!({"handles": [[0, true], [99999, false]], "dx": 1e308, "dy": -1e308, "cap": "round", "join": "bevel", "vertices": [0, 99999]}),
    ];
    for spec in crate::cmd::all_commands().iter().filter(|c| c.id.starts_with("vector.")) {
        for j in &junk {
            let mut s = Session::new();
            let (a, b) = (rect(&mut s, 0.0), rect(&mut s, 200.0));
            ex(&mut s, "vector.edit.begin", json!({"ids": [a, b]}));
            ex(&mut s, "vector.edit.selectAll", json!({}));
            ex(&mut s, "vector.edit.select", json!({"handles": [[0, true]], "add": true}));
            if let Err(crate::EngineError::Internal { cmd, msg }) = s.execute(spec.id, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
}
