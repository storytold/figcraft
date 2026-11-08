//! `vector.edit.position`: with several points, a number sets every selected point's own
//! coordinate, and `{add}`, `{mul}`, `{div}` apply to each point's own.

use serde_json::json;

use super::tests::{ex, net, rect};
use super::*;
use crate::Session;

/// Every point in the parent's coordinates (the layer re-fits its box after an edit, so its own
/// coordinates shift).
fn xs(s: &Session, id: NodeId) -> Vec<(f64, f64)> {
    let t = s.doc().unwrap().node(id).unwrap().transform;
    net(s, id).vertices.iter().map(|v| t * v.point()).map(|p| (p.x, p.y)).collect()
}

fn parent_xy(s: &Session) -> Vec<(f64, f64)> {
    let ve = s.ds().unwrap().vector_edit.clone().unwrap();
    selected_parent_points(s.doc().unwrap(), &ve).iter().map(|p| (p.x, p.y)).collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn a_number_sets_every_selected_points_own_coordinate() {
    let mut s = Session::new();
    let id = rect(&mut s);
    let before = xs(&s, id);
    ex(&mut s, "vector.edit.select", json!({"vertices": [0, 1]}));
    // The rectangle's points 0 and 1 differ in x; both end at -200 and keep their y.
    assert!(!near(before[0].0, before[1].0));
    ex(&mut s, "vector.edit.position", json!({"x": -200}));
    let after = xs(&s, id);
    assert!(near(after[0].0, -200.0) && near(after[1].0, -200.0), "{after:?}");
    assert!(near(after[0].1, before[0].1) && near(after[1].1, before[1].1));
    assert_eq!(after[2], before[2]);
    // One undo step restores.
    assert!(s.undo().unwrap());
    assert_eq!(xs(&s, id), before);
}

#[test]
fn one_point_sets_its_own_coordinates() {
    let mut s = Session::new();
    let id = rect(&mut s);
    let before = xs(&s, id);
    ex(&mut s, "vector.edit.select", json!({"vertices": [2]}));
    ex(&mut s, "vector.edit.position", json!({"x": 7, "y": 9}));
    let after = xs(&s, id);
    assert!(near(after[2].0, 7.0) && near(after[2].1, 9.0));
    assert_eq!(after[0], before[0]);
}

#[test]
fn relative_edits_apply_to_each_points_own_value() {
    let mut s = Session::new();
    let id = rect(&mut s);
    let before = xs(&s, id);
    ex(&mut s, "vector.edit.select", json!({"vertices": [0, 1]}));
    ex(&mut s, "vector.edit.position", json!({"x": {"add": 10}}));
    let a = xs(&s, id);
    assert!(near(a[0].0, before[0].0 + 10.0) && near(a[1].0, before[1].0 + 10.0), "{a:?}");
    ex(&mut s, "vector.edit.position", json!({"y": {"mul": 2}}));
    let b = xs(&s, id);
    assert!(near(b[0].1, a[0].1 * 2.0) && near(b[1].1, a[1].1 * 2.0), "{b:?}");
    ex(&mut s, "vector.edit.position", json!({"x": {"div": 2}}));
    let c = xs(&s, id);
    assert!(near(c[0].0, a[0].0 / 2.0) && near(c[1].0, a[1].0 / 2.0), "{c:?}");
}

#[test]
fn a_point_keeps_its_handles_relative_to_it() {
    let mut s = Session::new();
    let id = rect(&mut s);
    ex(&mut s, "vector.edit.select", json!({"vertices": [1]}));
    ex(&mut s, "vector.edit.smooth", json!({}));
    let before = net(&s, id);
    assert!(before.segments.iter().any(|g| g.ta != (0.0, 0.0) || g.tb != (0.0, 0.0)));
    ex(&mut s, "vector.edit.select", json!({"vertices": [0, 1]}));
    ex(&mut s, "vector.edit.position", json!({"x": {"add": 25}}));
    let after = net(&s, id);
    let tangents = |n: &VectorNetwork| n.segments.iter().map(|g| (g.ta, g.tb)).collect::<Vec<_>>();
    assert_eq!(tangents(&before), tangents(&after));
}

#[test]
fn the_values_are_in_the_parents_coordinates() {
    let mut s = Session::new();
    let r = ex(&mut s, "shape.rectangle", json!({"x": 40, "y": 30, "width": 100, "height": 100}));
    let id = crate::params::id_of(&r["id"]).unwrap();
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.select", json!({"vertices": [0, 1]}));
    let before = parent_xy(&s);
    assert!(near(before[0].0, 40.0), "{before:?}");
    ex(&mut s, "vector.edit.position", json!({"x": 10}));
    let after = parent_xy(&s);
    assert!(near(after[0].0, 10.0) && near(after[1].0, 10.0), "{after:?}");
    assert!(near(after[0].1, before[0].1));
}

#[test]
fn junk_is_refused_and_changes_nothing() {
    let mut s = Session::new();
    let id = rect(&mut s);
    let before = xs(&s, id);
    ex(&mut s, "vector.edit.select", json!({"vertices": [0, 1]}));
    for junk in [
        json!({"x": {"div": 0}}),
        json!({"x": {"pow": 2}}),
        json!({"x": "a"}),
        json!({"x": {"add": 1, "mul": 2}}),
        json!({"y": {"add": "x"}}),
        json!({"x": {}}),
        json!({"x": [1]}),
        json!({"x": {"mul": 1e308}}),
    ] {
        assert!(s.execute("vector.edit.position", &junk).is_err(), "{junk}");
    }
    assert_eq!(xs(&s, id), before);
}
