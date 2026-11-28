//! The arrow keys move a layer along the canvas axes whatever its parent's rotation (#2372).

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;

fn frame(s: &mut Session, parent: Option<NodeId>, w: f64, h: f64, rotation: f64) -> NodeId {
    let mut p = json!({"x": 10, "y": 20, "width": w, "height": h});
    if let (Some(par), Some(o)) = (parent, p.as_object_mut()) {
        o.insert("parent".into(), json!(par));
    }
    let r = s.execute("frame.create", &p).unwrap();
    let id = id_of(&r["id"]).unwrap();
    if rotation != 0.0 {
        s.execute("node.set", &json!({"id": id, "props": {"rotation": rotation}})).unwrap();
    }
    id
}

fn world(s: &Session, id: NodeId) -> (f64, f64) {
    let c = s.doc().unwrap().world_transform(id).as_coeffs();
    (c[4], c[5])
}

fn nudge(s: &mut Session, id: NodeId, params: Value) {
    let mut p = params;
    if let Some(o) = p.as_object_mut() {
        o.insert("ids".into(), json!([id]));
    }
    s.execute("transform.nudge", &p).unwrap();
}

fn moved_by(s: &mut Session, id: NodeId, params: Value) -> (f64, f64) {
    let before = world(s, id);
    nudge(s, id, params);
    let after = world(s, id);
    (after.0 - before.0, after.1 - before.1)
}

fn near(got: (f64, f64), want: (f64, f64)) {
    assert!((got.0 - want.0).abs() < 1e-6 && (got.1 - want.1).abs() < 1e-6, "moved by {got:?}, wanted {want:?}");
}

fn in_rotated(rotation: f64) -> (Session, NodeId) {
    let mut s = Session::new();
    let outer = frame(&mut s, None, 400.0, 300.0, rotation);
    let child = frame(&mut s, Some(outer), 100.0, 80.0, 0.0);
    (s, child)
}

#[test]
fn down_moves_a_child_of_a_frame_rotated_90_down_the_canvas() {
    let (mut s, child) = in_rotated(90.0);
    let k = s.prefs.nudge_small;
    near(moved_by(&mut s, child, json!({"dx": 0, "dy": 1})), (0.0, k));
}

#[test]
fn right_moves_it_right_on_the_canvas() {
    let (mut s, child) = in_rotated(90.0);
    let k = s.prefs.nudge_small;
    near(moved_by(&mut s, child, json!({"dx": 1, "dy": 0})), (k, 0.0));
}

#[test]
fn a_big_nudge_in_a_rotated_parent_follows_the_canvas_too() {
    let (mut s, child) = in_rotated(90.0);
    let k = s.prefs.nudge_big;
    near(moved_by(&mut s, child, json!({"dx": 1, "dy": 0, "big": true})), (k, 0.0));
}

#[test]
fn a_child_of_an_unrotated_frame_moves_as_before() {
    let (mut s, child) = in_rotated(0.0);
    let k = s.prefs.nudge_small;
    let local_before = s.doc().unwrap().node(child).unwrap().transform.as_coeffs();
    near(moved_by(&mut s, child, json!({"dx": 1, "dy": 0})), (k, 0.0));
    let local_after = s.doc().unwrap().node(child).unwrap().transform.as_coeffs();
    assert!((local_after[4] - local_before[4] - k).abs() < 1e-6 && (local_after[5] - local_before[5]).abs() < 1e-6);
}

#[test]
fn a_layer_two_rotated_frames_deep_moves_along_the_canvas() {
    let mut s = Session::new();
    let outer = frame(&mut s, None, 500.0, 400.0, 45.0);
    let inner = frame(&mut s, Some(outer), 300.0, 200.0, 30.0);
    let child = frame(&mut s, Some(inner), 100.0, 80.0, 0.0);
    let k = s.prefs.nudge_small;
    near(moved_by(&mut s, child, json!({"dx": 0, "dy": 1})), (0.0, k));
}
