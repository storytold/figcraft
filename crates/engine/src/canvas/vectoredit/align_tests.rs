//! Align and distribute the selected points in vector edit mode.

use serde_json::json;

use super::tests::{ex, net, pt, ve};
use super::*;

/// The measured path: points at (330, 480), (480, 430), (560, 560), (380, 620), in edit mode.
fn path(s: &mut Session) -> NodeId {
    let r = ex(s, "vector.create", json!({"points": [[330, 480], [480, 430], [560, 560], [380, 620]], "closed": false}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(s, "vector.edit.begin", json!({"id": id}));
    id
}

fn page(s: &Session, id: NodeId) -> Vec<Point> {
    let w = s.doc().unwrap().world_transform(id);
    net(s, id).vertices.iter().map(|v| w * v.point()).collect()
}

fn page_of(s: &Session, id: NodeId, p: Point) -> Point {
    s.doc().unwrap().world_transform(id) * p
}

fn select(s: &mut Session, v: &[usize]) {
    ex(s, "vector.edit.select", json!({"vertices": v}));
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

#[test]
fn each_alignment_moves_the_points_along_one_axis_to_their_box() {
    // Points 0 and 1: box x 330..480, y 430..480.
    let cases = [
        ("align.left", [pt(330.0, 480.0), pt(330.0, 430.0)]),
        ("align.hcenter", [pt(405.0, 480.0), pt(405.0, 430.0)]),
        ("align.right", [pt(480.0, 480.0), pt(480.0, 430.0)]),
        ("align.top", [pt(330.0, 430.0), pt(480.0, 430.0)]),
        ("align.vcenter", [pt(330.0, 455.0), pt(480.0, 455.0)]),
        ("align.bottom", [pt(330.0, 480.0), pt(480.0, 480.0)]),
    ];
    for (cmd, want) in cases {
        let mut s = Session::new();
        let id = path(&mut s);
        let before = page(&s, id);
        select(&mut s, &[0, 1]);
        ex(&mut s, cmd, json!({}));
        let after = page(&s, id);
        assert!(near(after[0], want[0]) && near(after[1], want[1]), "{cmd}: {after:?}");
        assert!(near(after[2], before[2]) && near(after[3], before[3]), "{cmd} left the others");
    }
}

#[test]
fn one_point_does_not_move() {
    let mut s = Session::new();
    let id = path(&mut s);
    let before = page(&s, id);
    select(&mut s, &[1]);
    for c in ["align.left", "align.bottom", "distribute.horizontal"] {
        ex(&mut s, c, json!({}));
    }
    assert_eq!(page(&s, id), before);
}

#[test]
fn two_points_do_not_distribute() {
    let mut s = Session::new();
    let id = path(&mut s);
    let before = page(&s, id);
    select(&mut s, &[0, 1]);
    for c in ["distribute.horizontal", "distribute.vertical"] {
        ex(&mut s, c, json!({}));
    }
    assert_eq!(page(&s, id), before);
}

/// A vector inside a frame at (100, 100), its left (150) well inside the frame.
fn framed(s: &mut Session) -> (NodeId, NodeId) {
    let f = crate::params::id_of(&ex(s, "frame.create", json!({"x": 100, "y": 100, "width": 400, "height": 400}))["id"]).unwrap();
    let r = ex(s, "vector.create", json!({"points": [[50, 100], [150, 50], [200, 200]], "closed": false, "parent": f}));
    (f, crate::params::id_of(&r["id"]).unwrap())
}

const ALIGNS: [&str; 6] = ["align.left", "align.hcenter", "align.right", "align.top", "align.vcenter", "align.bottom"];

#[test]
fn no_points_selected_moves_nothing_and_not_the_layer() {
    let mut s = Session::new();
    let (frame, v) = framed(&mut s);
    ex(&mut s, "vector.edit.begin", json!({"id": v}));
    ex(&mut s, "vector.edit.select", json!({"vertices": []}));
    assert_eq!(transform::edit_point_count(s.doc().unwrap(), &ve(&s)), 0);
    let (points, bounds, local) = (page(&s, v), s.doc().unwrap().world_bounds(v), net(&s, v).vertices);
    assert!(bounds.unwrap().x0 > s.doc().unwrap().world_bounds(frame).unwrap().x0 + 10.0, "starts well inside the frame");
    for c in ALIGNS.into_iter().chain(["distribute.horizontal", "distribute.vertical"]) {
        ex(&mut s, c, json!({}));
        assert_eq!(page(&s, v), points, "{c}");
        assert_eq!(s.doc().unwrap().world_bounds(v), bounds, "{c} moved the layer");
        assert_eq!(net(&s, v).vertices, local, "{c}");
    }
}

#[test]
fn points_of_several_layers_align_and_one_undo_restores_all() {
    let mut s = Session::new();
    let mut line = |p: serde_json::Value| crate::params::id_of(&ex(&mut s, "vector.create", json!({"points": p, "closed": false}))["id"]).unwrap();
    let (a, b) = (line(json!([[0, 100], [100, 150]])), line(json!([[300, 40], [400, 90]])));
    ex(&mut s, "vector.edit.begin", json!({"ids": [a, b]}));
    ex(&mut s, "vector.edit.select", json!({"id": a, "vertices": [0]}));
    ex(&mut s, "vector.edit.select", json!({"id": b, "vertices": [1], "add": true}));
    let (pa, pb) = (page(&s, a), page(&s, b));
    ex(&mut s, "align.top", json!({}));
    let (na, nb) = (page(&s, a), page(&s, b));
    let top = pa[0].y.min(pb[1].y);
    assert!(near(na[0], pt(pa[0].x, top)) && near(nb[1], pt(pb[1].x, top)), "{na:?} {nb:?}");
    assert!(near(na[1], pa[1]) && near(nb[0], pb[0]), "unselected points stay");
    s.undo().unwrap();
    let (ua, ub) = (page(&s, a), page(&s, b));
    assert!(pa.iter().zip(&ua).chain(pb.iter().zip(&ub)).all(|(x, y)| near(*x, *y)), "{ua:?} {ub:?}");
}

#[test]
fn a_rotated_layer_aligns_in_the_boxs_own_frame() {
    let mut s = Session::new();
    let id = path(&mut s);
    ex(&mut s, "node.set", json!({"id": id, "props": {"rotation": 30}}));
    select(&mut s, &[0, 1]);
    let b = transform::point_box(s.doc().unwrap(), &ve(&s)).unwrap();
    let inv = b.frame.inverse();
    let before: Vec<Point> = page(&s, id).iter().map(|p| inv * *p).collect();
    assert!((before[0].x - before[1].x).abs() > 1.0, "the points differ in box x");
    ex(&mut s, "align.left", json!({}));
    let after: Vec<Point> = page(&s, id).iter().map(|p| inv * *p).collect();
    for i in 0..2 {
        assert!(after[i].x.abs() < 1e-6, "box x is the box's left: {after:?}");
        assert!((after[i].y - before[i].y).abs() < 1e-6, "box y unchanged: {after:?}");
    }
}

#[test]
fn id_in_edit_mode_moves_the_layer_not_the_points() {
    let mut s = Session::new();
    let (frame, v) = framed(&mut s);
    ex(&mut s, "vector.edit.begin", json!({"id": v}));
    select(&mut s, &[0, 1]);
    let local = net(&s, v).vertices;
    ex(&mut s, "align.left", json!({"id": v}));
    let d = s.doc().unwrap();
    assert!((d.world_bounds(v).unwrap().x0 - d.world_bounds(frame).unwrap().x0).abs() < 1e-6, "layer at the frame's left");
    assert_eq!(net(&s, v).vertices, local, "points were not aligned to each other");
}

#[test]
fn ids_distribute_layers_and_leave_the_edited_layers_points() {
    let mut s = Session::new();
    let mut rect = |x: f64| crate::params::id_of(&ex(&mut s, "shape.rectangle", json!({"x": x, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    let (r1, r2, r3) = (rect(0.0), rect(60.0), rect(400.0));
    ex(&mut s, "vector.edit.begin", json!({"id": r2}));
    select(&mut s, &[0, 1]);
    let local = net(&s, r2).vertices;
    ex(&mut s, "distribute.horizontal", json!({"ids": [r1, r2, r3]}));
    let x0 = s.doc().unwrap().world_bounds(r2).unwrap().x0;
    assert!((x0 - 200.0).abs() < 1e-6, "middle layer centred between the outer two: {x0}");
    assert_eq!(net(&s, r2).vertices, local);
}

#[test]
fn distribute_spaces_the_points_evenly_between_the_outermost() {
    let mut s = Session::new();
    let id = path(&mut s);
    select(&mut s, &[0, 1, 2, 3]);
    ex(&mut s, "distribute.horizontal", json!({}));
    let p = page(&s, id);
    let want_x = [330.0, 483.0 + 1.0 / 3.0, 560.0, 380.0 + 80.0 / 3.0];
    for (q, x) in p.iter().zip(want_x) {
        assert!((q.x - x).abs() < 1e-6, "{p:?}");
    }
    let want_y = [480.0, 430.0, 560.0, 620.0];
    assert!(p.iter().zip(want_y).all(|(q, y)| (q.y - y).abs() < 1e-6), "y unchanged: {p:?}");

    ex(&mut s, "distribute.vertical", json!({}));
    let p = page(&s, id);
    // y sorted 430, 480, 560, 620 spread over 430..620.
    let step = 190.0 / 3.0;
    let want_y = [430.0 + step, 430.0, 430.0 + 2.0 * step, 620.0];
    assert!(p.iter().zip(want_y).all(|(q, y)| (q.y - y).abs() < 1e-6), "{p:?}");
}

#[test]
fn a_curved_points_handles_move_with_it() {
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"path": "M 0 0 C 0 50 50 100 100 100 L 200 0", "x": 0, "y": 0}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let before = net(&s, id);
    let page_before = page(&s, id);
    let handle = |s: &Session, n: &VectorNetwork, v: usize, t: (f64, f64)| page_of(s, id, n.vertices[v].point() + Vec2::new(t.0, t.1));
    let (h1_before, h0_before) = (handle(&s, &before, 1, before.segments[0].tb), handle(&s, &before, 0, before.segments[0].ta));
    select(&mut s, &[0, 1]);
    ex(&mut s, "align.left", json!({}));
    let after = net(&s, id);
    let moved: Vec<usize> = (0..2).filter(|i| !near(page(&s, id)[*i], page_before[*i])).collect();
    assert!(!moved.is_empty(), "something moved");
    let p = page(&s, id);
    assert!(near(p[0], pt(0.0, 0.0)) && near(p[1], pt(0.0, 100.0)), "{p:?}");
    assert!(near(p[2], page_before[2]), "vertex 2 stays");
    // Handles: vertex 1's incoming one (vertex + segment 0's `tb`) moves with it by (-100, 0).
    let a0 = &after.segments[0];
    assert!(near(h1_before, pt(50.0, 100.0)) && near(handle(&s, &after, 1, a0.tb), pt(-50.0, 100.0)), "vertex 1's handle moved by (-100, 0)");
    assert!(near(h0_before, handle(&s, &after, 0, a0.ta)), "vertex 0's handle stays");
    assert_eq!(before.segments.len(), after.segments.len());
    for (a, b) in before.segments.iter().zip(&after.segments) {
        assert_eq!((a.ta, a.tb), (b.ta, b.tb), "handle offsets travel with their vertex");
    }
}

#[test]
fn one_undo_restores_the_points() {
    let mut s = Session::new();
    let id = path(&mut s);
    let before = page(&s, id);
    select(&mut s, &[0, 1, 2, 3]);
    ex(&mut s, "align.right", json!({}));
    assert_ne!(page(&s, id), before);
    s.undo().unwrap();
    let after = page(&s, id);
    assert!(before.iter().zip(&after).all(|(a, b)| near(*a, *b)), "{after:?}");
}

#[test]
fn ids_in_edit_mode_still_align_layers() {
    let mut s = Session::new();
    let mut square = |x: f64| crate::params::id_of(&ex(&mut s, "shape.rectangle", json!({"x": x, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    let (a, b) = (square(0.0), square(200.0));
    ex(&mut s, "vector.edit.begin", json!({"id": a}));
    select(&mut s, &[0, 1]);
    let local = net(&s, a).vertices;
    ex(&mut s, "align.left", json!({"ids": [a, b]}));
    let x = |id| s.doc().unwrap().world_bounds(id).unwrap().x0;
    assert!((x(a) - x(b)).abs() < 1e-6, "layers aligned");
    assert_eq!(net(&s, a).vertices, local, "the selected points kept their local positions");
}

#[test]
fn a_segment_selected_in_another_edited_layer_counts_both_its_ends() {
    // `a`'s segment (two ends) and `b`'s first point: three points, so the panel offers
    // Distribute and the command spaces all three.
    let mut s = Session::new();
    let mut line = |p: serde_json::Value| crate::params::id_of(&ex(&mut s, "vector.create", json!({"points": p, "closed": false}))["id"]).unwrap();
    let (a, b) = (line(json!([[0, 0], [100, 50]])), line(json!([[300, 200], [400, 260]])));
    ex(&mut s, "vector.edit.begin", json!({"ids": [a, b]}));
    ex(&mut s, "vector.edit.select", json!({"id": a, "segments": [0]}));
    ex(&mut s, "vector.edit.select", json!({"id": b, "vertices": [0], "add": true}));
    assert_eq!(transform::edit_point_count(s.doc().unwrap(), &ve(&s)), 3);
    ex(&mut s, "distribute.horizontal", json!({}));
    let xs: Vec<f64> = page(&s, a).iter().chain(page(&s, b).first()).map(|p| p.x).collect();
    assert!(xs.iter().zip([0.0, 150.0, 300.0]).all(|(x, w)| (x - w).abs() < 1e-6), "{xs:?}");
}
