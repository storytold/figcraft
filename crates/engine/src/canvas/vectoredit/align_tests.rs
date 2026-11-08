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
    select(&mut s, &[0, 1]);
    ex(&mut s, "align.left", json!({}));
    let after = net(&s, id);
    let moved: Vec<usize> = (0..2).filter(|i| !near(page(&s, id)[*i], page_before[*i])).collect();
    assert!(!moved.is_empty(), "something moved");
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
    ex(&mut s, "align.left", json!({"ids": [a, b]}));
    let x = |id| s.doc().unwrap().world_bounds(id).unwrap().x0;
    assert!((x(a) - x(b)).abs() < 1e-6, "layers aligned");
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
