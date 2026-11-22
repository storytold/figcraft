//! A boolean group's corner radius through the engine: it rounds the result (and the SVG export
//! of it), children keep their own radius, Flatten bakes the arcs and clicks follow the result.

use serde_json::json;

use crate::Session;
use crate::booleans_tests::{area, click, ex};
use crate::params::id_of;
use figcraft_doc::{NodeId, NodeKind};
use figcraft_geom::{Point, Shape};

const L_AREA: f64 = 34656.6;

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"]).unwrap()
}

fn snapping_off(s: &mut Session) {
    ex(s, "select.none", json!({}));
    let ds = s.ds_mut().unwrap();
    ds.view.snap_objects = false;
    ds.view.snap_pixel = false;
}

fn set_radius(s: &mut Session, id: NodeId, r: f64) {
    ex(s, "node.set", json!({"ids": [id], "props": {"cornerRadius": r}}));
}

/// The L of two rectangles, united, nothing selected, snapping off.
fn l_group(s: &mut Session) -> NodeId {
    let (a, b) = (rect(s, 0.0, 0.0, 200.0, 100.0), rect(s, 150.0, 0.0, 100.0, 200.0));
    let g = id_of(&ex(s, "bool.union", json!({"ids": [a, b]}))["id"]).unwrap();
    snapping_off(s);
    g
}

/// Whether the page point is inside the group's drawn result.
fn covers(s: &Session, g: NodeId, x: f64, y: f64) -> bool {
    let doc = s.doc().unwrap();
    let (path, _) = figcraft_render::node_outline(doc, doc.node(g).unwrap()).unwrap();
    let p = doc.world_transform(g).inverse() * Point::new(x, y);
    path.winding(p) % 2 != 0
}

#[test]
fn a_boolean_group_rounds_every_sharp_corner_of_its_result() {
    let mut s = Session::new();
    let g = l_group(&mut s);
    assert!(covers(&s, g, 1.0, 1.0));
    set_radius(&mut s, g, 20.0);
    assert!(!covers(&s, g, 1.0, 1.0) && covers(&s, g, 148.0, 102.0));
    assert!((area(&s, g) - L_AREA).abs() < 1.0, "{}", area(&s, g));
    let doc = s.doc().unwrap();
    let svg = figcraft_svg::export_svg(doc, &[g], &figcraft_svg::SvgOptions::default()).unwrap();
    let d = svg.split("d=\"").nth(1).and_then(|r| r.split('"').next()).unwrap();
    let exported = figcraft_geom::svg_path(d).unwrap();
    assert!((exported.area().abs() - L_AREA).abs() < 1.0, "{}", exported.area());
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().node(g).unwrap().corners.radii, [0.0; 4], "one undo step");
    assert!((area(&s, g) - 35000.0).abs() < 1.0);
}

/// The notch of the measurement: A (200 by 100 at the origin) minus a 100 square on its top edge,
/// whose own radius is `a_radius`, under a group radius of `radius`.
fn notch(s: &mut Session, a: NodeId, radius: f64) -> NodeId {
    let b = rect(s, 50.0, -50.0, 100.0, 100.0);
    let g = id_of(&ex(s, "bool.subtract", json!({"ids": [a, b]}))["id"]).unwrap();
    snapping_off(s);
    set_radius(s, g, radius);
    g
}

/// The asserts of the measurement on a notched group at radius 20 under A's own 10.
fn assert_notch_at_20(s: &Session, g: NodeId) {
    assert!(covers(s, g, 4.0, 4.0), "A's 10 was not enlarged to 20");
    assert!(!covers(s, g, 48.0, 2.0), "the cut's top corner rounds");
    assert!(covers(s, g, 52.0, 48.0), "the cut's inner corner fills in");
}

#[test]
fn children_keep_their_radius_and_a_subtraction_rounds_its_cut() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 200.0, 100.0);
    set_radius(&mut s, a, 10.0);
    let g = notch(&mut s, a, 20.0);
    assert_notch_at_20(&s, g);
    assert_eq!(s.doc().unwrap().node(a).unwrap().corners.radii, [10.0; 4]);
    set_radius(&mut s, g, 500.0);
    assert_eq!(s.doc().unwrap().node(g).unwrap().corners.radii[0], 500.0, "the field keeps the value");
    assert!(!covers(&s, g, 40.0, 4.0), "25, not the 20 that half the 40-long edge would give");
    assert!(covers(&s, g, 27.0, 1.0), "and not more than 25");
    assert!(covers(&s, g, 100.0, 51.0) && !covers(&s, g, 100.0, 49.0), "the floor stays straight between 75 and 125");
    assert!(covers(&s, g, 4.0, 4.0));
}

#[test]
fn a_nested_group_keeps_its_own_radius() {
    let mut s = Session::new();
    let (l, r) = (rect(&mut s, 0.0, 0.0, 120.0, 100.0), rect(&mut s, 80.0, 0.0, 120.0, 100.0));
    let inner = id_of(&ex(&mut s, "bool.union", json!({"ids": [l, r]}))["id"]).unwrap();
    set_radius(&mut s, inner, 10.0);
    let g = notch(&mut s, inner, 20.0);
    assert_notch_at_20(&s, g);
}

#[test]
fn flatten_bakes_the_arcs_and_the_vector_has_no_radius() {
    let mut s = Session::new();
    let g = l_group(&mut s);
    set_radius(&mut s, g, 20.0);
    let flat = id_of(&ex(&mut s, "vector.flatten", json!({"ids": [g]}))["id"]).unwrap();
    let doc = s.doc().unwrap();
    let n = doc.node(flat).unwrap();
    let NodeKind::Vector { network } = &n.kind else { panic!("a vector: {:?}", n.kind) };
    assert_eq!(n.corners.radii, [0.0; 4]);
    assert!(network.vertices.iter().all(|v| v.radius == 0.0));
    let path = figcraft_doc::geometry::shape_path(n).unwrap();
    assert!((path.area().abs() - L_AREA).abs() < 1.0, "{}", path.area());
    assert!((n.width - 250.0).abs() < 1e-6 && (n.height - 200.0).abs() < 1e-6);
}

#[test]
fn clicks_follow_the_rounded_result() {
    let mut s = Session::new();
    let g = l_group(&mut s);
    set_radius(&mut s, g, 20.0);
    click(&mut s, 1.0, 1.0, false);
    assert!(s.selection().is_empty(), "the cut-off corner reaches what is behind");
    click(&mut s, 148.0, 102.0, false);
    assert_eq!(s.selection(), vec![g], "the concave fillet belongs to the group");
    ex(&mut s, "select.none", json!({}));
    click(&mut s, 100.0, 50.0, false);
    assert_eq!(s.selection(), vec![g]);
}
