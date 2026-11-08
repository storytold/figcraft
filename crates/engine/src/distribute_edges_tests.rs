//! Distributing by edges and centers: `distribute.left` and friends.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

#[test]
fn distribute_left_edges_interpolates_the_left_edge() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 30.0, 10.0);
    let c = rect(&mut s, 200.0, 0.0, 20.0, 10.0);
    s.execute("distribute.left", &json!({"ids": [a, b, c]})).unwrap();
    let doc = s.doc().unwrap();
    // Sorted by left edge: a at 0 stays, b's left lands halfway, c at 200 stays.
    assert_eq!(doc.world_bounds(a).unwrap().x0, 0.0);
    assert_eq!(doc.world_bounds(b).unwrap().x0, 100.0);
    assert_eq!(doc.world_bounds(c).unwrap().x0, 200.0);
    // Interior interpolation over four: b strays to 150, and lands a third of the span.
    let d = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    s.execute("transform.move", &json!({"ids": [b], "dx": -50.0, "dy": 0.0})).unwrap();
    s.execute("distribute.left", &json!({"ids": [a, d, b, c]})).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.world_bounds(a).unwrap().x0, 0.0);
    assert_eq!(doc.world_bounds(d).unwrap().x0, 300.0);
    assert_eq!(doc.world_bounds(b).unwrap().x0, 100.0);
    assert_eq!(doc.world_bounds(c).unwrap().x0, 200.0);
}

#[test]
fn distribute_horizontal_centers_keeps_the_outer_two() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 5.0, 40.0, 10.0);
    let c = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    s.execute("distribute.horizontalCenters", &json!({"ids": [b, a, c]})).unwrap();
    let doc = s.doc().unwrap();
    // Centers: a 5, b 120, c 205 -> b's centre lands halfway between 5 and 205.
    assert_eq!(doc.world_bounds(a).unwrap().x0, 0.0);
    assert_eq!(doc.world_bounds(c).unwrap().x0, 200.0);
    let bb = doc.world_bounds(b).unwrap();
    assert_eq!(bb.center().x, 105.0);
}

#[test]
fn distribute_top_and_bottom_edges_work_vertically() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 0.0, 50.0, 10.0, 10.0);
    let c = rect(&mut s, 0.0, 100.0, 10.0, 10.0);
    s.execute("distribute.top", &json!({"ids": [a, c, b]})).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.world_bounds(a).unwrap().y0, 0.0);
    assert_eq!(doc.world_bounds(b).unwrap().y0, 50.0);
    assert_eq!(doc.world_bounds(c).unwrap().y0, 100.0);

    let d = rect(&mut s, 0.0, 300.0, 10.0, 20.0);
    s.execute("distribute.bottom", &json!({"ids": [a, b, d]})).unwrap();
    let doc = s.doc().unwrap();
    // Bottoms: a 10 stays, d 320 stays, b's bottom lands halfway (165).
    assert_eq!(doc.world_bounds(a).unwrap().y1, 10.0);
    assert_eq!(doc.world_bounds(d).unwrap().y1, 320.0);
    assert_eq!(doc.world_bounds(b).unwrap().y1, 165.0);
}

#[test]
fn fewer_than_three_targets_are_a_no_op() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 10.0, 10.0);
    s.execute("distribute.left", &json!({"ids": [b, a]})).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.world_bounds(a).unwrap().x0, 0.0);
    assert_eq!(doc.world_bounds(b).unwrap().x0, 100.0);
    s.execute("distribute.left", &json!({"ids": [a]})).unwrap();
    s.execute("distribute.left", &json!({})).unwrap();
    assert_eq!(s.undo_depth(), (5, 0), "no-ops still take their undo steps");
}

#[test]
fn distribute_edges_is_one_undo_step() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 10.0, 10.0);
    let c = rect(&mut s, 200.0, 0.0, 10.0, 10.0);
    let depth_before = s.undo_depth().0;
    s.execute("distribute.left", &json!({"ids": [a, b, c]})).unwrap();
    assert_eq!(s.undo_depth(), (depth_before + 1, 0));
    assert!(s.undo().unwrap());
    let doc = s.doc().unwrap();
    assert_eq!(doc.world_bounds(b).unwrap().x0, 100.0);
}
