//! `select.area`: what a marquee over a box selects, without a pointer (keyboard box selection in
//! the app, agents over MCP): top-level layers, a frame only when fully covered (otherwise the
//! layers in it the box touches), the children of the frame the box starts in, Shift-like toggling, Cmd/Ctrl-like nested layers; no box asks the
//! app to draw one.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn rect(s: &mut Session, x: f64, y: f64, parent: Option<NodeId>) -> NodeId {
    id_of(&ex(s, "node.create", json!({"type": "rectangle", "parent": parent, "x": x, "y": y, "width": 20, "height": 20}))["id"]).unwrap()
}

fn area(s: &mut Session, x: f64, y: f64, w: f64, h: f64, extra: Value) -> Vec<NodeId> {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    let out = ex(s, "select.area", p);
    let listed: Vec<NodeId> = out["selection"].as_array().unwrap().iter().filter_map(id_of).collect();
    assert_eq!(listed, s.selection(), "the result lists the new selection");
    sorted(listed)
}

fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
    v.sort();
    v
}

#[test]
fn a_box_selects_like_a_marquee() {
    let mut s = Session::new();
    let frame = id_of(&ex(&mut s, "frame.create", json!({"x": 300, "y": 0, "width": 200, "height": 200}))["id"]).unwrap();
    let inner = rect(&mut s, 20.0, 20.0, Some(frame));
    let a = rect(&mut s, 0.0, 0.0, None);
    let b = rect(&mut s, 100.0, 0.0, None);
    // Top-level layers it touches; a frame only touched from outside gives the layers in it the box touches.
    assert_eq!(area(&mut s, -10.0, -10.0, 340.0, 40.0, json!({})), sorted(vec![a, b, inner]));
    // A negative size draws the box the other way.
    assert_eq!(area(&mut s, 130.0, 30.0, -140.0, -40.0, json!({})), sorted(vec![a, b]));
    // Covering the frame selects it.
    assert_eq!(area(&mut s, 290.0, -10.0, 220.0, 220.0, json!({})), vec![frame]);
    // Starting inside the frame selects its children.
    assert_eq!(area(&mut s, 305.0, 5.0, 60.0, 60.0, json!({})), vec![inner]);
    // Deep: nested layers too.
    assert_eq!(area(&mut s, -10.0, -10.0, 340.0, 40.0, json!({"deep": true})), sorted(vec![a, b, inner]));
    // Add toggles: what was selected and is covered again comes out.
    ex(&mut s, "select.set", json!({"ids": [a]}));
    assert_eq!(area(&mut s, 90.0, -10.0, 40.0, 40.0, json!({"add": true})), sorted(vec![a, b]));
    assert_eq!(area(&mut s, -10.0, -10.0, 40.0, 40.0, json!({"add": true})), vec![b]);
    // An empty spot clears the selection (without add).
    assert_eq!(area(&mut s, 1000.0, 1000.0, 10.0, 10.0, json!({})), Vec::<NodeId>::new());
}

#[test]
fn no_box_asks_the_app_for_one_and_bad_boxes_are_refused() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, None);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    s.ui_requests.clear();
    ex(&mut s, "select.area", json!({}));
    assert!(s.ui_requests.iter().any(|r| r.kind == "selectArea"));
    assert_eq!(s.selection(), vec![a], "nothing selected yet");
    for bad in [
        json!({"x": 0, "y": 0, "width": 10}),
        json!({"x": 0, "y": 0, "width": "wide", "height": 1}),
        json!({"x": 1e300, "y": 0, "width": 1, "height": 1}),
    ] {
        assert!(s.execute("select.area", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.selection(), vec![a]);
}
