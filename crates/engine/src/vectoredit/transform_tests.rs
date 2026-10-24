//! The transform box around several points and point snapping, through the session (pointer
//! gestures, commands and undo).

use serde_json::json;

use super::tests::{click, drag, ev, ex, net, pt, rect, ve, vertex_at};
use super::transform::{BoxHit, box_hit, shown_box};
use super::*;
use crate::EngineError;
use crate::tools::{Handle, PointerKind, ToolId};

/// Page position of point `v` of vector `id`.
fn page_pt(s: &Session, id: NodeId, v: usize) -> Point {
    s.doc().unwrap().world_transform(id) * net(s, id).vertices[v].point()
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

fn select_all(s: &mut Session) {
    ex(s, "vector.edit.selectAll", json!({}));
}

/// A drag with modifiers held from the start.
fn drag_with(s: &mut Session, from: (f64, f64), to: (f64, f64), shift: bool, alt: bool) {
    let with = |kind, x, y| {
        let mut e = ev(kind, x, y);
        e.shift = shift;
        e.alt = alt;
        e
    };
    s.pointer(&with(PointerKind::Down, from.0, from.1)).unwrap();
    s.pointer(&with(PointerKind::Move, to.0, to.1)).unwrap();
    s.pointer(&with(PointerKind::Up, to.0, to.1)).unwrap();
}

#[test]
fn the_box_shows_for_two_or_more_points_with_the_move_tool() {
    let mut s = Session::new();
    let id = rect(&mut s);
    let doc = s.doc().unwrap().clone();
    assert!(shown_box(&doc, &ve(&s)).is_none(), "nothing selected");
    ex(&mut s, "vector.edit.select", json!({"vertices": [0]}));
    assert!(shown_box(s.doc().unwrap(), &ve(&s)).is_none(), "one point has no box");
    select_all(&mut s);
    let b = shown_box(s.doc().unwrap(), &ve(&s)).unwrap();
    assert_eq!((b.width, b.height, b.points.len()), (100.0, 100.0, 4));
    assert!(near(b.corners()[2], pt(100.0, 100.0)));
    // Corners win over the points under them, points over edges, edges over segments.
    let doc = s.doc().unwrap().clone();
    let at = |x, y| {
        let h = hit(&doc, &ve(&s), pt(x, y), 6.0);
        box_hit(&doc, &ve(&s), h, pt(x, y), 1.0)
    };
    assert_eq!(at(100.0, 100.0), Some(BoxHit::Handle(Handle::Corner(2))));
    assert_eq!(at(100.0, 50.0), Some(BoxHit::Handle(Handle::Edge(1))));
    assert_eq!(at(110.0, -10.0), Some(BoxHit::Handle(Handle::Rotate(1))));
    assert_eq!(at(40.0, 60.0), Some(BoxHit::Inside));
    // Other edit tools don't show it.
    ex(&mut s, "vector.edit.tool", json!({"tool": "pen"}));
    assert!(shown_box(s.doc().unwrap(), &ve(&s)).is_none());
    let _ = id;
}

#[test]
fn corners_scale_points_in_one_undo_step() {
    let mut s = Session::new();
    let id = rect(&mut s);
    select_all(&mut s);
    drag(&mut s, (100.0, 100.0), (150.0, 120.0));
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, 0.0, 150.0, 120.0));
    assert_eq!(ve(&s).vertices.len(), 4, "the points stay selected");
    s.undo().unwrap();
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!((b.width(), b.height()), (100.0, 100.0), "one undo step");
    // Shift keeps proportions; Alt scales from the centre.
    select_all(&mut s);
    drag_with(&mut s, (100.0, 100.0), (150.0, 110.0), true, false);
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, 0.0, 150.0, 150.0));
    s.undo().unwrap();
    select_all(&mut s);
    drag_with(&mut s, (100.0, 100.0), (110.0, 120.0), false, true);
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (-10.0, -20.0, 110.0, 120.0));
}

#[test]
fn edges_scale_one_axis_and_handles_follow() {
    let mut s = Session::new();
    let id = rect(&mut s);
    // A curved handle on the right side scales with its point.
    let v = vertex_at(&net(&s, id), 100.0, 0.0);
    ex(&mut s, "vector.edit.select", json!({"vertices": [v]}));
    ex(&mut s, "vector.edit.smooth", json!({}));
    let before: Vec<(f64, f64)> = net(&s, id).incident(v).iter().map(|(sg, st)| net(&s, id).handle(*sg, *st).unwrap()).collect();
    select_all(&mut s);
    drag(&mut s, (100.0, 50.0), (200.0, 50.0));
    let n = net(&s, id);
    let after: Vec<(f64, f64)> = n.incident(v).iter().map(|(sg, st)| n.handle(*sg, *st).unwrap()).collect();
    for (a, b) in before.iter().zip(&after) {
        assert!((b.0 - a.0 * 2.0).abs() < 1e-6 && (b.1 - a.1).abs() < 1e-6, "{a:?} → {b:?}");
    }
    assert!(near(page_pt(&s, id, v), pt(200.0, 0.0)), "{:?}", page_pt(&s, id, v));
    assert!((0..4).any(|i| near(page_pt(&s, id, i), pt(0.0, 100.0))), "the left side stays");
}

#[test]
fn outside_a_corner_rotates_and_inside_moves() {
    let mut s = Session::new();
    let id = rect(&mut s);
    select_all(&mut s);
    let v = vertex_at(&net(&s, id), 100.0, 0.0);
    // From just outside the top-right corner, a quarter turn clockwise about the centre.
    s.pointer(&ev(PointerKind::Down, 110.0, -10.0)).unwrap();
    s.pointer(&ev(PointerKind::Move, 110.0, 110.0)).unwrap();
    let c = s.vector_edit_box().unwrap();
    assert!(near(c[0], pt(100.0, 0.0)), "the box turns with the points: {c:?}");
    s.pointer(&ev(PointerKind::Up, 110.0, 110.0)).unwrap();
    assert!(near(page_pt(&s, id, v), pt(100.0, 100.0)), "{:?}", page_pt(&s, id, v));
    s.undo().unwrap();
    assert!(near(page_pt(&s, id, v), pt(100.0, 0.0)));
    // Inside the box (clear of the network) the selected points move together.
    let n = net(&s, id);
    let (a, w) = (vertex_at(&n, 0.0, 0.0), vertex_at(&n, 100.0, 100.0));
    ex(&mut s, "vector.edit.select", json!({"vertices": [a, w]}));
    drag(&mut s, (60.0, 40.0), (80.0, 50.0));
    assert!(near(page_pt(&s, id, w), pt(120.0, 110.0)) && near(page_pt(&s, id, a), pt(20.0, 10.0)));
    assert!(near(page_pt(&s, id, vertex_at(&net(&s, id), 0.0, 100.0)), pt(0.0, 100.0)), "unselected points stay");
    // A click inside without a drag clears the point selection; Shift-drag marquees instead.
    click(&mut s, 60.0, 50.0);
    assert!(ve(&s).vertices.is_empty());
}

#[test]
fn box_commands_resize_rotate_and_transform() {
    let mut s = Session::new();
    let id = rect(&mut s);
    assert!(s.execute("vector.edit.bounds", &json!({})).is_err(), "nothing selected");
    select_all(&mut s);
    let b = ex(&mut s, "vector.edit.bounds", json!({}));
    assert_eq!((b["x"].as_f64(), b["width"].as_f64(), b["height"].as_f64()), (Some(0.0), Some(100.0), Some(100.0)));
    ex(&mut s, "vector.edit.resize", json!({"width": 50}));
    let r = s.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!((r.x0, r.x1, r.height()), (0.0, 50.0, 100.0), "the top-left stays");
    ex(&mut s, "vector.edit.resize", json!({"height": 200, "center": true}));
    let r = s.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!((r.y0, r.y1), (-50.0, 150.0), "about the centre");
    // Rotation is counter-clockwise like a layer's.
    s.undo().unwrap();
    s.undo().unwrap();
    select_all(&mut s);
    let v = vertex_at(&net(&s, id), 100.0, 0.0);
    ex(&mut s, "vector.edit.rotate", json!({"angle": 90}));
    assert!(near(page_pt(&s, id, v), pt(0.0, 0.0)), "{:?}", page_pt(&s, id, v));
    s.undo().unwrap();
    select_all(&mut s);
    ex(&mut s, "vector.edit.transform", json!({"matrix": [2, 0, 0, 1, 10, 0], "vertices": [v]}));
    assert!(near(page_pt(&s, id, v), pt(210.0, 0.0)));
    assert!(s.execute("vector.edit.transform", &json!({"matrix": [1, 0]})).is_err());
    // A singular or overflowing matrix changes nothing.
    let before = net(&s, id);
    ex(&mut s, "vector.edit.transform", json!({"matrix": [1e308, 0, 0, 1e308, 1e308, 0]}));
    assert_eq!(net(&s, id), before);
}

#[test]
fn flip_and_rotate_act_on_selected_points_while_editing() {
    let mut s = Session::new();
    let id = rect(&mut s);
    let n = net(&s, id);
    let (a, b) = (vertex_at(&n, 0.0, 0.0), vertex_at(&n, 100.0, 100.0));
    ex(&mut s, "vector.edit.select", json!({"vertices": [a, b]}));
    ex(&mut s, "object.flipH", json!({}));
    assert!(near(page_pt(&s, id, a), pt(100.0, 0.0)) && near(page_pt(&s, id, b), pt(0.0, 100.0)));
    assert!(s.ds().unwrap().vector_edit.is_some(), "still editing");
    ex(&mut s, "object.rotate180", json!({}));
    assert!(near(page_pt(&s, id, a), pt(0.0, 100.0)));
    // Without points selected the layer flips, as outside edit mode.
    ex(&mut s, "vector.edit.select", json!({}));
    let t = s.doc().unwrap().node(id).unwrap().transform;
    ex(&mut s, "object.flipV", json!({"ids": [id]}));
    assert_ne!(s.doc().unwrap().node(id).unwrap().transform, t);
}

#[test]
fn dragged_points_snap_to_other_vectors_points_and_midpoints() {
    let mut s = Session::new();
    // Another vector with a point clear of its box's edge and centre lines.
    ex(&mut s, "vector.create", json!({"points": [[300, 0], [400, 100], [320, 60]], "closed": true}));
    let id = rect(&mut s);
    let v = vertex_at(&net(&s, id), 100.0, 100.0);
    ex(&mut s, "vector.edit.select", json!({"vertices": [v]}));
    s.pointer(&ev(PointerKind::Down, 100.0, 100.0)).unwrap();
    s.pointer(&ev(PointerKind::Move, 318.0, 62.0)).unwrap();
    let marker = s.tool.overlay.guides.iter().any(|g| g.x0 == g.x1 && g.y0 == g.y1 && near(pt(g.x0, g.y0), pt(320.0, 60.0)));
    assert!(marker, "a marker on the point landed on: {:?}", s.tool.overlay.guides);
    s.pointer(&ev(PointerKind::Up, 318.0, 62.0)).unwrap();
    assert!(near(page_pt(&s, id, v), pt(320.0, 60.0)), "{:?}", page_pt(&s, id, v));
    assert!(s.tool.overlay.guides.is_empty(), "guides go with the gesture");
    s.undo().unwrap();
    // Segment midpoints of the vector itself (the top edge's middle).
    ex(&mut s, "vector.edit.select", json!({"vertices": [v]}));
    drag(&mut s, (100.0, 100.0), (52.0, 3.0));
    assert!(near(page_pt(&s, id, v), pt(50.0, 0.0)), "{:?}", page_pt(&s, id, v));
}

#[test]
fn ctrl_suppresses_snapping_and_the_pixel_grid_rounds() {
    let mut s = Session::new();
    ex(&mut s, "vector.create", json!({"points": [[300, 0], [400, 100], [320, 60]], "closed": true}));
    let id = rect(&mut s);
    let v = vertex_at(&net(&s, id), 100.0, 100.0);
    ex(&mut s, "vector.edit.select", json!({"vertices": [v]}));
    let cmd = |kind, x, y| {
        let mut e = ev(kind, x, y);
        e.cmd = true;
        e
    };
    // ⌘ held after the press (pressing with it bends instead).
    s.pointer(&ev(PointerKind::Down, 100.0, 100.0)).unwrap();
    s.pointer(&cmd(PointerKind::Move, 318.4, 62.3)).unwrap();
    s.pointer(&cmd(PointerKind::Up, 318.4, 62.3)).unwrap();
    assert!(near(page_pt(&s, id, v), pt(318.0, 62.0)), "no point snapping, whole pixels: {:?}", page_pt(&s, id, v));
    s.undo().unwrap();
    ex(&mut s, "view.snapPixel", json!({"on": false}));
    ex(&mut s, "vector.edit.select", json!({"vertices": [v]}));
    s.pointer(&ev(PointerKind::Down, 100.0, 100.0)).unwrap();
    s.pointer(&cmd(PointerKind::Move, 318.4, 62.3)).unwrap();
    s.pointer(&cmd(PointerKind::Up, 318.4, 62.3)).unwrap();
    assert!(near(page_pt(&s, id, v), pt(318.4, 62.3)), "{:?}", page_pt(&s, id, v));
}

#[test]
fn handles_follow_the_pointer_while_the_vector_refits() {
    // The preview re-fits the vector (its origin moves as the curve grows up and left); the handle
    // must still land under the pointer.
    let mut s = Session::new();
    let id = rect(&mut s);
    let a = vertex_at(&net(&s, id), 0.0, 0.0);
    let (seg, start) = net(&s, id)
        .incident(a)
        .into_iter()
        .find(|(sg, st)| {
            let n = net(&s, id);
            let x = n.segments[*sg];
            n.vertices[if *st { x.b } else { x.a }].y == 0.0
        })
        .unwrap();
    ex(&mut s, "vector.edit.setHandle", json!({"segment": seg, "start": start, "x": 30, "y": 0, "independent": true}));
    ex(&mut s, "vector.edit.select", json!({"vertices": [a]}));
    s.pointer(&ev(PointerKind::Down, 30.0, 0.0)).unwrap();
    for (x, y) in [(-10.0, -40.0), (-30.0, -60.0), (-50.0, -80.0)] {
        s.pointer(&ev(PointerKind::Move, x, y)).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, -50.0, -80.0)).unwrap();
    let n = net(&s, id);
    let h = n.handle(seg, start).unwrap();
    let p = page_pt(&s, id, a) + Vec2::new(h.0, h.1);
    assert!(near(p, pt(-50.0, -80.0)), "handle at {p:?}");
}

#[test]
fn pens_snap_their_clicks() {
    let mut s = Session::new();
    ex(&mut s, "vector.create", json!({"points": [[300, 0], [400, 100], [320, 60]], "closed": true}));
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.tool", json!({"tool": "pen"}));
    s.pointer(&ev(PointerKind::Hover, 322.0, 58.0)).unwrap();
    assert!(!s.tool.overlay.guides.is_empty(), "hovering shows where the click lands");
    click(&mut s, 322.0, 58.0);
    let n = net(&s, id);
    assert!(near(page_pt(&s, id, n.vertices.len() - 1), pt(320.0, 60.0)));
    ex(&mut s, "vector.edit.end", json!({}));
    // The pen tool: the second click lines up with the first.
    s.set_tool(ToolId::Pen);
    click(&mut s, 0.0, 300.0);
    click(&mut s, 80.0, 303.0);
    s.pen_finish().unwrap();
    let pen = s.selection()[0];
    let b = s.doc().unwrap().world_bounds(pen).unwrap();
    assert_eq!((b.y0, b.y1, b.x1), (300.0, 300.0, 80.0), "a level line");
}

#[test]
fn edit_mode_transform_commands_survive_junk() {
    let junk = [
        json!({}),
        json!(null),
        json!({"width": -1e308, "height": "x", "center": "maybe", "angle": 1e308, "cx": "NaN", "matrix": [1, 2, 3, "a", 5, 6]}),
        json!({"matrix": [0, 0, 0, 0, 0, 0], "vertices": [1e12, -1], "width": 1e308, "height": 0}),
    ];
    for c in
        ["object.flipH", "object.flipV", "object.rotate180", "object.rotateLeft", "object.rotateRight", "vector.edit.resize", "vector.edit.rotate"]
    {
        for j in &junk {
            let mut s = Session::new();
            let id = rect(&mut s);
            select_all(&mut s);
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(c, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
            let n = net(&s, id);
            assert!(n.vertices.iter().all(|v| v.x.is_finite() && v.y.is_finite()), "{c} broke the points on {j}");
        }
    }
}

/// Two edited layers turned differently: `a` a 100×100 square at the origin turned -20°, `b` one
/// at (200, 0) turned 30° (the active layer), with two of `a`'s corners and all of `b`'s points
/// selected. Neither layer's axes are the page's. Snapping (the pixel grid too) is off so drags
/// land exactly.
fn two_layers(s: &mut Session) -> (NodeId, NodeId) {
    let mut square = |x: f64| crate::params::id_of(&ex(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 100, "height": 100}))["id"]).unwrap();
    let (a, b) = (square(0.0), square(200.0));
    ex(s, "node.set", json!({"id": a, "props": {"rotation": -20}}));
    ex(s, "node.set", json!({"id": b, "props": {"rotation": 30}}));
    ex(s, "view.snapGeometry", json!({"on": false}));
    ex(s, "view.snapObjects", json!({"on": false}));
    ex(s, "view.snapPixel", json!({"on": false}));
    ex(s, "vector.edit.begin", json!({"ids": [a, b]}));
    let n = net(s, a);
    ex(s, "vector.edit.select", json!({"vertices": [vertex_at(&n, 0.0, 0.0), vertex_at(&n, 100.0, 100.0)]}));
    ex(s, "vector.edit.select", json!({"id": b, "vertices": [0, 1, 2, 3], "add": true}));
    assert_eq!(ve(s).id, b, "the layer named last is the active one");
    (a, b)
}

/// Every selected point's page position, layer by layer.
fn selected_page_points(s: &Session, a: NodeId, b: NodeId) -> Vec<(NodeId, usize, Point)> {
    let e = ve(s);
    let parts = e.parts();
    [a, b]
        .iter()
        .flat_map(|id| {
            let part = parts.iter().find(|p| p.id == *id).unwrap();
            part.vertices.iter().map(|v| (*id, *v, page_pt(s, *id, *v))).collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_box_wraps_points_selected_in_several_layers() {
    let mut s = Session::new();
    let (a, b) = two_layers(&mut s);
    let before = selected_page_points(&s, a, b);
    assert_eq!(before.len(), 6);
    // On the page's axes: `b` is turned, so neither layer's own axes fit both.
    let (x0, y0) = before.iter().fold((f64::INFINITY, f64::INFINITY), |(x, y), (_, _, p)| (x.min(p.x), y.min(p.y)));
    let (x1, y1) = before.iter().fold((f64::NEG_INFINITY, f64::NEG_INFINITY), |(x, y), (_, _, p)| (x.max(p.x), y.max(p.y)));
    let c = s.vector_edit_box().expect("a box around the points of both layers");
    assert!(near(c[0], pt(x0, y0)) && near(c[2], pt(x1, y1)), "{c:?} vs ({x0}, {y0})–({x1}, {y1})");

    // The bottom-right corner dragged to twice the width scales every point in both layers.
    drag(&mut s, (x1, y1), (x0 + 2.0 * (x1 - x0), y1));
    for (id, v, p) in &before {
        let q = page_pt(&s, *id, *v);
        assert!(near(q, pt(x0 + 2.0 * (p.x - x0), p.y)), "point {v} of {id:?}: {p:?} → {q:?}");
    }
    s.undo().unwrap();
    for (id, v, p) in &before {
        assert!(near(page_pt(&s, *id, *v), *p), "one undo step brings back point {v} of {id:?}");
    }

    // From just outside the top-right corner, a quarter turn clockwise about the box's centre.
    let ctr = pt((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let from = pt(x1 + 10.0, y0 - 10.0);
    let d = from - ctr;
    drag(&mut s, (from.x, from.y), (ctr.x - d.y, ctr.y + d.x));
    for (id, v, p) in &before {
        let r = *p - ctr;
        let q = page_pt(&s, *id, *v);
        assert!(near(q, pt(ctr.x - r.y, ctr.y + r.x)), "point {v} of {id:?}: {p:?} → {q:?}");
    }
    s.undo().unwrap();
    for (id, v, p) in &before {
        assert!(near(page_pt(&s, *id, *v), *p), "one undo step brings back point {v} of {id:?}");
    }
}

#[test]
fn inside_the_box_of_several_layers_drags_and_a_click_clears_them_all() {
    let mut s = Session::new();
    let (a, b) = two_layers(&mut s);
    let before = selected_page_points(&s, a, b);
    // Clear of both networks: between the two squares, inside the box.
    let doc = s.doc().unwrap().clone();
    assert!(hit_any(&doc, &ve(&s), pt(150.0, 50.0), 6.0).is_none());
    assert_eq!(box_hit(&doc, &ve(&s), None, pt(150.0, 50.0), 1.0), Some(BoxHit::Inside));
    drag(&mut s, (150.0, 50.0), (170.0, 60.0));
    for (id, v, p) in &before {
        assert!(near(page_pt(&s, *id, *v), *p + Vec2::new(20.0, 10.0)), "point {v} of {id:?} moves along");
    }
    s.undo().unwrap();
    click(&mut s, 150.0, 50.0);
    assert!(ve(&s).parts().iter().all(|p| p.vertices.is_empty()), "a click inside clears every layer's points");
}

#[test]
fn inside_the_box_of_another_layer_drags_over_a_region_of_the_active_one() {
    // `b`'s points selected, `a` (a filled 300 square around `b`) active with nothing selected:
    // the press at the box's centre lands on `a`'s region too.
    let mut s = Session::new();
    let mut square = |x: f64, size: f64| {
        crate::params::id_of(&ex(&mut s, "shape.rectangle", json!({"x": x, "y": x, "width": size, "height": size}))["id"]).unwrap()
    };
    let (a, b) = (square(0.0, 300.0), square(100.0, 50.0));
    ex(&mut s, "view.snapGeometry", json!({"on": false}));
    ex(&mut s, "view.snapObjects", json!({"on": false}));
    ex(&mut s, "view.snapPixel", json!({"on": false}));
    ex(&mut s, "vector.edit.begin", json!({"ids": [a, b]}));
    ex(&mut s, "vector.edit.select", json!({"id": b, "vertices": [0, 1, 2, 3]}));
    ex(&mut s, "vector.edit.select", json!({"id": a, "add": true}));
    let doc = s.doc().unwrap().clone();
    let e = ve(&s);
    assert!(e.id == a && e.vertices.is_empty() && e.others_selected());
    assert!(hit_any(&doc, &e, pt(125.0, 125.0), 6.0).is_none());
    assert!(regions::region_hit(&doc, &e, pt(125.0, 125.0), 1.0).is_some(), "a region of the active layer under the press");
    assert_eq!(box_hit(&doc, &e, None, pt(125.0, 125.0), 1.0), Some(BoxHit::Inside));
    let before: Vec<Point> = (0..4).map(|v| page_pt(&s, b, v)).collect();
    drag(&mut s, (125.0, 125.0), (145.0, 135.0));
    assert!(ve(&s).regions.is_empty(), "the region isn't selected");
    for (v, p) in before.iter().enumerate() {
        assert!(near(page_pt(&s, b, v), *p + Vec2::new(20.0, 10.0)), "point {v} of b moves along");
    }
}
